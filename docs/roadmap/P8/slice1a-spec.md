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
   a. naming a module the package does not make public to a consumer outside it;
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
package by its declared `package:` name, not an alias. "Single identifier" excludes a name
carrying `:` and a bare `*`, at both the `package:` and the `depends:` site: the `pkg`
segment of an `import: pkg::module ;` target is matched verbatim against the declared name,
so a `::` -bearing name would be unreachable from every import, and OQ3 reserves a bare `*`
for the wildcard import form.

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
depends: <pkg-name> path "<path>" ;
```

The first token after `depends:` is the dependency's canonical package name (must match
that package's own `package:` field). The keyword `path` follows, then a quoted path
string, interpreted relative to the declaring manifest's directory. No aliasing: the
consumer names the dependency by the same identifier the dependency declares as its
`package:` name. This is consistent with Phase 4 Slice 5's
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
terminated by `;`. Each token is a module name: a single identifier (e.g. `cmp`) or a
`::` -joined multi-segment name (e.g. `text::ascii`). See module naming below.
An empty `module:` list (no `module:` line at all) is valid: an
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
`core/sooth.pkg` is the module `text::ascii`. Nesting depth is not bounded. A module-name
segment must lex, on its own, as a single `Token::Word` (`src/lexer.rs`): run the segment
through `lexer::lex` and require exactly one token, of that variant. This is a token-level
rule, not a character list, and it decides every case in one pass: a segment containing a
delimiter (`; ( ) | [ ] "`) or whitespace lexes as more than one token; a segment that is
exactly `\` lexes as a comment marker, not a word; a purely numeric segment (`42`) lexes as
`Token::Int`; a segment shaped like a float (`3.5`) lexes as `Token::Float`. A directory name
must satisfy the same constraint, since it becomes a module-name segment. A bare `*` segment
is excluded by a separate rule below it (it lexes as an ordinary `Token::Word`, so the
token-level rule alone would admit it).

**Where the rule is enforced: the import target, not a path-to-name derivation.**
Resolution goes name → path (join the package root with the segments, append `.sth`), never
path → name, so there is no derivation step to reject a badly named file at. The rule is
checked on the segments of a module-name import target instead, before the join
(`packages::segment_defect`). The observable outcome is the same one this ruling wants: a
file whose name breaks the rule is *unnameable*, since the only spelling that would reach
it is rejected. Its located error is the OQ4 import header plus the offending segment and
the rule it broke. Consequence worth stating: an unimportable file is not itself an error,
because nothing ever looks at it -- `42.sth` may sit in a package unmolested as long as no
import names it. Under a path → name derivation over the discovered closure the same was
true, since an unimported file was never in that list either.

**Manifest locality.** A file's package is always its *nearest* ancestor manifest. Two
manifest files may nest (a monorepo with packages inside packages): the inner manifest wins
for files beneath it.

**F3 ruling (is `text.sth` beside `text/` legal?).** Yes. The hub pattern requires it:
`core::text` is a file (`text.sth`) and `core::text::ascii` is a file inside `text/`. The
name-derivation rule handles both correctly since `text.sth` derives `text` and
`text/ascii.sth` derives `text::ascii`; the two names are distinct and do not conflict.

**Why `:` is excluded even though it lexes as one word.** The single-`Token::Word` rule
above does not by itself exclude `:`: the lexer's word character set is `:`-permissive
(`self::text::ascii` lexes as one word), so a segment containing `:` passes the token-level
test. It is rejected anyway, as an explicit additional rule: `::` is the segment separator,
so a `:` surviving inside a segment would mean two spellings of one name and a
`name <-> path` map that is no longer one-to-one. This is the one collision in the whole
scheme, and this is the rule that closes it.

**`.` is not excluded.** Exactly one `.sth` is appended to the last segment, so a segment
containing `.` (e.g. `ascii.io`, naming `ascii.io.sth`) collides with nothing: the map
stays one-to-one. There is no rationale for excluding it, so this slice doesn't. Note the
implementation consequence: the extension is *appended*, not `set_extension`ed, which would
silently rewrite `ascii.io` to `ascii.sth`.

**Why a bare `*` segment is excluded.** `*` lexes as an ordinary `Token::Word`, so the
token-level rule admits it. It is rejected anyway: OQ3 reserves a bare `*` in the target
position for S2's wildcard import (`import: intrinsics * ;`), and a module literally named
`*` would be unreachable as an ordinary target (`import: pkg::* ;` would parse as the
wildcard shape, never as a module named `*`). A target segment violating the `Token::Word`
rule, the `:` rule, or the `*` rule is a located error at the `import:`.

### OQ3: Whether the local import qualifier stays freely chosen

**Ruling: the qualifier stays freely chosen by the importer, and the grammar changes to
make the common case terser.** The target now comes first, and the qualifier is optional:

```
import: <target> [<qualifier>] [ | <name>... | ] ;
```

The target begins with an optional `self::` prefix (SelfPackage anchor, own package) or
no prefix (Dependency anchor, package in `depends:`). `import: core::cmp c ;` binds `c` as
the local qualifier; `import: self::text::ascii ;` (qualifier omitted) binds `ascii`, the
last segment. Either spelling is legal, and both produce a bound qualifier; for a `Qualified`
import the qualifier is never absent at the semantic level, only optionally elided in
source (a `Wildcard` import, S2's `import: intrinsics * ;` shape, carries no qualifier at
all -- that is a different `ImportBinding` variant, not an elided one). The reason for
the reorder: putting the target first means the common case (no renaming wanted) needs no
ceremony at all, where the old
qualifier-always-first grammar forced every import to spell out a name even when it would
just echo the module's own. The reason for keeping the qualifier free rather than fixing it
to the last segment unconditionally: Phase 4 Slice 5's existing qualifier freedom is
load-bearing for code style (short qualifiers in dense files, disambiguating qualifiers
when two dependencies each have an `ascii` module), and the default only replaces the
ceremony, it doesn't remove the freedom.

Parsing is unambiguous: after the target, the parser sees either `|` (selective list, no
explicit qualifier), a bare word (the qualifier, optionally followed by `| ... |`), or `;`
(default qualifier, no selective list). A bare `*` immediately after the target, with
nothing but `;` following, is a fourth case reserved for S2's wildcard import
(`import: intrinsics * ;`) rather than an ordinary qualifier named `*`; this slice's parser
change recognizes the position (target, then `*`, then `;`, no `|`) so S2 doesn't have to
reopen the grammar, but the wildcard's semantics (bringing every exported name in
unqualified) are S2's to implement. `*` is never special inside `| ... |`: a selective list
naming `*` imports the literal word `*` (already a live word name for multiplication in
this symbol-operator style), so the two forms never collide.

The target is parsed for its anchor: a `self::` prefix means SelfPackage (own package,
package-root-relative); no prefix means Dependency (the first segment names a package in
`depends:`). The anchor drives resolution, never inference. `super::` is not supported;
see the declined list in `P8-packages-modules.md`.

### OQ4: Diagnostic wording and located-ness

This slice's error catalog is five located diagnostics: failure modes A, B, C, D below, plus
the bare-package-name case. "Located" means each names the span of the offending `import:`
or `depends:` declaration (file, line, col), not just the file.

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
public.**

Error format:

```
error: import `<pkg>::<module>` at line L, col C in <importer>:
  module `<module>` is not in `<pkg>`'s public `module:` list
  add `module: <module> ;` to <pkg-manifest-path> to make it public
