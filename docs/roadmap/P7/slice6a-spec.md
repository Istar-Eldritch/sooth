# P7.S6a -- Length parameters in `type:` headers, and the `Kind` type

**Status:** Implemented (6 phases plus one post-implementation review fix).
**Discovery:** `docs/roadmap/P7/slice6a-brief.md`

`type: Buffer['T 'N: Len] data array['T 'N] ;` now parses, instantiates per
length, unifies in a signature, lowers, and dispatches through `impl:`. A word
signature already carried length variables (`PolySig::len_var_names`,
`PolyType::Array(_, Len::Var)`); nothing on the *header* path did -- neither
the declaration parser, the field-substitution/instantiation machinery, the
two (in fact three) application parsers, check-time binding, nor lowering.

## Rulings

- **R1, `Kind`.** The parser-private `VarKind { Ty, Len }` was renamed in place
  to `Kind { Star, Len }` (`Star` replaces both `Ty` and the implicit "no kind"
  of `ty_var_names`). No `Const` (DESIGN.md's "dependent types: never") and no
  `Arrow` (P7b).
- **R2, header bracket syntax.** `parse_header_bracket` returns
  `(name, span, Kind)`; `'N: Len` (colon glued or spaced, mirroring a bound's
  `'T: Copy`) interns a length variable, a bare `'T` is `Star`. An annotation
  naming anything but `Len` is located; a name used at both kinds in one header
  is located (the header twin of `var_kind_conflict_error`).
  - **R2.1, at least one type variable.** A length-only header (`Buf['N: Len]`)
    is rejected, ruled out rather than supported: four sites in `src/check/poly.rs`
    (`collect_concrete_positions`/`collect_paired_positions`'s `Struct`/`Enum`
    arms) read an empty type-arg list as "not generic here", a convention that
    only holds because a zero-type-variable header was previously unconstructible.
    Lifting this means fixing those four sites first.
  - **R2.2, `Len` is reserved for `kind == "trait"`.** Entirely inside
    `reject_reserved_name`; `parse_trait_decl` already called it. Without it a
    user-declared `trait: Len` would be silently unreachable behind R2b's
    bracket intercept.
- **R2a, length-carrying fields.** `parse_generic_field_array` resolves a
  `'`-prefixed count against the header's own length list (resolve, never mint:
  the bracket already bound it; an unresolvable `'N` is the length twin of
  `unbound_generic_ty_var_error`). `parse_generic_field_application` -- the
  third, previously-unnamed application parser, for a header applied inside
  another header's field (`type: Pair['T 'N: Len] a Buffer['T 'N] ;`) -- got
  R6/R7's arity split and length-aware collapse gate one level down.
  `GenericHeader` and the `parse_generic_typedefs` accumulator carry two
  variable lists; `resolve_field_len_var`/`used_len` mirror the type-variable
  bookkeeping, and `check_no_phantom_len_var` is the phantom twin, called from
  the enum variant loop too. `generic_field_type_str` gained a `len_vars`
  parameter: its `Len::Var(_) => unreachable!()` arm was a *reachable* panic,
  since `parse_generic_field_array` calls the renderer unconditionally, not
  only on error.
- **R2b, word bound-bracket `'N: Len`.** Validation only, not a new interning
  path: a bare `'N` in an effect already interns through
  `PolyBuilder::intern_len_var`. The bracket entry tuple carries an
  `is_len_kind` flag (built without calling `parse_capabilities`, so `Len` never
  becomes a fake `Bound`), and `attach_bracket_bounds` looks such an entry up in
  `sig.len_var_names`; an absent name is the twin of `bracket_var_unused_error`.
- **R3, AST fields and their compile-forced ripple.**
  `GenericStructDecl`/`GenericEnumDecl` gained `len_var_names`;
  `PolyType::Generic`, `PolyType::GenericVariant` and `Operative::Generic`
  gained `len_args: Vec<Len>`. The carry-forward sites that needed a *specific*
  value rather than a placeholder: `ground_member_poly`'s `Generic` arm (clone,
  never ground -- a `Len` is not a `PolyType`), the three diagnostic renderers
  (`poly_type_shape_str`, `generic_field_type_str`, `poly_type_str`),
  `Operative::Generic`'s construction site, and `poly_destructure_generic`'s
  `enum_sites` push. Everything else forwards or takes `vec![]`; `cargo build`
  was the completeness gate.
