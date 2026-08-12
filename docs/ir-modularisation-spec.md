# `ir.rs` modularisation — technical spec

Source brief: [ir-modularisation-brief.md](./ir-modularisation-brief.md).

This is **pure structural refactoring**: code motion plus the minimum visibility
adjustments (`pub(crate)` / `pub(super)`) required to compile across module boundaries.
No behaviour change to lowering, layout computation, or destructor synthesis. Every
existing golden, example, and unit test must pass byte-for-byte after each phase,
unmodified in intent.

## Goal

`src/ir.rs` is 9216 lines (5421 non-test; `#[cfg(test)] mod tests` runs from L5422 to EOF,
3795 test lines across 139 `#[test]` fns). It is the second-largest non-test body in the
crate after `check.rs`, and its 0.70 test-to-code ratio is higher than `check.rs`'s. Split
it into `src/ir/*.rs` submodules grouped by the pipeline stages the brief measured, following
the existing `src/backend.rs` → `src/backend/qbe.rs` precedent (Rust 2021, no `mod.rs` at the
`ir` level; `func_builder` uses `mod.rs` because it is itself a directory).

After the split, `src/ir.rs` is a thin shim: `mod` declarations plus `pub(crate) use`
re-exports that keep the exact `crate::ir::*` surface reachable, so **no call site outside
`ir.rs` changes** (only import paths internal to the `ir` tree move).

## Precondition (re-check before starting)

`ir.rs` has no import relationship with `check.rs`'s in-flight split, so there is no hard
technical blocker, but the brief recommends sequencing to keep review and merge-conflict
surface down. Verified at spec time: `main` is at `aa7c28e`; `git worktree list` shows the
primary tree plus `impl/check_modularisation_spec-2608120153` (the concurrent `check.rs`
split, not yet merged). Re-run `git worktree list` before Phase 1. If the `check.rs` split is
still mid-flight, prefer to let it land first; if a new `impl/*` worktree touching `ir.rs`
exists, wait for it to merge.

## The concrete acceptance check (behaviour-preservation proof)

The single objective test that the split preserved behaviour is that the external
`crate::ir::*` surface is unchanged. Before Phase 1, snapshot it. Unlike the `check.rs`
surface, `ir`'s largest consumer (`backend/qbe.rs`) imports through a **multi-line**
`use crate::ir::{ … }` block, which a single-line grep silently undercounts, so the snapshot
command flattens those blocks:

```sh
{ grep -rhoE "ir::[A-Za-z_][A-Za-z0-9_]*" \
    src/repl.rs src/driver.rs src/check.rs src/backend/qbe.rs
  perl -0777 -ne 'while(/use crate::ir::\{([^}]*)\}/gs){my $b=$1;
    for my $n ($b=~/([A-Za-z_]\w*)/g){print "ir::$n\n"}}' \
    src/repl.rs src/backend/qbe.rs
} | grep -vE "ir::(self|new)$" | sort -u > /tmp/ir-surface-before.txt
```

(`ir::(self|new)$` drops the `use crate::ir::{self, …}` self-import and the `LibDir::new`
substring false-positive.) At spec time this yields **43 lines**:

`ALLOC_SYMBOL`, `Arity`, `ArrayLayout`, `Arrays`, `BinOp`, `Block`, `BlockId`,
`build_registries`, `carried_slot_bytes`, `Cells`, `CmpOp`, `collect_quot_sigs`,
`DropOverride`, `DropOverrides`, `EnumLayout`, `Enums`, `expand_path`, `FieldLayout`,
`FREE_SYMBOL`, `Instr`, `IrFunc`, `IrModule`, `IrType`, `ir_type_of`, `lower`,
`lower_instantiation`, `lower_line`, `lower_word`, `OOB_TRAP_SYMBOL`, `quotation_layout`,
`QuotSigId`, `QuotSigLayout`, `Refs`, `Registries`, `StructLayout`, `Structs`,
`synthesize_aggregate_destructors`, `Terminator`, `tests`, `TRACE_ALLOC_ENV`, `Value`,
`VariantLayout`, `WORD_WIDTH`.

