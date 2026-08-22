# Phase 7 Slice 3g: self-recursion in a non-inline generic body (brief)

A non-inline polymorphic word cannot call itself. Probe-verified at HEAD:

```sooth
import: intrinsics * ;
import: core::prelude * ;

: iszero ( i64 -- bool ) 0 eq ;

: loopg ( 'T: Copy 'T i64 -- 'T )
  dup iszero ~[ drop ] ~[ 1 sub loopg ] if ;

: main ( -- ) 5 3 loopg . ;
```

```text
error: `loopg` cannot call the polymorphic word `loopg` (line 7, col 33)
  a polymorphic word is not yet reachable from another polymorphic word across a module boundary
  inline the caller, make the callee concrete, or call the callee from a monomorphic word
```

The diagnostic is wrong on its face: `loopg` is not "another polymorphic word across a
module boundary," it is the very word being checked. This is `poly_calls_poly_word_error`
(P8.S2's generic-calls-generic message, `src/check/poly.rs:1443`), and self-recursion is
the one instance of that gap that needs no cross-word registry lookup at all — the
callee's signature is `sig`, the argument `poly_call_term` already carries for the caller
itself.

## Recon (verified against the source at HEAD, not inferred)

1. **The fall-through that produces the error does not distinguish self from other.**
   `poly_call_term` (`src/check/poly.rs:1269-1275`): after the operator-delegate path
   declines, `if poly_words.contains(name) { return Err(poly_calls_poly_word_error(...)) }`.
   `poly_words` is every poly word's name in the module (`check.rs:672`,
   `poly_env.keys().cloned().collect()`), built *before* any body is checked, so the
   currently-walked word's own name is always a member of its own `poly_words` set. There
   is no self-name check anywhere ahead of this.

2. **`PolySig` carries no name** (`src/ast.rs:1437-1449`: `row_in`, `inputs`, `outputs`,
   `row_out`, `bounds`, `ty_var_names`, `len_var_names`, `row_var_names` — no `name` field).
   So `sig` alone (already a `poly_call_term` parameter) cannot answer "is this a
   self-call"; the name has to come from `ctx`.

3. **`ctx` already carries it, for free.** `Ctx::Word` stores `name` (demangled) and
   `mangled` (`src/check/engine.rs:1147-1166`, `word_ctx`), with existing accessors
   `word_name()` (`engine.rs:1220-1226`) and `mangled_name()` (`engine.rs:1228-1232`),
   both already used by the concrete checker to recognize a self-tail-call back-edge
   (`captures.rs:100` and elsewhere). `poly_call_term` already takes `ctx: &Ctx` as a
   parameter, so a self-name comparison needs no new plumbing through
   `check_poly_body`/`poly_walk`'s signatures at all — it is available at the exact site
   the wrong error is currently returned.

4. **Resolved: `ctx.mangled_name()` is the correct comparand, not `ctx.word_name()`.**
   `resolve::mangle` (`src/resolve.rs:17-38`) is unconditional over every name except
   `main`/`drop`, and its module doc (`resolve.rs:1-15`) states it explicitly: the pass
   "mangles every decl name to a module-unique form ... and rewrites every body reference
   to match." A self-call term inside `loopg`'s own body is a body reference to `loopg`,
   so in a multi-module closure it is rewritten to the same `loopg__m1` spelling
   `word.name` (and therefore `ctx.mangled_name()`) already carries; in a single-module
   closure the pass is a byte-for-byte no-op (`resolve.rs:12-14`), so the two names still
   coincide there too. `ctx.mangled_name()` is correct in both cases; `ctx.word_name()`
   (the demangled display spelling) would falsely fail to match in the multi-module case.
   A two-file probe attempting to confirm this by actually building hit an unrelated,
   pre-existing arity anomaly in cross-module bound signatures (a monomorphic caller's
   `check_poly_call` reported `loopg`'s 2-input signature as needing 3 once `loopg` moved
   to a second file) — reproducible, but orthogonal to this slice and worth its own,
   separate bug report rather than being absorbed into S3g's scope.

5. **A self-call needs no unification or `Subst` at check time.** `sig` *is* the callee's
   signature, with the exact same rigid type-variable ids the current walk is already
   using — there is no second, independent instantiation to unify against. The correct
   check is a pointwise structural match of the operand window against `sig.inputs`,
   producing `sig.outputs` on success: exactly the same comparison `check_poly_body` already
   performs between the body's residual stack and `sig.outputs` at exit
   (`poly.rs:459-461`, `residual_pt != sig.outputs`), just run mid-body instead of at the
   end. No `PolyType::Var` grounding, no `apply_subst`, no `GenericTypes` mint.

