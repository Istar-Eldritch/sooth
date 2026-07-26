## What shipped

**`isize` (R1, R2)** — `Type::Isize` / `IrType::Isize` as a signed target-width mirror of `usize`; mixing the two is a plain type mismatch; printing routes to `$fmt`, not `$ufmt`. `norm_scalar` became a wrapper over `norm_scalar_ww(ty, word_width)` (`src/backend/qbe.rs:511`), so neither size type carries a literal `64`.

**Recursion rule (R3–R7)** — pinned, not changed. A type cycle is legal iff every cycle passes through at least one `^`, in struct-field and enum-variant position alike (R4). `type_node` now excludes `OwnedCell` deliberately with a comment (`src/check.rs:512`) rather than by fall-through. No uninhabitedness detection (R5); array elements stay by-value edges and `[^T N]` stays rejected by Slice 2's linear-element rule (R6). The by-value cycle diagnostic is unchanged: bare string, no span, unbackticked names (R7's carve-out from R20).

**Disposal ordering (R8–R10)** — globally reversed: every owning cell frees its block *before* dropping the copied-out payload, and disposal is pre-order (a node's own fields drop and its cell frees before descending). Sound because `load_owned_payload` copies the payload out before the block is touched, for every shape including nested `^^T`. The enumerated Slice 2 revision (R9) landed in the same phase: two `tests/phase0.rs` goldens and one `src/ir.rs` unit test inverted, with names, doc comments and assertion messages rewritten, plus `src/ir.rs:985`, `ROADMAP.md:73`, `docs/phase3-slice2-spec.md` and `docs/phase3-slice2-brief.md`.

**Fused iterative destructor (R11–R18)** — `recursive_loop_field` (`src/ir.rs:922`) is a purpose-built pass over `Registries`; it does not reuse the checker's graph, which deletes exactly the `^` edges it needs (R13). A field of type `^T` is a recursive edge iff the cell's payload is the enclosing type. Directly self-recursive structs and enums get one loop via `begin_loop`/`finalize_loop`; `emit_recursive_step` (`src/ir.rs:2025`) handles the non-looped edges. The copyout-ordering invariant holds (R12): every read of the current node is emitted before the copyout that overwrites the reused, entry-hoisted frame slot. The trailing `seal_block` is guarded on `b.terminated`, so a self-recursive struct's exit-less loop does not emit a duplicate `BlockId` (R16). Multi-child types loop on the **last** recursive field and recurse the others (R17). Mutually recursive types stay on the recursive path (R18). Non-recursive destructors keep straight-line synthesis (R15).

**Allocation failure (R19, R19b)** — trap stays; no allocator code touched. `ROADMAP.md` rewritten so it no longer claims this slice introduces optional / non-null pointers; those move to Phase 4 generics.

## Documented limitation (R14)

Constant-stack disposal is guaranteed **only** for directly self-recursive types. Still O(depth), verified as such: indirect cycles through an intervening struct, `^^Self` (excluded by construction, the phi type would not match), left-leaning / non-loop children, mutually recursive types, and the non-direct cycles of a mixed type.

## Criterion → test map (as landed)

| # | criterion | test |
|---|---|---|
| 1 | `isize` declares, computes, prints signed, converts to/from `i64` | `isize_round_trips_arithmetic_and_conversion` |
| 1b | both size types follow word width at 4 and 8 bytes | `norm_scalar_ww_follows_word_width_for_both_size_types` (+ `scalar_size_align_ww` cases in `src/ir.rs`) |
| 1c/1d | `usize`/`isize` mix is an error; declared `isize` output needs explicit conversion | `check_isize_mixed_with_usize_is_error`, `check_isize_declared_output_needs_conversion_is_error` |
| 2/3 | by-value self and mutual cycles rejected, naming the path | `check_recursion_by_value_self_cycle_is_error`, `check_recursion_by_value_mutual_cycle_is_error` |
| 4 | `^` cycle accepted in struct field and enum variant | `check_recursion_cell_cycle_in_struct_field_is_ok`, `check_recursion_cell_cycle_in_enum_variant_is_ok` |
| 4b | array element stays a value edge; `[^T N]` rejected | `check_recursion_array_element_cell_is_cut_then_rejected_as_linear`, existing `check_value_recursion_through_array_element_is_error` |
| 5/6/7 | R8/R9 ordering across scalar, aggregate, nested and unit-test shapes | `recursive_list_disposes_in_expected_order`, `owned_aggregate_payload_frees_before_dropping_fields`, `owned_linear_payload_frees_before_dropping_payload`, `nested_owned_frees_outer_before_inner`, `synthesized_cell_destructor_frees_before_dropping_a_linear_aggregate_payload` |
| 8 | 1M-node list disposes under `ulimit -s 1024`, exit 0 | `deep_list_disposes_in_constant_stack` |
| 9/10 | pre-order disposal; copyout invariant via distinct per-node tags | `recursive_disposal_is_pre_order`, `recursive_destructor_reads_node_before_overwriting_slot` |
| 11 | detection pass does not false-positive on three near-miss shapes | `non_recursive_cell_shapes_are_not_treated_as_recursive` |
| 12/13a/13b | tree disposal; loop takes the last field; 1M right-leaning tree at 1 MB | `recursive_tree_builds_and_disposes`, `multi_child_destructor_loops_on_last_recursive_field`, `deep_right_leaning_tree_disposes_in_constant_stack` |
| 14 | mutual pair disposes correctly, and a 300k chain overflows at 1 MB, proving the fallback | `mutually_recursive_types_dispose_on_recursive_path` |
| 15 | self-recursive struct (uninhabited, exit-less) compiles | `self_recursive_struct_destructor_compiles` |
| 16 | limitation boundary asserted as an asymmetry at 1M / 1 MB | `indirect_recursion_shapes_remain_depth_limited` |
| 17/18 | `examples/list.sth` golden; REPL disposes a residual recursive value at `:quit` | `example_list_matches_golden`, `repl_quit_frees_residual_recursive_value` |
| 19 | 14 prior examples byte-identical | existing suite |

Trace goldens assert full stdout with `assert_eq!`, distinct `__spy` tags per node (a balanced alloc/free trace cannot catch the copyout bug). Stack-bounded criteria use a `run_stack_bounded_golden` helper returning the exit status instead of unwrapping it, since a SIGSEGV would otherwise panic in the harness. R21 discharged: the base compiler segfaults at 100k nodes even at 64 MB.

## Deviations from the spec

- The `usize`-named coercion plumbing was **parameterised, not duplicated**: `is_size_type`, `SlotMatch::{LiteralSizeType, NeedsSizeConversion}`, `PairMatch::NeedsSizeConversion(Type)`, `size_conversion_needed_error(target)`.
- Criterion 4b's test is named `check_recursion_array_element_cell_is_cut_then_rejected_as_linear`, and criterion 4 gained a second test for the enum-variant half.
- `examples/list.sth` builds 10 nodes, consumes 3 via `pop`/`sum-first`, prints `6`, then drops the remaining 7 through the loop.
- **Stale wording**: `ROADMAP.md:84` calls the by-value cycle diagnostic "located". It is not (R7); `visit_recursion` still returns a bare span-less string. Worth a one-line fix.

## Coverage gaps, stated

R13 has no direct criterion (criterion 11 is its proxy). There is still **no OOM-trap runtime test anywhere in the suite**; the `LD_PRELOAD` technique remains recorded but unwritten, so "allocator unchanged" rests on the example sweep.

## Out of scope (unchanged)

Fused loops for indirect recursion and `^^Self` (the natural follow-on). Worklist disposal for branching structures. Fused loops over multi-type cycles. Compiler-provided `Option`/nullable pointers and returning allocation failure (Phase 4 generics). Pointer arithmetic and differences. Zippers. Second-class refs (Slice 4), refcounting (Slice 5), user destructor bodies (Slice 6), `Vec` (Phase 6).
