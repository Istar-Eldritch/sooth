# Phase 7 Slice 3g: self-recursion in a non-inline generic body (spec)

## Goal

A **non-inline** polymorphic word cannot call itself. Recursion is the ordinary way to
write a loop over an inductively-shaped value, so a generic word that wants to recurse is
forced to be `inline` (spliced per call site) instead of monomorphized once per type. This
slice closes the **self-call** case only: a `Term::Call` inside a poly word's own body whose
name is that very word.

Probe-verified at HEAD:

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

**Correction, found at implementation: that probe program is off by one operand slot, and
only reaches the diagnostic above because the self-call is rejected before the arm shapes
are compared.** A signature's *bound site* is itself an input, so
`( 'T: Copy 'T i64 -- 'T )` declares **three** inputs (`'T`, `'T`, `i64`), which `5 3 loopg`
does not supply and whose `~[ drop ]` arm leaves `'T 'T` against a declared `'T`. The
shipped golden below carries the corrected signature, `( 'T: Copy i64 -- 'T )`. Nothing
about the diagnostic, the gap, or the delivered shape changes: the self-call error fires
during the arm walk, ahead of the cross-arm shape comparison, in both spellings.

The diagnostic is wrong on its face: `loopg` is not "another polymorphic word across a
module boundary," it is the word being checked. The message is `poly_calls_poly_word_error`
(P8.S2's generic-calls-generic diagnostic), and self-recursion is the one instance of that
gap that needs no cross-word registry lookup at all: the callee's signature *is* the `sig`
the walk already holds.

**This slice delivers exactly the self-call.** Calling a *different* polymorphic word stays
`poly_calls_poly_word_error`, unchanged — that is P7.S3k's gap, not this slice's.

## Recon (verified against the source at HEAD, carried from the brief and re-checked)

The brief (`docs/roadmap/P7/slice3g-brief.md`) is the authoritative recon. The load-bearing
anchors, re-confirmed against the source at HEAD:

1. **The fall-through does not distinguish self from other.** `poly_call_term`
   (`src/check/poly.rs`, the `if poly_words.contains(name) { return
   Err(poly_calls_poly_word_error(ctx, span, name)) }` block, immediately after
   `poly_delegate_op` declines and before `unknown_word_error`). `poly_words` is every poly
   word's name in the module (`check.rs`, `poly_env.keys().cloned().collect()`), built
   *before* any body is walked, so the currently-walked word's own name is always a member
   of its own `poly_words` set. Nothing ahead of this block checks for a self-name.

2. **`PolySig` carries no name** (`src/ast.rs`: `row_in`, `inputs`, `outputs`, `row_out`,
   `bounds`, `ty_var_names`, `len_var_names`, `row_var_names` — no `name` field). So `sig`
   alone cannot answer "is this a self-call"; the name must come from `ctx`.

3. **`ctx` already carries it.** `Ctx::Word` stores `name` (demangled) and `mangled`
   (`src/check/engine.rs`, `word_ctx`), with accessors `word_name()` and `mangled_name()`
   (`engine.rs`), both already used by the concrete checker to recognize a self-tail-call
   back-edge. `poly_call_term` already takes `ctx: &Ctx`, so the comparison needs no new
   plumbing through `check_poly_body`/`poly_walk`.

4. **`ctx.mangled_name()` is the correct comparand, not `ctx.word_name()`.** `resolve::mangle`
   (`src/resolve.rs`) is unconditional over every name except `main`/`drop`, and rewrites
   every body reference to match. A self-call term inside `loopg`'s body is a body reference
   to `loopg`, so in a multi-module closure it is rewritten to the same `loopg__m1` spelling
   `word.name` (hence `ctx.mangled_name()`) already carries; in a single-module closure the
   pass is a byte-for-byte no-op, so the two coincide there too. `ctx.word_name()` (the
   demangled display spelling) would falsely fail to match in the multi-module case. This is
   the brief's finding 4, and it is why R1 compares against `ctx.mangled_name()`.

5. **A self-call needs no unification or `Subst` at check time.** `sig` *is* the callee's
   signature, with the exact same rigid type-variable ids the current walk is using — there
   is no second, independent instantiation to unify against. The correct check is a pointwise
   structural match of the operand window against `sig.inputs`, producing `sig.outputs` on
   success: the same comparison `check_poly_body` already performs between the body's residual
   stack and `sig.outputs` at exit (`poly.rs`, `residual_pt != sig.outputs`), run mid-body
   instead of at the end. No `PolyType::Var` grounding, no `apply_subst`, no `GenericTypes`
   mint.

6. **Lowering's general poly-callee mechanism does not fit here.** A call to a poly word from
   a monomorphic (or, once S3k lands, another poly) caller resolves through `instantiations:
   &HashMap<Span, CallInst>`, keyed by the call site's span (`src/ir/func_builder/calls.rs`).
   That table is never populated for any call made *inside* a poly body, self or otherwise,
   because the checker records no `CallInst` for such a call: the poly-body walk runs
   abstractly over rigid type variables, with no concrete `Subst` to record, so
   `instantiations.get(&span)` (`calls.rs:314`) misses at the self-call span regardless of
   which map lowering is handed. On the native `sooth build` path the monomorphization loop
   in `lower` (the `for (symbol, inst) in distinct` loop, `driver.rs:225-283`) calls
   `lower_word_parts` directly with the *real* `&module.instantiations` map (`driver.rs:262`),
   and that map still holds no entry for a self-call span; only the REPL-only helper
   `lower_instantiation` (`src/ir/driver.rs:770-799`, sole caller `src/repl.rs:1439`) passes
   `empty_instantiations()`. `env` (the module-wide symbol→arity map every `Term::Call`
   ordinarily resolves against) explicitly excludes `poly_indices` too (`driver.rs:110-124`),
   so a bare self-name lookup fails there as well — `self.env.get(name).expect("checked user
   word exists")` (`calls.rs:685`) in the user-word dispatch would panic on a poly self-name.
   A self-call's correct callee is *whichever instantiation is currently being lowered* (a
   different concrete symbol/effect each time the loop runs), which a single `CallInst`
   recorded once at check time cannot represent. So self-recursion needs a distinct lowering
   rule, not a `CallInst`/`env` entry.

