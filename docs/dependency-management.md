# Dependency management: enforced semver

Exploration, not decision. Records the design for Elm-style enforced
semantic versioning in Sooth, to preserve the reasoning before the
module system (Phase 8) lands and the details fade. Nothing here
overrides a decision in DESIGN.md; it specifies a mechanism the design
leaves open, and marks what it depends on from Phase 8.

## The goal

A Sooth program written in 2025 should compile in 2030 — including its
dependencies. This is the durability axis the language is built for, and
it only holds if dependencies can't break under you. Enforced semver
with bounded ranges is the mechanism that makes that true for
dependencies, not just for the compiler.

## The model (Elm-style)

Two properties, both enforced by tooling at publish time:

1. **The version bump is mechanically derived from the public API diff.**
   The compiler computes a module's exported API — every exported word
   with its stack effect, every exported type with its fields or
   variants — and compares it to the previous version. The diff
   classifies the bump: PATCH (no API change), MINOR (pure additions),
   MAJOR (removals or signature changes). No judgment calls, no
   heuristics — it's structural comparison of the checked AST.

2. **All dependency ranges are bounded** (`1.2.0 <= v < 2.0.0`), so
   MINOR and PATCH updates are guaranteed safe by construction. `sooth
   update` bumps every dependency to its latest compatible version, and
   it works because the enforced semver makes the bounds trustworthy.
   Without enforcement, a library author ships a "MINOR" update that
   removes a function, and the bound lets it through. With enforcement,
   the tooling refuses to publish a version whose bump doesn't match the
   API diff.

## Why Sooth's type system makes this simpler than Elm

Elm's type system has parameterized custom types, type aliases, and
extensible record types. The API diff must handle all of them, and the
last two are the awkward ones: an alias can expand to a different
structure without its name changing, and record types are structural, so
"same fields" is the comparison rather than "same name." (Elm
deliberately has *no* higher-kinded types — no abstraction over type
constructors, which is why it has no user-definable typeclasses and why
`andThen` is written once per type rather than once for all monads.)

