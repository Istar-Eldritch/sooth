# Phase 7 Slice 3b: quotations in a polymorphic body (spec, delivered)

## Goal

A quotation literal may appear in a **non-inline polymorphic** word body and be
consumed there, so a polymorphic word can **eliminate an enum**: the generated
eliminator's arms *are* quotation literals. Previously any quotation in a poly body
was rejected outright at `src/check/poly.rs`, which blocked the whole
`unwrap_or`/`map_or`/`Result`-combinator family, since no polymorphic word could
eliminate an enum at all.

The consumer is a *concrete* enum eliminated in a polymorphic body, so nothing here
depends on Phase 6's in-flight work. Trait bounds are P7.S3d; slices are P7.S3c.

## Delivered shape

```sooth
type: Shape | Circle r i64 | Rect w i64 h i64 ;

: area_and_keep ( 'T Shape -- 'T )
  ~[ ( Rect )   Rect> * drop ]
  ~[ ( Circle ) Circle> dup * 3 * drop ]
  Shape? ;

: main ( -- )
  1 5 Circle area_and_keep .     \ keeps the 'T=i64, prints 1
  2 3 4 Rect area_and_keep . ;   \ prints 2
```

`Shape?` takes its scrutinee from the slot directly beneath the arms, so the enum is
the *top* input: `( 'T Shape -- 'T )`. Arms **destructure** (`Rect>`) rather than
project (`&w @`): field projection is rejected in every generic body, so a projecting
arm does not compile even though its monomorphic twin does. `'T` is carried untouched
through the shared caller row across both arms and grounded per instantiation.

**Lowering is unchanged.** `lower_instantiation` already splices raw quotation bodies,
grounds `'T` structurally per instantiation, and picks the correct per-type destructor
for a linear `'T` dropped inside a spliced arm (`emit_drop` reads `value_type`, not a
span-keyed map).

## Design decisions

**D1 Eliminators only.** `check_eliminator_call` is reached by *name* through
`eliminator_registry`, ahead of the env/combinator paths, so it is not the row-typed
inline-combinator path `if` takes. The scrutinee is a concrete enum, so its `EnumId`,
variant set, and each arm's narrowed input are concrete; the only abstract data is the
caller row below the scrutinee and the arms' output slots, compared **structurally**
across arms, never row-unified against an abstract stack. `call`/`branch`/`if`/`times`/
`tag` need that unification and are deferred to **P7.S3b-follow**, each with a located
rejection here.

**D2 No abstract scrutinee.** An abstract scrutinee (a `'T` that is some enum) needs an
enum-kind bound, which is P7.S3d. Located rejection. No code dependency on P6.S3b in
either direction.

**D3 `PolySlot` struct, `lits` folded in.** `PolySlot { pt, int_val, quot }` replaces the
poly walk's `Vec<PolyType>` **and** its parallel `lits: Vec<Option<i64>>` shadow. A third
parallel vector would add a fourth length to keep lock-step across all stack-mutation
sites, and a desynced `quot` *mis-compiles* (splices the wrong body) where a desynced
`lits` only mis-diagnoses. `alias`/`deriv`/`surviving` are excluded: the poly walk tracks
none of them (borrows live in `PolyScope`).

**D4 Arm bodies are walked by re-entering `poly_walk`.** An eliminator arm has no
`~[ ..a -- ..b ]` row effect: it is annotated by *variant* (`( Rect )`), and its input is
the concrete narrowed variant the dispatch computes. So there is no poly analogue of
`check_literal_against_declared_effect`; the arm body is walked over
`(abstract row ++ concrete narrowed variant)`, yielding an abstract exit row.

