# Phase 7 Slice 3b: quotations in a polymorphic body (spec)

## Goal

A quotation literal may appear in a **non-inline polymorphic** word body and be
consumed there, so a polymorphic word can **eliminate an enum**: the generated
eliminator's arms *are* quotation literals (`examples/eliminator.sth`), and today
they hit a single wall (`src/check/poly.rs:505-513`) that rejects any quotation in
a poly body outright. Lifting it unblocks the whole
`unwrap_or`/`map_or`/`Result`-combinator family, none of which can exist while no
polymorphic word can eliminate an enum at all.

The forcing consumer is testable **today** against a *concrete* enum eliminated in
a polymorphic body — P6.S3 already shipped `Shape?` — so nothing here depends on
Phase 6's in-flight work (open question 2, ruled below).

### The consumer, verified failing at HEAD

Built against the compiler at HEAD, the concrete `area` from
`examples/eliminator.sth` moved into a polymorphic signature fails with this wall:

```sooth
: area_and_keep ( 'T Shape -- 'T )
  ~[ ( Rect )   Rect> * drop ]
  ~[ ( Circle ) Circle> dup * 3 * drop ]
  Shape? ;
```

**The operand order is load-bearing and was corrected after this spec's first
draft.** `Shape?` takes its scrutinee from the slot directly beneath the arms, so
the enum must be the *top* input: `( 'T Shape -- 'T )`, not `( Shape 'T -- 'T )`.
The draft's order is masked today because the quotation wall fires at
`poly_term` **before** any stack checking, so a stack-wrong program still produces
exactly the expected message — and would then fail the moment the wall is lifted,
which is precisely when the golden is supposed to pass. The corrected order was
verified by compiling the monomorphic twin, which exercises the same dispatch and
is buildable today. The twin vouches for the **operand order only**: its arms may
project (`&w @`) where a generic body's may not, so arm bodies here destructure
(`Rect>`) and a twin that compiles is no evidence that the generic arms do.

```text
: area_and_keep ( i64 Shape -- i64 ) ... Shape? ;   -> builds, runs, prints 1 then 2
: area_and_keep ( Shape i64 -- i64 ) ... Shape? ;
  error: type mismatch in `area_and_keep` (line 6)
    `Shape?` expected `Shape`, found `i64`
```

So "fails with this wall **and nothing else**" must be established against the
monomorphic twin, never against the polymorphic form alone — the wall hides every
later error behind it.

```text
error: a quotation in the polymorphic body of `area_and_keep` (line 4) is not yet supported
```

The wall reports the **first quotation literal's** line, not `Shape?`'s: in the
delivered layout (`type:` on line 1, blank line 2, signature line 3) the first `~[`
is line 4, and `cargo run -- build` on that program emits exactly the line above
(reproduced at HEAD). That message is produced only at `src/check/poly.rs:513`
(grep-confirmed: the sole non-test producer). This program is the R1 golden.

Trait bounds are P7.S3d and appear nowhere here. Slices are P7.S3c.

## Anchors (verified against the tree at HEAD)

Every `file:line` below was confirmed to resolve at HEAD. Where the brief and the
source diverge, the source wins and the divergence is called out.