6. **Lowering already has the general poly-callee mechanism, and it does not fit here.** A
   call to a polymorphic word from a monomorphic (or, once P7.S3k lands, another
   polymorphic) caller resolves through `instantiations: &HashMap<Span, CallInst>`, keyed
   by the call site's span (`src/ir/func_builder/calls.rs:307-315`). But that table is
   never populated for any call made from *inside* a poly body, self or otherwise, because
   the checker records no `CallInst` for such a call: the poly-body walk runs abstractly
   over rigid type variables, with no concrete `Subst` to put in a `CallInst`, so
   `instantiations.get(&span)` (`calls.rs:314`) misses at the self-call span regardless of
   which map the lowering pass is handed. On the native `sooth build` path the
   monomorphization loop in `lower` (the `for (symbol, inst) in distinct` loop,
   `src/ir/driver.rs:225-283`) calls `lower_word_parts` directly with the *real*
   `&module.instantiations` map (`driver.rs:262`), yet that map still holds no entry for a
   self-call span; only the REPL-only helper `lower_instantiation` (`src/ir/driver.rs:770-799`,
   sole caller `src/repl.rs:1439`) passes `empty_instantiations()`. `env`, the module-wide
   symbol→arity map every `Term::Call` ordinarily resolves against, explicitly excludes
   `poly_indices` (`driver.rs:110-124`), so a bare self-name lookup fails there as well.
   This is not just an oversight to patch: a self-call's correct callee is *whichever
   instantiation is currently being lowered*, which is a different concrete symbol/effect
   each time the loop runs (`loopg` at `i64`, `loopg` at `bool`, ...). A single `CallInst`
   recorded once at check time, the mechanism every other poly-callee call site uses,
   cannot represent "recurse into whatever `Subst` this particular emission is already
   running at" — so self-recursion needs a distinct lowering rule, not a `CallInst`/`env`
   entry.

7. **The needed lowering rule is already named, not yet built.** The comment inside the
   native monomorphization loop (`driver.rs:250-259`, immediately above its direct
   `lower_word_parts` call) states the intended shape
   directly: "a self-recursive polymorphic word is a nested polymorphic call ... so such a
   body still lowers correctly as an ordinary recursive call, just without the
   loop/back-edge transform a monomorphic self-tail word gets" — i.e., inside
   `lower_word_parts`'s call-lowering (`func_builder/calls.rs`), a `Term::Call(name)` whose
   `name` equals the bare/mangled name of the word currently being lowered (no `CallInst`
   at that span, since check never records one for a self-call) should emit an ordinary
   `Instr::Call(symbol, ...)` targeting *this instantiation's own* `symbol`, using the same
   `effect` this instantiation is already compiled against. `self_tail` stays hardcoded
   `false` at this call site regardless (item 9).

