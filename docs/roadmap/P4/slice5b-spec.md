# Phase 4 Slice 5b: REPL imports

**Status: implemented.** Makes `import:` work at the REPL. Slice 5a shipped a located *rejection* of `import:` in a session (R23/D7); this slice removes that rejection and gives the REPL the same closure-based `import:`/`export:`/selective-import semantics the native path has, on top of the session's existing frozen-callee-generation model.

## The core reuse (fixed constraint)

The machinery already existed; this slice is an application, not new design:

1. Bulk-compiling a module is the REPL's own per-line path (`Session::eval_def`, `src/repl.rs:923`) with a longer `funcs` vector: lower each word's body to `ir::Func`, add `synthesize_aggregate_destructors` glue, one `IrModule`, `backend::qbe::emit`, `driver::compile_so` (`src/driver.rs:303`).
2. Native import-closure resolution is reused: `driver::discover_closure` / `assemble_module` (`src/driver.rs:71,151`) parse the import graph, dedupe by canonical path, reject cycles, run one shared pre-pass, then `resolve::resolve_modules` mangles same-named decls apart, producing a checked `Module` (module-tagged decls).
3. The frozen-callee-generation rule answers reload-vs-frozen for free: a re-run `import:` is an ordinary redefinition applied to a *batch* of names.

**Therefore fixed:** the REPL's `import:` reuses `discover_closure` / `assemble_module` / `check::check` unchanged, bulk-lowers the `Module` through the single-`.so` path, and splices module 0's exports into the session under their qualified spelling, minting one fresh import epoch per event. No new compilation strategy, IR, or `qbe.rs` change.

## The one hazard: positional id remap

The session's `structs` / `enums` / `arrays` / `owned_cells` / `refs` registries are flat, append-only, positionally indexed (`StructId = index`). Native assembles the whole closure into one pre-pass and never remaps a positional id. The REPL has an *already-populated* registry and appends an imported batch on top, so splicing an imported aggregate whose fields reference other imported aggregates by closure-local index **forces an id remap into session index space** (R9). This is the load-bearing correctness point and why phase 1 is `hard`.

## Locked decisions

- **D1** Registry growth on re-import is accepted, not deduped: each `import:` mints a fresh batch of struct/enum/array/cell/ref entries, no content-hash dedup, no cap. Matches 5a's dumb-default philosophy.
- **D2** Selective import (`import: q | a b | "path.sth" ;`) ships at full native parity: same additive-to-qualifier semantics, same collision rule (error at the second, naming both).
- **D3** A REPL session overriding an imported type's `drop` is out of scope.
- **D4** An imported file declaring `main` is a located rejection at import time, naming file and word. Native's own exposure stays unfixed (recorded on ROADMAP).

## Resolved open questions

- **Q1** (R3) The REPL's own top-level `import:` resolves relative to the *process cwd*; transitive imports inside the closure keep 5a's importer-relative rule.
- **Q2** (R13) `import: q "different.sth" ;` when `q` is bound is a reload (rebind, fresh batch), no "you changed the target" error, with a precise `q::old` rule.
- **Q3** (R10) Transitive re-export stays closed: a third file imported by the imported file contributes no session-visible name.
- **Q4** (R2) Diagnostics fall out via the ordinary `scan_imports` / `parse_import` path (`src/parser.rs`).
- **Q5** (R17) The concrete dogfood session.

## Requirements by stage

**REPL entry and import-form parsing (`src/repl.rs`, `src/parser.rs`)**

- **R1** Remove 5a's R23 pre-parse rejection in `eval_line` (`src/repl.rs:499`); route `import:` (first token) to a new `Session::eval_import`, guarded beside the `type:` special-case before `parse_line_with_structs`. `export:` as a REPL line is a **new located rejection** naming the construct (a live session has no export boundary).
- **R2** *(located)* `eval_import` parses via the shared `scan_imports` / `parse_import` (R9 path fixed in `82b7a19`), so a malformed REPL `import:` yields R9's construct-naming located error. Confirmed by a REPL golden.

**Path resolution (Q1)**

- **R3** The REPL's own `import:` is explicit `.sth`, no search path, resolved relative to the **process cwd**; transitive imports keep 5a's importer-relative rule. Exactly one new frame of reference.

**Closure compilation reuse (`src/driver.rs`, `src/repl.rs`)**

