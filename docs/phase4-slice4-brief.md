# Phase 4 Slice 4: quotations + the internal loop primitive (brief)

Slice 1 gave `Sig` type/row/length variables and native monomorphization; slice 2
carried that to the REPL; slice 3 fixed the loop-carried aggregate copy. This slice adds
the one thing the whole iteration story rests on and that cannot be a library: a
quotation `[ ... ]` as a first-class code value, `call` to invoke it, and the internal
loop primitive a quotation compiles down to for constant-stack iteration. ROADMAP slice-4
frames it as "`[ ... ]` + `call`, plus the loop primitive they compile down to ... plus
call-site inlining", with capture downgraded by the pre-check to a quality-of-life
question that `fold`/`while` sidestep entirely.

That framing is mostly right, and the capture downgrade holds. What it undersells is how
much of the slice is decided by one absence: there is no type, anywhere in the compiler,
that can name a quotation. Everything else in the brief follows from that, including the
answer to the slice-4/slice-5 inliner contradiction, which turns out not to be a
contradiction but a boundary. And the paper pre-check inverts an expectation: the reason
you cannot write `fold`/`each`/`while` as ordinary polymorphic library words today is not
capture and not the type nesting, it is that the polymorphic-word path rejects `if` and
does not get the constant-stack loop transform. Both are measured below.

## Recon: what already exists (measured, not assumed)

**1. Quotations do not exist, at any layer, and `[`/`]` are type syntax only.** The
tokens lex (`src/lexer.rs:14-15,87-88`) but every parser use is in *type* position:
`parse_type_expr` (`src/parser.rs:1065`), `parse_array_type_expr` (`:1160`),
`parse_poly_slot`/`parse_poly_array` (`:884`/`:924`), `parse_field_type_expr` (`:1233`),
all reading `[i64 4]` / `['T 'N]`. There is no term-level `[`, so a bracket in a word
body is a hard parse error, verified:

```
: main ( -- ) [ 1 2 + ] ;
error: parse error: unexpected token LBracket at line 1, col 15
```

`TermKind` (`src/ast.rs:787`) is `IntLit`/`FloatLit`/`BoolLit`/`StrLit`/`Call`/`Bind`/`If`
with no quotation arm; `call` is not a builtin, an env word, or a token (grep for
`"call"` across `src/` is empty); and nothing matching `quot` exists in the source. This
slice adds new surface syntax *and* a new value kind, exactly as DESIGN.md:278 says it
must.

**2. No type at any layer can name a quotation, and this is the deepest finding.**
`Type` (`src/ast.rs:566`) is the twelve scalar/aggregate cases and carries the load-bearing
invariant that it stays `Copy` and self-renders without a registry; it has no code-value
variant. `PolyType` (`src/ast.rs:406`) is exactly `Concrete(Type)` / `Var(u32)` /
`Array(Box<PolyType>, Len)`. `Subst` (`src/ast.rs:443`) maps a variable id to a `Type` or
a `u32` length and *only* to those. `IrType` (`src/ir.rs:76`) has no code value either.
So a quotation value with its own stack effect is unrepresentable end to end.

   The specific wall is nesting. A combinator signature needs a quotation's own effect to
   sit inside a slot of the outer signature, e.g. `each ( ['a 'N] [ 'a -- ] -- )`, where
   the inner `[ 'a -- ]` is itself an effect, itself polymorphic in `'a`, and shares `'a`
   with the outer signature. Nothing in `PolyType` can hold an effect, and the variable id
   spaces are flat per-signature `u32` tables (`PolySig::ty_var_names`/`len_var_names`/
   `row_var_names`, `src/ast.rs:417-441`) with no notion of an inner scope. Adding a
   quotation type touches: `Type` or `PolyType` (a new variant), `IrType`, `unify_poly_input`
   and `apply_subst` (`src/check.rs:3349`/`:3419`, which today recurse only through `Array`
   and would need an effect-against-effect arm), `Subst` and `instantiation_symbol`
   (`src/ast.rs:483`, the mangling would have to encode a nested effect deterministically),
   the monomorphization walk (`concrete_effect`, driven from `src/ir.rs:1176` onward),
   `is_copy`/`poly_is_copy` (`src/check.rs:2853`), the layout registry, and the backend.
   This is a slice-1-sized representation change, and slice 1's own lesson (D1: reopen the
   `Sig` representation once, not three times) applies with force. **The slice's central
   decision is whether it pays this cost or avoids it** (recon 9, decision 1).

