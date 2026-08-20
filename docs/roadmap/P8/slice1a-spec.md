# P8.S1a spec — packages, manifests, module-name imports, and the layer check

**Slice:** P8.S1a (delivered) · **Design authority:** `docs/roadmap/P8-packages-modules.md`
· **Brief:** `docs/roadmap/P8/slice1-brief.md`
**Siblings (out of scope):** S1b (`--manifest` flag and fallback chain), S2 (single-mode
imports, intrinsics gating, wildcard semantics, re-export, prelude deletion).

---

## Scope

1. `sooth.pkg` manifest format and parser (Sooth-lexed).
2. Package boundaries by nearest-ancestor manifest.
3. Module names as `pkg`-root-relative paths (`text/ascii.sth` → `text::ascii`) and the
   `module:` public list.
4. Cross-package `pkg::module` resolution against `depends:`/`module:`, plus intra-package
   `self::` resolution (replacing the quoted-path import inside a package).
5. Post-discovery, pre-`assemble_module` audit: private module named from outside, package
   with no `depends:` entry, `depends:` on a higher layer.

Not this slice: `--manifest` (S1b); wildcard/intrinsics *semantics*, prelude deletion (S2);
semver, API descriptions (P8.S3); git dependency paths.

---

## Rulings

### OQ1: manifest grammar

A flat sequence of `;`-terminated declarations over four keywords. Only `module:` may repeat
(it accumulates, like `export:`).

```
package: core ;
layer: core ;
depends: core path "../core" ;
module: bool cmp text ;
```

- **`package:`** one identifier, the canonical name. Rejects a name carrying `:` (the `pkg`
  segment of an import target is matched verbatim, so a `::`-bearing name is unreachable)
  and a bare `*` (reserved for the wildcard target). Same rule at the `depends:` name site.
- **`layer:`** a fixed four-value enum, `core < fixed < alloc < hosted`, strict and total.
  Unknown value is a parse-time error at the `layer:` line. Fixed enum, not an open declared
  ordering: O(1) per check and a stable meaning in diagnostics.
- **`depends: <pkg-name> path "<path>" ;`** one dependency per declaration; the name must
  match that package's own `package:` field (no aliasing, so the table stays auditable); the
  path is relative to the declaring manifest's directory. `depends: intrinsics` in any form
  is a parse-time error (F6).
- **`module:`** each token is a module name (`cmp`, or `::`-joined `text::ascii`). Absent
  `module:` is valid: a package exposing nothing. Public surface only, no layout implication.
- No version/revision field (OQ5). `path` distinguishes the present form, so `git`/`rev`
  land later as an additive keyword. No placeholder.

### OQ2: manifest location, nesting, and the segment rule

The manifest sits at the package root; every descendant file belongs to the package.
Subdirectory nesting is unbounded. **Manifest locality:** a file's package is its *nearest*
ancestor manifest, so an inner manifest wins over an outer one. `text.sth` beside `text/` is
legal (F3): `text` and `text::ascii` are distinct names.

**Where the naming rule is enforced.** Resolution is name → path only (join the package root
with the segments, append `.sth`), so there is no path → name derivation to reject a badly
named file at. The rule is checked on an import target's segments before the join
(`packages::segment_defect`), which makes such a file *unnameable* rather than an error in
itself: `42.sth` may sit in a package untouched as long as no import names it.