- **R4** `eval_import` reuses `discover_closure(path)` → `assemble_module` → `check::check(&mut module)` unchanged. `discover_closure` / `assemble_module` elevated to `pub(crate)`; located cycle / self-import / missing-file errors (5a R4/R5) reused verbatim.
- **R5** The whole closure's words plus `synthesize_aggregate_destructors` glue lower into one `IrModule` → `emit` → `compile_so` → one `.so`, `dlopen`ed and retained in `self.libs`. Same call sequence as `eval_def`, longer `funcs` vector. No new `Instr`/`Terminator`, no `qbe.rs` change.

**Import-event generations and reload (recon #3)**

- **R6** Each `import:` mints one fresh session-wide **import epoch** (`self.import_epoch: u64`, incremented once per event, same shape as `override_epoch`). Every *word* gets symbol `{name}__import{epoch}`, distinct from an ordinary word's `{name}__gen{N}`. Cross-event/cross-scheme collision-freeness holds by construction (an `__import{epoch}` symbol's trailing digits are never preceded by `n`; `import_epoch` is globally unique). **Intra-event same-closure duplicate word names inherit the pre-existing native gap** (no located duplicate-word-name check anywhere; word env silently overwrites via `HashMap::insert`; native already leaks a bare `symbol already defined` assembler error) — recorded on ROADMAP, not patched here. Reload and D1 growth fall out for free. **Generated struct/enum accessor words get no symbol and need none**: they lower inline, recognized by spelling against `self.structs`/`self.enums` (`build_registries_ww`'s `swords` map, `src/ir.rs:573`; sigs via `struct_generated_sigs` `src/check.rs:1972` + `check_struct_peek_word` `src/check.rs:6395`, both keying off `self.structs`). Splicing the type is sufficient for all five forms.
- **R7** Registry growth on re-import accepted (D1). A session that never imports is byte-for-byte unchanged.

**Splicing exports (nameability and id remap)**

- **R8** *(load-bearing)* Only module 0's `export:`ed decls are spliced into callable session state (5a R14). Five mechanisms, each verified against its exact function:

  **(a)** Each import event gets its own distinct `module: u32` (not `module: 0`), one past the highest live module id at splice time. Prevents `check_duplicate_type_names` (`src/check.rs:1764`, comparing `(module, name_static)`) from firing a spurious duplicate on a later unrelated `type:` line. `name_static` stays the pretty user-typed spelling (`"q::T"`, no tag). This id also serves (d)'s type-position lookups, so it's reserved and stable for the qualifier's lifetime.

  **(b)** The stored `.name` (not `.name_static`) is epoch-tagged (`{q}::{T}__import{epoch}`) — but only for the `.name`-only, module-oblivious accessor/constructor recognizers (`struct_generated_sigs`, `swords`, `check_struct_peek_word`), so they agree on exactly one row per internal spelling on reload. Plays **no role** in type-position resolution (that's (d)). Mirrors `resolve_modules`' own `.name`/`.name_static` split.

  **(c)** A REPL-side term-rewrite pass mirroring `resolve_modules::rewrite_terms` (`src/resolve.rs:265`), scoped to body-position word/accessor calls only. `self.import_aliases: HashMap<String, String>` maps each user-facing spelling (`q::w`, `q::T`, selective bare `w`/`T`) to its current internal name. After `parse_line_with_structs`, a pass splits any bound-qualifier `TermKind::Word` on the first accessor operator (via `split_accessor` logic, `src/resolve.rs:44`), translates the base through `import_aliases`, re-appends the suffix; then the alias-oblivious checking path runs unchanged. Reload/rebind overwrites the alias entry; old rows stay resident (positional stability), unreferenced by any current alias, so frozen callers keep resolving.

  **(d)** Type-*position* references resolve through native's own `Parser::resolve_type` (`src/parser.rs:1563`) → `ast::resolve_type_name_in_module` (`src/ast.rs:136`), which already takes `imports`/`selective` module-id maps. The three REPL parser entry points (`parse_line_with_structs` `src/parser.rs:465`, `parse_typedef_line` `:509`, `parse_enum_typedef_line` `:561`) each gain real `imports`, `selective`, and `exports` params (previously hardcoded empty), built fresh per parse from the live qualifier table. Threading real `exports` is required: `type_is_exported` (`src/parser.rs:1594`) reads `self.exports.get(target)`, and an empty slice makes every qualified type reference falsely `not exported` once `imports` is non-empty. `find_type_in_module` (`src/ast.rs:160`) switches `s.name == name` to `s.name_static == name` (audited behavior-preserving: only reached pre-`resolve_modules`, where `.name == .name_static`).

  **(e)** A REPL-declared name containing `::` is a **new located rejection**, closing (b)'s tag from being user-forgeable (the lexer treats `:` and digits as word chars). Rejected at `eval_typedef`'s `type:` name (`src/repl.rs:540`) and `eval_line`'s `Line::Def` arm (`src/repl.rs:527-531`, before the drop/def/poly fan-out). **REPL-only**, not folded into shared `reject_reserved_name` (which also guards native `.sth` parsing). Native's identical latent gap is inert (no epoch-tagged name to collide with) — recorded on ROADMAP.

  Selective import of a type is a *second alias* pointing at the *same* internal spelling / `StructId`, not a second decl (native single-registry-entry parity). An exported word `w` is inserted into `self.env` under its `{name}__import{epoch}` symbol + remapped `Sig`; `import_aliases["q::w"]` points at it. An exported type's decl is appended to `self.structs`/`self.enums` under its epoch-tagged `.name` (module id per (a), fields remapped per R9). Private names retained per qualifier in `self.import_private: HashMap<String, HashSet<String>>` — a private word's bare name, and a private type's bare name plus its five generated accessor spellings (pre-enumerated so R15's lookup is a flat membership check). A polymorphic exported word (`poly_words`) whose signature names a concrete imported struct is out of scope (cross-file generics are Phase 6).

- **R9** *(load-bearing)* Splicing **remaps every type-id carried** (`Type::Struct`/`Enum`/`Array`/`OwnedCell`/`Ref`) from closure-local to session indices, shifting by current registry lengths at splice time, and mints each spliced decl its module id + epoch-tagged `.name` (R8a/b) in the same step. Covers struct/enum field decls, the array/owned-cell/ref registries, and every spliced word's `Sig` (a `Vec<Type>`, equality by positional id). **The closure's own internal `module: u32` remaps too**, separate from R8a: a multi-file closure arrives with per-file ids `0..N-1` disambiguating same-`name_static` files; those are base-shifted into a fresh disjoint session sub-range (not collapsed to R8a's one id, which would reintroduce collisions). Single-file is the degenerate N=1 case. Generated accessor sigs need no separate remap (regenerated from already-remapped decls). Word *bodies* need no remap (`TermKind` carries only name strings; R8c runs before checking).
- **R10** *(Q3)* Transitive re-export stays closed: only module 0's export list crosses under `q`. Stated, with a golden.

**Selective import at the REPL (D2)**

- **R11** `import: q | a b | "path.sth" ;` is additive at native parity: `q` binds *and* `a b` are spliced unqualified via a *second* `import_aliases` entry rewriting to the same internal epoch-tagged spelling (R8c). A selectively-imported type brings its generated words unqualified (5a R15c) and gets the parallel `selective` map entry R8d needs, pointing at the same module id its qualified spelling targets — one `StructId`, genuine parity. Each listed name must be exported by module 0, else 5a's visibility error, via `check::check_selective_imports` (`src/check.rs:1517`) applied to a synthesized entry for the REPL's top-level selection.
- **R12** *(located)* A selectively-exposed unqualified name colliding with an existing **session** name is a located error at the second, naming both. Extends 5a R21's collision rule to session scope: no precedence, no shadowing.

**Qualifier rebinding (Q2)**

- **R13** Re-running `import: q "..." ;` when `q` is bound (same or different path) is treated as redefinition: rebind, fresh epoch (R6), fresh module id (R8a), replace `q`'s `import_aliases` entries **and** its retained private-name set. "Replace" = overwrite every `import_aliases` key resolving to something owned by `q` (exact-prefix scan `key.starts_with("q::")`, plus any selective bare spelling) to the new epoch's spelling; old-epoch rows never touched (positional stability). No "you changed the target" error. A frozen call against `q::old` keeps working under `RTLD_GLOBAL`; a *new* `q::old` reference is judged against the *new* closure's splice + private set only (`not exported` if newly private, `unknown word` if absent), never a stale hit.

**`main` in a library (D4)**

- **R14** *(located)* If any closure word is named `main`, `eval_import` rejects at import time naming file and word, before codegen. Turns recon #4's latent native collision (`check_main_effect` `src/check.rs:2186` finds first `main` with no uniqueness check; `mangle` exempts `main`) into a diagnostic on this path. Native exposure stays unfixed (ROADMAP).

**Session integrity**

- **R15** A qualified `q::x` that fails ordinary lookup is checked against `q`'s retained private-name set before `unknown word`: a real-but-non-exported name raises 5a's exact `not exported` wording (`not_exported_error`, reused verbatim); only a name absent from both is `unknown word`. Reproduces 5a R16 at the REPL via one per-qualifier name set, not session-wide visibility marking.
- **R16** On any `eval_import` failure (parse, missing file, cycle, `main`-in-library, selective collision, check error) the session (env, stack, registries, generations, `libs`) is left **untouched**, matching `eval_line`'s commit-only-on-success contract.

**Dogfood and docs (Q5)**

- **R17** A REPL-session dogfood (piped stdin, `tests/`) imports a real library and: (a) calls `q::w` and `q::T>field` to a value; (b) re-imports after an edit, observing a frozen caller beside fresh resolution; (c) exercises the `main`-in-library rejection; (d) exercises selective import including the unqualified call.
- **R18** ROADMAP slice-5b marked implemented (native `main`-collision recorded for later). DESIGN.md "Modules and encapsulation" gains a REPL-import paragraph (cwd frame R3, import-as-redefinition R6, transitive re-export closed R10, `main`-in-library R14, no dedup/cap D1/R7); "REPL late binding for redefinition" notes import reload rides the frozen-generation rule, not late-binding.

## Load-bearing invariants preserved

QBE backend, `Ptr[T]` opaque, no LLVM/native/JIT/comptime. One `Module` downstream of `assemble_module`. Linear spine untouched (`dup`/`drop`, move-by-default, use-exactly-once). `core` stays `no_std`. Frozen-callee-generation unchanged; each import event mints its own epoch (R6), collision-free by construction. A session that never imports mints today's symbols and output byte-for-byte (R7).

## Phase sequencing (as delivered)

1. **(L, hard)** Core REPL import path. Route `import:` to `eval_import` (`export:` new located rejection); elevate `discover_closure`/`assemble_module` to `pub(crate)`; cwd frame; bulk-lower to one `.so`; import-epoch symbols (accessors none); distinct module id + base-shifted closure module ids (R8a/R9); `import_aliases` indirection (R8b/c); thread real `imports`/`selective`/`exports` maps + `find_type_in_module` name_static match (R8d); reject `::` in declared names (R8e); splice module 0 exports with positional-id remap; retain private names; qualified word/accessor/type-position resolution; transitive re-export closed; malformed-import located error; commit-only-on-success. R1–R10, R15, R16. → `49029a2` (src/ast.rs, src/driver.rs, src/parser.rs, src/repl.rs, src/resolve.rs, tests/phase4_modules.rs, tests/phase4_repl_imports.rs)
2. **(M, standard)** Reload/rebind/`main`-in-library. Fresh epoch recompiles all words while frozen caller keeps old; reload overwrites `import_aliases` not registry rows (unrelated `type:` line unaffected, resolution non-divergent); qualifier rebinding (R13); `main`-in-library rejection (R14). → `cb33f08` (src/driver.rs, src/repl.rs, tests/phase4_repl_imports.rs)
3. **(M, standard)** Selective import: additive `| name... |` via a second `import_aliases` entry pointing at the same internal entry (same `StructId`), must-be-exported, session-scope collision. R11, R12. → `5bd3b62`, `2c1960d` (src/check.rs, src/repl.rs, tests/phase4_repl_imports.rs)
4. **(S, standard)** Dogfood + docs. R17, R18. → `381c3ae` (DESIGN.md, ROADMAP.md, tests/phase4_repl_imports.rs)

## Exit criteria

Goldens in `tests/phase4_repl_imports.rs` (REPL-session harness, piped stdin, asserts stdout, writes lib files to a temp dir, sets process cwd for R3). Diagnostic goldens assert distinguishing wording, never IL strings or exit codes. Units beside stage functions in `src/repl.rs`.

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | import two-word library, call `q::w`, runs to value | `repl_import_word_is_callable_qualified` | golden | 1 |
| 2 | import type, name `q::T`, construct, read `q::T>field` | `repl_import_type_accessor_resolves` | golden | 1 |
| 2a | imported type in type *position*: signature + `type:` field naming `q::T` | `repl_import_type_resolves_in_signature_and_typedef_position` | golden | 1 |
| 2b | declaring a `::`-containing REPL name is a located rejection | `repl_double_colon_in_declared_name_is_located_rejection` | golden | 1 |
| 3 | real-but-non-exported name is `not exported`, distinct from absent | `repl_qualified_private_name_is_not_exported` | golden | 1 |
| 4 | carried struct-of-struct renders (id remap) | `repl_imported_nested_struct_ids_remap` | golden | 1 |
| 5 | path resolves relative to process cwd | `repl_import_path_is_relative_to_cwd` | golden | 1 |
| 6 | third file imported by library stays invisible | `repl_transitive_reexport_stays_closed` | golden | 1 |
| 7 | cycle / missing file reuse located native errors | `repl_import_cycle_and_missing_are_located` | golden | 1 |
| 8 | malformed `import:` is R9's located error | `repl_malformed_import_is_located_error` | golden | 1 |
| 9 | failed import leaves session untouched | `repl_failed_import_leaves_session_intact` | golden | 1 |
| U1 | spliced aggregate's field ids point at session indices | `imported_aggregate_ids_remap_to_session_space` | unit (repl) | 1 |
| U2 | spliced word's epoch symbol distinct from ordinary + prior import | `import_epoch_symbols_are_session_fresh` | unit (repl) | 1 |
| U3 | `discover_closure`/`assemble_module` yield a checked module | `repl_assembles_checked_module_for_library` | unit (repl) | 1 |
| U6 | retained private-name set distinguishes not-exported from absent | `import_private_names_distinguish_not_exported_from_absent` | unit (repl) | 1 |
| U8 | `find_type_in_module` matches name_static, module-gated | `find_type_in_module_matches_name_static_module_gated` | unit (ast) | 1 |
| 10 | reload: frozen caller keeps old, new call sees new | `repl_reimport_freezes_existing_caller` | golden | 2 |
| 10a | reload a type, then unrelated `type:` succeeds (no spurious dup) | `repl_reimport_of_type_leaves_unrelated_typedef_unaffected` | golden | 2 |
| 10b | reload with changed field type: post-reload constructor + word agree | `repl_reimport_of_type_resolution_does_not_diverge` | golden | 2 |
| 11 | rebind to different file; frozen `q::old` works, new judged vs new file | `repl_qualifier_rebind_frozen_and_rejudged` | golden | 2 |
| 12 | `main` in imported file rejected naming file+word; session untouched | `repl_import_of_library_declaring_main_is_rejected` | golden | 2 |
| U4 | scan rejects an imported `main` | `imported_main_is_rejected_by_scan` | unit (repl) | 2 |
| U7 | reload overwrites `import_aliases`; old row stays resident | `import_alias_reload_overwrites_not_appends` | unit (repl) | 2 |
| 13 | selective import exposes name unqualified; qualifier still binds | `repl_selective_import_exposes_unqualified` | golden | 3 |
| 13a | selective type import: unqualified + qualified are one `StructId` | `repl_selective_type_import_aliases_one_struct_id` | golden | 3 |
| 14 | selective import of a private name is a visibility error | `repl_selective_import_of_private_is_error` | golden | 3 |
| 15 | selective name colliding with session-local errors at second, naming both | `repl_selective_import_collides_with_local` | golden | 3 |
| U5 | session-scope selective-collision check rejects | `session_selective_collision_is_rejected` | unit (repl) | 3 |
| 16 | dogfood session runs all of R17 (a–d) | `repl_modules_dogfood_session_runs` | golden | 4 |

Load-bearing units to mutation-test: U1, U2, U4, U5, U6, U7, U8. U3 excluded (reachability smoke test, no guarded negative). #2a is the regression witness for the type-position gap (R8c's rewrite cannot reach a signature/`type:` field; only R8d can); #2b for R8e's forgeability guard. #10/#11 assert the frozen caller's *old* output beside the fresh *new* output in one session; #10a the unrelated `type:` still succeeds after reload; #10b/#13a two spellings reaching the same type produce the *same* behavior.

## Out of scope

Everything 5a deferred stays deferred: package/registry layer, re-exports, aliasing an import to a different local qualifier, wholesale unqualified import, cross-file generics (Phase 6); new disposal/export enforcement, the destructure-bypasses-`drop` gap (slice 8). Slice-specific: REPL override of an imported type's `drop` (D3); fixing the `main` collision on the *native* path (recon #4, ROADMAP); content-hash dedup or size cap on re-import growth (D1). No new `Instr`/`Terminator`, no `qbe.rs` change.