7. **The lowering rule is already named.** The comment inside the native monomorphization
   loop (`src/ir/driver.rs:250-259`, immediately above its direct `lower_word_parts` call)
   states the shape directly: "a self-recursive polymorphic word
   is a nested polymorphic call ... so such a body still lowers correctly as an ordinary
   recursive call, just without the loop/back-edge transform a monomorphic self-tail word
   gets." Inside `lower_word_parts`'s call-lowering (`func_builder/calls.rs`), a
   `Term::Call(name)` whose `name` equals the poly word currently being lowered
   (`inst.callee`, e.g. `loopg__m0`) with no `CallInst` at that span should emit an ordinary
   `Instr::Call(ret, symbol, args)` targeting *this instantiation's own* `symbol`
   (`self.cur_word_name`, the mangled instantiation symbol), using the `effect` already
   computed for this instantiation. `self_tail` stays hardcoded `false` at this call site
   regardless.

8. **The back-edge/loop half is unconditionally disabled today.** `check_poly_body` builds
   its `ctx` with `combs` fixed to `&CombinatorIndex::new()` (`poly.rs`), so
   `has_self_tail_call` always returns `false` for a poly body and `Ctx::Word.self_tail_call`
   is always `false`. On the lowering side the monomorphization loop passes `self_tail: false`
   into `lower_word_parts` (`driver.rs`), so no header/phi loop is built. Lifting this (making
   a self-tail-recursive poly call lower to a loop instead of real recursion) needs
   `has_self_tail_call`'s tail-position detection to run over a poly body plus a new lowering
   back-edge case. It is optional per the roadmap; without it the feature is correct but
   consumes a stack frame per recursion level. **This slice's phase-scoping decision on it is
   in D3 below.**

## Locked design decisions

### D1 Structural self-call only — no unification, no fresh instantiation (brief finding 5, finding 9)

