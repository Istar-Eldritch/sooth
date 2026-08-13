# `ir.rs` modularisation — technical spec

Source brief: [ir-modularisation-brief.md](./ir-modularisation-brief.md).

This is **pure structural refactoring**: code motion plus the minimum visibility adjustments (`pub(crate)` / `pub(super)`) required to compile across module boundaries. No behaviour change to lowering, layout computation, or destructor synthesis. Every existing golden, example, and unit test must pass byte-for-byte, unmodified in intent.

## Goal

`src/ir.rs` was 9216 lines (5421 non-test; 3795 test lines across 139 `#[test]` fns). It is the second-largest non-test body in the crate after `check.rs`, with a 0.70 test-to-code ratio. Split it into `src/ir/*.rs` submodules grouped by pipeline stage, following the existing `src/backend.rs` → `src/backend/qbe.rs` precedent (Rust 2021, no `mod.rs` at the `ir` level; `func_builder` uses `mod.rs` because it is itself a directory).

After the split, `src/ir.rs` is a thin shim: `mod` declarations plus `pub(crate) use` re-exports that keep the exact `crate::ir::*` surface reachable, so **no call site outside `ir.rs` changes**.

## The concrete acceptance check (behaviour-preservation proof)

The single objective test that the split preserved behaviour is that the `crate::ir::*` surface stays reachable **from the four in-crate consumer files** (`repl.rs`, `driver.rs`, `check.rs`, `backend/qbe.rs`). This is a crate-internal guard, not a `pub`-vs-`pub(crate)` check: `sooth` ships only a `[lib]` + `[[bin]]` pair with no external dependent, so a name narrowing from `pub` to `pub(crate)` (or to `#[cfg(test)] pub(crate)`) is invisible to every actual caller and does not fail this guard. Fifteen of the forty names below (`Registries`, `StructLayout`, `ArrayLayout`, `Arrays`, `Cells`, `DropOverride`, `DropOverrides`, `EnumLayout`, `Enums`, `FieldLayout`, `Refs`, `Structs`, `build_registries`, `carried_slot_bytes`, `synthesize_aggregate_destructors`) did narrow this way during the split, and `VariantLayout` is `#[cfg(test)]`-gated (reachable only to the in-crate test tree, which is where its one caller lives); this is deliberate crate-internal tightening, not a defect, and the guard below does not (and is not meant to) catch it. Before starting, the surface was snapshotted. Because `ir`'s largest consumer (`backend/qbe.rs`) imports through a multi-line `use crate::ir::{ … }` block that a single-line grep silently undercounts, the snapshot command flattens those blocks:

```sh
{ grep -rhoE "ir::[A-Za-z_][A-Za-z0-9_]*" \
    src/repl.rs src/driver.rs src/check.rs src/backend/qbe.rs
  perl -0777 -ne 'while(/use crate::ir::\{([^}]*)\}/gs){my $b=$1;
    for my $n ($b=~/([A-Za-z_]\w*)/g){print "ir::$n\n"}}' \
    src/repl.rs src/backend/qbe.rs
} | grep -vE "ir::(self|new)$" | sort -u > /tmp/ir-surface-before.txt
```

At spec time this yields **43 lines**, of which **40 are the real re-export contract**. Three are not live code surface and need no re-export:

- `ir::tests` — the test module name.
- `ir::expand_path` — a private `fn` in `destructors`; appears only in a `check.rs` doc comment. Do **not** re-export it.
- `ir::carried_slot_bytes` — `pub fn` in `layout`; the only external mention is a `repl.rs` doc comment. The shim re-exports it anyway (it is on the historical `pub` surface), but the guard does not require a live caller.

The guard passes on these three because pure code motion leaves comment text untouched. The remaining 40 are: `ALLOC_SYMBOL`, `Arity`, `ArrayLayout`, `Arrays`, `BinOp`, `Block`, `BlockId`, `build_registries`, `carried_slot_bytes`, `Cells`, `CmpOp`, `collect_quot_sigs`, `DropOverride`, `DropOverrides`, `EnumLayout`, `Enums`, `FieldLayout`, `FREE_SYMBOL`, `Instr`, `IrFunc`, `IrModule`, `IrType`, `ir_type_of`, `lower`, `lower_instantiation`, `lower_line`, `lower_word`, `OOB_TRAP_SYMBOL`, `quotation_layout`, `QuotSigId`, `QuotSigLayout`, `Refs`, `Registries`, `StructLayout`, `Structs`, `synthesize_aggregate_destructors`, `Terminator`, `TRACE_ALLOC_ENV`, `Value`, `VariantLayout`, `WORD_WIDTH`.

The four external consumers and their usage: `repl.rs` (heaviest, does its own per-line lowering), `backend/qbe.rs` (emit-facing types + layout surface), `check.rs` (only `CmpOp`), `driver.rs` (only `lower`).

