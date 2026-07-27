# Phase 3 Slice 4 — Generalized recursive disposal (condensed, as delivered)

Brief: [phase3-slice4-brief.md](./phase3-slice4-brief.md). Base: `main` @ `6f22576`.
Status: **delivered** across three phases (`281d61c4`/`2e30af1c`/`41556acd`, `4e8da83d`, `e5b6e97e`).

## Problem

Slice 3's fused destructor loop only recognized a `^Self` field declared directly on the
enclosing type (`recursive_loop_field`'s exact match). Three legal cyclic shapes therefore
kept the O(depth) recursive path and segfaulted at ~100,000 nodes on an 8 MB stack:

- **wrapper struct**: `type: Wrap v i64 next ^List ; type: List | Nil | Cons w Wrap ;`
- **`^^Self`**: `type: L | Nil | Cons n i64 next ^^L ;`
- **mutual cycle**: `type: A | ANil | ACons x i64 next ^B ; type: B y i64 z ^A ;`

## Detection (R1–R4)

`recursive_loop_field` is replaced by `recursive_disposal_path` (src/ir.rs:945), a
backtracking walk over the static type graph. Direct self-recursion is its base case, not
a preserved fast path.

**`PathStep`** (src/ir.rs:909) is a *tree*, not a flat list:

- `Project { field }` — byval aggregate field projection (`field_value`, no free)
- `Unwrap { field, cell }` — `load_owned_payload` + `free`; names the field index (a
  struct may hold two same-typed cells) and the cell; `field: None` when the current type
  *is* the cell (`^^Self`'s inner step)
- `Branch { enum_id, variants: Vec<Option<Vec<PathStep>>> }` — a tag dispatch, at entry or
  mid-path. `None` = this variant terminates; `Some(steps)` = it continues. **More than one
  variant may be `Some`**: variants are mutually exclusive at runtime, so each gets its own
  independent back-edge. A `Branch` is always the last step of its sequence.

Example paths: direct `^Self` struct → `[Unwrap]`; wrapper-struct `Cons` continuation →
`[Project, Unwrap]`; `^^Self` `Cons` continuation → `[Unwrap, Unwrap]`; mutual enum/enum →
two nested `Branch` steps.

**Search shape.** Two mutually recursive operations, seeded as `expand(Self, Self, {Self})`
— never `find_path`, which would trivially match the entry type against itself:

- `find_path` owns the target-match check (which **must** precede the visited-prune check,
  since `Self` is pre-seeded into `visited`) and the visited prune. `visited` is
  `Vec<IrType>`, pushed/popped per attempt, so an abandoned branch cannot poison a sibling.
- `expand` handles `Struct` (via `expand_fields`), `Enum` (every variant tried
  independently with a *copy* of `visited`; **all** successes kept), `OwnedCell` (recurse
  into payload, prepend `Unwrap`). No `Array` case: `[^T N]` is still rejected by Slice 2's
  linear-array-element rule (R11).
- `expand_fields` tries a **direct `^target` field first**, in reverse declaration order,
  before the general reverse scan. *(Implementation correction, `2e30af1c`: without this
  tier a later-declared field reaching `target` only indirectly wins, lengthening the path
  and defusing a loop that fused before.)* Otherwise: reverse declaration order,
  last-tried-first-success, decided *after* the sub-walk succeeds.

**D1 (unchanged).** At a **struct** level exactly one recursive edge is chosen; the rest
fall back to ordinary recursion, because two struct fields can be simultaneously live in
one node. An **enum**'s variants cannot, so multi-`Some` `Branch` is explicitly not this
restriction — rejecting it would regress `type: T | Nil | X n i64 next ^T | Y m i64 next
^T ;`, which already disposes in constant stack on the base commit.

The walk is detection-time only (R4). No visited set, cycle guard, or double-free check
exists at disposal time.

## Codegen (R5–R10)

- **R5** — every path level (entry or intermediate) drops its own non-continuing fields via
  `emit_drop` in declaration order. At a `Branch`, `dispatch_on_tag` is reused as-is; each
  `None` variant drops its fields and `ret`s, each `Some` variant drops its non-path fields
  in its own arm and continues. `drop_B` dispatches on `A`'s tag *mid-loop*, since `B` has
  no tag of its own.
- **R6** — every type on a cycle gets its own fused loop, the same cyclic path rotated to
  start at its own shape. No synthesized destructor calls another to traverse the cycle:
  synthesis bypasses the tail-call machinery entirely (`lower_call`'s back-edge transform
  is gated on `name == self.cur_word_name`; synthesized destructors carry an empty name;
  `emit_drop` emits an unconditional `Call`), so a `drop_B`-calls-`drop_A` design is always
  O(depth). Phase 2 Slice 6's tail-call lowering never sees these functions.
- **R7** — one iteration is one full trip around the path; no inner loop. Forced, not
  chosen: `FuncBuilder` has a single `header` and flat `header_phis`/`back_edges`, so
  nested loops are not representable. A path may end in `Project`, in which case the
  back-edge carries an interior pointer into that slot.
- **R8** — copyout ordering, per **unwrap site** (corrected in `af37e6f9` from "per distinct
  aggregate type"): each site gets its own hoisted `push_alloc` slot, reused every
  iteration. All reads of a slot — byval projection, field drop, tag dispatch, and the
  header phi itself when the path ends in `Project` — must be emitted before the unwrap
  that overwrites it. `^^Self` has exactly **one** hazardous step: the first unwrap reads a
  scalar pointer via a plain `FieldLoad` with no slot.
- **R9** — no aliasing or double-free guard, and none needed: `^T` ownership is exclusive
  and setters are whole-value transforms, so every type-level cycle still yields a
  value-level tree; `load_owned_payload` blits into a fresh slot rather than aliasing.
- **R10** — the `b.terminated = false`-after-`start_block` discipline is applied at **every**
  freshly-started block the path introduces, not just the entry dispatch, so the trailing
  `if !b.terminated { seal_block(Ret) }` stays correct. Missing it at an intermediate
  dispatch produces a duplicate `BlockId`, which `qbe` rejects (`multiple definitions of
  block @start`). Exit-less-ness now depends on whether *any* enum on the path has a
  reachable terminating variant.

## Delivery, as executed

1. **Detection + gated wiring.** `recursive_disposal_path` wired into both synthesis
   functions behind a shape-identity gate, so no behaviour changed yet. Gating on the flat
   `Vec<PathStep>` *length* is wrong (every enum-rooted path is one top-level `Branch`,
   length 1). Gating the whole `Branch` wholesale is **also** wrong (`41556acd`, the spec's
   own error caught by implementation): `type: Wrap v i64 n ^E ; type: E | Nil | Direct d
   ^E | Indirect w Wrap ;` fuses `Direct` on the base commit, and a wholesale gate defuses
   it on its sibling's account — verified as a fresh 1,000,000-node SIGSEGV. The gate is
   therefore **per variant**: each variant whose continuation is exactly `[Unwrap]` loops.
2. **General loop codegen** (`4e8da83d`), gate removed, applied uniformly — `drop_A` and
   `drop_B` each discover and emit their own rotated loop with no mutual special-casing.
   **Both Slice 3 golden inversions landed here, not in phase 3**: phase 2 makes those
   assertions false the moment it lands, so deferring them would ship a red suite. Deep
   assertions only; the mutual case's small-chain trace was verified unchanged (the
   generalized loop walks the cycle in the same pre-order the recursive path did), and
   `indirect_recursion_shapes_remain_depth_limited`'s left-leaning-tree assertion was left
   untouched as the surviving proof D1 held.
3. **Proofs and docs** (`e5b6e97e`): the 1,000,000-node constant-stack goldens (each
   verified to SIGSEGV on the base commit per R13, each with a self-tail-recursive builder
   per R14, plus one `ulimit -v` variant), near-miss/backtracking regression, and the
   ROADMAP rewrite (Slice 4 → done; Slice 3's now-false depth-limit blurb corrected; the
   Next-action line's worklist item moved to Phase 6).

## Test discipline (R12–R14)

Runnable goldens, never IL-string assertions; distinct `__spy` tags at every level holding
an aggregate slot; full ordered stdout via `assert_eq!`. At least one golden per shape
declares its continuing field **before** its spy fields, since a declaration-order-only
golden cannot distinguish correct emission from an R8 violation.

Criteria 2–4 are **correctness-preservation**, not mechanism proofs: all three shapes
already produced those traces on the base compiler. Only the constant-stack goldens prove a
loop exists.

## Criterion → test map

| # | criterion | test |
|---|---|---|
| 1 | path found for all three probes, the composed shape (`P`/`W`/`Q`), a two-independently-recursive-variant enum, and the enum/enum mutual shape; `None` for non-cyclic and misleading shapes; direct edge preferred over a later indirect one | `recursive_disposal_path_finds_indirect_nested_mutual_and_composed_cycles`, `recursive_disposal_path_finds_multi_variant_and_enum_enum_mutual_cycles`, `recursive_disposal_path_rejects_non_cyclic_and_misleading_shapes`, `recursive_disposal_path_prefers_direct_field_over_later_indirect_one` (unit, src/ir.rs) |
| 2 | wrapper-struct list, traced, cell field declared before spy | `wrapper_struct_recursive_list_disposes_in_expected_order` |
| 3 | `^^Self` list, traced, single hazardous unwrap | `double_cell_recursive_list_disposes_in_expected_order` |
| 4 | mutual chain from both roots, traced | `mutual_recursive_chain_disposes_from_both_directions` |
| 4b | multi-`Some` `Branch` codegens (small N alone cannot catch a collapse-to-one regression) | `multi_variant_recursive_enum_disposes_in_expected_order` |
| 4c | 1M alternating-variant chain, `ulimit -s 1024` — a **preservation** golden (already passes on base) | `deep_multi_variant_enum_disposes_in_constant_stack` |
| 5–7 | 1M wrapper-struct, 1M `^^Self`, 1M mutual from both roots (the `drop_B` one is R6's sole discriminator), plus a memory-bounded variant | `deep_wrapper_struct_list_disposes_in_constant_stack`, `deep_double_cell_list_disposes_in_constant_stack`, `deep_mutual_chain_disposes_in_constant_stack_from_a`, `deep_mutual_chain_disposes_in_constant_stack_from_b`, `deep_recursive_chain_disposes_within_bounded_memory` |
| 8 | exit-less all-struct two-type cycle, and one with a byval wrapper hop, compile without a duplicate block label (compilation only; both uninhabited) | `all_struct_recursive_cycle_destructor_compiles`, `all_struct_cycle_with_wrapper_hop_destructor_compiles` |
| 9 | mid-loop dispatch terminates with the base case declared on **either** side of its continuing sibling | `intermediate_dispatch_with_base_case_declared_first_terminates_correctly` |
| 11 | near misses stay straight-line; backtracking past a misleading last field; two unrelated self-recursive types | `non_recursive_cell_shapes_are_not_treated_as_recursive`, `recursive_disposal_path_backtracks_past_a_misleading_last_field`, `two_unrelated_self_recursive_types_dispose_independently` |
| 10 | no regression; examples and REPL goldens byte-identical, except the two deliberately inverted Slice 3 assertions | existing suite |

## Known follow-up

`mutually_recursive_types_dispose_on_recursive_path` and
`indirect_recursion_shapes_remain_depth_limited` still carry names describing their
*pre-inversion* behaviour. The rename was specced for phase 3 and was **not** done. No
example dogfood was added either: the 1,000,000-node goldens discharge the constant-stack
claim and the byte-identical example sweep was judged not worth the slowdown.

## Out of scope

Worklist-based disposal for branching structures (Phase 6): concretely a **struct** with
more than one simultaneously-live recursive field. Compiler-provided `Option`/`Result`
(Phase 4 generics). Pointer arithmetic. Second-class refs (Slice 5), refcounting (Slice 6),
user-definable destructor bodies (Slice 7), growable buffers (Phase 6 `alloc`).
