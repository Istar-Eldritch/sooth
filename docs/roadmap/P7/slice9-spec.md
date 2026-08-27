# P7.S9 — remove the REPL (spec)

Input: [`slice9-brief.md`](./slice9-brief.md), authoritative over the roadmap entry
(`P7-language-prereqs.md`, "P7.S9 — Remove the REPL") wherever they conflict. The brief's
correction stands: the roadmap's "all 20 named files carry deletable REPL-specific
workarounds" is false for most of `src/check/` and `src/ir/`, and `Ctx::Line` is the one
real restructuring task. Nothing here re-derives the brief's discovery.

**Nature of the change: a deletion slice with one type-level restructuring and one large
test migration.** There is no new language behaviour, no new diagnostic, and nothing for a
golden to newly accept or reject. Consequently the usual "mutation-test the guard" duty
inverts: mutation testing applies to the *migrated* tests (does the relocated form still
fail when the rule it guards breaks?), and completeness of the deletions is proved by grep
over the corpus and its review-graph notes, not by a green build.

Per-phase reports (the classifications, rulings and carried-forward findings the phases
below demand) go in [`slice9-phase-notes.md`](./slice9-phase-notes.md).

Three facts the brief did not surface, and they dominate the schedule:

1. **The bare-line path is load-bearing test infrastructure, not just REPL production
   code**, and it is reached *mostly* through shared per-file helpers, not scattered call
   sites. Three independent copies of an `infer_src` helper (`check.rs:3869` with **7**
   consumers, `check/engine.rs:1444` with **3**, `check/word_families.rs:1739` with **1**),
   plus `infer_variant_line` (`check/word_families.rs:2111`, **2** consumers),
   `ir/test_helpers.rs:245`'s `line_terms` (11 consumers: `ir/driver.rs` ×9,
   `ir/func_builder/calls.rs` ×2) and `backend/qbe.rs:1477`'s `emit_line` (3 consumers).
   (Re-derived; an earlier draft's per-helper counts each included the helper's own
   definition line, which the grep pattern also matches.) **Two call sites bypass every
   named helper** and call `crate::parser::parse_line` + match `crate::ast::Line::Expr`
   inline: `check/engine.rs:1749` (inside
   `check_dup_of_drop_overload_type_names_the_cause`, its `Ctx::Line` half) and
   `check/engine.rs:2152` (`releasable_into_withholds_a_name_used_in_a_back_edge_body`).
   The second is the dangerous one: it tests a general `Liveness`/`releasable_into` rule,
   not anything REPL-specific, and it never touches `Ctx` at all, so a helper-level or
   `Ctx::Line`-construction-level inventory misses it entirely — and the phase that deletes
   `parse_line`/`ast::Line` then fails to compile `cargo test`. Verified exhaustive:
   `grep -rn 'parse_line\b' src/` returns no other site outside the helpers, `repl.rs`,
   `parser.rs`'s own tests and `backend/qbe.rs:1900`'s named inline harness.
   Migration is helper-level plus those two, and must happen *before* deletion, against the
   live mechanism.
2. **Two whole integration files are REPL suites, and one of them is `tests/phase1.rs`.**
   All 49 of `tests/phase1.rs`'s tests go through its own `run_session`/`run_session_traced`
   (`tests/phase1.rs:11`, `:30`), which spawns `sooth repl`. The file has no non-REPL test.
   Six of the 49 are Phase 1 dogfood *exit criteria*. `tests/phase4_repl_imports.rs`
   (23 tests) is Phase 4 slice 5b's entire exit-criterion set. Those criteria die with the
   REPL and must be recorded as retired in the phase files that state them, not in
   `ROADMAP.md` alone.
3. **`infer_line` is the *only* production construction of `Ctx::Line`** (`check.rs:1348`);
   the other 23 constructions are all in `#[cfg(test)]` modules. `clippy -D warnings`
   builds the lib without `cfg(test)`, so deleting `repl.rs` without deleting `infer_line`
   in the same phase is an immediate `dead_code` failure, and deleting `infer_line` without
   flipping `Ctx` in the same phase makes `Ctx::Line` a never-constructed variant — also an
   immediate failure. Those three edits are one compile unit. There is no
   "record the warning rather than silencing it" option under `-D warnings`.

   **But that forcing covers less than an earlier draft claimed, and the difference is a
   whole phase.** `dead_code` fires only on items that are not externally visible.
   Verified pubness at HEAD: `infer_line` is `pub(crate)`, `InferredLine` is a private
   `type` alias, `check_poly_combinator_repl` is `pub(crate)`, `Ctx` is `pub(super)`,
   `reject_generic_typedef_in_repl` and `repl_unknown_capability_error` are private — all of
   these really do warn the moment `repl.rs` dies. Whereas `parser::parse_line` and
   `parser::parse_line_with_structs` are `pub fn` in `pub mod parser`, `ast::Line` is a
   `pub enum` in `pub mod ast`, and `ir::lower_line` is a `pub fn` in `pub mod ir`
   re-exported by `pub use` at `ir.rs:49`. By this spec's own R1 reasoning (a `pub` item in
   a `pub` module is not `dead_code` — the same reasoning that keeps `Library::symbol`
   alive), **none of those four trigger anything when their last caller dies.** So the
   bare-line *entry-point* family is a separable phase, and the spec splits it out (phases
   7a/7b below). `is_repl` and `reject_generic_typedef_in_repl` go with the entry-point
   half, not the forced half: their only assignments/callers live inside the still-`pub`
   `parse_line_*` family, so they stay reachable across 7a.

## Ruling on the brief's open question

`Ctx` (`src/check/engine.rs:1106`) **becomes a struct**, not an enum with one variant.
`Ctx::Line`'s only distinguishing content is "no word to cite", and every method that
serves it returns a placeholder (`None`, `0`, `false`, a borrowed fallback string). Keeping
a single-variant enum would keep 73 `match ctx` arms alive for no reason; keeping the
variant keeps a second checking path alive, which is the exact thing this slice exists to
kill. Method-by-method fate, verified against `engine.rs:1188-1341` (method *names* are
the real ones — `mangled_name`, `declared_outputs`, `is_self_tail_call`, not the shorter
spellings an earlier draft used):

| `Ctx` method | Line arm today | After |
| --- | --- | --- |
| `structs` (1188), `enums` (1194) | shared arm (1190, 1196) | plain field read |
| `static_type` (1211) | `None` (1214) | **stays `Option`** — a name may genuinely not be a static |
| `module` (1220) | `0` (1223) | `u32`, infallible |
| `modules` (1229) | `None` (1232) | **stays `Option`** — a retained poly word still passes `None`; do not collapse |
| `rendered_word_or` (1242) | `Cow::Borrowed(fallback)` (1245) | always renders; `fallback` parameter dies, rippling to **41** call sites |
| `mangled_name` (1249) | `None` (1252) | `&str`, infallible — **4 call sites compare against `Some(..)`** and must drop the wrapper: `check.rs:3434`, `check/terms.rs:922`, `:1502` (an `unwrap_or("the branch")` whose fallback becomes dead), `check/poly.rs:2050` |
| `effect` (1259) | `None` (1262) | `&StackEffect`, infallible |
| `declared_outputs` (1270) | `None` (1273) | `&[TypedSlot]`, infallible |
| `is_self_tail_call` (1282) | `false` (1285) | `bool`, infallible |
| `with_module` (1301) | rebuilds `Line` (1324) | single rebuild path |
| `generics` (1335) | `None` (1338) | **stays `Option`** — `None` off the native `check::check` path |

**`modules: Option`, `generics: Option` and `static_type: Option` staying `Option` is
load-bearing.** The brief is right that `ctx.modules().is_none()` is true for the REPL *and*
for a single-module native build; collapsing either to a non-optional would silently arm
gates that deliberately never fire off the whole-program path.

### Why the `Ctx` work is one phase and not three

73 `Ctx::Line` match arms exist (re-derived: `grep -rn 'Ctx::Line[^=]*=>' src/`), broken
down as `check.rs` **23**, `check/operators.rs` **12**, `check/word_families.rs` **11**,
`check/poly.rs` **11**, `check/terms.rs` **4**, and — omitted from the previous draft's
table — `check/engine.rs`'s own method block **12**. `check/captures.rs` has **0** arms
(its 4 `Ctx::Line` mentions are 3 test constructions and 1 comment).

A review round proposed splitting this into (a) rewrite the formatter arms to their `Word`
behaviour with the enum intact, (b) the type flip, (c) the `rendered_word_or` fallback
removal. **(a) is unbuildable**: every `Line` arm's `Word` counterpart cites the word (its
rendered name, mangled name, effect or declared outputs), which a `Ctx::Line` does not
carry, so there is nothing to rewrite the `Line` arm *to* while the variant still exists.
The only way to make (a) compile is to give `Ctx::Line` a synthetic `WordDef` payload —
manufacturing work solely to enable a split, which this slice forbids elsewhere. **(c) is
forced into the same unit** by `-D warnings`: once the `Line` arm is gone, `fallback` is an
unused parameter. So the restructuring is one phase, and the honest mitigation for its size
is that it is entirely compiler-driven: after the type flip, `cargo build` enumerates every
remaining site.

---

## 1. Verified anchors (HEAD `5c5edc2`)

