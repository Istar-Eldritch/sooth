# Phase 5 Slice 1: generic `type:` declarations (spec)

## Problem statement

A `type:` header today names one concrete struct or enum: every field type resolves
through `resolve_type` (`src/parser.rs:2151`) to a plain `Type`, and `StructDecl`/
`EnumDecl` (`src/ast.rs:207`, `:260`) store only concrete `(String, Type)` field lists.
There is no way to declare a struct or enum parameterized by a type variable
(`type: Box 'T | val 'T ;`) the way a word can already declare one
(`: id ( 'T -- 'T ) ;`, Phase 4 Slice 1). Phase 5's Result/Either (Slice 2) needs exactly
this mechanism first: `type: Result 'T 'E | Ok val 'T | Err val 'E ;` cannot be written
until a `type:` header can bind type variables and mint one concrete struct/enum per
distinct instantiation.

This slice ships that mechanism alone, proven against a throwaway generic type, not
against `Result`. Anything Result/Option-specific is Slice 2.

## Requirements

**R1 — a `type:` header may bind one or more type variables.** `type: Box 'T | val 'T ;`
and, for the multi-variable case, `type: Pair 'A 'B a 'A b 'B ;` both parse: the header
declares the variables (`'T`, or `'A 'B`), and every field type in the body may reference
any variable the header bound. A field naming a variable the header did not bind is a
located parse error.

**R2 — a field type may apply a generic type to concrete type arguments.**
`type: Wrap x Box i64 ;` (a concrete struct with a field of type `Box i64`) and a word
signature slot (`: unwrap ( Box i64 -- i64 ) ;`) both parse an explicit generic-type
application: a name that resolves to a generic `type:` header, followed by exactly as
many type-expressions as that header declared variables. Type arguments are always
explicit; there is no inference (a call-site stack has no value to unify a type
declaration's variables against, unlike a polymorphic word's call site).

**R3 — an application with the wrong argument count is a located error.** `Box` alone (no
argument) or `Box i64 bool` (one argument too many) applied where `Box` needs exactly one
is a located parse/resolution error naming the generic type, the number of variables it
declares, and the number of arguments supplied.

**R4 — distinct concrete applications monomorphize to distinct `StructId`/`EnumId`s; the
same application dedupes to one.** `Box i64` and `Box bool` mint two different, correctly
laid-out concrete struct registry entries. `Box i64` used a second time anywhere in the
same module resolves to the *same* `StructId` as the first use — structural dedup on
`(generic name, concrete type arguments)`, mirroring how `intern_bundle_struct`
(`src/ast.rs:525`) dedups a bundle shape and how a polymorphic word's `Subst`-keyed
`CallInst` (`:709`) dedups a call-site instantiation.

**R5 — a monomorphized instantiation behaves exactly like a hand-written concrete
`type:` of the same shape.** Once `Box i64` is minted as a `StructId`, its field accessor
(`Box>val`), its constructor (`1 Box`), destructor synthesis, layout, and every existing
struct/enum check (`src/check/declarations.rs`) apply unchanged — because they already
operate on the concrete `StructDecl` the registry holds, not on anything specific to how
that entry was declared (recon: `Vec2>x`-style accessors and constructors resolve by
looking up the concrete `StructId`'s field list directly, `src/check/declarations.rs:2339-2360`
et al., with no separate "generated word" registration step). No new accessor/constructor/
destructor logic is written in this slice.

**R6 — a generic type's own field may reference a concrete application of another (or the
same) generic type non-recursively.** `type: Wrap x Box i64 ;` (R2) covers the concrete
side. A generic type applying *another* generic type inside its own field list using its
own still-open variable (`type: Outer 'T x Box 'T ;`) is explicitly **out of scope**
(decision D3): the exit case is a flat generic type whose fields are concrete or
variable-only, never a nested open application.

## Non-functional requirements

- **No new `Type` variant.** A generic type's *declaration* carries a variable-bearing
  field list in a new declaration form; a concrete `Type::Struct`/`Type::Enum` is minted
  only once every type argument at a use site is concrete, mirroring Phase 4 Slice 1's own
  rule that `Type` gains no variable-carrying variant (`src/ast.rs:619-621`).
- **Deterministic, order-independent mangled names.** The synthesized name for a
  monomorphized instantiation (e.g. `Box`+`[i64]` -> `__generic_Box__t_i64` or similar) is a
  pure function of `(generic name, concrete type arguments)`, with no dependence on
  processing order — mirroring `instantiation_symbol` (`src/ast.rs:729`).
- **No regression to existing concrete `type:` declarations.** Every existing golden
  `.sth` file and `parse_typedef_*`/`check_struct_*`/`check_enum_*` test continues to pass
  unchanged; a concrete (non-generic) `type:` header parses exactly as it does today.

## Scope and boundaries

