# `ir.rs` modularisation (brief)

Not a ROADMAP slice — internal structural hygiene, the same motivation as the `check.rs`
split (`docs/check-modularisation-brief.md`): the file has grown past what's navigable for
humans and agents. No behaviour change is in scope: every existing golden, example, and unit
test must still pass, unmodified in intent, after the split.

**Timing / concurrent work note:** `check.rs`'s own modularisation is mid-flight on
`impl/check_modularisation_spec-2608120153` (7 of ~12 clusters extracted as of this writing:
`builtins`, `audits`, `declarations`, `drop_graph`, `engine`, `word_entry`, `terms` landed;
`poly`, `combinators`, `captures`, `operators`, `word_families`, and test relocation remain).
It has not merged to `main`. This is a different file with no import relationship in either
direction on the `check.rs` split's own clusters, so there is no hard blocker — but it is
concrete, working precedent for the pattern below, landed within the last day. Recommend
sequencing (finish or substantially land one before deep-diving the other) purely to keep
review load and merge-conflict surface down, not because of any technical dependency.

## Recon: measured against the built compiler

**1. `ir.rs` is 9216 lines; `mod tests` starts at line 5422, so 5421 lines are non-test, 3795
are test.** For comparison (from the `check.rs` brief, `ir.rs`'s own non-test count restated
here for the current file): `check.rs` non-test is 11158 (mid-split, shrinking); `repl.rs`
non-test is 3242; `parser.rs` non-test is 2235. So `ir.rs`'s non-test body is the second
largest in the crate after `check.rs`. Its test-to-non-test ratio (0.70) is markedly higher
than `check.rs`'s (0.46) — proportionally, more of this file is tests than `check.rs` had.

**2. Two precedents exist for the target shape.** `src/backend.rs` (4 lines, `pub mod qbe;`)
with real content in `src/backend/qbe.rs` — a thin shim over one submodule. `check.rs`'s
in-flight split is the closer precedent: `src/check.rs` becomes a thin `mod`-declarations +
`pub(crate) use`-re-exports shim, `src/check/*.rs` holds one file per cluster. `ir.rs` should
follow the same shim shape.

**3. The file's shape is a linear pipeline, not a dependency star like `check.rs`'s.**
`check.rs` had ~12 clusters, most of them mutually independent, all hanging off one shared
`engine`. `ir.rs` instead has each stage depending only on the stage(s) before it, with
nothing feeding back upward. Measured clusters, by line range in the current file:

- **`types`** (L1–46, L1025–1170): `IrModule`, `IrFunc`, `IrType`, `QuotSigId`/
  `QuotSigLayout`/`QuotLayout`/`quotation_layout`, `ir_type_of`; `Value`, `QuotId`, `BlockId`,
  `Block`, `Instr`, `BinOp`, `CmpOp`, `Terminator`, `Arity`, `Resolver`; the five `pub const`
  symbol names (`WORD_WIDTH`, `OOB_TRAP_SYMBOL`, `ALLOC_SYMBOL`, `FREE_SYMBOL`,
  `TRACE_ALLOC_ENV`). The IR's own data model — no dependency on anything below it.
- **`layout`** (L277–1024, ~750 lines): `StructLayout`, `field_is_linear`, `DropOverride`/
  `DropOverrides`, the three `*_drop_symbol` helpers, `FieldLayout`, `StructWord`, `Structs`
  (+ `impl Structs`), `EnumLayout`, `VariantLayout`, `ArrayLayout`, `Arrays`, `EnumWord`,
  `Enums`, `round_up`/`scalar_size_align`/`scalar_size_align_ww`, `carried_slot_bytes`,
  `Cells`, `Refs`, `Registries`, `build_registries`/`build_registries_ww`, `LayoutBuilder` +
  its impl. Computes memory layout of structs/enums/arrays/cells from the typed AST. Depends
  only on `types`.
