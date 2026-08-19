[← ROADMAP](./ROADMAP.md)

### Phase 8 — Packages and modules  `[L]`  `[the foundation the library ecosystem is structured on]`

Phase 4 Slice 5 made a file a compilation unit: an `import:` binds a qualifier to another
file, resolution is relative to the importing file with an explicit `.sth` and no search
path, and encapsulation came with it (default private, a per-file `export:` list, and the
Elm-style split between exporting a type name and exporting its constructors). That is
enough for personal reuse and not enough to structure an ecosystem: there is no unit above
the file, nothing expresses a dependency between two bodies of code, the `core` / `fixed` /
`alloc` / `hosted` layering is a convention about which words someone put in which file
rather than a checked property, and a program's most-used words arrive without being asked
for.

## The model

**A package is a directory with a manifest.** The manifest declares the package's name, its
layer, its dependencies, and which modules it makes public:

```
package: core ;
layer: core ;
depends: intrinsics builtin ;
module: text ;
module: cmp ;
```

**A module is a file, and its name derives from its path within the package.**
`text/ascii.sth` is the module `text::ascii`; nesting is naming, with no separate mechanism
behind it. A filename must be a legal module segment, which costs nothing since word names
already admit `-`. **Discovery and visibility are separate concerns**: files are still
discovered by walking the import graph, exactly as they are today, and the manifest's
`module:` list adds nothing to that. What it does is name the modules reachable from
*outside* the package. An undeclared module is package-private: importable by name from
its siblings, unnameable by a consumer. A package's public surface is normally a hub or
two rather than every file, so the list stays short by construction, and a package that
finds itself listing many modules is one that wants a hub.

**Imports name modules.** A quoted path survives only for a file belonging to no package
(the manifest-less scratch case); anything inside a package is named:

```
import: a text::ascii ;              \ same package
import: cmp core::cmp ;              \ another package, via depends:
import: i | * | intrinsics ;         \ every exported name, unqualified
import: s | split trim | core::text ; \ two names unqualified, plus the qualifier
```

The qualifier is always bound and is always a single segment, so use sites stay `a::decode`
regardless of how deeply the module is nested. A path-based cross-package import would bake
the dependency's internal layout into every consumer, so moving a file inside a dependency
would break them; naming modules is what stops that.

**Modules re-export, so a package curates its own surface.** `export:` accepts a name the
file imported as readily as one it declared, which makes a hub module: `core` can import
`intrinsics` and re-export the subset it endorses, and `text.sth` can gather
`text::ascii`/`text::utf8` behind one public name. This is also the *only* renaming
mechanism, which is why the manifest carries no path table: a hub that re-exports under a
chosen name does the same job in Sooth source, where it is greppable and can be deprecated.
Wildcard re-export is not provided; a hub lists what it promises, so a package's public
surface stays enumerable for P8.S3's API description.

**Nesting names, it does not nest visibility.** Importing `core::text` brings nothing from
`core::text::utf8`, and no intermediate module exists unless it is declared. An aggregate is
a hub, per above, so the two features compose rather than overlap.

**The intrinsics are a module too.** `BUILTIN_WORDS` (`src/check/declarations.rs:63-110`,
40 names: the shuffles, the arithmetic, the `u`-prefixed comparisons, `branch`, `tag`, `.`,
`fill`, `len`) is reachable only through `import: i | * | intrinsics ;`. It is a
compiler-provided module, not a package with sources, so `intrinsics` is one reserved name
resolved without a path. The table itself does not move, and `has_self_tail_call` keeps
using it unchanged: only *visibility* is gated. `>`-prefixed conversions are claimed by
prefix rather than by the table and are gated by the same rule.

**Layers are checked.** A package may not depend on one in a higher layer, so `core`
depending on `alloc` is a located build error rather than a code-review observation, and
DESIGN.md's "tag every stdlib word with the layer it needs" becomes a field with a rule
behind it. Phase 9 builds `fixed` / `alloc` / `hosted` as packages that must pass it, which
is what makes that phase's exit criteria mean anything.

**A manifest is optional.** A bare `.sth` file with no manifest above it belongs to no
package and builds as it does today, quoted-path imports included. The known cost is that
such a file is unconstrained and can path-import into another package's private modules;
accepted, because these checks exist to keep declared packages honest rather than to
sandbox, and the frictionless scratch file is what the optional manifest is for.

**Exit:** no word resolves without an `import:`, including the intrinsics; a program builds
against a dependency's module named as `pkg::module`; a module the package does not declare
is unnameable from outside it, and a package not in `depends:` is unnameable at all; a
package declaring a lower layer than its dependency is a located build error.
**Dogfood:** `lib/` restructured as packages, with a `core` package whose hub re-exports a
curated subset of `intrinsics` and whose modules are the typed core, a collections package
consuming it by module name, every example and golden importing what it uses, and a
deliberate layer violation rejected.

## Slices

The split is on whether a manifest is required. Everything file-level lands together in S1,
because all of it is one corpus-wide migration and doing it in two passes means editing the
same ~500 sites twice with a red suite in between.

