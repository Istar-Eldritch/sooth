# Phase 4 Slice 5a: native modules, imports, and encapsulation (spec)

Base: `main` @ `ab6083b` (slice 4 merged: quotations + `times`). Adds native multi-file
compilation, word and type imports by qualified name, per-file encapsulation with an
`export:` list, and selective unqualified import. REPL imports are slice 5b; this slice
ships a located REPL *rejection* of `import:`, not a REPL import path. Input brief:
`docs/phase4-slice5a-brief.md`, whose recon findings and D1..D9 (with the owner's
resolutions of D2, D3, and two smaller questions) are the source of these requirements.

## Central constraint

The parser resolves type names in a **pre-pass over raw tokens before any body parses**
(`prepass_type_decls`, `src/parser.rs:59`; `build_registries`, `:203`; both run inside
`parse`, `:240`, before word bodies). So an import cannot be a post-parse merge: the
importing file's own pre-pass already needs the imported file's type names present. The
attractive alternative (parse each file independently, merge afterwards) would require
remapping every positional `StructId`/`EnumId`/`ArrayId` in the second file's already-parsed
AST, which is strictly more work and more places to get it wrong. **The resolution model is
therefore fixed: resolve the import graph, order it topologically, reject cycles, run ONE
pre-pass across the whole import closure into ONE shared registry set, then parse bodies per
file against that shared set.** Everything else in this spec follows from that.

A second measured fact drives the encapsulation design: **a type name and its constructor
word are one identifier** (`type: Q a i64 ;` then `7 Q` constructs a `Q`). So `export: Queue`
has to say which it means; D3 resolves it as one unit, exporting the type together with its
generated words, because Sooth structs are dumb data and hiding their accessors buys little in
a language with no UB, trapped indexing, and linearity (and because visibility never protected
resource discipline anyway: destructure already bypasses a `drop` override today, measured).

## Load-bearing invariants preserved

- Backend stays QBE; `Ptr[T]` opaque; no LLVM, native backend, JIT, or comptime interpreter.
- The closure is assembled into **one `Module`**: `check::check(&mut module)`
  (`src/check.rs:1028`) keeps its single-module signature. Module identity rides on the
  decls (an owner tag), not on multiple `Module`s threaded through the pipeline. This is the
  cheapest change consistent with the merged-registry model.
- The linear spine is untouched: `dup`/`drop` semantics, move-by-default, use-exactly-once.
- `core` stays `no_std`.
- Green unchanged: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`. A
  single-file program mints exactly today's symbols and produces today's output (R22).

## Locked decisions (from the brief; D2/D3 and two smaller points resolved on the owner's behalf)

- **D1: closure resolution in the driver.** Resolve the import graph from the entry file,
  canonicalize and dedupe paths, order topologically, reject cycles (located, naming both
  files), run one shared pre-pass and one shared registry set across the closure, then parse
  bodies. A path is written with an explicit `.sth`, relative to the *importing* file, and
  deduped by canonical path (a diamond imports a file once).
- **D2 resolves to (b): name plus owning-module tag.** The registry stores the bare name with
  an owning module; resolution takes `(name, current_module, import_map)`, own module first,
  then imports by qualifier. Duplicate-type-name checking becomes per-module. Chosen because
  it keeps names file-agnostic in storage, at the cost of touching every
  `structs.iter().position(...)` site (`src/check.rs:6201`, `:6257`; `src/ast.rs:330`; the
  parser's effect resolver).
- **D3 resolves to transparent, with no opacity mechanism in this slice.** Naming a type in
  `export:` exports the type together with its generated words (constructor, getter, peek,
  setter, destructure). There is no `Queue(..)` distinction and no way to export a type
  abstractly. Sooth structs are dumb data, and hiding a constructor buys little here: there is
  no UB, indexing traps, and linearity prevents aliasing, so a bad field value is a bug in the
  consumer's own program rather than a hole in the library. The resource argument fails on a
  measured fact (destructure already bypasses a `drop` override today, single-file), so
  visibility never protected resource discipline. There is no per-member withholding in 5a:
  `export: Queue` is all-in (type and every generated word), and hiding accessors is the OOP
  ceremony this slice steps away from; a withhold marker is an additive feature for a real
  consumer, not a default. See the brief's D3 for the full reasoning and the three costs it
  specifies.
- **D4: exported signatures may only mention exported types.** Checked at the *declaration*
  (the module author's bug), located, naming the word and the private type.
- **D5: privacy is enforced at the use site**, by marking visibility, not by filtering names
  at merge/splice time. The diagnostic names the module and the fact of non-export, and is
  **distinct from `unknown word`**. An unexported name and a genuinely absent one must not
  produce the same error.
- **D6: no new disposal/export enforcement in 5a.** A destructor runs without being named:
  `drop` is compiler-known and dispatches on the concrete type (slice 3, 8b), so a consumer
  disposes an imported linear type with a bare `drop`, which runs the module's destructor glue
  whether or not it was exported. With D3 transparent there is a second route as well
  (destructure down to Copy leaves), so disposal is doubly reachable and the "export a disposal
  word" rule the ROADMAP hypothesizes has no case to fire on. Its real enforcement arrives only
  when a polymorphic `drop ( 'T -- )` can be structurally total (slice 8, whose constraint says
  exactly that a generic `drop` must *not* be total over resource types). 5a proves the
  positive: an imported linear type is disposed by `drop` and its destructor runs (R19).