The self-call check compares the operand window to the walking word's own `sig.inputs`
**pointwise**, using the exact same rigid type-variable ids the walk already holds; it
produces `sig.outputs` on success. It never derives, grounds, or unifies against a new
`Subst`, and never mints a `GenericTypes` entry.

**This is what makes the roadmap's termination hazard unreachable, and the spec states it
explicitly rather than adding a separate termination guard.** The roadmap worries about a
self-call recursing at a *different* type argument (`'T` recursing at `['T 2]`), which under
monomorphizing codegen would demand a fresh instantiation per level and never terminate. Under
D1's pure structural match, an operand shaped `PolyType::Array(Box::new(PolyType::Var(0)),
len)` (representable today) does **not** structurally equal `PolyType::Var(0)` — it is an
ordinary declared-type mismatch against `sig.inputs[i]`, rejected exactly as any other
operand/signature mismatch is, never treated as a request for a new instantiation. **Because
the self-call check never grounds or unifies, polymorphic recursion at a different type
argument is not spellable through bare self-call syntax and needs no termination guard.** Any
grounding/re-instantiation variant that would reopen the termination question is explicitly
declined here.

### D2 Splice/row machinery untouched

This is not a quotation consumer. Nothing here interacts with P7.S3b/S3b-follow/S3d's
quotation-literal or row machinery. `if`/`times` combinators may sit around the self-call (as
in the probe) exactly as they do around any other call in a non-inline poly body; the
self-call is an ordinary `Term::Call` reached through `poly_call_term`, and the surrounding
combinator arm-walk is unchanged.

### D3 Ordinary recursive call only — the back-edge/loop transform is deferred (brief OQ2)

The lowering fix is scoped to "call the current instantiation's own symbol" as an ordinary
`Instr::Call`. **The back-edge/loop transform (brief finding 8, the roadmap's optional second
piece) is explicitly deferred out of this slice, not shipped here.** Rationale: it is a
distinct, larger piece of work (tail-position detection over a poly body plus a new lowering
back-edge case), the roadmap already marks it optional, and without it the feature is *correct*
— a self-recursive poly word runs to the right result — merely consuming one stack frame per
recursion level, exactly as the roadmap states. The correctness deliverable does not depend on
it. This is a decision, not a drift: the slice ships the ordinary recursive call and files the
loop transform as a named follow-up (see Exit findings).

## Delivered shape

### R1 Checker: a self-call arm in `poly_call_term`

Add one arm ahead of the `if poly_words.contains(name)` fall-through (recon 1):

- **Guard:** `ctx.mangled_name() == Some(name)` (recon 4 — mangled, never demangled).
- **Check:** a pure structural pointwise match of the top-of-stack operand window against the
  walking word's own `sig.inputs` (D1). Let `n = sig.inputs.len()`; if `stack.len() < n`,
  reuse the existing poly-underflow diagnostic (the same one an ordinary operand-arity
  shortfall produces — the self-call is an ordinary call for arity purposes). Otherwise, with
  `base = stack.len() - n`, compare each `stack[base + i]` slot's `PolyType` to `sig.inputs[i]`
  structurally, the same comparison `check_poly_body` runs at exit (`residual_pt !=
  sig.outputs`). On the first per-slot mismatch, emit the existing located renderer
  `poly_rendered_type_mismatch_error` (`src/check/poly.rs:3580`), passing the self-call `name`
  as its `op` and the rendered expected/found slots as its `expected`/`found`
  (`poly_type_str(&sig.inputs[i], sig)` and `poly_type_str(&stack[base + i].pt, sig)`); it
  renders a located `type mismatch` message naming the enclosing word, the line, the self-call,
  and its expected-vs-found operand type (not a panic, not an infinite loop, see the golden's
  negative). On a full match, `stack.truncate(base)` and push `sig.outputs`, then
  `return Ok(stack)`.
- **No `Subst`, no grounding, no `GenericTypes` mint** (D1). The rigid type-variable ids in
  `sig.inputs`/`sig.outputs` are carried through unchanged.

**OQ1 confirmation (dispatch order — brief open question 1).** The new arm sits after locals,
intrinsics, `&`/`@`/`!`, shuffles, `poly_construct_generic`, the `env`-dispatch loop, and
`poly_delegate_op` have all declined, and immediately before the `poly_words.contains(name)`
fall-through. Every one of those earlier paths is keyed on the operand *shape* or on a name
resolvable through `env`/intrinsics, none of which a bare poly self-name like `loopg` resolves
through (recon 1: poly words are not in `env`). Confirm at implementation that nothing between
the top of `poly_call_term` and this fall-through can spuriously claim a bare word name — a
one-line note in the exit findings, verified by probe, not assumed. Placing the arm at the
fall-through (rather than at the top) keeps it from shadowing any operand-shape-keyed path that
might legitimately handle a self-name-shaped operator, and matches where the wrong error is
returned today.

### R2 Lowering: a self-call arm in `lower_word_parts`'s call-lowering

Recon 6-7. Two edits:

- **Thread the poly word's own name into the instantiation lowering.** `lower_word_parts`
  currently receives only `symbol` (the mangled instantiation symbol, which it stores as
  `cur_word_name`). It needs the poly word's bare name (`inst.callee`, e.g. `loopg__m0`) to
  recognize a self-call `Term::Call(inst.callee)` in the body. Add it as one new parameter,
  threaded from the monomorphization loop in `driver.rs` (which already holds `inst.callee`)
  and from `lower_instantiation`. Store it on the `FuncBuilder` (working name
  `cur_poly_callee: Option<String>`, `None` for an ordinary monomorphic word so the existing
  self-tail-combinator/whole-word back-edge paths keyed on `cur_word_name` are untouched).
- **Add the self-call arm in the user-word dispatch (`func_builder/calls.rs`), before the
  `self.env.get(name).expect(...)` lookup that would otherwise panic on a poly self-name.**
  When `self.cur_poly_callee.as_deref() == Some(name)` and there is no `CallInst` at this span
  (check never records one for a self-call), emit an ordinary `Instr::Call(ret, symbol, args)`
  targeting `self.cur_word_name` (this instantiation's own symbol), using the arity and output
  shape from the current `effect` (available on the builder as `cur_outputs`/the declared
  effect, not from `env`, which excludes the poly word). Materialize any phantom quotation args
  exactly as the ordinary dispatch does (R-D3). `self_tail` stays `false` (D3) — this is not a
  loop back-edge; the whole-word back-edge path keyed on `cur_word_name` is not reached, since
  `name` is the poly callee's name, not the instantiation symbol.

**REPL path scope.** R2 threads `cur_poly_callee` through *both* real `lower_word_parts` call
sites for a poly body: the native monomorphization loop and the REPL-only `lower_instantiation`
(recon 6). A self-recursive poly word *declared at the REPL* is therefore delivered by the same
mechanism as a native-built one, with no separate REPL-specific code; it is not separately
covered by a REPL golden in this slice (the goldens exercise the native `sooth build` path
only). No REPL path is excluded here.

**No new lowering data recorded (brief OQ3).** A self-call records no `CallInst` and adds no
`instantiations` entry. Consumers of `module.instantiations` beyond `lower_word_parts` — the
REPL's `emit_instantiations` (`src/repl.rs`) and the checker's own recording — all key on a
call site producing a *new concrete instantiation*; a self-call produces none (it reuses the
instantiation being lowered), so there is nothing for them to consume. Confirm this at exit
with a grep of `instantiations` consumers rather than asserting it.

### R3 The different-poly-word path is untouched

A `Term::Call` to a *different* poly word (`ctx.mangled_name() != Some(name)` but
`poly_words.contains(name)`) still hits the fall-through and still returns
`poly_calls_poly_word_error`, unchanged. R1's arm is guarded strictly by the self-name equality,
so it never intercepts a call to another poly word. This is P7.S3k's gap; this slice neither
closes it nor changes its diagnostic.

## Out of scope

- **Calling a *different* polymorphic word (P7.S3k).** A self-call reuses `sig` unchanged;
  a different callee's own type variables are not the caller's, needs the `poly_env` registry
  `poly_call_term` cannot see, and is explicitly harder. No interaction traced.
- **The back-edge/loop transform (D3).** Deferred by decision, filed as a follow-up.
- **Any grounding/re-instantiation design for the self-call site (D1).** Declined.
- **Trait bounds (P7.S3e).** No interaction traced.
- **Quotation/row machinery (P7.S3b, S3b-follow, S3d) (D2).** Untouched; a self-call may sit
  inside an `if`/`times` arm (as the probe does) with no special interaction.

## Incidental finding, filed separately (does not block this slice)

While probing the multi-module case (brief finding 4), a two-file build hit an **unrelated,
pre-existing** anomaly: a *monomorphic caller's* `check_poly_call` misreports a cross-module
poly callee's input arity (a 2-input `'T: Copy 'T i64 -- 'T` signature was demanded as needing
3 values once the callee moved to a second file). This is orthogonal to S3g and worth its own
standalone repro and ticket. **S3g's self-call mechanism does not touch `check_poly_call`** —
a self-call's signature comparison is a structural match against the walk's own `sig` inside
`poly_call_term` (D1, R1), never the concrete-caller-calls-poly-callee path where that bug
lives — so the anomaly does not block this slice. The single-module golden below does not go
through `check_poly_call` at all.

