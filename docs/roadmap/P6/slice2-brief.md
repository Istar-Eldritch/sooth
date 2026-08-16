# Phase 6 Slice 2: variant types and accessors (brief)

`Type::Variant(EnumId, usize)` — a type legal only as an arm's declared input and as
the value inside that arm, never a general first-class type, since the eliminator
(Slice 3) is its only introducer. Plus generated per-variant field accessors
(`Circle>r`) and whole-variant destructures (`Rect>`), mirroring the struct-generated
words that already exist. Slice 1's annotation grammar and Slice 3's eliminator word
are both out of scope; this slice's own exit case has no eliminator to bind a
`Type::Variant` value in the first place; it will need to be checked and tested
directly against a hand-built `WordDef`/env, the same way Slice 1's recon found no
route to a standalone quotation-literal check without going through a consuming
context.

## Recon (measured against the built compiler, 2026-08-16, `main` at `42f0764`)

`cargo test` is green at this HEAD. Claims below are read from source, not inferred.

1. **`Type` has no `Variant` case today.** The full enum (`src/ast.rs:1186-1226+`) lists
   `Int`, `Float`, `Struct`, `Enum`, `Array`, `OwnedCell`, `Ref`, `Usize`, `Isize`, `Str`,
   plus poly/quotation cases further down; no `Variant`. Adding it touches every
   exhaustive `match Type` in the tree (`Display`, `is_copy`, `is_aggregate`,
   `contains_reference`, IR lowering's type-size/align table, etc.) — this is the
   slice's real *breadth*, not its depth: one new leaf case, many match arms to extend,
   almost all of which should reject a bare `Type::Variant` outright (recon 6) rather
   than implement new behaviour for it.

2. **The clause payload-binding mechanism already does everything this slice's
   accessors need, just positionally and only inside a clause body.**
   `check_clause_body` (`src/check/word_entry.rs:405+`) already: resolves the top
   input's enum id, looks up `variant.fields` by declared position, and pushes each
   field (in reference mode, aliasing through the scrutinee's mutability, "projecting
   through it exactly as a struct-field projection would" per its own doc comment) onto
   the stack, then binds `| names |` against them positionally. This slice's per-field
   accessor and whole destructure are the same field-list walk and the same
   value-mode/ref-mode projection logic, restructured as *named* generated words
   (`Circle>r`, `Rect>`) instead of positional `| a b |` locals — not a new mechanism,
   a second surface over the one that exists.

3. **`enum_generated_sigs` (`src/check/declarations.rs:1256-1279`) explicitly documents
   the gap this slice closes.** Its own doc comment: "Unlike a struct, a variant has no
   destructure/getter/setter (D2: not a standalone type; elimination is clause-style,
   Phase 4)." It only emits the constructor `Sig` today. `struct_generated_sigs`
   (`:1215-1253`), three lines up in the same file, is the exact template: for each
   field it emits a `>` (destructure), a `>field` (getter), and a `<field` (setter) `Sig`
   keyed by mangled name, consumed through `check_struct_get_word`
   (`src/check/word_families.rs:719`) and its sibling set/destructure words. A variant
   getter has no `<field` setter analogue (recon 6 below): a `Type::Variant` value
   only exists inside its arm, so "set a field and hand back the same variant" has
   nowhere legal to flow to.

4. **`EnumLayout.variants[vi].fields` already carries the runtime layout this slice
   needs, unchanged.** `src/ir/layout.rs:197-224`: each `VariantLayout` is a `Vec<
   FieldLayout>`, offsets relative to `EnumLayout::payload_offset`, placed by the same
   natural-alignment walk struct fields use. Confirmed against `ir/layout.rs:687` and
   `:727` (the two `enum_memo` construction sites): nothing here changes at runtime, the
   roadmap's own claim holds. This slice is a checker/frontend-only feature; no
   `src/ir/`, `src/backend/`, or `EnumLayout` change is implied by variant-typed value
   passing an accessor through to the field it already knows the offset of.

5. **Reference-mode variant access already exists and needs no new plumbing.**
   `check_clause_word`'s `ref_mutable: Option<bool>` (`word_entry.rs:305-324`) already
   handles a clause's top input being `Enum` (value) or `&Enum`/`&!Enum` (reference),
   and `check_clause_body` already projects each field through that reference rather
   than moving it. A `Type::Variant` value born inside an arm is therefore itself either
   a value or (if the eliminator is invoked on a borrowed scrutinee) a reference-derived
   projection target — the exact same two modes an accessor call needs to check against,
   with the exact same logic clause bodies already run. Nothing about ref-mode is new
   work; it is the one existing branch this slice's accessors must not regress.

6. **Verified: `Type::Variant` cannot be smuggled anywhere but an arm today or after
   this slice, because nothing mints one except the (not-yet-built) eliminator.**
   Every *other* place a `Type` currently appears — a word's declared input/output slot,
   a struct field type, an array element type, a local's inferred type from `env` —
   is reached by parsing a type expression (`parse_type_expr`,
   `src/parser.rs`) or by ordinary stack-effect inference, neither of which this slice
   touches. So the closed-world claim ("legal only as an arm's declared input and the
   value inside that arm") is true *by construction*, not by an added guard: with no
   eliminator built yet, this slice has no way to construct a `Type::Variant` value from
   surface syntax at all, which is exactly why recon opens by flagging that this
   slice's own exit case cannot be reached through `examples/*.sth` end-to-end and needs
   a direct unit test against a hand-built AST/env instead (mirroring Slice 1's own
   `check_literal_against_declared_effect` unit-level testing, not a golden).

7. **Generic-enum naming precedent already handles the surface/mangled split this
   slice's accessors need.** `enum_generated_sigs` (recon 3) already runs
   `generic_surface_name` on each variant name before minting the constructor `Sig`'s
   surface key, exactly mirroring `struct_generated_sigs`'s identical call one line
   above it. A per-variant accessor for a monomorphized generic enum (`Result[i64
   i64]`'s `Ok`) needs no new naming logic: the same function, called the same way,
   on the same variant name, produces the right surface/mangled pair.

## Decisions (settled here, not reopened by the spec)

1. **`Type::Variant(EnumId, usize)` carries the owning enum and the variant's index
   into `EnumDecl.variants`**, not a separate registry entry (there is nothing to
   register: recon 4 shows the layout already lives on the enum's own `EnumLayout`,
   keyed by the same index). No leaked `&'static str` field the way `Struct`/`Enum`
   carry one — a variant's display name is recovered via
   `enums[id.index()].variants[vi].name`, avoiding a second copy of a name the enum
   registry already owns.

2. **The generated accessors mirror struct naming exactly**: `Variant>field` (getter),
   `Variant>` (whole destructure), field order matching declared order (first field
   deepest, same convention `enum_generated_sigs`' constructor already uses). No
   `Variant<field` setter (recon 3): a `Type::Variant` has no legal place to flow a
   "same variant, one field replaced" result to outside its arm, so there is nothing
   for a setter to hand back that a getter-then-rebuild couldn't already do inside the
   arm using the variant's own constructor.

3. **A zero-field variant gets no generated accessors at all** (nothing to project),
   mirroring how a zero-field struct today gets a constructor and a no-op `>` but no
   per-field words — Slice 1's own precedent of "an unannotated/unused case is
   unaffected, not specially rejected."

4. **Both value-mode and reference-mode variant access reuse `check_clause_body`'s
   existing projection logic** (recon 2, 5), refactored into a shared helper both the
   (retired-in-S4, still-live-in-S2) clause path and the new accessor path call, rather
   than duplicated. This is the one piece of this slice that touches existing,
   currently-passing code, and it must not change clause-body behaviour: every
   existing clause-style golden and dogfood stays green with identical generated code.

5. **This slice adds no surface syntax to bind a `Type::Variant` value.** Recon 6
   already establishes the eliminator (Slice 3) is the only introducer; until it lands,
   this slice's own exit witness is a unit test constructing the checker state
   directly (an arm-shaped `Ctx`/env with a `Type::Variant` slot already on the stack),
   not an `.sth` golden. The spec should not treat "no golden reaches this" as a defect
   to route around; it is the correct consequence of Slice 2 shipping ahead of Slice 3
   in a single-clause-eliminator-free codebase.

## Open questions for the spec

- **OQ1 — does the shared projection helper (Decision 4) also need to *change* clause
  body binding's own code path, or does it stay a pure extraction with clause bodies
  as the first caller and accessors as the second?** Recon 2 found the two use cases
  identical in every observed dimension (field order, ref-mode projection, exhaustive
  field list), but the spec should confirm no subtle difference exists before
  refactoring shared logic underneath a currently-green test suite — a wrong shared
  abstraction here risks a Slice 1-style "measured, not assumed" miss on the one
  existing code path this slice actually touches.

- **OQ2 — what diagnostic does an accessor called on a value that is not (yet, absent
  the eliminator) reachably `Type::Variant`-typed produce?** Since recon 6 shows no
  surface syntax can currently produce a `Type::Variant` value, is "unknown word
  `Circle>r`" (falling through the ordinary env lookup, since nothing registers the
  sig without an eliminator scoping it) an acceptable interim shape, or does this
  slice need to register the generated sigs globally now (Decision 2) even though no
  legal call site can supply the required operand type until Slice 3 ships? Leaning
  towards: register the sigs now (so Slice 3 has nothing left to add on the accessor
  side, only the eliminator itself), and let the ordinary type-mismatch path
  (`type_mismatch_error`, same as `check_struct_get_word`'s) fire once *something*
  can be `Type::Variant`-typed — which for this slice's own unit tests, is exactly
  what they construct by hand.

- **OQ3 — is a nested-aggregate variant field (a struct- or array-typed field inside
  a variant) accessed via `Variant>field` handled by the same interior-address
  aliasing device `check_struct_get_word`'s doc comment describes for `S>fi`
  (recon in `word_families.rs:706-716`), or does this slice restrict itself to
  scalar variant fields and defer aggregate-field variant access?** `examples/vm.sth`'s
  own dogfood target has no aggregate-typed `Op` variant field today (every payload is
  `i64`/`usize`), so nothing in the stated dogfood forces an answer, but the spec
  should say explicitly whether this is in scope or explicitly deferred, rather than
  leaving it to be discovered mid-implementation.

## Out of scope

- The eliminator word itself, arm-position effect elision (`( Circle )`), exhaustiveness
  and duplicate-arm checking: all Slice 3.
- Migrating any existing clause-dispatch site (`examples/vm.sth`, `Bool`, `Result`/
  `Option`) to the eliminator, or deleting `WordBody::Clauses`/`parse_clauses`: Slice 4.
- Any change to `EnumLayout`/`VariantLayout` or backend emission: recon 4 found no
  lowering gap.
- A `Variant<field` setter (Decision 2): no legal destination for its output exists
  within this slice's own scope.
- Rewriting `check_clause_body` to *stop* doing positional binding: Decision 4 asks it
  to share logic with the new accessor path, not to be replaced by it. Clauses keep
  working exactly as today until Slice 4 deletes them.

## Sequencing

No gate from any earlier slice or open Phase 4/5 item; recon 4 and 6 close out the two
questions this slice's own text could plausibly have needed answered first (a lowering
gap, a name-collision risk). Touches `src/ast.rs` (the new `Type::Variant` case and
every exhaustive match it forces), `src/check/declarations.rs` (a new
`variant_generated_sigs` beside `enum_generated_sigs`, mirroring
`struct_generated_sigs`), `src/check/word_families.rs` (the new accessor check
functions, and the extraction Decision 4 asks for), and `src/check/word_entry.rs`
(`check_clause_body` calling the extracted shared helper instead of its own inline
projection).

## Exit

A variant-typed value is reachable only inside an arm (vacuously true until Slice 3,
per Decision 5), with its fields readable by name through a generated `Variant>field`
getter and a whole-variant `Variant>` destructure, in both value and reference mode,
proven by unit tests against a hand-built checker state rather than an `.sth` golden.
Every existing clause-style test and dogfood (`examples/vm.sth`, `Bool`, Phase 5's
`Result`/`Option` consumers) stays green with identical generated code, since Decision
4's extraction is required to be behaviour-preserving.

## Ready to spec?

**Yes, with three open questions, none blocking a narrow reading.** OQ2 should settle
toward "register the sigs globally now" — it is strictly less work overall (nothing
left for Slice 3 to add there) and recon 6 already shows no unsound exposure results
from a sig existing with no legal caller yet. OQ3 is the one question that changes this
slice's actual test surface (whether an aggregate-field unit test is part of the exit
witness or explicitly deferred) and should be settled before writing tests, not
discovered while writing them. OQ1 is a pure refactor-safety question the spec can
answer by reading the two call sites side by side.
