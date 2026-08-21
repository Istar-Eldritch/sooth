# P8.S2 spec — single-mode imports, the intrinsics module, wildcard, and re-export

**Slice:** P8.S2 · **Design authority:** `docs/roadmap/P8-packages-modules.md`
(sections "Modules re-export", "The intrinsics are a module too", and the P8.S2
plan block) · **Brief:** `docs/roadmap/P8/slice2-brief.md` (source- and
probe-verified, `main` at `ccfbd89`) · **Builds on:** P8.S1a (manifest grammar,
module names, `export:` visibility gating, `resolve::NameTables`), P8.S1b (the
`--manifest` flag and `ResolutionConfig`).
**Siblings (out of scope):** S1a/S1b (done); S3 (API description, semver,
`publish --check`).

---

## What this slice does and why

Today every program silently inherits `lib/core.sth` (`if`, `unless`, the six
surface comparisons) through a compiler-baked prelude, and the ~40
`BUILTIN_WORDS` are dispatched ahead of any environment lookup with no import
gating them. This slice makes **no word resolve without an `import:`**, the
intrinsics included. It:

1. Gives the bare `*` wildcard import a real visibility effect (it parses
   already, and a real target is a hard build error today).
2. Adds **re-export** through `export:` — a structurally new resolution pass, not
   a permissions tweak (brief recon finding 6): a per-module exported-name →
   origin-module table, resolved to a fixpoint over hub-of-hubs chains, with a
   located cycle rejection, plus the existence check `export:` skips entirely
   today.
3. Deletes the compiler-baked prelude (`parser::prelude_words`, both injection
   sites, and the `is_prelude_word_name` mangling exemption), drops the six
   surface comparisons from the *second* un-mangling carve-out
   (`is_operator_dispatch_name`) so they resolve as ordinary `core` words (R3a),
   and gates `BUILTIN_WORDS` visibility behind an `intrinsics` import.
4. Splits `lib/core.sth` into a `core` package with a hub, and migrates every
   corpus file and every migrating test source to explicit imports.

Deleting the prelude removes one capability that has no current user (brief
recon finding 4): a **non-inline** polymorphic word could call the prelude's
poly comparison and cannot call an imported one. This slice takes brief decision
**(b)**: accept the narrowing with a located diagnostic and a rejection test,
rather than bundling the P7 generic-calls-generic fix into a packaging slice.

The brief's Decisions section is settled and is not reopened here: prelude
deletion mechanics, the wildcard grammar, the choice of narrowing option (b),
and the intrinsics gating design.

---

## Rulings

### R1 — Wildcard visibility desugars to an all-exports selective import

`driver::assemble_module` currently rejects a real-target wildcard outright
(`wildcard_import_is_error`, `src/driver.rs:245`, raised at the `None =>` arm of
the qualifier match, `src/driver.rs:317`); only the reserved `intrinsics`
wildcard (`target.is_none()`) is let through, and it binds nothing. A wildcard
import of a resolved target module `t` now binds **every name on `t`'s
`export:` list** into the importing module's *selective* map, pointing at `t` —
i.e. it is exactly a selective import of all of `t`'s exports. This reuses the
existing selective resolution branch in `resolve::NameTables::rewrite`
(`src/resolve.rs`, the `selective.get(...)` branches near the end of `rewrite`)
and the existing `check::check_selective_imports` validation, adding no new
resolution path for the wildcard itself.

**Not independent of R4 for a re-exporting hub.** Phase 1 alone fully handles a
wildcard of a module that **declares** its own exports directly: `rewrite`'s
selective branch matches `self.words[target].contains(name)`. A wildcard of a
*re-exporting hub* — `import: core *`, the headline ergonomic of this whole
slice — binds names the hub does **not** declare, so the selective branch
returns `Ok(None)` and the bare name is `unknown word` until R4 (phase 2)
threads `exported_origin` into that same branch (`src/resolve.rs:372`). So the
hub case is a phase-2 capability, not a phase-1 one; phase 1's wildcard test
targets a module that declares its exports, and the wildcard-of-a-re-export
test lives in phase 2, where it starts passing.

- A wildcard binds **no qualifier** (`Import::qualifier()` returns `None`,
  `src/ast.rs:213`), so there is no qualified spelling for its names; they are
  reachable bare only, per the design doc. A synthesized wildcard
  `SelectiveName` therefore carries **no qualifier string** and its `span` is the
  importer's `import: ... * ;` site (**not** the exporting file's `export:` site,
  whose `Span` from `exports_by_module[target]` would point a collision
  diagnostic into the wrong file). Because `check_selective_imports`'s
  collision/not-exported messages interpolate a qualifier, they need a
  wildcard-specific wording variant rather than a fabricated qualifier that would
  misdescribe the import shape.
