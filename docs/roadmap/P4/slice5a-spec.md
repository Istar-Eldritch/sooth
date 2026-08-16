# Phase 4 Slice 5a: native modules, imports, encapsulation

**Status: implemented.** Base `main` @ `ab6083b`. Adds native multi-file compilation, word/type imports by qualified name, per-file encapsulation via `export:`, and selective unqualified import. REPL imports are slice 5b; this slice ships only a located REPL *rejection* of `import:`.

## Core resolution model (the fixed constraint)

The parser resolves type names in a pre-pass over raw tokens *before any body parses* (`prepass_type_decls`, `build_registries`, both inside `parse`). So imports cannot be a post-parse merge: the importing file's pre-pass needs the imported file's type names present. Merging independently-parsed ASTs would require remapping every positional `StructId`/`EnumId`/`ArrayId`, which is strictly more error-prone.

**Therefore, fixed:** resolve the import graph → order topologically → reject cycles → run ONE pre-pass across the whole closure into ONE shared registry set → parse bodies per file against that shared set. The closure assembles into a single `Module`; `check::check(&mut module)` keeps its single-module signature, with module identity carried on the decls (an owner tag).

A second measured fact: a type name and its constructor word are one identifier (`type: Q a i64 ;` then `7 Q`). So export operates on that unit (see D3).

## Locked decisions