This is the authoritative list, not the brief's section-4 prose (which, e.g., lists
`carried_slot_bytes` as a live `repl.rs` call: it is not — the only external mention is a doc
comment). Three of the 43 lines are **not** live code surface and need no re-export:

- `ir::tests` — the test module name.
- `ir::expand_path` — a private `fn` in `destructors`; appears only in a `check.rs` doc
  comment (`check.rs:4445`). Do **not** re-export it.
- `ir::carried_slot_bytes` — `pub fn` in `layout`; the only external mention is a `repl.rs`
  doc comment (`repl.rs:623`). It lands in `layout` and the shim re-exports it anyway (it is
  on the historical `pub` surface), but the guard does not require a live caller.

The guard passes on these three because pure code motion leaves the comment text untouched,
so the grep keeps finding them. The remaining 40 are the real re-export contract.

Re-run the snapshot grep after each phase and diff against `/tmp/ir-surface-before.txt`; it
must stay empty:

```sh
{ grep -rhoE "ir::[A-Za-z_][A-Za-z0-9_]*" \
    src/repl.rs src/driver.rs src/check.rs src/backend/qbe.rs
  perl -0777 -ne 'while(/use crate::ir::\{([^}]*)\}/gs){my $b=$1;
    for my $n ($b=~/([A-Za-z_]\w*)/g){print "ir::$n\n"}}' \
    src/repl.rs src/backend/qbe.rs
} | grep -vE "ir::(self|new)$" | sort -u | diff - /tmp/ir-surface-before.txt
```

If a name would disappear, the fix is a `pub(crate) use` re-export in `ir.rs`, never a
call-site edit. The four external consumers stay at the same paths: `repl.rs` (heaviest,
does its own per-line lowering), `backend/qbe.rs` (emit-facing `types` + `layout` surface),
`check.rs` (only `CmpOp`), `driver.rs` (only `lower`).

## Target module shape

`src/ir.rs` becomes `mod` declarations + `pub(crate) use` re-exports only. Content moves into
one file per pipeline stage. Line ranges below are **recon-time anchors from `aa7c28e`**;
re-locate each item by symbol at extraction time (`documentSymbol` / grep), never by copying
line numbers.

