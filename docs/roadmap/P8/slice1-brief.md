# Phase 8 Slice 1: packages, manifests, module naming, and the layer check (brief)

**Split from a single, now-stale brief.** This file used to cover both the manifest work
and single-mode imports, written before the S1/S2 swap and before wildcard import,
re-export, path-derived naming, and the deleted quoted-path form existed. It is split three
ways: this file (the packaging core: manifest, package attribution, module naming and
visibility, cross-package resolution, the layer check), `slice1b-brief.md` (the
`--manifest` CLI flag and the three-tier fallback), and `slice2-brief.md` (single-mode
imports, the intrinsics module, wildcard, re-export, the prelude deletion). The design is in
`../P8-packages-modules.md`; that document is authoritative over anything below it that
conflicts.

A package is a directory with a manifest. Today's import model (Phase 4 Slice 5,
`docs/roadmap/P4/`) makes a single `.sth` file a compilation unit and nothing coarser: the
whole transitive closure is discovered from one entry file and merged into one `Module`,
and there is no notion above that of "a body of code with a name, a layer, or a dependency
list." This slice adds that unit and gives `core`/`fixed`/`alloc`/`hosted` a checked
dependency-direction rule instead of a filing convention.

## Recon (measured against the built compiler, 2026-08-19, `main` at `c1a0883`)

1. **The import closure has no notion of a package boundary; it is one flat graph of
   files.** `driver::discover_closure` (`src/driver.rs:76-108`) walks `import:` edges from
   one entry file, resolves each relative to the importing file's own directory, and dedupes
   into one `Closure` of `FileNode`s — a dependency two directories up and a sibling file in
   the same directory are the same kind of edge, indistinguishable in the graph. Introducing
   packages does not require replacing this: a package boundary can be computed as a
   property *over* the existing closure (which node's ancestor directories contain a
   manifest) rather than a new resolution mechanism.

2. **`lib/core.sth` mixes compiler primitives with Sooth code, and only half of it can
   become a module.** `branch`, `tag`, and the six `u`-prefixed comparisons are entries in
   `BUILTIN_WORDS` (`src/check/declarations.rs:85-110`), dispatched ahead of any environment
   lookup — not Sooth source, so there is no module to name them. `bool`, `if`/`unless`, and
   the surface comparisons are ordinary Sooth words and split cleanly into modules of one
   `core` package. This slice's naming and visibility rules apply to that split; the split
   itself (and the intrinsics-import gate) is S2's work, per `slice2-brief.md`.

## Decisions (settled here, not reopened by the spec)

- **A package is a directory with a manifest at its root; the manifest is a Sooth-lexed
  declaration file (`sooth.pkg` or similar), not TOML/JSON/YAML.** Pulling in a foreign
  config format for one small file is exactly the kind of dependency this project's
  `no_std`/no-LLVM bias argues against paying for. The same lexer that tokenizes `.sth`
  source tokenizes the manifest; its grammar is a handful of declaration keywords
  (`package:`, `layer:`, `depends:`, `module:`), parsed by a small dedicated parser rather
  than routed through `parse_bodies`'s word-declaration loop.

- **A module is a file, and its name derives from its path within the package.**
  `text/ascii.sth` is `text::ascii`; nesting is naming, with no separate mechanism behind
  it. This is what makes cross-package references stable: a path-based cross-package import
  would bake a dependency's internal layout into every consumer, so moving a file inside a
  dependency would break its consumers, which defeats the boundary a package exists to
  draw.

- **A package declares which modules are public; that is its only visibility surface.**
  `module: text cmp ;` (accumulating, like `export:`) names the modules reachable from
  *outside* the package; an undeclared module is package-private, importable by its
  siblings and unnameable by a consumer. This mirrors `export:` one level up and means no
  file is privileged — there is no root or entry module. A package's public surface is
  normally a hub or two rather than every file, so the list stays short by construction,
  and this also gives P8.S3's API description its natural scope (public modules and their
  export lists, not every file in the closure).