- **D1** Driver resolves the import graph from the entry file: canonicalize, dedupe by canonical path (diamond imports once), topo-order, reject cycles, one shared pre-pass + registry set, then parse bodies. Paths are explicit `.sth`, relative to the *importing* file.
- **D2** Registry stores bare name + owning-module tag; resolution is `(name, current_module, import_map)`, own module first then imports by qualifier. Duplicate-type-name check becomes per-module.
- **D3** Transparent export, no opacity mechanism. Naming a type in `export:` exports it *with* its generated words (constructor, getter, peek, setter, destructure) as one unit. No `Queue(..)` distinction, no per-member withholding. Rationale: Sooth structs are dumb data; no UB, indexing traps, linearity prevents aliasing, so a bad field value is a consumer bug, not unsoundness. Visibility never protected resource discipline anyway (destructure already bypasses a `drop` override today, measured).
- **D4** Exported signatures may only mention exported types; checked at the declaration, located, naming word and private type.
- **D5** Privacy enforced at the use site by marking visibility, not filtering names at merge time. The `not exported` diagnostic is distinct from `unknown word`.
- **D6** No new disposal/export enforcement. A destructor runs without being named: `drop` is compiler-known, dispatches on concrete type, runs the module's destructor glue whether or not exported. Transparency gives a second route (destructure to Copy leaves). Real enforcement defers to slice 8 (polymorphic `drop` that could be structurally total).
- **D7** `import:` at the REPL is a located, tested rejection in this slice (guards against the slice-1/2 silent-miscompile shape).
- **D8** Qualified names never reach the mangler: resolution maps a qualified spelling to a concrete decl before any symbol mints. Emitted symbols gain a module-disambiguating component minted like the existing `generation` suffix (no punctuation hits `instantiation_symbol`'s sanitizer). Single-module closures add no component (byte-for-byte green).
- **D9** Selective import is additive: `import: q | a b | "path.sth" ;` binds qualifier `q` *and* exposes `a b` unqualified. Dumb collision rule: two selective imports of the same unqualified name → error at the second (naming both modules); collision with a local word → same error. No precedence, no shadowing, no use-site disambiguation.
- `.sth` extension explicit; no implicit extension, no search path (consistent with `extern:`).

## Requirements by stage

**Driver / closure (`src/driver.rs`, `src/resolve.rs`)**
- **R1** `driver::build` restructured to closure-based: resolve graph → lex+pre-pass+parse whole closure into one `Module` → one check/lower/emit/link.
- **R2** Each `import:` path is explicit `.sth`, relative to importer's dir, canonicalized, deduped by canonical path.
- **R3** Topo order; one shared pre-pass into one shared registry set (structs/enums/arrays/cells/refs); bodies parse per file. No per-file-then-merge, no positional id remapping.
- **R4** *(located)* Import cycle → error naming both files at the closing edge; self-import is the degenerate case.
- **R5** *(located)* Missing/unreadable import → error naming the importing site (line/col) and path, distinct from a lex/parse error on the target.

**Parsing (`src/parser.rs`, `src/ast.rs`; no lexer change)**
- **R6** `import: <qualifier> [ | <name>... | ] "<path>" ;` joins the `type:`/`extern:` defining-word family. Parsed to an `Import` record (qualifier, optional name list with spans, path, `import:` span). `::` and `|` need no tokenizer work.
- **R7** `export: <name>... ;` → per-file export list (names with spans). Multiple `export:` lines accumulate (union). No `export:` = exports nothing.
- **R8** Qualified `q::name` is one `Token::Word`; parser/checker split on the *first* `::`. Both type parser and word/call resolver go through the module-aware resolver.
- **R9** *(located)* Malformed `import:`/`export:` → located parse errors naming the construct.
- **R10** `StructDecl`/`EnumDecl`/`WordDef`/`ExternDecl` gain an owning module id; entry file is module 0.
- **R11** Module-aware resolution `(raw, owner_module, import_map)`: unqualified → own module first; `q::Base` maps `q` through owner's import map subject to visibility. Touches every `structs.iter().position(...)` site and the effect resolver.
- **R12** Per-module duplicate-type-name check (`check_duplicate_type_names` partitions by owning module); intra-module duplicate still errors.
- **R13** Whole-closure interning + drop discovery run once over the merged module (`find_drop_overloads`, array interning); two files' `[i64 8]` dedupe to one `ArrayId`.

**Visibility (`src/check.rs`)**
- **R14** Default private: no `export:` = exports nothing. Existing single-file examples unaffected (run as programs).
- **R15** Transparent types: `export:` a type exports it and its five generated words as one unit. No opacity, no per-member withholding. Model: a `type:` decl is a name-scope whose generated words (`format!("{}>{}", ...)` and siblings) are its members; visibility is the ordinary export mechanism over that scope.
- **R15b** *(new)* Qualified accessor `q::Type>field` must resolve: one `Token::Word` (`>` not a delimiter), split on first `::` into `q` and `Type>field`. Needs its own golden incl. `<` and `|>`.
- **R15c** *(new)* A selectively-imported type brings its generated words unqualified too; those participate in R21's collision rule.
- **R16** *(located)* Reference to a name that exists but isn't exported → error naming the module and non-export (e.g. `` `grow` is not exported from module `queue` ``), distinct from `unknown word` (absent name).
- **R17** Enforcement by marking, not filtering. All names spliced into the shared env, marked with module + export status; rejection at use site. Filtering at merge time forbidden (collapses R16's two cases).

**Declaration-site / disposal (`src/check.rs`)**
- **R18** *(located)* Exported word whose effect names an unexported own-module non-primitive → error at the export declaration, naming word and private type. Survives transparency (a type may be declared but never exported).
- **R19** No export-site disposal enforcement. Positive golden: imported linear type disposed by bare `drop`, destructor observably runs. Two routes: `drop` (dispatches on concrete type), and destructure to Copy leaves. Defers to slice 8.
- **R19b** Destructure-bypasses-`drop` is out of scope; must not be silently fixed or half-guarded. Pre-existing single-file gap made newly reachable across a file boundary; real fix is a Rust-E0509-style rule recorded against slice 8.

**Selective import (`src/parser.rs`, `src/check.rs`)**
- **R20** `| name... |` binds qualifier *and* exposes listed names unqualified. Each listed name must be exported (else R16 error). Unlisted names reachable only qualified.
- **R21** *(located)* Second selective import of the same unqualified name → error naming both modules; collision with a local word → same error.

**Symbol minting (`src/ast.rs`, `src/ir.rs`)**
- **R22** Qualified spelling resolved to concrete decl before any symbol mints (no `::` reaches `instantiation_symbol`). Same-named words across modules mint distinct symbols via a module-disambiguating component flowing through the check→lower instantiation table (avoids the slice-2 `RTLD_GLOBAL` hazard). Single-module closure adds no component.

**REPL (`src/repl.rs`)**
- **R23** *(located)* `import:` as first REPL token → `` `import:` is not supported at the REPL yet (line N, col C) `` before `parse_line_with_structs`, guarded in `eval_line` beside the `type:` special-case. The seam is left intact for 5b.

**Dogfood / docs**
- **R24** `examples/` dogfoods the slice: a type exported from one file, words in another, imported and run together.
- **R25** ROADMAP slice-5a marked implemented; DESIGN.md records the closure/merged-registry model, type-as-name-scope framing, transparent export, use-site visibility, and D1–D9.

## Load-bearing invariants preserved

QBE backend, `Ptr[T]` opaque, no LLVM/native/JIT/comptime. One `Module` downstream of parsing. Linear spine untouched (`dup`/`drop`, move-by-default, use-exactly-once). `core` stays `no_std`. Green unchanged; single-file programs mint today's symbols and output (R22).

## Phase sequencing

1. **(hard)** Restructure `driver::build` into the import-closure pipeline: graph resolution, dedupe, topo-order, located cycle/self-import/missing-file errors, shared pre-pass + registry, module tags, module-aware resolution, per-module duplicate check, symbol disambiguation, parse `import:`/`export:` (export parsed, unenforced), REPL rejection. R1–R13, R22, R23.
2. **(standard)** Encapsulation: default-private flip, enforce `export:`, transparent type export with generated words, qualified accessor resolution, use-site `not exported` diagnostic. R14–R17, R15b.
3. **(standard)** Declaration-site + disposal: reject exported word naming a private own-module type; prove disposal crosses the boundary via bare `drop`; add no disposal enforcement / no partial destructure guard. R18, R19, R19b.
4. **(standard)** Selective import: additive `| name... |` clause, must-be-exported, dumb collision rule; selectively-imported types bring generated words. R20, R21, R15c.
5. **(standard)** Dogfood + docs. R24, R25.

The parser/driver restructure lands first (visibility is meaningless until qualified names resolve). Merged-registry work can't defer past phase 1: even a word-only import drags the exporting file's types into the shared registry. `export:` parses in phase 1 but is unenforced until phase 2. REPL rejection lands in phase 1 (no phase ships a degraded REPL). Each phase leaves the tree green.

## Exit criteria

Goldens in `tests/phase4_modules.rs` (multi-file harness writing the closure to a temp dir, running `driver::build`/`run`). Diagnostic goldens assert distinguishing wording, never an IL string or bare exit code. Units sit beside their stage functions.

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | two files, importer calls `q::word`, runs to a value | `two_files_word_import_compiles_and_runs` | golden | 1 |
| 2 | importer names `q::Type` in an effect, binds one, runs | `imported_type_is_nameable_and_runs` | golden | 1 |
| 3 | two modules each declare `Point`, both run | `same_named_types_in_two_modules_coexist` | golden | 1 |
| 4 | import cycle → located error naming both | `import_cycle_is_located_error_naming_both` | golden | 1 |
| 5 | self-import → located error | `self_import_is_located_error` | golden | 1 |
| 6 | missing file → located error naming importer + path | `missing_import_file_is_located_error` | golden | 1 |
| 7 | diamond parsed once, runs | `diamond_import_dedupes_by_canonical_path` | golden | 1 |
| 8 | path relative to importing file | `import_path_is_relative_to_importing_file` | golden | 1 |
| 9 | REPL `import:` → located rejection | `import_at_repl_is_located_rejection` | golden | 1 |
| U1 | graph resolution canonicalizes/dedupes/orders | `resolve_import_graph_dedupes_and_orders` | unit (driver) | 1 |
| U2 | cycle detection returns both-files error | `import_cycle_detected_with_both_files` | unit (driver) | 1 |
| U3 | duplicate-type check per-module | `duplicate_type_check_is_per_module` | unit (check) | 1 |
| U4 | resolution prefers own module then qualifier | `type_resolution_prefers_own_module_then_qualifier` | unit (check) | 1 |
| U9 | same-named words → distinct symbols; single module unchanged | `same_named_words_across_modules_get_distinct_symbols` | unit (ast/ir) | 1 |
| U10 | REPL guard returns located error | `repl_rejects_import_with_located_error` | unit (repl) | 1 |
| U11 | `import:`/`export:` forms parse | `import_and_export_forms_parse` | unit (parser) | 1 |
| 10 | unexported word → `not exported`, not `unknown` | `unexported_word_is_not_exported_error` | golden | 2 |
| 11 | absent word → unknown-word, differs from #10 | `absent_word_in_module_is_unknown_not_unexported` | golden | 2 |
| 12 | qualified get/set/peek accessors all resolve | `qualified_accessors_get_set_peek_all_resolve` | golden | 2 |
| 13 | unexported type → `not exported`, not `unknown` | `unexported_type_is_not_exported_error` | golden | 2 |
| U5 | visibility distinguishes unexported from absent | `visibility_lookup_distinguishes_unexported_from_absent` | unit (check) | 2 |
| U6 | export of type includes all five generated words | `export_of_type_includes_all_five_generated_words` | unit (check) | 2 |
| 14 | malformed `import:` → located parse error | `malformed_import_form_is_located_parse_error` | golden | 2 |
| 15 | exported word naming private type → located error | `exported_word_naming_private_type_is_error` | golden | 3 |
| 16 | exporting the type satisfies the rule | `exported_word_naming_exported_type_is_accepted` | golden | 3 |
| 17 | imported linear type disposed by `drop`, destructor runs | `imported_linear_type_is_disposed_by_drop` | golden | 3 |
| U7 | exported-signature helper flags private type | `exported_signature_rule_flags_private_type` | unit (check) | 3 |
| 18 | selective import exposes unqualified; qualifier stays | `selective_import_exposes_names_unqualified` | golden | 4 |
| 19 | selective import of private name → visibility error | `selective_import_of_private_name_is_error` | golden | 4 |
| 20 | two selective imports of one name → error at second | `colliding_selective_imports_are_error_at_second` | golden | 4 |
| 21 | selective name colliding with local word → error | `selective_import_colliding_with_local_word_is_error` | golden | 4 |
| 21a | selectively imported type exposes generated words | `selective_import_of_type_exposes_members_unqualified` | golden | 4 |
| 21b | colliding selective type-import members → error at second | `selective_type_import_member_collision_is_error` | golden | 4 |
| U12 | `[i64 8]` in two files dedupes to one `ArrayId` | `array_shape_dedupes_across_files` | unit (check) | 4 |
| U8 | selective-import collision helper rejects | `selective_import_collision_is_rejected` | unit (check) | 4 |
| 22 | dogfood example builds/links/runs | `modules_example_builds_and_runs` | golden | 5 |

Load-bearing units (guard against placebo tests): U3, U4, U5, U6, U9, U12. Each negative golden asserts message text + named identifiers, not an op name or exit code; #12 asserts all three accessor shapes; #17 asserts the destructor's observable output.

## Out of scope

REPL imports (5b); serializable API / version diffing / semver (Phase 6); package manifests / registries; `mod.sth` directory convention (declined for flat file-is-a-module); re-exports / import aliasing / wholesale unqualified import (declined per D9); cross-file generic type decls (Phase 6); any new disposal/export enforcement (slice 8); no new `Instr`/`Terminator`, no `qbe.rs` change beyond R22's component.

## Decisions the brief left open (resolved here)

1. **D6 over-anticipated** → resolved to a positive golden only (R19). `drop` always reaches an imported type's destructor glue and transparency adds a destructure route, so no case in 5a lets a consumer hold an undisposable value. Enforcement defers to slice 8. (Reached independently from requirements and design sides.)
2. **D3 reversed mid-authoring** from opaque-by-default (Elm) to transparent, no opacity. Review R15/R15b/R15c against the *revised* D3, treating the three transparency costs as requirements.
3. **Qualified accessor spellings now valid**, the reverse of the earlier draft: R15b specifies resolution (one `Token::Word`, split on first `::`). Largest consequence of the D3 reversal, most likely to be under-built.
4. **Multiple `export:` lines** → accumulate/union (R7), the cheapest choice.

## Implementation

New module `src/resolve.rs` carries closure resolution. Delivered across five commits, one per phase, each green:

- **Phase 1** `6fd46d6` — ast, check, driver, ir, lib, parser, repl, resolve
- **Phase 2** `b404872` — driver, parser, resolve, tests/phase4_modules.rs
- **Phase 3** `08132ee` — check, driver, resolve, tests
- **Phase 4** `1610058` — ast, check, driver, parser, resolve, tests
- **Phase 5** `793215d` — DESIGN.md, ROADMAP.md, examples/{modules,modules_ops,modules_point}.sth, tests
