# Phase 7 Slice 3b: quotations in a polymorphic body (brief)

A quotation literal in a **non-inline** polymorphic word body is rejected outright
(`src/check/poly.rs:505-513`). It fires for any quotation: a bare `~[ ]`, an `if`, a
declared comparator. Since `if` is an ordinary library word over the `branch` builtin
(slice 10c), **any polymorphic word that branches is forced `inline` today**, and an
`inline` word's body is spliced rather than compiled once, so it mints no monomorph
symbol and its code is duplicated at every call site.

This slice makes a non-inline polymorphic body able to hold a quotation, and therefore
to branch.

## Recon (worker-verified against the built compiler)

Three parallel recon workers, one of which built and ran a stubbed compiler in an
isolated copy. Findings are `file:line`-grounded or output-grounded; the honest
unknowns each worker declared are carried into "Open questions" rather than papered over.

**The rejection's own stated reason is false.** The comment says "`poly_term`'s stack is
`Vec<PolyType>`, not `Vec<Slot>`, so there is nowhere to hang the `quot` marker". There
is: `poly_term` already threads a parallel per-slot side vector
`lits: &mut Vec<Option<i64>>` (`poly.rs:385`), mirroring `Slot::int_val`, maintained in
lock-step at all 27 shuffle/produce/consume sites and guarded by
`debug_assert_eq!(stack.len(), lits.len())` (`poly.rs:413`). A `QuotId` is the same shape
of datum as an `int_val` and can ride the same way.

Two narrower statements *are* true, and they are the real constraints:

- **Identity must not live in the type.** `PolyType::Quotation(ins, outs, ..)`
  (`ast.rs:1179`) carries only the *effect*, so two distinct literal bodies with the same
  effect are the same `PolyType` and it cannot answer "which body to splice". A
  placeholder `PolyType` would additionally leak a fake type into output unification,
  `Subst`, and mangling. D1's refusal of a `PolyType` variant stands.