- **D7: `import:` at the REPL is a located, tested rejection in this slice**, not deferred to
  5b. Slice 1 shipped without pinning REPL behaviour for polymorphic words and slice 2's
  recon found the gap had produced a silent miscompile; this slice will not repeat that shape.
- **D8: qualified names never reach the mangler.** Resolution maps a qualified spelling to a
  concrete decl before any symbol is minted; the emitted symbol gains a
  module-disambiguating component (minted like the existing `generation` suffix, so no
  punctuation reaches `instantiation_symbol`'s sanitizer, `src/ast.rs:488`) so two modules'
  same-named words get distinct symbols. A single-module closure adds no component (green).
- **D9: selective import, additive to the qualifier.** `import: q | a b | "path.sth" ;` binds
  the qualifier `q` as always *and additionally* exposes `a b` unqualified. One form with an
  optional clause. The collision rule is deliberately dumb: two selective imports exposing the
  same unqualified name is an error at the second import (located, naming both modules); a
  selectively-exposed name colliding with a locally-defined word is the same error. No
  precedence, no shadowing, no use-site ambiguity resolution.
- **The `.sth` extension is explicit in the path.** No implicit extension, no search path, no
  resolution rule to learn (consistent with `extern:` naming its C symbol verbatim).

## Requirements by stage

Diagnostics marked *(located)* are behavioural negatives whose golden asserts the
distinguishing wording and the named identifiers/positions, never merely that compilation
failed.

### Driver / closure resolution (`src/driver.rs`, a new resolution helper)

- **R1.** `driver::build` (`src/driver.rs:18`) is restructured from straight-line single-file
  (`read_to_string` -> `lex` -> `parse` -> `check` -> `lower` -> `emit`) to closure-based:
  resolve the import graph from the entry file, lex + pre-pass + parse the whole closure into
  **one** `Module`, then one `check`, one `lower`, one `emit`, one link. The single-binary,
  single-`Module` shape is preserved downstream of parsing.
- **R2.** Import-graph resolution. Each `import:` names a path with an explicit `.sth`,
  resolved **relative to the importing file's directory**, canonicalized, and **deduped by
  canonical path**: a file reached by two importers (a diamond) is parsed once and its decls
  registered once.
- **R3.** Topological order over the closure; **one shared pre-pass** across the whole
  closure's tokens into **one shared registry set** (structs/enums/arrays/cells/refs), then
  bodies parse per file against that shared set. No per-file-parse-then-merge; no positional
  id remapping.
- **R4. Import cycle** *(located)*. A cycle in the import graph is an error naming **both**
  files at the edge that closes it. A file importing itself is the degenerate case and is
  rejected with the same shape.
- **R5. Missing / unreadable import file** *(located)*. An `import:` whose resolved path does
  not exist or cannot be read is an error naming the importing site (line/col of the
  `import:`) and the path, distinct from a lexer/parser error on the target.

### Surface syntax / parsing (`src/parser.rs`, `src/ast.rs`; no lexer change)

- **R6. `import:` form.** `import: <qualifier> [ | <name>... | ] "<path>" ;` joins the
  existing `type:`/`extern:` defining-word family (dispatched by string compare in `parse`,
  `src/parser.rs:261`/`:267`). Parsed into an `Import` record: qualifier, optional selective
  name list (each with a span), path string, and the `import:` span. `::` and `|` need no
  tokenizer work (`is_delimiter` is `; ( ) | [ ]`, `src/lexer.rs:24`; `queue::push` lexes as
  one `Token::Word`).
- **R7. `export:` form.** `export: <name>... ;` parsed into a per-file export list (each name
  with a span). Multiple `export:` lines in one file accumulate (union). A file with no
  `export:` exports nothing (R14).
- **R8. Qualified reference `q::name`** is a single `Token::Word`; the parser and checker
  split on the first `::`. Both the **type parser** (a `q::Type` in a stack effect) and the
  **word/call resolver** (a `q::word` in a body) go through the module-aware resolver (R11).
- **R9. Parse errors** *(located)*. A malformed `import:` (missing qualifier, missing path
  string, unterminated before `;`) and a malformed `export:` are located parse errors naming
  the construct, parallel to the existing defining-word error arms. Tested by criterion 14.

- **R10.** Each `StructDecl`/`EnumDecl`/`WordDef`/`ExternDecl` gains an owning module id; the
  shared registry holds every module's decls in one set. The entry file is module 0.
- **R11. Module-aware name resolution.** Type-name and word resolution take
  `(raw, owner_module, import_map)`: an unqualified name resolves **own module first**; a
  `q::Base` splits, maps `q` through the owner's import map to a target module, and resolves
  there subject to visibility (R16). This is the change that touches every
  `structs.iter().position(|d| d.name == ...)` site (`src/check.rs:6201`, `:6257`;
  `src/ast.rs:330`) and the parser's effect resolver; a bare-name first-match is no longer
  sufficient once two modules can share a name.
- **R12. Per-module duplicate-type-name check.** `check_duplicate_type_names`
  (`src/check.rs:1558`) partitions by owning module: two modules may each declare `Point`; an
  intra-module duplicate still errors exactly as today. Without this, the merged registry
  turns two files' `Point` into today's `duplicate type` error (recon finding 4), which is
  safe but wrong.
- **R13. Whole-closure interning and drop discovery.** `find_drop_overloads`
  (`src/check.rs:942`, called at `:1031`) and array interning run once over the merged module,
  so an imported type's `drop` override is discovered in the same pass as the importer's and
  two files' `[i64 8]` dedupe into one `ArrayId`. The merged-registry model gives this for
  free; a per-file model would not.

### Visibility / encapsulation (`src/check.rs`)

- **R14. Default private.** A module exports nothing unless it has an `export:` (R7). Every
  existing single-file example is unaffected: it exports nothing and runs as a program, not a
  library.
- **R15. Transparent types (D3).** Naming a type in `export:` exports the type *and* its
  generated words (constructor, getter, peek, setter, destructure) as one unit: a consumer may
  name `q::Type` in an effect, construct one, and reach its fields. There is no opacity
  mechanism and no per-member withholding in this slice: `export: Queue` is all-in, and there
  is no syntax to export the type while withholding, say, its setter. That matches the slice's
  premise that Sooth structs are dumb data and hiding their accessors is OOP ceremony the
  language does not need; if a real consumer ever wants per-member control it is an additive
  feature (a withhold marker on the export list) landing with that consumer, not a default.
  Rationale, and the model to hold: a `type:` declaration introduces a **name-scope** whose
  generated words are its members (the spellings are literally `format!("{}>{}", type, field)`
  and siblings, `src/check.rs:1788-1795`, `src/ir.rs:624-627`, an ad-hoc qualified namespace
  built by string concatenation). Visibility over those members is the ordinary export
  mechanism applied to that scope, not a special rule for types.
- **R15b. Qualified accessor references resolve** *(new, a cost of R15)*. A spelling such as
  `q::Type>field` is valid and must resolve. `>` is not a delimiter (`src/lexer.rs:24-26`), so
  the whole thing is one `Token::Word`; it splits on the **first** `::` into qualifier `q` and
  member name `Type>field`, which then resolves against the qualified module by D2's
  `(name, current_module, import_map)` rule. This spelling was unreachable under the earlier
  opaque-by-default draft (it was always a visibility error), so it is genuinely new surface
  and needs its own golden, including for `<` and `|>` members.
- **R15c. A selectively-imported type brings its generated words** *(new, a cost of R15)*.
  `import: q | Type | "..."` exposes `Type` unqualified *and* its generated words unqualified
  (`Type>field`, `Type<field`, `Type|>field`), since R15 treats a type and its members as one
  exported unit. Those unqualified member names participate in R21's collision rule exactly
  like any other selectively-exposed name, so two modules exporting same-named types cannot
  both be selectively imported without a collision error.
- **R16. Use-site visibility diagnostic (D5)** *(located)*. A qualified or selectively-imported
  reference to a name that **exists in the target module but is not exported** is an error
  naming the module and the fact of non-export, e.g. `` `grow` is not exported from module
  `queue` (line N) ``. It is **distinct** from the unknown-word error
  (`unknown_word_error`, `src/check.rs:3753`) raised for a name **absent** from the target
  module. The two paths must produce different messages (tested, criterion 11 and unit U5).
- **R17. Enforcement is by marking, not filtering.** Every module's names are spliced into the
  shared environment and *marked* with their module and export status; rejection happens at the
  use site (R16). Filtering unexported names at merge time is forbidden because it collapses
  R16's two cases into one `unknown word`.

### Exported-signature and disposal rules (`src/check.rs`)

- **R18. Exported-signature rule (D4)** *(located)*. An exported word whose stack effect names
  a non-primitive type of its own module that is **not** exported is an error at the **export
  declaration** (the module author's bug), naming the word and the private type. Exporting the
  type satisfies the rule. The rule survives R15's transparency because a module can still
  declare a type it never exports at all and mention it in an exported word's effect.
- **R19. Disposal across the export boundary (D6).** 5a adds **no** export-site disposal
  enforcement, because there is no way to hold an undisposable imported value. Two independent
  routes reach disposal: a bare `drop`, which dispatches on the concrete type and runs the
  module's destructor glue whether or not that glue was exported (a destructor runs without
  being named), and, since R15 made types transparent, destructuring down to Copy leaves.
  Proven by a positive golden: an imported linear type disposed by `drop` in the consumer, its
  destructor observably running. Enforcement defers to slice 8, where a polymorphic `drop`
  could be structurally total and the premise becomes reachable for the first time.
- **R19b. Destructure-bypasses-`drop` is out of scope and must not be silently "fixed" here.**
  Measured, single-file, today: destructuring a type with a `drop` override skips the override
  (`type: R tag i64 ;` with a `drop` override, then `r R>tag .`, prints `7` and never runs the
  destructor). R15's transparency makes this reachable across a file boundary, which is a
  pre-existing language gap becoming newly reachable, not a new hole class: the earlier
  opaque-by-default draft was papering over it, and only for types whose author chose opacity.
  The real fix is a Rust-E0509-style rule (cannot destructure a type with a destructor), which
  is an ownership-checker rule independent of modules and recorded against slice 8. This slice
  must not grow it, and must not add a partial guard that would foreclose the general rule.

### Selective import (`src/parser.rs`, `src/check.rs`)

- **R20. Additive exposure (D9).** The optional `| name... |` clause binds the qualifier as
  always **and** exposes the listed names unqualified. Each listed name must be exported by the
  source module; a listed private name is the R16 visibility error. Unlisted names stay
  reachable only qualified (`q::grow`).
- **R21. Collision rule (D9)** *(located)*. Two selective imports exposing the same unqualified
  name is an error at the **second** import, naming both modules. A selectively-exposed name
  colliding with a locally-defined word is the same error. No precedence, no use-site
  disambiguation.

### Symbol minting (`src/ast.rs`, `src/ir.rs`)

- **R22. Module-disambiguated symbols (D8).** A qualified spelling is resolved to a concrete
  decl before any symbol is minted, so no `::` reaches `instantiation_symbol`
  (`src/ast.rs:488`, whose sanitizer maps punctuation to `_`). Two modules' same-named words
  mint **distinct** symbols via a module-disambiguating component added the way `generation`
  already is; the component flows through the check->lower instantiation table so check and
  emit never disagree (the slice-2 `RTLD_GLOBAL` hazard). A **single-module** closure adds no
  component: existing goldens and the entry symbol are byte-for-byte unchanged.

### REPL (`src/repl.rs`)

- **R23. `import:` at the REPL is a located rejection (D7)** *(located)*. `import:` as the first
  token of a REPL line returns `` `import:` is not supported at the REPL yet (line N, col C) ``
  before `parse_line_with_structs` is reached (guarded in `eval_line` beside the existing
  `type:` special-case), replacing today's misdirected `parse error: unexpected token Semicolon`
  (which carries a line/col but points at the `;` and never names `import:`).
  The `parse_line_with_structs` seam (`src/parser.rs:311`) is left intact for 5b to widen.

### Dogfood / docs

- **R24.** An `examples/` program dogfoods the slice: a small type exported from one file, a
  words file, imported and used together by an example that builds, links, and runs.
- **R25.** ROADMAP slice-5a marked implemented; DESIGN.md records the closure/merged-registry
  model, the type-as-name-scope framing, transparent type export, the use-site visibility rule,
  and the D1..D9 resolutions.

## Exit criteria

Goldens live in a new `tests/phase4_modules.rs` (source-in -> run output, or source-in ->
expected diagnostic), using a multi-file harness that writes the closure to a temp dir and
runs `driver::build`/`run`. Diagnostic goldens assert the distinguishing wording, never an IL
string and never a bare non-zero exit. Unit tests sit beside their stage functions per the
CLAUDE.md convention.

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | two files, importer calls `q::word`, compiles/links/runs to a value | `two_files_word_import_compiles_and_runs` | golden | 1 |
| 2 | importer names `q::Type` in an effect, binds one, runs | `imported_type_is_nameable_and_runs` | golden | 1 |
| 3 | two modules each declare `Point`, both compile and run | `same_named_types_in_two_modules_coexist` | golden | 1 |
| 4 | import cycle -> located error naming both files | `import_cycle_is_located_error_naming_both` | golden | 1 |
| 5 | self-import -> located error | `self_import_is_located_error` | golden | 1 |
| 6 | missing file -> located error naming importer and path | `missing_import_file_is_located_error` | golden | 1 |
| 7 | diamond import parsed once, program runs | `diamond_import_dedupes_by_canonical_path` | golden | 1 |
| 8 | path resolves relative to the importing file | `import_path_is_relative_to_importing_file` | golden | 1 |
| 9 | `import:` at the REPL -> located rejection | `import_at_repl_is_located_rejection` | golden | 1 |
| U1 | graph resolution canonicalizes, dedupes, orders | `resolve_import_graph_dedupes_and_orders` | unit (driver) | 1 |
| U2 | cycle detection returns the located both-files error | `import_cycle_detected_with_both_files` | unit (driver) | 1 |
| U3 | duplicate-type check is per-module | `duplicate_type_check_is_per_module` | unit (check) | 1 |
| U4 | resolution prefers own module, then qualifier | `type_resolution_prefers_own_module_then_qualifier` | unit (check) | 1 |
| U9 | same-named words in two modules mint distinct symbols; single module unchanged | `same_named_words_across_modules_get_distinct_symbols` | unit (ast/ir) | 1 |
| U10 | REPL `import:` guard returns the located error | `repl_rejects_import_with_located_error` | unit (repl) | 1 |
| U11 | `import:`/`export:` forms parse into their records | `import_and_export_forms_parse` | unit (parser) | 1 |
| 10 | unexported word from importer -> `not exported`, not `unknown word` | `unexported_word_is_not_exported_error` | golden | 2 |
| 11 | absent word in module -> unknown-word error, **differs from** #10 | `absent_word_in_module_is_unknown_not_unexported` | golden | 2 |
| 12 | exported type: qualified get, set, and peek accessors (`q::Type>f`, `q::Type<f`, `q::Type|>f`) all resolve | `qualified_accessors_get_set_peek_all_resolve` | golden | 2 |
| 13 | unexported type: `q::Type` is `not exported`, not `unknown word` | `unexported_type_is_not_exported_error` | golden | 2 |
| U5 | visibility lookup distinguishes unexported from absent | `visibility_lookup_distinguishes_unexported_from_absent` | unit (check) | 2 |
| U6 | exporting a type exports all five generated words (constructor, getter, peek, setter, destructure) as one unit | `export_of_type_includes_all_five_generated_words` | unit (check) | 2 |
| 14 | malformed `import:` (missing qualifier or path string, unterminated) -> located parse error naming the form | `malformed_import_form_is_located_parse_error` | golden | 2 |
| 15 | exported word naming a private type -> located declaration error | `exported_word_naming_private_type_is_error` | golden | 3 |
| 16 | exporting the type satisfies the rule (positive) | `exported_word_naming_exported_type_is_accepted` | golden | 3 |
| 17 | imported linear type disposed by bare `drop`, destructor runs | `imported_linear_type_is_disposed_by_drop` | golden | 3 |
| U7 | exported-signature helper flags a private type | `exported_signature_rule_flags_private_type` | unit (check) | 3 |
| 18 | selective import exposes names unqualified; qualifier still available | `selective_import_exposes_names_unqualified` | golden | 4 |
| 19 | selective import of a private name -> visibility error | `selective_import_of_private_name_is_error` | golden | 4 |
| 20 | two selective imports of one name -> error at second, naming both | `colliding_selective_imports_are_error_at_second` | golden | 4 |
| 21 | selective name colliding with a local word -> error | `selective_import_colliding_with_local_word_is_error` | golden | 4 |
| 21a | selectively importing a type exposes its generated words unqualified | `selective_import_of_type_exposes_members_unqualified` | golden | 4 |
| 21b | two selective type imports whose members collide -> error at second, naming both | `selective_type_import_member_collision_is_error` | golden | 4 |
| U12 | array shape `[i64 8]` declared in two files dedupes to one `ArrayId` in the merged registry | `array_shape_dedupes_across_files` | unit (check) | 4 |
| U8 | selective-import collision helper rejects | `selective_import_collision_is_rejected` | unit (check) | 4 |
| 22 | dogfood example builds, links, runs | `modules_example_builds_and_runs` | golden | 5 |

Load-bearing units (completeness is the point, per the project's history of placebo tests):
U3 (per-module dup), U4 (own-first resolution), U5 (unexported vs absent), U6 (all five
generated words export as one unit), U9 (symbol disambiguation, the slice-2 hazard), U12
(array-shape dedup across files). Each negative golden asserts the message text and the named
identifiers, not an op name or an exit code; criterion 12 asserts all three accessor shapes
(`>`, `<`, `|>`) and criterion 17 asserts the destructor's observable output, not exit 0.

## Phase sequencing (and why)

The parser/driver restructure lands before visibility, per the task's guidance and because
visibility is meaningless until qualified names resolve across a merged registry. Within the
restructure, the merged-registry work cannot be deferred past phase 1: even a *word-only*
import drags the exporting file's types into the shared registry (the imported word's effect
names them and codegen needs their layout), so the shared pre-pass, module tags, and
per-module duplicate check are all phase-1 foundations. `export:` **parses** from phase 1 but
is **unenforced** until phase 2, so phase-1 goldens can declare their export lists and survive
the default-private flip unchanged. The REPL rejection lands in phase 1, the same moment the
native parser learns `import:`, so no phase ships a degraded REPL (the exact regression D7
guards against). Phases 3 (declaration-site rules), 4 (selective import), and 5 (dogfood) each
add one independently reviewable capability on top.

Each phase leaves the tree green (`cargo fmt --check && cargo clippy -- -D warnings &&
cargo test`).

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Restructure driver::build into an import-closure pipeline: resolve the import graph relative to each importing file, canonicalize and dedupe by path, order topologically, reject cycles and self-import and missing files with located errors, run one shared pre-pass into one shared registry set assembled as a single Module, tag every decl with an owning module, make type-name and word resolution module-aware (own module first, then qualifier), make the duplicate-type-name check per-module, disambiguate same-named words across modules in symbol minting, parse the import: and export: forms (export parsed but not yet enforced), and reject import: at the REPL with a located error. R1-R13, R22, R23. Difficulty hard.",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Encapsulation: flip modules to default-private, enforce the export: list, and resolve export: of a type as transparent so naming a type exports it together with its generated constructor, getter, peek, setter and destructure words as one unit. A qualified accessor spelling like q::Type>field must resolve, splitting on the first :: since > is not a delimiter. Reject use-site access to an unexported name with a diagnostic that names the module and the fact of non-export, distinct from unknown word. R14-R17, R15b.",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Declaration-site and disposal rules: reject an exported word whose stack effect names a private non-primitive type of its own module (located, at the export declaration), and prove disposal crosses the export boundary by disposing an imported linear type with a bare drop whose destructor runs. Add no disposal enforcement and no partial destructure guard: destructure bypassing a drop override is a pre-existing language gap recorded against slice 8 and must not be half-fixed here. R18, R19, R19b.",
      "difficulty": "standard"
    },
    {
      "phase": 4,
      "focus": "Selective import: the optional additive | name... | clause that binds the qualifier and additionally exposes listed names unqualified, requiring each listed name to be exported, with the dumb collision rule that a second selective import of the same name or a collision with a local word is a located error naming both sources. A selectively imported type brings its generated words unqualified too, and those participate in the same collision rule. R20, R21, R15c.",
      "difficulty": "standard"
    },
    {
      "phase": 5,
      "focus": "Dogfood and docs: an examples/ program importing a type from one file and words from another, building and linking and running as one native program; ROADMAP slice-5a marked implemented and DESIGN.md updated with the closure/merged-registry/visibility model, the type-as-name-scope framing, and the D1-D9 resolutions. R24, R25.",
      "difficulty": "standard"
    }
  ]
}
```

## Out of scope

- **REPL imports (slice 5b):** what an import *means* in a session, generation-mangled
  redefinition of an imported module, reload-on-edit vs frozen bindings. This slice only
  *rejects* `import:` at the REPL (R23).
- **A serializable API description, version diffing, semver enforcement (Phase 6):** everything
  `docs/dependency-management.md` needs. It consumes the export list this slice introduces; it
  does not define it.
- **Package manifests, dependency resolution, a registry.**
- **A `mod.sth`-style directory-mirrors-module-tree convention** (declined: a flat
  file-is-a-module model with qualified access covers the only consumer that exists).
- **Re-exports; aliasing an import to a different local qualifier; wholesale unqualified import**
  (Factor's `USING:` shape, declined per D9's collision argument).
- **Generic type declarations crossing files (Phase 6):** they do not exist yet.
- **Any new disposal/export enforcement (D6):** deferred to slice 8, where a polymorphic `drop`
  first creates a case in which `drop` fails to reach disposal.
- **No new `Instr`/`Terminator`; no `qbe.rs` change beyond what R22's symbol component forces
  (expected: none, since the component is minted in `instantiation_symbol`'s existing seam).**

## Underspecified or internally inconsistent in the brief

1. **D6 is over-anticipated for 5a; resolved to a positive golden only.** The brief and ROADMAP
   frame a disposal/export rule ("an exported linear type must also export the word that
   discharges its obligation, or the consumer is stuck") as possibly enforced here. But `drop`
   in 5a is compiler-known and dispatches on the concrete type, so it always reaches an
   imported type's destructor glue (the ROADMAP itself notes "a destructor runs without being
   named"), and since D3 is transparent a consumer can also destructure down to Copy leaves. No
   case in 5a lets a consumer hold an undisposable value. The rule only bites once a
   polymorphic `drop ( 'T -- )` exists and could be structurally total, which is exactly what
   slice 8's constraint forbids. Resolved to R19, a positive golden, with enforcement deferred.
   Recorded because it reads as a decision to make here and the honest decision is that the
   premise is not yet reachable. *(This spec reached that conclusion from the requirements; the
   owner reached it independently from the design side. Agreement from two directions, not one
   source repeated.)*

2. **The brief's own D3 reversed mid-authoring, and this spec follows the revision.** An earlier
   draft resolved D3 to opaque-by-default after Elm; the settled resolution is transparent with
   no opacity mechanism, on the grounds that Sooth structs are dumb data, a violated invariant
   is a bug in the consumer's own program rather than unsoundness (no UB, trapped indexing,
   linearity), and visibility never protected resource discipline in the first place. Review
   should check R15/R15b/R15c against the brief's revised D3 rather than against the Elm
   framing, and should treat the three named costs of transparency as requirements, not asides.

3. **Qualified accessor spellings are now valid and need resolution machinery, the reverse of
   the earlier draft.** Under opaque-by-default, `queue::Queue>buf` was always a visibility
   error, so its tokenization never had to resolve. Under transparency it is ordinary valid
   code, so R15b specifies the resolution: one `Token::Word` (since `>` is not a delimiter),
   split on the **first** `::`. This is the largest single consequence of the D3 reversal and the
   most likely place for an implementer to under-build, since the earlier framing explicitly
   told them not to.

4. **Multiple `export:` lines.** The brief's examples show one `export:` per file; it does not
   say whether a second is an error or accumulates. This spec chooses **accumulate/union**
   (R7), the least surprising and cheapest choice; noted as a decision the brief left open.