- **R4, `substitute_generic_field`.** The `Array` arm resolves `Len::Var(i)`
  from the instantiation's length list exactly as `Var(v)` resolves a type; the
  `Generic` arm substitutes a *nested* application's own `len_args` and passes
  them down (the shape R2a's field-application case produces).
- **R5, instantiation plumbing.** `instantiate_struct`/`instantiate_enum` take
  a length list; `struct_keys`/`enum_keys` widened to
  `(usize, u32, Vec<Type>, Vec<Len>)` so `Buffer[u8 256]` and `Buffer[u8 512]`
  mint distinct monomorphs; `type_instantiation_name` renders the lengths. A
  zero-length-arg call is byte-identical, so no existing symbol moved.
  `struct_instantiation_of`/`enum_instantiation_of` kept their *public*
  signature until R8, which is what deferred their six consumers cleanly.
- **R6/R7, use sites.** `parse_type_arguments` splits a concrete application's
  bracket into `0..ty_arity` type expressions then `ty_arity..+len_arity` count
  literals (`1..=u32::MAX`, `parse_array_count`'s range); `generic_arity_error`
  reports the two counts separately. On the signature path, `RawTy::Generic`
  gained `Vec<RawLen>`, and the eager-concrete collapse now requires every
  *length* arg concrete too -- otherwise `Buffer[i64 'N]` wrongly collapses to
  a concrete struct with nowhere to place `'N`. `parse_impl_target` routes
  through the same fold, so `impl: Show for Buffer['T 4]` parses with no
  separate ruling.
- **R8a, unification and lowering.** `unify_poly_input`'s `Generic` arm binds
  `len_args` from a concrete instantiation's recovered lengths, mirroring its
  neighbouring `Array` arm; `apply_subst`'s `Generic` *and* `GenericVariant`
  arms resolve them through `subst.len`; `subst_polytype` mirrors that before
  `lookup_struct`/`lookup_enum`, which now match `&[Len]` alongside `&[Type]`.
  Lowering was load-bearing, not cosmetic: without it a length-carrying
  monomorph resolves to the wrong instantiation or trips
  `subst_polytype`'s own `.expect`. `poly_mentions_len_var`'s `Generic` arm
  also scans `len_args`, keeping poly-body cross-calls to a length-carrying
  callee rejected.
- **R8b, impl-target matching and specificity.** `match_impl_target_rec` zips
  and matches `len_args`; `collect_positions`/`collect_concrete_positions`/
  `collect_paired_positions` push `Position::LenConcrete`/`LenVar` per length
  arg. **Ordering convention: type positions (declaration order), then length
  positions (declaration order)**, matching the `Array` arm's element-then-count
  shape; a mismatch between a pattern walk and its concrete walk silently
  aligns a type position against a length position. `generic_len_args_of` is the
  new length twin of `generic_args_of` (the plumbing R8b assumed existed).

## Phases as landed

1. `7ec422a`/`50e0284` -- R1, R2, R2.1, R2.2.
2. `bec373c`/`c4b242b` -- R2a's array-field sub-case and plumbing; R2b.
3. `d488e82` -- R3, R4, R5, R2a's field-application sub-case. Three review
   cycles; see **Soundness gap found in review** below.
4. `f1cfbb5` -- R6/R7. Review round 1 blocked: the collapse-gate's only test
   (`Buffer['T 4]`) was a placebo, killed by the *pre-existing* type-args-only
   concreteness check; the real witness is `Buffer[i64 'N]` (concrete type,
   variable length). Removed phase 3's now-dead placeholder rejection.
5. `b20d7e1` -- R8a. `apply_subst`'s `GenericVariant` length resolution was
   correct but unwitnessed (the existing test used a length-less header).
6. `a20e531` -- R8b plus the integration goldens. A unit test
   (`specificity_struct_header_length_positions_are_not_ignored`) was a
   confirmed placebo: two *concrete*-length impls never compete at specificity,
   because `match_impl_target_rec` rejects the length mismatch first. Rewritten
   around the scenario that does compete: a length-*variable* pattern against a
   length-concrete one, both matching the same concrete operand.
7. `106bc9c` -- post-implementation review fix, below.

### The P0 found after implementation

`poly_cross_match`'s `Generic`/`Generic` arm guarded on
`(is_enum, idx, module, args.len())` and zipped `args` only. The spec had
explicitly ruled it safe *because* `poly_mentions_len_var`'s widening made a
length-carrying cross-call unreachable -- but that widening only matches
`Len::Var`, and R7 made a **concrete** length spellable in a header. So a
poly-body cross-call from a caller over `Buffer['T 8]` into a callee over
`Buffer['T 4]` slipped through: reproduced both as a silent accept plus
miscompile (exit 0, value from the wrong monomorph) and as an ICE at
`subst_polytype`'s checked `.expect` when only one monomorph existed. Fixed by
comparing `len_args` in the arm's guard, mirroring the neighbouring `Array`
arm's `dl == sl`. Lesson: "safe because an upstream guard makes this
unreachable" is only as strong as the guard's own predicate.

### Soundness gap found in review (phase 3)

`substitute_generic_field`'s length lookup was reachable with an empty length
list via `resolve_type_or_apply` and `parse_poly_generic_application`, both of
which instantiated a length-declaring header with a placeholder before R6/R7
existed -- a live ICE. Closed by rejecting a length-declaring header on both
paths (`generic_length_application_unsupported_error`) until phase 4 supplied
the real parsing, then deleting the rejection.

## Tests

Unit tests sit beside their stage code in `src/parser.rs`, `src/ast.rs`,
`src/check/poly.rs`; the five integration goldens are `tests/phase7_slice6a.rs`
(exit dogfood printing `256`; distinct-length types; a poly-body cross-call
rejection; the mismatched-`'N` rejection; and non-overlapping
`impl: Show for Buffer['T 4]`/`['T 8]` dispatching to distinguishable per-impl
constants). Test names diverged from this spec's planned list during
implementation; grep the sources, not this doc.

Two fixture shapes are load-bearing and easy to regress:

- The signature-unification golden carries `'N` in a **second, bare-array
  parameter**, not by projecting into `Buffer`'s field: aggregate projection out
  of a `PolyType::Generic` receiver is rejected in a non-inline generic body,
  and the `inline` rescue folds `len` off an already-concrete monomorph without
  ever consulting `'N`'s binding.
- The discriminating golden is the **negative** one (mismatched concrete lengths
  for one `'N` must be rejected): the positive case stays green under a
  length-blind `unify_poly_input`.

The mangled-symbol migration claim rests on two pre-existing tests
(`type_instantiation_name_unambiguous_struct_arg_stays_bare`,
`instantiation_symbol_reproduces_native_spelling_expected`), not a new one.

## Known gaps and limitations

- **Constructing a length-carrying header from a generic body panics.**
  `poly_bind_construction_arg`'s "a generic type: field is never {other:?}"
  `unreachable!` catch-all (`src/check/poly.rs:4355`) restricts construction
  inference to `Var`/`Concrete` fields, so any array-shaped field already
  panicked here before this slice. `poly_construct_generic` therefore passes a
  **permanent** empty length list, not a deferred placeholder; a future
  length-inference-at-construction extension starts here.
- **`substitute_generic_field`'s length lookup is unchecked**
  (`lens[*v as usize]`, `src/ast.rs:866`), safe only via the upstream invariant
  that every reaching path supplies a full length list. See the phase-3 gap
  above for what happens when that invariant lapses.
- **Destructuring a generic enum with an array-shaped variant field ICEs.**
  `substitute_generic_variant_field` (`src/ast.rs:2216`) panics with "a generic
  enum variant field is never Array(..)"; its claim that `array['A N]` is a
  parser rejection for a variant field is false, and was already false at the
  base ref (verified: `type: Ring['T] | Full data array['T 4] | Empty c 'T ;`
  plus an eliminating word panics identically at `a1ed36d` and at HEAD). This
  slice does not regress it, but it widens the reachable shapes: a variant field
  can now carry `Len::Var` too. Not fixed here.
- Lowering's `subst_polytype` length path is witnessed by the integration
  goldens, not by a dedicated unit test.
- `docs/roadmap/P7-language-prereqs.md` still marks S6a `[ planned ]`.

## Out of scope

- **Generic-length array indexing in a non-inline body** (`&>` on
  `array['T 'N]`; `poly_generic_length_index_error` stands, workaround is
  `inline`). This is why the impl-dispatch golden distinguishes by a hardcoded
  per-impl constant, not an indexed element.
- **Length-only headers** (R2.1), **non-length const kinds**, and the
  **`Arrow` kind** (P7b).
- **S6b**: explicit length arguments at a *word* call site (`sum[i64 4]`),
  seeding `subst.len` in `check_poly_call`. A different mechanism from binding a
  length out of a header type an operand already carries.
- `tree-sitter-sooth/grammar.js` and `docs/book/`.