- **Cross-package imports resolve `pkg::module` through the manifest; there is no other
  form.** `pkg` resolves through the importing package's `depends:`, `module` through that
  package's own `module:` declarations. The quoted-path form is deleted from the language
  entirely, not merely unused for this case — see the model doc's "Quoted-path imports"
  entry under Declined. Intra-package references also name modules (there is nothing left
  to gain from a second form once names derive from paths).

- **The layer and `depends:` checks run as a pass over the discovered closure.** Naming a
  package no `depends:` entry lists is an error; a package may not depend on one in a higher
  layer. Package membership is computed over the existing `Closure` by attributing each
  `FileNode` to its nearest ancestor manifest (recon finding 1), so the file-graph resolver
  is extended at the naming layer, not replaced.

- **Dependencies are source locations, not registry lookups.** A `depends:` entry names a
  path (later, a git URL + revision) directly, matching `docs/dependency-management.md`'s
  premise that Sooth has no separately-compiled artifact to publish and fetch — a
  dependency's sources join the same closure the consumer's own files do, via the same
  `discover_closure` walk, just seeded from more than one root.

## Open questions for the spec

- **Manifest grammar, exactly.** Field syntax for `package:`/`layer:`/`depends:`/`module:`,
  whether a dependency may be aliased locally or must be named by its own `package:` name
  (aliasing an import is declined per Phase 4 Slice 5, which argues for the latter), and
  whether `layer:` is a fixed four-value enum (`core`/`fixed`/`alloc`/`hosted`) checked
  against a hardcoded ordering or an open list ordered by some declared rule.
- **Where the manifest lives relative to a multi-file package's own files** — root of the
  directory tree the package's files are discovered under, and whether a package's files
  may nest in subdirectories at all (today's flat `lib/` has no subdirectories to test this
  against; the paper dogfood in `P8/dogfood/` does, and found no blocker, only naming
  questions).
- **Whether the local qualifier stays free in the module form.** Today's qualifier
  (`import: q "path.sth"`) is a locally chosen identifier unrelated to the target. Does
  `import: q core::cmp` keep that freedom, or is the qualifier fixed to the module's own
  declared name once the module has one?
- **Diagnostic wording and located-ness for the two new failure modes**: an import crossing
  a package boundary with no matching `depends:` entry, and a `depends:` entry naming a
  lower-layered package than the declaring one's own `layer:`. Both need a located error
  naming the two packages and (for the layer case) their declared layers, matching this
  project's existing diagnostic bar.
- **Whether `depends:` needs a version/revision field at all in this slice**, or whether
  that's cleanly deferred to `docs/dependency-management.md`'s later semver work (P8.S3)
  without leaving `depends:`'s grammar needing a breaking change to add one later.

## Out of scope

- The `--manifest` CLI flag, the user-level manifest fallback, and the implicit anonymous
  package are `slice1b-brief.md`, not here.
- Single-mode imports, the intrinsics module, wildcard import, re-export/hubs, and the
  prelude deletion are `slice2-brief.md`, not here.
- Semver enforcement, the serializable API description, and `sooth publish --check` stay
  P8.S3, per `docs/roadmap/P8-packages-modules.md`.
- Git-based dependency resolution (only a path-based `depends:` lands here); a git revision
  is an additive grammar extension to the same field, not a redesign.

## Sequencing

1. Design and implement the manifest parser.
2. Implement the package-boundary attribution pass over `Closure` (nearest ancestor
   manifest per file).
3. Implement `pkg::module` cross-package resolution against `depends:`/`module:`, and
   intra-package module-name resolution.
4. Implement the dependency-direction and missing-`depends:` checks as a validation pass
   run after `discover_closure`, before `assemble_module`.

## Exit

A program builds against a dependency's module named as `pkg::module`; a package-private
module is unnameable from outside; a package declaring a lower layer than its dependency is
a located build error.

## Ready to spec?

Recon is source-verified. The five open questions above are all syntax/wording, not shape:
none of them changes any decision recorded here, so none should block starting the spec —
they belong in it. Ready to spec once `slice1b-brief.md` exists alongside it, since S1's
exit criteria and dogfood both assume the CLI flag is available.
