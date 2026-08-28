# P7.S9 — remove the REPL (condensed, implemented)

A deletion slice with one type-level restructuring (`Ctx` enum → struct) and one large test
migration. No new language behaviour, no new diagnostic, no golden newly accepting or
rejecting. Mutation testing therefore applied to the *migrated* tests, and deletion
completeness was proved by grep over `src/`, `tests/` and `docs/roadmap/`, not by a green
build.

Shipped in 11 phases: `Library` relocation (1), test migration off the bare-line path
(2–3 check/ir side, 4–6 integration side), the forced deletion unit (7), the bare-line
entry-point family (8), incremental-compile state (9), doc sweep (10), exit sweep (11).
Per-phase classifications, mutation tables and findings live in
[`slice9-phase-notes.md`](./slice9-phase-notes.md).

## 1. What the schedule turned on

1. **The bare-line path was load-bearing test infrastructure**, reached mostly through
   per-file helpers (three `infer_src` copies, `infer_variant_line`,
   `ir/test_helpers.rs`'s `line_terms`, `backend/qbe.rs`'s `emit_line`) but with two sites
   bypassing every helper: the `Ctx::Line` half of
   `check_dup_of_drop_overload_type_names_the_cause`, and
   `releasable_into_withholds_a_name_used_in_a_back_edge_body` — a general `Liveness` test
   that never builds a `Ctx` and so was invisible to a `Ctx::Line`-keyed inventory. Missing
   it would have broken phase 8's `cargo test` *compile*. Migration ran against the live
   mechanism, before any deletion.
2. **Two whole integration files were REPL suites**, one of them `tests/phase1.rs` (49
   tests, no non-REPL member, six Phase 1 exit criteria plus the separately stated
   `calculator_session_dogfood`). `tests/phase4_repl_imports.rs` (23 tests) was Phase 4
   slice 5b's entire exit-criterion set. Those criteria die with the REPL and are recorded
   retired in the phase files that state them, not in `ROADMAP.md` alone.
3. **`dead_code` forcing is narrower than "everything REPL-adjacent".** `pub` items in
   `pub` modules do not warn, so `parser::parse_line`, `parse_line_with_structs`,
   `ast::Line` and `ir::lower_line` were separable (phase 8), while the `pub(crate)`/private
   `infer_line`, `InferredLine`, `check_poly_combinator_repl` and `pub(super) Ctx` were one
   compile unit with the REPL's deletion (phase 7).

## 2. Rulings

**`Ctx` is a struct, not a single-variant enum.** `Ctx::Line`'s only distinguishing content
was "no word to cite", served by placeholders. Keeping the variant would keep a second
checking path alive, which is what the slice exists to kill. Splitting the collapse from the
type flip is unbuildable: every `Line` arm's `Word` counterpart cites the word, so there is
nothing to rewrite the `Line` arm *to* while the variant exists, short of manufacturing a
synthetic `WordDef` payload.

**Three methods stay `Option`, and this is load-bearing, not tidiness.** `modules` (a
retained poly word passes `None` off the whole-program path), `generics` (`None` off
`check::check`) and `static_type` (a name may genuinely not be a static). Collapsing
`modules` or `generics` would silently *arm* gates that deliberately never fire.

**`calculator_session_dogfood` is retired, not migrated.** Its subject is the interactive
session (seven lines fed one at a time, asserting the per-line echo); there is no `run` form
of it. Its ordinary language facts were confirmed covered by named `run` tests first.

**There is no separate `resolve` deletion.** The brief's claim that a REPL call site passes
`false` into `resolve::resolve_modules` is wrong; what the REPL passed `false` to was
`driver::assemble_module` (`repl.rs:2055`), which died with the file. `resolve_modules`, its
`always_mangle` parameter and the single-module forcing that closes the QBE symbol-hijack
class are untouched, and `tests/symbol_hijack.rs` stayed green as the proof.

**E2's grep is substring-with-exclusions, not `-w`.** `-w` dodges `replace`/`replica` but
goes blind to compound identifiers: `grep -rnwiE 'repl'` matches
`check_poly_combinator_repl`, `repl_unknown_capability_error` and
`reject_generic_typedef_in_repl` zero times — three of this slice's own deletion targets.
The shipped form is
`grep -rniE 'repl|session|dlopen|override_epoch|drop_generation' src/ | grep -viE 'replac|replic'`.
Likewise `#[ignore]` is counted with `grep -rnE '^[[:space:]]*#\[ignore' tests/`; the loose
form also matches note bodies quoting the attribute.

## 3. Requirements as delivered

