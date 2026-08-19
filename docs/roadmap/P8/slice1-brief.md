# Phase 8 Slice 1: packages, manifests, and single-mode imports (brief)

The unit above the file. Today's import model (Phase 4 Slice 5, `docs/roadmap/P4/`) makes
a single `.sth` file a compilation unit and nothing coarser: `import: q "path.sth"` names
another file by a path resolved relative to the importer, the whole transitive closure is
discovered from one entry file and merged into one `Module`, and there is no notion above
that of "a body of code with a name, a version, a dependency list, or a declared layer."
This slice adds that unit — the **package** — as a directory with a manifest, gives
`core`/`fixed`/`alloc`/`hosted` a checked dependency-direction rule instead of a filing
convention, and deletes the compiler-baked prelude that every program silently depends on
today.

## Recon (measured against the built compiler, 2026-08-19, `main` at `c1a0883`)

1. **The import closure has no notion of a package boundary; it is one flat graph of
   files.** `driver::discover_closure` (`src/driver.rs:76-108`) walks `import:` edges from
   one entry file, resolves each relative to the importing file's own directory
   (`dir.join(&imp.path)`), canonicalizes, and dedupes into one `Closure` of `FileNode`s —
   a dependency two directories up and a sibling file in the same directory are the same
   kind of edge, indistinguishable in the graph. `assemble_module` (`:170-`) then merges
   every node into one `Module` and hands it to `resolve::mangle` to pull same-named decls
   apart by a per-file module id. Introducing packages does not require replacing any of
   this: a package boundary can be computed as a property *over* the existing closure (which
   node's ancestor directories contain a manifest) rather than a new resolution mechanism.

2. **The prelude is compiled into the binary, not read from disk at build time.**
   `parser::prelude_words` (`src/parser.rs:488-494`) does `include_str!("../lib/core.sth")`
   and lexes/parses it at Rust compile time; `parser::parse` (`:496-500`) unconditionally
   extends every parsed module's word list with it, and `driver::assemble_module` does the
   same for a multi-file closure (`driver.rs:301-304`). There is no per-file opt-out and no
   import path a program can take to *not* get these words — deleting the prelude means
   deleting both call sites, not adding a flag to them.

3. **The mangling exemption for prelude words is one function, sized to shrink.**
   `resolve::mangle` (`src/resolve.rs:26-31`) exempts `main`, `drop`, and
   `is_prelude_word_name` from the `__m{module}` suffix every other decl gets, so a prelude
   word stays reachable by its bare name from every module at once. `is_prelude_word_name`
   (`:52-67`) reads its list off `parser::prelude_words()` rather than a hand-maintained
   set, specifically so the exemption tracks whatever `lib/core.sth` contains. Deleting the
   prelude call sites (finding 2) makes this function's body dead; the exemption list
   shrinks to exactly `main`/`drop`, which is where `project_repl_bypasses_module_checks`
   already flagged the REPL's own bypass of this same list as a related gap.

4. **`lib/core.sth`'s own header already draws the split this slice needs.** Its comment
   names three compiler primitives (`branch`, `tag`, six `u`-prefixed comparisons) and says
   everything else — `bool`, `if`, `unless`, `=`/`<`/`>`/`<=`/`>=`/`<>` — is "written here,
   in Sooth," built on top. That is a two-package split waiting to happen: an intrinsics
   package holding only what the compiler actually special-cases, and a typed-core package
   depending on it that holds the rest. Splitting the file is mechanical once imports are
   single-mode; nothing in the checker or backend treats `lib/core.sth` as a single
   indivisible unit today (it's read once via `include_str!` and never referenced by path
   at runtime).

5. **The CLI takes a bare entry file, not a project root.** `main.rs:16-35` dispatches
   `build <file.sth>` / `run <file.sth>` straight to `driver::build`/`driver::run`, both
   typed `&Path` to a single file (`driver.rs:403`). There is no existing notion of "the
   current package" or "the workspace root" for a manifest to be discovered from.

## Decisions (settled here, not reopened by the spec)

- **A package is a directory with a manifest at its root; the manifest is a Sooth-lexed
  declaration file (`sooth.pkg` or similar), not TOML/JSON/YAML.** Pulling in a foreign
  config format for one small file (a name, a layer, a dependency list) is exactly the
  kind of dependency this project's `no_std`/no-LLVM bias argues against paying for. The
  same lexer that already tokenizes `.sth` source tokenizes the manifest; its own grammar
  is a handful of declaration keywords (`package:`, `layer:`, `depends:`), parsed by a
  small dedicated parser rather than routed through `parse_bodies`'s word-declaration loop.

- **The manifest does not replace `import:`; it constrains it.** An `import:` line keeps
  meaning exactly what it means today — a path, resolved relative to the importing file,
  with no search path. What the manifest adds is a *check* over the existing closure: for
  every import edge that crosses from one package's directory into another's, the target
  package must appear in the source package's `depends:` list, and the target's declared
  `layer:` must not be lower than the source's. An edge that stays inside one package (the
  overwhelmingly common case — most of `lib/`'s existing files importing each other) is
  unconstrained, exactly as it is today. This is why finding 1 matters: no new resolution
  mechanism, one new validation pass over `Closure` after `discover_closure` runs,
  attributing each `FileNode` to the nearest ancestor directory holding a manifest.

- **Dependencies are source locations, not registry lookups.** A `depends:` entry names a
  path (later, a git URL + revision) directly, matching `docs/dependency-management.md`'s
  premise that Sooth has no separately-compiled artifact to publish and fetch — the
  dependency's sources are discovered and merged into the same closure the consumer's own
  files are, via the same `discover_closure` walk, just seeded from more than one root.

- **A manifest is optional, not mandatory.** A bare `.sth` file with no manifest anywhere
  in its ancestor directories builds exactly as it does today: one package-less closure,
  no layer check performed because there is nothing declared to check. Forcing every
  `examples/*.sth` golden and every scratch file to carry a manifest is ceremony this
  project's craft bias argues against paying for something that has no cross-package
  dependency to declare. Manifests matter where the layer check actually has teeth: the
  stdlib packages Phase 9 builds, and any real multi-package program.

- **`is_prelude_word_name` and `parser::prelude_words` are deleted, not deprecated.**
  Every existing `.sth` file — every example, every golden, every `lib/` file that isn't
  the new intrinsics/typed-core split itself — gains explicit `import:` lines for what it
  uses. This is mechanical migration work, not a design question, and is scoped to this
  slice rather than left half-done: a corpus file that still resolves `if` through the
  deleted prelude is a build failure, and the fix is adding the import, not restoring the
  exemption.

- **Ordering: the prelude split does not need to wait on the manifest, and shouldn't.**
  The two were flagged in `docs/roadmap/P8-packages-modules.md` as having an open
  ordering question. Given the "manifest is optional" decision above, there's no real
  coupling: deleting the compiler-baked prelude and requiring explicit imports is a
  self-contained change to `parser.rs`/`resolve.rs`/every corpus file, checkable with
  `cargo test` and diffing the whole corpus's import lines, independent of whether a
  manifest parser exists yet. Building the prelude split first gives the manifest work a
  smaller, already-cleanly-layered `lib/core.sth` to point `depends:` at instead of one
  monolithic file.

## Open questions for the spec

- **Manifest grammar, exactly.** Field names and syntax for `package:`/`layer:`/
  `depends:` (is a dependency's local qualifier declared in the manifest, in the
  importing `import:` line, or derived from the dependency's own `package:` name?), and
  whether `layer:` is a fixed four-value enum (`core`/`fixed`/`alloc`/`hosted`) checked
  against a hardcoded ordering or an open list the checker orders lexically by some rule.
- **Where the manifest lives relative to a multi-file package's own files** — root of the
  directory tree the package's files are discovered under, and whether a package's files
  may nest in subdirectories at all (today's flat `lib/` has no subdirectories to test
  this against).
- **What names a cross-package qualifier in `import:`.** Today's qualifier (`import: q
  "path.sth"`) is just a locally chosen identifier with no relationship to the target
  file's own name. Does crossing a package boundary keep that (any local name, whatever
  the target package calls itself) or does the manifest's dependency list fix the
  qualifier a consumer must use?
- **Diagnostic wording and located-ness for the two new failure modes**: an import
  crossing a package boundary with no matching `depends:` entry, and a `depends:` entry
  naming a lower-layered package than the declaring one's own `layer:`. Both need a
  located error naming the two packages and (for the layer case) their declared layers,
  matching this project's existing diagnostic bar.
- **Whether `depends:` needs a version/revision field at all in this slice**, or whether
  that's cleanly deferred to `docs/dependency-management.md`'s later semver work (P8.S2)
  without leaving `depends:`'s grammar needing a breaking change to add one later.

## Out of scope

- Semver enforcement, the serializable API description, and `sooth publish --check`
  stay P8.S2, per `docs/roadmap/P8-packages-modules.md`.
- Git-based dependency resolution (only a path-based `depends:` lands here); a git
  revision is an additive grammar extension to the same field, not a redesign.
- Re-exports, import aliasing, wholesale unqualified import, and a `mod.sth`-style
  directory convention stay declined, per Phase 4 Slice 5 and DESIGN.md's Modules section.
- A manifest flag making a dependency's exports visible unqualified package-wide (the
  documented escape hatch for prelude ergonomics) is not built in this slice.
- A user-level manifest for the REPL (`$XDG_CONFIG_HOME/sooth`) is a REPL-only question,
  untouched here; the REPL's own import path (Slice 5b) is not otherwise touched by this
  slice beyond losing prelude injection alongside the native path.

## Sequencing

1. Delete the compiler-baked prelude (`parser::prelude_words`, both call sites, the
   `is_prelude_word_name` exemption) and migrate every corpus file to explicit imports.
   Split `lib/core.sth` into an intrinsics file and a typed-core file that imports it,
   per finding 4, as part of this same pass (the split is only meaningful once the
   consuming files already import explicitly).
2. Design and implement the manifest parser and the package-boundary attribution pass
   over `Closure`.
3. Implement the dependency-direction and missing-`depends:` checks as a validation pass
   run after `discover_closure`, before `assemble_module`.

## Exit

A program builds from a manifest against a package dependency it names; a package
declaring a lower layer than its dependency is a located build error; no word is visible
without an `import:`; `is_prelude_word_name` and `parser::prelude_words` are gone; every
example and golden program imports what it uses.

## Ready to spec?

Recon is source-verified, not inferred, for both halves (prelude deletion and package
boundary). The prelude-deletion half has no open design question left — it's sequencing
item 1 above and can go straight to a spec. The manifest half has four real open
questions (grammar, nesting, qualifier naming, diagnostic wording) that should be
answered in the spec itself rather than guessed at here, since none of them changes the
shape of *this* brief's decisions, only their concrete syntax.