- **The side state must be positional, not name-keyed.** The existing coarse borrow table
  (`PolyScope.borrows`, `poly.rs:66-71`) is keyed by a local's *name* and does **not**
  transfer: a quotation literal is an anonymous stack value with no stable key surviving a
  `swap`. `lits` is the correct precedent; `borrows` is not. The borrow table is also
  self-described as a lossy compromise (`poly.rs:81-89`, "one unrelated live reference
  keeps *every* recorded borrow alive"), and quotation identity cannot be
  over-approximated: a coarse merge mis-splices rather than merely over-rejecting.

**The comment also cites a sibling that no longer exists.** "Mirrors the
`if`-in-a-polymorphic-body rejection above" refers to deleted code: `TermKind` has no
`If` variant (`ast.rs:1793`), and slice 10c's turning `if` into a library word left only a
tombstone (`src/check/drop_graph.rs:233`, "the role the deleted `TermKind::If` descent
played"). The adjacent array-constructor rejection's back-reference (`poly.rs:517-527`) is
accurate; this one is stale.

**Lowering is free, and this is output-grounded rather than reasoned.** A worker stubbed
the check-side rejection open in an isolated copy and pushed programs through to running
QBE:

- `[ dup . ] call` in a non-inline poly body splices and grounds `'T`: emits
  `call $printf($fmt, %v0)` then `ret %v0`.
- One quotation body span, two instantiations, both correct: the same `.` lowered to an
  i64 print in `sooth_mono_foo__m0__t0_i64` and to a length-and-pointer str print in
  `..._t0_str`.
- The case most likely to break did not: a linear `'T` dropped inside a spliced quotation
  selected the correct per-type destructor (`call $sooth_struct_drop_0(:Res %v0)`),
  despite the `drop` having a single shared span across instantiations.
- `branch` (the primitive `if` compiles to) emitted a correct `jnz`/two-arm/join.

The mechanism: `lower_instantiation` (`ir/driver.rs:760`) grounds only the *signature*
via `concrete_effect` and hands the raw AST body to `lower_word_parts`, the ordinary
concrete path. Nothing threads `Subst` into the body walk; every op derives behaviour from
the runtime value's `IrType` (`emit_drop` at `ir/func_builder/quotation.rs:297` matches on
`self.value_type(v)`, consulting no span-keyed map). So a shared body span across
instantiations is harmless for these ops.

**But there is no abstract branch-join at all.** `poly_walk` (`poly.rs:387`) is a strictly
linear fold over one stack; `poly_term` (`poly.rs:424`) has arms only for
`IntLit`/`FloatLit`/`StrLit`/`Bind`/`Call`/`Quotation`(rejected)/`ArrayCtor`(rejected).
`PolyScope` holds a `Moves` (`poly.rs:56`) whose two-arm reconciler `Moves::join`
(`check/engine.rs:453`) exists and is **never called from `poly.rs`**. All branch-and-join
lives in the concrete `Slot` checker (`check_branch_join`, `terms.rs:1097`, ~290 lines),
reachable from a polymorphic word only by `inline` splicing — which is exactly why every
branch test in the poly suite declares `inline` (`poly.rs:3075-3082`).

**And the whole combinator family is absent from the poly path.** `poly_call_term`'s
`match name` has no `call`, no `branch`, no `if`, and no inline-combinator splice. The
`resolve_combinator_overload`/`inline_combinator` machinery (`poly.rs:1620-1760`) exists
but is wired only for combinator *parameters*, not for a quotation *literal spliced inside
a poly body*. This is the bulk of the slice.

**Mechanical surface for the representation change**: 28 stack-mutation call sites
(~18 logical ops) plus ~26 type-reads, across the 6 functions threading the stack
(`poly.rs:384/421/537/985/1147/1431`). Hand-counted, accurate to about ±1.

## Shape of the work

Three check-side layers and no lowering work:

1. **Representation.** `PolySlot { pt: PolyType, quot: Option<QuotRef> }` replacing the
   bare `Vec<PolyType>`. Exactly one new field beyond the type: `alias`, `deriv`, and
   `surviving` are all excluded because the poly walk tracks none of them, and carrying
   them would be dead weight inviting "why is this always `None`". Folding `lits`'s
   `int_val` into the struct (deleting the parallel vector) is optional consolidation and
   should be decided, not drifted into.
2. **Combinator dispatch.** Teach `poly_call_term` to consume a quotation slot: `call`,
   `branch`, and the inline-combinator splice, so `if` works. The bulk.
3. **Abstract branch-join.** Port the depth check, per-slot agreement, and `Moves::join`
   wiring from `check_branch_join`. `Moves::join` is reusable as-is.

## Locked decisions

**Type variables stay rigid across arms; the body gets no mid-body `Subst`.** Today
nothing in a polymorphic body binds a type variable — `Var(v)` is a skolem, and every
`Subst::default()` in `poly.rs` is at a call-site/instantiation boundary
(`:177/1611/1655/1811`), never in the term walk. Keeping that means the arm merge is a
decidable structural `PolyType` comparison: arm A leaving `Var(0)` and arm B leaving
`Var(1)` disagree, and arm A `Var(0)` against arm B `Concrete(i64)` is a new located
error. Admitting the latter by binding `'T := i64` would mean a genuinely new mid-body
unifier with ripples into `poly_output_mismatch_error` and `instantiation_symbol`
mangling. Not needed for `if`, and ruled out.

**Splice-consumed quotations only.** A quotation in a polymorphic body must be consumed by
`call`/`branch`/a combinator argument in that same body. It may not be materialised: stored
in a field or array element, returned, or erased into a capture set. This is what keeps
`surviving` out of `PolySlot`, and it avoids two known pre-existing ICEs (a quotation inside
a row-typed combinator's row, and a materialized quotation returning a ref) plus the
`unreachable!("a quotation effect never reaches monomorphized lowering")` at
`ir/driver.rs:664`, which fires only when a quotation type reaches the grounded *signature*.
The rejection for a materialised one must be located and must name why, not fall through to
the generic message this slice is deleting.

**Shuffling a quotation stays legal.** Restricting a quotation to immediate consumption
(no `dup`/`swap` before use) would be cheaper but rejects legal programs, since combinator
code shuffles quotations routinely. Identity must survive reordering.

## Open questions

1. **`PolySlot` struct vs. a third lock-step vector.** A parallel
   `quots: Vec<Option<QuotRef>>` is fewer edits (the ~26 `stack[i]` type-reads stay
   untouched) but adds a third vector to keep synchronised across all 28 mutation sites and
   widens the length invariant to three-way. The struct is more edits but removes that
   invariant class, mirrors the concrete `Slot`, and lets `lits` be deleted. Recon
   recommends the struct on the grounds that a desynced `quot` **mis-compiles** where a
   desynced `lits` only mis-diagnoses. The spec should confirm and record the choice.
2. **What does the arm-merge do about the coarse borrow table?** `PolyScope.borrows` has no
   arm-aware logic and `prune_dead_borrows` is prefix-only. Does each arm clone and merge
   it, or does its existing coarseness already fail safe across a join? Unverified.
3. **Does the quotation literal's own body get walked abstractly, and against what?** The
   concrete path checks a literal against a declared effect
   (`check_literal_against_declared_effect`); there is no poly analogue. For `if`'s arms
   this is where a `~[ ..a -- ..b ]` row effect meets an abstract stack.
4. **Capture admission has no poly twin.** The concrete `check_branch_join` carries
   capture-admission machinery. Under the splice-only rule a quotation cannot escape, so
   this may be vacuous — but "may be" needs to become "is", with a test.
5. **Does `times` need to be in scope, or only `call`/`branch`?** `times` also consumes a
   quotation by splicing. Including it is probably cheap; excluding it needs a located
   rejection.

## Out of scope

- Trait bounds (P7.S3d).
- Slices (P7.S3c).
- Materialised, escaping, or erased quotations in a polymorphic body (see locked
  decisions); the two pre-existing ICEs in that neighbourhood are not this slice's to fix.
- Mid-body unification of type variables (see locked decisions).
- The array-constructor rejection (`poly.rs:517-527`), which is a separate gap with the
  same shape and should not be quietly bundled in.

## Ready to spec?

Yes, with the five open questions above to be resolved in the spec. The two facts that
most affect sizing are settled and evidence-backed: lowering needs no work, and no abstract
branch-join exists. Two stale/false source comments (`poly.rs:505-513`) should be corrected
by this slice, since it is deleting the rejection they justify.

Expect **L**, comparable to P7.S3a: a mechanical representation change across ~54 sites, a
new combinator dispatch family, and a ~290-line port, against zero lowering work.
