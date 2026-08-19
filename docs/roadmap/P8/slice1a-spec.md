# P8.S1a spec — packages, manifests, path-derived module names, and the layer check

**Slice:** P8.S1a  
**Design authority:** `docs/roadmap/P8-packages-modules.md` (overrides anything below).  
**Brief:** `docs/roadmap/P8/slice1-brief.md`.  
**Siblings (out of scope):** S1b (`--manifest` flag and fallback chain), S2 (single-mode
imports, intrinsics gating, wildcard, re-export, prelude deletion).

---

## Scope

This slice covers exactly:

1. The manifest file format and its parser (`sooth.pkg`, Sooth-lexed).
2. Package-boundary attribution over the discovered `Closure` (nearest ancestor manifest
   per file).
3. Path-derived module naming (`text/ascii.sth` → `text::ascii`) and the `module:` public
   list.
4. Cross-package `pkg::module` import resolution against `depends:`/`module:` tables, and
   intra-package module-name resolution (replacing today's quoted-path import for
   everything inside a package).
5. The three validation checks run after `discover_closure`, before `assemble_module`:
   a. naming a module the package does not make public;
   b. naming a package no `depends:` entry lists;
   c. a `depends:` entry whose target package is in a higher layer.

**Not this slice** (do not add stubs or pre-wiring):

- The `--manifest` CLI flag and fallback chain (S1b).
- Single-mode imports, intrinsics gating, wildcard, re-export, the prelude deletion (S2).
- Semver enforcement, API descriptions (P8.S3).
- Git-based dependency paths.

---

## Ruling on open questions

### OQ1: Manifest grammar exactly

**Field syntax.** A `sooth.pkg` file is tokenised by the existing Sooth lexer. Its grammar
is a flat sequence of declarations; each declaration starts with one of four keywords
(`package:`, `layer:`, `depends:`, `module:`), followed by its value tokens, and terminated
by `;`. No two declarations of the same keyword may appear except `module:`, which
accumulates segments exactly as `export:` does.

```
package: core ;
layer: core ;
depends: core path "../core" ;
module: bool cmp text ;
```

**`package:` value.** A single identifier token. It is the package's canonical name.
Distinct from the qualifier used in `depends:` on the consumer side: the consumer names the
package by its declared `package:` name, not an alias.

**`layer:` value.** A single identifier token from a fixed four-value set: `core`, `fixed`,
`alloc`, `hosted`. The compiler enforces this at manifest-parse time. The ordering is strict
and total: `core < fixed < alloc < hosted`. An unknown layer name is a parse-time error at
the `layer:` line.

Ruling: a fixed enum is the right call. The brief's "open list ordered by some declared
rule" alternative requires a package-graph-wide ordering query before any layer check can
run. A fixed enum is O(1) per check, has a stable meaning in diagnostics ("depends on a
`hosted`-layer package from a `core`-layer package"), and matches the design doc's prose
which names the four layers by name throughout with no hedging.

**`depends:` syntax.** Each `depends:` declaration names one dependency:

```
depends: <pkg-name> path <quoted-or-word-path> ;
```

The first token after `depends:` is the dependency's canonical package name (must match
that package's own `package:` field). The keyword `path` follows, then a path value (either
a quoted string or a bare word token, interpreted relative to the declaring manifest's
directory). No aliasing: the consumer names the dependency by the same identifier the
dependency declares as its `package:` name. This is consistent with Phase 4 Slice 5's
decision that import qualifiers are chosen by the consumer, but the package name itself is
declared, not aliased. The `import:` qualifier at the file level is still freely chosen by
the consumer (see OQ3).

**Aliasing ruling:** no package-name aliasing at the `depends:` level. The consumer-side
qualifier that appears in `import: pkg::module q ;` is the freely chosen `q` at the file
level; the `pkg` segment of `pkg::module` must exactly match the package's `package:` name.
This keeps the `depends:` table auditable: `depends: core path "..." ;` and `import:
core::cmp c ;` name the same package consistently, and renaming the package would be a
one-site `depends:` update rather than a hunt across qualifier aliases.