## Target module shape

`src/ir.rs` becomes `mod` declarations + `pub(crate) use` re-exports only. Content moves into one file per pipeline stage:

| File | Cluster | Depends on |
|------|---------|------------|
| `types.rs` | `IrModule`, `IrFunc`, `IrType`, `QuotSigId`/`QuotSigLayout`/`quotation_layout`, `ir_type_of`, `Value`, `BlockId`, `Block`, `Instr`, `BinOp`, `CmpOp`, `Terminator`, `Arity`, `Resolver`, five symbol-name `pub const`s | nothing (root) |
| `layout.rs` | `StructLayout`, `field_is_linear`, `DropOverride`/`DropOverrides`, `FieldLayout`, `Structs`, `EnumLayout`, `VariantLayout`, `ArrayLayout`, `Arrays`, `Enums`, `Cells`, `Refs`, `Registries`, `build_registries`, `carried_slot_bytes`, `LayoutBuilder` | `types` |
| `destructors.rs` | `synthesize_aggregate_destructors`, `PathStep`, `recursive_disposal_path`, `find_path`, `expand_path`, struct/enum/cell destructor synthesis | `layout` |
| `func_builder/mod.rs` | `FuncBuilder` + state types (`CarriedSlot`, `LoopStateSnapshot`, `MaterializedQuot`, `EnvCapture`, `EnvPlan`), bookkeeping primitives, and the five shared helpers (see Q2) | `types`, `layout`, `destructors` |
| `func_builder/calls.rs` | `lower_terms`, `lower_term`, `lower_self_tail_combinator`, `lower_call` | `func_builder` siblings |
| `func_builder/word_families.rs` | reference/array/owned-cell/struct-field lowering primitives | `func_builder` siblings |
| `func_builder/quotation.rs` | `materialize_*`, `build_env`, `quotation_captures`, `lower_indirect_call`, `dispatch_on_tag`, `emit_drop`, `pack_bundle`/`unpack_bundle`, `lower_poly_call`, `lower_struct_word`, `lower_enum_word` | `func_builder` siblings |
| `func_builder/control_flow.rs` | `emit_select`, `total_order_key`, `lower_if`, `seal_arm`, `lower_clauses` | `func_builder` siblings |
| `driver.rs` | `lower`, `collect_quot_sigs`, `lower_line`, `lower_word`, `lower_instantiation`, `bind_env_capture`, `lower_word_parts`, `lower_materialized`, `concrete_effect`, `subst_polytype`, `empty_*` stubs | `layout`, `destructors`, `func_builder` |

Rust permits multiple `impl FuncBuilder { … }` blocks for the same type across files in the same crate, so each `func_builder` sibling file carries its own `impl` block.

## Resolved open questions

### 1. `func_builder` split granularity — split it (multi-file)

The impl block is ~2500 lines, larger than the entire non-test body of `repl.rs` or `parser.rs`. Moving it unchanged into one file reproduces the "too big to navigate" complaint. Split by five method groups: `mod.rs` (struct + state types + bookkeeping primitives) plus sibling files `calls.rs`, `word_families.rs`, `quotation.rs`, `control_flow.rs`. If a group needs a private helper physically in another group's range, either leave that helper in `mod.rs` (visible to all sibling files as an ancestor-private item) or co-locate the two groups, rather than adding cross-file `pub(super)` leakage on a one-off helper.

### 2. Shared helper placement (a correction to the brief's `driver` cluster)

`word_ret_ty`, `bundle_of`, `free_locals_into`, and `body_tail_calls_self` are called by both `driver` and `func_builder`. `is_aggregate` is func_builder-only. Homing them in `driver` would force a `func_builder → driver` import (a circular-dependency signal), since `driver` already depends on `func_builder`. Resolution: **home all five in `func_builder/mod.rs`** and let `driver` import them via `super::func_builder::…`. This keeps the dependency arrow one-way (`driver → func_builder`). `is_aggregate` becomes module-private there. `concrete_effect` and `subst_polytype` are driver-only and stay in `driver`.

### 3. `driver`'s split around `destructors` — keep `lower()` intact

`lower()`'s body calls `synthesize_aggregate_destructors` partway through, which is why the `driver` cluster was two line ranges in the original file. Do **not** split `lower()`: it rejoins as one function in `driver.rs`, calling `destructors::synthesize_aggregate_destructors` across the module boundary. This follows the `check.rs` precedent (`pub fn check()` stayed intact as the crate's top-level entry point).

### 4. Test relocation granularity — mechanical pass, hand-fix, tie-break to subject