## The golden (a test fixture, not a `lib/` word)

`tests/phase7_slice3g.rs`, complete `.sth` sources (not fragments — an abbreviated sketch
cannot be probed for the exact rejection a deleted arm produces).

- **Behavioural (`self_recursive_poly_word_runs_to_base_case`)** — the brief's probe program
  at the corrected arity: a non-inline generic `loopg` with an `Ord`-free `Copy` bound,
  self-recursing through an `if` combinator down to a base case. The harness appends the
  `import:` lines the fixture's own text implies (`tests/common`), so the source is:

  ```sooth
  : iszero ( i64 -- bool ) 0 eq ;

  : loopg ( 'T: Copy i64 -- 'T )
    dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;

  : main ( -- )
    5 3 loopg .
    true 2 loopg .
  ;
  ```

  Compiles and runs, printing `3 2 1 5 2 1 true`. Run at a **second instantiation**
  (`'T = bool`) so `'T` is carried rigidly rather than coincidentally matching the only
  instantiation there is. The recursive arm prints the counter *before* it recurses, so the
  number of recursion levels is observable in stdout: a self-call lowered to the wrong callee,
  or a loop that ran the wrong number of times, changes the transcript rather than merely the
  build. **Mutation guards:** deleting R1's self-call arm makes the fixture fail to compile with
  `poly_calls_poly_word_error` (the wrong pre-slice diagnostic returns); deleting R2's self-call
  lowering arm makes it die at lowering on the poly self-name `env` has no entry for; retargeting
  R2's arm to `(self.resolve)(name)` instead of `cur_word_name` makes it fail to link.