**`module:` syntax.** Module names accumulate in one or more `module:` declarations, each
terminated by `;`. Each token is a module segment (single identifier or dotted segment; see
module naming below). An empty `module:` list (no `module:` line at all) is valid: an
application package that exposes nothing. `module:` applies to the package's public surface
only and carries no discovery or file-layout implication.

**`depends:` version field.** Not in this slice. The path form is the only form, and the
`depends:` grammar is designed for a later additive extension. When git-URL-and-revision
lands (P8.S3 / `docs/dependency-management.md`), `depends: core git "<url>" rev "<sha>" ;`
extends the same keyword without a breaking change to the parser, since the path keyword
distinguishes the form. No version field is emitted, required, or checked in this slice.

### OQ2: Manifest location and subdirectory nesting

**Where the manifest lives.** A manifest (`sooth.pkg`) lives at the root of its package
directory. Every file that is a descendant of that directory (transitively) belongs to the
package, and its module name is derived from its path relative to the manifest's directory.

**Subdirectory nesting.** Fully allowed. `text/ascii.sth` in a package whose manifest is at
`core/sooth.pkg` is the module `text::ascii`. Nesting depth is not bounded. A filename must
be a valid Sooth identifier segment (the existing `word name` character set, including `-`).
A directory name must satisfy the same constraint, since it becomes a module-name segment.

**Manifest locality.** A file's package is always its *nearest* ancestor manifest. Two
manifest files may nest (a monorepo with packages inside packages): the inner manifest wins
for files beneath it.

**F3 ruling (is `text.sth` beside `text/` legal?).** Yes. The hub pattern requires it:
`core::text` is a file (`text.sth`) and `core::text::ascii` is a file inside `text/`. The
name-derivation rule handles both correctly since `text.sth` derives `text` and
`text/ascii.sth` derives `text::ascii`; the two names are distinct and do not conflict.

However, `text/text.sth` would derive a second module named `text`, which is a conflict and
a located error at package-attribution time. The check is: within one package, no two path
derivations may produce the same module name. The error names both files and the conflicting
module name.

### OQ3: Whether the local import qualifier stays freely chosen

**Ruling: the qualifier stays freely chosen by the importer, and the grammar changes to
make the common case terser.** The target now comes first, and the qualifier is optional:

```
import: <target> [<qualifier>] [ | <name>... | ] ;
```

`import: core::cmp c ;` binds `c` as the local qualifier; `import: core::cmp ;` (qualifier
omitted) binds `cmp`, the module's own last segment. Either spelling is legal, and both
produce a bound qualifier — the qualifier is never absent at the semantic level, only
optionally elided in source. The reason for the reorder: putting the target first means the
common case (no renaming wanted) needs no ceremony at all, where the old
qualifier-always-first grammar forced every import to spell out a name even when it would
just echo the module's own. The reason for keeping the qualifier free rather than fixing it
to the last segment unconditionally: Phase 4 Slice 5's existing qualifier freedom is
load-bearing for code style (short qualifiers in dense files, disambiguating qualifiers
when two dependencies each have an `ascii` module), and the default only replaces the
ceremony, it doesn't remove the freedom.

Parsing is unambiguous: after the target, the parser sees either `|` (selective list, no
explicit qualifier), a bare word (the qualifier, optionally followed by `| ... |`), or `;`
(default qualifier, no selective list). The wildcard form benefits most from the default:
`import: intrinsics | * | ;` needs no qualifier at all when every name is already coming in
unqualified via `*`, whereas the old grammar forced one (`import: i | * | intrinsics ;`)
purely to satisfy the parser.

The `pkg` segment of `pkg::module` in the import target is not the qualifier; it resolves
against `depends:` by exact package name match and is distinct from the binding.

### OQ4: Diagnostic wording and located-ness