```

**Failure mode D: a module target does not exist where its anchor says it should.**

D covers two sub-cases, both raised directly inside `discover_closure`'s resolution step
(`src/driver.rs`), the same file and a sibling check to R5's existing missing-import error
(R5 fires for a missing quoted-path target; D is the module-name-target analogue). Neither
sub-case is recorded for a later pass: `check_package_graph` never sees either, since both
are pure file-attribution questions with no `depends:`/`module:` cross-package data
involved.

**D1: the module does not exist at all.** Nothing is at the path the module name joins to
inside the importer's package (SelfPackage anchor) or inside the named dependency's package
(Dependency anchor). This is the case the F6 ruling leans on for
`self::intrinsics`: `intrinsics` is a reserved Dependency-anchor name and never matches a
`self::` target, so `self::intrinsics` resolves like any other own-package module name and
hits D1 when no `intrinsics.sth` exists.

Error format:

```
error: import `self::<module>` at line L, col C in <importer>:
  package `<pkg>` has no module `<module>` (looked for <path-tried>)
```

(Dependency anchor: the header reads `import <pkg>::<module>` in place of
`import self::<module>`.) Located to the `import:` span. Names the package, the module
name, and the path that was tried (the package root joined with the module's path
segments and `.sth`).

**D2: the joined path exists, but a nested manifest claims it first.** Path-joining the
package root with the module's segments lands on a real file, but that file's own
nearest-ancestor manifest is a *different*, more deeply nested manifest than the one the
import named (manifest locality, OQ2): a package inside a package. The target does not
name a module of the package the import thinks it does, so this is not the same situation
as D1 and gets its own wording naming the boundary crossed, rather than reusing D1's
"not found" text for a file that, in fact, was found:

```
error: import `self::<module>` at line L, col C in <importer>:
  package `<pkg>` has no module `<module>`: `<path-tried>` belongs to the nested package
  rooted at `<inner-manifest-path>`, not `<pkg>`
```

(Dependency anchor: same second line, with `import <pkg>::<module>` in the header.) Located
to the `import:` span. Names the package the import named, the module, the path tried, and
the inner manifest that actually owns it.

**Bare package name, no module segments (`import: core ;`).** A Dependency-anchored target
with no segments beyond the package name identifies a package, not a module; a package is
not importable on its own. Error format:

```
error: import `<pkg>` at line L, col C in <importer>:
  `<pkg>` names a package, not a module -- import one of its `module:` entries instead
```

Located to the `import:` span, names the package. Checked before the `depends:` lookup, so
a typo'd `import: core ;` gets this message rather than a confusing "no `depends:` entry"
(there may well be one). The degenerate `import: self ;` is the same shape on the
Dependency anchor's side, since bare `self` with no `::` is an ordinary package name, not
the `self::` prefix: without this check it would report "no `depends:` entry for `self`",
technically correct if no such dependency exists but a confusing remedy for what is
actually a missing module segment. No separate handling is needed for it; the "names a
package, not a module" wording already covers both cases.

**Ill-formed module-name segment (OQ2).** A module-name target whose segment breaks the
OQ2 naming rule, checked before the path join and under either anchor:

```
error: import `<target>` at line L, col C in <importer>:
  module-name segment `<seg>` is not a single identifier
```

with `is reserved for the wildcard import target` and ``contains `:`, which is reserved for
the `::` separator`` as the second line's other two forms.

**Module name outside any package.** A file with no ancestor manifest has no package root
to join to and no `depends:` table to look in, so a module-name target there names nothing.
Silently declining to resolve it would leave the import unbound with no message, so:

```
error: import `<target>` at line L, col C in <importer>:
  <importer> has no `sooth.pkg` ancestor, so a module name has no package to resolve against
  add a manifest, or use a quoted-path import for now
```

This is the mirror of the quoted-path-inside-a-package error: each form is rejected exactly
where the other one is the answer. Both are S1b's to revisit, since a user-level manifest
gives manifest-less files a `depends:` table.

**`depends:` entry pointing where no manifest is.** Located to the entry in the declaring
manifest, not to the import that tripped over it: the manifest is what is wrong, and the
same broken entry would fail for every importer.

```
error: `depends:` entry `<pkg>` at line L, col C in <manifest>:
  no manifest at <path-tried>