**In scope:** parsing a generic `type:` header (struct and enum forms) with one or more
type variables; an explicit generic-type-application syntax at a field-type position
(`parse_field_type_expr`) and at a word-signature slot position (`parse_slot`/
`parse_poly_slot`); a monomorphization/instantiation table keyed by `(generic name,
concrete type arguments)` producing a `StructId`/`EnumId`; wiring that table so an
instantiation lands in the ordinary `Module::structs`/`enums` registries the existing
layout, accessor, constructor, and destructor machinery already walks.

**Out of scope (explicit; see Decisions for the "why"):**

- `Result`, `Either`, `Option`, `?` sugar, and branch-on-result codegen: Slice 2.
- A generic type's field applying another still-open generic type (`Outer 'T x Box 'T ;`,
  R6): deferred; not needed by this slice's exit case or by Slice 2's `Result`/`Option`.
- A generic type recursively self-referencing through `^` (`type: List 'T | Nil | Cons val
  'T next ^List 'T ;`): deferred; `Result`/`Option` are not recursive, and this is `Vec`/
  `List`-shaped territory that belongs with Phase 6's stdlib types.
- Bounds (`Copy`/`Ord`) on a generic type's variables: deferred; not needed by a plain data
  carrier.
- Cross-module generic type import/instantiation (`import:` a generic type, then apply it
  with a qualifier): deferred to whichever slice first needs a generic type living outside
  its instantiating module.
- The default-allocator-parameter question (`Vec['T 'A = Global]`): Phase 6, per ROADMAP.

## Decisions

**D1 (settles brief OQ1, narrow reading) — no generic-in-generic nesting.** A generic
type's own fields may only be concrete or a bare variable (R1), never an application of
another generic type using an as-yet-unresolved variable. A *concrete* application inside
an ordinary (non-generic) struct's field (`type: Wrap x Box i64 ;`, R2) is in scope, since
`Box i64` is fully concrete at that point — the restriction is specifically on an *open*
variable flowing into another generic application inside a declaration body. Reason:
Result/Option (Slice 2) need only single-level generic types; nesting adds a second axis
of instantiation-table complexity (an instantiation whose own type arguments are
themselves unresolved) with no concrete consumer yet.

**D2 (settles brief OQ2, minimal-but-not-single reading) — multiple type variables are
supported from the start, no bounds.** Result needs two variables (`Result 'T 'E`), so
restricting Slice 1's grammar to exactly one variable would force a re-parse change in
Slice 2 for no reason: the header grammar (`'T` `'E` ... in sequence) generalizes to N
variables at the same implementation cost as one. Bounds (`Copy`/`Ord`) are left out: nothing
in this slice's exit case or in `Result`/`Option` needs one.

**D3 (settles brief OQ3, narrow reading) — no recursive self-reference in the exit case.**
The exit witness is a flat generic struct or enum (e.g. `Box 'T`, `Pair 'A 'B`, or a
non-recursive two-variant enum). `List`/`Vec`-shaped recursive generics are explicitly
deferred (out of scope above): they are Phase 6 territory per the ROADMAP's own framing,
and proving the recursive self-reference case adds the pre-pass sequencing problem (recon
6 in the brief: registering a generic name before its own body can reference it) that
neither this slice's exit case nor Slice 2 needs solved.