| File | Cluster | Recon range | Depends on |
|------|---------|-------------|------------|
| `types.rs` | `IrModule`, `IrFunc`, `IrType`, `QuotSigId`/`QuotSigLayout`/`QuotLayout`/`quotation_layout`, `ir_type_of`, `Value`, `QuotId`, `BlockId`, `Block`, `Instr`, `BinOp`, `CmpOp`, `Terminator`, `Arity`, `Resolver`, the five symbol-name `pub const`s (`WORD_WIDTH`, `OOB_TRAP_SYMBOL`, `ALLOC_SYMBOL`, `FREE_SYMBOL`, `TRACE_ALLOC_ENV`) | L1–46, L1025–1170 | nothing (root) |
| `layout.rs` | `StructLayout`, `field_is_linear`, `DropOverride`/`DropOverrides`, the three `*_drop_symbol` helpers, `FieldLayout`, `StructWord`, `Structs`, `EnumLayout`, `VariantLayout`, `ArrayLayout`, `Arrays`, `EnumWord`, `Enums`, `round_up`/`scalar_size_align`/`scalar_size_align_ww`, `carried_slot_bytes`, `Cells`, `Refs`, `Registries`, `build_registries`/`build_registries_ww`, `LayoutBuilder` | L277–1024 | `types` |
| `destructors.rs` | `synthesize_aggregate_destructors`, `PathStep`, `recursive_disposal_path`, `find_path`, `expand_path`, `expand_fields`, `prepend`, `synthesize_struct_destructor`(`_override`), `synthesize_enum_destructor`, `synthesize_cell_destructor` | L1537–2008 | `layout` |
| `func_builder/mod.rs` | `FuncBuilder` + state types (`CarriedSlot`, `LoopStateSnapshot`, `MaterializedQuot`, `EnvCapture`, `EnvPlan`), the **bookkeeping** method group, and the shared helpers (see below) | L2711–3227 + shared helpers | `types`, `layout`, `destructors` |
| `func_builder/calls.rs` | `lower_terms`, `lower_term`, `lower_self_tail_combinator`, `lower_call` (~500-line dispatcher) | L3228–3920 | `func_builder` siblings |
| `func_builder/word_families.rs` | reference/array/owned-cell/struct-field lowering primitives (`push_reference`…`load_field_onto_stack`) | L3921–4656 | `func_builder` siblings |
| `func_builder/quotation.rs` | `materialize_*`, `build_env`, `quotation_captures`, `lower_indirect_call`, `dispatch_on_tag`, `emit_drop`, `pack_bundle`/`unpack_bundle`, `lower_poly_call`, `lower_struct_word`, `lower_enum_word` | L4657–5122 | `func_builder` siblings |
| `func_builder/control_flow.rs` | `emit_select`, `total_order_key`, `lower_if`, `seal_arm`, `lower_clauses` | L5123–5421 | `func_builder` siblings |
| `driver.rs` | `lower`, `collect_quot_sigs`, `lower_line`, `word_ret_ty`†, `bundle_of`†, `concrete_effect`, `subst_polytype`, `lower_word`, `lower_instantiation`, `bind_env_capture`, `lower_word_parts`, `lower_materialized`, the four `empty_*` stubs | L1170–1536, L2009–2710 | `layout`, `destructors`, `func_builder` |

† See "Shared helper placement" below: `word_ret_ty` and `bundle_of` are named in the
`driver` row for readability but actually move to `func_builder`.

The nine files above hold the ~5.4k non-test lines. `src/ir.rs` retains only `mod`
declarations and the `pub(crate) use` re-export lines.

## Resolved open questions

### 1. `func_builder` split granularity — split it (multi-file)

Move `FuncBuilder` into its own `src/ir/func_builder/` directory, split by the brief's five
method groups: `mod.rs` (struct + state types + the **bookkeeping** primitives everything
calls: `new`, `fresh_value`, `value_type`, `fresh_block`, `push_instr`, `push_alloc`,
loop-state save/restore, `seal_block`, `start_block`, `reopen_block`, `reseal_block_at`,
`begin_loop`, `finalize_loop`) plus sibling files `calls.rs`, `word_families.rs`,
`quotation.rs`, `control_flow.rs`. Rust permits multiple `impl FuncBuilder { … }` blocks for
the same type across files in the same crate, so each sibling file carries one `impl
FuncBuilder` block.

Rationale: the impl block is ~2500 lines, larger than the entire non-test body of `repl.rs`
or `parser.rs`. Moving it unchanged into one file reproduces the exact "too big to navigate"
complaint that motivated this refactor, so a single `func_builder.rs` is a half-measure the
brief explicitly flags. The five groups are read from signatures and call proximity, not a
full body read, so **confirm each boundary holds at extraction time**: if a group needs a
private helper physically in another group's range, either (a) leave that helper in `mod.rs`
(visible to all sibling files as an ancestor-private item), or (b) co-locate the two groups,
rather than adding cross-file `pub(super)` leakage on a one-off helper. Do not pre-merge on
suspicion; let `cargo build` decide.

### 2. Shared helper placement (a correction to the brief's `driver` cluster)

The brief slots `word_ret_ty`, `bundle_of`, `is_aggregate`, `free_locals_into`, and
`body_tail_calls_self` into `driver`, but grepping their call sites at `aa7c28e` shows
`func_builder` also calls them:

- `word_ret_ty` (driver L1288, L2412; func_builder L4845)
- `bundle_of` (driver L2411; func_builder L4844)
- `free_locals_into` (driver L2670–2673; func_builder L4743)
- `body_tail_calls_self` (driver L2702; func_builder L3815)
- `is_aggregate` (func_builder L3130 only)

