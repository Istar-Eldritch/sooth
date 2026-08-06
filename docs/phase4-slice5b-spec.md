# Phase 4 Slice 5b: REPL imports

**Status: specified.** Base `main` @ `60ebb51`. Makes `import:` work at the REPL. Slice 5a shipped a deliberate located *rejection* of `import:` in a session (R23/D7), so no phase ever shipped a degraded import path; this slice removes that rejection and gives the REPL the same closure-based `import:`/`export:`/selective-import semantics the native path has, on top of the session's existing frozen-callee-generation model.

## What this slice is not (the fixed constraint)

Recon measured that the machinery already exists and this slice is an application of it, not new design:

1. **Bulk-compiling a module is the REPL's own per-line path with a longer `funcs` vector.** `Session::eval_def` (`src/repl.rs:923`) already lowers one word's body to an `ir::Func`, appends `ir::synthesize_aggregate_destructors` glue, wraps one `IrModule`, `backend::qbe::emit`, then `driver::compile_so` (`src/driver.rs:303`), which takes arbitrary QBE text and produces a `.so`. Compiling *N* words (an imported file's whole word set) plus their glue is the same call sequence, not a new strategy.
2. **The native import-closure resolution is directly reusable.** `driver::discover_closure` / `assemble_module` (`src/driver.rs:71,151`) parse an import graph rooted at a file, dedupe by canonical path, reject cycles, run one shared pre-pass into one shared registry set, then `resolve::resolve_modules` (`src/resolve.rs`) mangles same-named decls apart. It produces a checked `Module` (structs, enums, words, each carrying a `module: u32` tag) the REPL can bulk-lower exactly as recon #1 describes.
3. **The frozen-callee-generation rule (DESIGN.md, decided) answers reload-vs-frozen for free.** Every REPL word is frozen at whichever generation of its callees existed when it compiled; redefining a word never retroactively changes an already-compiled caller. A re-run `import:` line is an ordinary redefinition event applied to a *batch* of names, not a new rule.

**Therefore, fixed:** the REPL's `import:` reuses `discover_closure` / `assemble_module` / `check::check` unchanged, bulk-lowers the resulting `Module` through the existing single-`.so` path, and splices the entry file's exports into the session under their qualified spelling, minting one fresh import epoch (R6) per import event. No new compilation strategy, no new IR, no `qbe.rs` change.

## The one hazard this slice actually introduces

The session's `structs` / `enums` / `arrays` / `owned_cells` / `refs` registries are flat, append-only, positionally indexed: `StructId = index`, so a carried value keeps meaning what it meant when minted. Native never re-parses a file twice in one process and assembles the whole closure into *one* pre-pass, so it never has to remap a positional id (5a's core-resolution note: parse-then-merge was rejected precisely because it would force remapping every positional id). The REPL has an *already-populated* registry and appends an imported batch on top of it, so splicing an imported aggregate whose fields reference other imported aggregates by closure-local index **does** force an id remap into session index space. This is the load-bearing correctness point of the slice (R9), and the reason phase 1 is `hard`.

## Locked decisions (from the brief, carried verbatim)

- **D1** Registry growth on re-import is accepted, not deduped. Re-running an `import:` line mints a fresh batch of struct/enum/array/cell/ref entries every time, exactly as a redefined word mints a fresh generation every time. No content-hash dedup, no cap. Matches the dumb-default philosophy 5a used for import collisions.
- **D2** Selective import (`import: q | a b | "path.sth" ;`) ships at full parity with native: same additive-to-the-qualifier semantics, same collision rule (error at the second, naming both sources). The same mangle-and-splice mechanism recon #1/#2 cover, not new design.
- **D3** A REPL session overriding an imported type's `drop` is out of scope. Native's rule ("disposal crosses the export boundary for free") stays as-is; whether a session can later type a `: drop` line naming an imported struct is deferred to whoever asks.
- **D4** An imported file declaring `main` is a located rejection, at import time, naming the file and the word. The native path's own exposure to this stays unfixed (recorded on ROADMAP), out of scope here.

## Open questions the brief left for this spec (resolved here)