- **`destructors`** (L1537–2008, ~470 lines): `synthesize_aggregate_destructors`, `PathStep`,
  `recursive_disposal_path`, `find_path`, `expand_path`, `expand_fields`, `prepend`,
  `synthesize_struct_destructor`(`_override`), `synthesize_enum_destructor`,
  `synthesize_cell_destructor`. Depends on `layout` (`Structs`/`Enums`/`Cells`/`FieldLayout`),
  not on `func_builder`.
- **`driver`** (L1170–1536 and L2009–2710, ~1600 lines, split around the `destructors` block
  because `lower()` calls into destructor synthesis partway through its own body): `lower`,
  `collect_quot_sigs`, `lower_line`, `word_ret_ty`, `bundle_of`, `concrete_effect`,
  `subst_polytype`, `lower_word`, `lower_instantiation`, `bind_env_capture`,
  `lower_word_parts`, `lower_materialized`, the four `empty_*` stub helpers, `is_aggregate`,
  `free_locals_into`, `body_tail_calls_self`. The module-level orchestration: builds
  registries, synthesizes destructors, and calls into `func_builder` per word body. Depends
  on `layout`, `destructors`, and `func_builder`.
- **`func_builder`** (L2711–5421, ~2710 lines): `FuncBuilder` + its ~90-method impl block,
  plus the small state types only it uses (`CarriedSlot`, `LoopStateSnapshot`,
  `MaterializedQuot`, `EnvCapture`, `EnvPlan`). Depends on `types`, `layout`, and
  `destructors` (calls drop symbols via `emit_drop`). Nothing outside `func_builder` depends
  on it except `driver`, which calls it once per word body.

  This alone is bigger than the entire non-test body of `repl.rs` or `parser.rs`, and even
  moved into its own file unchanged, it reproduces the exact "too big to navigate" complaint
  that motivated this split. Measured method groups within the current single impl block, by
  absolute line range (grouped by what they touch, not just proximity):
  - **bookkeeping** (L2919–3227, ~310 lines): `new`, `fresh_value`, `value_type`,
    `fresh_block`, `push_instr`, `push_alloc`, `save_loop_state`, `restore_loop_state`,
    `seal_block`, `start_block`, `reopen_block`, `reseal_block_at`, `begin_loop`,
    `finalize_loop` — the value/block/loop-state primitives everything else calls.
  - **calls** (L3228–3920, ~695 lines): `lower_terms`, `lower_term`,
    `lower_self_tail_combinator`, `lower_call` — `lower_call` alone is ~500 lines, the
    single biggest method in the file, dispatching every call-term kind.
  - **word_families** (L3921–4656, ~735 lines): `push_reference`/`referent_of`,
    `lower_reference_word`, `lower_borrow`, `lower_access_word`, `alloc_struct`/`alloc_enum`/
    `alloc_array`, `array_parts`/`array_id_of`/`value_size`/`elem_addr`/`store_elem`,
    `lower_array_word`, `cell_id_of`, `alloc_aggregate`, `load_owned_payload`,
    `drop_level_fields`, `emit_unwrap`, `emit_path_steps`/`emit_field_level`/`emit_branch`,
    `store_owned_payload`, `lower_owned_cell_word`, `bounds_check`, `field_ptr`/
    `field_aggregate_value`/`store_field`/`field_value`/`load_field_onto_stack` — the
    reference/array/owned-cell/struct-field lowering primitives, tightly coupled to each
    other (array and owned-cell word lowering both bottom out in the same field/elem
    helpers).
  - **quotation** (L4657–5122, ~465 lines): `materialize_if_phantom`, `materialize_quot_value`,
    `build_env`, `quotation_captures`, `materialize_join_quotations`, `lower_indirect_call`,
    `dispatch_on_tag`, `emit_drop`, `pack_bundle`/`unpack_bundle`, `lower_poly_call`,
    `lower_struct_word`, `lower_enum_word`.
  - **control_flow** (L5123–5419, ~300 lines): `emit_select`, `total_order_key`, `lower_if`,
    `seal_arm`, `lower_clauses`.

  These five groups sum to 2505 lines, matching the impl block's measured span. **Rust allows
  multiple `impl FuncBuilder { .. }` blocks for the same type across different files in the
  same crate** (no orphan-rule issue — same crate, same type), so splitting by method group
  into separate files under one `func_builder/` directory is mechanically available, not just
  a labelling exercise. This grouping is read from method signatures and call proximity, not
  a full read of every body — the spec should confirm each boundary by reading the code, the
  same caveat the `check.rs` brief gave its fuzzier clusters.