- A wildcard-bound name colliding with a **local declaration** is a **hard
  error**, inheriting `check_selective_imports`'s existing
  collides-with-local rule, not a silent shadow — consistent with every other
  selective-import collision already in the codebase; no special wildcard
  shadowing case is invented.
- Wildcard and selective (`| ... |`) are mutually exclusive on one line, already
  enforced by the grammar (`ImportBinding` is one variant or the other,
  `src/ast.rs:190`). No change.
- `assemble_module`'s duplicate-qualifier scan (`src/driver.rs:319`) must not
  fire for a wildcard (it binds no qualifier); today the wildcard reaches that
  loop only via the reserved-`intrinsics` `continue`. The desugaring runs in the
  same per-file loop that builds `import_map`/`selective_map`
  (`src/driver.rs:305-350`), on the branch where a wildcard has a resolved
  `target`.
- Wildcard **re-export is not provided** (design doc): a hub lists what it
  promises. `export:` stays a name list; there is no `export: * ;`.

### R2 — The `intrinsics` module gates `BUILTIN_WORDS` visibility

`BUILTIN_WORDS` (`src/check/declarations.rs:63-110`) and its predicate
`is_builtin_word_name` (`src/check/declarations.rs:118`) do not move: the table
is unchanged and `has_self_tail_call`/`terms_tail_call_self`
(`src/check/drop_graph.rs:107,378`) keep reading it unchanged. Only *visibility*
is gated. Each module carries an **intrinsic-visibility** value derived at
assembly:

**Gate set (not the raw `is_builtin_word_name`).** The gate keys on
`BUILTIN_WORDS` **minus `{eq, lt, gt, lte, gte, ne}`**. Those six surface
comparisons are `lib/` words — they left `BUILTIN_TABLE` in Slice 10c
(`builtin_table`, `src/check/builtins.rs`) and `is_builtin_word_name` returns
`true` for them only so `has_self_tail_call` does not misread a trailing `lt` —
and after R3a they resolve through the ordinary import/mangle path as `core`
words. Keying the gate on the raw set would fire the R6(a)
`import: intrinsics *` remedy for an unimported `lt`, whose real home is `core`,
not `intrinsics`. **`.` stays *in* the gate set**: it is a genuine
`BUILTIN_TABLE` intrinsic (14 `Print` rows, `src/check/builtins.rs`;
`builtin_table_has_a_row_per_printable_type_for_print`), dispatched by
`check_operator`, and does not move to `core`, so a bare `.` with no `intrinsics`
import is correctly the R6(a) error. (Both r1 reviews listed `.` in the
exclusion set; that is wrong, verified against `BUILTIN_TABLE` — the complete
exclusion set is the six comparisons only.) The gate runs **after** the
specialized dispatch/env path (`check_operator`, `scoped_operator_overloads`,
the env lookup), never before, so a correctly-imported-and-mangled `core` word —
now `lt__mN`, which `is_builtin_word_name` no longer matches — is never
intercepted and misdirected.

- `import: intrinsics * ;` → all builtin names visible in that module.
- `import: intrinsics | dup add ... | ;` → only the listed subset visible.
- no `intrinsics` import → no builtin visible in that module.

A bare call to a builtin name in a module that has not imported it (or imported
a selective subset not containing it) is a **located error** naming the word and
the missing import (`R6`), raised at the checker's builtin-dispatch site rather
than falling through to `unknown word`. This gate **cannot fire on the
REPL/`Ctx::Line` path** (`ctx.modules()` is `None`, `src/check/engine.rs`),
exactly as the existing `drop`-visibility scoped gate never fires there
([[project_repl_bypasses_module_checks]]): the "no builtin without an
`intrinsics` import" invariant holds on the file/driver path and is explicitly
exempted at the REPL prompt, consistent with the REPL's existing bypass of
module-level checks (not a new hole). The `intrinsics` import is recognised
exactly as it is for closure discovery: `ImportAnchor::Dependency` with
`segments == ["intrinsics"]` (`src/packages.rs:631`), which adds no closure edge
(`resolve_import` returns `Ok(None)`); `self::intrinsics` is an ordinary module
name and not the reserved one (`src/driver.rs:1409`, unchanged).

Threading: intrinsic visibility is per module, so it is carried on `ModuleInfo`
(`src/ast.rs`, alongside `imports`/`exports`/`selective`) and read by the
checker where a builtin is dispatched, not recomputed. `>`-prefixed numeric
conversions (`>u8`, `src/check/declarations.rs:119`) are part of the intrinsic
surface and gate with the rest.