Homing them in `driver` would force a `func_builder → driver` import, i.e. the circular-
dependency signal CLAUDE.md warns against, since `driver` already depends on `func_builder`.
Resolution: **home all five in `func_builder/mod.rs`** and let `driver` import them via
`super::func_builder::…`. This keeps the dependency arrow one-way (`driver → func_builder`),
matching the established direction. `is_aggregate` is func_builder-only, so it becomes
module-private there. `concrete_effect` and `subst_polytype` are driver-only (verified: no
func_builder caller) and stay in `driver`. Confirm these call-site sets at extraction time
before committing the placement.

### 3. `driver`'s split around `destructors` — keep `lower()` intact

`lower()`'s body calls `synthesize_aggregate_destructors` partway through, which is why the
`driver` cluster is two line ranges in the current file. Do **not** split `lower()`: it
rejoins as one function in `driver.rs`, calling `destructors::synthesize_aggregate_destructors`
across the module boundary. This follows the `check.rs` precedent (its `pub fn check()` driver
stayed intact as the crate's top-level entry point). Only split `lower()` if reading the code
surfaces a concrete reason (e.g. a genuine two-responsibility seam); there is no such reason on
the current read, so the default is intact.

### 4. Test relocation granularity — mechanical pass, hand-fix, tie-break to subject

For each `#[test] fn` in the flat `mod tests`, grep its body for the public function names of
each destination module and move it into the module it exercises. All relocation happens in
the final phase, after every source module exists, as one pass. Tests keep `use super::*`
against their new parent; a test needing cross-module items imports them explicitly.

Tie-break for tests that span modules: attribute to the **more specific subject** — the
behaviour asserted, not the entry point called. Many lowering tests drive through `lower` /
`lower_line` (driver entry points) but assert `FuncBuilder` block/back-edge/loop-header shape;
those belong in the relevant `func_builder` module, not `driver`. Layout tests
(`struct_layout_*`, `enum_layout_*`, `array_layout_*`, `carried_slot_bytes_*`) go to `layout`;
destructor-path tests (`recursive_disposal_path_*`, `synthesized_*_destructor_*`) go to
`destructors`. A test that legitimately spans (e.g.
`struct_linearity_agrees_across_the_checker_and_both_lowering_folds`) goes to the module whose
invariant it primarily guards; note the call in the phase commit. Within `func_builder`, place
each test beside the method group it asserts on (`calls` / `word_families` / `quotation` /
`control_flow`), falling back to `func_builder/mod.rs` when it spans groups.

### 5. `func_builder` visibility tightening — `pub(super)`, no shim re-export

Nothing outside `crate::ir` names `FuncBuilder` or any of its methods (grep confirms zero
external callers). The brief's phrasing "fully module-private (no `pub` needed)" is imprecise:
once `func_builder` is its own submodule, its parent (`driver`, and the driver code that lives
in `ir.rs` until Phase 9) must still reach `FuncBuilder`, its constructor, and the methods /
shared helpers `driver` calls, which requires `pub(super)` (= `pub(in crate::ir)`), not
private. The accurate invariant is: **no `pub(crate)`, and no `pub(crate) use` re-export in the
shim** for anything in `func_builder`. Give moved `FuncBuilder` items `pub(super)` uniformly
(covers the parent driver and all sibling group files); leave items with no cross-file caller
private and let clippy flag over-broad visibility. Bookkeeping primitives in `mod.rs` that only
sibling group files call can stay private (child modules can see ancestor-private items) —
promote to `pub(super)` only when a build error demands it. This is the one place the refactor
could in principle change what compiles from outside; Phase 4 must confirm via `cargo build`
that nothing external needed the wider visibility, and say so in the commit rather than
assuming.

### 6. Phase granularity — strict one move per phase, 10 phases

One extraction (or one `func_builder` sub-split) per phase, each a single reviewable/revertible
code-motion with a green full checkpoint and empty surface diff before the next. This matches
the brief's settled "one cluster per phase" and the `check.rs` spec's precedent. The count is
10 (`types`, `layout`, `destructors`, `func_builder` whole, its four sub-splits, `driver`,
tests), inside the brief's own arithmetic (up to 4 stage clusters, up to 5 for `func_builder`,
1 for tests). Extracting `func_builder` whole first (Phase 4), then carving method groups out
of it (Phases 5–8), isolates the one risky step — going module-private and confirming zero
external callers — into a single phase, leaving the sub-splits as pure intra-module motion.

