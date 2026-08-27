# Phase 7 Slice 9: remove the REPL (brief)

Four probe subagents (2026-08-27) verified the roadmap entry's structural claims and
audited every file it names for REPL-conditional code. Structural claims all confirmed
exactly. The file-list premise -- that all 20 named files carry "REPL-specific
workarounds," each "a candidate for outright deletion" -- is confirmed for some files
and wrong for others; corrected per-file scope below.

## Structural claims: confirmed exactly

- `src/repl.rs` is 5299 lines, `src/editor.rs` 1071.
- `dlopen` has exactly one call site in the whole codebase: `repl.rs:59`, inside
  `Library::open`. All other `dlopen` hits are the FFI declaration, comments, or error
  message text.
- `driver::compile_so` (`driver.rs:947-973`) is a pure `ssa -> qbe -> cc -> .so`
  pipeline. It takes `&str`/`&Path`, touches no `Session` or REPL state, and is a
  clean relocation candidate as the roadmap entry states.
- `Library` (`repl.rs:46-85`, exactly 40 lines) holds a single `handle: *mut c_void`
  field and two methods (`open`, `symbol`). No `Session` state. Clean relocation
  candidate.
- `repl` subcommand wiring is exactly as described: `main.rs:69` `Some("repl") =>
  driver::repl()`, `driver.rs:939-943` `driver::repl` delegates to `repl::run`.
- `Session` (`repl.rs:1048-1200`) holds the stack buffer (`buf`, `top`, `types`), the
  word env (`env: HashMap<String, WordEntry>`), and the full set of per-line
  incremental registries (`structs`, `enums`, `arrays`, `owned_cells`, `refs`,
  `slices`, `drop_overloads`, `poly_words`, `combinators`, `override_epoch`, `libs`,
  `seq`, import tracking, `bool_enum`) -- matches the roadmap's description of what
  actually goes.

## File-list premise: confirmed for some files, wrong for others

**Confirmed -- real REPL-only code to delete, not just comments:**

- `ast.rs:1604` -- `enum Line { Def(WordDef), Expr(Vec<Term>) }`, the REPL input unit.
- `check.rs:90` -- `check_poly_combinator_repl`, the REPL's standalone poly-combinator
  entry point.
- `check.rs:122-123` -- `InferredLine` type alias (REPL line residual + field
  projections).
- `check.rs:1401-1407` -- the "a quotation cannot be left on the stack at the end of a
  line" error, which only fires in REPL-line context.
- `parser.rs:1203-1436` -- `parse_line_*` (word def, `type:` struct, `type:` enum)
  REPL line-parsing entry points, `reject_generic_typedef_in_repl`, and the paired
  `is_repl: true` field assignments.
- `parser.rs:1821-1830` -- `repl_unknown_capability_error`.
- `ir/driver.rs`'s `lower_line` (~`ir/driver.rs:465-522`) -- the whole function, a
  REPL-only lowering entry point, not the doc comments around it.
- `ir/layout.rs`'s `override_epoch`/drop-generation tracking and its `RTLD_GLOBAL`
  symbol-reload handling (`layout.rs:30-126`, `832-974`) -- real mutable state the
  REPL drives at runtime across lines, not documentation to trim.

**Must narrow, not delete outright:**

- `lib.rs:11` `pub mod repl;` -- goes with the module, fine as a plain deletion.
- `ast.rs:2105` `generation: Option<u64>` -- a field shared by both paths; only REPL
  ever populates `Some`. Removing REPL likely lets this collapse, but it threads
  through poly dispatch and needs its own check, not a blind delete.
- `resolve.rs:736-740` `always_mangle` -- this is **build-path** logic: it forces
  per-module mangling even for a single-module build, closing the QBE symbol-hijack
  class recorded in `project_qbe_type_symbols_forgeable`. REPL merely passes `false`
  into it. Do not touch this function; only the REPL call site (and the `false` it
  passes) goes away.

**Wrong -- these files carry no REPL-only conditional logic; the roadmap's "workaround
deletion" framing does not apply:**