Tests are relocated in the final phase, after every source module exists, as one pass. Each `#[test] fn` is moved into the module it exercises, keeping `use super::*` against its new parent. Tie-break for tests that span modules: attribute to the **more specific subject** — the behaviour asserted, not the entry point called. Many lowering tests drive through `lower` / `lower_line` but assert `FuncBuilder` block/back-edge/loop-header shape; those belong in the relevant `func_builder` module. Layout tests go to `layout`; destructor-path tests go to `destructors`. Within `func_builder`, place each test beside the method group it asserts on, falling back to `func_builder/mod.rs` when it spans groups.

### 5. `func_builder` visibility tightening — `pub(super)`, no shim re-export

Nothing outside `crate::ir` names `FuncBuilder` or any of its methods. Once `func_builder` is its own submodule, its parent (`driver`) must still reach `FuncBuilder`, its constructor, and the methods/shared helpers `driver` calls, which requires `pub(super)` (= `pub(in crate::ir)`), not private. The invariant: **no `pub(crate)`, and no `pub(crate) use` re-export in the shim** for anything in `func_builder`. Bookkeeping primitives in `mod.rs` that only sibling group files call can stay private (child modules can see ancestor-private items) — promote to `pub(super)` only when a build error demands it.

### 6. Phase granularity — strict one move per phase, 10 phases

One extraction (or one `func_builder` sub-split) per phase, each a single reviewable/revertible code-motion with a green full checkpoint and empty surface diff. Extracting `func_builder` whole first (Phase 4), then carving method groups out (Phases 5–8), isolates the one risky step — going module-private and confirming zero external callers — into a single phase, leaving the sub-splits as pure intra-module motion.

## Out of scope

- Any behavioural change to lowering, layout computation, or destructor synthesis.
- Splitting any other file (`check.rs`, `repl.rs`).
- Promoting or restructuring the `crate::ir` surface *as seen by the four in-crate consumer files*: their call sites stay at the same paths. `pub` → `pub(crate)` (or `#[cfg(test)] pub(crate)`) narrowing of individual items is in scope and expected wherever a name has no caller outside the crate — `sooth` has no external lib consumer, so this is a visibility tightening, not a surface change.
- Adding new tests: this is code motion; existing tests in their new homes are the proof.

## Exit criteria

- `src/ir.rs` is a thin `mod` + `pub(crate) use` shim; the ~5.4k non-test lines live in `src/ir/{types,layout,destructors,driver}.rs` and `src/ir/func_builder/{mod,calls,word_families,quotation,control_flow}.rs`.
- Every relocated test lives in the module of the function it tests.
- `cargo build && cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- The surface grep diffs empty against `/tmp/ir-surface-before.txt`: the four in-crate consumer files still name exactly the pre-refactor names at the same paths, with zero changes outside `src/ir.rs` and `src/ir/`. `func_builder` names remain unreachable from outside `crate::ir` (no re-export). This guard is crate-internal only; it does not assert `pub`-ness is preserved (see the acceptance-check note above).

## Implementation

Each phase was a single code-motion commit, verified green (`cargo build && cargo fmt --check && cargo clippy -- -D warnings && cargo test`) with an empty surface diff before proceeding.

| Phase | Area | Commits | Key files |
|------|------|--------|----------|
| 1 | Extract `types.rs` — IR data model + symbol-name consts | `62e08d3` | `src/ir/types.rs` |
| 2 | Extract `layout.rs` — layout computation + registries | `dbbd86e`, `d201561` | `src/ir/layout.rs` |
| 3 | Extract `destructors.rs` — destructor synthesis | `dfa1749` | `src/ir/destructors.rs` |
| 4 | Move `FuncBuilder` + state types + shared helpers into `func_builder/mod.rs` (hard) | `01d46bf` | `src/ir/func_builder/mod.rs` |
| 5 | Carve `calls` method group | `2667d90` | `src/ir/func_builder/calls.rs` |
| 6 | Carve `word_families` method group | `28acb9b` | `src/ir/func_builder/word_families.rs` |
| 7 | Carve `quotation` method group | `40cabf8` | `src/ir/func_builder/quotation.rs` |
| 8 | Carve `control_flow` method group | `8a5203f` | `src/ir/func_builder/control_flow.rs` |
| 9 | Extract `driver.rs` — `lower()` kept intact, plus all driver functions | `52678a7`, `e9f1462`, `eae7f32` | `src/ir/driver.rs` |
| 10 | Relocate all `#[test]` fns into subject modules + final full-crate check | `21cfa5d`, `eda7fb0` | all `src/ir/` modules, `src/ir/test_helpers.rs` |

The `d201561`, `e9f1462`, `eae7f32`, and `eda7fb0` commits address review feedback within their respective phases. Phase 10 also introduced `src/ir/test_helpers.rs`, a shared test-utility module not anticipated in the original module shape table.