**3. Even the cheap path costs a checker-stack change.** The checker's `Slot`
(`src/check.rs:64`) carries a `Type` plus provenance; it cannot hold a quotation. So even
an inline-only design, where a quotation is never a runtime value and exists only as a
compile-time marker flowing on the virtual stack until its `call` splices it, needs `Slot`
to grow a quotation case (an inferred effect plus the body to splice) or needs quotations
handled off the `Slot` stack entirely. The precedent to copy is the reference-escape
machinery: the checker already tracks a value that must not escape its scope and rejects
the escape with a located diagnostic (`check_reference_across_back_edge`, `src/check.rs:4041`,
preserved verbatim by slice 3). A non-escaping quotation is the same shape of obligation.

**4. Slice 1 did close the multi-output lowering hole. Verified, not trusted.**

```
: pair ( i64 -- i64 i64 ) dup ;
: main ( -- ) 5 pair . . ;          -> 5 5
: trip ( i64 -- i64 i64 i64 ) dup dup ;
: main ( -- ) 7 trip . . . ;        -> 7 7 7
: keep   ( ..s 'T -- ..s 'T ) ;     : main ( -- ) 1 2 keep . . ;    -> 2 1
: dropit ( ..s 'T -- ..s )   drop ; : main ( -- ) 1 2 3 dropit . . ; -> 2 1
```

All four compile and run correctly against the built compiler. `..s` in output position
works, so a combinator whose result count depends on its quotation (a `..s` passthrough)
has a lowering path. The mechanism is the synthesized bundle: `lower_call` (`src/ir.rs:2450`
onward) makes a multi-output callee return one bundle struct and unpacks it onto the
stack, discriminated by the layout's `bundle` flag, not by `out_arity >= 2`. This is the
machinery a combinator's multi-output signature rides, and it is load-bearing already.

**5. The internal loop primitive is the existing back-edge machinery, reachable only
through a syntactic self-call today.** `begin_loop`/`finalize_loop` (`src/ir.rs:2301`/`:2348`)
build a header block of carried slots and back-patch the back-edges, and slice 3 just
reworked them to stage carried aggregates through entry-hoisted stable slots with a
read-before-write blit (`stage_aggregates`, `CarriedSlot`, `src/ir.rs:2301-2395`). A
monomorphic self-tail loop runs in constant stack, verified:

```
: countdown ( i64 i64 -- i64 ) | x n | n 0 = if x else x n 1 - countdown end ;
: main ( -- ) 42 50000000 countdown . ;   -> 42, exit 0 (no overflow)
```

But the *trigger* is a plain-name self-`Call` in tail position: `has_self_tail_call`
(`src/check.rs:2130`) recognizes only `callee == word.name`, and `lower_call` back-edges
only when `tail && self.header.is_some() && name == self.cur_word_name` (`src/ir.rs:2670`).
A quotation loop has no self-call: it is `[ body ] <combinator>`. So the loop primitive a
quotation compiles to reuses the IR back-edge machinery and slice 3's carried-slot staging
unchanged, but needs a **new front-end trigger** into `begin_loop`/`finalize_loop` from a
combinator/quotation site, not the self-call recognizer. The constant-stack guarantee then
depends on the loop body not doing a per-iteration `alloc` bump (slice 3 recon 6: QBE
`alloc` emits inline with no hoisting), which is exactly the hazard slice 3's stable-slot
scheme already neutralizes for carried aggregates.