| What | Where (verified) |
| --- | --- |
| `repl` subcommand dispatch, and its `usage()` line | `src/main.rs:69`, `src/main.rs:11` |
| unknown-command path `sooth repl` must fall into | `src/main.rs:71-74` |
| `driver::repl` (delegate) | `src/driver.rs:939` |
| `driver::compile_so` — **already in `driver.rs`; nothing to move** | `src/driver.rs:947`, unit test `compile_so_produces_loadable_object` at `2415` |
| `RTLD_NOW`/`RTLD_GLOBAL` (both `cfg` arms) | `src/repl.rs:30-35` |
| `dlopen`/`dlsym`/`dlerror`/`fflush` extern block | `src/repl.rs:37-42` (`fflush` at `41`) |
| `Library` + `open`/`symbol` | `src/repl.rs:44-80` (doc `44-45`, struct `46-48`, impl `50-80`) |
| `last_dlerror` | `src/repl.rs:82-92` |
| `Session` struct / `impl` / `repl::run` | `src/repl.rs:1048-1200`, `1201`, `3725` |
| `driver::assemble_module(_, false)` — the REPL's only `always_mangle: false` production caller | `src/repl.rs:2055` (tests at `4600`, `4929`) |
| `pub mod repl;` / `pub mod editor;` | `src/lib.rs:11`, `src/lib.rs:5` |
| `ast::Line` enum | `src/ast.rs:1607` |
| `ast::Instantiation::generation: Option<u64>` | `src/ast.rs:2109` |
| `instantiation_symbol(word, subst, generation)` | `src/ast.rs:2176`; `4226` and `4216` both die with the parameter, only `4206` survives (see R7) |
| `check_poly_combinator_repl` | `src/check/poly.rs:470`; import at `src/check.rs:90` |
| `InferredLine` alias | `src/check.rs:125` |
| `infer_line` | `src/check.rs:1331`; sole production `Ctx::Line` construction at `1348` |
| line-residual quotation error (fires only under `infer_line`) | `src/check.rs:1399-1407` |
| `Ctx` enum / `word_ctx` / method block | `src/check/engine.rs:1106`, `1165`, `1188-1341` |
| `parse_line` (`pub fn`) | `src/parser.rs:1206`; `parse_line_with_structs` (`pub fn`) `1257`; family runs to `1436` |
| direct `parse_line` + `ast::Line::Expr` sites outside every named helper | `src/check/engine.rs:1749` (Line half of the test at `1726`), `src/check/engine.rs:2152` (`releasable_into_withholds_a_name_used_in_a_back_edge_body`, a general Liveness test) |
| `reject_generic_typedef_in_repl` (def / call) | `src/parser.rs:1370` / `1436` |
| `is_repl` field, 5 `false` and 3 `true` assignments, 2 readers | `src/parser.rs:2228`; `816`, `894`, `965`, `1145`, `1192`; `1294`, `1351`, `1434`; readers `3684`, `3699` |
| `repl_unknown_capability_error` | `src/parser.rs:1830` |
| `parse_line`-only parser unit tests | `src/parser.rs:6844` (`parse_line_src` helper), tests `6850`, `6862`, `6873`, `6881`, `6889`; `is_repl` doc + test at `9579`ff |
| `lower_line` (`pub fn`) | `src/ir/driver.rs:493`; re-export `src/ir.rs:49` (`pub use`) |
| stale doc comments naming `ctx.mangled_name()`'s `Option` return | `src/driver.rs:1103`, `src/check/poly.rs:2046` — **not** matched by E2's grep (neither `mangled_name` nor `ctx` is in that pattern) |
| stale `Ctx::Line` prose in production doc comments | `src/check/engine.rs:1129`, `:1300`; `src/check/word_families.rs:1174`, `:1274` (plus `engine.rs:1580`, a doc comment on the surviving `ctx_word_carries_owning_module` test) |
| `*_drop_symbol(id, epoch)` — the four generation-parameterised minters | `src/ir/layout.rs:130`, `139`, `148`, `158` |
| `drop_generation` fields | `src/ir/layout.rs:59` (`StructLayout`), `232` (`EnumLayout`), `264` (`ArrayLayout`), `410` (`Cells::drop_generations`) |
| override-epoch doc + `None` seeds | `src/ir/layout.rs:32-58`, `126`, `157`, `229-231`, `259-263`, `408-409`; seeds `690`, `720`, `833`, `867`, `909`, `974` |
| `drop_generation` argument at symbol-minting sites | `src/ir/destructors.rs:351`, `407`, `455`, `498`, `536`, `1330`; `src/ir/func_builder/quotation.rs:400`, `405`, `409`, `413`; `src/backend/qbe.rs:3090` |
| `sooth_line_{seq}` carve-out note in `qbe_name` | `src/backend/qbe.rs:303` |
| `repl_core_import` / `repl_core_lines` / `REPL_CORE_ECHO` | `tests/common/mod.rs:64`, `74`, **`85`** (not `83`, which is the doc comment above it) |
| whole-file REPL suites | `tests/repl_ux.rs` (16 tests), `tests/phase4_repl_imports.rs` (23), `tests/phase1.rs` (49 `#[test]`s + 2 helper fns, all via `run_session`) |
| `#[ignore]` REPL notes (P7.S3s findings) | **10** total, attribute lines only: `tests/phase1.rs` **3**, `tests/phase4_combinators.rs` **5**, `tests/phase3_strings.rs` 1, `tests/phase4_slice10c_tail_splice.rs` 1 |
| non-REPL `#[ignore]`s, must not be swept in | `tests/phase7_slice3b_follow.rs` **3** (`:84`, `:736`, `:768`) |
| retired exit criterion, Phase 4 slice 5b | `docs/roadmap/P4-polymorphism-quotations.md:295` ("5b — imports at the REPL") |
| retired exit criteria, Phase 1 — **two lines, not one** | `docs/roadmap/P1-repl-and-liveness.md:12-13` ("**Exit (met):** define/test words interactively…") **and `:14-15`** ("**Dogfood (met):** a tiny interactive calculator session (`tests/phase1.rs`, `calculator_session_dogfood`)"), linked done from `docs/roadmap/ROADMAP.md:53` |
| CLAUDE.md invariant asserting the REPL's `dlopen` path | `CLAUDE.md`, "Load-bearing invariants", last bullet |

**Counts to re-derive, never hardcode from this table.** The `#[ignore]` set, the
`Ctx::Line` construction set, the `rendered_word_or` call-site set and the REPL-driving
integration files must be re-grepped at implementation time.

**`#[ignore]` must be counted with `grep -rnE '^[[:space:]]*#\[ignore' tests/` — attribute
lines only.** An earlier draft used `grep -rn '#\[ignore'`, which returns 22 hits against
13 real attributes because several `#[ignore = "…"]` note bodies quote the literal text
`#[ignore]` while explaining the ignore. That inflated every count in this spec: 16 REPL
notes claimed vs **10** actual, and 6 non-REPL claimed vs **3** actual.

**A correction to the brief.** The brief (`slice9-brief.md:57`, `:102`) says a REPL call
site passes `false` into `resolve::resolve_modules`. It does not: `resolve_modules` has
exactly one production caller (`src/driver.rs:834`), which threads
`assemble_module`'s `always_mangle`, and the `false`-passing calls in `src/resolve.rs`
(`1055`, `1085`, `1106`, `1417`) are that function's own single-module unit tests. What the
REPL actually passes `false` to is `driver::assemble_module` (`src/repl.rs:2055`), which
dies with `repl.rs`. So there is **no separate resolve deletion**: `resolve_modules`, its
`always_mangle` parameter, and the single-module forcing that closes the QBE symbol-hijack
class are untouched, and after this slice every surviving non-test caller passes `true`.
`tests/symbol_hijack.rs` is the proof and must stay green.

---

## 2. Requirements

**R1 — relocate `Library`.** `Library`, `last_dlerror`, the `dlopen`/`dlsym`/`dlerror`
externs and the `RTLD_NOW`/`RTLD_GLOBAL` constants move verbatim into `src/driver.rs`,
beside `compile_so`. `fflush` does **not** move: it exists for the REPL's interactive
prompt flushing, not for library loading. `compile_so` does not move — it is already in
`driver.rs`; the roadmap's "moves into `driver.rs`" is satisfied by construction, and the
slice must not manufacture motion to satisfy prose. `Library` keeps a unit test in
`driver.rs` that survives `repl.rs`'s deletion: compile a trivial `.so` through
`compile_so`, `Library::open` it, `symbol()` a known export, and assert a bad symbol name
errors. `Library::symbol`'s only production caller dies with the REPL; it stays `pub`, as
the load-bearing primitive for the roadmap's library-output target, and a `pub` item in a
`pub` module is not `dead_code`. `Library`'s doc comment is rewritten in the same move to
stop describing a session (see R9/E2).

**R2 — migrate the bare-line unit-test harness (check side).** Every unit test built on
`parse_line` + `infer_line` + `Ctx::Line` is reclassified before anything is deleted, per
the retirement-migration rule: classify by *test subject*, not by grep count. The work is
mostly helper-level: three separate `infer_src` copies (`check.rs:3869`, 7 consumers;
`check/engine.rs:1444`, 3; `check/word_families.rs:1739`, 1) and `infer_variant_line`
(`check/word_families.rs:2111`, 2 consumers).

**Plus the two helper-bypassing sites** (fact 1 above), which no other part of the
inventory reaches:

- `check/engine.rs:1749`, the `Ctx::Line` half of
  `check_dup_of_drop_overload_type_names_the_cause` (`:1726`): the Word half already pins
  the same message on the surviving path, so the Line half, its inline `parse_line` and its
  `// The`Ctx::Line`arm:` comment at `:1745` go. Confirm the Word half's assertions are a
  superset before deleting; if the Line half pins anything the Word half does not, keep
  that assertion on the Word call (the `captures.rs:946` pattern below).
- `check/engine.rs:2152`, `releasable_into_withholds_a_name_used_in_a_back_edge_body`:
  **migrate, do not delete.** Its subject is the R1 `releasable_into`/`Liveness` rule for a
  back-edge body, which has nothing to do with the REPL — it uses `parse_line` only as a
  convenient way to get a `Vec<Term>` out of `"a drop True ~[ 1 . ] ~[ ] if"`, and never
  builds a `Ctx` at all. Re-express the term list through a module parse (the terms of a
  one-word body) and keep every assertion. This test is invisible to a `Ctx::Line`-keyed or
  helper-keyed inventory, and it is the one that breaks phase 7b's `cargo test` compile if
  it is missed.

- **Subject is a general checking rule** (most of the 23 test-only `Ctx::Line`
  constructions — `check/poly.rs` ×14, `check/captures.rs` ×3, `check/engine.rs` ×3,
  `check/word_families.rs` ×2, `check/operators.rs` ×1) and
  `check/engine.rs:2152`: migrate to a `Ctx::Word` built by
  `word_ctx` (`check/engine.rs:1165`) over a synthetic single-word `WordDef`, and to a
  module parse in place of `parse_line`.
