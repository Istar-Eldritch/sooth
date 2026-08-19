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
module: text cmp ;
```

**A module is a file, and its name derives from its path within the package.**
`text/ascii.sth` is the module `text::ascii`; nesting is naming, with no separate mechanism
behind it. A filename must be a legal module segment, which costs nothing since word names
already admit `-`. **Discovery and visibility are separate concerns**: files are still
discovered by walking the import graph, exactly as they are today, and the manifest's
`module:` list adds nothing to that. What it does is name the modules reachable from
*outside* the package, and its lines accumulate exactly as `export:`'s do, since a
manifest's `module:` is to a package what a file's `export:` is to a file. An undeclared
module is package-private: importable by name from its siblings, unnameable by a consumer.
A package's public surface is normally a hub or two rather than every file, so the list
stays short by construction, and a package that finds itself listing many modules is one
that wants a hub.

**Imports name modules, and that is the only form.** There is no path-based import. The
target comes first; the qualifier is optional and, when omitted, defaults to the module's
last segment:

```
import: text::ascii a ;               \ same package, by path-derived name, qualifier a
import: core::cmp ;                  \ another package, qualifier defaults to cmp
import: intrinsics i | * | ;         \ every exported name, unqualified
import: core::text s | split trim | ; \ two names unqualified, plus the qualifier
```

The qualifier is always bound and is always a single segment, so use sites stay `a::decode`
regardless of how deeply the module is nested; only its spelling in source is optional. A
path-based import would bake a dependency's internal layout into every consumer, so moving
a file inside a dependency would break them; naming modules is what stops that. Keeping the
quoted form for the manifest-less case would leave a second resolution rule alive for one
consumer, so it is deleted from the language and the manifest-less case is answered below
instead.

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
`fill`, `len`) is reachable only through `import: intrinsics | * | ;`. It is a
compiler-provided module, not a package with sources, so `intrinsics` is one reserved name
resolved without a path. The table itself does not move, and `has_self_tail_call` keeps
using it unchanged: only *visibility* is gated. `>`-prefixed conversions are claimed by
prefix rather than by the table and are gated by the same rule.

**Layers are checked.** A package may not depend on one in a higher layer, so `core`
depending on `alloc` is a located build error rather than a code-review observation, and
DESIGN.md's "tag every stdlib word with the layer it needs" becomes a field with a rule
behind it. Phase 9 builds `fixed` / `alloc` / `hosted` as packages that must pass it, which
is what makes that phase's exit criteria mean anything.

**A manifest is optional, and resolution falls back three ways.** Highest priority: `sooth
build`/`run` take an explicit `--manifest <path>` naming the manifest to resolve against,
overriding discovery entirely; this is the escape hatch for a named entry file that is not
sitting inside its package's own tree (a test harness fixture written to a temp directory,
a one-off script pointed at a project without living in it). Failing that, a file's package
is the nearest ancestor manifest. Failing that, the **user-level manifest** at
`$XDG_CONFIG_HOME/sooth/global_sooth.pkg` supplies the `depends:` a scratch file resolves
against, which is the same manifest the REPL reads for a session. Failing *all three*, the
file is an **implicit anonymous package** with no dependencies: it can import `intrinsics`
and its own path-derived siblings, and naming any other package is a located error whose
remedy is a manifest, `--manifest`, or the user-level file. A scratch file therefore stays
frictionless without a second import form, and the old loophole (a manifest-less file
reaching past a package's `module:` list into a private module) is not policed but
unspellable.

**`--manifest` is explicit, so it doesn't reopen the reproducibility guarantee below it.**
An ancestor manifest and the user-level fallback are both *discovered*, which is why the
latter is barred inside a package; `--manifest` is *named on the command line*, so a CI
invocation that pins it is exactly as reproducible as pinning the entry file itself. This is
what answers dogfood finding F1's cost: the ~460 inline test fixtures can point at one
shared manifest via `--manifest` rather than the harness generating one per fixture.

**The fallback never applies inside a package.** A file with an ancestor manifest resolves
against that manifest and nothing else (an explicit `--manifest` may still override it,
since that is a deliberate act at the call site, not a discovered one), so a package's build
cannot depend on machine-local configuration. The same reasoning applies to the test corpus:
fixtures must carry, be given, or be pointed at an explicit manifest rather than inheriting
a developer's global one, or CI stops being reproducible.

**Exit:** no word resolves without an `import:`, including the intrinsics; a program builds
against a dependency's module named as `pkg::module`; a module the package does not declare
is unnameable from outside it, and a package not in `depends:` is unnameable at all; a
package declaring a lower layer than its dependency is a located build error.
**Dogfood:** `lib/` restructured as packages, with a `core` package whose hub re-exports a
curated subset of `intrinsics` and whose modules are the typed core, a collections package
consuming it by module name, every example and golden importing what it uses, and a
deliberate layer violation rejected.

## Slices

**Ruled by the paper dogfood in `P8/dogfood/`: manifests land first.** Deleting the
quoted-path form left single-mode imports with no spelling to migrate the corpus to before a
manifest exists — nothing can say `core::bool` without a `depends:` table to resolve `core`
against — so the reorder that dogfood finding F1 flagged as forced is now the slice order,
not a risk to rule on later. The alternative, reserving `core` as a compiler-known package
the way `intrinsics` is, was rejected: it re-privileges the standard library this phase
exists to de-privilege.

The split is still on whether a manifest is required, it's just S1 that needs one now.
Everything manifest-level (S1) has to exist before everything file-level (S2) can migrate
the corpus once, to its final form, instead of twice with a red suite in between. S1 itself
splits into **S1a** (the manifest and the checker/resolver work it drives) and **S1b** (the
CLI-level question of which manifest resolves a given invocation): they share the manifest
grammar and nothing else, no file and no checker pass, so bundling them would review a new
file format alongside `main.rs` argument parsing as if they were one concern.

**P8.S1a — Packages, manifests, path-derived module names, and the layer check.** The
manifest and its parser, package attribution over the discovered closure, module names
derived from paths, the public-module list, cross-package `pkg::module` resolution, and the
three checks (naming a module its package does not make public; naming a package no
`depends:` entry lists; depending on a higher layer). Lands first because S2 has nothing to
migrate the corpus to without it. Brief: `docs/roadmap/P8/slice1-brief.md`.
**Exit:** a program builds against a dependency's module named as `pkg::module`; a
package-private module is unnameable from outside; a layer violation is a located build
error.
**Dogfood:** `lib/`'s modules restructured as layered packages, with a deliberate violation
rejected.

**P8.S1b — The `--manifest` CLI flag and the fallback chain.** `sooth build`/`run` gain an
explicit `--manifest <path>` flag ranked above discovery, then the nearest ancestor
manifest, then the user-level manifest (`$XDG_CONFIG_HOME/sooth/global_sooth.pkg`, also read
by the REPL for a session), then an implicit anonymous package with no dependencies. This is
what lets S2's ~460-inline-test-fixture migration point at one shared manifest instead of
generating one per fixture. Brief: `docs/roadmap/P8/slice1b-brief.md`.
**Exit:** `sooth build entry.sth --manifest path/to/sooth.pkg` resolves against the named
manifest regardless of `entry.sth`'s own directory, unconditionally overriding an ancestor
manifest; a manifest-less, flag-less file resolves against the user-level manifest, then
falls back to an implicit anonymous package with no dependencies; each fallback tier's
failure names its own remedy.
**Dogfood:** the test harness pointing a temp-directory fixture at a shared manifest via
`--manifest`.

**P8.S2 — Single-mode imports, the intrinsics module, wildcard import, and re-export.**
Delete `parser::prelude_words` and its two injection sites, shrink the mangling exemption to
`main`/`drop`, gate `BUILTIN_WORDS` visibility on an `intrinsics` import, add the `| * |`
wildcard form and re-export through `export:`, split `lib/core.sth` into modules with a hub,
and migrate every `.sth` file and every inline test source to module names now that S1a
gives them something to name and S1b gives fixtures a manifest to name it against. `if`'s
locals lose their `if--` prefixes: bodies are checked in their own module's scope now, so
the hygiene hack the whole-program environment forced is gone for good.
Brief and probe results in `docs/roadmap/P8/slice2-brief.md`. Probing established that an
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

**Quoted-path imports.** Deleted rather than kept for the manifest-less case. Once every
file inside a package names modules, the path form has exactly one consumer left, and
carrying a second resolution rule for it costs more than the three-line manifest a scratch
file writes when it wants more than `intrinsics`. Deleting it also makes the
reach-past-the-module-list loophole unspellable rather than merely tolerated.

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
additive grammar extension to `depends:`, not a redesign. **A REPL exemption in the
compiler** stays declined, since that is the special case this phase deletes; the REPL gets
its session imports from the user-level manifest above, like any other file without an
ancestor manifest.