**6. Polymorphic self-tail recursion does NOT get the loop transform. Constant stack is
monomorphic-only today.** `self_tail` is hardcoded `false` on the polymorphic
instantiation path, with an explicit comment that a self-recursive polymorphic body
"still lowers correctly as an ordinary recursive call, just without the loop/back-edge
transform a monomorphic self-tail word gets" (`src/ir.rs:1176-1182`). So a polymorphic
self-tail word runs in linear stack. This is the first half of why the natural way to
write a combinator does not work.

**7. `if` in a polymorphic body is rejected outright. This is the second half.** Verified:

```
: countdown ( 'T i64 -- 'T ) | x n | n 0 = if x else x n 1 - countdown end ;
error: `if` in the polymorphic body of `countdown` (line 2) is not yet supported
```

The rejection is deliberate (`src/check.rs:2997-3008`): the monomorphic arm machinery
(condition-pop, per-arm unconsumed-linear check, move-join) is not lifted to the
`PolyType` stack, and a partial version would over-reject or leave the stack in a
state that panics later.

   **Recon 6 and 7 together are the pre-check inversion.** A combinator is polymorphic
   over its element/accumulator type by nature, and its body needs a loop-termination
   conditional and constant stack. Both are unavailable to a polymorphic Sooth word today:
   it cannot branch (`if` rejected) and it cannot loop in constant stack (self-tail not
   transformed). So `fold`/`each`/`while` **cannot be written as ordinary polymorphic
   library words at all** as the tree stands, independent of the quotation-type question
   and independent of capture. The constant-stack combinator has to arise either by
   inlining the combinator's body into a monomorphic call-site context, where `if` and the
   self-tail loop transform both already work, or from a compiler-known intrinsic loop
   combinator lowered directly to the back-edge machinery. This reframes the inliner (see
   recon 8): it is not a performance optimization layered on a working library, it is what
   makes the library expressible and constant-stack in the first place.

**8. The inliner contradiction is a boundary, not a contradiction.** ROADMAP slice-4 says
"plus call-site inlining"; slice-5 says "This slice owns the inliner, and it is the only
one ... a user `:` word is always a real call." Both hold once "inlining" is read as two
different mechanisms. Slice 4's call-site inlining is **quotation-literal fusion**:
splicing a literal quotation's body at its `call` (or at the intrinsic combinator that
consumes it), a local peephole that never crosses a word boundary and never touches a
`:` word. Slice 5's inliner is an **interprocedural user-word inliner**: inlining an
ordinary `:` library word (`each`/`fold`) and threading its quotation arguments, which is
what "a user `:` word is always a real call" is about, and there is genuinely no such pass
today (slice 1 recon 5, re-confirmed: everything called "inline" is a `lower_call` match
arm that never emits `Instr::Call`; a user word is always a real call). DESIGN.md:285-289
names the split directly: "a thin floor (one or two intrinsic combinators) bottoms out on
the loop primitive; the rest are pure library." Slice 4 owns the floor and the fusion;
slice 5 owns inlining the pure-library combinators built on it. What recon *cannot* settle
is where the floor is cut (decision 5, open question 1).

**9. Capture is free by construction in this slice, and the pre-check claim holds.**
`fold` and `while` thread loop state through the accumulator and the stack respectively, so
they capture nothing; confirmed by hand-writing them against the planned features. The
deeper reason capture is a non-issue here is that, since a quotation in this slice is
always fused or inlined (recon 8), its body is spliced into a context where any local it
names is simply still in lexical scope (an ordinary `| ... |` binding), so there is no
environment, no allocation, and no `FnOnce`/`FnMut`/`Fn` split. The `Copy`-locals
restriction the ROADMAP floats is only load-bearing when a captured local is read on every
iteration of a loop body that runs N times, which is the exact `@`-needs-`Copy` constraint
slice 1 already established for `each`'s element variable (slice-1 recon 6); it is not a
new mechanism. The real capture question belongs to **escaping** quotations (stored in an
array, returned, handed to a non-inlined word), which need the uniform-runtime-stack
fallback and Phase 6's alloc layer and are out of scope regardless (DESIGN.md:497,512).