- **Subject is a line-boundary fact that dies with the REPL**
  (`infer_line_rejects_a_quotation_left_on_the_residual` at `check.rs:4107`,
  `infer_line_net_effect_expected` `4567`, `infer_line_carries_entry_depth` `4571`,
  `infer_line_carries_slot_types_expected` `4577`,
  `line_underflow_against_carried_stack_is_error` `4594` (an `infer_src` consumer the
  previous draft did not name), `infer_line_unknown_word_is_error`
  `4601`, `infer_line_consumes_a_carried_linear_slot_ok` `4726`, and
  `check/engine.rs:1599`'s `ctx_line_is_module_zero`): delete, with the classification
  recorded per test in the phase report. These are not losses of coverage; they are
  coverage of a retired mechanism.
- Each migrated test must be mutation-proved individually: break the rule it guards and
  confirm the *migrated* form fails. A `Ctx::Line` fixture that becomes green-and-inert
  under `Ctx::Word` is the known failure mode here (a narrowed guard loses its witness; a
  widened harness helper makes a backstop unreachable). Use the narrowest parse path that
  compiles the fixture — do **not** reach for `parse_with_core` because it is convenient.
- Known blocker to route around, not to discover late: `check::check` does not run the
  trait/impl pre-passes, so a fixture carrying a `Bound::User` cannot simply be moved onto
  a whole-module harness. Where that bites, keep the test at the `check_terms`/`Ctx::Word`
  level rather than promoting it to a source-level harness.
- `check/captures.rs:946`'s `check_capture_admission_rejects_captured_inline_quotation`
  runs the same call twice, once under `Ctx::Line` and once under `Ctx::Word`, and the two
  halves assert **different** content: the Line half pins that the error contains
  ``"`~`"`` *and* `"captured"` (the located rejection, not the ordinary quotation
  deferral); the Word half pins that it names the enclosing word ``"`outer`"``. Collapsing
  the pair keeps **both** assertions on the surviving `Ctx::Word` call. Do not read
  "collapse the pair" as "drop one assertion" — an earlier draft of this spec said
  "collapse to a single assertion", which would have silently retired the ``"`~`"``/
  `"captured"` witness. What goes is the redundant *second call* and the comment
  explaining why a second `Ctx` flavour was needed.

**R3 — migrate the bare-line unit-test harness (ir/backend side).** The shared helper is
`ir/test_helpers.rs:245`'s `line_terms`, with **11** consumers: `ir/driver.rs`
(`1283`, `1316`, `1398`, `1438`, `1481`, `1530`, `1582`, `1618`, `1686` — 9 call sites
across 8 test fns) and, omitted from the previous draft, `ir/func_builder/calls.rs:872`
(`quotation_literal_emits_no_instr_and_records_body`) and `:946`
(`self_tail_combinator_saves_and_restores_loop_state`). Plus `backend/qbe.rs`'s `emit_line`
helper (`:1477`, 3 consumers at `2027`, `2035`, `2045`) and its second inline `lower_line`
harness in `emit_float_slot_round_trips_with_float_load_store` (`:1896-1907`).

Same classification: `lower_line_marshals_all_inputs_and_outputs` (`ir/driver.rs:1276`),
`lower_line_returns_advanced_top` (`1310`),
`lower_line_scalar_only_uses_eight_byte_cells_and_no_blit` (`1472`) and the carried-slot
tests (`lower_line_struct_slot_blits_in_and_out` `1387`,
`lower_line_carried_str_slot_keeps_its_own_ir_type` `1428`,
`lower_line_carried_narrow_slot_relabels_after_load` `1520`,
`lower_line_carried_float_slot_loads_as_float` `1614`,
`lower_line_enum_slot_blits_in_and_out` `1663`) are about the *session stack marshalling
protocol* and die with it. Anything testing a shared lowering fact (slot IR types, blit
shape for a struct/enum) is re-expressed over `ir::lower` on a one-word module.
`ir/test_helpers.rs` must end up with no `parse_line` dependency (import at `:8`), since it
is the shared helper the rest of `ir/`'s tests import — do that first, it gates the rest.

**`lower_call_uses_resolved_generation_symbol` (`ir/driver.rs:1568`) does not die here.**
It is the `generation` field's only unit-level witness, and `generation` does not die until
R7. Migrate it off `line_terms` onto `ir::lower` and keep it green until the R7 phase
deletes the field it guards; deleting it earlier would leave R7's stop condition resting on
the QBE baseline alone.

**R4 — migrate or retire the REPL integration suites.** Re-grep the set
(`grep -rln 'arg("repl")\|repl::run\|repl_core_lines\|repl_core_import' tests/`, 18 files
at spec time). Per file, per test, classified:

- `tests/repl_ux.rs` (16 tests, spawns `sooth repl`): deleted in full. Its subject is the
  interactive UX (prompt, banner, `:words`, editing), which has no non-REPL counterpart.
- `tests/phase4_repl_imports.rs` (23 tests): deleted in full. It is Phase 4 slice 5b's
  exit-criterion set ("`import:` at the REPL"). Before deleting, confirm each
  *module-system* fact it pins has a `sooth run`/`build` twin elsewhere in
  `tests/phase4_modules.rs`; migrate any that does not. Then mark the criterion retired at
  **`docs/roadmap/P4-polymorphism-quotations.md:295`**, where it is actually stated, with a
  one-line reason.
- `tests/phase1.rs` is a **whole-file REPL suite**: all 49 tests go through `run_session`
  (`:11`) / `run_session_traced` (`:30`), which spawn `sooth repl`. Six are Phase 1 dogfood
  exit criteria (`sign_definable_and_callable_in_repl:168`,
  `vectors_dogfood_runs_in_repl:354`, `shapes_dogfood_runs_full_program_in_repl:459`,
  `stack_dogfood_runs_in_repl:511`,
  `self_tail_recursive_word_completes_in_constant_stack_in_repl:603`,
  `vm_dogfood_runs_in_repl:635`). Each of those six is rewritten against `sooth run` over
  the same `examples/` source **unless an equivalent `run` golden already exists
  elsewhere in the corpus**, in which case it is deleted as a duplicate and the duplication
  is named. A Phase 1 exit criterion may not simply disappear.

  **Three of those six carry `#[ignore]`, and the ignore-sweep must not reach them.** All 3
  of this file's `#[ignore]` attributes sit on exit-criterion tests:
  `sign_definable_and_callable_in_repl` (`:159`),
  `self_tail_recursive_word_completes_in_constant_stack_in_repl` (`:600`) and
  `vm_dogfood_runs_in_repl` (`:632`). They are **migrated, not deleted**: the `run`-based
  rewrite drops the `#[ignore]` as well, because the gap the note cites (P7.S3s's
  `splice_import` losing non-inline poly words at the REPL) does not exist off the REPL
  path, so the migrated form must run and pass. If a migrated form still fails, that is a
  real finding about `run` and the phase reports it — it does not re-add `#[ignore]`.

  **`calculator_session_dogfood` (`tests/phase1.rs:202`) is Phase 1's separately stated
  *Dogfood* criterion** (`docs/roadmap/P1-repl-and-liveness.md:14-15`), distinct from the
  Exit criterion at `:12-13`, and it was unnamed in the previous draft — which would have
  dropped it into the anonymous "other tests, classify and likely retire" bucket, silently
  retiring a stated criterion. **Ruling: it is retired, not migrated.** Its subject is the
  interactive session itself — seven lines fed one at a time, asserting the per-line
  `defined …` / `stack: …` echo after each — and there is no `run` form of "a tiny
  interactive calculator session". The language facts underneath it (a `| n |`-bound local,
  `sq`/`neg` definition, `mul`/`sub`/`add`, `.`) are ordinary and must be confirmed covered
  by named `run` tests before deletion. `P1-repl-and-liveness.md:14-15` is then updated in
  the same phase as `:12-13`: **both** lines, not just Exit.

  The other 43 are classified
  individually as migrate-to-`run` or delete-as-retired-mechanism (the inter-line carry
  tests, `:quit` disposal tests and redefinition/generation tests are retired-mechanism by
  construction; the linear-discipline and enum/struct declaration tests mostly have `run`
  twins — check, do not assume). Phase 1's **two** stated criteria
  (`docs/roadmap/P1-repl-and-liveness.md:12-13` Exit and `:14-15` Dogfood) and its
  `ROADMAP.md:53` row are then updated: the interactive halves are retired, and the notes
  say which of Phase 1's facts survive as `run` goldens.
- The remaining 14 files each carry a small REPL surface: locally-defined `run_session`/
  `repl_session` spawn helpers in `phase3_locals.rs:13`, `phase3_resources.rs:12`,
  `phase3_strings.rs:17`, `phase4_generics.rs:15`, `phase4_slice10c_tail_splice.rs:102`,
  `phase7_slice1.rs:48`; in-process `sooth::repl::run` at `phase4_slice11_inline.rs:81` and
  `phase4_slice12_partd.rs:47`; and the named tests
  `phase3_refs.rs::times_def_hand_copy_is_pinned_to_the_library`,
  `phase3_strings.rs::usize_comparison_across_a_repl_line_matches_same_line_semantics`,
  `phase4_combinators.rs`'s 7 (`combinator_and_hand_threaded_loops_agree_across_stack_limits`,
  `repl_self_tail_combinator_definition_is_accepted`, `repl_while_define_runs_to_fixpoint`,
  `repl_two_output_combinator_define_and_call`, `repl_imported_while_runs_to_fixpoint`,
  `repl_imported_filter_runs`, `repl_combinators_dogfood_matches_native`),
  `phase4_slice10c_tail_splice.rs::repl_defined_spliced_self_tail_loops_in_constant_stack`,
  `phase7_slice3g.rs::self_call_concrete_operand_mismatch_is_located_type_error`,
  `phase7_slice3i.rs::not_on_a_three_variant_enum_named_bool_is_an_error`,
  `phase7_slice3t.rs::explicit_instantiation_is_rejected_at_the_repl`,
  `phase7_slice3v.rs::an_owning_cell_payload_of_a_plain_quotation_is_still_rejected`,
  `phase3_locals.rs::repl_line_binding_more_than_the_session_stack_holds_is_error`,
  `phase3_resources.rs::repl_dispose_of_session_defined_override_is_unaffected`,
  `phase7_slice1.rs::repl_session_projects_struct_fields`. Classify each as *migrate to
  `run`* or *delete as covered*, naming the covering test.
  `phase7_slice3t.rs::explicit_instantiation_is_rejected_at_the_repl` is a REPL-shaped
  restatement of a rule that must still hold under `build`; migrate it, do not drop it.
  Every spawn helper left callerless is deleted with its last caller.
- The `#[ignore]` REPL notes are **10**, not 16 (see the counting note in section 1). Of
  those, **3 are `tests/phase1.rs`'s exit-criterion tests and migrate** per the ruling
  above; the remaining **7** are deleted along with their tests
  (`tests/phase4_combinators.rs` 5 — `repl_while_define_runs_to_fixpoint:1593`,
  `repl_two_output_combinator_define_and_call:1611`,
  `repl_imported_while_runs_to_fixpoint:2050`, `repl_imported_filter_runs:2072`,
  `repl_combinators_dogfood_matches_native:2118`; `tests/phase3_strings.rs` 1 —
  `usize_comparison_across_a_repl_line_matches_same_line_semantics:199`;
  `tests/phase4_slice10c_tail_splice.rs` 1 —
  `repl_defined_spliced_self_tail_loops_in_constant_stack:231`). Their root notes document
  the P7.S3s REPL gaps (`splice_import` losing non-inline poly words); those gaps are
  closed by deletion of the path, and that is the honest reason to record.
- `tests/common/mod.rs`'s `repl_core_import` (`:64`), `repl_core_lines` (`:74`) and
  `REPL_CORE_ECHO` (`:85`) go once no caller remains.

**R5 — delete the REPL, in two phases split along what `-D warnings` actually forces.**

**R5a, the forced compile unit.** `src/repl.rs` and `src/editor.rs` in full; `pub mod repl;`
and `pub mod editor;` from `src/lib.rs`; `Some("repl") => driver::repl()` from
`src/main.rs:69` and the subcommand line from `usage()` (`src/main.rs:11`); `driver::repl`
from `src/driver.rs:939`. `sooth repl` must then report the ordinary
`unknown command: repl` path (`src/main.rs:71-74`). In the **same** phase, because each of
these is non-`pub` and so warns as `dead_code` the moment `repl.rs` dies:
`check::InferredLine` (private alias), `check::infer_line` (`pub(crate)`) including the
line-residual quotation error at `check.rs:1399-1407`, the `check_poly_combinator_repl`
import at `check.rs:90` and the `pub(crate)` function itself
(`check/poly.rs:470`) — and therefore R6's `Ctx` flip too (`Ctx` is `pub(super)`, and
`infer_line`'s deletion removes its last production construction).