Both new failure modes produce located errors. "Located" means the error names the span of
the offending `import:` or `depends:` declaration (file, line, col), not just the file.

**Failure mode A: import crosses a package boundary without a matching `depends:` entry.**

The consumer file imports `pkg::module` but the consumer's nearest ancestor manifest has no
`depends:` entry naming `pkg`.

Error format:

```
error: import `<pkg>::<module>` at line L, col C in <importer>:
  package `<consumer-pkg>` has no `depends:` entry for `<pkg>`
  add `depends: <pkg> path "<path>" ;` to <consumer-manifest-path>
```

Located to the `import:` span. Names both packages. Includes the remedy (exact field to
add).

**Failure mode B: a `depends:` entry names a package in a higher layer.**

A package at layer X lists a `depends:` on a package at layer Y where Y > X (e.g., a
`core`-layer package depending on a `hosted`-layer package).

Error format:

```
error: layer violation in <declaring-manifest-path>, line L, col C:
  package `<declaring-pkg>` is layer `<X>` but depends on `<dep-pkg>` which is layer `<Y>`
  a `<X>` package may only depend on packages at the same layer or below
```

Located to the `depends:` span within the declaring manifest. Names both packages and both
layer values. The layer ordering stated in the message is the fixed enum ordering.

**Failure mode C: a cross-package import names a module the target package does not make
public** (not in OQ4, but this slice introduces it and needs a diagnostic).

Error format:

```
error: import `<pkg>::<module>` at line L, col C in <importer>:
  module `<module>` is not in `<pkg>`'s public `module:` list
  add `module: <module> ;` to <pkg-manifest-path> to make it public
```

### OQ5: `depends:` version/revision field

Deferred cleanly: no version or revision field in this slice. The `path` keyword
distinguishes the present form; adding `git`, `rev`, or `version` later is an additive
keyword extension with no grammar overlap. Ruling: do not add a placeholder or FIXME for
this; the grammar is complete for this slice's purposes.

---

## F2 ruling (intra-package reference base, dogfood finding F2)

Module names are package-root-relative inside the declaring package. A file at
`core/text/ascii.sth` (package root `core/`) has module name `text::ascii`. When it
imports a sibling:

```
import: cmp c | lt gt | ;
```

`cmp` resolves against the same package's path-derived module names, without a `core::`
prefix. The rule: **inside a package, bare module names resolve against the package's own
module table; `pkg::module` names resolve against `depends:`.**

Cross-package imports always require the full `pkg::module` form, even if the target
package happens to share a module name with a local one. This is unambiguous because `::`
is absent in intra-package names.

---

## F6 ruling (`intrinsics` and the layer check)

`intrinsics` is compiler-provided and has no manifest, so it has no `layer:`. The layer
check treats `intrinsics` as implicitly below every declared layer: any package at any layer
may depend on it. No `depends:` entry is required or accepted for `intrinsics` (it is a
reserved name, resolved without a path). Attempting `depends: intrinsics path "..." ;` in a
manifest is a parse-time error.

---

## Implementation

### New file: `src/manifest.rs`

Owns the manifest data type, its parser, and the manifest-driven lookup tables. Responsible
for:

- `ManifestDecl` (parsed manifest value; the `Manifest` struct below).
- `parse_manifest(src: &str, path: &Path) -> Result<Manifest, String>`: tokenise with the
  existing lexer, drive the manifest-grammar parser, return errors with file-and-line
  locations.
- `PackageLayer` enum (`Core`, `Fixed`, `Alloc`, `Hosted`) with `PartialOrd` implementing
  the strict ordering.
- `DependsEntry { pkg_name: String, path: PathBuf, span: Span }` with the manifest
  path-token's raw value.
- `Manifest { package: String, layer: PackageLayer, depends: Vec<DependsEntry>, modules:
  Vec<String> }`.

The manifest parser uses the existing `lexer::lex` to tokenise the `sooth.pkg` text (the
lexer produces the same `Token`/`Span` pairs it does for `.sth` source). It does not route
through `parse_bodies` or any word-declaration machinery; it is a dedicated keyword-driven
loop.

