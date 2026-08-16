# Phase 5 Slice 1: generic `type:` declarations (spec)

## Problem statement

A `type:` header today names one concrete struct or enum: every field type resolves
through `resolve_type` (`src/parser.rs:2171`) to a plain `Type`, and `StructDecl`/
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
declares, and the number of arguments supplied. **This check belongs in the same phase as
the application syntax itself (Phase 2, not Phase 3):** the argument count is only known
at the parse/resolve site the application syntax adds, so deferring only the diagnostic to
a later phase would leave Phase 2 shipping an application parser with no defined behavior
on a bad count (see Phases section).

**R4 — distinct concrete applications monomorphize to distinct `StructId`/`EnumId`s; the
same application dedupes to one.** `Box i64` and `Box bool` mint two different, correctly
laid-out concrete struct registry entries. `Box i64` used a second time anywhere in the
same module resolves to the *same* `StructId` as the first use — structural dedup on
`(generic name, concrete type arguments)`, mirroring how `intern_bundle_struct`
(`src/ast.rs:525`) dedups a bundle shape and how a polymorphic word's `Subst`-keyed
`CallInst` (`:709`) dedups a call-site instantiation.

**R5 — a monomorphized instantiation behaves exactly like a hand-written concrete
`type:` of the same shape, once its generated words are registered without colliding.**
Once `Box i64` is minted as a `StructId`, its layout, destructor synthesis, and every
existing struct/enum check (`src/check/declarations.rs`) apply unchanged — they operate
on the concrete `StructDecl` the registry holds, not on anything specific to how that
entry was declared. **Corrected during review (round 1, B1):** the earlier claim that
accessor/constructor dispatch needs "no new logic" is false as stated. The generated
constructor/destructure/accessor `Sig`s (`struct_generated_sigs`/`enum_generated_sigs`,
`src/check/declarations.rs:1175`/`:1223`) are keyed by the *bare* `decl.name` (e.g.
`"Box"`, `"Box>val"`) and registered via `env.insert(name, vec![Overload{..}])`
(`src/check.rs:449-454`) — an **overwrite**, not an append. Two instantiations that both
set `decl.name == "Box"` (`Box i64`, `Box bool`) would silently clobber each other's
constructor/accessor entries.

The fix stays inside the existing overload-resolution machinery, so it is not new
dispatch logic in the sense R5 originally meant: `struct_generated_sigs`/
`enum_generated_sigs` registration changes from `env.insert(name, vec![Overload{..}])` to
the same `env.entry(name).or_default().push(Overload{..})` pattern the user-word path
already uses (`src/check.rs:520`). Every instantiation's generated words keep the bare
spelling (`Box>val`, `1 Box`), living as additional overloads under that name; the
already-existing operand-type overload resolution (used today for user-word overloads)
disambiguates `Box i64`'s accessor from `Box bool`'s by receiver type at the call site,
exactly as it disambiguates any other overloaded word. This is the one concrete change
R5 requires; no other accessor/constructor/destructor logic changes.

**R6 — a generic type's own field may reference a concrete application of another (or the
same) generic type non-recursively.** `type: Wrap x Box i64 ;` (R2) covers the concrete
side. A generic type applying *another* generic type inside its own field list using its
own still-open variable (`type: Outer 'T x Box 'T ;`) is explicitly **out of scope**
(decision D1): the exit case is a flat generic type whose fields are concrete or
variable-only, never a nested open application.

## Non-functional requirements

- **No new `Type` variant.** A generic type's *declaration* carries a variable-bearing
  field list in a new declaration form; a concrete `Type::Struct`/`Type::Enum` is minted
  only once every type argument at a use site is concrete, mirroring Phase 4 Slice 1's own
  rule that `Type` gains no variable-carrying variant (`src/ast.rs:619-621`).
- **Deterministic, order-independent mangled names.** The synthesized name for a
  monomorphized instantiation's QBE-facing symbol reuses `instantiation_symbol`
  (`src/ast.rs:729`)'s own sanitize-and-join scheme directly (`sooth_mono_{name}__t{id}_{ty}`-style),
  not a new ad hoc format: a pure function of `(generic name, concrete type arguments)`,
  with no dependence on processing order. `decl.name`/`decl.name_static` for the minted
  `StructDecl`/`EnumDecl` uses this mangled string (leaked to `'static` via `Box::leak`,
  matching `StructDecl::name_static`'s existing obligation, `src/ast.rs:207`); the
  generated words' *env keys* stay the bare surface spelling (`Box>val`) per R5's fix, so
  the mangled name is purely the registry/QBE-symbol identity, never user-visible.
- **`module: u32` is set on every minted declaration.** `StructDecl`/`EnumDecl`/
  `VariantDecl` all carry a `module` field (`src/ast.rs:207-232`); D4 fixes it to the
  instantiating module's id (`0` under this slice's single-module scope) but the mint
  step must set it explicitly, not leave it defaulted.
- **No regression to existing concrete `type:` declarations.** Every existing golden
  `.sth` file and `parse_typedef_*`/`check_struct_*`/`check_enum_*` test continues to pass
  unchanged; a concrete (non-generic) `type:` header parses exactly as it does today.