**R5b, the bare-line entry-point family.** Not forced by `-D warnings` — see fact 3 — so it
is its own phase: `ast::Line` (`pub enum`); `parser::parse_line` and
`parse_line_with_structs` (both `pub fn`) and the rest of the family through `:1436`;
`reject_generic_typedef_in_repl`; the `is_repl` field with all 8 assignments and both
readers at `parser.rs:3684`/`3699` (the `Ord` wording fork collapses to its non-REPL arm);
`repl_unknown_capability_error`; `ir::lower_line` (`pub fn`) and its `ir.rs:49` `pub use`
re-export; and the parser unit tests that exist only to exercise the family. Before
finalising, re-grep each item for a production caller outside the family itself — verified
at HEAD: `parse_line`'s only non-test, non-`repl.rs` callers are none; `parse_line_with_
structs`'s are `repl.rs:1418`/`:1694` only; `lower_line`'s is `repl.rs:3414` only;
`reject_generic_typedef_in_repl` is called once, at `parser.rs:1436`, inside the family;
`repl_unknown_capability_error` is called only from the two `is_repl` readers.

**R6 — restructure `Ctx`.** Per the ruling above: enum to struct, 11 methods per the table,
73 `Ctx::Line` match arms collapsed to their `Word` body, `rendered_word_or`'s `fallback`
parameter removed across 41 call sites (`check/poly.rs` 39, `check/captures.rs` 1,
`check/word_families.rs` 1), `with_module`'s single rebuild path. The `Option`-shedding
methods have consumers that must shed the wrapper with them: `mangled_name`'s 4 sites
above, `effect`'s 1 and `declared_outputs`'s 1. No diagnostic *text* on
the `Word` path may change: the `Word` arm's string is the surviving string, verbatim. Any
diagnostic golden that changes is a defect in the collapse, not drift.

The flip also strips prose that names the type it deletes. Two doc comments compare against
`ctx.mangled_name()`'s soon-to-be-gone `Option` return — `src/driver.rs:1103` and
`src/check/poly.rs:2046` — and **E2's grep will not catch either**, since neither
`mangled_name` nor `ctx` appears in E2's word list; they are named here explicitly or they
survive. Same for the stale `Ctx::Line` prose at `check/engine.rs:1129`, `:1300`, `:1580`
and `check/word_families.rs:1174`, `:1274`. This is a **deliberate, labelled exception** to
"tier 3 is comment-only and belongs to the doc-sweep phase": a comment describing a type
signature that this phase changes cannot outlive the phase that changes it, so it moves
here rather than to R8. (`engine.rs:1745` is the exception's exception — it is inside the
test whose `Ctx::Line` half R2 deletes, so it goes there.)

R6 runs **after** R5's deletion phase only if `Ctx::Line` still has a production
construction at that point — it does not. `infer_line`'s deletion (R5a) removes the last
one, so **R6 is bundled into R5a's phase**. See "Why the `Ctx` work is one phase" above;
the merged phase is the smallest green cut available.

**R7 — narrow the incremental-compile state.** `ast::Instantiation::generation:
Option<u64>` (`ast.rs:2109`) and `instantiation_symbol`'s third parameter (`ast.rs:2176`);
the `(PolySig, Option<u64>)` pairing in `check.rs:134-150` and its readers in
`check/poly.rs:4614-4625`, `4937-4938`, `5198`, `5231`;
`StructLayout`/`EnumLayout`/`ArrayLayout::drop_generation` (`ir/layout.rs:59`, `232`,
`264`), `Cells::drop_generations` (`:410`), the `epoch` parameter of all four
`*_drop_symbol` minters (`ir/layout.rs:130`, `139`, `148`, `158`), the override-epoch
doc block (`ir/layout.rs:32-58`) and every `None` seed (`690`, `720`, `833`, `867`, `909`,
`974`); the `drop_generation` argument at every symbol-minting site in
`ir/destructors.rs` (`351`, `407`, `455`, `498`, `536`, `1330`),
`ir/func_builder/quotation.rs` (`400`, `405`, `409`, `413`) and `backend/qbe.rs:3090`.
Each of these is `None` on every surviving path, so each collapses to an unparameterised
symbol. Delete the field rather than leaving a permanently-`None` `Option`: a field that
can only be `None` is a dead branch that reads as live.

**Of `ast.rs`'s three `instantiation_symbol` tests, two die and one survives** — the
previous draft claimed two survivors, which is wrong. `:4226`
`instantiation_symbol_distinct_generations_are_distinct_symbols_expected` (asserts
`Some(0)`'s symbol differs from `Some(1)`'s) and `:4216`
`instantiation_symbol_some_appends_gen_component_expected` (asserts `Some(0)` appends
`__gen0`) both assert *only* the behaviour of the deleted parameter; with it gone there is
nothing left for `:4216` to say, so it is retired-mechanism, not a survivor "re-expressed
without it". Only `:4206`
`instantiation_symbol_none_reproduces_native_spelling_expected` survives — it already pins
the native spelling `sooth_mono_id__t0_i64`, and the only edit is dropping the now-absent
`None` argument from its call.
`ir/driver.rs:1568`'s `lower_call_uses_resolved_generation_symbol` dies **here**, not in R3.

Verify by regenerating `tests/qbe_baseline` and confirming symbol names change only by
losing a generation suffix that was never emitted off the REPL path — if any baseline symbol
actually changes shape, **stop**: the field was live somewhere and this requirement is
wrong. R7 is its own phase precisely so that a stop here does not take R8 or the exit sweep
with it.

**R8 — docstring, design-doc and invariant sweep.** The sweep list is not hardcoded: it is
**derived from E2's grep** (in E2's corrected form — see E2), which at spec time returns
275 hits across 28 files, or 315 in the previous draft's word-boundary form. Both counts
exclude `src/repl.rs` and `src/editor.rs` themselves, which the previous draft failed to
say: over all of `src/`, the word-boundary form returns **791** hits across 30 files. The
file set is the same 28 either way —
`ast.rs`, `backend/qbe.rs`, `check.rs`, `check/{audits,builtins,combinators,declarations,
drop_graph,engine,operators,poly,terms,word_families}.rs`, `driver.rs`, `ir.rs`,
`ir/{destructors,driver,layout,types}.rs`, `ir/func_builder/{calls,mod,quotation}.rs`,
`lexer.rs`, `lib.rs`, `main.rs`, `parser.rs`, `resolve.rs`, `test_support.rs`. (The
previous draft's hand-listed set omitted `ast.rs` at 15 hits, `driver.rs` at 6,
`check/audits.rs` at 9, and `ir/func_builder/calls.rs`.) Most hits die with earlier phases;
what is left for this requirement is comment/docstring text, including
`backend/qbe.rs:303`'s `sooth_line_{seq}` note. No logic changes in this requirement; if a
"strip a comment" edit changes behaviour, it belongs to R5a/R5b/R6/R7 and was mis-scoped.
(The `Ctx`-signature comments listed under R6, and the `ast::Line`/`lower_line` doc prose
that goes with R5b, are the labelled exceptions: a comment describing a signature cannot
outlive the phase that changes the signature.)

Then the stated-design sweep, which the previous draft scoped too narrowly:

- **`CLAUDE.md`** — the load-bearing invariant "no in-process JIT … the REPL loads freshly
  compiled words in-process via `dlopen`" states the surviving rule without the REPL.
- **`DESIGN.md`**, **`docs/roadmap/ROADMAP.md`** (including the `P1` row at `:53`) and
  **`docs/roadmap/P7-language-prereqs.md`**'s S9 entry — current design only, no history of
  the removal.
- **`docs/design/`**: `control-flow.md` (7 word-boundary REPL mentions), `modules.md` (6),
  `codegen.md` (4). `memory-model.md` has **0** and is not in scope — a review round
  claimed it carries REPL mentions; verified false (the hits were `replace`).
- **`docs/roadmap/P8-packages-modules.md`**: `:111` and `:176` describe a manifest-read
  path "the REPL reads for a session"; `:291-292` records a declined "REPL exemption in the
  compiler". The declined-exemption note is now moot and says so; the manifest-read prose
  loses its REPL clause.
- **`docs/roadmap/P12-self-hosting.md`** — its two lines need *different* treatment, and
  the previous draft lumped them:
  - `:14`, "No metacircular JIT: the self-hosted REPL/build path still runs on the
    backend." This **is** a clean mention strip: drop `REPL/` and the invariant survives
    verbatim as "the self-hosted build path still runs on the backend". No decision needed.
  - `:19`, "The FFI boundary this phase depends on is one-directional today (host calling
    Sooth, via `dlopen` in the REPL); a progressive port additionally needs Sooth calling
    host code…". This is a substantive claim about FFI directionality, and **R1's decision
    answers it**: the host→Sooth direction is `driver::Library`'s `dlopen`/`dlsym` over a
    `compile_so` output, which R1 keeps `pub` in `driver.rs` precisely as the surviving
    load-bearing primitive, callerless but retained. So the correct rewrite names that
    mechanism ("via `driver::Library`'s `dlopen` over a `compile_so` output") and the rest
    of the sentence — P12's actual dependency, the *reverse* direction pulled forward into
    Phase 8 — stands untouched. P12's bootstrap plan is therefore **not** left open by this
    slice, and the phase must say so rather than record a non-answer.
- Historical implementation specs (`docs/roadmap/P{0..8}/slice*-{brief,spec}.md`,
  `docs/repl-ux-spec.md`, `docs/{check,ir}-modularisation-*.md`) are **out of scope**: they
  record what was built at the time and are not statements of current design.

A slice that changes a stated invariant and leaves the statement standing is the known
failure mode here.

**One note for a future reader, so this is not re-litigated:** after this slice
`resolve_modules`'s `always_mangle` parameter is *unreachable-but-retained*. Verified at
HEAD, `resolve_modules` has exactly one production caller (`driver.rs:834`), which threads
`assemble_module`'s flag, and every surviving non-test `assemble_module` call passes `true`
(`driver.rs:885`) once `repl.rs:2055` dies. The `false` path stays exercised only by
`resolve.rs`'s own four unit tests (`:1055`, `:1085`, `:1106`, `:1417`). It is kept anyway:
the single-module forcing it selects is what closes the QBE symbol-hijack class, and
collapsing the parameter would make that forcing implicit. Do not "tidy" it in a later
slice on the grounds that no caller passes `false`.

**Out of scope, tracked not done.** The book rewrite (`docs/book/preface.md` 5 mentions,
`getting-started.md` 6, `words.md` 2, `the-stack.md` 1 — the last omitted from the roadmap
entry's list; plus `the-interactive-book.md` dropping from `SUMMARY.md:55`) is explicit
follow-up doc work in the roadmap entry, not an exit criterion. Record it as a follow-up;
do not start it.

---

## 3. Exit criteria

| # | Criterion | Witness |
| --- | --- | --- |
| E1 | `sooth repl` is not a subcommand | `sooth repl` prints `unknown command: repl`; `usage()` lists no repl |
| E2 | No source file references the REPL or its incremental-compile machinery | `grep -rniE 'repl\|session\|dlopen\|override_epoch\|drop_generation' src/ \| grep -viE 'replac\|replic'` returns only `src/driver.rs`'s relocated `dlopen`/`dlsym` extern declarations. See the note below on why this form and not `-w` |
| E3 | `Library` and `compile_so` both live in `driver.rs` and are covered | new `Library` round-trip unit test in `driver.rs`, `compile_so_produces_loadable_object` still green |
| E4 | `Ctx::Line` does not exist; `Ctx` is a struct | `grep -rn 'Ctx::Line' src/` empty; `modules`/`generics`/`static_type` still return `Option` |
| E5 | Every named workaround is deleted, not unreached | per-item grep over `src/`, `tests/`, `docs/roadmap/`, including each item's own review-graph note text |
| E6 | No REPL-only test module is skipped or stubbed | `grep -rnE '^[[:space:]]*#\[ignore' tests/` — attribute lines only — shows exactly `phase7_slice3b_follow.rs`'s **3** non-REPL notes (`:84`, `:736`, `:768`) and nothing else. The loose `grep -rn '#\[ignore'` form is not a witness: it also matches note bodies quoting the attribute, and returns 22 against 13 real attributes at HEAD |
| E7 | Every migrated test is proved live | per-test mutation result recorded; every deleted test classified as retired-mechanism or duplicate |
| E8 | Retired exit criteria are recorded where they are stated | `docs/roadmap/P4-polymorphism-quotations.md:295` marks the REPL-import criterion retired; **both** `docs/roadmap/P1-repl-and-liveness.md:12-13` (Exit) **and `:14-15`** (Dogfood, `calculator_session_dogfood`) are updated, plus `ROADMAP.md:53`, recording which Phase 1 facts survive as `sooth run` goldens and which are retired. A run updating only the Exit line fails this criterion |
| E9 | P12's `:19` FFI claim names the surviving host→Sooth mechanism | `docs/roadmap/P12-self-hosting.md:19` names `driver::Library`'s `dlopen` over a `compile_so` output (R1's retained primitive) as the one-directional boundary, and does **not** say "in the REPL" and does **not** record the question as open. `:14` is a plain mention strip (`REPL/` dropped, "no metacircular JIT" verbatim). The previous draft's "names a surviving mechanism *or* records that the question is open" was an unfalsifiable disjunction and is replaced |
| E10 | Green | `cargo fmt --check && cargo clippy -- -D warnings && cargo test` (assess with `--no-fail-fast`) |

**Why E2's grep is substring-with-exclusions, not `-w`.** `-w` was chosen to dodge
`replace`/`replica`, and it does — but it also goes **blind to compound identifiers**,
because `-w` requires the whole matched alternative to be word-bounded rather than allowing
a substring inside a larger identifier. Measured at HEAD: `grep -rnwiE 'repl' src/check/poly.rs`
matches `check_poly_combinator_repl` **zero** times, and the same holds for
`repl_unknown_capability_error` and `reject_generic_typedef_in_repl` — three of this
slice's own deletion targets, invisible to the criterion meant to prove they are gone.
(`is_repl` escaped only because it was spelled out as its own alternative.) The corrected
form, `grep -rniE '…' src/ | grep -viE 'replac|replic'`, was measured empirically against
HEAD: it returns 275 hits across the same 28 files (excluding `repl.rs`/`editor.rs`), and
enumerating every distinct matched identifier shows **no** false-positive class beyond
`replac*`/`replic*` — so it is neither noisy nor blind. If R7 aborts, the `drop_generation`
clause is recorded blocked with R7's stop reason rather than reported green.

## 4. Risks

- **Silent coverage loss.** The dominant risk, in both directions: a migrated test that no
  longer fails when its rule breaks, and a deleted test whose subject was *not* actually
  REPL-only. `tests/phase1.rs` is the sharp end — 49 tests, no non-REPL member, six of them
  exit criteria. R2/R3/R4's per-test classification plus per-test mutation proof is the
  whole mitigation; a phase that reports "green" without it has not done the work.
- **The merged R5a/R6 phase is large, and only one of its two candidate splits is real.**
  Splitting the `Ctx` work out is unbuildable (see "Why the `Ctx` work is one phase");
  splitting the bare-line *entry points* out is not, and R5b does exactly that, because
  `-D warnings` does not force `pub` items. What remains merged is compiler-driven (after
  the type flip, `cargo build` enumerates every remaining site) but is still one commit.
  Commit R5a/R6's phase before running its mutation passes.
- **R5b breaks `cargo test`'s *compile* if R2 or R3 left one bare-line test behind.** The
  sharp case is `check/engine.rs:2152`, a general Liveness test that touches no `Ctx` and
  so appears in no `Ctx::Line` inventory. R5b's precondition is a re-grep, and its
  instruction on a hit is to finish R2's migration, never to delete the test to make the
  build pass.
- **`Ctx` collapse changing a diagnostic.** R6 forbids it. Watch for a `Word` arm that was
  itself wrong and only looked right next to the `Line` arm.
- **`generation` being live somewhere.** R7 has an explicit stop condition rather than an
  assumption, and its own phase so the stop is contained.
- **Split signals.** `src/check.rs`, `src/parser.rs` and `src/check/engine.rs` all shrink
  materially here. Re-run CLAUDE.md's five refactor signals at slice exit against
  `check.rs`, `check/engine.rs`, `check/poly.rs` and `parser.rs` and record yes/no; note
  that `poly.rs`'s split is already deferred by a prior decision, so a signal firing there
  is not automatically a new action.
- **Commit before mutation testing, and never `cp -r` the worktree** for the mutation
  passes; a scratch copy shares the real gitdir.

---

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "R1: relocate `Library` into `src/driver.rs`. Move `Library` (src/repl.rs:44-80: doc 44-45, struct 46-48, impl 50-80), `last_dlerror` (src/repl.rs:82-92), the `dlopen`/`dlsym`/`dlerror` extern block (src/repl.rs:37-42, WITHOUT `fflush` at :41 -- that is the REPL's prompt flushing, it stays in repl.rs until repl.rs itself goes) and the `RTLD_NOW`/`RTLD_GLOBAL` constants (src/repl.rs:30-35, including both cfg arms) into src/driver.rs beside `compile_so` (src/driver.rs:947). Verbatim EXCEPT `Library`'s doc comment, which today says 'The session keeps every handle resident' -- rewrite it to describe the primitive without a session, since E2's grep is word-boundary `session` and must come back clean. Do NOT move `compile_so`: it is ALREADY in driver.rs, verified; the roadmap's 'compile_so moves into driver.rs' is satisfied by construction and this phase must not manufacture motion to satisfy prose -- say so in the phase report. Repoint src/repl.rs to `crate::driver::Library` and delete its now-duplicate items; repl.rs must still compile and `cargo test --test repl_ux` must still pass, proving the relocation is behaviour-neutral while the REPL is still alive to prove it against. Add a `Library` round-trip unit test in driver.rs's test module that survives repl.rs's deletion: build trivial QBE IL exporting one symbol, `compile_so` it into a TempDir, `Library::open`, `symbol(\"that_name\")` returns non-null, and `symbol(\"no_such_symbol\")` returns Err -- model the IL and temp-dir handling on the existing `compile_so_produces_loadable_object` (src/driver.rs:2415). Keep `Library`/`open`/`symbol` `pub`; a pub item in a pub module is not dead_code, so clippy stays quiet once the REPL caller dies. No other file changes. Green on `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.",
      "difficulty": "easy"
    },
    {
      "phase": 2,
      "focus": "R2: migrate the check-side bare-line unit-test harness, against the still-live mechanism. The work is MOSTLY helper-level: three independent `infer_src` copies (src/check.rs:3869 with SEVEN consumers at :4111, :4568, :4574, :4591, :4595, :4602, :4733; src/check/engine.rs:1444 with THREE at :1833, :1839, :1840; src/check/word_families.rs:1739 with ONE at :1971) and `infer_variant_line` (src/check/word_families.rs:2111, TWO consumers at :2143, :2151) are what wrap `parser::parse_line` + `check::infer_line`. An earlier draft said 8/4/2 and 3 -- those figures each counted the helper's own definition line, which `grep 'infer_src('` also matches. TWO SITES BYPASS EVERY HELPER and are invisible to both a helper-keyed and a `Ctx::Line`-keyed inventory; they are the reason phase 8 would otherwise fail to compile `cargo test`: (i) src/check/engine.rs:1749, the `Ctx::Line` half of `check_dup_of_drop_overload_type_names_the_cause` (fn at :1726), which calls `crate::parser::parse_line` inline at :1750 and `infer_line` at :1753 -- DELETE that half plus its inline parse and its `// The `Ctx::Line` arm:` comment at :1745, but FIRST confirm the surviving Ctx::Word half's assertions are a superset (it pins \"cannot `dup`\" and \"`File` is linear because it defines `drop`\" and !\"no bits to copy\"; the Line half pins only the middle one) -- if the Line half pins anything the Word half does not, keep that assertion on the Word call; (ii) src/check/engine.rs:2152, `releasable_into_withholds_a_name_used_in_a_back_edge_body` (fn at :2150) -- MIGRATE, DO NOT DELETE. Its subject is the R1 `releasable_into`/`Liveness` rule for a back-edge body, entirely non-REPL; it uses `parse_line` only to turn \"a drop True ~[ 1 . ] ~[ ] if\" into a `Vec<Term>` and never builds a `Ctx` at all. Re-express the term list via a module parse (the terms of a one-word body) and keep EVERY assertion including the `unused` control. Verified exhaustive: `grep -rn 'parse_line\\b' src/` shows no third bypassing site (remaining hits are the named helpers, src/repl.rs, src/parser.rs's own tests, and src/backend/qbe.rs:1900's named inline harness, which phase 3 owns). Re-derive (do not trust these counts) every unit test built on those helpers and on `Ctx::Line`. `Ctx::Line` has 24 constructions, of which exactly ONE is production (src/check.rs:1348, inside `infer_line`) and 23 are test-only: src/check/poly.rs x14 (:9994, :10041, :10227, :10279, :10489, :10734, :12131, :12364, :12436, :12546, :12663, :12706, :12793, :13251), src/check/captures.rs x3 (:785, :904, :962), src/check/engine.rs x3 (:1535, :1602, :2070), src/check/word_families.rs x2 (:2250, :2570), src/check/operators.rs x1 (:575). Classify EVERY test, in the phase report, as (a) subject is a general checking rule -> migrate to a `Ctx::Word` built via `word_ctx` (src/check/engine.rs:1165) over a synthetic single-word WordDef, and to a module parse in place of `parse_line`; or (b) subject is a line-boundary fact that dies with the REPL -> delete. Category (b) is expected to include infer_line_rejects_a_quotation_left_on_the_residual (src/check.rs:4107), infer_line_net_effect_expected (:4567), infer_line_carries_entry_depth (:4571), infer_line_carries_slot_types_expected (:4577), line_underflow_against_carried_stack_is_error (:4594 -- an infer_src consumer an earlier draft did not name; its subject is underflow AGAINST THE CARRIED STACK, so check whether the same underflow rule has a non-line witness before retiring it), infer_line_unknown_word_is_error (:4601, but only if a Ctx::Word twin already exists -- check), infer_line_consumes_a_carried_linear_slot_ok (:4726), and ctx_line_is_module_zero (src/check/engine.rs:1599); verify each classification against what the test ASSERTS, not its name. SPECIAL CASE, and read it carefully: src/check/captures.rs:946 `check_capture_admission_rejects_captured_inline_quotation` calls `check_capture_admission` TWICE -- once with the Ctx::Line at :962 asserting the error contains \"`~`\" AND \"captured\", once with a Ctx::Word (via `word_ctx`) asserting it contains \"`outer`\". These are DIFFERENT content assertions, not a value compared against itself. Collapse the pair by deleting the redundant Ctx::Line call and its explanatory comment (:982-985) and keeping BOTH assertions against the surviving Ctx::Word call. An earlier draft of this spec said 'collapse to a single assertion' -- that instruction was wrong and would have retired the \"`~`\"/\"captured\" witness. Mutation-prove every migrated test individually: break the rule it guards and confirm the MIGRATED form fails; a fixture that goes green-and-inert under Ctx::Word is the known failure mode (a narrowed guard losing its witness). Use the narrowest parse path that compiles each fixture -- do NOT reach for `parse_with_core`, whose pre-passes have previously made a backstop unreachable. Known blocker to route around, not to discover late: `check::check` does not run the trait/impl pre-passes, so a fixture carrying a `Bound::User` cannot be promoted to a whole-module source harness; keep those at the check_terms/Ctx::Word level. Delete NOTHING in src/ production code this phase (Ctx::Line, infer_line and parse_line all remain), so the phase is provable on a live REPL. Commit before running the mutation passes; never `cp -r` the worktree for a mutation copy. Green.",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "R3: migrate the ir/backend-side bare-line unit-test harness, still against the live mechanism. The shared helper is src/ir/test_helpers.rs:245 `line_terms` (its `parse_line` import at :8), with ELEVEN consumers -- src/ir/driver.rs x9 (:1283, :1316, :1398, :1438, :1481, :1530, :1582, :1618, :1686, across 8 test fns) and, MISSING from an earlier draft of this spec, src/ir/func_builder/calls.rs:872 (`quotation_literal_emits_no_instr_and_records_body`, fn at :848) and src/ir/func_builder/calls.rs:946 (`self_tail_combinator_saves_and_restores_loop_state`, fn at :904). Also src/backend/qbe.rs's `emit_line` helper (:1477, consumers :2027, :2035, :2045) and its second inline lower_line harness inside `emit_float_slot_round_trips_with_float_load_store` (:1896-1907, with the `Line::Expr` unwrap at :1900). src/ir/test_helpers.rs must end the phase with ZERO `parse_line` dependency, since every other ir/ test imports it -- do that first, it gates the rest. Classify each test in the report: the session stack-marshalling tests DIE with the protocol -- lower_line_marshals_all_inputs_and_outputs (:1276), lower_line_returns_advanced_top (:1310), lower_line_struct_slot_blits_in_and_out (:1387), lower_line_carried_str_slot_keeps_its_own_ir_type (:1428), lower_line_scalar_only_uses_eight_byte_cells_and_no_blit (:1472), lower_line_carried_narrow_slot_relabels_after_load (:1520), lower_line_carried_float_slot_loads_as_float (:1614), lower_line_enum_slot_blits_in_and_out (:1663). Anything asserting a SHARED lowering fact (slot IR types, struct/enum blit shape, the two func_builder/calls.rs tests) is re-expressed over `ir::lower` on a one-word module and mutation-proved. DO NOT DELETE src/ir/driver.rs:1568 `lower_call_uses_resolved_generation_symbol` in this phase: it is the `generation` field's only unit-level witness and `generation` does not die until phase 9. Migrate it off `line_terms` onto `ir::lower` and keep it GREEN; phase 9 deletes it along with the field it guards. Deleting it here would leave phase 9's stop condition resting on the QBE baseline alone. For the backend/qbe.rs sites, re-express the IL assertions over `lower` and confirm the asserted IL text still names the same construct; if the one-word-module form emits different-but-equivalent IL, update the expectation and say which line changed and why. Delete no production code this phase. Mutation-prove each migrated test (break the lowering rule, confirm the migrated form fails). Green.",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "R4 part a: retire the two whole-file non-Phase-1 REPL suites and the shared harness. Re-grep the REPL-driving set first (`grep -rln 'arg(\\\"repl\\\")\\|repl::run\\|repl_core_lines\\|repl_core_import' tests/`, measured at spec time as 18 files); do not reuse a stale list. (a) Delete tests/repl_ux.rs (16 tests) in full: its subject is interactive UX -- prompt, banner, `:words`, line editing -- with no non-REPL counterpart. (b) tests/phase4_repl_imports.rs (23 tests) is Phase 4 slice 5b's ENTIRE exit-criterion set. Before deleting it, check each module-system fact it pins for a `run`/`build` twin in tests/phase4_modules.rs; migrate any fact with no twin, then delete the file. (c) Mark that criterion retired where it is actually STATED: docs/roadmap/P4-polymorphism-quotations.md:295, the '5b -- imports at the REPL' paragraph -- NOT in ROADMAP.md, which only carries the phase row. One line, giving the reason (the REPL import path is gone), current-state wording with no narration of the removal. (d) Delete tests/common/mod.rs's repl_core_import (:64), repl_core_lines (:74) and REPL_CORE_ECHO (:85 -- NOT :83, which is that constant's doc comment) ONLY IF callerless after (a)+(b) -- tests/phase1.rs and tests/phase3_strings.rs and tests/phase4_slice10c_tail_splice.rs still call repl_core_import at this point, so expect this to defer to phase 6 and say so rather than forcing it. Every migrated integration test must be mutation-proved: break the behaviour it asserts in src/ and confirm the `run`-based form fails. The REPL still exists at the end of this phase. Assess with `cargo test --no-fail-fast`.",
      "difficulty": "medium"
    },
    {
      "phase": 5,
      "focus": "R4 part b, highest scrutiny: tests/phase1.rs. This file is a WHOLE-FILE REPL suite -- all 49 tests run through its own `run_session` (:11) / `run_session_traced` (:30), which spawn `sooth repl`; there is no non-REPL test in it. SIX are Phase 1 dogfood EXIT CRITERIA and none of them may simply vanish: sign_definable_and_callable_in_repl (:168), vectors_dogfood_runs_in_repl (:354), shapes_dogfood_runs_full_program_in_repl (:459), stack_dogfood_runs_in_repl (:511), self_tail_recursive_word_completes_in_constant_stack_in_repl (:603), vm_dogfood_runs_in_repl (:635). Rewrite each of the six against `sooth run` over the SAME examples/ source, UNLESS an equivalent `run` golden already exists somewhere in tests/ -- in which case delete it AND name the covering test in the report. THE IGNORE SWEEP MUST NOT REACH THREE OF THOSE SIX: all 3 of this file's `#[ignore]` ATTRIBUTES (re-count with `grep -nE '^[[:space:]]*#\\[ignore' tests/phase1.rs` -- it is 3, not the 5 an earlier draft claimed, which counted note bodies quoting the attribute) sit on exit-criterion tests: sign_definable_and_callable_in_repl (attribute :159), self_tail_recursive_word_completes_in_constant_stack_in_repl (:600), vm_dogfood_runs_in_repl (:632). Those three are MIGRATED and LOSE their `#[ignore]` in the rewrite -- the gap their notes cite (P7.S3s: the REPL's `splice_import` binds no non-inline poly word) does not exist off the REPL path, so the `run`-based form must actually run and pass. If a migrated form still fails, that is a real finding about `run` and this phase REPORTS it; it does not re-add `#[ignore]` and it does not delete the criterion. SEVENTH NAMED CRITERION, unnamed in an earlier draft and therefore at risk of vanishing into the anonymous bucket: `calculator_session_dogfood` (tests/phase1.rs:202) is Phase 1's separately stated DOGFOOD criterion (docs/roadmap/P1-repl-and-liveness.md:14-15), distinct from the Exit criterion at :12-13. RULING: it is RETIRED, not migrated -- its subject is the interactive session itself (seven lines fed one at a time, asserting the per-line `defined ...`/`stack: ...` echo after each), and there is no `run` form of \"a tiny interactive calculator session\". Before deleting it, confirm the ordinary language facts underneath it (a `| n |`-bound local, `sq`/`neg` definition, mul/sub/add, `.`) are each covered by a NAMED `run` test, and name them. Then classify the other 42 individually as migrate-to-run or delete-as-retired-mechanism: the inter-line carry tests (subword_carried_value_survives_line_boundary :232, carried_float :251, carried_struct :264/:278/:301, enum_large_payload :558, array_and_usize_cross_repl_line_boundary_and_render :497), the `:quit` residual-disposal tests (:682, :716, :827, :879, :908, :925) and the redefinition/generation tests (:92, :135) are retired-mechanism BY CONSTRUCTION -- they test the session boundary itself. The linear-discipline tests (:700, :733, :779, :803, :856), the declaration-error tests (:324, :571), the polymorphic-definition tests (:963, :973, :1000, :1016, :1071, :1095) and the enum tests (:402, :433) mostly have `run`/`build` twins elsewhere: CHECK per test and name the twin, do not assume. Do NOT run a blanket `#[ignore]` deletion in this file -- its only 3 attributes are the migrating exit-criterion tests handled above. Then update BOTH stated criteria, not just Exit: docs/roadmap/P1-repl-and-liveness.md:12-13 ('**Exit (met):** define/test words interactively; redefinition works; the first throwaway-but-real interactive session exists.') records that the interactive half is retired and names which Phase 1 facts survive as `sooth run` goldens, AND :14-15 ('**Dogfood (met):** a tiny interactive calculator session (`tests/phase1.rs`, `calculator_session_dogfood`)') records that dogfood as retired with the REPL and stops naming a deleted test. A run that edits only the Exit line fails E8. docs/roadmap/ROADMAP.md:53's P1 row is brought in line. Current-state wording only; no history of the removal. Every migrated test mutation-proved. The REPL still exists at the end of this phase; `--test phase1` no longer spawns it. Assess with `cargo test --no-fail-fast`.",
      "difficulty": "hard"
    },
    {
      "phase": 6,
      "focus": "R4 part c: the remaining 14 single-surface test files, then the `#[ignore]` sweep. Locally-defined spawn helpers to delete with their last caller: tests/phase3_locals.rs:13 `run_session`, tests/phase3_resources.rs:12, tests/phase3_strings.rs:17, tests/phase4_generics.rs:15 `repl_session`, tests/phase4_slice10c_tail_splice.rs:102, tests/phase7_slice1.rs:48. In-process `sooth::repl::run` sites: tests/phase4_slice11_inline.rs:81, tests/phase4_slice12_partd.rs:47. Named REPL-driving tests to classify as migrate-to-run or delete-as-covered, naming the covering test for every deletion: phase3_locals.rs::repl_line_binding_more_than_the_session_stack_holds_is_error (:282), phase3_resources.rs::repl_dispose_of_session_defined_override_is_unaffected (:59), phase3_refs.rs::times_def_hand_copy_is_pinned_to_the_library, phase3_strings.rs::usize_comparison_across_a_repl_line_matches_same_line_semantics, phase4_combinators.rs's seven (combinator_and_hand_threaded_loops_agree_across_stack_limits, repl_self_tail_combinator_definition_is_accepted, repl_while_define_runs_to_fixpoint, repl_two_output_combinator_define_and_call, repl_imported_while_runs_to_fixpoint, repl_imported_filter_runs, repl_combinators_dogfood_matches_native), phase4_slice10c_tail_splice.rs::repl_defined_spliced_self_tail_loops_in_constant_stack, phase7_slice1.rs::repl_session_projects_struct_fields (:143), phase7_slice3g.rs::self_call_concrete_operand_mismatch_is_located_type_error, phase7_slice3i.rs::not_on_a_three_variant_enum_named_bool_is_an_error, phase7_slice3t.rs::explicit_instantiation_is_rejected_at_the_repl (:265), phase7_slice3v.rs::an_owning_cell_payload_of_a_plain_quotation_is_still_rejected. phase7_slice3t.rs:265 restates a rule that must still hold under `build` -- MIGRATE it, do not drop it. Then the `#[ignore]` sweep: count with ATTRIBUTE LINES ONLY (a leading-whitespace-anchored match on the ignore attribute), then delete the remaining SEVEN REPL notes with their tests -- NOT the 11 an earlier draft claimed, which came from a loose grep that also matches note bodies quoting the attribute while explaining the ignore (22 loose hits vs 13 real attributes at HEAD). The seven, verified: tests/phase4_combinators.rs x5 -- repl_while_define_runs_to_fixpoint (attribute :1584, fn :1593), repl_two_output_combinator_define_and_call (:1608/:1611), repl_imported_while_runs_to_fixpoint (:2047/:2050), repl_imported_filter_runs (:2069/:2072), repl_combinators_dogfood_matches_native (:2115/:2118); tests/phase3_strings.rs x1 -- usize_comparison_across_a_repl_line_matches_same_line_semantics (:190/:199); tests/phase4_slice10c_tail_splice.rs x1 -- repl_defined_spliced_self_tail_loops_in_constant_stack (:222/:231). All seven already appear in this phase's named-test list above, so the sweep adds no test beyond it -- that coincidence is the check that the count is right. tests/phase1.rs's 3 attributes are NOT in this sweep: phase 5 migrates them with their exit-criterion tests. Record the honest reason -- their root notes document the P7.S3s REPL gaps (splice_import losing non-inline poly words, materialized-quotation link failure), which are closed by deleting the path. Do NOT touch tests/phase7_slice3b_follow.rs's THREE (not 6) unrelated ignore attributes at :84, :736, :768, which pin a `times-helper` cross-call gap, not anything REPL. Finally delete tests/common/mod.rs's repl_core_import (:64), repl_core_lines (:74) and REPL_CORE_ECHO (:85 -- NOT :83, which is that constant's doc comment), which should now be callerless -- verify by grep, do not assume. Exit witness for this phase: `grep -rn 'arg(\\\"repl\\\")\\|repl::run\\|repl_core' tests/` empty, and `grep -rn '#\\[ignore' tests/` (attribute lines only) shows only phase7_slice3b_follow.rs's 3. Every migrated test mutation-proved. Assess with `cargo test --no-fail-fast`.",
      "difficulty": "medium"
    },
    {
      "phase": 7,
      "focus": "R5a + R6 in ONE phase: the compile unit `clippy -D warnings` genuinely forces. Read the forcing carefully, because it covers LESS than an earlier draft of this spec claimed and the difference is phase 8. `dead_code` fires only on items that are not externally visible. Verified pubness at HEAD: `infer_line` (src/check.rs:1331) is `pub(crate)`, `InferredLine` (src/check.rs:125) is a private `type` alias, `check_poly_combinator_repl` (src/check/poly.rs:470) is `pub(crate)`, and `Ctx` (src/check/engine.rs:1106) is `pub(super)` -- so all four warn the moment repl.rs dies. `infer_line` has no non-test caller outside repl.rs, and src/check.rs:1348 inside `infer_line` is the ONLY production construction of `Ctx::Line` (the other 23 are #[cfg(test)] and phase 2 dealt with them), so deleting infer_line alone makes `Ctx::Line` a never-constructed variant -- also a hard failure. THOSE edits are one compile unit. (a) Delete src/repl.rs and src/editor.rs in full; `pub mod repl;` (src/lib.rs:11) and `pub mod editor;` (src/lib.rs:5); `Some(\"repl\") => driver::repl()` (src/main.rs:69) and the repl line in `usage()` (src/main.rs:11); `driver::repl` (src/driver.rs:939). Confirm `sooth repl` now takes the ordinary unknown-command path (src/main.rs:71-74). (b) Delete `InferredLine` (src/check.rs:125), `infer_line` (src/check.rs:1331) including the line-residual quotation error (:1399-1407), the `check_poly_combinator_repl` import (src/check.rs:90) and the function itself (src/check/poly.rs:470). (c) R6, same phase: convert `enum Ctx<'a>` (src/check/engine.rs:1106) to a struct carrying the former Ctx::Word fields and delete the Line variant. Method fates (src/check/engine.rs:1188-1341), verified against source including the real method NAMES: `structs` (:1188) and `enums` (:1194) become plain field reads; `module` (:1220) returns u32 infallibly; `mangled_name` (:1249) returns &str -- and it has FOUR external consumers that compare against `Some(..)` and must shed the wrapper (src/check.rs:3434, src/check/terms.rs:922, src/check/terms.rs:1502 where `unwrap_or(\"the branch\")`'s fallback becomes dead, src/check/poly.rs:2050); an earlier draft of this spec claimed zero external callers, which is false; `effect` (:1259) returns &StackEffect (1 consumer, src/check.rs:3435); `declared_outputs` (:1270) returns &[TypedSlot] (1 consumer, src/check/terms.rs:1461); `is_self_tail_call` (:1282) returns bool (2 real consumers, src/check/terms.rs:922 and src/check/poly.rs:2077, shape unchanged); `with_module` (:1301) keeps one rebuild path; `rendered_word_or` (:1242) always renders, so its `fallback` parameter DIES, rippling to 41 call sites (src/check/poly.rs 39, src/check/captures.rs 1, src/check/word_families.rs 1 -- re-derive with `grep -rn 'rendered_word_or(' src/`, and note the fn definition itself does not match that pattern because of its `<'f>`). THREE methods must STAY `Option`, load-bearing and not tidiness: `modules` (:1229, a retained poly word passes None off the whole-program path), `generics` (:1335, None off `check::check`), `static_type` (:1211, a name may genuinely not be a static). Collapsing `modules` or `generics` would silently ARM gates that deliberately never fire on a single-module or retained-poly path. Collapse all 73 `Ctx::Line` match arms to their `Word` body: src/check.rs 23, src/check/operators.rs 12 (:381-:528), src/check/word_families.rs 11 (:687-:1557), src/check/poly.rs 11 (:4659-:8421), src/check/terms.rs 4 (:1196-:1839), and src/check/engine.rs's own method block 12 -- that last group was OMITTED from an earlier draft's table. src/check/captures.rs has ZERO arms. NO diagnostic text on the Word path may change: the Word arm's string survives verbatim; if any diagnostic golden changes, that is a defect in the collapse, not golden drift -- stop and fix the collapse. (d) DOC PROSE, a deliberate and LABELLED exception to 'tier 3 is comment-only, handled in the doc-sweep phase': a comment describing a type signature this phase changes cannot outlive the phase that changes it. Strip the stale `Ctx::Line` prose in production doc comments at src/check/engine.rs:1129 and :1300 and src/check/word_families.rs:1174 and :1274, plus the doc comment at src/check/engine.rs:1580 on the surviving `ctx_word_carries_owning_module` test. AND the two stale doc comments that describe `ctx.mangled_name()`'s old `Option` return -- src/driver.rs:1103 and src/check/poly.rs:2046 -- which E2's grep will NOT catch, because neither `mangled_name` nor `ctx` is in E2's word list; they are named here explicitly or they survive the slice. (src/check/engine.rs:1745 is NOT here: it is inside the test whose `Ctx::Line` half phase 2 deletes.) NOTE on resolve: there is NO REPL call site of `resolve::resolve_modules` to delete -- the brief's claim (slice9-brief.md:57, :102) is wrong. The REPL's `always_mangle: false` caller is `driver::assemble_module(&closure, false)` at src/repl.rs:2055, which dies with (a). `resolve_modules` (src/resolve.rs:741), its `always_mangle` parameter and its single-module forcing stay UNTOUCHED -- that forcing closes the QBE symbol-hijack class, and `cargo test --test symbol_hijack` must stay green as the proof. After this phase `always_mangle` is unreachable-but-retained: every surviving non-test caller passes `true` (src/driver.rs:885) and the `false` path lives only in src/resolve.rs's own four unit tests (:1055, :1085, :1106, :1417). Say so in the phase report so a later slice does not 'tidy' it. Verify: `grep -rn 'Ctx::Line' src/ tests/` empty; `grep -rn 'repl::\\|editor::\\|Session' src/ tests/` empty of production references; the three surviving Options confirmed still Option; full suite green with ZERO golden diffs. `parse_line`, `ast::Line` and `ir::lower_line` all still EXIST at the end of this phase and that is correct -- they are `pub` in `pub` modules, so nothing warns; phase 8 removes them. Prove each deletion is a deletion and not a de-reachability: for every item, grep src/, tests/ and docs/roadmap/ for its own name AND its review-graph note text, and report the remaining hits. Commit before running any mutation pass. Green.",
      "difficulty": "hard"
    },
    {
      "phase": 8,
      "focus": "R5b: the bare-line ENTRY-POINT family. Its own phase, and the justification is the inverse of phase 7a's: NONE of these items triggers `dead_code` when its last caller dies, so `-D warnings` does not force them into 7a. Verified pubness at HEAD, by this spec's own R1 reasoning (a `pub` item in a `pub` module is not dead_code -- the same reasoning that keeps `Library::symbol` alive): `parser::parse_line` (src/parser.rs:1206) and `parser::parse_line_with_structs` (:1257) are `pub fn` in `pub mod parser` (src/lib.rs:10); `ast::Line` (src/ast.rs:1607) is a `pub enum` in `pub mod ast` (src/lib.rs:1); `ir::lower_line` (src/ir/driver.rs:493) is a `pub fn` re-exported by `pub use self::driver::{lower, lower_line};` at src/ir.rs:49 from `pub mod ir` (src/lib.rs:6). `reject_generic_typedef_in_repl` (:1370), `repl_unknown_capability_error` (:1830) and the `is_repl` field (:2228) are private, but they stayed reachable across 7a because their only callers/assigners live INSIDE the still-`pub` parse_line family. RE-GREP each item for a production caller before deleting -- do not take this list on trust. Verified at HEAD: `parse_line`'s only non-test callers were in src/repl.rs (now gone); `parse_line_with_structs`'s were src/repl.rs:1418 and :1694 only; `lower_line`'s was src/repl.rs:3414 only; `reject_generic_typedef_in_repl` has exactly one call site, src/parser.rs:1436, inside the family; `repl_unknown_capability_error` is called only from the two `is_repl` readers. Delete: `ast::Line` (src/ast.rs:1607); `parse_line` (:1206), `parse_line_with_structs` (:1257) and the rest of the family through :1436; `reject_generic_typedef_in_repl` (def :1370, call :1436); `repl_unknown_capability_error` (:1830); the `is_repl` field (:2228) with all 8 assignments (:816, :894, :965, :1145, :1192 false; :1294, :1351, :1434 true) and BOTH readers (:3684, :3699) -- the `Ord` capability wording forks collapse to their non-REPL arm, and the surviving message text must be byte-identical to today's non-REPL arm, so pin it with the existing golden rather than retyping it; `ir::lower_line` (src/ir/driver.rs:493) and its re-export (src/ir.rs:49). Delete the parse_line-only parser unit tests (helper `parse_line_src` :6844; tests :6850, :6862, :6873, :6881, :6889; the `parse_line_with_structs` is_repl tests at :9590, :9618, :9644, :9681 and their doc prose) after confirming, per test, that no non-line parsing rule loses its only witness -- if one does, migrate it to a module parse instead. PRECONDITION on phases 2 and 3: `cargo test` must still COMPILE after this phase, which it only does if every `parse_line` / `ast::Line::Expr` reference in a #[cfg(test)] module is already gone. Re-run `grep -rn 'parse_line\\|ast::Line\\|Line::Expr\\|line_terms\\|lower_line' src/` BEFORE deleting and confirm the only hits are the items this phase removes. The two helper-bypassing sites phase 2 owns (src/check/engine.rs:1749 and :2152, the latter a general Liveness test that must have been MIGRATED, not deleted) are the ones most likely to have been missed; if either is still present, STOP and finish phase 2's migration rather than deleting the test to make the build pass. Verify: `grep -rn 'parse_line\\|InferredLine\\|lower_line\\|is_repl' src/` empty; `grep -rn 'ast::Line\\|Line::Def\\|Line::Expr' src/` empty. Prove each deletion is a deletion and not a de-reachability: grep src/, tests/ and docs/roadmap/ for each item's own name AND its review-graph note text, and report remaining hits. Green.",
      "difficulty": "medium"
    },
    {
      "phase": 9,
      "focus": "R7: narrow the incremental-compile state. Its own phase, separate from the doc sweep, so its stop condition can fire without taking anything else down. Delete `ast::Instantiation::generation` (src/ast.rs:2109) and `instantiation_symbol`'s third parameter (src/ast.rs:2176), with its test instantiation_symbol_distinct_generations_are_distinct_symbols_expected (src/ast.rs:4226). TWO of the three sibling tests die and only ONE survives -- an earlier draft claimed two survivors, which is wrong. :4216 instantiation_symbol_some_appends_gen_component_expected asserts ONLY that Some(0) appends `__gen0`; with the parameter gone it has nothing left to assert, so it is retired-mechanism, NOT a survivor 're-expressed without it'. Only :4206 instantiation_symbol_none_reproduces_native_spelling_expected survives -- it already pins the native spelling `sooth_mono_id__t0_i64`, and its only edit is dropping the now-absent `None` argument from the call. Delete the `(PolySig, Option<u64>)` generation pairing in src/check.rs:134-150 and its readers (src/check/poly.rs:4614-4625, :4937-4938, :5198, :5231). Delete `StructLayout::drop_generation` (src/ir/layout.rs:59), `EnumLayout::drop_generation` (:232), `ArrayLayout::drop_generation` (:264), `Cells::drop_generations` (:410), the `epoch` PARAMETER of all four symbol minters -- struct_drop_symbol (:130), enum_drop_symbol (:139), cell_drop_symbol (:148), array_drop_symbol (:158), which an earlier draft did not name -- the override-epoch doc block (:32-58) and its mirrors (:126, :157, :229-231, :259-263, :408-409), and every `None` seed (:690, :720, :833, :867, :909, :974). Delete the drop_generation argument at every minting site: src/ir/destructors.rs:351, :407, :455, :498, :536, :1330; src/ir/func_builder/quotation.rs:400, :405, :409, :413; src/backend/qbe.rs:3090. Delete src/ir/driver.rs:1568 lower_call_uses_resolved_generation_symbol HERE -- phase 3 kept it alive on purpose as this field's only unit-level witness. Delete the fields rather than leaving permanently-`None` Options: a field that can only be None reads as live and is a dead branch. STOP CONDITION, do not work around it: regenerate tests/qbe_baseline with REGEN_QBE_BASELINE=1 and confirm symbol names change ONLY by losing a generation suffix that was never emitted off the REPL path. If any baseline symbol changes shape, the field was live somewhere, R7 is wrong, and the phase REPORTS that instead of forcing the deletion -- phases 10 and 11 do not depend on this phase landing. Green.",
      "difficulty": "hard"
    },
    {
      "phase": 10,
      "focus": "R8: docstring, design-doc and invariant sweep. (a) Comment/docstring text. Do NOT work from a hardcoded file list -- DERIVE the list from E2's grep IN ITS CORRECTED FORM (see E2 and phase 11): `grep -rniE 'repl|session|dlopen|override_epoch|drop_generation' src/ | grep -viE 'replac|replic'`. At spec time that returned 275 hits across 28 files (the previous draft's word-boundary form returned 315 across the same 28 -- both figures EXCLUDE src/repl.rs and src/editor.rs, which the previous draft failed to say; over all of src/ the word-boundary form returns 791 across 30 files): src/ast.rs, backend/qbe.rs, check.rs, check/{audits,builtins,combinators,declarations,drop_graph,engine,operators,poly,terms,word_families}.rs, driver.rs, ir.rs, ir/{destructors,driver,layout,types}.rs, ir/func_builder/{calls,mod,quotation}.rs, lexer.rs, lib.rs, main.rs, parser.rs, resolve.rs, test_support.rs. An earlier draft's hand-listed set omitted ast.rs (15 hits), driver.rs (6), check/audits.rs (9) and ir/func_builder/calls.rs -- which is exactly why the list is derived, not written down. Most hits died in phases 7-9; what remains here is prose, including src/backend/qbe.rs:303's `sooth_line_{seq}` note. Comment-only: if a comment edit changes behaviour it was mis-scoped and belongs to phase 7, 8 or 9. (b) Stated design. Rewrite CLAUDE.md's load-bearing-invariants bullet that asserts 'no in-process JIT ... the REPL loads freshly compiled words in-process via dlopen' to state the surviving rule without the REPL. Update DESIGN.md, docs/roadmap/ROADMAP.md and docs/roadmap/P7-language-prereqs.md's S9 entry to the post-removal state. Broaden to docs/design/: control-flow.md (7 word-boundary REPL mentions), modules.md (6), codegen.md (4). docs/design/memory-model.md has ZERO word-boundary REPL mentions and is NOT in scope -- a review round claimed otherwise; verified false, the hits were `replace`. docs/roadmap/P8-packages-modules.md: :111 and :176 describe a manifest-read path 'the REPL reads for a session' (lose the REPL clause), and :291-292 records a declined 'REPL exemption in the compiler' (now moot -- say so). docs/roadmap/P12-self-hosting.md needs its TWO lines treated DIFFERENTLY, which an earlier draft lumped together. :14 -- 'No metacircular JIT: the self-hosted REPL/build path still runs on the backend.' -- IS a clean mention strip: drop 'REPL/' and the invariant survives verbatim as 'the self-hosted build path still runs on the backend'. No decision needed. :19 -- 'The FFI boundary this phase depends on is one-directional today (host calling Sooth, via `dlopen` in the REPL); a progressive port additionally needs Sooth calling host code...' -- is a substantive claim about FFI directionality, and R1's decision ANSWERS it: the host->Sooth direction is `driver::Library`'s `dlopen`/`dlsym` over a `compile_so` output, which R1 keeps `pub` in driver.rs as the surviving load-bearing primitive, callerless but retained. So rewrite :19 to NAME that mechanism ('via `driver::Library`'s `dlopen` over a `compile_so` output'), leaving the rest of the sentence -- P12's actual dependency, the REVERSE direction pulled forward into Phase 8 -- untouched. P12's bootstrap plan is therefore NOT left open by this slice and the phase must say so rather than record a non-answer. Historical implementation specs (docs/roadmap/P{0..8}/slice*-{brief,spec}.md, docs/repl-ux-spec.md, docs/{check,ir}-modularisation-*.md) are OUT OF SCOPE: they record what was built at the time, not current design. Current design only throughout -- NO history of the removal in any of these files. (c) Record the book rewrite as explicit follow-up, out of scope: docs/book/preface.md (5 mentions), getting-started.md (6), words.md (2), the-stack.md (1 -- omitted from the roadmap entry's list), plus the-interactive-book.md dropping from docs/book/SUMMARY.md:55. Green.",
      "difficulty": "medium"
    },
    {
      "phase": 11,
      "focus": "Exit sweep, its own phase so a phase-9 stop does not take it down. Run every E1-E10 witness from section 3 and report each result verbatim. E2's grep MUST be the substring-with-exclusions form, NOT the word-boundary form an earlier draft mandated: `grep -rniE 'repl|session|dlopen|override_epoch|drop_generation' src/ | grep -viE 'replac|replic'` -- expected to return only src/driver.rs's relocated `dlopen`/`dlsym` extern declarations. `-w` was chosen to dodge `replace`/`replica`, and it does, but it also goes BLIND to compound identifiers, because `-w` requires the whole matched alternative to be word-bounded rather than allowing a substring inside a larger identifier. Measured at HEAD: `grep -rnwiE 'repl' src/check/poly.rs` matches `check_poly_combinator_repl` ZERO times, and the same holds for `repl_unknown_capability_error` and `reject_generic_typedef_in_repl` -- three of this slice's own deletion targets, invisible to the criterion meant to prove they are gone (`is_repl` escaped only because it was spelled out as its own alternative). The corrected form was measured empirically against HEAD content this slice does not touch: 275 hits across the same 28 files, and enumerating every distinct matched identifier shows NO false-positive class beyond `replac*`/`replic*` -- so it is neither noisy nor blind. Report the exact command and its output. If phase 9 aborted on its stop condition, record E2's `drop_generation` clause as BLOCKED with phase 9's stop reason rather than reporting the criterion green. E6: attribute-lines-only count `grep -rnE '^[[:space:]]*#\\[ignore' tests/` shows exactly tests/phase7_slice3b_follow.rs's THREE non-REPL notes (:84, :736, :768) and nothing else. The loose `grep -rn '#\\[ignore'` form is NOT a witness: it also matches note bodies quoting the attribute, and returns 22 against 13 real attributes at HEAD. E4: `grep -rn 'Ctx::Line' src/` empty and modules/generics/static_type still return Option. E8: confirm the retirement notes actually landed in docs/roadmap/P4-polymorphism-quotations.md:295 and BOTH docs/roadmap/P1-repl-and-liveness.md:12-13 (Exit) AND :14-15 (Dogfood, `calculator_session_dogfood`), plus ROADMAP.md's P1 row -- not merely in ROADMAP.md, and not the Exit line alone. E9: confirm docs/roadmap/P12-self-hosting.md:19 now NAMES `driver::Library`'s `dlopen` over a `compile_so` output (R1's retained primitive) as the one-directional host->Sooth boundary, does NOT say 'in the REPL', and does NOT record the question as open; and that :14 is a plain mention strip ('REPL/' dropped, 'No metacircular JIT' verbatim). An earlier draft's 'names a surviving mechanism OR records that the question is open' was an unfalsifiable disjunction and does not satisfy this criterion. Then re-run CLAUDE.md's five refactor signals against src/check.rs, src/check/engine.rs, src/check/poly.rs and src/parser.rs, all four of which shrank materially, and record yes/no per signal (poly.rs's split is already deferred by a prior decision, so a signal firing there is not automatically a new action). Final gate: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`, assessed with `--no-fail-fast`.",
      "difficulty": "easy"
    }
  ]
}
```