A segment must lex, on its own, as exactly one `Token::Word`. That single rule covers
delimiters and whitespace (multi-token), `\` (comment marker), `42` (`Token::Int`), `3.5`
(`Token::Float`). Two explicit exclusions the token rule admits on its own:

- **`:`** — the lexer's word set is `:`-permissive, and `::` is the segment separator, so a
  surviving `:` would give one name two spellings.
- **bare `*`** — reserved for S2's wildcard target position; a module named `*` would be
  unreachable as an ordinary target.

**`.` is not excluded.** The extension is *appended*, never `set_extension`ed, so `ascii.io`
names `ascii.io.sth` (not `ascii.sth`) and the name ↔ path map stays one-to-one.

### OQ3 / F2: import grammar and anchors

```
import: <target> [<qualifier>] [ | <name>... | ] ;
```

Target first, qualifier optional and freely chosen; omitted, it defaults to the target's last
segment (`import: self::text::ascii ;` binds `ascii`) or the file stem for a quoted path. The
default replaces the ceremony, not the freedom (short qualifiers in dense files,
disambiguation when two dependencies both have an `ascii`).

Anchors are syntactic, never inferred:

- `self::` → `ImportAnchor::SelfPackage`, package-root-relative to the importer's own package.
- no prefix → `ImportAnchor::Dependency`, first segment names a `depends:` entry.

A local `text/` and a dependency named `text` coexist; there is no precedence rule and no
ambiguity error. `module:` visibility is never consulted for a SelfPackage anchor. `super::`
is not in this design (see the declined list in `P8-packages-modules.md`).

Parsing after the target: `|` means no explicit qualifier; a `*` followed immediately by `;`
(no `|`) builds `ImportBinding::Wildcard`; any other word is the qualifier; `;` alone
defaults it. `*` inside `| ... |` is the ordinary word `*`. This slice parses the wildcard
shape so S2 need not reopen the grammar; its visibility *effect* is S2's.

### F6: `intrinsics`

Compiler-provided, no manifest, no layer, implicitly below every layer. Matched as a reserved
single-segment Dependency target *before* any `depends:` lookup, and adds no closure edge, so
it can never raise OQ4-A. `self::intrinsics` is not the reserved name and resolves as an
ordinary own-package module (normally D1). No `depends:` entry is required or accepted.

---

## OQ4: diagnostic catalog

Every one is located to the offending `import:` or `depends:` span (file, line, col). Module
resolution diagnostics share a header: ``error: import `<target>` at line L, col C in
<importer>:``.

**A — no matching `depends:` entry** (import-triggered, raised by the audit):

```
error: import `<pkg>::<module>` at line L, col C in <importer>:
  package `<consumer-pkg>` has no `depends:` entry for `<pkg>`
  add `depends: <pkg> path "<path>" ;` to <consumer-manifest-path>
```

`<path>` is a literal placeholder: no dependency manifest was ever located.

**B — `depends:` on a higher layer** (manifest-triggered, located to the entry):

```
error: layer violation in <declaring-manifest-path>, line L, col C:
  package `<declaring-pkg>` is layer `<X>` but depends on `<dep-pkg>` which is layer `<Y>`
  a `<X>` package may only depend on packages at the same layer or below
```

**C — cross-package import of a non-public module** (import-triggered):

```
error: import `<pkg>::<module>` at line L, col C in <importer>:
  module `<module>` is not in `<pkg>`'s public `module:` list
  add `module: <module> ;` to <pkg-manifest-path> to make it public
```

**D1 — nothing at the joined path** (locate failure, raised inline during resolution;
sibling to R5's missing quoted-path error):

```
  package `<pkg>` has no module `<module>` (looked for <path-tried>)
```

**D2 — the path exists but a nested manifest owns it** (its own wording: the file *was*
found, it just is not that package's module):

```
  package `<pkg>` has no module `<module>`: `<path-tried>` belongs to the nested package
  rooted at `<inner-manifest-path>`, not `<pkg>`
```

**Bare package name** (`import: core ;`, and the degenerate `import: self ;`), checked ahead
of the `depends:` lookup so a typo does not get a confusing "no `depends:` entry":

```
  `<pkg>` names a package, not a module -- import one of its `module:` entries instead
```

**Ill-formed segment** (OQ2, either anchor, before the join): the shared header plus one of
``module-name segment `<seg>` is not a single identifier`` / ``… is reserved for the wildcard
import target`` / ``… contains `:`, which is reserved for the `::` separator``.

**Module name outside any package** (mirror of the next one; each form is rejected exactly
where the other is the answer, both revisited by S1b):

```
  <importer> has no `sooth.pkg` ancestor, so a module name has no package to resolve against
  add a manifest, or use a quoted-path import for now
```

**Quoted path inside a package:**

```
error: quoted-path import at line L, col C in <file>:
  file is in package `<pkg>`: use a module name (`self::<name>`, or `<pkg>::<name>` for a dependency) instead
```

Manifest-less files keep quoted-path support for this slice; S2 removes it everywhere.

**`depends:` path with no manifest** (located to the entry, not the import that tripped over
it: the same entry fails for every importer):

```
error: `depends:` entry `<pkg>` at line L, col C in <manifest>:
  no manifest at <path-tried>
```

**`depends:` name mismatch:**

```
error: `depends:` entry names `<foo>` at line L, col C in <manifest>:
  that package declares `package: <bar>` -- rename the entry to match