Unit tests in `src/manifest.rs` under `#[cfg(test)] mod tests`:

- `parse_manifest_minimal_ok`: a `package:` + `layer:` only manifest parses cleanly.
- `parse_manifest_full_ok`: all four fields parse; `depends` and `modules` round-trip.
- `parse_manifest_unknown_layer_is_error`: `layer: enterprise ;` is rejected with a located
  error naming the unknown value.
- `parse_manifest_duplicate_package_is_error`: two `package:` lines is rejected.
- `parse_manifest_depends_intrinsics_is_error`: `depends: intrinsics path "." ;` is
  rejected.
- `parse_manifest_missing_semicolon_is_error`: a declaration without `;` is rejected.
- `package_layer_ordering_core_lt_fixed`: `Core < Fixed` holds; `Hosted > Alloc` holds.

### New file: `src/packages.rs`

Owns the package-attribution pass and the module-name derivation. Responsible for:

- `PackageAttribution`: a mapping from each `Closure` node index to its owning `Manifest`
  (and manifest path). For nodes with no ancestor manifest, the owning manifest is `None`
  (S1b handles the fallback; this slice makes the pass available).
- `attribute_packages(closure: &Closure) -> Result<PackageAttribution, String>`: walk each
  node's canonical path upward, looking for a `sooth.pkg` alongside it; report errors for
  conflicting module-name derivations within one package.
- `derive_module_name(file: &Path, pkg_root: &Path) -> String`: strips the package root and
  the `.sth` extension, replaces `/` with `::`. Called by `attribute_packages` for each
  attributed node.
- `ModuleTable`: for one package, a map from module name (`String`) to the `Closure` node
  index that provides it. Built by `attribute_packages`.
- `check_package_graph(attribution: &PackageAttribution, closure: &Closure) -> Result<(),
  String>`: runs the three validation checks (A, B, C from OQ4 above):
  - Missing `depends:` for a cross-package import.
  - Public-module violation on a cross-package import.
  - Layer violation in any `depends:` entry.
  Also checks: `intrinsics` requires no `depends:` but is not registerable as a path-based
  dep.

Cross-package import resolution is threaded into the existing `discover_closure` walk (see
below) rather than a separate pass, so `check_package_graph` only validates, it does not
re-resolve.

Unit tests in `src/packages.rs` under `#[cfg(test)] mod tests`:

- `derive_module_name_top_level`: `foo.sth` at pkg root → `foo`.
- `derive_module_name_nested_one`: `text/ascii.sth` → `text::ascii`.
- `derive_module_name_nested_two`: `a/b/c.sth` → `a::b::c`.
- `attribute_packages_no_manifest_returns_none`: a closure of one file with no ancestor
  manifest; its attribution is `None`.
- `attribute_packages_detects_duplicate_module_name`: two files in one package that both
  derive the same module name is a located error naming both files and the name.
- `check_package_graph_missing_depends_is_error`: consumer importing `pkg::mod` with no
  `depends:` for `pkg` produces a located error matching the OQ4-A wording.
- `check_package_graph_private_module_is_error`: consumer importing `pkg::private` where
  `private` is absent from `pkg`'s `module:` list produces a located error matching OQ4-C
  wording.
- `check_package_graph_layer_violation_is_error`: `core`-layer package `depends:` on
  `hosted`-layer package produces a located error matching OQ4-B wording.
- `check_package_graph_layer_equal_is_ok`: two `core`-layer packages depending on each
  other is legal (same layer is allowed).
- `check_package_graph_intrinsics_no_depends_ok`: a package that uses `intrinsics` without
  a `depends:` entry for it compiles without error (intrinsics are implicitly below all
  layers).

### Changes to `src/driver.rs`

**`discover_closure` extension for module-name imports.**

