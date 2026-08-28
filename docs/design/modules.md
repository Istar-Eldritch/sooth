# Sooth — modules and encapsulation

Design detail for modules and encapsulation, split from [DESIGN.md](../../DESIGN.md).

## Modules and encapsulation

A file is a compilation unit (Phase 4 Slice 5a), and
a directory tree under a `sooth.pkg` manifest is a **package**. `export: name... ;`
(lines accumulate) is the only way a name leaves its file; a module with none exports
nothing, so every pre-5a example is unaffected — it exports nothing and stays a
program, not a library.

**Inside a package an import names a module, not a file.** `import: <target> [<q>]
[ | name... | ] ;` puts the target first and the qualifier last, because the common
case wants no renaming at all: an elided qualifier defaults to the target's last
segment, so `import: self::text::ascii ;` binds `ascii`. The target's anchor is
syntactic and never inferred — a `self::` prefix is the importing file's own package,
package-root-absolute (`self::text::ascii` is `text/ascii.sth` under the package
root), and a bare first segment names a dependency the manifest's `depends:` lists
(`import: core::cmp c ;`). A dependency named `text` and a local `text/` directory
therefore coexist with no precedence rule and no ambiguity error. A module-name
segment must lex as a single word, which keeps the file-to-module-name map
one-to-one; `::` and a bare `*` are reserved for the separator and the wildcard
target, so `42.sth` and `*.sth` are simply unnameable.

A file with no ancestor manifest keeps the quoted-path form, `import: "path.sth" q ;`,
resolved relative to the *importing* file with an explicit `.sth` and no search path
(consistent with `extern:` naming its C symbol verbatim: no implicit extension is one
fewer resolution rule to learn). Inside a package that form is a located error naming
the module-name spelling to use instead: one file, one way to name it.

**Why resolution has to happen as one merged pass, not a parse-then-merge.** The
parser resolves every type name in a pre-pass over raw tokens *before any word body
parses* (`prepass_type_decls`, then `build_registries`, both inside `parse`). An
importing file's own pre-pass needs the imported file's type names present before its
bodies can parse at all, so parsing each file independently and merging the two ASTs
afterward would mean remapping every positional `StructId`/`EnumId`/`ArrayId` in the
second file's already-parsed tree — strictly more work, and more places to get it
wrong, than doing it once. The model instead: resolve the import graph from the entry
file, canonicalize and dedupe by path (a diamond import is parsed once), order it
topologically and reject a cycle or self-import with a located error naming both
files, then run **one shared pre-pass** across the whole closure's tokens into **one
shared registry set**, and only then parse bodies per file against that shared set.
The closure still assembles into one `Module`, so `check::check` keeps its
single-module signature; module identity rides on a per-decl owning-module tag, not on
threading multiple `Module`s through the pipeline.

**Name resolution is "own module first, then qualifier," not filtering at merge
time.** The registry stores a bare name plus its owning module (rather than, say, a
fully-qualified stored name), so an unqualified reference resolves in its own module
first and a `q::base` splits on the qualifier, maps `q` through the current module's
import table, and resolves in the target module subject to its export list. Every
module's names are spliced into one shared environment and *marked* with their module
and export status; rejecting an unqualified-but-private reference happens at the use
site, never by hiding the name at merge time — filtering there would collapse two
distinct failure modes (a name that exists but is private, vs. a name that is simply
absent) into one `unknown word`, which is a worse diagnostic for a language that
otherwise turns Forth's silent failures into sharp errors. Two modules may each
declare `Point`; the duplicate-type-name check is per-module, not global. Same-named
words in two modules mint distinct emitted symbols via a module-disambiguating
component, added to `instantiation_symbol` the same way its `generation` suffix
already is, so `::` never has to survive to the symbol sanitizer and a single-module
closure (every pre-5a program) is byte-for-byte unchanged.