```

### OQ5: `depends:` version/revision field

Deferred cleanly: no version or revision field in this slice. The `path` keyword
distinguishes the present form; adding `git`, `rev`, or `version` later is an additive
keyword extension with no grammar overlap. Ruling: do not add a placeholder or FIXME for
this; the grammar is complete for this slice's purposes.

---

## F2 ruling (intra-package reference base, dogfood finding F2)

Import anchors are syntactic, not inferred. The anchor is determined by the presence or
absence of a `self::` prefix on the import target:

- `self::` always means the importing file's own package, package-root-relative:
  `import: self::text::ascii a ;` names `text/ascii.sth` relative to the package root,
  regardless of whether a dependency named `text` exists.
- A bare first segment always means a dependency package: `import: core::cmp c ;` names
  the `core` entry in `depends:`.

```
import: self::text::ascii a | upper? lower? | ;   \ own package, path-derived
import: core::cmp c | lt gt | ;                   \ dependency package
```

Resolution picks the local module table (SelfPackage anchor) or the `depends:` table
(Dependency anchor) by the token the parser records, never by guessing. No ambiguity is
representable: a dependency named `text` and a local `text/` directory coexist fine. There
is no precedence rule and no ambiguity error.

`module:` visibility is never consulted for a SelfPackage anchor: the public/private
distinction (OQ4 failure mode C) applies only to a Dependency-anchored import crossing into
another package. Every module in the importer's own package stays reachable via `self::`,
whether or not it is in that package's `module:` list.

The default qualifier is the last segment: `import: self::text::ascii ;` binds `ascii`;
`import: core::cmp ;` binds `cmp`.

`super::` (parent-module-relative) is not in this design: `self::` is package-root-
absolute and already names every module in the package. See the declined list in
`P8-packages-modules.md`.

---

## F6 ruling (`intrinsics` and the layer check)

`intrinsics` is compiler-provided and has no manifest, so it has no `layer:`. The layer
check treats `intrinsics` as implicitly below every declared layer: any package at any layer
may depend on it. No `depends:` entry is required or accepted for `intrinsics`:
attempting `depends: intrinsics ...` in any form is a parse-time error. Rationale:
`intrinsics` is compiler-provided with no path, manifest, or layer, and no package can
avoid needing it, so a universally-required declaration carries no information. The per-
file `import: intrinsics * ;` is where the auditability argument applies.

---

## Implementation

### New file: `src/manifest.rs`

Owns the manifest data type, its parser, and the manifest-driven lookup tables. Two top-
level shapes share the `depends:` line grammar:

- A **package manifest** (`sooth.pkg` with an ancestor directory): `package:` and `layer:`
  are mandatory; `depends:` and `module:` are optional and accumulate.
- The **user-level file** (`global_sooth.pkg`): a bare `depends:` table with no `package:`
  or `layer:`. S1a owns the `depends:` line grammar; S1b (the `--manifest` flag and
  fallback chain) owns the user-level file's top-level shape and its parse path.

`parse_manifest` in this slice only produces the `Manifest` struct (package manifest).
A file with no `package:` declaration is rejected by the end-of-file located error below,
whatever it begins with; there is no separate diagnostic for the user-level file's bare
`depends:` shape. S1b therefore gives the user-level file its own entry point over the
shared `depends:` line grammar rather than routing `global_sooth.pkg` through
`parse_manifest`, which would report a missing `package:` for a file that is never meant to
have one.

Responsible for:

- `parse_manifest(src: &str, path: &Path) -> Result<Manifest, String>`: tokenise with the
  existing lexer, drive the manifest-grammar parser, return errors with file-and-line
  locations.
- `PackageLayer` enum (`Core`, `Fixed`, `Alloc`, `Hosted`) with `PartialOrd` implementing
  the strict ordering.
- `DependsEntry { pkg_name: String, path: PathBuf, span: Span }` with the manifest
  path-token's raw quoted string value.
- `Manifest { package: String, layer: PackageLayer, depends: Vec<DependsEntry>, modules:
  Vec<String> }`: `package:` and `layer:` are mandatory; a missing `package:` or
  `layer:` is a located error at end of file.

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
- `parse_manifest_qualified_package_name_is_error`: `package: core::x ;` is rejected with a
  located error naming the offending name; likewise `parse_manifest_wildcard_package_name_is_error`
  for a bare `*`, and `parse_manifest_qualified_depends_name_is_error` at the `depends:` site.
- `parse_manifest_missing_package_is_error`: a manifest with `layer:` but no `package:` is
  rejected at end of file with a located error stating `package:` is required.
- `parse_manifest_missing_layer_is_error`: a manifest with `package:` but no `layer:` is
  rejected at end of file with a located error stating `layer:` is required.
- `parse_manifest_depends_intrinsics_is_error`: `depends: intrinsics path "." ;` is
  rejected (any form of `depends: intrinsics` is a parse-time error).
- `parse_manifest_missing_semicolon_is_error`: a declaration without `;` is rejected.
- `package_layer_ordering_core_lt_fixed`: `Core < Fixed` holds; `Hosted > Alloc` holds.

### New file: `src/packages.rs`

Owns package boundaries and module-name import resolution. `packages.rs` depends on `ast`,
`lexer`, and `manifest` only, never on `driver`: the driver hands one `Import` in at a time
and gets a path back, so `Closure` and the walk that builds it stay the driver's concern.
(An earlier draft of this section said `packages.rs` "never sees an `ImportTarget`". That
was a proxy for the constraint that matters -- no `driver` dependency, so the module is
unit-testable without constructing a `Closure` -- and resolution reading an `Import`
does not weaken it.)

**Resolution is name → path, not path → name.** A module name is resolved by joining the
package root with its segments and appending `.sth` to the last, then re-checking that the
resulting file's own nearest-ancestor manifest is still the package the import named. There
is no `PackageGraph`, no `ModuleTable`, and no `attribute_packages`/`derive_module_name`
enumeration of a package's files. The reasons this shape wins:

- Forward references cost nothing. Path-joining does not care whether BFS has reached the
  target yet, so the deferred-edge pass an earlier draft of this spec called for is not
  needed, and with it goes the ordering hazard of adding edges after `reject_cycles`.
- One pass, not two. Nothing has to know a package's full file list before the first import
  in it can resolve.
- No enumeration means no directory walk, so a package's private files are never read.

What the eager join gives up, and how it is paid for: nothing derives a module name from a
file path any more, so OQ2's naming rule has no derivation to fire at. It is enforced on the
import target's segments instead (see OQ2 above) -- same observable rule, checked one step
earlier.

Responsible for:

- `find_package_root(file: &Path) -> Option<PathBuf>`: the nearest ancestor directory
  holding a `sooth.pkg` (OQ2 manifest locality, so an inner manifest wins over an outer).
- `PackageSite { manifest_path, root, manifest }`: one package as resolution sees it, with
  `module_file(segments)` doing the path join. The extension is appended, not
  `set_extension`ed, so a `.` inside a segment survives.
- `ManifestCache`: `find_package_root` plus a `BTreeMap<PathBuf, Manifest>`, so each manifest
  is read and parsed at most once per build however many files it owns. Ordered, not hashed:
  phase 4's audit walk is seeded from the cache's key set and reports the first defect it
  finds, so a `HashMap` would make *which* of several defects a build names depend on hash
  order.
- `segment_defect(seg) -> Option<String>`: OQ2's naming rule, and the reason a segment
  breaks it.
- `resolve_import(...) -> Result<Option<PathBuf>, String>`: one import to the file it names.
  `Ok(None)` means the import adds no closure edge (the reserved `intrinsics` name, or a
  cross-package import recorded as unresolved). Owns every OQ4 located diagnostic except the
  duplicate qualifier, which fires where `import_map` is built (`driver.rs`).
- `UnresolvedImport { importer_pkg: String, importer_manifest: PathBuf, importer: PathBuf,
  pkg: String, module: Vec<String>, span: Span, kind: UnresolvedKind }` with
  `UnresolvedKind { MissingDepends, PrivateModule }`. Failure modes A and C are
  import-triggered -- their wording (OQ4-A, OQ4-C) names the specific offending `import:` --
  so `check_package_graph` needs to know which cross-package imports were attempted and how
  each failed. Resolution pushes one of these and adds no edge, rather than raising the
  diagnostic itself.

**Phase 4's inputs (changed by the above; read this before starting phase 4).**

- `check_package_graph` no longer has a `PackageGraph` to take. Its inputs are the closure's
  `Vec<UnresolvedImport>` and a `ManifestCache` (which by then holds every manifest the walk
  touched): `check_package_graph(manifests: &mut ManifestCache, unresolved:
  &[UnresolvedImport]) -> Result<(), String>`. The importer's manifest path is on each
  `UnresolvedImport`, so OQ4-A and OQ4-C need no other lookup.
- The layer check and the `depends:` name-mismatch check are manifest-triggered, and nothing
  in phase 3 reads the manifest of a `depends:` target that no import names -- the cache is
  populated by resolution, which only loads a dependency's manifest when an import reaches
  for it. Phase 4 must therefore walk `depends:` entries and load their manifests itself,
  starting from the entry file's package. Golden 2b (a higher-layer `depends:` with no
  import) is exactly the test that fails if it doesn't.
- The four checks are otherwise unchanged:
  - Missing `depends:` for a cross-package import (OQ4-A): each `MissingDepends` entry.
  - Public-module violation (OQ4-C): each `PrivateModule` entry.
  - Layer violation (OQ4-B): every `depends:` entry of every reachable manifest, whether or
    not anything imports across it.
  - `depends:` name mismatch: an entry naming `foo` where `foo`'s manifest declares
    `package: bar`. Diagnostic:

    ```
    error: `depends:` entry names `<foo>` at line L, col C in <manifest>:
      that package declares `package: <bar>` -- rename the entry to match
    ```

**Audit order.** The manifest walk (layer, name mismatch) runs to completion before any
`unresolved` entry is reported, so a layer violation anywhere in the reachable graph
masks an OQ4-A/OQ4-C import error. That is the intended order: a `depends:` entry that is
both an unimported layer violation and the source of an import that cannot resolve has
the layer violation as its root cause, and reporting the manifest defect first is what
makes Golden 2 and Golden 2b agree regardless of whether the fixture's `import:` line is
present. The manifest walk itself is BFS, seeded from the cache's key order (see
`ManifestCache` above) and then declaration order within each manifest, so the defect a
build names is a function of the source tree alone.

`check_package_graph` (scheduled after discovery, before `assemble_module`) is the only
place that turns a recorded `UnresolvedImport` into OQ4-A/OQ4-C text. Nothing before it can
raise either error, so a mutation test against its wording exercises a reachable path rather
than a guard whose message is also produced earlier in the walk. (Failure mode D is
different: a locate failure, not an audit failure, raised inline during resolution -- see
OQ4.)

Unit tests in `src/packages.rs` under `#[cfg(test)] mod tests`:

- `find_package_root_no_manifest_returns_none`: a file with no ancestor `sooth.pkg`.
- `find_package_root_nested_manifest_inner_wins`: OQ2 manifest locality, the inner of two
  nesting manifests owns the file beneath it.
- `module_file_joins_segments_under_the_root`: `text::ascii` is `<root>/text/ascii.sth`.
- `module_file_appends_extension_keeping_a_dotted_segment`: `ascii.io` is `ascii.io.sth`,
  not `ascii.sth`. Guards the append-vs-`set_extension` choice, which is silent otherwise.
- `module_segment_single_word_is_ok`: the segments that lex as one `Token::Word` are legal,
  `ascii.io` among them (OQ2's `.` ruling).
- `module_segment_non_word_is_rejected`: table-driven over the cases that pass a raw
  character check but fail the single-`Token::Word` rule -- `\` (a comment marker), `42`
  (`Token::Int`), `3.5` (`Token::Float`) -- plus the multi-token cases (`my file`, `a;b`,
  `(`, a quoted string).
- `module_segment_colon_is_rejected`: `:` lexes inside an ordinary word, so only the
  explicit rule excludes it.
- `module_segment_star_is_rejected`: a bare `*` likewise, reserved for the wildcard target.

The three segment tests above are mutation-tested by deleting their clause; the two
end-to-end witnesses that the rule is *reached* (with the offending file actually present,
so a not-found error cannot stand in for the naming rule) are
`import_target_non_word_segment_is_error` and `import_target_star_segment_is_error` in
`src/driver.rs`, mutation-tested by deleting the `check_module_name` call.

Phase 4's own unit tests, unchanged from this spec's original list except that they run
against `check_package_graph`'s revised inputs (above), not a `PackageGraph`:

- `check_package_graph_missing_depends_is_error`: consumer importing `pkg::mod` with no
  `depends:` for `pkg` produces a located error matching the OQ4-A wording. Test must pin
  the exact message substring including line/col and both package names. Mutation-test by
  deleting the check and verifying the test fails.
- `check_package_graph_private_module_is_error`: consumer importing `pkg::private` where
  `private` is absent from `pkg`'s `module:` list produces a located error matching OQ4-C
  wording. Pin exact message; mutation-test required.
- `check_package_graph_layer_violation_is_error`: `core`-layer package `depends:` on
  `hosted`-layer package produces a located error matching OQ4-B wording. Pin exact message
  including both layer names; mutation-test required.
- `check_package_graph_layer_equal_is_ok`: two `core`-layer packages depending on each
  other is legal. Not a guard-deletion test: deleting the layer check entirely still leaves
  this passing. Its only real mutation is the comparison operator (`>` flipped to `>=`);
  state that as the mutation to run, rather than treating it like the other three
  `check_package_graph_*` tests above.
- `check_package_graph_depends_name_mismatch_is_error`: a `depends:` entry naming `foo`
  where `foo`'s manifest declares `package: bar` is a located error naming both. Pin exact
  message; mutation-test required.

### Changes to `src/driver.rs` and `src/repl.rs`

**`src/repl.rs` is a compile-breaking consumer of `import.path`, `import.qualifier`, and
`import.selective`** (line 1764 for `.path`; lines 1806, 1810, 1819, 1833, 1844, 1875 for
`.qualifier`/`.selective`) and also generates old-order import source in a test helper
(line 4018). All sites must be updated in the same commit as `parser.rs` and `ast.rs` (see
Phase 3 below).

**REPL behaviour under module-name imports.** A module-name import at the REPL is REJECTED
with a located error in S1a:

```
error: module-name import at line L, col C in <repl>:
  REPL imports resolve against the user-level manifest, which is S1b's work
  use a quoted-path import for now, or add `--manifest` support (S1b)
```

Rationale: the REPL resolves against the user-level manifest (S1b's work); wiring that
resolution into `assemble_module` is S1b's job. Anything not wired into `assemble_module`
is unenforced at the REPL, so an explicit rejection is required rather than silent
fall-through.

Test: `repl_module_name_import_is_rejected` in `src/repl.rs` tests, asserting the exact
diagnostic wording above.

**`discover_closure` extension for module-name imports.**

Today `discover_closure` resolves each `import:` by its `imp.path` field (a quoted file
path). After this slice, `Import` gains a variant distinguishing a quoted-path import from
a module-name import. Quoted-path imports are deleted from the language for files inside a
package (S2 completes the corpus migration; this slice is the first step: files inside a
package that still carry quoted-path imports will fail the attribution pass with a located
error). The resolution path for module names in `discover_closure`:

1. The reserved name `intrinsics` is matched first, before any anchor-based lookup: a
   single-segment Dependency-anchored target spelled `intrinsics` resolves to the
   compiler-provided module and adds no closure edge (it has no file). It is never a
   `depends:` lookup, so it cannot raise the missing-`depends:` error, consistent with the
   F6 ruling that no `depends:` entry is required or accepted for it. `self::intrinsics` is
   not the reserved name and resolves like any other own-package module, which normally
   means a located not-found error.
2. Otherwise the `Import`'s `ImportAnchor` distinguishes SelfPackage (own package, `self::`
   prefix) from Dependency (bare first segment names a `depends:` entry). Resolution is by
   anchor, not inference.
3. For SelfPackage: path-join the importer's package root with `ModuleName::segments`,
   then re-check the resulting file's own nearest-ancestor manifest (D2, below).
4. For Dependency: look up the first segment in the current package's `depends:` entries.
   If absent, resolution stops here and records an `UnresolvedImport { kind:
   MissingDepends, .. }` (below) rather than raising OQ4-A itself; there is no path to
   join to, so this is the one point where resolution's own bookkeeping, not a diagnostic,
   is the visible effect. If present, resolve the entry's path to the dependency's manifest
   (a path with no manifest at it is its own located error, see OQ4) and path-join the
   dependency's root with the remaining segments. The join is not filtered by the
   dependency's `module:` public list -- a private file still physically exists and still
   needs to be locatable here. If nothing is at the joined path, that is failure mode D1
   ("no module `<module>`"), raised inline, same as the SelfPackage case. If the file
   exists but the module is not in the dependency's `module:` public list, resolution
   records an `UnresolvedImport { kind: PrivateModule, .. }` (OQ4-C is import-triggered, an
   audit check, not a locate failure). If the module is public, re-check that the file's
   own nearest-ancestor manifest is still that same dependency manifest; if a nested inner
   manifest claims the file first, that is failure mode D2, raised inline with its own
   wording (see OQ4).

The driver owns `Closure` and walks it, handing `packages::resolve_import` one `Import` at
a time along with the importer's path and package, and writing the returned resolutions
back into the closure. `packages.rs` has no `use crate::driver` dependency.

**`discover_closure` stays single-pass.**

The walk is BFS from the entry, as before. Module-name resolution needs to know which
package a file belongs to before its imports can be resolved, and that means reading a
manifest: when the walk reaches a file, its nearest ancestor manifest is located from its
canonical path and loaded through `ManifestCache`, so each manifest is parsed at most once
per build however many files it owns.

No second pass and no deferred edges: path-joining does not care whether BFS has reached
the target yet, so a forward reference to a sibling inside the same package resolves the
first time it is seen. Every edge is therefore in the graph before `reject_cycles` runs,
which removes the ordering hazard a deferred pass would have introduced (a cycle existing
only in deferred edges goes undetected if those edges land after the check).

Path-joining alone is not sufficient: a package may nest another package inside it
(manifest locality, OQ2), so after joining, resolution re-checks that the candidate file's
own nearest-ancestor manifest is still the manifest the import named. If an inner manifest
claims the file first, the target does not name a module of that package at all; resolution
raises failure mode D2 inline, naming the inner manifest that actually owns the file (see
OQ4), rather than silently resolving across the boundary. This is what keeps `self::` from
reaching into a nested package's private modules or around its layer check.

**Interaction with `assemble_module`.** `assemble_module` receives a `Closure` whose
`import_targets` are already resolved to module indices. No change to `assemble_module` is
required for this slice's package-graph logic; `check_package_graph` runs between
`discover_closure` and `assemble_module` and produces errors before assembly begins. Its
call site is a phase 4 change, not phase 3: phase 3 only extends `discover_closure` and
`Closure` (adding the `unresolved_imports: Vec<packages::UnresolvedImport>` field described
under `src/packages.rs` above); phase 4 adds the call to `check_package_graph` in the
orchestration function that already holds the closure and the walk's `ManifestCache` (see
"Phase 4's inputs" under `src/packages.rs`), so nothing in phase 3 references a function
that does not exist yet.

**Quoted-path imports inside a package.** A file inside a package (has an ancestor
manifest) that uses a quoted-path `import:` is a located error:

```
error: quoted-path import at line L, col C in <file>:
  file is in package `<pkg>`: use a module name (`self::<name>` or `<pkg>::<name>`) instead
```

Files with no ancestor manifest (S1b territory) retain quoted-path import support for this
slice; S2 removes it everywhere.

**Duplicate-qualifier diagnostic.** `src/driver.rs:212` does a bare `import_map
.insert` with no duplicate check. After this slice, inserting a qualifier that already
exists in the same file is a located error at the SECOND import, naming both:

```
error: duplicate import qualifier `<q>` at line L, col C in <file>:
  qualifier `<q>` was first bound at line L2, col C2
```

No shadowing, no precedence: the second binding is always the error. This fires for both
explicit qualifiers and defaulted-last-segment qualifiers, since both produce a concrete
binding at parse time.

Unit tests in `src/driver.rs` under `#[cfg(test)] mod tests`:

- `discover_closure_intra_package_forward_reference_resolves`: a file with a forward
  reference to a sibling not yet seen in BFS order resolves on the spot, and every edge is
  in the graph before `reject_cycles` runs.
- `discover_closure_manifest_cache_reads_once`: two files in the same package trigger only
  one `sooth.pkg` file read (verify by counting `fs::read_to_string` calls or wrapping IO).
- `discover_closure_inner_manifest_wins`: a file closer to an inner manifest is attributed
  to the inner package, not the outer one. Mutation-test by inverting the "nearest ancestor"
  walk and confirming the test fails.
- `discover_closure_quoted_path_inside_package_is_error`: a package file with a quoted-path
  import produces the diagnostic above. Pin exact wording; mutation-test required.
- `driver_duplicate_import_qualifier_is_error`: two imports in the same file that resolve
  to the same qualifier (e.g. `self::text::ascii` and `other::ascii` both defaulting to
  `ascii`) produce the duplicate-qualifier error at the second import. Pin exact message
  including both line/col values; mutation-test required.
- `self_anchored_import_into_nested_package_is_error`: a `self::` import path-joins into a
  file that actually belongs to a nested inner package (manifest locality). Pin the located
  error naming the boundary crossed; mutation-test by deleting the nearest-ancestor re-check
  and confirming the test fails.
- `dependency_anchored_import_into_nested_package_is_error`: the symmetric case for a
  Dependency anchor, where the dependency's own path-joined module lands inside one of the
  dependency's own nested inner packages.
- `self_intrinsics_is_not_the_reserved_name`: `self::intrinsics` with no `intrinsics.sth` in
  the importer's package produces the failure-mode-D1 located error naming the package, the
  module, and the path tried, distinct from the F6 reserved-name fast path.
- `self_import_of_non_public_module_is_ok`: a `self::` import of a module absent from the
  package's own `module:` list resolves without error, proving `module:` visibility is
  never consulted for a SelfPackage anchor. Not mutation-tested like the tests above it:
  by design, nothing in the SelfPackage resolution path ever reads `module:` at all, so
  there is no guard to delete. This is a regression fence against a future accidental
  coupling, not a killed-mutant guard; the description says so rather than implying
  otherwise.
- `resolve_bare_package_name_no_module_is_error`: `import: core ;` (no segments past the
  package name) produces the "names a package, not a module" located error rather than a
  missing-`depends:` error, even when `core` has no `depends:` entry.
- `import_target_non_word_segment_is_error` and `import_target_star_segment_is_error`:
  OQ2's naming rule reached through a real import. The offending file (`42.sth`, `*.sth`)
  is written in the fixture, so a D1 not-found error cannot stand in for the naming rule.
  Mutation-test by deleting the `check_module_name` call.
- `module_import_outside_a_package_is_error`: a module-name target in a file with no
  ancestor manifest. Pin exact wording; mutation-test by returning `Ok(None)` in place of
  the error, which would otherwise leave the import silently unbound.
- `depends_entry_with_no_manifest_is_error`: a `depends:` path with no `sooth.pkg` at it is
  located to the entry in the declaring manifest, not to the import. Pin exact wording;
  mutation-test by replacing the message with the raw IO error.
- `resolve_intrinsics_precedes_depends_lookup`: `import: intrinsics * ;` in a package whose
  manifest has no `depends:` at all resolves without error and adds no closure edge,
  proving the reserved name is matched before the `depends:` lookup rather than falling
  through to the missing-`depends:` error. (Moved here from the `packages.rs` list: this
  needs `ImportBinding::Wildcard`, phase 3's parser output, and `discover_closure`'s
  reserved-name fast path, neither of which `packages.rs` has visibility into.)

### Changes to `src/parser.rs`, `src/ast.rs`, and corpus migration (Phase 3)

Phase boundaries must be compile-green boundaries. The new grammar puts the target FIRST,
so every existing import site breaks the moment the parser changes. Phase 3 must therefore
contain, in one compile-green commit: the `ast.rs` type changes below, the `parser.rs`
rewrite, ALL consumer updates (`src/driver.rs` lines 66, 91, 212, 213, 217; `src/repl.rs`
lines 1764, 1806, 1810, 1819, 1833, 1844, 1875; the `parser.rs` tests around lines
3708/3715/3777; and `src/repl.rs` line 4018, which GENERATES old-order source in a test
helper), AND the mechanical corpus migration of all existing import sites. The migration is
a pure syntactic transform on the quoted-path form: `import: q | a b | "path.sth" ;`
becomes `import: "path.sth" q | a b | ;`. It is scriptable, but not with a single naive
regex:

- The script needs two patterns, not one: `import: (\S+)(?:\s*\|[^|]*\|)?\s*"([^"]+)"` for
  the bare-quote form (`lib/*.sth`, `examples/*.sth`), and the same shape with `\"` in place
  of `"` for the backslash-escaped form, which is what the vast majority of real `src/` and
  `tests/` sites use (they are string literals inside `.rs` source).
- The script must anchor on the `import:` token itself, not on line start (`^`): several
  sites (e.g. `src/driver.rs`'s multi-line fixture strings) carry `import:` embedded
  mid-literal after an escaped `\n`, not at the start of a line.
- Measured directly (`grep -rn "import: " --include=*.rs src tests`, then `lib/*.sth` and
  `examples/*.sth`, with doc-comment lines, `//`-comment prose mentions, and the one
  keyword-literal check in `src/repl.rs` (`if w == "import:"`) excluded by hand): roughly
  125 real `import:` forms, not "~183" -- about 112 in `src/` and `tests/`, 13 in `lib/`
  and `examples/`. Re-measure before running the script rather than trusting this number;
  it is a snapshot, not a contract.

Seven sites beyond the field-access consumers above generate or assert OLD-order import
source as interpolated strings, so the corpus-migration regex cannot reach them; they need
their own edits in the same phase 3 commit:

- `src/check/word_families.rs:1035`: a user-facing diagnostic (the `drop`-visibility remedy)
  that teaches the old order (`` import: {qualifier} | {source} | "..." ``). Ships wrong
  guidance to a real user if left unchanged.
- `src/check/engine.rs:1606`: a test expectation hardcoding that same old-order remedy text.
- `tests/phase4_generics.rs:67` and `tests/phase4_slice10b.rs:63`: `format!("import:
  {qualifier} \"{}/lib/combinators.sth\" ;\n", ...)`.
- `tests/phase4_repl_imports.rs:66` and `:70`: `format!("import: {qualifier} \"{}\" ;",
  ...)` and `format!("import: {qualifier} | {names} | \"{}\" ;", ...)`.
- `tests/phase4_modules.rs:426` and `:430`: pin the exact old-order remedy text
  `` add `Res` to the import (`import: lib | Res | "..."`) `` produced by
  `word_families.rs:1035`; both need re-pinning once that message's wording changes.
  (`tests/phase4_slice10b.rs:322` and `:366` only assert the substring `"has not imported
  by name"`, not the old-order remedy text, so despite reading superficially similar, they
  need no change.)

Every `.qualifier` / `.selective` read in `src/driver.rs` and `src/repl.rs` becomes a match
on `ImportBinding` once `Qualified` and `Wildcard` are separate variants: `driver.rs:212`
and `:217` (inside the selective loop building `check::SelectiveName`), and `repl.rs:1810`,
`:1819`, `:1833`, `:1844`, `:1875`. Stated once, generally, rather than as seven bespoke
arms: the `Qualified` arm at each site keeps today's behaviour unchanged; the `Wildcard`
arm carries no qualifier and no selective list, so it is a no-op at every site that reads
those fields for its own per-name logic (`driver.rs:217`'s selective loop simply has
nothing to iterate; `repl.rs:1810`'s and `:1819`'s selective-collision/export checks have
nothing to check; `repl.rs:1844`'s splice has no qualifier-prefixed alias to install). The
two sites that print or key on the qualifier text itself are the one place with a visible
Wildcard-specific string: `driver.rs:212`'s `import_map.insert` inserts no qualifier-keyed
entry (there is no qualifier to key on), and `repl.rs:1833`'s
`writeln!(writer, "imported {}", import.qualifier)` prints `imported <target> (wildcard)`
instead. Every arm must exist and compile in this slice even though `Wildcard`'s visibility
*effect* is S2's job (OQ3): this only names the placeholder behaviour so each match is
exhaustive, it does not gate any names.

`Import` (in `ast.rs`) gains a `target` field distinguishing quoted-path from module-name
form, and the grammar itself reorders per OQ3: target first, then an optional qualifier,
then the optional selective list.

```rust
pub enum ImportAnchor {
    Dependency,   // bare first segment: import: core::cmp c ;
    SelfPackage,  // self:: prefix: import: self::text::ascii a ;
}

pub struct ModuleName {
    pub anchor: ImportAnchor,
    pub segments: Vec<String>,
}

pub enum ImportTarget {
    Path(String),         // today's quoted form, for manifest-less files only
    Module(ModuleName),   // resolved by packages.rs
}

pub enum ImportBinding {
    Qualified { qualifier: String, selective: Vec<(String, Span)> },
    Wildcard,
}
```

`selective` keeps its `Span` per name, not just the bare `String` a first pass at this type
might reach for: `driver.rs:213-220` destructures that span into `check::SelectiveName.span`,
and `repl.rs:1806` and `:1875` destructure the same pair. That span is what locates every
R20/R21 selective-import diagnostic; dropping it silently de-locates all of them.

`parser::parse_import` is rewritten for the new token order: `<target> [<qualifier>]
[ | <name>... | ] ;`, plus S2's wildcard shape `<target> * ;`. It parses the target first
(a quoted string for the still-supported manifest-less path form, or an optional `self::`
prefix followed by a `::` -joined identifier sequence for a module name; the presence of
`self::` sets `ImportAnchor::SelfPackage`, absence sets `ImportAnchor::Dependency`).
After the target, it peeks: a `Pipe` token means no explicit qualifier, jump straight to the
selective list; the literal word `*` followed immediately by `Semicolon` (no `Pipe`) builds
`ImportBinding::Wildcard`; any other `Word` token is an explicit qualifier, consumed and
then checked for a following `Pipe`; a `Semicolon` alone means the qualifier defaults. The
default qualifier is computed immediately: the last element of `ModuleName::segments` for a
module target, or the file stem for a quoted-path target. The result is always an
`ImportBinding::Qualified { qualifier, selective }` or `ImportBinding::Wildcard`, with no
optionality leaks past the parser. Setting `ImportBinding::Wildcard`'s effect (gating which
names are visible) is S2's job; this slice only makes the token parse and land on `Import`
unambiguously.

`parser::scan_imports` is updated to populate `ImportTarget` based on whether the leading
token is a quoted string (manifest-less path form) or a bare identifier / `self::` prefixed
sequence (module-name form).

Note: the existing `import_and_export_forms_parse` (R6, `src/parser.rs:3708`) asserts the
old grammar (qualifier-first order) and must be REWRITTEN for the new order, not merely
added to. `malformed_import_missing_path_is_located_error` (R9, `src/parser.rs:3779`) does
not survive at all: it asserts `import: q ;` is a parse error, but under the new grammar
that is a legal Dependency-anchored import (target `q`, qualifier defaulted to `q`), so R9's
parse-error path has no remaining witness. It is DELETED, replaced by a new test on a shape
that is still malformed under the new grammar,
`malformed_import_missing_target_is_located_error`, asserting `import: ;` (nothing after
the keyword) is a located parse error. This keeps parse-error coverage for a malformed
`import:` rather than losing it silently.

Unit tests in `src/parser.rs` under `#[cfg(test)] mod tests`:

- `parse_import_explicit_qualifier_binds_given_name`: `import: core::cmp c ;` produces
  `ImportBinding::Qualified { qualifier: "c", selective: [] }`.
- `parse_import_omitted_qualifier_defaults_to_last_segment`: `import: core::cmp ;` produces
  `ImportBinding::Qualified { qualifier: "cmp", selective: [] }`.
- `parse_import_self_prefix_sets_self_anchor`: `import: self::text::ascii a ;` produces
  `ImportAnchor::SelfPackage` with segments `["text", "ascii"]` and qualifier `"a"`.
- `parse_import_omitted_qualifier_self_defaults_to_last_segment`: `import: self::text::ascii
  ;` produces qualifier `"ascii"`.
- `parse_import_bare_wildcard_builds_wildcard_variant`: `import: intrinsics * ;` produces
  `ImportBinding::Wildcard`.
- `parse_import_selective_list_star_is_literal_word`: `import: core::cmp | * | ;` parses
  `*` as an ordinary selective-import name; the result is `Qualified`, not `Wildcard`.
- `parse_import_selective_with_explicit_qualifier_ok`: `import: core::text s | split trim |
  ;` produces `Qualified { qualifier: "s", selective: ["split", "trim"] }`.
- `parse_import_quoted_path_target_parses`: the manifest-less quoted-path form parses with
  the reordered grammar, path in first position. Note: the path token MOVES to first
  position; the previous grammar had it last.
- `malformed_import_missing_target_is_located_error`: `import: ;` (nothing after the
  keyword) is a located parse error. Replaces R9's
  `malformed_import_missing_path_is_located_error`, which asserted `import: q ;` was a
  parse error; that shape is a legal import under the new grammar.

Duplicate-qualifier detection is in `src/driver.rs` (where `import_map.insert` happens),
not in the parser. See the driver.rs unit-test list below.

### Growth-structure check (per CLAUDE.md)

The manifest-parsing and package-attribution logic is genuinely new responsibility with
distinct dependencies from `driver.rs`'s orchestration. Two new files are warranted:

- `src/manifest.rs`: manifest data type and parser. Depends on `lexer`, `ast::Span`.
- `src/packages.rs`: package boundaries, module-name resolution, the naming rule, and the
  graph checks. Depends on `manifest`, `ast`, `lexer`. No dependency on `driver` (the
  driver hands one `Import` in and gets a path back), so it is unit-testable without
  constructing a `Closure`.

`driver.rs` imports both, and keeps only what owns the `Closure`: the BFS walk, cycle
rejection, assembly, and build/run orchestration. Re-checking CLAUDE.md's split signals at
phase 3's exit is what put resolution here rather than leaving it in `driver.rs`, where it
was first written: with it there, `driver.rs` did discovery *and* attribution *and* manifest
IO *and* assembly *and* process orchestration, with the import divergence (`manifest`,
`lexer`, `packages` against `Command`, `ExitStatus`) and the never-call-each-other function
clusters to match. No further split is needed at this slice's scope.

---

## Golden tests

Five golden tests, added to `tests/` (or as `#[test]` with fixture source strings in a
`tests/` file, matching existing golden pattern). Every error golden must pin the exact
diagnostic message substring including line/col and both relevant names (package names,
layer values, module names). An assertion of `result.is_err()` alone is a placebo; this
project has shipped placebo tests five times. Mutation-test each guard by deleting the
checked code and confirming the test fails.

**Entry point: through the driver, not through `check_package_graph`.** Goldens 2, 2b, 3
and 4 must reach the check via `driver::build` (or `driver::discover_closure`) on a fixture
tree on disk, never by calling `packages::check_package_graph` directly. Phase 4's unit
tests all call it directly, which leaves the single call site in `discover_closure`
unguarded: deleting that line passes the whole suite. These four goldens are the only tests
that can close that hole, so the mutation test for them is deleting the
`packages::check_package_graph(...)` call in `discover_closure` -- each of the four must
fail with it gone, in addition to failing when the individual check it pins is deleted.

**Golden 1: cross-package build succeeds.**  
A minimal two-package tree: a `core` package with one public module (`cmp`) and a consumer
`app` package that imports `cmp::lt` via `import: core::cmp c ;`. The consumer's `main`
calls `c::lt`. The build produces a binary; the binary runs and exits cleanly. `app`'s
manifest states `layer: hosted` explicitly (`layer:` is mandatory; a top-level application
package with no consumers of its own is the natural fit for the top layer).

Test name: `cross_package_import_public_module_builds`.

**Golden 2: layer violation is a located error.**  
A `core`-layer package with a `depends: app path "../app" ;` in its manifest; the `app`
package has `layer: hosted`. The layer check (OQ4-B) is manifest-triggered, matching the
exit criterion's wording ("a package that `depends:` on a higher-layer package is a
located build error") and `check_package_graph`'s own description above ("walks every
`depends:` entry ... directly"): it fires from the `depends:` declaration alone, whether or
not anything in the package actually imports from `app`. The fixture therefore does not
need an import to trigger the check; one may or may not be present. The build fails with
the OQ4-B error message; the test pins the substring `package \`core\` is layer \`core\`
but depends on \`app\` which is layer \`hosted\`` including both layer values.

Test name: `layer_violation_core_depends_on_hosted_is_error`.

**Golden 2b: an unimported higher-layer `depends:` is still a located error.** Pins the
manifest-triggered choice above against its alternative: the same fixture as Golden 2 but
with the `import: app::util u ;` line deleted (the `depends: app path "../app" ;` line
stays) still fails with the identical OQ4-B message. If this test and Golden 2 ever
disagree, the layer check has silently become import-triggered.

Test name: `layer_violation_fires_without_an_import`.

Both Golden 2 tests depend on phase 4 having walked `depends:` entries and loaded their
manifests itself: resolution only loads a dependency's manifest when an import reaches for
one, so an unimported `depends:` target's `layer:` is otherwise never read. See "Phase 4's
inputs" under `src/packages.rs`.

**Golden 3: private module is a located error.**  
A consumer importing `core::detail` where `core`'s manifest lists `module: cmp ;` but not
`detail`. The build fails with the OQ4-C error message; the test pins the substring
`module \`detail\` is not in \`core\`'s public \`module:\` list` including both names.

Test name: `cross_package_import_private_module_is_error`.

**Golden 4: missing `depends:` is a located error.**  
A consumer package `app` importing `collections::vec` with no `depends: collections ...` in
its manifest. The test pins the substring `package \`app\` has no \`depends:\` entry for
\`collections\`` (both package names, in the same substring) and separately asserts the
error text contains the offending import's `line 1, col 1`(or whatever line/col the
fixture's single import actually sits at) -- the location prefix (`at line L, col C in
<importer>`) is a different part of the same message, not inside the two-line pinned
substring, so the test checks it as its own assertion rather than folding it into one
quoted string.

Test name: `cross_package_import_no_depends_is_error`.

---

## Exit criteria

- A program builds against a dependency's module named as `pkg::module`.
- A package-private module (absent from `module:`) is unnameable from outside the package;
  the attempt is a located error naming both packages and the missing module.
- A package that `depends:` on a higher-layer package is a located build error naming both
  packages and both layers.
- All five golden tests pass.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.

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