Today `discover_closure` resolves each `import:` by its `imp.path` field (a quoted file
path). After this slice, `Import` gains a variant distinguishing a quoted-path import from
a module-name import. Quoted-path imports are deleted from the language for files inside a
package (S2 completes the corpus migration; this slice is the first step: files inside a
package that still carry quoted-path imports will fail the attribution pass with a located
error). The resolution path for module names in `discover_closure`:

1. Parse the import target as either `module-name` (intra-package) or `pkg::module` (cross-
   package).
2. For intra-package: look up the module name in the current file's package `ModuleTable`
   (built by `attribute_packages` before the walk begins, requiring a two-pass structure or
   a deferred resolution — see below).
3. For cross-package: look up `pkg` in the current package's `depends:` entries, resolve
   the path to a manifest, look up `module` in that manifest's `module:` table, then locate
   the file.

**Two-pass structure in `discover_closure`.**

The existing walk is single-pass (BFS from the entry). Module-name resolution requires
knowing which package a file belongs to before its imports can be resolved; and knowing the
package requires reading the manifest. The approach:

1. First, before the BFS walk begins, scan for the ancestor manifest of the entry file and
   load it. This seeds the package context.
2. During the BFS walk, when `make_node` reads a file, its ancestor manifest is located
   from its canonical path (walking up from its directory). Manifests are cached in a
   `HashMap<PathBuf, Manifest>` keyed by manifest path so each manifest is read at most
   once.
3. Intra-package module-name resolution: the package's `ModuleTable` is built lazily. When
   the first file from a package is processed, its manifest is read and its package root is
   noted; `ModuleTable` entries are added as files are discovered (BFS order means all
   files in a package may not have been found yet, so forward references within one package
   require a second resolution pass or deferred edge resolution).

**Deferred edge resolution for intra-package imports.** To avoid a two-full-pass design,
use deferred resolution: when a module-name import target is not yet in the `ModuleTable`,
record it as an unresolved edge. After the BFS walk completes, resolve all deferred edges
(all files have been attributed by then). This is safe because `discover_closure` already
dedupes by canonical path; intra-package edges resolve to a canonical path by joining the
package root with the module path segments, then appending `.sth`.

**Interaction with `assemble_module`.** `assemble_module` receives a `Closure` whose
`import_targets` are already resolved to module indices. No change to `assemble_module` is
required for this slice's package-graph logic; `check_package_graph` runs between
`discover_closure` and `assemble_module` and produces errors before assembly begins.

**Quoted-path imports inside a package.** A file inside a package (has an ancestor
manifest) that uses a quoted-path `import:` is a located error:

```
error: quoted-path import at line L, col C in <file>:
  file is in package `<pkg>` — use a module name (`<name>`, `<pkg>::<name>`) instead
```

Files with no ancestor manifest (S1b territory) retain quoted-path import support for this
slice; S2 removes it everywhere.

### Changes to `src/parser.rs` / `src/ast.rs`

`Import` (in `ast.rs`) gains a `target` field distinguishing quoted-path from module-name
form, and the grammar itself reorders per OQ3: target first, then an optional qualifier,
then the optional selective list.

```rust
pub enum ImportTarget {
    Path(String),         // today's quoted form
    Module(ModuleName),   // new: resolved by packages.rs
}

pub struct ModuleName {
    pub pkg: Option<String>,  // None for intra-package
    pub module: String,       // path-derived name
}
```

`parser::parse_import` is rewritten for the new token order: `<target> [<qualifier>]
[ | <name>... | ] ;`. It parses the target first (a quoted string for the still-supported
manifest-less path form, or a bare identifier / `::`-joined sequence for a module name).
After the target, it peeks: a `Pipe` token means no explicit qualifier, jump straight to
the selective list; a `Word` token means an explicit qualifier, consume it and then check
for a following `Pipe`; a `Semicolon` means neither, and the qualifier defaults. The default
is computed immediately (not deferred): `ModuleName::last_segment()` for a module target,
or the file stem for a quoted-path target, so `Import.qualifier: String` is always a
concrete bound value downstream — no optionality leaks past the parser.