| What | Anchor | Status |
| --- | --- | --- |
| The quotation-literal rejection | `src/check/poly.rs:505-513` (message `:513`) | confirmed |
| The two stale/false comments | `src/check/poly.rs:507` ("nowhere to hang"), `:510` ("`if`-in-a-polymorphic-body rejection above") | confirmed both |
| Array-ctor rejection (accurate back-ref, out of scope) | `src/check/poly.rs:517-527` | confirmed |
| `lits` parallel per-slot vector | `poly_term` param `lits: &mut Vec<Option<i64>>` (`poly.rs:422`); threaded through `poly_walk` (`:385`), `poly_call_term` (`:538`), and `:986/:1148/:1432` | re-verified: the `poly_term` param is `:422`, **not** the `:461` this table first claimed (`:461` is the `Bind` arm's duplicate-local check); the other five `lits` sites are correct |
| `PolyScope` / borrow table | `poly.rs:54-60` (`borrows: Vec<PolyBorrow>`), `prune_dead_borrows` `:94`, `live_borrow_of` `:111` | re-verified: `live_borrow_of` is `:111`, **not** the `:105` this table first claimed (`:105` is its doc comment); struct `:54`, prune `:94` (the brief's `:66-71`/`:81-89`/`:90-104` are ~12 lines low) |
| `PolyType::Quotation` (effect only, no identity) | `src/ast.rs:1179` | confirmed |
| `TermKind` has no `If` | `src/ast.rs:1793` (variants: `IntLit`/`FloatLit`/`StrLit`/`Call`/`Bind`/`Quotation`/`ArrayCtor`) | confirmed — the `:510` comment's "above" sibling is gone |
| `TermKind::Quotation` shape | `Quotation(Vec<Term>, bool, Option<QuotAnnot>)` (`ast.rs:1816`) | re-verified: `:1816`, not the `:1815` first claimed (the `enum TermKind` header is `:1793`) |
| `TermKind::If` tombstone | `src/check/drop_graph.rs:233` | confirmed |
| `Moves::join` (two-arm; never called from `poly.rs`) | `src/check/engine.rs:453` | confirmed |
| Concrete branch join | `check_branch_join` `src/check/terms.rs:1097` | confirmed |
| Concrete N-arm eliminator | `check_eliminator_call` `src/check.rs:2193`; N-arm `Moves::join` reduce `:2443` (`arm_moves.into_iter().reduce(Moves::join)`); per-slot `merge_arm_output_slot` `src/check.rs:2480` | re-verified: the reduce is `:2443` (the brief had this right); this table first mis-"corrected" it to `:2452`, which is a doc comment inside the `merge_arm_output_slot` region |
| Eliminator registry / intercept | `eliminator_registry` `src/check/declarations.rs:1469`; concrete intercept in `check_term` at `src/check/terms.rs:495` (`if let Some(enum_id) = poly.eliminators.get(name)…`) | re-verified and **repointed**: `check_term` is `src/check/terms.rs:99` (not in `check.rs` at all); the real by-name intercept is `terms.rs:495`. The `src/check.rs:637` this table first cited is registry *construction* (`let eliminators = eliminator_registry(&module.enums)`) inside module assembly, not an intercept |
| `check_literal_against_declared_effect` (no poly analogue) | `src/check.rs:1919`, called from `check_eliminator_call` at `src/check.rs:2363` | re-verified (the `:~2365` first claimed was approximate; the call is `:2363`) |
| Lowering splices the raw body per instantiation | `lower_instantiation` `src/ir/driver.rs:760`; `subst_polytype` `:633`; the `unreachable!` `:664` | confirmed |
| `emit_drop` reads the runtime value type | `src/ir/func_builder/quotation.rs:297` (`match self.value_type(v)`) | confirmed |
| `resolve_combinator_overload` (defined, unwired from `poly.rs`) | `src/check/poly.rs:1701` | confirmed |
| `inline_combinator` (concrete-only reach) | `src/check/combinators.rs:364`, reached from `src/check/terms.rs:644` | confirmed |
| Clause-style-body rejection (DO NOT TOUCH) | `src/check/poly.rs:196-201` and `:336-341` | confirmed both |

The table above was **re-verified cell by cell against HEAD**, not trusted from
the first draft, because three cells were wrong there: the eliminator intercept was
cited as `check.rs:637` (registry construction, and `check_term` is not in `check.rs`
at all — it is `terms.rs:99`, the intercept `terms.rs:495`); the `poly_term` `lits`
param as `:461` (it is `:422`); and the N-arm `Moves::join` reduce as `:2452` (it is
`:2443`). `live_borrow_of` was also `:105` (its doc comment) rather than `:111`, and
the `TermKind::Quotation` shape `:1815` rather than `:1816`. All corrected above and
re-resolved; none changes the design. The brief's borrow-table line numbers were a
dozen lines low (struct `:54`, not `:66`), and the brief had the reduce right at
`:2443`.

## Rulings on the brief's open questions

Each is ruled explicitly with a rationale, since an unruled question ships
permissive and cannot be reviewed.

### OQ1 — the poly path can dispatch a generated eliminator without solving row-typed combinator dispatch. **In scope: eliminators. Deferred: `call`/`branch`/`if`/`times`.**

Investigated, not assumed. `check_eliminator_call` (`src/check.rs:2193`) is
reached by **name** through `eliminator_registry` (`declarations.rs:1469`), ahead
of the env/combinator paths — it is *not* the row-typed inline-combinator path
that `if` takes. Its abstract obligations are narrow:

- The **scrutinee is a concrete enum** in this slice's consumer, so its `EnumId`,
  its variant set, and each arm's narrowed variant input type are all **concrete**.
  Arm collection (`resolve_quotation_operand` in the concrete path), exhaustiveness,
  duplicate-arm and unknown-variant checks are structural over concrete data and
  port unchanged.
- The **only abstract data** is the caller row *below* the scrutinee and the arms'
  output slots. Those are compared **structurally** across arms (rigid type
  variables, OQ/locked below), never row-*unified* against an abstract stack.

So eliminator dispatch needs no row-unification-against-an-abstract-stack, which
is exactly the expensive machinery `if` demands (`if` is a row-typed inline
combinator taking quotation *parameters*). Scope this slice to **eliminator
dispatch**; `call`, `branch`, `if`, and `times` become a named follow-up
(**P7.S3b-follow: quotation-consuming combinators in a poly body**). This is the
biggest sizing lever and it lands on the cheaper side.

### OQ2 — an abstract scrutinee does **not** interact with P6.S3b here; vacuous for this consumer. Confirmed

P6.S3b moves eliminator arm-tag typing to check time against the `EnumId` the
scrutinee operand carries. This slice's consumer is a **concrete** enum in a poly
body, so that `EnumId` is statically known and the tag resolution is identical to
the concrete path's. An *abstract* scrutinee (a `'T` that is some enum) is not
constructible without an enum-kind bound, which is P7.S3d, and is therefore out of
scope. This slice must not depend on P6.S3b; it does not. The seam is recorded so
the two threads stay in touch, but there is no code dependency in either direction.

### OQ3 — `PolySlot` struct, not a third parallel vector. Confirmed, and `lits` is folded in

The struct removes an invariant class rather than widening it: a third
`quots: Vec<Option<QuotRef>>` would add a fourth length to keep lock-step across
all 28 stack-mutation sites, and recon's decisive point stands — **a desynced
`quot` mis-compiles** (it splices the wrong body), where a desynced `lits` only
mis-diagnoses. We adopt `PolySlot { pt, int_val, quot }` and **delete the parallel
`lits` vector**, folding its `Option<i64>` into `int_val`. Rationale for folding
(the brief left this to be decided, not drifted into): Phase 1 already rewrites all
28 mutation sites, so retaining `lits` beside the struct reintroduces precisely the
desync class the struct exists to remove. Fold it. `alias`, `deriv`, and
`surviving` are excluded — the poly walk tracks none of them (borrows live in
`PolyScope`, not per-slot), and carrying an always-`None` field invites "why is
this dead".

### OQ4 — a quotation literal's body is walked **abstractly by re-entering `poly_walk`**, against the arm's concrete narrowed-variant input on top of the abstract caller row. There is no declared-effect string to match

The concrete path checks a literal against a declared effect
(`check_literal_against_declared_effect`). An **eliminator arm has no
`~[ ..a -- ..b ]` row effect** — it is annotated by *variant* (`( Rect )`), and its
input type is the concrete narrowed variant the dispatch computes, not a declared
row. So the poly analogue is not a new "check-literal-against-declared-effect": it
is a recursive `poly_walk` of the arm body over the stack `(abstract row ++
concrete narrowed variant)`, yielding an abstract exit row. This is why eliminators
are dispatchable without the `if`-style declared-row machinery (OQ1), and it is the
one place the abstract N-arm join (R3) is needed. A declared-row poly literal check
is only required by `if`/`call`, which are deferred (OQ1/OQ6).

### OQ5 — capture admission has no poly twin because no poly-body quotation can escape; **pinned by a test on each escape route**, not asserted universal

The splice-only locked rule (below) forbids a quotation escaping into a capture
set: it must be consumed by the eliminator in the same body. The concrete
`check_branch_join`'s capture-admission machinery therefore has no poly twin to
port. This is **not** "vacuous by construction" that a single witness could stand in
for: the escape routes L2/R4 enumerate reach rejection by *two different paths*, so
each needs its own test.

- *Returned / left unconsumed at word exit* → the word-exit `quot`-slot check
  (`poly_quotation_not_consumed_error`, R4).
- *Fed into a struct or array constructor, or into arithmetic (the **data-operand**
  route)* → the R2 pt-marker predicate rejections (`poly_is_copy → false`,
  `is_reference_slot → false`, "any attempt to use it as a data operand … is a
  located rejection"). A **different** rejection path, and the one most likely to
  regress silently (a stubbed-open predicate would let a quotation slot flow into a
  constructor and materialise). It gets its own golden (Testing) and its own mutation
  entry.

So the claim this section pins is the weaker, true one: **every enumerated escape
route is rejected before any capture-admission logic could run, and each route has a
test that flips if its rejection is stubbed open** — not one universal "vacuous by
construction" leaning on a single witness. R4 owns both rejections.

### OQ6 — scope of the quotation-consuming family: **eliminators only.** Everything else gets a **located** rejection

`call`, `branch`, `if`, `times`, and `tag` are out (OQ1). Each, and any quotation
that is *not* consumed by an in-body eliminator, must produce a **located** message
naming the word/why — never an `unknown word` fallthrough, which is what they emit
today with the wall stubbed open. R4 owns these rejections.

## Locked decisions (carried from the brief, restated as binding)

- **L1 Type variables stay rigid across arms; no mid-body `Subst`.** The arm merge
  is a decidable structural `PolyType` comparison. Arm A leaving `Var(0)` and arm B
  leaving `Var(1)` disagree; arm A `Var(0)` against arm B `Concrete(i64)` is a
  **new located error**, not a bind of `'T := i64`. Every `Subst::default()` in
  `poly.rs` stays at a call-site/instantiation boundary (`:177/1611/1655/1811`),
  never in the term walk.
- **L2 Splice-consumed quotations only.** A poly-body quotation must be consumed by
  the eliminator in that same body. It may not be materialised (stored in a field
  or array element, returned, or erased into a capture set). This keeps `surviving`
  out of `PolySlot` and steers clear of two known pre-existing ICEs (a quotation in
  a row-typed combinator's row; a materialized quotation returning a ref) and the
  `unreachable!("a quotation effect never reaches monomorphized lowering")` at
  `src/ir/driver.rs:664`, which fires only when a quotation type reaches the
  grounded *signature*. The rejection is R4's, located and named.
- **L3 A quotation's identity rides its slot; a *tagged* arm is still written adjacent
  to its eliminator.** The `QuotRef` in a `PolySlot` moves with the slot, so
  `dup`/`swap`/`drop` reorder indices with zero special handling — pinned at the
  **unit** level, because what a *source* program may write is narrower. A
  variant-tagged literal must reach its eliminator by written adjacency, the same rule
  the concrete path applies, so a `swap` between two arms is rejected on **both** paths
  with the same message: a tagged literal that no eliminator call collects is never
  checked against anything, and admitting quotation literals in a generic body is
  exactly what would otherwise let one through. The generic path does not get to be
  laxer than the concrete one here. Identity motion also does **not** extend
  across a `| q |` bind: `PolyScope.locals` is `HashMap<String, PolyType>` and carries
  no `QuotRef` (unlike the concrete `Binding.quot`), so a bound-then-named quotation
  loses its identity and is safely over-rejected as an untagged arm — an over-reject,
  not a regression and not an escape.
- **L4 The arm merge UNIONS the borrow table.** This is a **false-accept** risk, not
  a detail (its own requirement, R3, with its own test). `PolyScope.borrows` is
  keyed by a local's *name*; a **missing** record reads as "no conflict" and
  accepts (`live_borrow_of` returning `None`). If arm A borrows `&!x` and arm B
  `&!y`, a merge that picks one arm or intersects drops the other's live record and
  a later use of that place is silently accepted. Within a single linear path the
  table only ever over-rejects (safe until now); a branch breaks that. The merge
  must **union** both arms' `borrows` (by place) and **reject** a genuine
  disagreement rather than erase it, mirroring `merge_arm_output_slot`
  (`src/check.rs:2480`).

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

The arms **destructure** (`Rect>`) rather than project (`&w @`): field projection is
rejected in every generic body, so a projecting arm does not compile here even though
its monomorphic twin does. `Shape?` in a polymorphic body now checks, with `'T` carried
untouched through the shared caller row across both arms and grounded per instantiation
at concrete lowering. Lowering is **unchanged** — probe-verified in the brief: the
concrete `lower_instantiation` path already splices raw quotation bodies, grounds `'T`
structurally per instantiation, and selects the correct per-type destructor for a linear
`'T` dropped inside a spliced arm (`emit_drop` reads `value_type`, not a span-keyed
map). A phase that "adds lowering support" would be inventing work.

## Requirements

### R1 `PolySlot` representation replacing the bare `Vec<PolyType>`, `lits` folded in

`struct PolySlot { pt: PolyType, int_val: Option<i64>, quot: Option<QuotRef> }`.
The six stack-threading functions (`poly_walk` `poly.rs:382`, `poly_term` `:419`,
`poly_call_term` `:534`, and the three at `:982`/`:1144`/`:1428` — `poly_construct_
generic`/`poly_reference_word`/`poly_delegate_op`, whose `lits` params sit at
`:986/:1148/:1432`) take `Vec<PolySlot>` in place of
`Vec<PolyType>` plus the separate `lits: &mut Vec<Option<i64>>` parameter, which is
**deleted** (OQ3). `int_val` carries what `lits` did — set on `IntLit`, `None`
elsewhere, truncated on `Bind` exactly as `lits` was. The `debug_assert_eq!(stack.
len(), lits.len())` guard becomes structurally impossible and is removed, not
retargeted. `quot` is introduced by this requirement but written/read only in R2:
to keep the phase clippy-clean without a dead field, R1 and the first `quot`
reader ship together (see Phasing — R1 is Phase 1, R2/R3/R4 are Phase 2, and Phase
1 introduces `PolySlot` with only `pt`/`int_val` while the quotation literal stays
rejected; `quot` is added in Phase 2 with its writer and reader in the same
commit).

`QuotRef` is the poly twin of the concrete `QuotId`/`prov.quotations` pair
(`src/check.rs:181`): an index into an append-only per-body interner
(`Vec<QuotLit>`) recording each encountered literal's `body: &[Term]`, inline
flavour `bool`, resolved `Option<QuotAnnot>` (whose `variant_tag` gives the arm
tag), and span. The interner **lives as a field on `PolyScope`** (already
`&mut`-threaded through the whole walk, so it matches "per-body" and needs no 7th
parameter across the six functions). It is **not** lock-step with the stack (it never
shrinks on a pop), so it is not a fourth parallel vector; it is the same shape as
`prov.quotations`. A `PolySlot` stays cheap to clone (an index moves under `swap`,
L3).

Two stack-reading helpers beyond the six threading functions also change with the
representation: `prune_dead_borrows` (`poly.rs:94`) and `live_borrow_of` (`:111`)
take `&[PolyType]` today and become `&[PolySlot]`, reading `slot.pt`. They fall
inside the ~54-site count but are not among the six functions named above.

**Phase 1 exit:** behaviour byte-for-byte unchanged. The quotation literal still
returns the `poly.rs:513` message; every existing `poly.rs` and `phase7_slice3a`
test passes untouched.

### R2 quotation-literal admission and eliminator dispatch

Replace the `TermKind::Quotation` rejection at `poly.rs:505-513` with **admission**:
push a `PolySlot { pt: <a compile-only marker, not a real PolyType>, int_val: None,
quot: Some(QuotRef(..)) }`. The slot carries **no** `PolyType` identity (L/D1: two
bodies with one effect are one `PolyType`, and a placeholder would leak into output
unification/`Subst`/mangling). Concretely the `pt` field for a quotation slot is a
dedicated `PolyType` marker that every predicate treats as "not a value type":
`poly_is_copy` → `false`, `is_reference_slot` → `false`, and any attempt to use it
as a data operand (arithmetic, construction, output at word exit) is a **located**
rejection, not a silent pass. A quotation slot reaching word exit unconsumed is the
materialisation rejection (L2, R4).

`poly_call_term` gains an eliminator intercept **before** the ordinary `env`
dispatch (mirroring `check_term`'s intercept at `src/check/terms.rs:495`, and R3's
ordering discipline in slice3a: a single registered concrete candidate would
otherwise commit). One difference the port already handles correctly: `terms.rs:495`
reads the registry from `poly.eliminators` (a precomputed `PolyCtx` field), but
`poly_call_term` has **no `PolyCtx`**, so it builds `eliminator_registry(enums)`
(`declarations.rs:1469`) locally from its `enums` param instead. Then, if
`name` resolves to an `EnumId`, run the abstract eliminator check
(`poly_eliminator_call`, new). That routine ports `check_eliminator_call`
(`src/check.rs:2193`) with these substitutions:

- Arms are collected off the top of the `Vec<PolySlot>` by their `quot` field
  (each must be `Some` and tagged), not via `resolve_quotation_operand` over a
  `Slot`. Untagged/forwarded-abstract arm → the located
  `eliminator_untagged_arm` diagnostic, reused.
- The scrutinee slot's `pt` must be `PolyType::Concrete(Type::Enum(id, ..))`. An
  abstract scrutinee is a **located** rejection naming that an abstract enum
  scrutinee needs a bound (OQ2; forward-refs P7.S3d). A **reference** scrutinee is
  its own located rejection (`poly_reference_scrutinee_error`): eliminating through a
  ref leaves each arm projecting fields off a borrowed variant, and field projection
  is rejected in every generic body, so those arms cannot be written at all. Arms
  destructure instead, as the delivered shape above does.
- Exhaustiveness, duplicate-arm, unknown-variant, and variant-escape checks reuse
  the concrete diagnostics verbatim (`eliminator_non_exhaustive_error`,
  `eliminator_duplicate_arm_error`, `eliminator_unknown_variant_error`,
  `eliminator_variant_escape_error`).
- Each arm body is checked by **re-entering `poly_walk`** over `(row ++ narrowed
  concrete variant)` (OQ4), producing an abstract exit row. The narrowed variant is
  concrete, so the arm's own input needs no abstract row match. Implementer caveat:
  the narrowed input is a `Concrete(Type::Variant(..))` (or a ref to one), so the poly
  predicates the arm walk runs (`poly_is_copy`, `is_reference_slot`) must handle
  `Type::Variant` for a linear payload — `Type::Variant` has fallen through predicate
  matches in this codebase before. `eliminator_variant_escape_error` is reused so escape
  is caught, but confirm the predicate behaves. **Checked at phase 2 exit, and it does
  not** — see "The `Type::Variant` caveat, answered" below; the guard is exit-row only
  and the in-arm hole is a standing concrete-path bug this slice does not widen. An arm
  that **binds** a local is where R3's poly `Scope::leave` analogue earns its keep (the
  arm-local must-consume check).

### R3 abstract N-arm join (structural, rigid, borrow-unioning)

Port the join from `check_eliminator_call`'s arm loop (`src/check.rs:2443-2483`) in
its **N-arm** form:

- **Depth agreement**: all arms leave the same stack depth or a **located**
  `combinator_branch_output_mismatch_error` (reused).
- **Per-slot structural agreement (L1)**: two arms' exit rows agree iff each
  `PolyType` position is structurally equal under **rigid** type variables — arm A
  `Var(0)` vs arm B `Var(1)`, or `Var(0)` vs `Concrete(i64)`, is a **new located
  error** (`poly_arm_output_disagreement_error`, new), *not* a `Subst` bind. No
  `apply_subst` is introduced in the term walk.
- **Move-state join, with a poly analogue of `Scope::leave` first (`Moves::join` is
  NOT reusable as-is).** `Moves::join` (`engine.rs:453`) iterates the first arm's keys
  and **indexes** `else_arm.states[name]`, so it is sound only when every arm carries an
  **identical** local-name key set: a local present in one arm and absent in another
  **panics** (`HashMap` `Index`). The concrete path survives this only because a
  quotation body is a *block* and `Scope::leave` (`engine.rs:545`) removes arm-bound
  locals (`moves.states.remove` per binding past `depth`) before the join, so every arm
  presents the enclosing key set. The poly walk has **no block scope and no `leave`**:
  `poly_term`'s `Bind` inserts into `scope.moves.states` (`poly.rs:484`) and
  `scope.locals` and nothing removes them. So this slice **adds a poly analogue of
  `Scope::leave`**, run per arm: at arm entry record the enclosing key set; at arm exit
  **first** reject an unconsumed arm-local *linear* value with a **located** error — this
  is the must-consume check the concrete path gets for free from block exit
  (`poly_arm_local_not_consumed_error`, new) — **then** truncate `moves`/`locals` back to
  the enclosing key set. Only then reduce the arms with `Moves::join` (generalized two→N
  by the same `into_iter().reduce(Moves::join)` the concrete path uses at
  `src/check.rs:2443`), writing the joined `Moves` back into the enclosing `PolyScope`.
  **Rejected alternative:** a key-set-tolerant join (skip a missing key rather than index
  it) stops the panic but leaves the linearity hole — an arm-local leak goes unreported
  and an out-of-scope name is written into the enclosing scope — violating the
  linear-spine invariant CLAUDE.md calls load-bearing. The `leave` analogue closes both
  the ICE and the leak; the tolerant join closes only the ICE.
- **Borrow-table UNION (L4)**: union each arm's `PolyScope.borrows` **by place**
  into the enclosing scope's table. A place borrowed on one arm and consumed on
  another, or borrowed at differing mutability across arms, is a **located**
  rejection (`poly_arm_borrow_disagreement_error`, new) — never an erase-to-empty
  that `live_borrow_of` would later read as "no conflict". This is the false-accept
  guard and gets its own test (R5).

Each arm is checked against its own **clone** of the enclosing `PolyScope` (exactly
as `check_eliminator_call` clones `scope` per arm), and the join reconciles the
clones. Type variables are never bound, so no clone diverges on `Subst`.

### R4 located rejections for everything out of scope (no `unknown word` fallthrough)

- **L2 materialisation**: a quotation slot that is stored, returned, or reaches word
  exit unconsumed → `poly_quotation_not_consumed_error` (new), naming the word and
  that only in-body elimination consumes a quotation here. Covers OQ5's vacuity: a
  quotation forwarded toward a capturing position is rejected before any
  capture-admission logic could run.
- **OQ6 deferred family**: `call`, `branch`, `if`, `times`, `tag` applied to a
  quotation slot in a poly body → `poly_quotation_combinator_unsupported_error`
  (new), naming the word and pointing at **P7.S3b-follow**. Not `unknown word`.
- **abstract scrutinee** (OQ2): `poly_abstract_enum_scrutinee_error` (new).

### R5 comment corrections at `poly.rs:505-513`

The rejection is deleted by R2, so the two comments justifying it go with it. When
the surrounding admission code is written, ensure no residual comment repeats
either falsehood:

- The "`poly_term`'s stack is `Vec<PolyType>` … nowhere to hang the `quot` marker"
  reason (`:507`) is **false** and now demonstrably so — `PolySlot.quot` is the
  hang point, and `lits` was already the precedent.
- The "Mirrors the `if`-in-a-polymorphic-body rejection above" back-reference
  (`:510`) points at **deleted code**: `TermKind` has no `If` variant
  (`ast.rs:1793`), only a tombstone at `drop_graph.rs:233`. Remove it.

The adjacent array-constructor rejection's back-reference (`poly.rs:517-527`) is
**accurate** and stays. The array-ctor rejection itself is **not** lifted here
(out of scope — a separate gap of the same shape).

## Out of scope (stated, not silently worked around)

- **The clause-style-body rejection at `poly.rs:196-201` / `:336-341`.** Its subject
  (`WordBody::Clauses`) is scheduled for deletion by **P6.S4**. Not touched, not
  lifted.
- **`call`/`branch`/`if`/`times`/`tag` in a poly body** — P7.S3b-follow (OQ1/OQ6).
  `if` is the expensive, row-typed one. Each gets a located rejection here.
- **The array-constructor rejection** (`poly.rs:517-527`) — a separate gap, not
  bundled.
- **Materialised / escaping / erased quotations in a poly body** (L2); the two
  pre-existing ICEs in that neighbourhood are not this slice's to fix.
- **Mid-body unification of type variables** (L1).
- **Abstract enum scrutinees / enum-kind bounds** — needs P7.S3d.
- **Any lowering change** — probe-verified free.

## Testing

Golden (`tests/phase7_slice3b.rs`):

- `poly_word_eliminates_a_concrete_enum_runs` (R1/R2/R3): the `area_and_keep`
  program builds and runs, `75` then `12` discarded, `'T=i64` carried across both
  arms. This is the exit criterion.
- `poly_eliminator_carries_a_type_variable_across_arms_at_two_instantiations`
  (R3/L1): one body, two instantiations (`'T=i64` and `'T=str`), both correct — the
  N-arm structural join over an abstract row.
- `poly_eliminator_arm_output_type_disagreement_is_error` (L1): one arm leaving
  `Var(0)`, another leaving `Concrete(i64)` → located
  `poly_arm_output_disagreement_error`, asserted on message text (two `Type`s can
  `Display` alike; assert the structural pairing, not just failure).
- `poly_eliminator_arm_borrow_disagreement_is_the_false_accept_guard` (L4): arm A
  takes `&!x`, arm B takes `&!y`; the merged stack must keep **both** records so a
  later use of **each** of `x` and `y` is rejected. The test must assert **both**
  directions (a later `x` use rejected *and* a later `y` use rejected), because the
  union can be half-implemented: "pick arm A" keeps `x` and drops `y`, so a
  `x`-only assertion would not flip. Mutation targets: **both** "pick arm A" (breaks
  the `y` direction) and "pick arm B" (breaks the `x` direction); each must flip its
  direction to a false accept. Constructibility: this leans on the re-entered
  `poly_walk` actually recording an arm-nested `&!x` into `PolyScope.borrows`
  (`poly_borrow_sigil_gap` is about *top-level* `&name`, so an arm-nested `&!x`
  parses) — the implementer must confirm the borrow is recorded before relying on
  the union.
- `poly_eliminator_arm_binds_and_leaks_a_linear_local_is_error` (B1/R3): two arms
  binding the **same-named** linear local — one consumes it, the other leaks it —
  must be a **located** `poly_arm_local_not_consumed_error` from the poly `Scope::
  leave` analogue (the arm-local must-consume check), *before* `Moves::join`. Mutation
  target: stub the must-consume check open → this test must flip to a false accept.
- `poly_eliminator_one_arm_binds_a_local_the_other_does_not_no_ice` (B1/R3): one arm
  binds `| x |` (and consumes it), the other binds nothing. Without the `Scope::leave`
  analogue the arms present `Moves::join` **divergent key sets** and it panics
  (`else_arm.states["x"]`); with the truncation both arms present the enclosing key set
  and the word type-checks. Pins the ICE case.
- `poly_eliminator_non_exhaustive_missing_arm_names_the_variant`,
  `..._duplicate_arm_is_error`, `..._unknown_variant_names_it_and_enum` (R2): the
  reused concrete diagnostics fire on the poly path too — i.e. the poly intercept
  actually *reaches* them. They carry **no** new mutation entry because they reuse the
  concrete `check_eliminator_call` diagnostics already mutation-covered in Phase 6; the
  poly tests confirm the intercept routes to them rather than re-guarding them. Confirm
  the non-exhaustive case is not masked by an arm-collection underflow firing first
  once the wall is lifted.
- `poly_body_materialized_quotation_is_located_error` (L2/OQ5, **word-exit route**): a
  quotation returned / left unconsumed → `poly_quotation_not_consumed_error`, rejected
  before any capture-admission logic could run.
- `poly_body_quotation_as_data_operand_is_located_error` (L2/OQ5, **data-operand
  route**): a quotation slot fed into a struct/array constructor (and a sibling case
  feeding it to arithmetic) → the R2 pt-marker predicate located rejection, a
  *different* path from the word-exit one. This is the escape route OQ5's single
  witness did not cover. Mutation entry below (stub the pt-marker predicate open →
  this test must flip).
- `poly_body_call_on_a_quotation_is_located_error` (OQ6): `[ … ] call` in a poly
  body → `poly_quotation_combinator_unsupported_error` naming P7.S3b-follow, **not**
  `unknown word`.
- `poly_body_tagged_arm_not_adjacent_to_its_eliminator_is_error` (L3): a `swap`
  between two arm literals before `Shape?` is rejected, and the test asserts the
  **poly and concrete paths reject with the same message** — the point being that the
  generic body is not laxer. Pure slot motion (an *untagged* literal shuffled) is
  automatic under `Vec<PolySlot>` and is pinned instead by the unit test
  `poly_quotation_identity_moves_with_the_slot_under_swap`, which no deletable guard
  backs and which is (correctly) absent from the mutation list.
- `poly_eliminator_arm_unconsumed_quotation_reports_the_inner_literal_span` (L2/OQ5,
  **arm-exit route**, the third of three): a quotation left on the stack by an arm →
  `poly_quotation_not_consumed_error` at the *nested* literal's own span, not the
  enclosing arm's, so the diagnostic does not blame a quotation the `Shape?` call in
  fact consumes. Mutation entry below.

Unit tests beside their stage:

- `src/check/poly.rs`: `polyslot_int_val_folds_lits` (R1, an `IntLit` then `Bind`
  round-trips the literal exactly as `lits` did); `poly_quotation_slot_is_not_copy`;
  `poly_arm_join_unions_borrows` (L4 at the unit level);
  `poly_arm_join_rejects_rigid_type_variable_disagreement` (L1);
  `poly_eliminator_arm_leaving_its_own_variant_is_error` (R2 step 5b);
  `poly_eliminator_registry_intercept_precedes_env_dispatch` (R2 ordering, the slice3a
  R3 trap). **Its discriminator (B4):** the eliminator registers a `PolySig` in
  `poly_env` (`enum_eliminator_sigs`, `declarations.rs:1422`) whose arm parameters are
  in enum **declaration order** and would be matched **by slot position**; the
  intercept instead matches arms **by annotation tag** (`( Rect )`/`( Circle )`). The
  test writes its arms in the **reverse** of declaration order (`Shape | Circle |
  Rect`, arms `( Rect )` then `( Circle )`), and it is that reversed order which makes
  the **accept** evidence of tag matching rather than of position matching. Deleting
  the intercept does **not** reach env dispatch on this path at all: `poly_call_term`
  has no `PolyCtx`, so the eliminator's `PolySig` (registered in `poly_env`) is
  unreachable and the call falls through to `unknown word`. The mutation therefore
  flips accept → reject, just not via a positional mismatch; the test carries that
  caveat rather than asserting an env-dispatch diagnostic this path cannot produce.

  **`poly_eliminator_arm_leaving_its_own_variant_is_error` needs a single-variant
  enum**, and that constraint is the whole reason the guard is easy to leave
  uncovered: with two arms, R3's rigid-arm-disagreement check fires on the differing
  exit shapes before the escaping `Type::Variant` is ever looked at, so no two-arm
  program reaches the guard. A one-variant enum is exhaustive with one arm and reaches
  it directly. Stubbing the guard with a linear payload under it builds and
  **double-drops** — the `Type::Variant`-falls-through-`is_copy` hole R2 warns about
  above, so the test carries a linear `Spy` rather than an `i64`.
- `src/check.rs`: confirm `check_eliminator_call`'s existing units
  (`check_eliminator_call_*`) stay green (shared diagnostics).

Mutation-tested guards (each must fail when the arm it guards is deleted; Sooth has
shipped placebo tests five times, so prove each can fail):

- the borrow **union** (L4) — collapse to a single-arm pick in **both** directions
  ("pick arm A" *and* "pick arm B"); the false-accept test must catch each direction;
- the arm-local must-consume check (B1/R3, the poly `Scope::leave` analogue) — stub it
  open, the bind-and-leak test must catch the leak;
- the rigid-disagreement error (L1) — delete the arm, the disagreement test must
  catch it;
- the word-exit materialisation rejection (L2) — stub it open, the unconsumed-
  quotation test must catch it;
- the **arm-exit** materialisation rejection (L2/R4) — mutate it **twice**: stub it
  open, and separately revert its span to the enclosing arm literal's; the arm-exit
  test must flip both times (it asserts the reported line, not just the failure);
- the **variant-escape** rejection (R2 step 5b) — stub it open; the single-variant
  unit test must flip. Verify by *running*, not only by checking: with the guard open
  the linear payload is destructed twice;
- the pt-marker **data-operand** predicate (L2/R2) — stub it open so a quotation slot
  flows into a constructor/arithmetic, the data-operand test must catch it;
- the eliminator intercept ordering (R2/B4) — delete the intercept so env dispatch
  wins, the reverse-arm-order golden must flip from accept to the env-dispatch located
  error;
- the deferred-family rejection (OQ6) — stub it open, `[ … ] call` must then hit
  `unknown word` and the test must catch the changed message;
- `int_val` truncation on `Bind` (R1) — skip the truncate, the fold round-trip test
  must catch the desync.

Regression, green and untouched: `tests/phase7_slice3a.rs` (the `PolySlot` change
threads every `poly.rs` stack site slice3a touches), `tests/phase6_*` eliminator
suites (the concrete `check_eliminator_call` path is shared, not forked),
`tests/qbe_baseline.rs`.

## Implementation

Two phases, each independently committable and green
(`cargo fmt --check && cargo clippy -- -D warnings && cargo test`). No pre-staged
plumbing: the `quot` field and its first reader ship in the same phase, split at
first use.

**Phase 1 — `PolySlot` representation, `lits` folded (R1).** Replace `Vec<PolyType>`

- `lits` with `Vec<PolySlot>` across the six stack-threading functions; `PolySlot`
carries `pt` and `int_val` only. The quotation literal stays rejected at
`poly.rs:513`. Pure refactor: behaviour byte-for-byte unchanged, existing tests
untouched.

**Phase 2 — admission, eliminator dispatch, N-arm join, rejections, comment fixes
(R2/R3/R4/R5).** Add `PolySlot.quot`, replace the rejection with admission, add the
`poly_eliminator_call` intercept, the abstract N-arm join with the borrow union,
the located rejections for the deferred family and materialisation, and the comment
corrections. The R1 golden lands here.

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Replace the poly walk's Vec<PolyType> stack and its parallel lits vector with a Vec<PolySlot { pt, int_val }> across poly_walk/poly_term/poly_call_term and the three other stack-threading functions (poly.rs:382/419/534/986/1148/1432, plus the &[PolyType] reads in prune_dead_borrows/live_borrow_of); delete the lits parameter and its debug_assert length guard. Pure refactor: the quotation literal stays rejected at poly.rs:513, and every existing poly.rs and phase7_slice3a test passes untouched. Unit test: an IntLit then Bind round-trips the folded int_val exactly as lits did.",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Add PolySlot.quot (a QuotRef index into an append-only per-body literal interner, the poly twin of QuotId/prov.quotations); replace the TermKind::Quotation rejection at poly.rs:505-513 with admission of a quotation slot carrying no PolyType identity; intercept eliminator_registry in poly_call_term ahead of env dispatch and port check_eliminator_call (src/check.rs:2193) as poly_eliminator_call over a concrete-enum scrutinee, walking each arm body by re-entering poly_walk over (row ++ narrowed concrete variant); port the N-arm join with rigid type variables (a Var/Var or Var/Concrete arm-output disagreement is a new located error, no mid-body Subst), a poly analogue of Scope::leave per arm (record the enclosing key set at arm entry; reject an unconsumed arm-local linear value at arm exit; then truncate moves/locals) so Moves::join sees an identical key set per arm and neither panics nor leaks a linear arm-local, the Moves::join reduction (check.rs:2443), and a borrow-table UNION by place (a cross-arm borrow disagreement is a new located error, never erased). Add located rejections for a materialised/unconsumed quotation at word exit and for a quotation used as a data operand (the pt-marker predicate route), for call/branch/if/times/tag applied to a quotation slot (naming P7.S3b-follow, not unknown word), and for an abstract enum scrutinee. Correct the two false comments at poly.rs:507/:510. The area_and_keep golden builds and runs.",
      "difficulty": "hard"
    }
  ]
}
```

## Sizing

**L**, comparable to P7.S3a: a mechanical `PolySlot` change across ~54 sites
(Phase 1), plus an eliminator-only dispatch and an N-arm port of the concrete
eliminator join (Phase 2), against **zero** lowering work. OQ1's ruling holds the
slice on the cheaper side — the row-typed `if`/`call`/`times`/`tag` family is
deferred to P7.S3b-follow, so no row-unification-against-an-abstract-stack is
built here.

## Phase 2 exit notes

### The `Type::Variant` caveat, answered: the predicate does not behave

R2 asked the implementer to confirm `poly_is_copy`/`is_reference_slot` handle a
`Type::Variant` with a linear payload. They do not. `is_copy`
(`src/check/builtins.rs`) matches `Type::Struct` and `Type::Enum` and ends `_ =>
true`, so a narrowed variant reads as trivially `Copy` and
`~[ ( A ) dup A> drop A> drop ]` runs the payload's destructor **twice**. Step 5b
guards only the arm's *exit* row, which is what stops the variant leaving the call;
it does nothing about a `dup` **inside** the arm that bound it.

This reproduces on the **concrete** path at this slice's parent commit, so it is not
a P7.S3b regression — the slice makes the same hole reachable from a second path
without widening it. Recorded rather than fixed here: the fix is a `Type::Variant`
arm on the predicate family (`is_copy`, `contains_reference`, the `drop`-import
visibility check in `check_shuffle`), which belongs to whichever slice owns that
family, and probing it needs a linear payload (a scalar-payload enum is `Copy`
anyway and hides the bug).

### Structure signals at phase exit: split deferred, deliberately

CLAUDE.md asks for the split signals to be re-run at phase exit. `src/check/poly.rs`
is 3348 source lines after this phase. Three of the five signals fire (a module doing
several things, high- and low-level code mixed, ~30 diagnostic formatters that never
call each other); two do not (no import divergence — the file has a single `use
super::*`; no circular dependency forcing a split). The decision is to **defer**, for
reasons that are about the split point rather than about the churn:

- The layer-shaped split (`poly/diagnostics.rs`) is the one CLAUDE.md names as wrong
  ("group by responsibility, not by technical layer") and has no precedent here:
  `src/check.rs` carries 40 error formatters beside their checks, `terms.rs` 10,
  `declarations.rs` 17, all interleaved. Moving poly's 30 out would invent an
  organizing principle the rest of the checker does not use.
- The responsibility-shaped split (`poly/eliminator.rs`) would cut a mutual recursion
  — `poly_call_term` → `poly_eliminator_call` → `poly_walk` → `poly_call_term` — across
  a file boundary to move ~430 lines, raising coupling to lower a line count.
- The split point becomes real at **P7.S3b-follow**, which adds `call`/`branch`/`if`/
  `times`/`tag` as a *second* quotation consumer. Two consumers plus their shared arm
  machinery is a responsibility; one consumer is not. Re-run the signals there.

### What the unblocked family can be written against today

Two standing limits, both pre-existing and both verified at this slice's parent
commit, bound the `unwrap_or`/`map_or` family this slice unblocks:

- **A generic word cannot call another generic word** (`: f ( 'T: Copy -- 'T ) g ;`
  → `` error: unknown word `g__m0` ``): poly words are not registered in `env`, and
  `poly_call_term` has no `PolyCtx` to read `poly_env` from. So a combinator written
  here composes **concrete and builtin callees only**.
- **Field projection (`&w`) is rejected in every generic body**, so an arm
  destructures (`Rect>`) rather than projects. This is why a monomorphic twin is
  evidence for operand order and nothing else.

Neither is this slice's to fix; both belong to whichever slice takes generic-to-generic
dispatch.

## Acceptance report

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "Spec written to docs/roadmap/P7/slice3b-spec.md with an anchor table where every file:line was verified against HEAD (divergences from the brief called out: borrow-table lines ~12 low, check.rs:2443 vs merge_arm_output_slot at :2480). The R1 golden was confirmed to fail today with exactly `error: a quotation in the polymorphic body of area_and_keep (line 4) is not yet supported` via `cargo run -- build` (the wall reports the first quotation literal's line, line 4 in the delivered layout), and that message's sole non-test producer is poly.rs:513."
    }
  ],
  "changedFiles": [
    "docs/roadmap/P7/slice3b-spec.md"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "cargo run -q -- build /tmp/aak.sth",
      "result": "passed",
      "summary": "Reproduced the exact rejection message the golden must fix; nothing else fired."
    },
    {
      "command": "grep -n (anchor verification across src/check/poly.rs, src/check.rs, src/ast.rs, src/ir/driver.rs, src/check/declarations.rs, src/check/combinators.rs, src/check/terms.rs, src/check/engine.rs, src/check/drop_graph.rs)",
      "result": "passed",
      "summary": "All cited anchors resolve at HEAD; brief-vs-source divergences recorded in the spec's anchor table."
    }
  ],
  "validationOutput": [
    "area_and_keep probe: `error: error: a quotation in the polymorphic body of `area_and_keep` (line 4) is not yet supported` (line 4 is the first quotation literal; the doubled `error:` prefix is the build driver's own wrapper around the checker string).",
    "poly.rs:513 is the only non-test site producing that message.",
    "check_eliminator_call is reached by name via eliminator_registry ahead of env/combinator paths, confirming OQ1: eliminator dispatch avoids row-typed combinator dispatch."
  ],
  "residualRisks": [
    "OQ1's scope split assumes eliminator dispatch alone satisfies the consumer; if a reviewer wants call/branch in-scope, Phase 2 grows and the P7.S3b-follow deferral must be re-litigated.",
    "The QuotRef-as-interner-index design (poly twin of QuotId/prov.quotations) is specified but not prototyped; a lifetime snag threading a per-body Vec<QuotLit> through poly_walk could push it toward Rc<[Term]>, which would not change behaviour but would change the field type.",
    "P6.S3b runs in another session; OQ2 rules the interaction vacuous for a concrete scrutinee, but if P6.S3b lands a check-time arm-tag change that touches the shared check_eliminator_call, Phase 2's port must be rebased onto it."
  ],
  "noStagedFiles": true,
  "diffSummary": "Adds one new file, docs/roadmap/P7/slice3b-spec.md. No source files touched.",
  "reviewFindings": [
    "no blockers",
    "note: docs/roadmap/P7-stdlib-nostd.md:112 cites the rejection as poly.rs:505-513, which is accurate; the roadmap example word name (`w`) differs from the brief's (`area_and_keep`) but both are illustrative, not load-bearing.",
    "note: the brief's borrow-table anchors (poly.rs:66-71/81-89/90-104) are ~12 lines low against HEAD (struct at :54, prune at :94); corrected in the spec anchor table."
  ],
  "manualNotes": "Ruled on all six open questions explicitly. Biggest lever (OQ1) resolved toward eliminator-only scope with a named follow-up (P7.S3b-follow) for call/branch/if/times/tag. Did not modify any source; verified the golden fails today and every anchor resolves. Left poly.rs:196-201/:336-341 (clause-style rejection, P6.S4) and poly.rs:517-527 (array-ctor) explicitly out of scope."
}
```
