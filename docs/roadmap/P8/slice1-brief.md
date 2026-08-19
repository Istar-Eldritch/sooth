# Phase 8 Slice 1: packages, manifests, and single-mode imports (brief)

**Read the phase file first; parts of this brief are superseded.** The current model lives
in `docs/roadmap/P8-packages-modules.md`. What still stands here, and is why this file is
kept, is the **Recon** and **Resolved recon**: source-verified findings about the closure
walker, the prelude injection sites and mangling exemption, plus five compiled probes of the
prelude deletion. Those are unaffected by the design changes.

What changed after this brief was written:

- The phase is now three slices, in this order: **P8.S1** is everything manifest-level
  (packages, module names, the layer check); **P8.S2** is everything file-level (prelude
  deletion, the `intrinsics` module, wildcard import, re-export/hubs, the corpus migration);
  **P8.S3** is the API description. S1 lands first, reversing this brief's own original
  sequencing, because the paper dogfood (`P8/dogfood/README.md`, finding F1) found that S2
  has no manifest to resolve a module name against otherwise: deleting the quoted-path import
  form removed the only spelling S2 could have migrated the corpus to first.
- **Superseded: "the manifest does not replace `import:`, it constrains it."** A
  cross-package import names a module, so the manifest participates in resolution.
- **Superseded: the manifest's `module:` entries carry paths.** Module names derive from
  the file's path within the package; the manifest declares only which are public, since
  hubs already provide renaming and two mechanisms for one job is duplication.
- **Superseded: intrinsics stay ambient.** They are reachable only through an
  `intrinsics` import, which is what the wildcard form exists to make bearable.

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

4. **`lib/core.sth`'s own header already draws the split this slice needs, but it is a
   split into modules, not packages.** Its comment names three compiler primitives
   (`branch`, `tag`, six `u`-prefixed comparisons) and says everything else — `bool`, `if`,
   `unless`, `=`/`<`/`>`/`<=`/`>=`/`<>` — is "written here, in Sooth," built on top. Only
   the second half is Sooth code: the primitives are entries in `BUILTIN_WORDS`
   (`src/check/declarations.rs:85-110`), dispatched by `is_builtin_word_name` ahead of any
   environment lookup, so there is no intrinsics *package* to depend on and no dependency
   edge to draw. The intrinsics stay ambient (they are the language surface, not a library);
   what splits is the Sooth half, into modules of one `core` package. Splitting the file is
   mechanical once imports are single-mode; nothing in the checker or backend treats
   `lib/core.sth` as indivisible today (it's read once via `include_str!` and never
   referenced by path at runtime).

5. **The CLI takes a bare entry file, not a project root.** `main.rs:16-35` dispatches
   `build <file.sth>` / `run <file.sth>` straight to `driver::build`/`driver::run`, both
   typed `&Path` to a single file (`driver.rs:403`). There is no existing notion of "the
   current package" or "the workspace root" for a manifest to be discovered from.

## Resolved recon (probe-verified against the built compiler, 2026-08-19, `main` at

`e43df46`; probe files built, run, and deleted, `git status` and `cargo test` clean
afterward)

The prelude-deletion half was probed before speccing, because "every file just gains
`import:` lines" assumes the imported words behave like ordinary words, and they don't:
`if`/`unless` are row-typed inline combinators taking quotation parameters and
`=`/`<`/`>` are `'T: Copy Ord` inline words going through operator dispatch. Four of the
five probes confirmed the brief; one falsified a decision.

1. **An imported inline combinator splices correctly, qualified and selectively.** A
   copy of `if`/`unless` in a second module, exported and imported, produced correct
   branch selection through both `c::if2` and a selectively-imported bare `if2`.