## Why the obvious approach is dead

**Writing the combinator library as polymorphic Sooth words does not compile.** It is the
approach the phase's dogfood implies and it is dead on arrival: recon 6 and 7 mean a
polymorphic combinator body can neither branch nor loop in constant stack. So the slice
cannot deliver a runnable constant-stack combinator by "just write it in Sooth" the way
`vm.sth` delivered the Phase 2 verdict. Something compiler-side has to give: either the
combinator is inlined into a monomorphic context (which lifts both restrictions for free,
because the inlined body is checked and lowered as monomorphic code), or an intrinsic loop
combinator is compiler-known and lowered straight to the back-edge machinery.

**Making quotations first-class values to dodge that is the expensive path, and this
slice probably should not take it.** A first-class quotation type (recon 2) buys
non-inlined and escaping quotations, but escaping quotations are Phase 6 anyway, and
non-inlined combinators are the thing recon 6/7 say do not work yet regardless of the
type. So paying the slice-1-sized type-nesting cost now buys capability the rest of the
slice cannot use. The cheap path (recon 3: a compile-time-only quotation marker on the
checker stack, fused at `call`, never a runtime value) covers every quotation this slice
can actually run, at the cost of a `Slot` change and a non-escape check rather than a
`Type`/`IrType`/unification change.

## Decisions the spec has to make

1. **Runtime quotation type, or compile-time-only marker.** The recommendation is the
   marker (recon 2, 3, 9): a quotation is a compile-time stack entry carrying its inferred
   effect and its body, consumed by `call` or by a combinator via fusion/inlining, with a
   located non-escape diagnostic (modeled on `check_reference_across_back_edge`) when it
   would cross a branch merge, a back-edge, an array store, or a non-inlined word boundary.
   This defers the entire `Type`/`PolyType`/`IrType`/unification/mangling change (recon 2)
   to Phase 6 alongside the escaping-quotation fallback that actually needs it. The spec
   must state this explicitly, because taking the runtime type by reflex reopens the
   slice-1 representation at its most invasive for capability this slice cannot exercise.

2. **What `call` accepts.** With decision 1's marker, `call` typechecks only when the
   quotation's effect is statically known at that point, i.e. the quotation on the stack is
   traceable to a literal (directly, or forwarded through binds/shuffles without merging).
   A quotation whose identity is lost at a branch merge (`cond [ a ] [ b ] if call`), out
   of an array, or arriving as a non-inlined word parameter has no known effect and must be
   a located rejection, not a panic. The spec should name each unexpressible position and
   its diagnostic, since these are the exact positions Phase 6 later enables.