`parser::scan_imports` is updated to populate `ImportTarget` based on whether the leading
token (now the target, not the qualifier) is a quoted string or a bare identifier / `::`
sequence.

Unit tests in `src/parser.rs` under `#[cfg(test)] mod tests` (alongside the existing R6/R9
import tests):

- `parse_import_explicit_qualifier_binds_given_name`: `import: core::cmp c ;` binds `c`.
- `parse_import_omitted_qualifier_defaults_to_last_segment`: `import: core::cmp ;` binds
  `cmp`.
- `parse_import_wildcard_with_omitted_qualifier_ok`: `import: intrinsics | * | ;` parses
  with qualifier defaulted to `intrinsics` and the selective list `["*"]`.
- `parse_import_selective_with_explicit_qualifier_ok`: `import: core::text s | split trim |
  ;` binds `s` and the two-name selective list.
- `parse_import_quoted_path_target_still_parses`: the manifest-less quoted-path form still
  parses with the reordered grammar (target first, so no change to which token is the
  path string, only to what follows it).

### Growth-structure check (per CLAUDE.md)

The manifest-parsing and package-attribution logic is genuinely new responsibility with
distinct dependencies from `driver.rs`'s orchestration. Two new files are warranted:

- `src/manifest.rs`: manifest data type and parser. Depends on `lexer`, `ast::Span`.
- `src/packages.rs`: package attribution, module naming, graph checks. Depends on
  `manifest`, `ast`, `driver::Closure`.

`driver.rs` imports both. No further split is needed at this slice's scope.

---

## Golden tests

Two golden tests, added to `tests/` (or as `#[test]` with fixture source strings in a
`tests/` file, matching existing golden pattern):

**Golden 1: cross-package build succeeds.**  
A minimal two-package tree: a `core` package with one public module (`cmp`) and a consumer
`app` package that imports `cmp::lt` via `import: core::cmp c ;`. The consumer's `main`
calls `c::lt`. The build produces a binary; the binary runs and exits cleanly.

Test name: `cross_package_import_public_module_builds`.

**Golden 2: layer violation is a located error.**  
A `core`-layer manifest listing `depends: app path "../app" ;` where `app` has `layer:
hosted`. The build fails with the OQ4-B error message naming both packages and both layers.

Test name: `layer_violation_core_depends_on_hosted_is_error`.

**Golden 3: private module is a located error.**  
A consumer importing `core::detail` where `core`'s manifest lists `module: cmp ;` but not
`detail`. The build fails with the OQ4-C error message.

Test name: `cross_package_import_private_module_is_error`.

**Golden 4: missing `depends:` is a located error.**  
A consumer importing `collections::vec` with no `depends: collections ...` in its manifest.

Test name: `cross_package_import_no_depends_is_error`.

---

## Exit criteria

- A program builds against a dependency's module named as `pkg::module`.
- A package-private module (absent from `module:`) is unnameable from outside the package;
  the attempt is a located error naming both packages and the missing module.
- A package that `depends:` on a higher-layer package is a located build error naming both
  packages and both layers.
- All four golden tests pass.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.

---

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "manifest.rs: ManifestDecl data types, parse_manifest, PackageLayer ordering, unit tests",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "packages.rs: PackageAttribution, derive_module_name, attribute_packages, ModuleTable, unit tests",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "ast.rs + parser.rs: ImportTarget variant, scan_imports update for module-name form",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 4,
      "focus": "driver.rs: two-pass discover_closure with deferred intra-package edge resolution and manifest caching; quoted-path-inside-package error",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "packages.rs: check_package_graph (missing depends, private module, layer violation), all three located diagnostics, unit tests",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 6,
      "focus": "golden tests (cross-package build, layer violation, private module, missing depends) and exit-criteria sweep",
      "effort": "M",
      "difficulty": "standard"
    }
  ]
}
```