**R1 — `Library` relocated.** `Library`/`open`/`symbol`, `last_dlerror`, the
`dlopen`/`dlsym`/`dlerror` externs and the `RTLD_*` constants moved into `src/driver.rs`
beside `compile_so` (already there — no motion was manufactured to satisfy the roadmap's
prose). `fflush` stayed behind and died with `repl.rs`. `Library` stays `pub`, callerless,
as the load-bearing host→Sooth primitive. `library_opens_and_resolves_a_compiled_symbol`
carries `repl.rs`'s transmute-and-call (`sq(5) == 25`), so "the resolved symbol is a usable
fn pointer" did not die with the REPL. Two doc comments were de-sessioned, not one: `open`'s
"objects loaded by later lines" was REPL-line prose in a phrasing E2's grep cannot see.

**R2/R3 — bare-line unit-test harness migrated.** Check side: 8 tests + 2 helpers + one
quotation-audit row retired as coverage of a mechanism that dies with the REPL; the rest
migrated onto `infer_probe_body` (a new `#[cfg(test)] pub(super)` helper in
`check/engine.rs` walking a source string as a synthetic one-word body) or `word_ctx`
directly. All 30 mutations killed. `captures.rs`'s double-`Ctx` test kept **both** content
assertions on the surviving `Ctx::Word` call. IR side: `ir/test_helpers.rs` ended the phase
with zero `parse_line` dependency; session stack-marshalling tests retired; shared lowering
facts re-expressed over `ir::lower` on a one-word module.
`lower_call_uses_resolved_generation_symbol` was deliberately migrated and kept green as
`generation`'s only unit-level witness until phase 9 deleted the field.

**R4 — integration suites retired.** `tests/repl_ux.rs` and `tests/phase4_repl_imports.rs`
deleted in full (4 module-system facts migrated into `tests/phase4_modules.rs` first, 11
named as already covered, 7 retired as session mechanism, 1 recorded as a gap).
`tests/phase1.rs` deleted in full with nothing migrated: all six exit criteria already had a
`run` golden over the *same* `examples/` source, each named. The remaining 14 single-surface
files lost their spawn helpers and REPL-driving tests; `tests/common/mod.rs`'s
`repl_core_*`/`REPL_CORE_ECHO` went once callerless. Corpus `#[ignore]` count 13 → 3.

**R5a + R6 — the REPL deleted and `Ctx` flipped, one commit.** `src/repl.rs`,
`src/editor.rs`, both `pub mod` lines, the `repl` subcommand and `usage()` line,
`driver::repl`. **The forced cascade was larger than the spec's four items**: `check_def`,
`check_def_collecting_drop_sites`, `ResolvedCalls`, `check_drop_overload_reachability`,
three `CombinatorEnv` items, `ir::lower_word`, `ir::lower_instantiation` and
`ir::layout::DropOverride` also fell; `discover_closure`, `TraitCtx::scratch` and
`TraitResolveCtx::scratch` were narrowed to `#[cfg(test)]`. `DropOverride` collapsed to
`&WordDef` rather than becoming a single-variant enum, on the same reasoning as the `Ctx`
ruling. **61 match arms collapsed, not 73** — the spec's count included
`check/engine.rs`'s own 12-arm method block, rewritten as field reads.
`rendered_word_or`'s `fallback` died across 41 call sites; `mangled_name`/`effect`/
`declared_outputs` shed their `Option` and their six consumers with them. No diagnostic text
on the surviving path changed, proved by diffing every changed string literal; zero
`tests/qbe_baseline` diffs.

**R5b — bare-line entry points deleted.** `ast::Line`, `parse_line`,
`parse_line_with_structs` and the rest of the family, `reject_generic_typedef_in_repl`,
`repl_unknown_capability_error`, the `is_repl` field with all 8 assignments and both readers
(the `Ord` wording fork collapsed to its non-REPL arm), `ir::lower_line` and its `ir.rs`
re-export, plus the family-only parser unit tests.

**R7 — incremental-compile state narrowed; the stop condition did not fire.**
`CallInst::generation` (the struct is `CallInst`, not `Instantiation`),
`instantiation_symbol`'s third parameter, the `(PolySig, Option<u64>)` pairing in `PolyEnv`,
the three layouts' `drop_generation`, `Cells::drop_generations`, the four minters' `epoch`
parameter, the override-epoch doc block and every `None` seed and argument. Four consumer
sites the spec did not name were enumerated by `cargo build` (two `check/terms.rs`
destructures on the live dispatch path, two `check/poly.rs` signatures, three extra
`instantiation_symbol` call sites). Regenerating `tests/qbe_baseline` produced an **empty**
diff — stronger than the spec's condition: off the REPL path the suffix was never emitted.
Two retired tests, one renamed (`instantiation_symbol_reproduces_native_spelling_expected`,
assertion verbatim), both surviving minters mutation-proved.