- All 11 `src/check/` submodule files it names (`audits.rs`, `builtins.rs`,
  `captures.rs`, `combinators.rs`, `declarations.rs`, `drop_graph.rs`, `engine.rs`,
  `operators.rs`, `poly.rs`, `terms.rs`, `word_families.rs`). Every REPL reference in
  them is a docstring explaining that a *shared* function is also called by the REPL,
  or a gate that checks `ctx.modules().is_none()` -- and that condition is true for
  the REPL *and* for single-module native builds alike. There is no `if is_repl`
  branch anywhere in this directory. The load-bearing mechanism is the `Ctx::Line` vs
  `Ctx::Word` variant in `check/engine.rs` (`Ctx<'a>`, ~`engine.rs:1102`) and its
  `modules: None` / `generics: None` fields -- removing REPL support here means
  removing the `Ctx::Line` variant and narrowing every match on it, a type-level
  change across `check/engine.rs` and its ~9 downstream callers, not a grep-and-delete
  pass over 11 files. `check_poly_combinator_repl` in `poly.rs` is the one real
  REPL-only function in this group (already listed above under `check.rs`, separate
  file from the 11).
- Most of `src/ir/` (`ir.rs`, `ir/destructors.rs`, most of `ir/driver.rs`,
  `ir/test_helpers.rs`, `ir/types.rs`): REPL references are consumer lists in doc
  comments (e.g. "handed to every lowering path with no spliced-combinator inner poly
  calls (the REPL, destructor synthesis, unit tests...)"). No conditional gates.
  `ir/test_helpers.rs` has zero REPL references (one false-positive grep hit on the
  word "call"). Safe to strip REPL naming from these docstrings once the REPL callers
  are gone; nothing here needs code deletion beyond that.

## Corrected scope for the spec

The exit criterion "every workaround named above is deleted, not merely unreached" is
right in spirit but the file list overstates the check/ir surface. Per-file work
breaks into three tiers:

1. **Delete outright** (real REPL-only code): `ast.rs:1604` `Line` enum; `check.rs`'s
   `check_poly_combinator_repl`, `InferredLine`, and the quotation-on-stack-line
   error; `parser.rs`'s `parse_line_*` family, `reject_generic_typedef_in_repl`,
   `repl_unknown_capability_error`, and `is_repl` field plumbing; `ir/driver.rs`'s
   `lower_line`; `ir/layout.rs`'s override-epoch/`RTLD_GLOBAL` tracking; `lib.rs`'s
   `pub mod repl;`; `src/repl.rs` and `src/editor.rs` in full (after relocating
   `Library` and confirming `compile_so`'s new home).
2. **Narrow, verify no dangling reference, do not restructure the shared mechanism**:
   `ast.rs`'s `generation` field; `resolve.rs`'s `always_mangle` call site (delete the
   REPL caller, keep the function and its build-path forcing intact);
   `check/engine.rs`'s `Ctx::Line` variant and every `match ctx { ... }` arm across
   `check/` that handles it -- this is the one place real restructuring work is
   required, not a deletion pass.
3. **Comment/docstring cleanup only**: the 11 `check/` submodule files (minus
   `poly.rs`'s one real function) and most of `ir/` -- strip REPL naming from
   consumer-list docstrings, no logic changes.

## Correction (post-review, verified against HEAD `5c5edc2`)

The two places above that speak of "the REPL call site" of `resolve::resolve_modules`
(`:57` in the file-list section, `:102` in tier 2) describe a call site that does not
exist. `resolve_modules` (`src/resolve.rs:741`) has exactly one production caller,
`src/driver.rs:834`, which threads `assemble_module`'s `always_mangle` through; the
`false`-passing calls in `src/resolve.rs` (`:1055`, `:1085`, `:1106`, `:1417`) are that
function's own single-module unit tests. What the REPL actually passes `false` to is
`driver::assemble_module(&closure, false)` at `src/repl.rs:2055`, which dies with
`repl.rs`. The conclusion stands unchanged — `resolve_modules`, its `always_mangle`
parameter and its single-module forcing are untouched — but there is no separate resolve
deletion to schedule, and after the slice every surviving non-test caller passes `true`.
The spec ([`slice9-spec.md`](./slice9-spec.md)) is written against this corrected
mechanism.

## Open question for the spec

`check/engine.rs`'s `Ctx::Line` removal is the one piece of real design work in this
slice (everything else is deletion or comment cleanup). The spec should scope exactly
which `Ctx` methods collapse to `Ctx::Word`-only behavior and which call sites across
`check/` need their `Ctx::Line` match arm removed -- worth its own inventory pass
during phase planning rather than assumed away as "strike every REPL-conditional
branch."

## Exit

Matches the roadmap entry's exit criterion: `sooth repl` does not exist as a
subcommand; no source file references the REPL or its incremental-compile machinery
(`Session`, REPL-only functions/types listed above, `Ctx::Line`); `Library` and
`compile_so` live in `driver.rs`; every workaround named above is confirmed deleted by
grepping the corpus for its own review-graph notes, not merely unreached;
`cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green with no
REPL-only test module skipped or stubbed out.