**4. `ir::`'s external surface is used from exactly four files, asymmetrically.** Grepping
every `ir::`/`crate::ir::` reference outside `ir.rs`:

- `src/repl.rs` — by far the heaviest consumer, because the REPL does its own incremental
    per-line lowering: `build_registries`, `lower`, `lower_word`, `lower_instantiation`,
    `lower_line`, `synthesize_aggregate_destructors`, `collect_quot_sigs`, `Structs`, `Enums`,
    `Arrays`, `Cells`, `Registries`, `DropOverride`, `DropOverrides`, `FieldLayout`,
    `VariantLayout`, `IrType`, `Instr`, `ir_type_of`, `quotation_layout`, `WORD_WIDTH`,
    `Arity`. Touches almost the entire `layout`/`driver` surface plus `types`.
- `src/backend/qbe.rs` — the emit-facing surface: `ArrayLayout`, `BinOp`, `BlockId`,
    `CmpOp`, `EnumLayout`, `Instr`, `IrFunc`, `IrModule`, `IrType`, `QuotSigId`,
    `QuotSigLayout`, `StructLayout`, `Terminator`, `Value`, `ALLOC_SYMBOL`, `FREE_SYMBOL`,
    `OOB_TRAP_SYMBOL`, `TRACE_ALLOC_ENV`, `WORD_WIDTH` (plus, test-only, `lower`,
    `lower_line`, `Arrays`, `Cells`, `Enums`, `Refs`, `Registries`, `Structs`) — all `types` +
    `layout`, nothing from `func_builder` or `destructors` directly.
- `src/check.rs` — only `CmpOp`, at 3 sites.
- `src/driver.rs` — only `lower`, at 2 sites.

  Note `func_builder` itself has **zero external callers** — nothing outside `ir.rs` names
  `FuncBuilder` or any of its methods. It can be made fully module-private (no `pub` needed on
  the struct or its methods beyond what `driver` needs) regardless of how it's split.

**5. Tests are 139 `#[test]` fns across 3795 lines, one flat `mod tests` block today**,
same shape as `check.rs`'s pre-split test module. Per `CLAUDE.md`'s convention, tests should
move with the function they exercise into that function's new submodule. A skim of test names
suggests most exercise `func_builder` (lowering behaviour: `lower_*`, `synthesize_*`,
back-edge/loop-header shape assertions), a meaningful minority exercise `layout` directly
(`struct_layout_*`, `enum_layout_*`, `array_layout_*`, `carried_slot_bytes_*`), and a few
exercise `destructors` directly (`recursive_disposal_path_*`, `synthesized_*_destructor_*`).
This needs the same real attribution pass the `check.rs` brief called for, not a guess — some
tests (e.g. the `struct_linearity_agrees_across_the_checker_and_both_lowering_folds` test)
plausibly span more than one cluster.

## What this brief treats as settled

- Target shape: `src/ir.rs` (thin, `mod` declarations + `pub(crate) use` re-exports) +
  `src/ir/{types,layout,destructors,driver}.rs` + a `func_builder` submodule holding the
  lowering engine — either `src/ir/func_builder.rs` as one file, or `src/ir/func_builder/
  {mod,bookkeeping,calls,word_families,quotation,control_flow}.rs` per section 3's method
  groups, whichever the spec decides after reading the actual bodies. Working names; the spec
  may rename or merge/split further once the code is in front of the implementer.
- No behaviour change: every existing test (relocated) and every existing golden/example
  passes byte-for-byte on `cargo test`. Pure refactor — no new tests required beyond what
  exists.