8. **The optional back-edge half is currently unconditionally disabled, not merely
   unimplemented.** `check_poly_body` builds its `ctx` via `word_ctx(word, ..., combs, ...)`
   with `combs` fixed to `&CombinatorIndex::new()` (an empty index) at every call site
   (`poly.rs:392`, comment: "`poly_walk` never reaches the concrete back-edge guard (R15)
   ... an empty index is correct here, not just convenient — lowering never back-edges a
   polymorphic instantiation either"). `has_self_tail_call(word, combs)` therefore always
   returns `false` for a poly body today, and `Ctx::Word.self_tail_call` is always `false`.
   Lifting this (roadmap's optional second piece) needs `has_self_tail_call`'s tail-position
   detection to actually run over a poly body, plus a new lowering back-edge case — real
   work, correctly scoped as optional; without it the feature is correct but consumes a
   stack frame per recursion level, exactly as the roadmap already says.

9. **The stated termination hazard may not be reachable through this mechanism at all —
   this is a design decision to lock, not an open question to defer.** The roadmap worries
   about a self-call recursing at a *different* type argument (`'T` recursing at `['T 2]`),
   which would demand a fresh instantiation per level and never terminate under
   monomorphizing codegen. But finding 5 fixes the check as a **pure structural match
   against the existing `sig`** — no unification, no fresh `Subst`. Under that design, an
   operand shaped `PolyType::Array(Box::new(PolyType::Var(0)), len)` (representable today,
   per `ast.rs:1374` and P7.S3a's own note that "an array carrying a type variable in a
   poly signature already works") does not structurally equal `PolyType::Var(0)` — it is
   simply an ordinary declared-type mismatch against `sig.inputs[i]`, rejected the same way
   any other operand/signature mismatch is, never treated as a request for a new
   instantiation. **If the self-call check never grounds/unifies — only structurally
   matches — polymorphic recursion at a different type argument is not spellable through
   bare self-call syntax and needs no separate termination guard.** That must be locked
   explicitly (a design decision, stated in the spec, not left implicit): this slice
   implements structural self-call only, and declines any grounding/re-instantiation
   variant that would reopen the termination question.

## Locked decisions carried forward

**No unification, no fresh instantiation (finding 5).** The self-call check compares the
operand window to the walking word's own `sig.inputs` pointwise, using the exact same
rigid type-variable ids; it never derives or unifies against a new `Subst`. This is what
makes finding 9's termination argument hold — a different design (unify-and-re-instantiate)
would reopen the polymorphic-recursion hazard and is explicitly declined here.

**Splice/row machinery is untouched.** This is not a quotation consumer; nothing here
interacts with P7.S3b/S3b-follow/S3d's quotation-literal or row machinery. `if`/`times`
combinators may appear around the self-call (as in the probe above) exactly as they do
around any other call in a non-inline poly body.

**Ordinary recursive call, no loop transform by default (findings 6-8).** The lowering fix
is scoped to "call the current instantiation's own symbol" only; the back-edge/loop
optimization is a separately deferrable piece within the same slice, not a prerequisite for
correctness.

## Open questions

1. **Where exactly does the self-name comparison belong relative to the existing dispatch
   order in `poly_call_term`?** Finding 1's fall-through is the last resort after locals,
   intrinsics, `&`/`@`/`!`, shuffles, the env-dispatch loop, and `poly_delegate_op` have all
   declined — all of which are keyed on the operand *shape*, not the callee name, so none of
   them should ever misfire on a self-call's name. Worth a one-line confirmation in the spec
   that nothing between the top of `poly_call_term` and the current fall-through can
   spuriously claim a bare word name like `loopg`, rather than assuming it.
2. **Is the back-edge/loop transform (finding 8, roadmap's "optional" piece) in scope for
   this slice's exit, or a follow-up?** The roadmap text files it under the same slice
   ("without the second the feature is correct but consumes stack") but calls it optional;
   the spec should decide up front rather than let it become scope creep mid-implementation.
3. **Does a self-call ever need to appear in `module.instantiations` for anything downstream
   of lowering (debug info, an eventual profiler, dead-code elimination over
   instantiations)?** Not traced here; worth a quick grep of `instantiations` consumers
   beyond `lower_word_parts` before assuming "lowering-only, no data recorded" is safe.

## Out of scope

- Calling a *different* polymorphic word (P7.S3k) — no interaction traced; S3k's own
  recon already distinguishes the two ("harder than S3g in one specific way: the callee's
  own type variables are not the caller's" — a self-call reuses `sig` unchanged).
- Any grounding/re-instantiation design for the self-call site — declined per finding 9.
- Trait bounds (P7.S3e) — no interaction traced.
- Quotation/row machinery (P7.S3b, S3b-follow, S3d) — untouched; a self-call may sit
  inside an `if`/`times` arm (as the probe does) with no special interaction.

## The golden

The probe program above (`loopg`, a non-inline generic word with an `Ord`-free `Copy`
bound, self-recursing through an `if` combinator down to a base case) compiling and
running to the correct result. Plus the negative: a self-call whose operand does not
structurally match `sig.inputs` (e.g., passing an array of `'T` where `'T` itself is
declared) is a located type-mismatch error, not an infinite check-time loop or a backend
panic — the concrete evidence for finding 9's termination argument. And a regression: a
call to a **different** polymorphic word still produces `poly_calls_poly_word_error`
unchanged (S3k's gap, not touched by this slice).

## Ready to spec?

**Yes.** The checker-side mechanism (findings 1, 3, 4, 5) is small and precisely located:
one new arm ahead of the existing fall-through, comparing `name` to `ctx.mangled_name()`,
no new plumbing, no unification. The lowering-side mechanism (findings 6-7) is exactly
named by an existing code comment, not invented here. The termination hazard the roadmap
raises is resolved by design choice (finding 9), not deferred. Sizing: **S**, with the
optional back-edge piece (finding 8, open question 2) as the one scope call the spec should
make explicitly rather than let drift.

**A separate, unrelated bug was found while probing this brief and should be filed on its
own**, not folded into S3g: a monomorphic caller's `check_poly_call` misreports a
cross-module poly callee's input arity (a 2-input `'T: Copy 'T i64 -- 'T` signature demanded
3 values once the callee moved to a second file). Worth a minimal standalone repro and its
own ticket before or independent of this slice; S3g's self-call mechanism does not touch
`check_poly_call` and is not blocked by it, since a self-call's own signature comparison
never goes through that path (finding 5).