**D4 (settles brief OQ4, narrow reading) — single-module only.** A generic type is
declared and instantiated within one module in this slice. Cross-module generic import
(`import: v | Vec | "vec.sth" ; v::Vec i64`) is deferred: Phase 4 Slice 5's qualified-name
machinery (`resolve_type`'s `::` handling) already threads a `module: u32` through
concrete types, so extending it to a generic-type dictionary is mechanical once a
consumer needs it — no consumer needs it in this slice or in Slice 2, where `Result`/
`Option` ship as ordinary `core`-module types consumed unqualified or via the existing
concrete-type import path.

## Codebase map

- `src/parser.rs:2091-2136` (`parse_typedef`) — struct field parsing; gains a header-level
  type-variable list and (R1) a per-field check that a bare variable reference resolves
  against that list.
- `src/parser.rs:2228-2299` (`parse_enum_typedef`, `parse_variant_fields`) — enum/variant
  field parsing; same header-level variable-list threading as the struct path.
- `src/parser.rs:2118-2136` (`parse_field_type_expr`) and the signature-side type parser
  reached from `parse_slot`/`parse_poly_slot` (`:1397`, `:1690`, `:1743`) — both gain the
  explicit generic-type-application syntax (R2): a generic-type name followed by its
  argument type-expressions.
- `src/parser.rs:2151-2170` (`resolve_type`) — today resolves a name to a concrete `Type`
  via `resolve_type_name_in_module`; gains the lookup-and-instantiate path for a generic
  name plus explicit arguments (R2/R4), returning the monomorphized `Type::Struct`/
  `Type::Enum`.
- `src/ast.rs:207-273` (`StructDecl`, `EnumDecl`) — unchanged in shape (decision: stays
  concrete-only, mirrors `PolyType` living apart from `Type`); a new declaration-time
  registry (e.g. `GenericStructDecl`/`GenericEnumDecl`, each holding `ty_var_names:
  Vec<String>` and a `PolyType`-shaped field list) is added alongside `Module::structs`/
  `enums`.
- `src/ast.rs:525-548` (`intern_bundle_struct`) — the closest existing template for
  structural dedup-and-mint, but keyed on structure not name+variables; the new
  instantiation table is a distinct function/table, not an extension of this one (recon 3
  in the brief).
- `src/ast.rs:623-651` (`PolyType`), `:663-676` (`PolySig`), `:679-696` (`Subst`),
  `:729-747` (`instantiation_symbol`) — the direct templates: a generic type's field list
  is `PolyType`-shaped like a `PolySig`'s inputs/outputs; a use site's concrete type
  arguments form a `Subst`-like key; the monomorphized name is minted the same
  deterministic, sanitized way `instantiation_symbol` mints a word's mangled symbol.
- `src/check/declarations.rs:2322-2360` and surrounding tests — struct/enum constructor
  and accessor checks (`Vec2` constructor arity/type checks, `Vec2>x` accessor checks);
  confirmed to resolve directly against a concrete `StructId`'s field list, so R5 requires
  no new code here — only that monomorphization produces a `StructDecl` these checks can
  already see.
- `src/check.rs:14` (imports from `ast.rs` used across the checker) — gains the new
  instantiation-table type and any accessor functions it needs (e.g. `intern_generic_struct`/
  `intern_generic_enum`, mirroring `intern_bundle_struct`'s naming).

## Open questions

None blocking; all four raised in the brief are settled above (D1-D4). If implementation
surfaces a fifth (e.g. exact syntax for a multi-argument application: `Pair i64 bool` vs a
bracketed `Pair[i64 bool]`), prefer the bare juxtaposed form — it matches how a word's
concrete call arguments are already juxtaposed with no bracketing, and how `'T 'E` are
juxtaposed in a `type:` header itself (R1/R2).

## Solution approach (advisory)

1. Add a generic-declaration registry (`GenericStructDecl`/`GenericEnumDecl` or an
   equivalent `ty_vars`-carrying variant of the existing decl types), populated by the
   pre-pass when a `type:` header's name is followed by one or more `'`-prefixed tokens
   before the first field name.
2. Parse the header's type variables into a local name->index table scoped to that
   declaration; parse each field's type through a `PolyType`-shaped path (reusing
   `RawTy`-style folding from Phase 4 Slice 1 where possible) so a bare variable reference
   resolves against that table and a bare unresolvable name is still a located error.
3. Add the explicit application syntax at `parse_field_type_expr` and the signature-slot
   type parser: a resolved generic-type name followed by exactly `ty_vars.len()` further
   type-expressions (R2/R3).
4. Add the instantiation table: `(generic name, module, Vec<Type> args) -> StructId/EnumId`,
   structurally deduped (R4), populated lazily the first time a use site's arguments are
   all concrete — mirroring `intern_bundle_struct`'s "call site mints, checker reads back"
   shape but keyed nominally rather than structurally-only.
5. When minting a new instantiation, substitute the generic declaration's `PolyType` field
   list against the concrete arguments to produce an ordinary concrete `StructDecl`/
   `EnumDecl`, push it into `Module::structs`/`enums` exactly like any hand-written
   concrete `type:` (R5) — no new accessor/constructor/destructor path.
6. Golden tests: two distinct instantiations of one generic struct with distinct,
   correctly laid-out fields and working accessors; the same instantiation used twice
   dedupes to one `StructId` (direct assertion, mirroring
   `intern_bundle_struct_same_tuple_dedups_expected`); a missing-argument and an
   extra-argument use site each produce the R3 located error; existing concrete `type:`
   goldens are unaffected.

## Success criteria (observable)

- A generic `type:` declaration with at least one type variable monomorphizes per
  distinct concrete instantiation: two applications with different concrete arguments
  produce two distinct `StructId`s (or `EnumId`s) with correct field layout, verified by a
  golden `.sth` program that constructs and reads back both.
- The same concrete application used twice resolves to one `StructId`/`EnumId` (a direct
  unit-test assertion, not merely "the program runs").
- A use site with the wrong number of type arguments is a located compile error naming the
  generic type, the expected argument count, and the supplied count.
- All pre-existing tests (`parse_typedef_*`, `check_struct_*`, `check_enum_*`, and every
  golden `.sth` file) pass unchanged.
- Every new test is mutation-tested: reverting the code it guards must make it fail (the
  project's standing placebo-test hazard — see `CLAUDE.md`/memory on mutation-testing
  guards).

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Generic type: header parsing (struct + enum) and the type-variable-scoped field list, with no instantiation yet",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Explicit generic-type-application syntax at field-type and signature-slot positions, plus the instantiation table minting concrete StructId/EnumId with structural dedup",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Wiring monomorphized instantiations into the existing layout/accessor/constructor/destructor machinery, arity/argument-count error diagnostics, and golden test coverage",
      "difficulty": "standard"
    }
  ]
}
```