### R3 — The prelude and its mangling exemption are deleted, not deprecated

Delete `parser::prelude_words` (`src/parser.rs:514`), its unconditional
injection in `parser::parse` (`src/parser.rs:526`), and the multi-file twin in
`driver::assemble_module` (`words.extend(parser::prelude_words())`, the
"Slice 10c" line after the modules loop). Delete `resolve::is_prelude_word_name`
(`src/resolve.rs:61`) and remove its clause from `resolve::mangle`
(`src/resolve.rs:32`), which then exempts exactly `main` and `drop`.

Deleting `parser::prelude_words`/`is_prelude_word_name` breaks every live
consumer, and R3 must migrate each. The r1 spec cited only
`src/check.rs:3358-3362` as "the REPL's own prelude seeding"; that is **not** the
REPL but a `#[cfg(test)]` bare-line inference helper (`infer_src`, near
`SPY_DEF`, now `src/check.rs:3477`) — R7 already treats the same helper
correctly, and it is *not* a `--manifest` fixture consumer, so its seeding of a
bare line's `bool`/comparison env stays an in-process helper. The actual live
consumers:

- **`src/repl.rs:1120`** — the real REPL seed: `Session::new`'s
  `for word in prelude_words() { session.eval_def(word, ..) }` loop. This loop
  is **deleted**; a session no longer auto-seeds `if`/`lt`, and one that wants
  them writes `import: core *` exactly as a file does, so a bare comparison with
  no import is `unknown word`, matching a compiled build. (The neighbouring
  `bool` print overload seed, `bool_print_word_def`, is not a prelude word and
  stays.)
- **`src/repl.rs:2270`** — a live `is_prelude_word_name(&w.name)` in the
  import-rename filter (`body_rename`), excluding prelude words from
  epoch-renaming because they were both session-seeded **and** closure-injected,
  a dual existence a rename would strand. Deleting the prelude ends that dual
  existence, so this clause is **removed** (not merely made to compile): an
  imported closure's `core` words are ordinary module-0 words and epoch-rename
  like any other import. The `main`/`drop`/`.` exclusions beside it stay for
  their own distinct reasons (never-mangled entry/destructor; the separately
  seeded `bool` print overload).
- **`src/check/word_entry.rs:415`** — a test pulling `eq` out of
  `prelude_words()` as its witness; rewrite it to declare its own witness word
  rather than reach for the deleted prelude.
- **`src/parser.rs:4066`** — a test asserting a parsed word count against
  `prelude_words().len()`; with no injection that addend is gone and the
  assertion drops it.