- **Q1 Path resolution frame of reference** to R3: the REPL's own top-level `import:` resolves relative to the *process current working directory*; every transitive import *inside* the closure keeps 5a's importer-relative rule.
- **Q2 Qualifier rebinding** to R13: `import: q "different.sth" ;` when `q` is already bound is treated identically to a reload (rebind, mint a fresh batch), with no "you changed the target" error, and a precise rule for a `q::old` reference after the rebind.
- **Q3 Transitive re-export stays closed** to R10: a third file imported by the imported file contributes no session-visible name, consistent with 5a declining re-exports.
- **Q4 Diagnostics fall out via the ordinary parser path** to R2: confirmed against `scan_imports` / `parse_import` (`src/parser.rs`), not assumed.
- **Q5 Dogfood shape** to R17: the concrete session test the brief sketched.

## Requirements by stage

**REPL entry and import-form parsing (`src/repl.rs`, reusing `src/parser.rs`)**

- **R1** Remove 5a's R23 pre-parse rejection in `eval_line` (`src/repl.rs:499`). `import:` as the first token routes to a new `Session::eval_import`, guarded beside the `type:` special-case and before `parse_line_with_structs` (which never learns about qualifiers). `export:` as a REPL line stays a *located* rejection naming the construct (a live session has no export boundary to cross), so no leading defining-word ever falls through to a misdirected `unknown word` / `unexpected token` message.
- **R2** *(located)* `eval_import` parses the line into an `Import` via the existing shared form parser (`scan_imports` / `parse_import`, `src/parser.rs`, the R9 path fixed in `82b7a19`), so a malformed `import:` at the REPL yields R9's construct-naming located error unchanged. Confirmed by a REPL golden, not assumed, since the REPL's line entry differs from the native scan entry.

**Path resolution (Q1)**