3. **The `call`-of-literal lowering (slice 4's "inlining").** A literal quotation reaching
   its `call` splices the body inline: `[ 1 + ] call` lowers as `1 +`, emitting no
   `Instr::Call` and creating no runtime code value. This is a term-level fusion in
   lowering, parallel to how builtins and struct words already lower as `lower_call` arms.
   The spec should fix that this is the *only* inlining slice 4 owns, and that it never
   crosses a `:` word boundary (that is slice 5, recon 8).

4. **The internal loop primitive's trigger and reuse.** The IR back-edge machinery
   (`begin_loop`/`finalize_loop` plus slice 3's carried-slot staging) is reused unchanged;
   what is new is the front-end path that drives it from a quotation loop rather than a
   self-`Call` (recon 5). The spec must say what that path is: an intrinsic loop combinator
   (a compiler-known word, e.g. an intrinsic `times`/`each`-floor, that consumes a fused
   quotation and lowers to a back-edge loop), and it must state that the constant-stack
   guarantee rides slice 3's stable-slot scheme so a per-iteration aggregate construction
   in the quotation body does not reintroduce an `alloc` bump.

5. **The slice-4/slice-5 boundary (see open question 1).** The recon settles that the two
   "inlining"s are different mechanisms (recon 8); it does not settle whether slice 4 ships
   a runnable end-to-end constant-stack demo (which needs an intrinsic loop combinator plus
   fusion, decisions 3+4) or ships quotation-literal + `call` + fusion as inert plumbing
   and defers the first runnable loop to slice 5. The spec has to pick, and the pick
   defines what slice 4's exit criterion can even assert.

6. **Interaction with `if`, unchanged this slice.** ROADMAP slice 8 turns `if` into a
   quotation combinator; this slice does not. `if` stays a keyword and stays rejected in
   polymorphic bodies (recon 7). The spec should say so, so the implementation does not
   drift into lifting `if` to `PolyType` (a slice-8 job) while building quotations.

## Scope

In: quotation literal syntax `[ ... ]` and a new `TermKind`; `call`; the compile-time
quotation marker and its effect inference; the non-escape check and its diagnostics; the
`call`-of-literal fusion lowering; and the front-end trigger from a quotation loop into
the existing back-edge machinery, with at least the intrinsic floor needed for the exit
criterion the spec settles in decision 5.

Out: a first-class runtime quotation type and the `Type`/`PolyType`/`IrType`/unification/
mangling changes it implies (recon 2, deferred with escaping quotations to Phase 6);
escaping quotations and the uniform-runtime-stack fallback (Phase 6); the interprocedural
user-word inliner and the `each`/`map`/`filter`/`fold`/`while`/`times` library (slice 5);
`if` as a combinator and `Bool` as an enum (slice 8); lifting `if` to polymorphic bodies
(recon 7, a slice-8 prerequisite, not this slice); static overloading and multimethods
(slices 6, 7).

## Exit

A quotation literal parses and checks; `call` on a literal fuses to its body and runs; a
quotation reaching a position where its effect is unknown (recon-2 nesting: a merged
`if`, an array element, a non-inlined parameter) is a located rejection, not a panic; and
the loop-primitive demo the spec settles in decision 5 runs in constant stack, witnessed
the way slice 3 witnessed its fix (a deep iteration that would overflow a real recursion
returns cleanly). Goldens for each, including the non-escape rejections as behavior, per
the diagnostics-are-behavior convention.

## Open questions for the owner

1. **Where is the slice-4/slice-5 boundary, and does slice 4 ship a runnable loop?**
   Recon 8 settles that slice 4's "call-site inlining" (quotation-literal fusion) and slice
   5's "the inliner" (interprocedural user-word inlining) are different mechanisms, so the
   two ROADMAP statements do not actually conflict. What recon cannot settle is the cut:
   (a) slice 4 ships quotation + `call` + fusion + an intrinsic loop combinator, so it has
   a runnable constant-stack demo of its own, and slice 5 only adds inlining of the
   *library* combinators on top; or (b) slice 4 ships quotation + `call` + fusion as
   plumbing and the first runnable loop waits for slice 5's inliner to inline a library
   `each` into a monomorphic context. Tradeoff: (a) gives slice 4 an honest exit criterion
   and an integration test of the loop machinery against quotations one slice earlier, at
   the cost of a compiler-known intrinsic that slice 5's library words may later subsume;
   (b) keeps the intrinsic floor minimal but leaves slice 4 with nothing end-to-end to
   demonstrate, which sits badly with this project's "write the program first" ethic.
   Recommendation: (a). Recon 6/7 show the library path cannot run standalone, so without
   an intrinsic floor in slice 4 there is no constant-stack witness until slice 5, and the
   phase's riskiest integration (type machinery against quotations against the loop
   primitive) would arrive untested until then. This is a judgment call because it is a
   scope-and-sequencing preference, not something the code forces.

2. **Runtime quotation type now, or deferred to Phase 6?** Decision 1 recommends the
   compile-time-only marker, which is enough for every quotation slice 4 can run and defers
   the slice-1-sized type-nesting change (recon 2). The counter-argument is that if a later
   slice or Phase 6 will pay that cost anyway, designing the marker now and the type later
   risks touching the checker's quotation handling twice, the exact multiple-reopen hazard
   slice 1's D1 warns against. Recommendation: defer, because the runtime type buys only
   non-inlined and escaping quotations, both already out of scope (Phase 6), so paying now
   is speculative structure against a consumer that does not exist in this slice. But
   whether the marker's design should be *shaped* to extend cleanly into the eventual type,
   versus kept deliberately minimal, is the owner's call on how firm the Phase 6 commitment
   is.

3. **Does the intrinsic loop combinator, if slice 4 ships one (open question 1a), get a
   user-facing name or stay an internal lowering target?** DESIGN.md:285 allows "one or two
   intrinsic combinators" as the floor. If one is user-callable it becomes surface API that
   slice 5's library-word `each`/`times` must either wrap or shadow, and retiring it later
   (if slice 5's inliner subsumes it) is a visible change; if it stays internal, slice 4's
   demo must reach it through some already-legal syntax, which may force at least a minimal
   library word forward into slice 4. Recommendation: keep it internal and reach the demo
   through the thinnest possible intrinsic, but this depends entirely on the answer to open
   question 1.

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "Brief reports concrete findings with src/file.rs:line citations and ran probes; severity-bearing findings flagged: recon 2 (no quotation type at any layer, slice-1-sized change) and recon 6+7 (polymorphic bodies reject `if` and skip the constant-stack loop transform, so combinators cannot be plain library words)."
    }
  ],
  "changedFiles": [
    "docs/phase4-slice4-brief.md"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "cargo build",
      "result": "passed",
      "summary": "compiler built clean; used to run probes"
    },
    {
      "command": "cargo run -- build /tmp/*.sth on multi-output, deep self-tail, and polymorphic-if probes",
      "result": "passed",
      "summary": "multi-output/`..s` outputs run correctly; monomorphic 50M self-tail returns cleanly (constant stack); polymorphic `if` body rejected with a located error"
    }
  ],
  "validationOutput": [
    "probe: `[` in body -> parse error: unexpected token LBracket",
    "probe: `5 pair . .` -> 5 5; `7 trip . . .` -> 7 7 7; `..s` passthrough/consume -> 2 1 / 2 1",
    "probe: monomorphic countdown 50000000 -> 42, exit 0 (no overflow)",
    "probe: polymorphic countdown with `if` -> error: `if` in the polymorphic body of `countdown` is not yet supported",
    "docs-only commit; untracked docs/concurrency-implementation-notes.md left unstaged"
  ],
  "residualRisks": [
    "The brief recommends a compile-time-only quotation marker over a first-class runtime type; if the owner wants non-inlined/escaping quotations sooner, the deeper Type/PolyType/IrType change (recon 2) lands earlier than framed.",
    "The slice-4/slice-5 boundary (open question 1) is escalated, not resolved; the spec-writer must settle it before scoping.",
    "Probes exercised representative cases, not the full combinator library (which cannot be written today, recon 6/7); the paper pre-check is by construction, not by a running program."
  ],
  "noStagedFiles": false,
  "diffSummary": "Adds docs/phase4-slice4-brief.md (recon brief for quotations + the internal loop primitive); no source or test changes.",
  "reviewFindings": [
    "no blockers: docs-only addition, no code paths touched",
    "note: recon 6+7 (polymorphic bodies reject `if` and skip the loop transform) is the highest-value finding and reframes the inliner from a perf pass to an expressibility requirement",
    "note: recon 8 resolves the ROADMAP slice-4/slice-5 inliner contradiction as two distinct mechanisms; the remaining boundary is escalated as open question 1"
  ],
  "manualNotes": "Probe files under /tmp were cleaned up. ROADMAP.md was left unmodified per instructions; the inliner contradiction is flagged in the brief (recon 8, open question 1) rather than resolved in the roadmap."
}
```