**R8 — doc and invariant sweep.** Derived from E2's grep, which by then returned 115 hits
across 23 files (phases 4–9 had already retired most of the surface). `CLAUDE.md`'s
invariant now reads "`driver::Library` loads a compiled `.so` in-process via `dlopen`";
`DESIGN.md`, `ROADMAP.md`, `P7`'s S9 entry, `docs/design/{control-flow,modules,codegen}.md`,
`P8`'s manifest-read prose and declined-exemption note, and `P12:14`/`:19` all state the
post-removal design with no removal narrative. `memory-model.md` had zero mentions and was
out of scope. Two doc-prose items outside the derived list were caught by the phases that
changed their subject (`driver.rs`'s "earlier generations" comment,
`P7-language-prereqs.md`'s S3v `drop_generation` claim) — neither matches E2's word list, so
both would otherwise have survived the slice.

## 4. Findings worth carrying

- **A committed mutation regressed `struct_base` and was wrongly accepted as a fix.**
  `9b24f99` changed `struct_base.push(structs.len())` to `push(0)` in `assemble_module`,
  claiming parity with `enum_base` (which pushes `enums.len()`), then reworded the failing
  test's comment to match the broken behaviour. Reverted in `e948ea0`.
- **A `::` in a declared name is silently accepted on the native path.** The REPL rejected
  it; natively `type: q::T x i64 ;` and `: q::foo ( -- ) ;` build clean, the name is
  uncallable (`unknown word`), and an import binding `q` wins outright. The gap is a
  dead-but-accepted declaration, not a shadowing hazard. Left for whichever phase owns
  native declaration validation.
- **`E5` failed on first pass in `P7-language-prereqs.md`**: `:366`, `:384`, `:514` named
  `src/repl.rs`'s `lower_instantiation` in present tense and `:709-715` described a live
  REPL diagnostic. Fixed in phase 11. A grep over `src/` alone does not prove the roadmap
  clean.
- **`always_mangle` is now unreachable-but-retained.** Every surviving non-test caller
  passes `true`; the `false` path lives only in `resolve.rs`'s own four unit tests. Kept
  deliberately: the single-module forcing it selects closes the QBE symbol-hijack class.
  Do not "tidy" it.
- `cargo clippy --all-targets` remains red at HEAD in three pre-existing places
  (`src/parser.rs`'s `bool_comparison`, two `needless_borrow`s in
  `tests/phase4_combinators.rs`), outside the `clippy -- -D warnings` gate.

## 5. Exit criteria, as met

All ten pass; no criterion recorded blocked.

| # | Result |
| --- | --- |
| E1 | `sooth repl` → `unknown command: repl`, exit 2; no `repl` in `usage()` |
| E2 | Six hits, all `driver.rs`'s relocated `Library`/`dlopen` primitive; `drop_generation`/`override_epoch` zero |
| E3 | `Library` (`:983`) and `compile_so` (`:945`) both in `driver.rs`; both unit tests green |
| E4 | `grep -rn 'Ctx::Line' src/` empty; `modules`/`generics`/`static_type` still `Option` |
| E5 | Per-item grep clean; the only `tests/` hits are retirement-record comments |
| E6 | Exactly `phase7_slice3b_follow.rs`'s 3 non-REPL notes |
| E7 | Per-test mutation results recorded in phases 2–6; phase 9's three deletions and one rename accounted for |
| E8 | `P4:295`, **both** `P1` Exit and Dogfood lines, and `ROADMAP.md`'s P1 row updated |
| E9 | `P12:19` names `driver::Library`'s `dlopen` over a `compile_so` output, records no open question; `:14` is a plain `REPL/` strip |
| E10 | `fmt --check` clean, `clippy -- -D warnings` clean, `cargo test --no-fail-fast` 2673 passed / 0 failed / 3 ignored, zero baseline diffs |

**Split signals at slice exit** (`check.rs` −505, `check/engine.rs` −66, `check/poly.rs`
−152, `parser.rs` −463): three of four fire nothing; `poly.rs`'s X-and-Y-and-Z signal is a
repeat of the already-deferred split decision, not a new action.

## 6. Follow-up, out of scope

The book rewrite: `docs/book/{preface,getting-started,words,the-stack}.md` and
`the-interactive-book.md` dropping from `SUMMARY.md:55`. Recorded, not started.