## Per-phase checkpoint (every phase)

A phase is not done until, in order:

```sh
cargo build
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

all pass, **and** the surface diff is empty (the grep+perl pipeline above diffed against
`/tmp/ir-surface-before.txt`). Then one commit for that phase. Do not begin the next phase on a
red tree. If `cargo build` fails with a cyclic-dependency or unresolved-import error, that
boundary was wrong: pull the offending item toward its actual dependency root (or co-locate the
two modules) within the same phase rather than papering over it with a wider `pub`.

## Extraction mechanics (per module)

1. Create `src/ir/<module>.rs` (or `src/ir/func_builder/<group>.rs`).
2. Cut the cluster's items (types, fns, impls, private helpers) out of `ir.rs` into it,
   verbatim. Do not edit bodies.
3. Add `mod <module>;` to `ir.rs` (for `func_builder` sub-splits, add `mod <group>;` to
   `func_builder/mod.rs`).
4. Add `use` lines at the top of the new file: `use super::*;` for sibling/root items and
   crate-level types (`ast`, `check` imports, spans), plus the explicit `std`/external imports
   the moved code used.
5. Set visibility: items used only within the `ir` tree become `pub(super)`; items on the
   external surface get a `pub(crate) use self::<module>::Name;` re-export line in `ir.rs`.
   Nothing in `func_builder` gets a re-export (Q5).
6. Run the full checkpoint. Fix visibility until green; never touch call sites outside the
   `ir` tree.

`super::*` glob imports are acceptable (one module tree being reshaped, not a new public API);
prefer them over hand-maintaining long import lists and let `cargo fmt` / clippy flag genuinely
unused ones.

## Delivery plan

Order follows the pipeline, leaves first: the two dependency-free stages (`types`, `layout`),
then `destructors` (needs `layout`), then `func_builder` (needs `types` + `layout` +
`destructors`; nothing downstream but `driver` depends on it) and its sub-splits, then `driver`
(touches all four others), then test relocation and a final full-crate check.

- **Phase 1 — `types`.** Extract `types.rs`: the IR data model and the five symbol-name
  consts. External names (all of them): `IrModule`, `IrFunc`, `IrType`, `Value`, `BlockId`,
  `Block`, `Instr`, `BinOp`, `CmpOp`, `Terminator`, `Arity`, `QuotSigId`, `QuotSigLayout`,
  `WORD_WIDTH`, `OOB_TRAP_SYMBOL`, `ALLOC_SYMBOL`, `FREE_SYMBOL`, `TRACE_ALLOC_ENV`,
  `ir_type_of`, `quotation_layout` — re-export each. Root module, no downstream risk.
- **Phase 2 — `layout`.** Extract `layout.rs`. External: `StructLayout`, `EnumLayout`,
  `ArrayLayout`, `VariantLayout`, `FieldLayout`, `DropOverride`, `DropOverrides`, `Structs`,
  `Enums`, `Arrays`, `Cells`, `Refs`, `Registries`, `build_registries`, `carried_slot_bytes` —
  re-export. Depends only on `types`.
- **Phase 3 — `destructors`.** Extract `destructors.rs`. External:
  `synthesize_aggregate_destructors` — re-export. `expand_path` stays private (comment-only
  external mention; no re-export). Depends only on `layout`.
- **Phase 4 — `func_builder` whole (hard).** Move `FuncBuilder`, its state types, its entire
  impl, and the five shared helpers (Q2) into `src/ir/func_builder/mod.rs`. Set visibility to
  `pub(super)` for the parent driver's callers; confirm via `cargo build` that nothing external
  named any of it, so **no** `pub(crate)` and **no** shim re-export are required (Q5); state the
  confirmation in the commit. Depends on `types` + `layout` + `destructors`; `driver` (still in
  `ir.rs`) calls it via `func_builder::…`.
- **Phase 5 — `func_builder/calls`.** Carve the `calls` group (`lower_terms`, `lower_term`,
  `lower_self_tail_combinator`, `lower_call`) out of `mod.rs` into `calls.rs` as its own
  `impl FuncBuilder` block. Pure intra-module motion.
- **Phase 6 — `func_builder/word_families`.** Carve the reference/array/owned-cell/struct-field
  lowering primitives into `word_families.rs`. Watch for shared field/elem helpers used by both
  array and owned-cell lowering; keep them together (Q1).
- **Phase 7 — `func_builder/quotation`.** Carve the quotation/env/bundle/poly-call group into
  `quotation.rs`.
- **Phase 8 — `func_builder/control_flow`.** Carve `emit_select`, `total_order_key`, `lower_if`,
  `seal_arm`, `lower_clauses` into `control_flow.rs`.
- **Phase 9 — `driver`.** Extract `driver.rs`: `lower` (kept intact, Q3), `lower_line`,
  `collect_quot_sigs`, `lower_word`, `lower_instantiation`, `bind_env_capture`,
  `lower_word_parts`, `lower_materialized`, `concrete_effect`, `subst_polytype`, the `empty_*`
  stubs. Imports `destructors::` and `func_builder::` (including the shared helpers) via
  `super`. External: `lower`, `lower_line`, `lower_word`, `lower_instantiation`,
  `collect_quot_sigs` — re-export.
- **Phase 10 — test relocation + final full-crate green.** Move each `#[test] fn` from the flat
  `mod tests` into its subject module per Q4 (mechanical grep pass, hand-fix ambiguous,
  tie-break to the asserted subject; `func_builder` tests distribute to the method-group file
  they exercise). `ir.rs` retains only tests for items that stayed in `ir.rs` (there should be
  none beyond the re-export shim; if a helper stayed, its tests stay with it). Final gate: full
  checkpoint green **and** the surface diff byte-identical to `/tmp/ir-surface-before.txt` with
  zero external call-site changes.