2. **Self-tail-call-to-loop lowering survives an imported combinator.** A `gcd`-shaped
   `count` recursing 5,000,000 deep through an *imported* `if2` ran to completion with no
   stack growth, matching its prelude-`if` control. This was the largest suspected risk
   (INV-INLINE-COMBINATOR has the checker read the callee's body, and a spliced inline
   combinator is re-scoped under the callee's module) and it is not a risk.

3. **The corpus's actual shape works.** An `inline` poly word (the shape of
   `examples/poly_if.sth`'s `mymax`, `lib/arrays.sth`'s `sort`/`bin_search`,
   `lib/combinators.sth`'s `each`/`map`/`fold`/`filter`) calling an imported comparison
   *and* an imported `if` monomorphized and ran correctly at both `i64` and `f64`.

4. **Falsified: the prelude mangling exemption is load-bearing, not merely a bare-name
   convenience.** A **non-inline** poly word calling the prelude's poly `<`
   (`: mylt ( 'T: Copy Ord 'T -- bool ) < ;`) works; the identical word calling an
   imported copy fails with `` unknown word `lt2__m1` ``. Same-module non-inline
   poly-to-poly fails identically (`lt2__m0`), so the underlying defect is the
   already-known generic-calls-generic gap and the exemption is what has been hiding it.
   Deleting the prelude does not cause this bug; it exposes it.

5. **Blast radius of finding 4: no live corpus word, but the next one written.** Every
   poly word in `lib/` and `examples/` that uses a comparison or `if` is `inline`, so
   nothing in the tree breaks. The three non-inline poly words (`poly_borrow_first`'s
   `first`, `poly_borrow_setat`'s `setat`, and the paper-grammar `bin_search` in the
   untracked `lib/binary_search.sth`) use no comparison that compiles today — but
   `binary_search.sth`'s intended shape is exactly a non-inline poly word over a
   comparison, so this gap is the first thing a real `bin_search` hits. Separately, `if`
   in a non-inline poly body is already rejected outright by the P7.S3b-follow
   diagnostic, independent of imports.

## Decisions (settled here, not reopened by the spec)

- **A package is a directory with a manifest at its root; the manifest is a Sooth-lexed
  declaration file (`sooth.pkg` or similar), not TOML/JSON/YAML.** Pulling in a foreign
  config format for one small file (a name, a layer, a dependency list) is exactly the
  kind of dependency this project's `no_std`/no-LLVM bias argues against paying for. The
  same lexer that already tokenizes `.sth` source tokenizes the manifest; its own grammar
  is a handful of declaration keywords (`package:`, `layer:`, `depends:`), parsed by a
  small dedicated parser rather than routed through `parse_bodies`'s word-declaration loop.

- **A cross-package import names a module; an intra-package import stays a path.**
  (This reverses an earlier decision in this brief that the manifest would only *constrain*
  `import:` and never participate in resolution.) A quoted string keeps today's meaning
  exactly: a path relative to the importing file, no search path, staying within one
  package. An unquoted `pkg::module` resolves `pkg` through the importing package's
  `depends:` and `module` through that package's own `module:` declarations. The reason is
  that a path-based cross-package import bakes the dependency's internal file layout into
  every consumer, so moving a file inside a dependency breaks its consumers, which defeats
  the boundary a package exists to draw. This is not the declined search path: the location
  is written down in a manifest table and nothing is discovered by scanning.

- **A package declares the modules it exposes, and that is its export surface.** A
  `module: name "file.sth" ;` entry both names a module and makes it public; a file the
  manifest does not declare is package-private, reachable by path from inside the package
  and unnameable from outside it. This mirrors `export:` one level up (`export:` decides
  which names leave a file, `module:` which modules leave a package) and means no file is
  privileged: there is no root or entry module. It also gives P8.S3's API description its
  natural scope, the declared modules and their export lists, rather than every file in the
  closure.

- **The layer and `depends:` checks still run as a pass over the discovered closure.**
  Naming a package no `depends:` entry lists is an error, and a package may not depend on
  one in a higher layer. Per finding 1, package membership is computed over the existing
  `Closure` by attributing each `FileNode` to its nearest ancestor manifest, so the
  file-graph resolver is extended at the naming layer, not replaced.

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
  stdlib packages Phase 9 builds, and any real multi-package program. **The known cost:** a
  manifest-less file belongs to no package, so nothing constrains it and it can path-import
  into another package's undeclared files, bypassing the `module:` surface. Accepted: it is
  exactly today's trust level, and these checks exist to keep declared packages honest, not
  to sandbox. The alternative (closing a package's directory even to manifest-less
  consumers) costs the frictionless scratch file, which is the thing the optional manifest
  is for.

- **`is_prelude_word_name` and `parser::prelude_words` are deleted, not deprecated.**
  Every existing `.sth` file — every example, every golden, every `lib/` file that isn't
  the newly split `core` modules themselves — gains explicit `import:` lines for what it
  uses. A corpus file that still resolves `if` through the deleted prelude is a build
  failure, and the fix is adding the import, not restoring the exemption.

- **The migration is mechanical for the live corpus but is not purely mechanical, and the
  spec must rule on the difference** (resolved recon, finding 4). Deleting the exemption
  removes one capability that exists today: a *non-inline* polymorphic word may call the
  prelude's poly comparison and may not call an imported one. No word in the tree uses
  that capability, so nothing breaks on migration, but the spec must choose explicitly
  between two options rather than discovering this in review: (a) declare the
  generic-calls-generic fix a hard prerequisite of this slice, which moves work into P7
  and grows the slice, or (b) accept the narrowing for now, with a located diagnostic and
  a rejection test naming it, so the next author to write a non-inline poly `bin_search`
  gets an error that explains itself instead of `` unknown word `<__m1` ``. **(b) is the
  recommendation**: the capability has no current user, the diagnostic is small, and
  bundling a type-system fix into a packaging slice is how a slice stops being reviewable.
  A silent third option — leaving the exemption in place for comparisons only — is
  declined, since it keeps the hole this slice exists to close.

- **Ordering: the prelude split does not need to wait on the manifest, and shouldn't.**
  The two were flagged in `docs/roadmap/P8-packages-modules.md` as having an open
  ordering question. Given the "manifest is optional" decision above, there's no real
  coupling: deleting the compiler-baked prelude and requiring explicit imports is a
  self-contained change to `parser.rs`/`resolve.rs`/every corpus file, checkable with
  `cargo test` and diffing the whole corpus's import lines, independent of whether a
  manifest parser exists yet. Building the prelude split first gives the manifest work a
  smaller, already-cleanly-layered `lib/core.sth` to point `depends:` at instead of one
  monolithic file. The probes above were run specifically to test this independence claim,
  and it survived: nothing about importing core's words needs a manifest to exist.

## Open questions for the spec

- **Ruled: `--manifest` unconditionally overrides an ancestor manifest**, silently, with no
  separate conflict diagnostic — a named, deliberate act beats a discovered one, full stop.
  What still needs spec wording is the *ordinary* located error when the named manifest
  itself doesn't resolve the file's imports (a module or package the flag's manifest
  doesn't grant), which is the same shape as an ancestor-manifest failure today, not a new
  diagnostic category.
- **Manifest grammar, exactly.** Field names and syntax for `package:`/`layer:`/
  `depends:`/`module:`, whether a dependency may be aliased locally or must be named by its
  own `package:` name (aliasing an import is declined per Phase 4 Slice 5, which argues for
  the latter), and whether `layer:` is a fixed four-value enum
  (`core`/`fixed`/`alloc`/`hosted`) checked against a hardcoded ordering or an open list
  ordered by some declared rule.
- **Where the manifest lives relative to a multi-file package's own files** — root of the
  directory tree the package's files are discovered under, and whether a package's files
  may nest in subdirectories at all (today's flat `lib/` has no subdirectories to test
  this against).
- **Whether the local qualifier stays free in the module form.** Today's qualifier
  (`import: q "path.sth"`) is a locally chosen identifier unrelated to the target. Does
  `import: q core::cmp` keep that freedom, or is the qualifier fixed to the module's own
  declared name once the module has one?
- **Diagnostic wording and located-ness for the two new failure modes**: an import
  crossing a package boundary with no matching `depends:` entry, and a `depends:` entry
  naming a lower-layered package than the declaring one's own `layer:`. Both need a
  located error naming the two packages and (for the layer case) their declared layers,
  matching this project's existing diagnostic bar.
- **Whether `depends:` needs a version/revision field at all in this slice**, or whether
  that's cleanly deferred to `docs/dependency-management.md`'s later semver work (P8.S3)
  without leaving `depends:`'s grammar needing a breaking change to add one later.
- **The wording of the narrowing diagnostic**, if the spec takes option (b) above: a
  non-inline poly word calling an imported poly word needs a located error naming the
  caller, the callee, and the reason (a polymorphic callee is not yet reachable from a
  polymorphic body across a module boundary), not the raw `` unknown word `<__m1` ``
  that leaks the mangled name today.

## Out of scope

- Semver enforcement, the serializable API description, and `sooth publish --check`
  stay P8.S3, per `docs/roadmap/P8-packages-modules.md`.
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
   `is_prelude_word_name` exemption) and migrate every corpus file to explicit imports,
   adding the narrowing diagnostic and its rejection test. Split `lib/core.sth` into
   modules per recon finding 4 (the compiler's builtins stay ambient), as part of this same
   pass, since the split is only meaningful once the consuming files already import
   explicitly.
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

Recon is source-verified for both halves and probe-verified for the prelude-deletion
half, which is where the one falsified decision was found (resolved recon, finding 4:
the mangling exemption is load-bearing for non-inline poly callees, and the brief's
original "mechanical migration, not a design question" claim was wrong as written).
That is now a stated ruling with a recommendation, not a discovery left for review.

The prelude-deletion half is ready to spec, with the one ruling above to make explicit.
The manifest half has five open questions (grammar, nesting, qualifier naming, and the
two diagnostics) that belong in the spec rather than guessed at here, since none of them
changes the shape of this brief's decisions, only their concrete syntax.