- **`src/resolve.rs:1001-1012`** — the `#[cfg(test)]` test
  `single_module_closure_is_left_unchanged`, whose whole premise (its `:1006`
  comment) is that `parse` appends unmangled `lib/core.sth` words and which
  filters them out of its name assertion with `is_prelude_word_name` (`:1012`).
  With the prelude deleted `parse` appends nothing, so the filter and its
  premise both go: drop the `.filter(|n| !is_prelude_word_name(n))` (the closure
  is now just the file's own `p`/`main`) and the stale comment.
- (`tests/phase4_slice10c_primitives.rs:62,249` also read `prelude_words()`;
  they retire with the prelude.)

A corpus file that still resolves `if` through the deleted prelude is a build
failure whose fix is the import, not a restored exemption.

### R3a — `is_operator_dispatch_name` drops the six surface comparisons

Deleting `is_prelude_word_name` is not sufficient on its own: a **second**,
independent un-mangling carve-out strands the same six words.
`resolve::is_operator_dispatch_name` (`src/resolve.rs:85-116`) lists
`eq lt gt lte gte ne` alongside the real operators, and `NameTables::rewrite`
guards **both** its own-module and selective rewrite branches on
`!is_operator_dispatch_name(core)` (`src/resolve.rs:342`, `:371`), so a bare
comparison call is left *unrewritten* regardless of imports. Today that is
harmless only because `is_prelude_word_name` leaves the comparison *declaration*
unmangled too, so bare call and bare decl coincide. After R3 the decl mangles
(`lt__mN`) while `is_operator_dispatch_name` still leaves the call bare — they no
longer match, and the call is `unknown word` with no resolution path.

**Verified safe to drop the six (they carry no operator dispatch).** The six
surface comparisons are **not** `BUILTIN_TABLE` keys — their rows moved wholesale
to the `u`-prefixed primitives in Slice 10c (`builtin_table`,
`src/check/builtins.rs`; pinned by
`builtin_table_comparisons_have_a_row_per_numeric_type`, which asserts
`!table.contains_key("lt")` for each). `check_operator`'s own `is_operator` list
(`src/check/operators.rs:79-95`) does not list them either, so it returns
`NotOperator` for a bare `lt`. And `scoped_operator_overloads`
(`src/check/word_families.rs:1065`) early-returns `None` for any name not in
`BUILTIN_TABLE` (`:1075`). So **no** operator-overload dispatch path touches the
six; listing them in `is_operator_dispatch_name` buys nothing but the
bare-call/bare-decl coincidence above. Their **only** consumers are `rewrite`'s
two branches (`src/resolve.rs:342`, `:371`); no `check/*` path reads the
function. Removing them therefore regresses no real operator-overload dispatch.

**Ruling.** `is_operator_dispatch_name` drops `eq lt gt lte gte ne`. It keeps
the genuinely operator-overloaded names — the arithmetic/bitwise set
(`add sub mul div mod and or xor not shl shr`), the `u`-prefixed comparison
primitives (`ueq ult ugt ulte ugte une`), and `max max-total .` — all of which
**are** `BUILTIN_TABLE` keys reached through
`scoped_operator_overloads`/`check_operator` and would regress if dropped. The
six comparisons become ordinary words: `resolve::mangle` mangles the decl to
`lt__mN`, and `rewrite`'s own-module and selective branches now mangle a bare
call to match, so a consumer reaches them by import through the normal machinery
like any other name — the compiler special-cases no specific library word.

### R4 — Re-export resolution: an exported-name → origin-module table with a fixpoint

`export:` accepts a name the file **imported** as readily as one it declared
(design doc). Today this is structurally unsupported: `NameTables::rewrite`
resolves a qualified or selective word only against `words[target]`
(`src/resolve.rs`, the `self.words[target].contains(rest)` branch and the
selective branch), which `NameTables::build` fills only from decls whose
`module == target`; a re-exported name's origin module is never consulted, so
`hub::lw` is `unknown word` even after `check_selective_imports` passed it
against `hub`'s export list.

**Table.** Add a per-module map `exported_origin: Vec<HashMap<String, u32>>`:
for module `m`, each name on `m`'s `export:` list maps to the module id that
actually declares it. Built in `resolve::resolve_modules` (which already
materialises `import_maps`, `selectives`, and `exports` per module) *before*
body rewriting, by, for each `(m, name)` on an export list:

- if `name` is a local decl of `m` (in `NameTables.words[m]`/`structs`/`enums`),
  origin is `m`;
- else look `name` up in `m`'s selective map (which R1's wildcard desugaring has
  already populated for wildcard imports too); that gives the immediate source
  module;
- else scan the modules `m`'s `import_maps` **values point at** and check each
  one's own local declaration table for `name`. Each qualified import binds
  `qualifier → module id` (`import_maps` is `Vec<HashMap<String, u32>>`, values
  are `u32` module ids), so `import_maps[m].values()` is the set of
  qualified-imported module ids; for each `dep_id` in it, test
  `tables.words[dep_id].contains(name)` (and `structs[dep_id]`/`enums[dep_id]`).
  A hub that imports a dependency *qualified* (`import: dep ;`, no
  wildcard/selective) and then `export: lw ;` reaches `lw` only as `dep::lw`, so
  it is neither a local decl nor a selective entry, but `dep`'s own decl table
  holds it. This is **not** a lookup of `name` *in* `import_maps` — that map is
  keyed by qualifier, never by word name, so `import_maps[m].get(name)` always
  misses — it is an iteration over the qualified-imported modules' decl tables.
  Without this scan R5's existence check would wrongly reject a legitimate
  qualified-only re-export; the compiler must not restrict which import shapes
  are re-exportable, so the origin is the qualified-imported module that
  declares `name`.
  - **Ambiguity (multiple qualified origins).** If **two or more** of the
    qualified-imported modules each declare `name`
    (`import: dep1 ; import: dep2 ;`, both declaring `lw`, then `export: lw ;`),
    the bare export name gives no qualifier to pick between them, and `export:`
    stays a flat name list ("No new AST", below), so no `dep1::lw` export
    spelling exists to disambiguate. This is a **located ambiguity error**
    (`ambiguous_re_export_error`, `R6(c)`) naming the `export:` site and the
    two-or-more colliding origin modules — **not** a silent first-import-wins
    pick. That ruling follows this codebase's design (CLAUDE.md: turn Forth's
    silent failures into sharp compile errors) and R4's own "the compiler must
    not restrict which import shapes are re-exportable" (a first-wins tiebreak
    would silently privilege one import's declaration order). A `name` that is a
    local decl, or resolves through the selective/wildcard map, is unambiguous
    and never reaches this scan, so the error fires only for the
    qualified-only-collision case.