- **Negative — structural mismatch is a located type error, not a loop or panic
  (`self_call_operand_mismatch_is_located_type_error`)** — a self-call whose operand window
  does not structurally match `sig.inputs` (D1's termination witness). This is the concrete
  evidence for D1: the compiler produces a **located** diagnostic naming the self-call site,
  never an infinite check-time loop and never a backend panic. Assert on
  `poly_rendered_type_mismatch_error`'s rendered shape (R1): the located `type mismatch` line
  naming the enclosing word and line number, plus the `expected`/`found` operand types the
  fixture forces, rather than a bare "build fails" assertion (which passes identically against
  an unrelated fallthrough).

- **Regression — a different poly word still rejects unchanged
  (`different_poly_word_call_still_names_the_narrowing`)** — a non-inline generic word calling
  a *second, different* generic word still produces `poly_calls_poly_word_error` with its
  current wording (R3). This is P7.S3k's gap, and this slice must not perturb it. Assert a
  stable substring (`err.contains("cannot call the polymorphic word")` and
  `err.contains("a polymorphic word is not yet reachable from another polymorphic word across a
  module boundary")`, the `tests/phase7_slice3d.rs` `err.contains(...)` pattern), not the full
  located string — `poly_calls_poly_word_error` interpolates `(line {}, col {})`, which differs
  per fixture — so a later change that widens R1's guard fails loudly without a brittle line/col
  baked in.

## Testing

Unit tests beside the stage code (`src/check/poly.rs`, `#[cfg(test)] mod tests`, CLAUDE.md
naming `thing_condition_expected`):

- `poly_self_call_structural_match_produces_outputs` — the operand window matching
  `sig.inputs` truncates and pushes `sig.outputs`.
- `poly_self_call_operand_mismatch_is_located_error` — a per-slot mismatch renders through
  `poly_rendered_type_mismatch_error` (a located diagnostic, not a panic); assert its shape,
  not a bare failure.
- `poly_self_call_underflow_reuses_arity_error` — too few operands surface the ordinary
  poly-underflow diagnostic.
- `poly_self_call_uses_mangled_name_not_demangled` — the guard matches `ctx.mangled_name()`;
  pin that a demangled-name comparison would miss the multi-module spelling (finding 4), so a
  later refactor to `word_name()` fails.
- `poly_different_word_call_still_rejects` — R3: a different poly-word name still hits the
  fall-through and `poly_calls_poly_word_error`.

Unit test beside the lowering code (`src/ir/func_builder/`, or the `driver.rs` lowering tests
`mod`): `poly_self_call_lowers_to_ordinary_recursive_call` — a self-call in an instantiation
body emits an `Instr::Call` to the instantiation's own symbol (not a back-edge, `self_tail`
false), and does not consult `env` for the poly self-name.