```

**Duplicate import qualifier** (in `driver.rs`, where `import_map` is built; second binding
is always the error, for explicit and defaulted qualifiers alike):

```
error: duplicate import qualifier `<q>` at line L, col C in <file>:
  qualifier `<q>` was first bound at line L2, col C2
```

---

## Implementation

### `src/manifest.rs`

Depends on `lexer` and `ast::Span`; a dedicated keyword-driven loop, not `parse_bodies`.

- `parse_manifest(src, path) -> Result<Manifest, String>`.
- `PackageLayer { Core, Fixed, Alloc, Hosted }` with the strict `PartialOrd`.
- `DependsEntry { pkg_name, path, span }`, `Manifest { package, layer, depends, modules }`.
  `package:` and `layer:` are mandatory; either missing is a located end-of-file error.

A file with no `package:` hits that end-of-file error whatever it begins with. There is no
separate diagnostic for the user-level `global_sooth.pkg` shape: S1b gives it its own entry
point over the shared `depends:` line grammar rather than routing it through `parse_manifest`,
which would demand a `package:` the file is never meant to have.

Tests: `parse_manifest_minimal_ok`, `_full_ok`, `_unknown_layer_is_error`,
`_duplicate_package_is_error`, `_qualified_package_name_is_error`,
`_wildcard_package_name_is_error`, `_qualified_depends_name_is_error`,
`_missing_package_is_error`, `_missing_layer_is_error`, `_depends_intrinsics_is_error`,
`_missing_semicolon_is_error`, `package_layer_ordering_core_lt_fixed`.

### `src/packages.rs`

Depends on `ast`, `lexer`, `manifest`; never on `driver` (the driver hands one `Import` in
and gets a path back), so it is unit-testable without a `Closure`.

**Name → path, eagerly.** Join the package root with the segments, append `.sth`, then
re-check that the resulting file's own nearest-ancestor manifest is still the package the
import named. No `PackageGraph`, no `ModuleTable`, no enumeration of a package's files.
Consequences: forward references cost nothing (so no deferred-edge pass, and no hazard of
adding edges after `reject_cycles`), one pass instead of two, and no directory walk, so a
package's private files are never read.

- `find_package_root(file) -> Option<PathBuf>`: nearest ancestor holding a `sooth.pkg`.
- `PackageSite { manifest_path, root, manifest }` with `module_file(segments)` doing the join.
- `ManifestCache`: `find_package_root` plus a `BTreeMap<PathBuf, Manifest>`, so each manifest
  is parsed at most once per build. Ordered, not hashed: the audit walk is seeded from its key
  set, so a `HashMap` would make *which* of several defects a build reports depend on hash
  order.
- `segment_defect(seg) -> Option<String>`: OQ2's rule and the reason a segment breaks it;
  `check_module_name` applies it to a target before the join.
- `resolve_import(importer, importer_dir, imp, site, manifests, unresolved) ->
  Result<Option<PathBuf>, String>`: `Ok(None)` means no closure edge (reserved `intrinsics`,
  or a cross-package import recorded as unresolved). Owns every OQ4 diagnostic except the
  duplicate qualifier.
- `UnresolvedImport { importer_pkg, importer_manifest, importer, pkg, pkg_manifest, module,
  span, kind }`, `UnresolvedKind { MissingDepends, PrivateModule }`. A and C name a specific
  `import:`, so resolution records one of these and adds no edge instead of raising.

**Resolution order** for a module target: reserved `intrinsics` first; then by anchor.
SelfPackage joins the importer's root and re-checks ownership. Dependency looks the first
segment up in `depends:` (absent → record `MissingDepends`; path with no manifest → its own
located error), joins the dependency's root with the remaining segments *unfiltered by
`module:`* (a private file still exists and must be locatable), then D1 if nothing is there,
`PrivateModule` if it is not public, D2 if a nested inner manifest owns it. The nested
re-check is what keeps `self::` from reaching into a nested package's private modules or
around its layer check.

**`check_package_graph(manifests: &mut ManifestCache, unresolved: &[UnresolvedImport])`**
runs after `discover_closure`, before `assemble_module`, from one call site in
`discover_closure`. Four checks:

- Manifest walk (layer violation, `depends:` name mismatch, missing dependency manifest) over
  every `depends:` entry of every reachable manifest, whether or not anything imports across
  it. A **worklist**, not a pass over what the walk loaded: resolution only loads a
  dependency's manifest when an import reaches for it, so an unimported `depends:` target's
  `layer:` is otherwise never read (Golden 2b is the test that fails if it isn't). Seeded
  from `known_manifest_paths()` (sorted) and grown as manifests load, so the defect a build
  names is a function of the source tree alone.
- Then the recorded `unresolved` entries: `MissingDepends` → A, `PrivateModule` → C.

**Audit order is load-bearing.** The manifest walk runs to completion first, so a layer
violation anywhere in the reachable graph masks an A/C import error. Intended: a `depends:`
entry that is both an unimported layer violation and the source of an unresolvable import has
the layer violation as its root cause, and this is what makes Golden 2 and 2b agree
regardless of whether the fixture's `import:` line is present.

Tests: `find_package_root_no_manifest_returns_none`, `_nested_manifest_inner_wins`,
`module_file_joins_segments_under_the_root`,
`module_file_appends_extension_keeping_a_dotted_segment` (guards append vs `set_extension`,
silent otherwise), `module_segment_single_word_is_ok`, `_non_word_is_rejected` (table-driven
over `\`, `42`, `3.5`, `my file`, `a;b`, `(`, a quoted string), `_colon_is_rejected`,
`_star_is_rejected`, plus `check_package_graph_missing_depends_is_error`,
`_private_module_is_error`, `_layer_violation_is_error`, `_depends_name_mismatch_is_error`
(each pinning the exact message with line/col and both names, each mutation-tested by
deleting its check) and `_layer_equal_is_ok` (not a guard-deletion test; its only real
mutation is `>` → `>=`).

### `src/driver.rs`, `src/repl.rs`

`discover_closure` stays **single-pass** BFS. Each file's manifest is located from its
canonical path through `ManifestCache`; every edge is in the graph before `reject_cycles`.

A **module-name import at the REPL is rejected** with a located error: REPL imports resolve
against the user-level manifest, which is S1b's work, and anything not wired into
`assemble_module` is unenforced at the REPL, so silent fall-through is not an option.

`assemble_module` is unchanged: it receives a closure whose `import_targets` are already
resolved, and `check_package_graph` errors before assembly.

Tests in `driver.rs`: `discover_closure_intra_package_forward_reference_resolves`,
`_manifest_cache_reads_once`, `_inner_manifest_wins` (mutate: invert the nearest-ancestor
walk), `_quoted_path_inside_package_is_error`, `driver_duplicate_import_qualifier_is_error`,
`self_anchored_import_into_nested_package_is_error` (mutate: delete the ownership re-check),
`dependency_anchored_import_into_nested_package_is_error`,
`self_intrinsics_is_not_the_reserved_name`, `resolve_bare_package_name_no_module_is_error`,
`import_target_non_word_segment_is_error` and `import_target_star_segment_is_error` (the
offending file is written in the fixture, so a D1 error cannot stand in for the naming rule;
mutate: delete the `check_module_name` call), `module_import_outside_a_package_is_error`
(mutate: return `Ok(None)`, which would leave the import silently unbound),
`depends_entry_with_no_manifest_is_error`, `resolve_intrinsics_precedes_depends_lookup`, and
`self_import_of_non_public_module_is_ok` — explicitly a regression fence, not a killed-mutant
guard: nothing in the SelfPackage path reads `module:`, so there is no guard to delete.
`repl_module_name_import_is_rejected` in `repl.rs`.

### `src/ast.rs`, `src/parser.rs`

```rust
pub enum ImportAnchor { Dependency, SelfPackage }
pub struct ModuleName { pub anchor: ImportAnchor, pub segments: Vec<String> }
pub enum ImportTarget { Path(String), Module(ModuleName) }
pub enum ImportBinding {
    Qualified { qualifier: String, selective: Vec<(String, Span)> },
    Wildcard,
}
```

`selective` keeps a `Span` per name: it becomes `check::SelectiveName.span`, which locates
every R20/R21 selective-import diagnostic. `parse_import` computes the default qualifier
immediately, so no optionality leaks past the parser.

Every `.qualifier`/`.selective` read in `driver.rs` and `repl.rs` becomes an `ImportBinding`
match. The `Qualified` arm keeps today's behaviour; the `Wildcard` arm carries neither field
and is a no-op wherever the site iterates per-name (selective loops, collision/export checks,
the splice's alias install). The two sites keying on qualifier text: `import_map.insert`
inserts no entry, and the REPL prints `imported <target> (wildcard)`. Arms exist so matches
are exhaustive; they gate no names.

`word_families.rs`'s `drop`-visibility remedy teaches the import order to real users and was
re-worded for target-first, along with the expectations pinning it.

Test-suite churn: `import_and_export_forms_parse` (R6) rewritten for the new order.
`malformed_import_missing_path_is_located_error` (R9) **deleted** — it asserted `import: q ;`
is a parse error, which is now a legal Dependency import — and replaced by
`malformed_import_missing_target_is_located_error` (`import: ;`), keeping parse-error
coverage. New: `parse_import_explicit_qualifier_binds_given_name`,
`_omitted_qualifier_defaults_to_last_segment`, `_self_prefix_sets_self_anchor`,
`_omitted_qualifier_self_defaults_to_last_segment`, `_bare_wildcard_builds_wildcard_variant`,
`_selective_list_star_is_literal_word`, `_selective_with_explicit_qualifier_ok`,
`_quoted_path_target_parses`.

### Growth structure (CLAUDE.md)

Resolution lives in `packages.rs` rather than `driver.rs`, where it was first written: with
it there, `driver.rs` did discovery *and* attribution *and* manifest IO *and* assembly *and*
process orchestration, with matching import divergence (`manifest`/`lexer`/`packages` against
`Command`/`ExitStatus`) and never-call-each-other function clusters. `driver.rs` keeps what
owns the `Closure`: the walk, cycle rejection, assembly, build/run orchestration. No further
split at this scope.

---

## Golden tests (`tests/phase8_slice1a.rs`)

Every error golden pins the exact message substring including line/col and both relevant
names. `is_err()` alone is a placebo; this project has shipped five.

**Entry point: through `driver::build` / `driver::discover_closure` on a fixture tree on
disk, never `packages::check_package_graph` directly.** Phase 4's unit tests all call it
directly, leaving its single call site unguarded — deleting that line passes the whole suite.
Goldens 2, 2b, 3, 4 are the only tests that close that hole, so each must fail both when the
call site is deleted and when the individual check it pins is deleted.

1. `cross_package_import_public_module_builds` — a `core` package with public `cmp` and an
   `app` package (`layer: hosted`) importing it via `import: core::cmp c ;`; builds, runs,
   exits cleanly.
2. `layer_violation_core_depends_on_hosted_is_error` — pins ``package `core` is layer `core`
   but depends on `app` which is layer `hosted``` with both layer values.
3. `layer_violation_fires_without_an_import` — Golden 2's fixture with the `import:` line
   deleted, same message. If these two ever disagree, the layer check has silently become
   import-triggered.
4. `cross_package_import_private_module_is_error` — pins ``module `detail` is not in
   `core`'s public `module:` list``.
5. `cross_package_import_no_depends_is_error` — pins ``package `app` has no `depends:` entry
   for `collections``` and asserts the location prefix separately (it is a different part of
   the same message). The fixture's import deliberately does not sit at (1,1), so a
   degenerate location cannot pass by accident.

---

## Exit criteria (met)

- A program builds against a dependency's module named `pkg::module`.
- A package-private module is unnameable from outside; the attempt is located and names both
  packages and the module.
- A `depends:` on a higher-layer package is a located build error naming both packages and
  both layers.
- All five goldens pass; `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
  green.

---

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "manifest.rs: parse_manifest, PackageLayer ordering, Manifest/DependsEntry types, unit tests",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "packages.rs: find_package_root, PackageSite path-join, ManifestCache, the OQ2 segment rule, unit tests",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "ast.rs + parser.rs (ImportAnchor, ImportBinding, grammar rewrite) + packages.rs resolve_import (eager path-join, nested-manifest re-check, manifest cache) wired into discover_closure + REPL rejection + repl.rs consumer updates + mechanical corpus migration (125 import sites); one compile-green commit",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "packages.rs: check_package_graph (missing depends, private module, layer violation, name mismatch), all located diagnostics, unit tests",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 5,
      "focus": "golden tests (cross-package build, layer violation, layer violation without an import, private module, missing depends) and exit-criteria sweep",
      "effort": "M",
      "difficulty": "standard"
    }
  ]
}
```