**Fixpoint (per-resolution visited set, keyed on `(module, name)`).** The
immediate source may itself re-export the name (hub-of-hubs). Resolve each export
entry on demand, following `exported_origin[src][name]` until reaching a module
that declares `name` locally, carrying a **per-resolution visited set keyed on
the `(module, name)` pair**. This is the committed strategy, not one of two
options: a naive "repeat passes until no entry changes" fixpoint is rejected
because it **cannot detect a re-export cycle** — on a cycle an entry keeps being
rewritten one hop and never stabilises, so the pass hangs rather than
converging, and it carries no per-pair structure at which to notice the revisit.
The pair-keying (not module-keying) is load-bearing: a hub re-exporting two
different names both routed through one downstream hub is a legitimate diamond,
not a cycle, and a module-keyed visited set would false-positive on it.

**Cycle rejection.** A re-export chain that revisits a `(module, name)` pair
before reaching a local declaration is a **re-export cycle**: a located error
(`re_export_cycle_error`, `R7`) naming the name and the `export:` site, raised
instead of looping or overflowing the fixpoint. This is the one place the
fixpoint can fail to converge and must be guarded explicitly.

**Threading.** `NameTables::rewrite`'s two lookup branches consult
`exported_origin[target]` when `target` does not declare the name locally: a
qualified `hub::lw` (or a selective bare `lw` re-exported through `hub`) whose
name is on `hub`'s export list and resolves through `exported_origin` to origin
`o` rewrites to `mangle(name, o)`, gated by `hub`'s export list (the entry that
made it a re-export). `rewrite` needs `exported_origin` alongside `exports`;
rather than a bare 9th parameter (`rewrite` already takes 8, so a 9th trips
`clippy::too_many_arguments` under `-D warnings`, and it must thread through
`rewrite_terms`, its recursive caller, plus the ~6 in-file test call sites),
bundle the new lookup tables into a small `struct`-of-tables parameter.

**No new AST.** A re-export needs no representation distinct from a plain export:
it is detected purely by "the name is not a local decl of the exporting module,
so its origin is found through the import map." `export:` stays a flat
`Vec<(String, Span)>`.

### R5 — `export:` existence validation lands in this slice

`export:` performs zero existence validation today (`export: nonexistent ;` with
the name declared and imported nowhere builds and runs clean; brief recon
finding 6). Building R4's `exported_origin` table makes an unresolvable export
name observable: a name that is neither a local decl nor an imported name of the
exporting module has no origin. This is now a **located error**
(`export_unknown_name_error`, `R7`), raised where the table is built.
`check_exported_signatures` (`src/check/declarations.rs:243`) keeps its distinct
job (private-type-in-signature) and does not absorb this check.

The same table-construction pass also surfaces re-export **ambiguity**: a bare
`export:` name declared by two or more qualified-imported modules, with no local
decl and no selective/wildcard entry to disambiguate, is a located
`ambiguous_re_export_error` (R6(c)). Existence validation and ambiguity
validation both land in this slice, built off the same `exported_origin`
construction pass (R4).

**Prerequisite grep (implementation gate):** before landing R5, grep the corpus
(`lib/`, `examples/`, committed test fixtures) for any `export:` naming a word
declared/imported nowhere in its file. The brief's probe found none in the tree,
but silent acceptance must be confirmed unused rather than assumed; a real
reliance would be migrated to a correct `export:`, not a retained silent pass.

### R6 — Diagnostic wording (three located messages)

All three are located (line, col) and name the surface spelling, never a mangled
name (`resolve::demangle_word`/`demangle_call`, `src/resolve.rs`).

**(a) Ungated intrinsic (R2).** A bare builtin call in a module that has not
imported it:

```
error: `<word>` is an intrinsic and is not imported in `<caller>` (line L, col C)
  add `import: intrinsics * ;` (or `import: intrinsics | <word> ... | ;`) to this file
```

**(b) Narrowing: non-inline poly word calls an imported poly word (brief
decision (b)).** In a polymorphic body, an imported poly word's call is mangled
to `<name>__m<k>` and reaches `poly_call_term`'s fall-through
(`src/check/poly.rs:1059`, `Err(unknown_word_error(...))`) because a poly callee
is registered in `poly.env`, never in the `env` this path reads. Before that
fall-through, if the demangled call name names a **polymorphic word of another
module** (detected against a threaded set of the program's poly-word names,
keyed by their post-mangle spelling), raise:

```
error: `<caller>` cannot call the polymorphic word `<callee>` (line L, col C)
  a polymorphic word is not yet reachable from another polymorphic word across a module boundary
  inline the caller, make the callee concrete, or call the callee from a monomorphic word
```

`<caller>` is the enclosing word's demangled name, `<callee>` the demangled call
name. This replaces the raw `` unknown word `<__m1` `` that leaks the mangled
name today. The same-module non-inline poly-to-poly gap emits an identical
message (its `<callee>` is a same-module poly word); both are the one underlying
generic-calls-generic gap.

**(c) Re-export existence / cycle / ambiguity (R4/R5).**

```
error: `<name>` in `export:` of `<module>` names nothing declared or imported here (line L, col C)
error: `<name>` re-exports itself through a cycle of `export:` chains (line L, col C)
error: `<name>` in `export:` of `<module>` is declared by more than one qualified-imported module (`<origin1>`, `<origin2>`, ...) and cannot be re-exported without disambiguation (line L, col C)
```

### R7 — Shared test-fixture manifest: one manifest for the whole migrating suite

The migrating file-based test sources point at **one shared manifest**, not one
per test-module grouping. Every migrating source depends on the same two things
(`core` and `intrinsics`); per-grouping manifests would be near-identical copies
that drift, and no grouping has a distinct dependency set that would justify its
own. The shared manifest declares `core` (resolved to `lib/`'s `core` package)
and is passed via S1b's `--manifest` flag through a single test helper: switch
`tests/common/mod.rs`'s `build_example` (which today calls `driver::build`,
`tests/common/mod.rs:23-37`) to the manifest-passing
`driver::build_with_manifest(&copy, Some(<shared manifest>))`
(`src/driver.rs:542`); the per-call sibling copy still resolves relative to the
manifest. In-process `check_src`/`check_error` tests, which never
resolve `import:`, keep seeding what they need in process — including the
`#[cfg(test)]` bare-line helper `infer_src` (`src/check.rs:3477`), which the r1
spec mis-labelled as the REPL seed but is an in-process helper (R3); these are
not part of the `--manifest` fixture set, which is the file-based, driver-built
sources only.

### R8 — `lib/core.sth` splits into a `core` package with a hub

The Sooth half of `lib/core.sth` (`bool`, `if`/`unless`, the six surface
comparisons) splits into modules of one `core` package; its compiler half
(`branch`, `tag`, the `u`-prefixed comparisons) does not move — those are
`BUILTIN_WORDS` reached through `import: intrinsics` (R2). The `core` hub imports
`intrinsics` and **re-exports** (R4) the curated subset it endorses, so a
consumer writes `import: core ...` for the typed surface and `import: intrinsics
*` only where it wants the raw builtins. The six comparisons the hub re-exports
have a working resolution path only because of R3a: as ordinary mangled `core`
words they go through `rewrite`'s selective branch, which consults R4's
`exported_origin`, with no operator-dispatch carve-out intercepting the lookup
(the gate-specific carve-out the r1 review flagged is gone). `core` is an
ordinary package, not a compiler-reserved name (design doc: reserving it would
re-privilege the library this phase de-privileges).

---

## Implementation

Three phases. The first two are independently reviewable; the third
(prelude deletion + gating + `core` split + corpus/test migration) is one
**atomic** phase, not several — see the note under it. Cited lines are `main` at
`ccfbd89`; verify before editing, several will shift as earlier steps land.

1. **Wildcard visibility (R1).** In `driver::assemble_module`'s per-file
   import loop, on a `Wildcard` binding with a resolved `target`, insert every
   name of `exports_by_module[target]` into that module's `selective_map` and
   `selective_entries` (so `check_selective_imports` validates them), with the
   synthesized `SelectiveName` carrying no qualifier and the importer's
   `import: ... * ;` span (R1). Keep the reserved-`intrinsics` `continue` and the
   real-target rejection only for the *no-target* wildcard. Delete/retarget
   `wildcard_import_is_error` and its test `driver_wildcard_import_is_error`,
   which now must assert the wildcard *binds*. This phase handles a wildcard of a
   module that **declares** its own exports; the re-exporting-hub case
   (`import: core *`) only fully works once phase 2's `exported_origin` lands
   (R1, R4).

2. **Re-export (R4/R5).** In `resolve::resolve_modules` (`src/resolve.rs`,
   after `import_maps`/`selectives`/`exports` are built), build
   `exported_origin: Vec<HashMap<String, u32>>` with the fixpoint and the
   cycle/existence guards, then pass it into `rewrite` and consult it in both
   the qualified `self.words[target].contains(rest)` branch and the selective
   branch. New errors `re_export_cycle_error` and `export_unknown_name_error`
   beside `not_exported_error` (`src/resolve.rs:390`).