Sooth's type system is simpler for this purpose. The stack effect
`( in -- out )` is a flat tuple of types — no higher-kinded types, no
type classes, no associated types, no coherence rules, and no structural
record subtyping. Computing a
module's public API is: enumerate exported words, emit their `(name,
stack effect)` pairs; enumerate exported types, emit their `(name,
fields/variants)` definitions. Diff is structural comparison. The
compiler already has all of this — it's the checked AST.

## Why the linear spine makes this more precise than Elm

In Elm, a function's type is `Int -> String -> Maybe Int`. In Sooth, a
word's stack effect encodes ownership: `( File -- )` consumes the file,
`( &File -- )` borrows it, `( File -- File )` takes and returns it.

A change from consuming to borrowing (or vice versa) is a breaking
change — the caller's ownership contract changed — and the stack effect
diff catches it automatically. Elm's type system can't express this
distinction; Sooth's enforces it as part of the API contract. The semver
enforcement is stricter and more accurate than Elm's, not just equal to
it.

## Bump classification

Mechanically computable from the AST:

| Change | Bump | Why |
|---|---|---|
| Internal implementation change, same signatures | PATCH | No API change |
| New exported word, new exported type | MINOR | Pure addition |
| Remove an exported word | MAJOR | Existing code may call it |
| Change a word's stack effect | MAJOR | Caller's contract changed |
| Add a field to an exported struct | MAJOR | Layout + construction changed |
| Add a variant to an exported enum | MAJOR | Exhaustive matching breaks |
| Change ownership in a signature (`T` -> `&T`) | MAJOR | Caller's contract changed |
| Change a destructor body | PATCH | Behavior change, not API change |

No judgment calls. The tooling reads two ASTs and produces a bump.

## Soundness: why dropping open multimethods closed the gap

Open multimethods (`generic:`/`method:`, declined — see DESIGN.md) would
have created the one scenario where the API diff says MINOR but
existing callers' behavior changes: adding a new `method:` arm doesn't
change any word's signature (so the diff calls it a MINOR addition),
but if the new arm is more specific, it can intercept calls that
previously went to a different arm. The enforcement has a gap: the API
didn't change, the behavior did.

Without open multimethods, this gap doesn't exist. Static overloading
(Slice 8) dispatches on the concrete type at the call site — adding a
new overload is a pure addition (MINOR) that can't shift resolution for
existing callers, because existing callers' concrete types already
resolve to their existing overloads. The handler-struct convention
(functions as values, Slice 7) dispatches through a closed match the
caller can see — adding a new handler is a new match clause (MINOR),
and the exhaustiveness check tells the caller to update, which is a
compile-time event, not a silent runtime shift.

With open multimethods declined, the semver enforcement is fully sound:
every MINOR is a pure addition, every MAJOR is a real breaking change,
and there is no scenario where the bump classification lies about
behavioral impact.

## What it depends on from Phase 8

Two things, both straightforward:

1. **Explicit export lists.** Elm uses `module Foo exposing (bar, baz)`.
   Sooth needs the equivalent — a way to mark which words, types, and
   externs are public. Everything not exported is private and invisible
   to the API diff. Without this, every declaration is public and the
   diff is meaningless. The module system (Phase 8) introduces
   compilation units and imports; export lists are the natural companion
   that says what crosses the boundary.

2. **A serializable API description.** The compiler emits a file (e.g.,
   `foo.api`) listing all exported declarations with their full
   signatures. This is what gets diffed between versions. It's a
   compiler pass: walk the checked AST, filter to exported declarations,
   serialize. The infrastructure already exists — the checker produces
   exactly this information; it just needs a serialization target.

Neither is specified here. Both are Phase 8's job; this doc records what
the semver enforcement needs from them, not how they're built.

## No central registry needed to start

Elm uses a central registry (package.elm-lang.org) for publish-time
enforcement. That's infrastructure — a server, a database, a CLI that
talks to it. For a craft language, the enforcement can be a local tool:
`sooth publish --check` computes the API diff between the working tree
and the last git tag, and refuses to create a release tag whose bump
doesn't match. This is the same mechanism, just without the server. A
registry can come later if the ecosystem grows; the enforcement logic is
identical either way.

Source-based dependencies fit naturally too. Sooth compiles one
compilation unit — there's no per-crate LLVM overhead — so adding a
dependency is just pulling in source files and compiling them into your
program. No binary artifacts, no `.so` resolution, no ABI compatibility
questions. The compile-time advantage (a full build in tens of
milliseconds) means dependency compilation is cheap too, unlike
languages where transitive dependency builds take minutes.

## Relationship to existing decisions

- **Extends "Concurrency: a library, not a core feature" (DESIGN.md)**
  only in spirit — both reflect the "own the small thing" philosophy.
  Dependency management is a tooling concern, not a language feature.
- **Consumes Phase 4 Slice 5's export list; the tooling stays Phase 8.**
  Multi-file compilation, word and type imports, and the `export:` list
  itself all land early (Phase 4 Slice 5, pulled forward once a reusable
  component — usually a type plus its operations — needed somewhere to
  live, and encapsulation came with it because a type cannot hold an
  invariant while its generated setters cross the boundary). So the
  "what's public" half of this doc's first prerequisite is answered there,
  including the Elm-style opaque-type distinction between exporting a type
  name and exporting its constructors. What stays Phase 8 is the second
  prerequisite: a serializable API description and the diffing that reads
  it. This doc records the design so the reasoning survives, but the
  tooling still waits.
- **Interacts with "Open multimethods: Declined" (DESIGN.md).** The
  soundness argument in this doc relies on that decision. If open
  multimethods were ever revisited, the semver soundness section would
  need to be revisited with it.
- **Interacts with static overloading (Phase 4 Slice 8).** Adding a new
  overload is a MINOR addition that can't shift existing dispatch —
  this is the property that keeps the enforcement sound for overloaded
  words.