**D5 No quotation escapes, and each escape route has its own rejection.** Capture
admission has no poly twin because a poly-body quotation must be consumed by the
eliminator in the same body. Three distinct routes, three distinct rejections, three
tests: word exit, arm exit (reported at the *nested* literal's span), and the
data-operand route through the marker predicates.

## Locked rules

- **L1 Type variables stay rigid across arms; no mid-body `Subst`.** Arm A leaving
  `Var(0)` against arm B `Var(1)`, or `Var(0)` against `Concrete(i64)`, is a located
  error, not a bind of `'T := i64`. Every `Subst::default()` in `poly.rs` stays at a
  call-site/instantiation boundary, never in the term walk.
- **L2 Splice-consumed quotations only.** A poly-body quotation may not be materialised
  (stored in a field or array element, returned, or erased into a capture set). This
  keeps `surviving` out of `PolySlot` and steers clear of two pre-existing ICEs (a
  quotation in a row-typed combinator's row; a materialized quotation returning a ref)
  and the `unreachable!("a quotation effect never reaches monomorphized lowering")`.
- **L3 Identity rides the slot; a *tagged* arm is still written adjacent to its
  eliminator.** The `PolyQuotRef` in a `PolySlot` moves with the slot, so `dup`/`swap`/
  `drop` reorder indices with zero special handling (pinned at the **unit** level). A
  variant-tagged literal must reach its eliminator by written adjacency, the same rule
  the concrete path applies, so a `swap` between two arms is rejected on both paths with
  the same message: a tagged literal no eliminator collects is never checked against
  anything, and the generic path does not get to be laxer. Identity does not survive a
  `| q |` bind (`PolyScope.locals` carries no `PolyQuotRef`), so a bound-then-named
  quotation is safely over-rejected as an untagged arm.
- **L4 The arm merge UNIONS the borrow table.** A false-accept risk, not a detail.
  `PolyScope.borrows` is keyed by a local's *name*, and a **missing** record reads as "no
  conflict" (`live_borrow_of` returns `None`). If arm A borrows `&!x` and arm B `&!y`, a
  merge that picks one arm or intersects drops the other's live record and a later use of
  that place is silently accepted. The merge unions by place and **rejects** a genuine
  disagreement rather than erasing it.

## Delivered

### R1 `PolySlot` representation (`Vec<PolySlot>`, `lits` deleted)

`struct PolySlot { pt: PolyType, int_val: Option<i64>, quot: Option<PolyQuotRef> }`
threads the six stack-threading functions (`poly_walk`, `poly_term`, `poly_call_term`,
`poly_construct_generic`, `poly_reference_word`, `poly_delegate_op`) and the two
stack-reading helpers (`prune_dead_borrows`, `live_borrow_of`, now `&[PolySlot]`).
`int_val` carries what `lits` did: set on `IntLit`, `None` elsewhere, truncated on
`Bind`. The `debug_assert_eq!(stack.len(), lits.len())` guard became structurally
impossible and was removed, not retargeted.

`PolyQuotRef` is the poly twin of `QuotId`/`prov.quotations`: a `Copy` index into an
append-only `Vec<PolyQuotLit>` interner **on `PolyScope`** (already `&mut`-threaded, so
no seventh parameter), recording each literal's body, inline flavour, resolved annotation
(whose `variant_tag` gives the arm tag), and span. Append-only, so every index stays
valid across the per-arm `PolyScope` clones.

### R2 Quotation-literal admission and eliminator dispatch

A quotation literal pushes `PolySlot::quotation(..)`: `pt = PolyType::QuotLit`, a
dedicated marker with **no** value identity (two bodies with one effect are one
`PolyType`, and a placeholder would leak into output unification/`Subst`/mangling).
Every predicate treats it as not-a-value: `poly_is_copy → false`, `is_reference_slot →
false`, and any use as a data operand (arithmetic, construction, output at word exit) is
a located rejection. `PolyType::QuotLit` is `unreachable!` in signature positions, since
the marker is body-only.

`poly_call_term` intercepts `eliminator_registry` (built locally from its `enums` param:
`poly_call_term` has no `PolyCtx`) **before** the ordinary `env` dispatch, then runs
`poly_eliminator_call`, a port of `check_eliminator_call` with these substitutions:

- Arms are collected off the top of the `Vec<PolySlot>` by their `quot` field, each
  `Some` and tagged, not via `resolve_quotation_operand` over a `Slot`. Untagged or
  forwarded-abstract arm reuses the located `eliminator_untagged_arm` diagnostic.
- The scrutinee's `pt` must be `PolyType::Concrete(Type::Enum(..))`. An abstract
  scrutinee is `poly_abstract_enum_scrutinee_error`. A **reference** scrutinee is its own
  `poly_reference_scrutinee_error`: eliminating through a ref leaves arms projecting
  fields off a borrowed variant, and projection is rejected in every generic body, so
  those arms cannot be written at all.
- Exhaustiveness, duplicate-arm, unknown-variant and variant-escape checks reuse the
  concrete diagnostics verbatim.
- Each arm body is walked by re-entering `poly_walk` over `(row ++ narrowed concrete
  variant)` (D4).

### R3 Abstract N-arm join (structural, rigid, borrow-unioning)

- **Depth agreement**: all arms leave the same depth, or the reused
  `combinator_branch_output_mismatch_error`.
- **Per-slot structural agreement (L1)**: rigid type variables;
  `poly_arm_output_disagreement_error` on disagreement, never a `Subst` bind. No
  `apply_subst` in the term walk.
- **A poly analogue of `Scope::leave`, run per arm, before the join.** `Moves::join`
  iterates the first arm's keys and *indexes* the other's, so a local present in one arm
  and absent in another **panics**. The concrete path survives only because a quotation
  body is a block and `Scope::leave` removes arm-bound locals first; the poly walk has no
  block scope. So: record the enclosing key set at arm entry; at arm exit **first** reject
  an unconsumed arm-local linear value (`poly_arm_local_not_consumed_error`), **then**
  truncate `moves`/`locals` back to the enclosing set; only then reduce with
  `into_iter().reduce(Moves::join)`. A key-set-tolerant join would close the ICE but leave
  the linearity hole (a leaked arm-local unreported, an out-of-scope name written into the
  enclosing scope), violating the linear spine.
- **Borrow-table UNION by place (L4)**, with `poly_arm_borrow_disagreement_error` for a
  genuine cross-arm disagreement (borrowed on one arm and consumed on another, or
  differing mutability), never an erase-to-empty that `live_borrow_of` reads as "no
  conflict".

Each arm runs against its own clone of the enclosing `PolyScope`; the join reconciles the
clones. Type variables are never bound, so no clone diverges on `Subst`.

### R4 Located rejections, no `unknown word` fallthrough

`poly_quotation_not_consumed_error` (materialised, stored, returned, or unconsumed at
word or arm exit), `poly_quotation_combinator_unsupported_error` (`call`/`branch`/`if`/
`times`/`tag`, naming P7.S3b-follow), `poly_abstract_enum_scrutinee_error`,
`poly_reference_scrutinee_error`, plus the marker's own operand rejections through
`poly_copy_gate` and the shared operand renderer.

### R5 Comment corrections

The two comments justifying the old wall are gone with it: "nowhere to hang the `quot`
marker" (false: `PolySlot.quot` is the hang point, and `lits` was the precedent), and
"mirrors the `if`-in-a-polymorphic-body rejection above" (points at deleted code:
`TermKind` has no `If` variant). The adjacent array-constructor rejection's
back-reference is accurate and stays; that rejection itself is **not** lifted.

## Out of scope

- The clause-style-body rejection (`WordBody::Clauses` is P6.S4's to delete).
- `call`/`branch`/`if`/`times`/`tag` in a poly body: **P7.S3b-follow**.
- The array-constructor rejection: a separate gap of the same shape.
- Materialised / escaping / erased quotations (L2), and the two pre-existing ICEs there.
- Mid-body unification of type variables (L1).
- Abstract enum scrutinees / enum-kind bounds: P7.S3d.
- Any lowering change.

## Testing

Goldens (`tests/phase7_slice3b.rs`): the `area_and_keep` exit criterion; a type variable
carried across arms at two instantiations (`'T=i64`, `'T=str`); arm output disagreement;
arm depth mismatch; reference scrutinee; abstract scrutinee; the borrow-union
false-accept guard (asserting **both** directions, since "pick arm A" keeps `x` and drops
`y`) and cross-arm mutability disagreement; bind-and-leak of a linear arm-local; one arm
binding and one not (the no-ICE case); non-exhaustive / duplicate / untagged / ordinary-
bracket / bound-quotation arms; the three materialisation routes (word exit, arm exit at
the inner literal's span, data operand); `[ … ] call`; non-adjacent tagged arm and orphan
tagged literal.

Unit tests in `src/check/poly.rs`: `polyslot_int_val_folds_lits`,
`poly_term_admits_a_quotation_literal_as_a_marker_slot`,
`poly_quotation_identity_moves_with_the_slot_under_swap` (L3 at the level where a source
program cannot reach it), `poly_quotation_slot_is_not_copy`,
`poly_eliminator_registry_intercept_precedes_env_dispatch`,
`poly_arm_join_unions_borrows`, `poly_arm_join_rejects_rigid_type_variable_disagreement`,
`poly_eliminator_arm_leaving_its_own_variant_is_error`.

Two test-design constraints worth keeping:

- **The intercept-ordering unit test.** The eliminator's `PolySig` in `poly_env` orders
  arm parameters by enum *declaration* order and would match them by slot position; the
  intercept matches by annotation tag. The test writes arms in the **reverse** of
  declaration order, which is what makes the accept evidence of tag matching. Deleting
  the intercept does not reach env dispatch at all (`poly_call_term` has no `PolyCtx`),
  so the mutation flips accept to `unknown word`, not to a positional mismatch.
- **`poly_eliminator_arm_leaving_its_own_variant_is_error` needs a single-variant enum.**
  With two arms, R3's rigid-disagreement check fires on the differing exit shapes before
  the escaping `Type::Variant` is looked at, so no two-arm program reaches the guard. It
  carries a linear `Spy` payload, not an `i64`: with the guard stubbed the payload is
  destructed twice.

Mutation-tested guards: the borrow union (both single-arm picks), the arm-local
must-consume check, the rigid-disagreement error, the word-exit and arm-exit
materialisation rejections (the latter mutated twice: stubbed open, and with its span
reverted to the enclosing arm), the variant-escape rejection (verified by *running*), the
marker data-operand predicate, the intercept ordering, the deferred-family rejection, and
`int_val` truncation on `Bind`.

Regression, green and untouched: `tests/phase7_slice3a.rs`, `tests/phase6_*` eliminator
suites, `tests/qbe_baseline.rs`.

## Exit findings

### The `Type::Variant` predicate hole (recorded, not fixed)

`is_copy` matches `Type::Struct` and `Type::Enum` and ends `_ => true`, so a narrowed
variant reads as trivially `Copy` and `~[ ( A ) dup A> drop A> drop ]` runs the payload's
destructor **twice**. The variant-escape guard covers only the arm's *exit* row, which is
what stops the variant leaving the call; it does nothing about a `dup` **inside** the arm.

This reproduces on the **concrete** path at this slice's parent commit, so it is not a
P7.S3b regression: the slice makes the same hole reachable from a second path without
widening it. The fix is a `Type::Variant` arm on the predicate family (`is_copy`,
`contains_reference`, the `drop`-import visibility check in `check_shuffle`), which
belongs to whichever slice owns that family. Probing it needs a linear payload; a
scalar-payload enum is `Copy` anyway and hides the bug.

### Structure signals: split deferred, deliberately

`src/check/poly.rs` is 3348 source lines. Three of five signals fire (a module doing
several things, high- and low-level code mixed, ~30 diagnostic formatters that never call
each other); two do not (single `use super::*`, no circular dependency). Deferred, for
reasons about the split point rather than the churn:

- The layer-shaped split (`poly/diagnostics.rs`) is the one CLAUDE.md names as wrong and
  has no precedent: `src/check.rs` carries 40 error formatters beside their checks,
  `terms.rs` 10, `declarations.rs` 17, all interleaved.
- The responsibility-shaped split (`poly/eliminator.rs`) would cut the mutual recursion
  `poly_call_term → poly_eliminator_call → poly_walk → poly_call_term` across a file
  boundary to move ~430 lines, raising coupling to lower a line count.
- The split point becomes real at **P7.S3b-follow**, which adds a *second* quotation
  consumer. Re-run the signals there.

### What the unblocked family can be written against today

Two standing limits, both pre-existing and both verified at this slice's parent commit:

- **A generic word cannot call another generic word** (`unknown word g__m0`): poly words
  are not registered in `env`, and `poly_call_term` has no `PolyCtx` to read `poly_env`
  from. A combinator written here composes **concrete and builtin callees only**.
- **Field projection (`&w`) is rejected in every generic body**, so an arm destructures
  rather than projects. This is why a monomorphic twin is evidence for operand order and
  nothing else.

Both belong to whichever slice takes generic-to-generic dispatch.