- Incremental, checkpointed extraction: one cluster per phase, `cargo build && cargo fmt
  --check && cargo clippy -- -D warnings && cargo test` green before moving to the next, one
  commit per phase. Not a single diff.
- Extraction order follows the pipeline in section 3, leaves first: `types` and `layout` carry
  no risk of breaking anything downstream and can move without touching `func_builder`;
  `destructors` next (depends only on `layout`); `func_builder` next (depends on `types` +
  `layout` + `destructors`, but nothing downstream depends on it except `driver`); `driver`
  last among the code clusters (it's the one thing that touches all four others); test
  relocation and a final full-crate check last.
- No public API changes: `ir::` stays exactly as reachable as today from `repl.rs`,
  `backend/qbe.rs`, `check.rs`, and `driver.rs` — the four-file surface in section 4 stays at
  the same paths. `func_builder`'s types/methods, having zero external callers, should end up
  module-private (not even `pub(crate)`) if they aren't already, tightening rather than
  preserving over-broad visibility — flag any such tightening explicitly in the phase that
  does it, since it's the one place this refactor could in principle change what compiles
  from outside (nothing should, since nothing external names it, but the phase should say so
  after checking, not assume).

## Open questions for the spec

- **`func_builder` split granularity** — one file vs. the five method-group files in section
  3. The method groups are evidence-suggestive from signatures and proximity, not verified by
  reading every body; the spec should confirm boundaries hold (no group secretly needs a
  private helper that lives in another group's range) before committing to multi-file, and
  should say which it picked and why.
- **`driver`'s split around `destructors`** — `lower()`'s body currently calls into
  `synthesize_aggregate_destructors` partway through; when `destructors` moves to its own
  file, decide whether `driver`'s two now-separated line ranges (L1170–1536, L2009–2710)
  become one `driver.rs` file with an internal call into `destructors::`, or whether that
  call boundary suggests `lower()` itself should be split (e.g. registry-building split from
  per-word lowering orchestration). No strong preference here; check.rs's `pub fn check()`
  driver stayed intact and became the crate's top-level entry point — the analogous move here
  is keeping `lower()` intact in `driver.rs` unless reading the code surfaces a reason not to.
- **Test relocation granularity** — same choice the `check.rs` brief posed: mechanical
  first-pass attribution by which cluster's public fns a test body references, hand-fix
  ambiguous cases (tests spanning `layout` + `func_builder`, etc.). Pick one and state it.
- **Phase granularity** — the settled order above implies roughly 5–9 phases (up to 4 for
  `types`/`layout`/`destructors`/`driver`, 1–5 for `func_builder` depending on the split
  granularity question above, 1 for tests + final check). State the choice explicitly.
- **Tightening `func_builder` visibility** — confirm via `cargo build` with the type/methods
  made module-private that nothing external actually needed broader visibility (section 3
  claims zero external callers; verify, don't assume).

## Out of scope

- Any behavioural change to lowering, layout computation, or destructor synthesis — pure code
  motion plus the minimum visibility changes required to compile (and the deliberate
  visibility *tightening* on `func_builder` noted above).
- Splitting any other oversized file (`check.rs`, `repl.rs`) — this brief is `ir.rs` only.
  `check.rs`'s split is separate, in-flight work; do not fold the two together.
- Renaming or restructuring the public `crate::ir` surface — section 4's four-file consumer
  list stays put at the same paths.

## Exit (sketch, spec settles the real one)

- `src/ir.rs` is a thin `mod`/re-export shim; the ~5.4k lines of non-test logic live in
  `src/ir/*.rs` (and possibly `src/ir/func_builder/*.rs`), grouped by the clusters in section 3
  (or the spec's refined version of them).
- Every relocated test lives beside the function it tests, in that function's new module.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green throughout, and
  `repl.rs`/`backend/qbe.rs`/`check.rs`/`driver.rs` need no call-site changes beyond import
  paths.