**A `type:` declaration is a name-scope, and visibility is the ordinary export
mechanism applied to it, not a special rule for types.** Its generated words — a bare
constructor (`Type`) and a destructure (`Type>`) — are named by string concatenation, an
ad-hoc qualified namespace the compiler already builds for every struct. Exporting
`Type` therefore exports that whole name-scope as one unit: naming a type in `export:`
is **transparent**, with no opacity mechanism and no per-member withholding in this
slice. A consumer may name `q::Type` in an effect, construct one with `q::Type`, and
destructure it with `q::Type>`. Individual fields are reached through `&field`/`&!field`
(Phase 7 Slice 1), resolved against the receiver's type at each call site rather than
through a per-type qualified name, so field projection is unqualified regardless of
which module declared the type.

This was a reversal mid-design, not the obvious choice: the first draft made export
opaque by default, Elm-style, distinguishing "export the type" from "export its
constructor." It didn't survive contact with what Sooth actually is. Structs are dumb
data; a violated field invariant is a bug in the *consumer's* program, not unsoundness,
because there is no UB, indexing traps at the bound, and linearity already prevents
aliasing a value into two invariant-breaking places at once. And the resource argument
for opacity has nothing to add: moving the fields out of a type with a `drop` override
is rejected outright (`type: R tag i64 ;` with a `drop` override, then `r R>`, is a
located error), and the field *read* that remains cannot launder a resource either,
because `@` refuses a linear referent. So a consumer never obtains ownership of a field
it did not construct, by two rules in the ownership checker that know nothing about
modules — and hiding an accessor behind export visibility would protect nothing those
rules don't already guarantee. Hiding an accessor behind a visibility rule is the OOP
ceremony this language is declining to need; a withhold marker on `export:` is an
additive feature for a real consumer that wants it, not a default this slice should
guess at.

The same rule holds across a file boundary as within one: a library consumer
destructuring an imported linear type down to `Copy` leaves is rejected exactly as a
single-file program's own destructure would be, since the rule lives in the ownership
checker and knows nothing about modules.

**Disposing an imported resource type requires that type to be visible to the disposing
module.** `drop` is compiler-known and dispatches on the concrete type (Slice 3/8b), but
a bare `drop` on an imported linear value runs a destructor the *owning* module declared,
so the calling module must have that type in scope — imported by name, or declared
locally — the same visibility a bare use of any other name from that module needs. A
qualified-only import that never names the type is a located error at the `drop`, naming
the remedy (add the type to the import, or dispose it in a module that declares it). A
consumer that has imported the type by name can always discharge it, so the ROADMAP's
hypothesized "an exported linear type must also export its discharging word" rule has
nothing to fire on: the discharging word is `drop` itself, reached through the type's own
visibility. It only becomes a live question once a polymorphic `drop ( 'T -- )` could be
structurally total — exactly what Slice 8's own constraint forbids — so enforcement is
deferred there, not decided here.

**Declaration-site and selective-import rules round out encapsulation.** An exported
word whose stack effect names a private, non-primitive type of its own module is
rejected at the `export:` declaration itself (the module author's bug, not the
consumer's), naming the word and the private type; exporting the type satisfies it.
Selective import, `import: core::text s | split trim | ;`, is additive to the
qualifier: `s` is always bound, and the listed names are *additionally* exposed
unqualified (a selectively-imported type brings its generated words unqualified too,
one unit as ever). The collision rule is deliberately dumb, with no precedence and no
use-site disambiguation: two selective imports exposing the same unqualified name is
an error at the second, naming both modules, and a selectively-exposed name colliding
with a local definition is the same error.

Out of scope for this slice, all deferred to Phase 8's eventual package/versioning
layer or later: a serializable API description and semver enforcement (which will
consume this slice's export list, not redefine it), package manifests and a registry,
re-exports or aliasing an import to a different local qualifier, a `mod.sth`-style
directory-mirrors-module-tree convention (declined: flat file-is-a-module plus
qualified access covers the only consumer that exists), and generic type declarations
crossing files (they don't exist yet).
