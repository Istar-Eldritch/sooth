[← ROADMAP](./ROADMAP.md)

### Phase 8 — Packages and modules  `[L]`  `[the foundation the library ecosystem is structured on]`

Phase 4 Slice 5 made a file a compilation unit: an `import:` binds a qualifier to another
file, resolution is relative to the importing file with an explicit `.sth` and no search
path, and encapsulation came with it (default private, a per-file `export:` list, and the
Elm-style split between exporting a type name and exporting its constructors). That is
enough for personal reuse and not enough to structure an ecosystem: there is no unit above
the file, nothing expresses a dependency between two bodies of code, and the `core` /
`fixed` / `alloc` / `hosted` layering is a convention about which words someone put in
which file rather than a checked property.

This phase adds the unit above the file and makes the layering enforceable. A **package**
is a directory with a manifest naming the package, its layer, and its dependencies.
Dependencies are **source-based**: a dependency resolves to a source location (a path, or
later a revision), and its sources are compiled into the program alongside the consumer's,
because Sooth compiles one unit with no per-crate overhead and so has no binary artifact to
resolve. There is deliberately no search path: a dependency's location is written down, not
discovered.

The manifest's **layer** field is what gives the layering teeth. A package may not depend
on a package in a higher layer, so `core` depending on `alloc` is a build error rather than
a code-review observation, and DESIGN.md's "tag every stdlib word with the layer it needs"
stops being discipline and becomes a field with a rule behind it. Phase 9 then builds
`fixed` / `alloc` / `hosted` as packages that have to pass that check, which is what makes
its exit criteria mean anything.

**Imports are single-mode: every name a file uses comes from an `import:`, with no
implicitly present words at all.** `lib/core.sth` is today injected into every program
unqualified, which is a hole in the resolution model rather than a convenience: prelude
words are exempted from per-module mangling (`src/resolve.rs:32`, via
`is_prelude_word_name`), and because they are checked against the whole program's word
environment they have no hygiene against user names, which is why `if`'s own locals are
spelled `if--cond`/`if--then-arm` to avoid colliding with any program that defines a word
`t`. Deleting the special case rather than relocating it is the point: prelude words arrive
through the same import path as everything else, the mangling exemption shrinks to `main`
and `drop`, and those locals become plain names. The cost is a visible one: every existing
`.sth` file gains import lines, and a new file's most likely first error is `unknown word:
if`, which needs a diagnostic naming the missing import.

That splits today's `lib/core.sth` along the boundary its own header already draws: the
compiler's primitives (`branch`, `tag`, and the six `u`-prefixed comparisons) are one
package, and everything typed that is built on them (`bool`, `=`, `if`/`unless`,
arithmetic, combinators) is another that depends on it. Both are `core`-layer; the split is
package granularity, not a layer boundary, and it gives bare metal an honest floor to
depend on.

**Exit:** a program builds from a manifest against a package dependency it names; a package
declaring a lower layer than its dependency is a located build error; no word is visible
without an `import:`, and `is_prelude_word_name` and `parser::prelude_words` are gone.
**Dogfood:** `lib/`'s existing words restructured as packages (intrinsics, the typed core
built on it, and the array/combinator words as consumers), with every example and golden
program importing what it uses.

**P8.S1 — Packages, manifests, and single-mode imports.** The unit above the file, the
manifest and its layer field, source-based dependency resolution, the layering
dependency-direction check, and the deletion of the implicit prelude. Needs its own brief
before implementation; the ordering question inside it is whether the manifest lands before
or after the prelude split, since the split is what forces every program to have a manifest
to name its imports against.

**P8.S2 — The serialisable API description.** "Which words, types, and externs are public"
is already answered by Phase 4 Slice 5's `export:` list, and answered where it had to be,
since a type cannot hold an invariant while its generated setters cross the boundary
unchecked. What is left is one thing, not two: a **serializable API description**, a
compiler pass that walks the checked AST, filters to the exported declarations Slice 5
already distinguishes, and emits a file listing every exported signature for the API diff to
compare between versions. That is the remaining prerequisite in
`docs/dependency-management.md`, and it is a packaging/publishing concern (letting other
people depend on you with enforced semver) rather than a personal-reuse one, which is why it
waited. Needs P7.S2 (statics) and P7.S3d (bounds), since a global clause on an exported word
is part of that word's exported signature.
**Exit:** a published package's API diff correctly classifies a PATCH/MINOR/MAJOR bump
across a two-file change.
**Dogfood:** `sooth publish --check` on a two-version bump of a small library, one that adds
a word (MINOR) and one that removes one (MAJOR).

**Deferred, with reasons.** A **C-ABI export target** (emitting a library other programs
can link) is codegen work (symbol naming, calling convention, header generation) and
shares nothing with dependency resolution, so it is not bundled here; it is also the
prerequisite for **Rust↔Sooth FFI**, which is what a self-hosted compiler module would need
and which waits on it. **Semver enforcement itself** (the API diff and `sooth publish
--check`) is tooling on top of P8.S2's format, specified in
`docs/dependency-management.md`. **A user-level manifest** (`$XDG_CONFIG_HOME/sooth`) is
how the REPL eventually gets a session's imports; a REPL exemption in the compiler is not,
since that is the special case this phase deletes. **Re-exports, aliasing an import to a
different local qualifier, wholesale unqualified import, and a `mod.sth` directory
convention** stay declined per Phase 4 Slice 5. A manifest flag marking a dependency's
exports visible unqualified package-wide is the sanctioned way to reintroduce prelude
ergonomics later, and is not built now.