- **R3** The REPL's own `import:` path is explicit `.sth`, no search path (consistent with 5a and `extern:`), resolved relative to the **process current working directory**. Every transitive `import:` *inside* the discovered closure keeps 5a's rule: relative to its own importing file. So exactly one frame of reference is new (the REPL's top line = cwd); everything below it is unchanged native resolution.

**Closure compilation reuse (`src/driver.rs`, `src/repl.rs`)**

- **R4** `eval_import` reuses the native pipeline unchanged: `discover_closure(path)` to `assemble_module` to `check::check(&mut module)`, producing a checked `Module` (module-tagged decls, same-named decls already mangled apart by `resolve_modules`, single-module component rules from 5a R22). `discover_closure` and `assemble_module` are elevated from private `fn` to `pub(crate)` (their lowest common ancestor is now `driver` + `repl`); the located cycle / self-import / missing-file errors (5a R4/R5) are reused verbatim, not reimplemented.
- **R5** The whole closure's words plus `synthesize_aggregate_destructors` glue lower into one `IrModule`, `backend::qbe::emit`, `driver::compile_so` to one `.so`, `dlopen`ed and retained in `self.libs` (recon #1). Intra-closure calls resolve within that one `.so`; the same call sequence `eval_def` uses, with a longer `funcs` vector. No new `Instr`/`Terminator`, no `qbe.rs` change.

**Import-event generations and reload (recon #3, D-covered)**

- **R6** Each `import:` evaluation mints one fresh, session-wide **import epoch** (`self.import_epoch: u64`, incremented once per import event, never reused, the same shape as the existing `override_epoch`). Every *word* compiled in that closure gets a symbol `{name}__import{epoch}`, where `name` is the word's own already-intra-closure-unique spelling straight off the checked `Module` (module-disambiguated by `resolve_modules` if the closure itself has 2+ modules), tagged with a fixed `__import` marker distinct from an ordinary session word's `{name}__gen{N}` symbol. Cross-event and cross-scheme collision-freeness are provably sound by construction, not by sanitizing a compound string and hoping: `mangled_symbol`'s literal suffix is `__gen{N}` (`src/repl.rs:137`), an `__import{epoch}` symbol's trailing digit run is never preceded by `n`, and `import_epoch` is globally unique per event, so an ordinary word's symbol and a cross-event collision are both unreachable regardless of what `name` is. **Intra-event, same-closure duplicate word names are not guarded by any existing check and this scheme inherits that gap rather than closing it**: there is no located duplicate-word-name check anywhere in the compiler (`check_duplicate_type_names`, `src/check.rs:1764`, covers structs/enums only; the word env silently overwrites via `HashMap::insert`), so a *native* build of a file with two same-named words already leaks a bare `symbol already defined` assembler error today, verified directly. An imported closure inherits this exact pre-existing hole unfixed; it is not a new hazard this slice introduces, and it is recorded on ROADMAP against the same place as the `main`-collision (recon #4) rather than patched here as a drive-by. Reload (R13) and D1's unconditional growth both fall out for free: re-running `import:` mints a *new* epoch and recompiles every word fresh under it, whether or not the file actually changed, exactly D1's choice applied to symbols instead of registry entries. **Generated struct/enum accessor words get no symbol at all and need none.** They lower inline, recognized purely by their spelling against `self.structs`/`self.enums`: `ir::lower`'s `swords` map (`src/ir.rs:615`) covers all five forms (construct, destructure, get, set, peek), and their signatures are regenerated the same way an ordinary session-local type's already are today, though not through one single function -- `check::struct_generated_sigs` (`src/check.rs:1972`, already merged into `typed_env`, `src/repl.rs:375`) covers construct/destructure/get/set, while peek is checked separately by `check_struct_peek_word` (`src/check.rs:6385`), which string-splits `Struct|>field` and resolves it against the struct registry directly. Both paths key off `self.structs` alone, so splicing the type is still sufficient for all five. So splicing the type into `self.structs`/`self.enums` under `q::T` with its fields remapped (R8, R9) is sufficient on its own; no accessor ever needs an env entry, a symbol, or a `dlopen`-resolved call.
- **R7** Registry growth on re-import is accepted (D1): a re-import appends a fresh batch of struct/enum/array/cell/ref entries every time, no content-hash dedup and no cap, consistent with a redefined ordinary word minting a fresh generation. A single-module session that never imports is byte-for-byte unchanged.

**Splicing exports into the session (nameability and id remap)**

- **R8** Only module 0's `export:`ed decls are spliced into callable session state (5a R14: `export:` is the only way a name leaves its file). An exported word `w` becomes a `self.env` entry keyed by its **qualified spelling** `q::w`, bound to its import-epoch symbol (R6) and its `Sig`, remapped per R9. An exported type `T` is appended to `self.structs` / `self.enums` under the qualified display name `q::T` with its fields remapped per R9; its five generated words need **no separate env entry and no symbol** (R6) -- the existing inline accessor path already regenerates `q::T>field` / `q::T<field` / `q::T|>field` and siblings from the struct decl the moment `self.structs` holds it, the same way an ordinary session-local type's accessors already work. Nothing unexported is spliced into callable or checkable state. A plain (post-import) REPL line's own type-name resolution must treat `q::T` as one literal, opaque struct-decl name (an exact string match against `self.structs[i].name`), *not* re-split it on `::` through native's qualifier-map-aware `Parser::resolve_type` (`src/parser.rs:1584`) -- the REPL has no per-line qualifier map to gate on, so that path is native-only and must not be reused verbatim here. Alongside the splice, the session additionally retains, per qualifier, the full set of **reference spellings** module 0's non-exported words and types would answer to (not bodies, not symbols, no synthesized env entry or symbol either): a private word's bare name, and for a private type, its bare name *plus* the same five generated accessor spellings its export path would have produced (`T>field`, `T<field`, `T|>field` and siblings) -- `self.import_private: HashMap<String, HashSet<String>>`, keyed by qualifier. Pre-enumerating every accessor spelling at retention time is what lets R15's lookup stay a flat membership check rather than needing to know, at lookup time, whether a name is a word or a type. A polymorphic exported word (`poly_words`) whose signature names a concrete imported struct is out of scope for this slice's remap (cross-file generic types are Phase 6, per the brief's out-of-scope list); state this explicitly rather than let R9 silently assume every exported word is monomorphic.
- **R9** *(load-bearing)* Splicing an imported closure into the session's positionally-indexed registries **remaps every type-id it carries** (`Type::Struct` / `Enum` / `Array` / `OwnedCell` / `Ref`) from closure-local indices to session indices, shifting by the session's current registry lengths at splice time. This covers every carrier the ids reach: struct/enum field declarations (base-shifted the way `assemble_module`'s `struct_base` / `enum_base` shift within a closure), the array/owned-cell/ref registries (interned into the closure's own shared vecs during `parse_bodies` rather than base-shifted the same way, but still carrying ids that must remap), and, easy to miss, every spliced word's `Sig` (R8), since a `Sig` is `Vec<Type>` and `Type::Struct(StructId, ..)` equality is by that positional id. A generated accessor word's signature needs **no separate remap**: it is never stored, only regenerated on demand by `struct_generated_sigs` from the struct/enum decl's own (already-remapped) field types (R6/R8), so remapping the declaration once is sufficient. Word *bodies* need no remap: `TermKind` carries no `Type`/`StructId`, only name strings, so a call resolves by name through the session env, not through a baked-in id. A carried value of an imported type then indexes `self.structs` correctly, and an imported struct whose field is another imported struct, or a spliced word whose signature names an imported struct, both resolve to the right session id. This is the append-with-remap the flat `StructId = index` invariant forces, the remap 5a's single shared pre-pass avoided and the REPL cannot, and the reason phase 1 is `hard`.
- **R10** *(Q3)* Transitive re-export stays closed: only module 0's export list crosses under `q`. A third file imported *by* the imported file contributes no session-visible name (5a supports no re-export). Stated, with a golden, not discovered by a failing test.

**Selective import at the REPL (D2)**

- **R11** `import: q | a b | "path.sth" ;` is additive at full native parity: `q` binds *and* `a b` are additionally spliced unqualified (a selectively-imported type brings its generated words unqualified too, one unit, 5a R15c). Each listed name must be exported by module 0, else the 5a visibility error. The exported-and-no-intra-closure-collision checks reuse `check::check_selective_imports` on the assembled module unchanged.
- **R12** *(located)* A selectively-exposed unqualified name colliding with an existing **session** name (a locally-defined word, or a prior selective import's unqualified name) is a located error at the second, naming both sources. This extends 5a R21's dumb collision rule to session scope: no precedence, no shadowing, no use-site disambiguation.

**Qualifier rebinding (Q2)**

- **R13** Re-running `import: q "..." ;` when `q` is already bound (same path or a different one) is treated identically to any redefinition: rebind `q`, mint a fresh import epoch (R6), and replace the `q::*` env and registry splices **and** `q`'s retained private-name set (R8). "Replace" means the old `q::`-prefixed `self.env` entries are removed by an exact-prefix scan on the literal key string (`key.starts_with("q::")`) before the new closure's splice is inserted -- the one place this design treats an env key as a structured string rather than an opaque lookup key, needed because `q` itself is never a standalone env key, only ever a prefix on a compound one. There is **no** "you changed the target" error. A call already compiled against `q::old` stays frozen and keeps working under `RTLD_GLOBAL`; a *new* reference to `q::old` (a plain word, a bare type name, or one of a type's accessor spellings, e.g. `q::old>field`) after a rebind is judged against the *new* closure's freshly rebuilt splice and retained private-name set only: `not exported` if the new file declares that exact spelling privately, `unknown word` if it does not declare it at all, never a stale hit on the old file's export status.

**`main` in a library (D4)**

- **R14** *(located)* If any word in the imported closure is named `main`, `eval_import` rejects at import time, naming the file and the word, before any codegen. This turns recon #4's latent native collision (`check_main_effect`, `src/check.rs:2186`, finds the first `main` with no uniqueness check; `mangle`, `src/resolve.rs`, exempts `main`) into a diagnostic on the path this slice builds. The native path's own exposure stays unfixed and is recorded on ROADMAP (out of scope, per D4).

**Session integrity and the not-exported case at the REPL**

- **R15** A qualified reference `q::x` that fails ordinary session lookup is checked against `q`'s retained private-name set (R8) before falling through to `unknown word`: if `x` names a real but non-exported word or type of the module `q` is bound to, the REPL raises 5a's exact `not exported` wording (`src/resolve.rs`'s `not_exported_error`, reused verbatim); only a name absent from both the spliced session state and the private set is `unknown word`. This reproduces 5a R16's distinction at the REPL rather than dropping it -- the cost is one small per-qualifier name set populated from data `assemble_module` already computes, not session-wide visibility marking.
- **R16** On any failure in `eval_import` (parse, missing file, cycle, `main`-in-library, selective collision, check error) the session (env, stack, registries, generations, `libs`) is left **untouched**, matching `eval_line`'s existing commit-only-on-success contract (`src/repl.rs`). This is the full end-state contract; `main`-in-library (R14) and selective-collision (R12) are failure modes that only exist once phases 2 and 3 add their checks, so phase 1's own goldens exercise only the triggers already live then (parse, missing file, cycle, check error), and phase 1 fixtures import no `main`-bearing library (that guard is deferred to phase 2, not yet built).

**Dogfood and docs (Q5)**

- **R17** A REPL-session dogfood test (piped stdin, in `tests/`) imports a real library file and: (a) calls a qualified word `q::w` and a qualified accessor `q::T>field`, running to a value; (b) redefines/edits the library and re-imports, observing a frozen existing caller alongside the fresh resolution (R6/R13); (c) exercises the `main`-in-a-library rejection (R14); (d) exercises a selective import (R11), including calling the exposed name unqualified.
- **R18** ROADMAP slice-5b marked implemented, with the native `main`-collision (recon #4) recorded for a later slice. DESIGN.md's "Modules and encapsulation" gains a REPL-import paragraph (cwd frame of reference per R3, import-as-redefinition via a fresh import epoch per R6, transitive re-export closed per R10, `main`-in-library rejection per R14, no dedup / no cap per D1/R7); the "REPL late binding for redefinition" entry notes that import reload rides the same frozen-generation rule rather than late-binding.

## Load-bearing invariants preserved

QBE backend, `Ptr[T]` opaque, no LLVM / native / JIT / comptime. One `Module` downstream of `assemble_module`. Linear spine untouched (`dup` / `drop`, move-by-default, use-exactly-once). `core` stays `no_std`. The frozen-callee-generation rule is unchanged; each import event mints its own epoch (R6), collision-free by construction rather than by sanitizing a compound string. A session that never imports mints today's symbols and output byte-for-byte (R7).

## Phase sequencing

1. **(hard)** Core REPL import path: remove the R23 guard and route `import:` to `eval_import` (`export:` still a located rejection); reuse `discover_closure` / `assemble_module` / `check::check` (elevated to `pub(crate)`); cwd path frame of reference; bulk-lower the whole closure to one `.so`; mint one fresh import epoch and symbol each compiled word by it (accessor words get no symbol, they lower inline off the spliced struct/enum decl); splice module 0's exports under `q::` with correct positional-id remap across registries and word signatures; retain module 0's private names, including a private type's generated accessor spellings, for the not-exported diagnostic; qualified word and accessor resolution; transitive re-export stays closed; malformed-import located error via the shared parser; commit-only-on-success (fixtures avoid `main`-bearing libraries, since R14's guard lands in phase 2). R1 to R10, R15, R16.
2. **(standard)** Redefinition and reload observability: prove a re-import mints a fresh epoch and recompiles every word under it while a frozen caller keeps the old (R6), qualifier rebinding semantics (R13), and the `main`-in-a-library located rejection (R14).
3. **(standard)** Selective import at the REPL: additive `| name... |` at native parity, must-be-exported, session-scope collision rule. R11, R12.
4. **(standard)** Dogfood plus docs. R17, R18.

The core path lands first: id remap and import-epoch symbol minting are prerequisites for every observable behavior, and reload cannot be demonstrated until that minting exists. Selective import is a pure addition on top of a working qualified path. Each phase leaves the tree green.

## Exit criteria

Goldens in `tests/phase4_repl_imports.rs` (a REPL-session harness feeding piped stdin and asserting stdout, writing any imported library files to a temp dir and setting the process cwd for R3). Diagnostic goldens assert distinguishing wording, never an IL string or bare exit code. Units sit beside their stage functions in `src/repl.rs`.

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | import a two-word library, call `q::w`, runs to a value | `repl_import_word_is_callable_qualified` | golden | 1 |
| 2 | import a type, name `q::T`, construct it, read `q::T>field` | `repl_import_type_accessor_resolves` | golden | 1 |
| 3 | a qualified reference to a real but non-exported name is `not exported`, distinct from a genuinely absent one | `repl_qualified_private_name_is_not_exported` | golden | 1 |
| 4 | carried value of an imported struct-of-struct renders (id remap) | `repl_imported_nested_struct_ids_remap` | golden | 1 |
| 5 | path resolves relative to the process cwd | `repl_import_path_is_relative_to_cwd` | golden | 1 |
| 6 | third file imported by the library stays invisible | `repl_transitive_reexport_stays_closed` | golden | 1 |
| 7 | cycle / missing file at the REPL reuse the located native errors | `repl_import_cycle_and_missing_are_located` | golden | 1 |
| 8 | malformed `import:` at the REPL is R9's located error | `repl_malformed_import_is_located_error` | golden | 1 |
| 9 | a failed import leaves the session untouched | `repl_failed_import_leaves_session_intact` | golden | 1 |
| U1 | spliced imported aggregate's field ids point at session indices | `imported_aggregate_ids_remap_to_session_space` | unit (repl) | 1 |
| U2 | a spliced word's import-epoch symbol is distinct from an ordinary word's and from a prior import's | `import_batch_symbols_are_session_fresh` | unit (repl) | 1 |
| U3 | `discover_closure` / `assemble_module` reachable, yield a checked module for a lib path | `repl_assembles_checked_module_for_library` | unit (repl) | 1 |
| U6 | the retained private-name set distinguishes not-exported from absent | `import_private_names_distinguish_not_exported_from_absent` | unit (repl) | 1 |
| 10 | reload: redefine the library, re-import; frozen caller keeps old, new call sees new | `repl_reimport_freezes_existing_caller` | golden | 2 |
| 11 | qualifier rebind to a different file; frozen `q::old` works, new `q::old` judged against the new file only (not exported or unknown, never a stale hit on the old file) | `repl_qualifier_rebind_frozen_and_rejudged` | golden | 2 |
| 12 | `main` in an imported file is a located rejection naming file and word; session untouched | `repl_import_of_library_declaring_main_is_rejected` | golden | 2 |
| U4 | the `main`-in-closure scan rejects an imported `main` | `imported_main_is_rejected_by_scan` | unit (repl) | 2 |
| 13 | selective import exposes the name unqualified; the qualifier still binds | `repl_selective_import_exposes_unqualified` | golden | 3 |
| 14 | selective import of a private name is a visibility error | `repl_selective_import_of_private_is_error` | golden | 3 |
| 15 | a selective name colliding with a session-local word errors at the second, naming both | `repl_selective_import_collides_with_local` | golden | 3 |
| U5 | the session-scope selective-collision check rejects | `session_selective_collision_is_rejected` | unit (repl) | 3 |
| 16 | dogfood session builds and runs all of R17 (a to d) | `repl_modules_dogfood_session_runs` | golden | 4 |

Load-bearing units (mutation-test the guards): U1, U2, U4, U5, U6; U3 is deliberately excluded, it is a reachability/plumbing smoke test with no guarded negative invariant a mutant could defeat, not a guard. Each negative golden asserts the message text and the named identifiers, never an op name or exit code; #10 and #11 assert the frozen caller's *old* output alongside the fresh resolution's *new* output in the same session.

## Out of scope

Everything 5a deferred stays deferred: a package / registry layer, re-exports, aliasing an import to a different local qualifier, wholesale unqualified import, cross-file generic types (Phase 6); any new disposal / export enforcement, and the destructure-bypasses-`drop` gap (slice 8). Plus, specific to this slice: a REPL-side override of an imported type's `drop` (D3); fixing the `main` collision on the *native* path (recon #4, recorded on ROADMAP, not a drive-by here); content-hash dedup or a size cap on re-import registry growth (D1). No new `Instr` / `Terminator`, no `qbe.rs` change.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "title": "Core REPL import path",
      "focus": "Route `import:` at the REPL to a new `eval_import` (removing 5a's R23 rejection, keeping `export:` a located rejection); reuse `discover_closure`/`assemble_module`/`check::check` (elevated to pub(crate)); resolve the top-level path relative to the process cwd; bulk-lower the whole closure into one `.so`; mint one fresh import epoch and symbol each compiled word by it (accessor words get no symbol); splice module 0's exports under their qualified spelling with correct positional type-id remap across registries and word signatures; retain module 0's private names, including a private type's accessor spellings, for the not-exported diagnostic; resolve qualified words and accessors; keep transitive re-export closed; surface malformed imports via the shared R9 parser; commit only on success (fixtures avoid main-bearing libraries). R1-R10, R15, R16.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "title": "Reload, rebinding, and the main-in-library rejection",
      "focus": "Demonstrate that a re-import mints a fresh epoch and recompiles every word under it while an already-compiled caller stays frozen under RTLD_GLOBAL (R6), define qualifier-rebinding semantics including the post-rebind `q::old` rule (R13), and reject an imported file that declares `main` with a located error naming file and word, leaving the session untouched (R14).",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "title": "Selective import at the REPL",
      "focus": "Ship the additive `import: q | a b | \"path.sth\" ;` form at native parity: bind the qualifier and also splice the listed names unqualified, require each to be exported, and reject a name colliding with an existing session name at the second occurrence naming both sources. R11, R12.",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 4,
      "title": "Dogfood and docs",
      "focus": "Add a piped-stdin REPL-session dogfood test that imports a real library, calls a qualified word and accessor, re-imports after an edit to observe a frozen caller beside a fresh resolution, exercises the main-in-library rejection and a selective import; mark ROADMAP slice-5b implemented (recording the native main-collision for later) and extend DESIGN.md's modules and REPL-late-binding sections. R17, R18.",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