**P8.S1 — Single-mode imports, the intrinsics module, wildcard import, and re-export.**
Delete `parser::prelude_words` and its two injection sites, shrink the mangling exemption to
`main`/`drop`, gate `BUILTIN_WORDS` visibility on an `intrinsics` import, add the `| * |`
wildcard form and re-export through `export:`, split `lib/core.sth` into modules with a hub,
and migrate every `.sth` file and every inline test source. `if`'s locals lose their `if--`
prefixes: bodies are checked in their own module's scope now, so the hygiene hack the
whole-program environment forced is gone for good.
Brief and probe results in `docs/roadmap/P8/slice1-brief.md`. Probing established that an
imported inline combinator splices correctly (qualified and selective), that
self-tail-to-loop lowering survives an imported `if` at 5M iterations, and that an inline
poly word over imported comparisons monomorphizes at `i64` and `f64`. It also found the
mangling exemption is **load-bearing** rather than a bare-name convenience: a non-inline
polymorphic word can call the prelude's poly `<` and cannot call an imported one, so
deleting the prelude exposes the generic-calls-generic gap. No live corpus word is in that
shape, so the slice accepts the narrowing behind a located diagnostic rather than pulling a
P7 type-system fix into a packaging slice.
**Exit:** no word resolves without an `import:`; `is_prelude_word_name` and
`parser::prelude_words` are deleted; a hub module re-exports an imported word and a consumer
uses it; the corpus builds and every golden passes; a non-inline poly word calling an
imported poly word is a located error naming the caller, the callee, and the reason.
**Dogfood:** `examples/gcd.sth` and `factorial.sth` building with explicit imports, and a
`core` hub re-exporting a curated subset of `intrinsics`.

**P8.S2 — Packages, manifests, path-derived module names, and the layer check.** The
manifest and its parser, package attribution over the discovered closure, module names
derived from paths, the public-module list, cross-package `pkg::module` resolution, and the
three checks (naming a module its package does not make public; naming a package no
`depends:` entry lists; depending on a higher layer). Wants its own brief: the manifest-half
recon and the open questions are in `docs/roadmap/P8/slice1-brief.md`.
**Exit:** a program builds against a dependency's module named as `pkg::module`; a
package-private module is unnameable from outside; a layer violation is a located build
error.
**Dogfood:** `lib/`'s modules restructured as layered packages, with a deliberate violation
rejected.

**P8.S3 — The serialisable API description.** "Which words, types, and externs are public"
is answered by Phase 4 Slice 5's `export:` list plus the manifest's public-module list, and
answered where it had to be, since a type cannot hold an invariant while its generated
setters cross the boundary unchecked. What is left is one thing: a compiler pass that walks
the checked AST, filters to the exported declarations of public modules, and emits a file
listing every exported signature for the API diff to compare between versions. Bounding
that surface by the public-module list is what stops a package-private refactor churning
the diff. That is the remaining prerequisite in `docs/dependency-management.md`, and it is a
packaging concern (letting other people depend on you with enforced semver) rather than a
personal-reuse one, which is why it waited. Needs P7.S2 (statics) and P7.S3e (bounds),
since a global clause on an exported word is part of that word's exported signature.
**Exit:** a published package's API diff correctly classifies a PATCH/MINOR/MAJOR bump
across a two-file change.
**Dogfood:** `sooth publish --check` on a two-version bump of a small library, one that adds
a word (MINOR) and one that removes one (MAJOR).

## Declined and deferred, with reasons

**A Rust-style source-side `mod` declaration.** Declined. `mod foo;` exists in Rust as the
*discovery* mechanism, because Rust has no manifest-level listing of sources; Sooth
discovers files by walking the import graph and declares its public surface in the manifest,
so a source-side mount is a third mechanism for something already handled twice, and it buys
the `mod`/`use`/`pub use` confusion with it. The declare-versus-use separation it offers is
already present: the manifest declares, `import:` uses.

**Intra-package visibility levels** (a submodule visible only to its parent, Rust's private
`mod`). Deferred, and this is what declining `mod` actually costs. Sooth has two levels,
file (`export:`) and package (`module:`), with everything inside a package mutually
reachable. Revisit when a package is large enough that its internal structure needs
defending; none is.

**Manifest path tables** (`module: text::ascii "text/ascii.sth" ;`). Declined once hubs
existed: a hub re-exporting under a chosen name is already a renaming mechanism, and two
mechanisms for one job is the duplication this project avoids. The cost accepted with it is
that directory layout is semantically load-bearing inside a package, so renaming a file
renames a module. Public surfaces should therefore be hubs, behind which files can move
freely.

**Wildcard re-export** (`export: | * | ;`). Declined: a hub exists to curate, and an
implicit public surface is exactly what P8.S3 has to enumerate.

**Listing every module in the manifest.** Declined, which is a different thing from the
public list the manifest does carry. Discovery is the import-graph walk and needs no
listing; the `module:` entries are visibility only, so what gets written down is a
package's promises rather than its contents. That keeps the list proportional to the
public surface, which hubs keep small, and it bounds what P8.S3's API diff must treat as
public.

**A C-ABI export target** (emitting a library other programs can link) is codegen work
(symbol naming, calling convention, header generation) sharing nothing with dependency
resolution. It is also the prerequisite for **Rust↔Sooth FFI**, which a self-hosted compiler
module would need, so both wait.

**Semver enforcement itself** (the API diff and `sooth publish --check`) is tooling on top
of P8.S3's format, specified in `docs/dependency-management.md`. **Git dependencies** are an
additive grammar extension to `depends:`, not a redesign. **A user-level manifest**
(`$XDG_CONFIG_HOME/sooth`) is how the REPL eventually gets a session's imports; a REPL
exemption in the compiler is not, since that is the special case this phase deletes.
