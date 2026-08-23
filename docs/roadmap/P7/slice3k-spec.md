## Problem (closed)

`poly_call_term` was threaded `poly_words: &HashSet<String>`, callee *names* only, and
never `poly_env: &PolyEnv`, so a non-inline generic body could learn that a name was a
generic word but never fetch its signature. Every call to a *different* generic word fell
through to `poly_calls_poly_word_error` ("a polymorphic word is not yet reachable from
another polymorphic word across a module boundary"). The self-call arm (P7.S3g) was never
an instance: it special-cases `ctx.mangled_name() == Some(name)` and reuses the walk's own
`sig`, with no lookup, unification, or fresh variables.

Also removed: the six-name "comparisons need `Ord`" carve-out in `poly_call_term`, dead in
any real build because `eq`/`lt`/`gt`/`lte`/`gte`/`ne` are `inline` `lib/cmp.sth` words and
so arrive mangled; it only ever fired under `check_src`'s unmangled harness.

## Requirements (as delivered)

- **R1** A non-inline generic body calls another generic word, same-module or imported,
  user-defined or a library word, passing its own rigid variables through. Grounds at
  check time.
- **R2** The checker relates callee input `PolyType`s to the caller's rigid operand slots
  symbolically, producing a variable-to-variable mapping (`Vec<(u32, Image)>`, `Image =
  Concrete(Type) | CallerVar(u32)`), not a ground substitution. A callee variable pinned
  to two different caller images is a located call-site error, mirroring
  `poly_var_conflict_error`.
- **R3** Each `(callee_var, bound)` is discharged at the call site: symbolically via
  `caller_sig.has_bound` for a `CallerVar` image, by the ordinary predicate for a
  `Concrete` one. Never a deferred monomorphization-time panic.
- **R4** The callee is monomorphized once per distinct concrete instantiation the caller
  reaches, deduped by `instantiation_symbol` with a concrete caller's instantiations.
- **R5** Non-growing indirect/mutual recursion (`g` ↔ `h`, each hop a pure renaming)
  compiles and runs.
- **R6** A *growing* cross-call is a located call-site rejection
  (`poly_growing_cross_call_error`). The rule: the image of every callee variable must be
  fully concrete (at any depth) or a bare `Var(_)`; reject a compound image that *mentions*
  a caller variable (`Array(.. Var(T) ..)`, `Ref(Var(T), _)`, `Generic{ args:[Var(T)] }`),
  which is the caller wrapping its own variable before the call.
  - Accepted, forwarding: `h ( &'U -- )` from `g ( &'T -- )`, the image of `'U` is bare
    `Var(T)`, nothing was wrapped.
  - Rejected, growth: `h ( 'U -- )` from `g ( 'T -- 'T ) ... Box h`, the image is
    `Generic{ Box, args:[Var(T)] }`.
  - Accepted cost: a single non-recursive wrap is also rejected, though it would
    terminate. Deliberate simplification, bought a check-time structural rule with no
    cycle detection; the diagnostic names the restriction so a later slice can lift it.
- **R7** `poly_calls_poly_word_error`, the six-name carve-out, and
  `check_poly_ord_word_accepts_comparison_body` are deleted. The two narrowing tests
  (`phase7_slice3g.rs::different_poly_word_call_still_names_the_narrowing`,
  `phase8_slice2.rs::a_poly_word_calling_an_imported_poly_word_names_the_narrowing`) are
  retired and replaced by grounding goldens; the prose references in
  `tests/phase7_slice3b_follow.rs` were updated.

**N1** Every rejection is located and at check time; no cross-call reaches
`calls.rs`'s `expect("checked user word exists")`. **N2** IL for every pre-existing
concrete-caller case is byte-identical (`transitive_instantiations` is empty for such a
program, so the drain's `distinct` list is unchanged). **N3** Termination is R6's
theorem, not a cap: a composed θ assigns each callee variable a fixed concrete type or
θ_w's image of one caller variable, never a constructor over it, so the reachable
`(word, θ)` closure is finite and symbol-dedup reaches a fixpoint. **N4** Craft scope: no
worklist abstraction beyond these two phases.

## Architecture (as built)

Discovery and interning run at **check time**, not in `driver.rs`: the ground caller
substitutions are exactly the concrete `CallInst`s the checker records, and composing θ_h
must reuse `apply_subst`'s registry interning.

- **Phase 1 record.** `Module.poly_cross_calls: HashMap<String, Vec<PolyCrossCall>>`
  (`ast.rs:71`), keyed by the generic word whose body holds the calls (no instantiation
  exists at walk time). `PolyCrossCall { callee, span, mapping }`. Never grounded; read
  only by phase 2's fixpoint.
- **Phase 2 fixpoint.** `discover_transitive_instantiations` (`check/poly.rs:4504`, called
  from `check.rs:990`, after the concrete `out_arity >= 2` bundle loop): seed from the
  concrete instantiations, compose θ_h = θ_w ∘ mapping per record, ground the callee's
  declared *outputs* through `apply_subst`, dedup by `instantiation_symbol` before
  recursing, then intern composed bundles as a post-pass (`intern_composed_bundles`).
- **Two records, because `Module.instantiations` is `Span`-keyed** and one body span
  serves N caller instantiations: `CallInst.poly_calls: HashMap<Span, CallInst>`
  (`ast.rs:1601`, mirrors `trait_calls` but carries the composed callee `CallInst`, since
  lowering needs its `subst`/`symbol`/`out_arity`/`bundle`) and
  `Module.transitive_instantiations: Vec<CallInst>` (`ast.rs:79`, flat, symbol-deduped,
  sorted by symbol, one `IrFunc` per distinct composed pair, each entry carrying its own
  populated `poly_calls`). Composed instantiations inherit the caller's `generation`.
- **Routing (thin).** `ir/driver.rs:271` chains `module.transitive_instantiations` into
  the existing `emitted`/`distinct` dedup; the per-instantiation lowering threads
  `&inst.poly_calls` (`:320`) as it already threads `trait_calls`, with
  `empty_poly_calls()` everywhere else; `func_builder/calls.rs:340` consults `poly_calls`
  *before* the global instantiation lookup and calls `lower_poly_call`.

## Phase 1: reachability, symbolic relation, bound discharge, growth rejection

Landed in `92e73916` (+ `70fc441b`, `119ab9da`, `7b57eaed`, `bca2a175`). `poly_env`
threaded through the walk chain; new arm `poly_cross_call` (`poly.rs:1666`) with
`poly_cross_relate`/`poly_cross_match`/`poly_cross_output`; no IL.

Deviations:

1. **A fifth diagnostic, `poly_cross_call_unsupported_error`**, for the shapes a
   `Vec<(u32, Image)>` mapping structurally cannot carry, each of which N1 needs located:
   a row (`..s`), a quotation parameter, a length variable, a user trait bound, a compound
   *output*, plus two operand-side reasons from deviations 5 and 6. Seven reason strings,
   each naming itself. Not the deleted whole-feature narrowing renamed.
2. **Compound outputs are rejected, symmetrically with R6.** A declared compound always
   mentions a variable (a concrete one folds to `Concrete` at parse), so the rule is
   symmetric in both directions: a type mentioning a caller variable must be bare.
3. **The imported-callee golden split.** Unit half `check_generic_word_calls_mangled_
   generic_grounds` runs real `resolve_modules` mangling (the arm dispatches on
   `poly_env`'s post-mangle keys, never a spelling); the end-to-end half is phase 2's.
4. **An `inline` generic callee landed end-to-end in phase 1**, since lowering splices it
   and it needs no monomorph or routing: `lib/cmp.sth` comparisons on a body's own `'T`
   build and run, and `clampsum` (P7.S3b-follow's exit criterion) is un-`#[ignore]`d.
5. **Review fix: R6 over-rejected a fully concrete compound image.** The `Var` arm sent
   every `Ref`/`Array`/`Generic` supplied type to the growth error, including one
   mentioning no caller variable (`&i64`). `poly_type_mentions_caller_var`
   (`poly.rs:2072`) now separates them, so the concrete case gets the honest
   *unsupported* error instead of a false growth claim. Root cause: folding it would mint
   a fresh `RefId`/`ArrayId`, and the walk holds no mutable array/ref registry.
6. **Review fix: the `Copy` discharge panicked on a body-local generic instantiation.**
   `is_copy` indexes `structs`/`enums`, and `check::check` appends a walk-minted batch
   only after `check_poly_body` returns, so the id sits past the end. Guarded by
   `type_is_registered` (`poly.rs:1785`), rejecting honestly. `Bound::Ord` needs no guard;
   a user bound is gated earlier. Also pinned this round: four structural guards in
   `poly_cross_match`/`poly_cross_relate` (operand count, `Concrete`/`Concrete` equality,
   reference mutability, array length) that were deletable with the suite green and each
   failed *open*.

**REPL keeps today's behaviour deliberately:** it passes an empty registry, so a session
line calling another polymorphic word still gets `unknown word`. REPL lowering resolves
instantiations through its own per-generation store and nothing composes a cross-call's
substitution into it, so grounding there would check clean and mis-lower. Pinned by
`repl_poly_word_calling_another_poly_word_is_unknown_word_not_grounded`.

**R6 fixtures must not use an array wrapper.** Array construction in a polymorphic body is
rejected by a pre-existing guard (`poly.rs:770`), so an array-based growth fixture is a
placebo. The witness is a single-variant generic enum, `type: Box 'T | Box 'T ;`,
constructed in the caller and passed to a callee declaring a bare `'U`.

## Phase 2: check-time transitive fixpoint, routing, regression

Landed in `6115c862` (+ `39f5c252`, `4c8e6b52`, `9d754af0`, `73ccdc00`, `3d0b0a10`).

Deviations:

1. **An overload set is rejected, not keyed on `(name, PolySig)`.** `CallInst::callee` is
   a bare name on both sides and nothing on it records which candidate resolved, so a
   cross-call into *or out of* a polymorphic overload set is located
   (`overloaded_cross_call_error`, via `sole_poly_word`). An overloaded non-inline generic
   word already mis-lowers today with no cross-call anywhere (name-keyed `poly_arities`,
   last-wins); this slice only stops a cross-call from reaching that panic. Note that
   phase 1's *call-site* arm does resolve an overload set by first-match-wins
   (`no_poly_overload_matches_error` when none match); the refusal lands in the fixpoint.
2. **A callee's user trait bound stays a located rejection.** `resolve_user_bound` writes
   `CallInst::trait_calls` keyed by the *callee body's* spans, and composing those per
   caller instantiation is a second mechanism, not a line. Own slice.
3. **Declared *inputs* are not grounded.** There is nothing to intern: `poly_cross_match`
   decomposes a compound input structurally and R6 rejects a caller-built compound, so
   input shapes mirror operand slots the caller's own instantiation already interned.
   Shipped, then deleted as unkillable. Output grounding stays (it is where
   `out_arity`/`output_types` come from), but its `generics_cell` rebase/flush bracket was
   also dead: every output reaching `compose` passed `poly_cross_output`, which rejects
   every compound, so `apply_subst`'s minting arm is unreachable. `CrossGround::generics`
   removed; `compose` grounds with `ctx.generics() == None`.
4. **`transitive_instantiations` is sorted by symbol**, so a test reading the field sees a
   deterministic sequence, not just a deterministic set.
5. **An `inline` callee whose own body calls a polymorphic word is a located rejection**
   (found by code review). `check.rs`'s own-body loop excludes combinators and checks one
   standalone with dummy types, so a call from `h`'s body never reaches
   `poly_cross_calls`, yet lowering really splices `h`'s generic body. Unfixed this hit
   the exact N1 panic (`f` non-inline, `g inline` calling `id`) and a silent wrong-symbol
   splice at two different θ. `cross_calls_of` now runs a conservative one-level scan
   (`body_calls_a_poly_word`, recursing only into `Quotation`) and rejects the outer call
   site. No recursion through a chain of `inline` hops is needed: each hop trips the same
   check when reached as a callee.

**One unkilled claim.** A composed `CallInst` inherits the caller's `generation` and
nothing can exercise it, since the REPL hands the walk an empty registry on purpose.
Writing `None` would be wrong the day the REPL grows its own composition step.

Goldens in `tests/phase7_slice3k.rs`: an imported comparison on a body's own variable, an
undeclared bound, a growing cross-call, monomorphize-once per reached instantiation,
imported non-inline callee, mutual non-growing pair, composed bundle layout, overloaded
cross-call rejection, and one callee reached both concretely and across a cross-call
linking one monomorph. N2 baseline:
`phase7_slice3a.rs::two_asymmetric_instantiations_mint_distinct_symbols_nm` passes
unchanged through the new drain.

## Open, out of scope here

- **R6's accept case for a `Ref`/`Array` image** is still an honest rejection: folding it
  needs a fresh `RefId`/`ArrayId` and the walk holds no mutable path for those two
  registries (`structs`/`enums` do, via `ctx.generics()`, so the generic-aggregate case
  already worked and needed no phase 2 change). Lifting it means widening `Image` with a
  ground-but-uninterned variant the fixpoint interns lazily, or threading a
  `refs`/`arrays` `RefCell` into `Ctx`. Own slice, by design: it is a reviewer's call.
- **A polymorphic body walks registries stale for its own mints.** `check_poly_body`
  rebases the instantiator at entry but the batch flushes only after it returns, so every
  registry-indexing predicate the walk reaches is exposed, not just the cross-call arm:
  `: g ( 'T -- 'T ) 1 Box dup drop drop ;` panics identically through `poly_is_copy` with
  no cross-call at all, and panicked the same way before this slice. Phase 2's fixpoint
  runs after every body, so it does not inherit the fix; closing it needs the walk to see
  pending mints.
- **Pre-existing, confirmed, left alone:** an `inline` generic caller spliced at two
  different θ mints one callee monomorph and segfaults (span-keyed global instantiation
  map is last-write-wins across a splice's call sites, same family as the stale-registry
  finding).

**Split-signals re-check (CLAUDE.md).** `poly.rs` grew 9204 to 10590 lines across the
slice. Import divergence and high/low-level mixing are still absent; a third signal now
fires (the discovery code is called only from `check.rs`, never from another walk
function in the file). Two of four is below threshold, so the prior deferral holds:
revisit alongside it rather than splitting phase 2's code out alone.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Checker: thread poly_env, symbolic variable-to-variable relation with consistency check, cross-signature bound discharge, growing-type and inconsistent-mapping call-site rejection, dead-code/test retirement; records a symbolic cross-call, emits no IL", "effort": "L", "difficulty": "hard" },
    { "phase": 2, "focus": "Check-time transitive instantiation fixpoint (compose + apply_subst interning + bundle interning) populating new CallInst.poly_calls and Module.transitive_instantiations, plus the thin driver.rs drain extension and func_builder poly_calls routing arm, plus run/IL and the named N2 regression baseline", "effort": "M", "difficulty": "hard" }
  ]
}
```