## Out of scope

- Any behavioural change to lowering, layout computation, or destructor synthesis. Pure code
  motion plus the minimum visibility changes to compile, and the deliberate `func_builder`
  visibility tightening (Q5).
- Splitting any other file (`check.rs`, `repl.rs`). `check.rs`'s split is separate, in-flight
  work; do not fold the two together.
- Promoting or restructuring the `crate::ir` surface: the four-file consumer list stays at the
  same paths, `pub(crate)` where it is `pub(crate)` today.
- Adding new tests: this is code motion; existing tests in their new homes are the proof.

## Exit criteria

- `src/ir.rs` is a thin `mod` + `pub(crate) use` shim; the ~5.4k non-test lines live in
  `src/ir/{types,layout,destructors,driver}.rs` and
  `src/ir/func_builder/{mod,calls,word_families,quotation,control_flow}.rs`.
- Every relocated test lives in the module of the function it tests.
- `cargo build && cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- The surface grep diffs empty against `/tmp/ir-surface-before.txt`: `crate::ir::*` exposes
  exactly the pre-refactor names at the same paths, with zero changes outside `src/ir.rs` and
  `src/ir/`. `func_builder` names remain unreachable from outside `crate::ir` (no re-export).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Extract src/ir/types.rs (IR data model: IrModule, IrFunc, IrType, Value, QuotId, BlockId, Block, Instr, BinOp, CmpOp, Terminator, Arity, Resolver, QuotSigId/QuotSigLayout/QuotLayout, ir_type_of, quotation_layout, and the five symbol-name consts WORD_WIDTH/OOB_TRAP_SYMBOL/ALLOC_SYMBOL/FREE_SYMBOL/TRACE_ALLOC_ENV); re-export every external name; full checkpoint green + surface diff empty; one commit" },
    { "phase": 2, "focus": "Extract src/ir/layout.rs (StructLayout, field_is_linear, DropOverride/DropOverrides, the three *_drop_symbol helpers, FieldLayout/StructWord/Structs, EnumLayout/VariantLayout/EnumWord/Enums, ArrayLayout/Arrays, round_up/scalar_size_align(_ww), carried_slot_bytes, Cells, Refs, Registries, build_registries(_ww), LayoutBuilder); depends only on types; re-export external names; full checkpoint green + surface diff empty; one commit" },
    { "phase": 3, "focus": "Extract src/ir/destructors.rs (synthesize_aggregate_destructors, PathStep, recursive_disposal_path, find_path, expand_path, expand_fields, prepend, synthesize_struct_destructor(_override), synthesize_enum_destructor, synthesize_cell_destructor); depends only on layout; re-export synthesize_aggregate_destructors; keep expand_path private (comment-only external mention); full checkpoint green + surface diff empty; one commit" },
    { "phase": 4, "focus": "Move FuncBuilder + state types (CarriedSlot, LoopStateSnapshot, MaterializedQuot, EnvCapture, EnvPlan) + its whole impl + the five shared helpers (word_ret_ty, bundle_of, is_aggregate, free_locals_into, body_tail_calls_self) into src/ir/func_builder/mod.rs; set pub(super) for the parent driver's callers; confirm via cargo build that nothing external names FuncBuilder so no pub(crate) and no shim re-export are needed; state the confirmation in the commit; full checkpoint green + surface diff empty; one commit", "difficulty": "hard" },
    { "phase": 5, "focus": "Carve the calls method group (lower_terms, lower_term, lower_self_tail_combinator, lower_call) out of func_builder/mod.rs into src/ir/func_builder/calls.rs as its own impl FuncBuilder block; pure intra-module motion; full checkpoint green + surface diff empty; one commit" },
    { "phase": 6, "focus": "Carve the word_families method group (reference/array/owned-cell/struct-field lowering primitives push_reference..load_field_onto_stack) into src/ir/func_builder/word_families.rs; keep shared field/elem helpers together; full checkpoint green + surface diff empty; one commit" },
    { "phase": 7, "focus": "Carve the quotation method group (materialize_*, build_env, quotation_captures, lower_indirect_call, dispatch_on_tag, emit_drop, pack_bundle/unpack_bundle, lower_poly_call, lower_struct_word, lower_enum_word) into src/ir/func_builder/quotation.rs; full checkpoint green + surface diff empty; one commit" },
    { "phase": 8, "focus": "Carve the control_flow method group (emit_select, total_order_key, lower_if, seal_arm, lower_clauses) into src/ir/func_builder/control_flow.rs; full checkpoint green + surface diff empty; one commit" },
    { "phase": 9, "focus": "Extract src/ir/driver.rs (lower kept intact, lower_line, collect_quot_sigs, lower_word, lower_instantiation, bind_env_capture, lower_word_parts, lower_materialized, concrete_effect, subst_polytype, the empty_* stubs); import destructors:: and func_builder:: (incl shared helpers) via super; re-export lower, lower_line, lower_word, lower_instantiation, collect_quot_sigs; full checkpoint green + surface diff empty; one commit" },
    { "phase": 10, "focus": "Relocate each #[test] fn from the flat mod tests into its subject module (mechanical grep pass, hand-fix ambiguous, tie-break to the asserted subject; layout/destructors tests to their modules, func_builder tests distributed to the method-group file they exercise); final full-crate checkpoint green and surface grep byte-identical to /tmp/ir-surface-before.txt with zero external call-site changes; one commit" }
  ]
}
```