3. **Prelude deletion + intrinsics gating + `core` split + migration
   (R2/R3/R3a/R6/R7/R8) — one atomic phase.** Delete `parser::prelude_words` and
   its two injection sites; shrink `resolve::mangle` to `main`/`drop`, delete
   `is_prelude_word_name`, and drop `eq lt gt lte gte ne` from
   `is_operator_dispatch_name` (R3a). Migrate every live prelude consumer
   (`repl.rs:1120` seed loop deleted, `repl.rs:2270` filter clause removed,
   `word_entry.rs:415` and `parser.rs:4066` tests rewritten — R3). Add per-module
   intrinsic visibility to `ModuleInfo`, populate it in `assemble_module`'s
   import loop (from `intrinsics` imports, wildcard = all / selective = subset),
   and gate the builtin-dispatch site in the checker with the R6(a) error, keyed
   on the corrected gate set (`BUILTIN_WORDS` minus the six comparisons, R2) and
   placed **after** the specialized dispatch/env path. Add the narrowing
   diagnostic R6(b): thread the poly-word-name set into the poly body checker and
   branch before `poly_call_term`'s `unknown_word_error` (`src/check/poly.rs:1059`).
   Split `lib/core.sth` into the `core` package with a re-exporting hub, and add
   explicit `import:` lines to every `.sth` file (examples, goldens, non-`core`
   `lib/` files) and to the migrating file-based test sources, which resolve
   against the shared fixture manifest via `--manifest` (R7, `build_example`).
   This step also carries S1a's `lib/`-as-layered-packages dogfood: a manifest
   over `lib/` rejects `arrays.sth`'s quoted-path import until its imports are
   module names.

   **Why atomic (not two phases).** Deleting the prelude removes `if`/the six
   comparisons from every closure, and activating the intrinsic gate removes
   ambient `BUILTIN_WORDS`, so *every* existing golden and example fails to build
   until the corpus gains its imports. Conversely the imports cannot be added
   first: while the prelude still injects `if`, a file's own `import: core *`
   would double-bind the name. No ordering leaves the tree green except doing
   deletion, gating, split, and migration together, so a fourth
   "independently reviewable" boundary between them would be fictional — which
   CLAUDE.md's "goldens pass at phase exit" rule forbids. Land it as one phase
   whose single exit is a green corpus.

---

## Tests

- **Wildcard binds (R1, phase 1):** a wildcard of a module that **declares** an
  exported name makes it reachable bare; replaces `driver_wildcard_import_is_error`.
  A non-exported name of the target stays `unknown word` (wildcard binds exports
  only). A wildcard-bound name colliding with a local declaration is a hard
  error (R1).
- **Wildcard of a re-export (R1×R4, phase 2):** `import: core *` where `core`
  **re-exports** (does not declare) a name binds it and a consumer calls it bare.
  Placed in phase 2, not phase 1, because it only starts passing once
  `exported_origin` is threaded into `rewrite`'s selective branch (the headline
  `import: core *`-over-a-hub path).
