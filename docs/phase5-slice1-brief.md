# Phase 5 Slice 1: generic `type:` declarations (brief)

Today a `type:` header names a concrete struct or enum and every field type is resolved
through `resolve_type`/`parse_field_type_expr` to a plain `Type` (`src/parser.rs:2091-2136`,
`:2228-2299`); there is no notion of a type parameter anywhere in `StructDecl`/`EnumDecl`
(`src/ast.rs:207-273`), both of which store `fields: Vec<(String, Type)>` with no variable
slot. This slice adds that: a `type:` header parameterized by one or more `'T`-style type
variables (mirroring Phase 4 Slice 1's `PolyType`/`PolySig` for words), one `StructId`/
`EnumId` minted per concrete instantiation the way a polymorphic word already monomorphizes
per call site, plus the per-instantiation generated words (field accessors, variant
constructors, destructor) that concrete usage needs. No specific generic type ships in this
slice — `Result`/`Option` are Slice 2, built on this. The exit witness is a throwaway
generic struct or enum, not either of them.

## Recon (measured against the built compiler, 2026-08-16, `main` at `a562be8`)

`cargo test` is green at this HEAD. Claims below are read from source, not inferred.

1. **Nothing in the type-declaration path knows about a variable.** `parse_typedef`
   (`src/parser.rs:2091`) and `parse_enum_typedef`/`parse_variant_fields`
   (`:2228`, `:2270`) call `parse_field_type_expr` (`:2118`) for every field, which bottoms
   out in `resolve_type` (`:2151`) — a lookup into `self.structs`/`self.enums` via
   `resolve_type_name_in_module`, with no branch for a bare `'T` token. A field type is
   parsed by `expect_field_type_token` (`:2141`), an ordinary word token; nothing distinguishes
   `'T` from any other unresolvable name today, so `type: Box 'T | val 'T ;` currently fails
   with `error: unknown type 'T`.

2. **The struct/enum registries carry no variable slot and no instantiation key.**
   `StructDecl`/`EnumDecl` (`src/ast.rs:207`, `:260`) store only concrete `fields`; there is
   no `PolySig`-equivalent (no `ty_var_names`, no per-field `PolyType`). `StructId`/`EnumId`
   (`:230`, `:277`) are bare registry indices with no `Subst` attached, unlike `CallInst`
   (`:709-721`), which pairs a callee name with a `Subst` and a mangled `symbol`. There is
   today no way to ask "which struct is `Box<i64>`", because no struct is `Box` un-instantiated
   in the first place — the pre-pass that registers a `type:` name
   (referenced by `parse_enum_typedef`'s doc comment, "already registered by the pre-pass")
   registers one concrete name per declaration, one-to-one.

3. **The monomorphization pattern to mirror lives in `PolySig`/`Subst`/`instantiation_symbol`
   (words), not in `intern_bundle_struct` (bundles).** `PolyType`
   (`src/ast.rs:623-651`) is the per-signature variable-bearing type; `Subst`
   (`:679-696`) is the ground substitution a call site unifies to; `instantiation_symbol`
   (`:729-747`) mints a deterministic mangled name from `(word, θ, generation)`; `CallInst`
   (`:709`) is the per-call-site record the checker writes and lowering reads back
   (`src/check.rs:107-121`, `PolyCtx.insts: &mut HashMap<Span, CallInst>`). `intern_bundle_struct`
   (`:525-548`) is structural-dedup-only (an output tuple has no name and no variable), so it
   is the wrong template: this slice needs a *named*, *variable-parameterized* declaration
   dictionary keyed by `(name, module)`, plus a `Subst`-keyed instantiation table analogous to
   `CallInst` but producing a `StructId`/`EnumId` rather than a call-site symbol — a genuinely
   new table, `intern_bundle_struct` extended to accept variables would not fit (its dedup key
   is structural, not nominal).

4. **A concrete `Type` still has no variant for "unresolved generic type name" and must not
   gain one for the un-instantiated case.** `Type` (`src/ast.rs:850`) enumerates concrete
   shapes only (mirroring how `PolyType` — not `Type` — carries Phase 4 Slice 1's variable
   forms, per that slice's own R4 note at `:619-621`, "`Type` itself gains no variant").
   A generic type's *declaration* needs a `PolyType`-shaped field list (a `GenericStructDecl`/
   `GenericEnumDecl`, or a `ty_vars: Vec<String>` alongside a `PolyType`-typed `fields`); a
   *use site* (`Result i64 String`, a struct field of type `Vec i64`, a word signature
   mentioning `Option 'T`) still needs to resolve to a concrete `Type::Struct(StructId)`/
   `Type::Enum(EnumId)` once its type arguments are all concrete, exactly as a word call
   resolves to a concrete `IrFunc` symbol only once its `Subst` is ground.

5. **Where a use site's type arguments come from is a real design question, not a detail.**
   A polymorphic *word*'s type variables are resolved by unifying the declared signature
   against the concrete stack at the call site (`check_poly_body`, `src/check/poly.rs:248`) —
   there is a live value on the stack to unify against. A generic *type* has no such value at
   a **declaration** site: `type: Node 'T | val 'T | next ^Node 'T ;` used inside a word
   signature, `Vec 'T`, or a struct field must state its type arguments **explicitly**
   (`Vec i64`, not inferred from surrounding context), because there is nothing to unify
   against at parse time the way a call-site stack exists at check time. This slice therefore
   needs an explicit generic-type-application syntax parsed wherever a field/effect type is
   parsed today (`parse_field_type_expr`, and the signature-side type parser reached from
   `parse_slot`/`parse_poly_slot`, `:1397`/`:1690`/`:1743`), not a unification pass.

6. **Recursive generic types are already a shipped shape for the concrete case and must not
   regress.** `Type Node ( val 'T next ^Node 'T )`-style self-reference through `^` already
   works concretely (Phase 3 Slice 3, `parse_typedef_self_referential_field_resolves_to_own_type`,
   `src/parser.rs:2939`); a generic linked-list `type: List 'T | Nil | Cons val 'T next ^List 'T ;`
   is exactly this slice's hardest recursive case, and it is untested territory: a
   self-reference through a still-open generic name (`^List 'T` inside `List`'s own body)
   needs the pre-pass to register the *generic* name before any instantiation can occur, then
   resolve the self-reference against the same variable scope as the enclosing declaration.

## Decisions (settled here, not reopened by the spec)

1. **A generic `type:` declaration is stored once, as a template, alongside — not instead of
   — the existing concrete registries.** A new declaration kind (`GenericStructDecl`/
   `GenericEnumDecl`, or a `ty_vars: Vec<String>` field added to a variant of the existing
   decl that is only ever populated for a generic header) records the field list as
   `PolyType`, mirroring `PolySig`'s `ty_var_names`. `StructDecl`/`EnumDecl` themselves stay
   concrete-only (decision 4's "`Type` gains no variant" principle extends here): a
   *monomorphized instantiation* is an ordinary `StructDecl`/`EnumDecl` with concrete `Type`
   fields, minted into the existing registries the way `intern_bundle_struct` already mints
   into `structs`.

2. **Instantiation is keyed by `(generic name, Subst)`, mirroring `CallInst`, and produces a
   real `StructId`/`EnumId` an ordinary `Type::Struct`/`Type::Enum` can name.** A new table
   (module-level, alongside `Module::structs`/`enums`) records, per distinct concrete
   application, the minted id — deduping structurally so `Result i64 String` used twice
   yields one `StructId`, exactly as `intern_bundle_struct` dedups a bundle shape. This table
   is consulted at every explicit type-application site (decision 3) and is the single source
   of truth both the checker and lowering read, matching how `CallInst`/`instantiation_symbol`
   keep the checker and `ir::lower` from disagreeing about a word's mangled name.

3. **Type arguments are always explicit at a use site; there is no inference.** Recon 5's
   conclusion: `Vec i64`, `Option 'T` (inside another generic declaration, itself a variable),
   `Result i64 String` (in a word signature or a struct field) are the only shapes this slice
   parses. A bare generic name with no arguments (`type: Box | val 'T ;` field typed just
   `Box`, missing `'T`'s argument) is a located parse/arity error, not a shorthand for
   anything.

4. **Per-instantiation generated words are minted eagerly at instantiation, keyed by the same
   `Subst`-mangled symbol scheme `instantiation_symbol` already uses for words.** A struct's
   field accessors and an enum's variant constructors/destructor today are generated per
   concrete struct/enum (the existing, unconditional machinery for a monomorphic `type:`); a
   generic instantiation reuses exactly that generation path against the monomorphized
   `StructDecl`/`EnumDecl` decision 1 produces, so no new accessor-generation logic is
   written — only the trigger (instantiation, not declaration) changes.

## Open questions for the spec

- **OQ1 — instantiation trigger points.** Recon 5 established explicit type arguments are
  required, but not every syntactic position that can carry a type is equally reachable
  today: a struct field (`parse_field_type_expr`), a word signature slot
  (`parse_slot`/`parse_poly_slot`), and a *generic type's own field referencing another
  generic type* (`Cons val 'T next ^List 'T` — recon 6) are three distinct call sites in the
  parser. Does this slice cover all three, or only the first two, deferring "a generic type
  nested inside another generic type's fields" (e.g. `type: Tree 'T | Leaf | Node left ^Tree
  'T val 'T right ^Tree 'T ;` used as `Tree (Option i64)`) to Slice 2, where Result/Option
  are the first real consumers and can surface whether the nested case is actually needed?

- **OQ2 — how many type variables, and any bounds.** Phase 4 Slice 1 words support multiple
  type variables and `Bound` (`Copy`/`Ord`) constraints (`src/ast.rs:602-608`,
  `PolySig::bounds`). Does a generic `type:` header support more than one variable from the
  start (`Result 'T 'E` needs two), and do field-level bounds matter for Slice 1's own exit
  case, or is the exit witness deliberately single-variable, unbounded, leaving multi-variable
  and bounded generics to be proven by Slice 2's `Result 'T 'E` (which has no bound
  requirement either — recon shows no bound is needed for a plain data carrier)?

- **OQ3 — recursive generic self-reference (recon 6) sequencing.** Is `type: List 'T | Nil |
  Cons val 'T next ^List 'T ;` in this slice's exit case, or does the exit witness
  deliberately avoid recursion (a flat `type: Box 'T | val 'T ;` or `Pair 'A 'B`) and leave
  the recursive shape to whichever later slice actually needs it? DESIGN.md's disposal
  machinery already treats a concrete recursive type as a solved case (Phase 3 Slice 4); the
  question is only whether *this* slice re-proves that machinery still works once the
  variable is threaded through the self-reference, or whether a non-recursive exit case is
  sufficient to close Slice 1 and the recursive case is Slice 2's or a dedicated slice's
  problem (`List`/`Vec`-shaped types are Phase 6 territory per the ROADMAP; `Option`/`Result`
  are not recursive).

- **OQ4 — module-qualified generic names.** Phase 4 Slice 5 gave every type a `module: u32`
  field and qualified-name resolution (`resolve_type`'s `::` handling, `src/parser.rs:2196-2205`).
  Does a generic type imported across a module boundary (`import: v | Vec | "vec.sth" ;` then
  `v::Vec i64`) work in this slice, or is Slice 1's exit case single-module only, with
  cross-module generic import deferred as an explicit out-of-scope line (analogous to how
  Phase 5's own text scopes out the allocator-parameter slot)?

## Out of scope

- `Result`/`Option` themselves, `?` sugar (dropped from the phase entirely), and
  branch-on-result codegen: Slice 2.
- The default-allocator-parameter question (`Vec['T 'A = Global]`): Phase 6, per the
  ROADMAP's own text — this slice's exit case allocates nothing.
- Rebuilding the allocator's OOM trap to return `Option`/`Result`: a future consumer, not a
  consequence of this slice.
- `Vec`/`Map`/`String`/`Box`/`Rc`/`Arc` as real library types: Phase 6, though they are this
  mechanism's downstream consumers and should build on whatever this slice ships without
  rework.

## Sequencing

No gate from open Phase 4 items (the row-combinator quotation ICE and the borrow-liveness
fallback are quotation/row concerns, orthogonal to type declarations). Touches
`src/parser.rs` (the generic `type:` header, the explicit-type-argument syntax at every
field/signature-type site OQ1 settles), `src/ast.rs` (the generic declaration
representation, the instantiation table, a type-analog of `instantiation_symbol`), and
`src/check.rs` (wiring the instantiation table alongside the existing struct/enum layout
pass so a monomorphized instantiation gets accessors/constructors/destructor the same way a
hand-written concrete `type:` does today).

## Exit

A generic `type:` declaration with at least one type variable monomorphizes per concrete
instantiation the way a polymorphic word already monomorphizes per call site: two distinct
concrete applications (e.g. `Box i64` and `Box bool`) mint two distinct `StructId`s with
correctly laid-out concrete fields, generated accessors work on each, and the same
application used twice dedupes to one `StructId` (asserted directly, mirroring
`intern_bundle_struct_same_tuple_dedups_expected`). Golden test(s) cover the happy path and
at least one arity/argument-count error (a use site missing a required type argument).

## Ready to spec?

**Yes, with four open questions handed to the spec, none blocking.** The recon confirms the
ROADMAP's own framing: the layout half of generic instantiation has a direct model to copy
(`CallInst`/`Subst`/`instantiation_symbol`), and the declaration/resolution half is
genuinely unbuilt, not merely unexercised. OQ1 and OQ3 are the two most consequential —
they decide whether this slice's exit case is a flat non-recursive generic struct or
something closer to `Result`'s eventual shape — and should be settled toward the narrower
reading (no nesting, no recursion) unless the spec finds a reason Slice 2 needs the wider
one on day one.