## Scope and boundaries

**In scope:** parsing a generic `type:` header (struct and enum forms) with one or more
type variables; an explicit generic-type-application syntax at a field-type position
(`parse_field_type_expr`) and at a word-signature slot position (`parse_slot`/
`parse_poly_slot`, delegating to `parse_type_expr`, `src/parser.rs:1834`, for the actual
type-expression parse); a monomorphization/instantiation table keyed by `(generic name,
concrete type arguments)` producing a `StructId`/`EnumId`, minted through a `&mut`-threaded
side registry (see D5 below), not from inside the read-only `resolve_type`; wiring that
table so an instantiation lands in the ordinary `Module::structs`/`enums` registries the
existing layout, accessor, constructor, and destructor machinery already walks, with the
generated-word registration fix from R5.

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
consumer needs it. **Corrected during review (round 1): this does not mean Slice 2 is
unaffected.** ROADMAP.md's own Phase 5 exit criterion states `Option['T]` must be
"importable from `core`" — importing a generic declaration from one module and
instantiating it in another (`Option i64` in a consumer module, with `Option` declared in
`core`) *is* cross-module generic instantiation, and cannot ride the existing
concrete-type import path (which only ever imports an already-concrete type). Slice 1
itself stays single-module (no change to this slice's scope), but Slice 2's own spec must
treat cross-module generic import as a named prerequisite it either builds itself or pulls
forward from here — it is not free.

**D5 (new; settles reviewer B2) — an instantiation is minted through a `&mut`-threaded
side registry populated during body parsing, mirroring `intern_array_type`/
`intern_ref_type`, not from inside `resolve_type` and not at check-time like
`intern_bundle_struct`.** `resolve_type` (`src/parser.rs:2171`) takes `&self` over
`structs: &'t [StructDecl]` (`Parser`'s own field, `src/parser.rs:965`) — an immutable
borrow that cannot mint a new `StructId`. `intern_bundle_struct` mints at check-time,
after parsing, which is too late for a struct *field*'s type (resolved during
`parse_field_type_expr`, at parse time). The correct model is `intern_array_type`
(`src/ast.rs`): a `&mut Vec<StructDecl>`-style instantiation registry threaded through
`parse_bodies` alongside the existing `arrays`/`owned_cells`/`refs` registries, appended
into `Module::structs`/`enums` (`StructId` computed from the pre-pass length plus the
instantiation registry's own growing offset) before `check`'s `struct_generated_sigs`/
`enum_generated_sigs`/layout pass runs over the assembled module.

## Codebase map

- `src/parser.rs:2091-2136` (`parse_typedef`) — struct field parsing; gains a header-level
  type-variable list and (R1) a per-field check that a bare variable reference resolves
  against that list.
- `src/parser.rs:2228-2299` (`parse_enum_typedef`, `parse_variant_fields`) — enum/variant
  field parsing; same header-level variable-list threading as the struct path.
- `src/parser.rs:2119-2136` (`parse_field_type_expr`), `src/parser.rs:1777` (`parse_slot`),
  `src/parser.rs:1417` (`parse_poly_slot`), and `src/parser.rs:1834` (`parse_type_expr`,
  the shared signature-side type-expression resolver `parse_slot` delegates to) — all four
  gain the explicit generic-type-application syntax (R2): a generic-type name followed by
  its argument type-expressions.
- `src/parser.rs:2171` (`resolve_type`) — today resolves a name to a concrete `Type` via
  `resolve_type_name_in_module`; gains the lookup-and-instantiate path for a generic name
  plus explicit arguments (R2/R4) — but per D5, the actual mint happens through the new
  `&mut`-threaded instantiation registry passed alongside, not inside `resolve_type`'s own
  `&self` body.
- `src/ast.rs:207-273` (`StructDecl`, `EnumDecl`) — unchanged in shape (decision: stays
  concrete-only, mirrors `PolyType` living apart from `Type`); a new declaration-time
  registry, settled as a **separate `GenericStructDecl`/`GenericEnumDecl` pair** (not a
  `ty_vars` field bolted onto the existing decl types — keeping the concrete registries
  untouched-in-shape is exactly the point), each holding `ty_var_names: Vec<String>` and a
  `PolyType`-shaped field list, added alongside `Module::structs`/`enums`.
- `src/ast.rs:525-548` (`intern_bundle_struct`) — the closest existing template for
  structural dedup-and-mint, but keyed on structure not name+variables; the new
  instantiation table is a distinct function/table, not an extension of this one (recon 3
  in the brief).
- `src/ast.rs:623-651` (`PolyType`), `:663-676` (`PolySig`), `:679-696` (`Subst`),
  `:729-747` (`instantiation_symbol`) — the direct templates: a generic type's field list
  is `PolyType`-shaped like a `PolySig`'s inputs/outputs; a use site's concrete type
  arguments form a `Subst`-like key; the monomorphized name is minted the same
  deterministic, sanitized way `instantiation_symbol` mints a word's mangled symbol.
- `src/check/declarations.rs:1175` (`struct_generated_sigs`), `:1223`
  (`enum_generated_sigs`) — the actual constructor/destructure/accessor `Sig` synthesis,
  keyed by `decl.name`; unchanged in logic, but its registration site (below) must change.
  (`src/check/declarations.rs:2322-2360` are the *tests* exercising this path, not the
  synthesis itself — useful as confirming evidence, not as the edit site.)
- `src/check.rs:449-454` — struct/enum generated `Sig`s registered into `env` via
  `env.insert(name, vec![Overload{..}])` (overwrite); per R5's fix, changes to
  `env.entry(name).or_default().push(Overload{..})` (matching the user-word registration
  at `src/check.rs:520`), so two instantiations sharing a bare accessor/constructor name
  become two overloads disambiguated by operand type instead of clobbering each other.
- `src/check.rs:14` (imports from `ast.rs` used across the checker) — gains the new
  instantiation-table type and any accessor functions it needs (e.g. `intern_generic_struct`/
  `intern_generic_enum`, mirroring `intern_bundle_struct`'s naming, but with D5's `&mut`
  parse-time threading rather than check-time interning).

## Open questions

None blocking; all four raised in the brief are settled above (D1-D4), plus D5 (parse-time
minting site) settled during round-1 review. If implementation surfaces a further question
(e.g. exact syntax for a multi-argument application: `Pair i64 bool` vs a bracketed
`Pair[i64 bool]`), prefer the bare juxtaposed form — it matches how a word's concrete call
arguments are already juxtaposed with no bracketing, and how `'T 'E` are juxtaposed in a
`type:` header itself (R1/R2).

**Flagged for the user, not blocking this slice:** D4's correction means Slice 2's own
spec (not this one) must explicitly plan for cross-module generic instantiation to meet
ROADMAP's "`Option` importable from `core`" exit criterion — either as an in-scope line on
Slice 2 or a small prerequisite slice ahead of it. No action needed now; noting it so it
isn't silently rediscovered when Slice 2 is briefed.

## Solution approach (advisory)

1. Add the generic-declaration registry (`GenericStructDecl`/`GenericEnumDecl`, per the
   codebase map's settled representation), populated by the pre-pass when a `type:`
   header's name is followed by one or more `'`-prefixed tokens before the first field
   name. State explicitly what a declared-but-never-instantiated generic type does to a
   whole-program build: it must compile clean (the new registry is walked by nothing
   `check`/`lower` touch until an instantiation appends into `Module::structs`/`enums`),
   giving Phase 1 a defined stopping point with no dependency on Phase 2/3.
2. Parse the header's type variables into a local name->index table scoped to that
   declaration; parse each field's type through a `PolyType`-shaped path (reusing
   `RawTy`-style folding from Phase 4 Slice 1 where possible) so a bare variable reference
   resolves against that table and a bare unresolvable name is still a located error.
3. Add the explicit application syntax at `parse_field_type_expr` and the signature-slot
   type parser: a resolved generic-type name followed by exactly `ty_vars.len()` further
   type-expressions (R2/R3).
4. Add the instantiation table: `(generic name, module, Vec<Type> args) -> StructId/EnumId`,
   structurally deduped (R4), threaded as a `&mut` side registry through `parse_bodies`
   per D5 (mirroring `intern_array_type`'s mutation discipline, not `intern_bundle_struct`'s
   check-time one), populated the first time a use site's arguments are all concrete; the
   wrong-argument-count diagnostic (R3) lives here too, in this same phase.
5. When minting a new instantiation, substitute the generic declaration's `PolyType` field
   list against the concrete arguments to produce an ordinary concrete `StructDecl`/
   `EnumDecl` (mangled `name`/`name_static` per the NFR, `module` set per D4), push it
   into `Module::structs`/`enums` exactly like any hand-written concrete `type:` (R5) — no
   new layout/destructor path, and the one generated-word registration change from R5
   (insert -> overload-append).
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
- A field naming a variable its `type:` header did not bind (R1) is a located parse
  error, asserted directly.
- A generic-type application at a **word-signature slot** (`: unwrap ( Box i64 -- i64 )
  ;`, R2), not only at a struct field, parses and resolves — a distinct golden from the
  field-position case, since it is a distinct parser call site (`parse_slot`/
  `parse_poly_slot` vs `parse_field_type_expr`).
- A monomorphized instantiation's destructor is synthesized and runs like a hand-written
  concrete type's (R5), not merely constructed and read back.
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
      "focus": "Explicit generic-type-application syntax at field-type and signature-slot positions (parse_field_type_expr, parse_slot/parse_poly_slot via parse_type_expr), the parse-time-threaded instantiation table minting concrete StructId/EnumId with structural dedup, and the wrong-argument-count located error at the same site",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Fix generated-word registration from overwrite to overload-append (struct_generated_sigs/enum_generated_sigs into env), wiring monomorphized instantiations into the existing layout/destructor machinery, and golden test coverage including the signature-slot and destructor witnesses",
      "difficulty": "standard"
    }
  ]
}
```