- **Re-export (R4):** a hub re-exports an imported word and a consumer calls it
  (qualified through the hub and, where applicable, bare). A hub-of-hubs chain
  (two re-export hops) resolves to the origin. A hub that imports a dependency
   *qualified* and re-exports a name reachable only as `dep::name` resolves too
   (R4's qualified-import decl scan), not a spurious `export_unknown_name_error`.
- **Re-export ambiguity (R4/R6c):** two qualified-imported dependencies both
  declaring `lw` (`import: dep1 ; import: dep2 ;`), with the hub doing
  `export: lw ;` (no local decl, no selective/wildcard entry to disambiguate), is
  a located `ambiguous_re_export_error` naming the `export:` site and both origin
  modules — not a silent first-wins pick.
- **Re-export cycle (R4/R6c):** two modules re-exporting each other's name is a
  located `re_export_cycle_error`, asserted to terminate (not hang/overflow).
- **Export existence (R5/R6c):** `export: nonexistent ;` (declared and imported
  nowhere) is a located `export_unknown_name_error`.
- **Intrinsic gating (R2/R6a):** a bare `add` with no `intrinsics` import is the
  located R6(a) error; `import: intrinsics * ;` and the selective form both make
  it resolve; a selective import missing `add` still rejects a bare `add`.
- **Prelude deleted (R3):** a file using `if`/`lt` with no import of the `core`
  module is a build failure; the same file with the import builds. Golden
  `gcd.sth`/`factorial.sth` build with explicit imports.
- **Narrowing diagnostic (R6b):** a non-inline poly word (`: mylt ( 'T: Copy Ord
  'T -- bool ) < ;` over an imported `<`) is the located R6(b) error naming
  caller, callee, and reason — not `` unknown word `<__m1` ``. Mutation-guard it:
  deleting the new branch must fall back to the raw unknown-word and fail the
  test (per `[[workflow_mutation_test_the_guards]]`).
- **Migration (R7/R8):** the corpus builds and every existing golden passes
  against the shared fixture manifest.

---

## Exit criteria

- No word resolves without an `import:` on the file/driver path, the intrinsics
  included (the REPL/`Ctx::Line` prompt is exempt, since the gate cannot fire
  where `ctx.modules()` is `None`, R2 — consistent with the REPL's existing
  module-check bypass); `is_prelude_word_name` and `parser::prelude_words` are
  deleted, `resolve::mangle` exempts only `main`/`drop`, and
  `is_operator_dispatch_name` no longer lists the six surface comparisons (R3a).
- A hub module re-exports an imported word and a consumer uses it; a re-export
  cycle, an unresolvable `export:` name, and a bare `export:` name declared by
  two or more qualified-imported modules are each a located error, not a hang or
  a silent first-wins pick.
- A wildcard import binds every exported name of its target; the reserved
  `intrinsics` wildcard makes the builtins visible.
- A non-inline poly word calling an imported (or same-module) poly word is a
  located error naming the caller, the callee, and the reason.
- The corpus builds, every golden passes, and `cargo fmt --check && cargo clippy
  -- -D warnings && cargo test` is green.

---

## Out of scope

- The generic-calls-generic P7 fix (brief decision (b): narrowed with a
  diagnostic, not fixed here).
- Manifest grammar, package attribution, module naming/visibility, cross-package
  resolution, the layer check (S1a), the `--manifest` flag and fallback chain
  (S1b) — depended on, not modified.
- Wildcard re-export (`export: * ;`), manifest path tables, and a package-wide
  unqualified-exports escape hatch (design doc: declined / deferred).
- Semver, the serialisable API description, `sooth publish --check` (S3).

---

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "driver.rs assemble_module: wildcard visibility (R1) desugars a real-target Wildcard import into an all-exports selective binding (insert every exports_by_module[target] name into selective_map/selective_entries), keep the reserved-intrinsics continue and no-target rejection only; delete/retarget wildcard_import_is_error and rewrite driver_wildcard_import_is_error to assert the wildcard binds; unit tests",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "resolve.rs re-export (R4/R5): build per-module exported_origin table in resolve_modules with a transitive fixpoint over hub-of-hubs chains (origin found by local decl, then the selective/wildcard map, then a scan of the qualified-imported modules' own decl tables via import_maps' target ids -- tables.words[dep_id].contains(name), NOT a name lookup in import_maps which is keyed by qualifier), thread it into NameTables::rewrite's qualified and selective branches, add located re_export_cycle_error, export_unknown_name_error (existence check, gated on the R5 corpus grep), and ambiguous_re_export_error (a bare export name declared by two or more qualified-imported modules, no local/selective/wildcard entry to disambiguate); unit tests for hub, hub-of-hubs, cycle termination, unresolvable export, and two-qualified-deps ambiguity",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "ATOMIC (prelude deletion + intrinsics gating + core split + corpus/test migration, R2/R3/R3a/R6/R7/R8 — one phase because no ordering keeps goldens green otherwise): delete parser::prelude_words and its two injection sites, shrink resolve::mangle to main/drop, delete is_prelude_word_name, and drop the six surface comparisons from is_operator_dispatch_name (R3a); migrate every live prelude consumer (repl.rs:1120 seed loop deleted, repl.rs:2270 filter clause removed, word_entry.rs:415 and parser.rs:4066 tests rewritten); add per-module intrinsic visibility to ModuleInfo populated in assemble_module's import loop, gate the checker's builtin-dispatch site with the R6a error keyed on BUILTIN_WORDS minus {eq,lt,gt,lte,gte,ne} and placed after the specialized dispatch/env path, add the R6b narrowing diagnostic before poly_call_term's unknown_word_error at poly.rs:1059 (threaded poly-word-name set, mutation-guarded); split lib/core.sth into a core package with a re-exporting hub and migrate every .sth file and file-based test source to explicit imports against one shared fixture manifest via build_example's build_with_manifest, carry S1a's lib/-as-layered-packages dogfood; corpus builds and every golden passes",
      "effort": "XL",
      "difficulty": "hard"
    }
  ]
}
```