Goldens (`tests/phase7_slice3g.rs`): the three fixtures above, each behavioural assertion
running the binary, each negative/regression asserting the **exact message text**.

Mutation-tested guards (delete/flip the guarded code, watch the named test fail, then restore
to a clean `git status`; commit before mutation testing; a mutation copy needs `examples/`;
touch sources after any rollback; end each cycle on a clean `git status`):

- R1's self-call arm (deletion → the behavioural golden fails to compile with
  `poly_calls_poly_word_error`, the wrong pre-slice diagnostic).
- R1's `ctx.mangled_name()` guard (flip to `word_name()` → the mangled-name unit test fails;
  build a two-module probe if a single-module test cannot discriminate them, since the pass is
  a no-op in a single module — see finding 4).
- R1's structural per-slot match (weaken it to accept any operand → the mismatch negative
  starts compiling / loops / panics; assert it stays a located rejection).
- R2's self-call lowering arm (deletion → the behavioural golden fails at lowering, not runs).
- R3 is guarded by the different-poly-word regression golden (widening R1's guard to match a
  non-self poly name → that golden's message changes).

Regression, green and untouched:
`tests/phase7_slice3b.rs`, `tests/phase7_slice3b_follow.rs`, `tests/phase7_slice3d.rs`,
`tests/phase7_slice3a.rs`, the `tests/phase6_*` eliminator suites, `tests/qbe_baseline.rs`,
and `tests/phase8_slice2.rs` (which pins the `poly_calls_poly_word_error` narrowing R3 must
leave intact). Green is `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Exit findings (confirmed at implementation)

**OQ1 dispatch-order confirmation — one earlier path does claim a bare self-name, and it is
pre-existing.** Probed by declaring a poly word under each name an earlier arm of
`poly_call_term` could plausibly claim (`dup`, `swap`, `over`, `nip`, `rot`, `eq`, `call`,
`add`), self-calling it, and building with R1's arm enabled and again disabled. Seven of the
eight reach R1: with the arm disabled each falls straight through to
`poly_calls_poly_word_error`, so no local, intrinsic gate, `&`/`@`/`!`, shuffle, `env`-dispatch
or `poly_delegate_op` path claimed the name. The reason is `resolve::mangle`: a module
declaring `dup` has both the declaration *and* every body reference rewritten to `dup__m0`,
which matches none of the earlier arms' bare-name keys. The exception is `add`. An **operator**
name escapes mangling (the single-module operator-dispatch carve-out), so `poly_delegate_op`
claims it and reports a `stack effect mismatch` naming `add`, measured against the concrete
suffix of the operand window — identically with R1 enabled and disabled, so the slice
neither causes nor worsens it. A polymorphic word named after an operator therefore cannot
self-call; that belongs with the operator-mangling carve-out, not here.

**OQ3 instantiations consumers — a self-call recording nothing is safe for every one.**
The consumers of `Module::instantiations`, grepped:

- `src/check.rs:902` (`module.instantiations = insts`) — the checker's own recording.
  `poly_call_term`'s self-call arm writes no `CallInst`, so the map is unchanged.
- `src/ir/driver.rs:236` (the monomorphization loop) — one `IrFunc` per distinct recorded
  instantiation, deduped by symbol. A self-call reuses the instantiation being lowered and
  produces no new one, so there is nothing to emit.
- `src/ir/func_builder/calls.rs:314` (`self.instantiations.get(&span)`) — the per-call-site
  lookup. It misses at a self-call span by construction; R2's arm sits immediately after it.
- `src/repl.rs:1399` `emit_instantiations` (called at `3042`, `3241`) — emits one
  monomorphized func per pending `CallInst`, keyed on symbol. A self-call adds no `CallInst`,
  so nothing becomes pending.
- `src/resolve.rs:1565`, `src/driver.rs:519`, `src/ast.rs:2381`/`2505` construct empty maps.
  `src/ir/layout.rs:597` and `src/check.rs:2239`/`2294` say "instantiations" about
  `GenericTypes` (generic *type* instantiations), a different registry this slice never
  touches.

Every consumer keys on a call site producing a **new concrete instantiation**; a self-call
produces none.

**D3 back-edge/loop transform, deferred by decision, filed as `P7.S3g-follow`.**
The follow-up entry is in `docs/roadmap/P7-language-prereqs.md`, immediately after S3g's, and
names both halves: tail-position detection over a poly body (`check_poly_body` builds its
`Ctx` with an empty `CombinatorIndex`, so `has_self_tail_call` returns `false` for every poly
body) and a lowering back-edge case beside R2's arm (the monomorphization loop passes
`self_tail: false`). The cost left standing is stack depth only — one frame per recursion
level — not correctness.

**`poly.rs` split signals, re-run at exit: still defer, unchanged.** `poly.rs` stands at 7223
lines. This slice added one 36-line arm inside `poly_call_term` and five unit tests; it pulled
in no new dependency (the arm uses `sig`, `ctx`, `poly_type_str` and
`poly_rendered_type_mismatch_error`, all already local), added no new responsibility, and
introduced no would-be circular dependency. So the signal count is exactly what S3d's re-run
found — 3 of 5 firing, both available splits still wrong — and the standing
S3b/S3b-follow/S3d deferral carries over untouched. The next re-run trigger is unchanged: a
second quotation consumer, not this.

**Incidental: a leftover debug fixture from phase 1.** `tests/phase7_slice3g.rs` carried a
`tmp_probe_add` test whose body was a bare `panic!` on a probe's diagnostic. Deleted here; the
suite was red on that binary until it was.

## References

- `docs/roadmap/P7/slice3g-brief.md` (the probe-grounded brief; authoritative recon, findings
  1-9, and the separately-filed cross-module arity anomaly).
- `docs/roadmap/P7-language-prereqs.md` (the S3g roadmap entry; S3k's distinction between a
  self-call and a different-word call; S3d/S3b-follow scoping around this slice).
- `docs/roadmap/P7/slice3b-follow-spec.md` (the combinator arm-walk `loopg` sits inside; its
  golden explicitly loops with a literal `times` *because* self-recursion was blocked, naming
  S3g — this slice is what lets a self-recursive body reach that machinery).
- `docs/roadmap/P7/slice3d-spec.md` (format precedent).
- `src/ir/driver.rs:250-259` (the code comment already naming R2's ordinary-recursive-call
  shape).
