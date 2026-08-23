# Phase 7 Slice 3n: a generic type's field wrapping its own type variable (as shipped)

## Goal

A generic `type:`'s field may now wrap one of the declaration's own type variables:
`['T 2]`, `[['T 2] 2]`, `^'T`, `&'T`, `Ent['K 'V]`, and the enum twin (a named variant
field). Before this slice `parse_generic_field_type_expr` was a single `if` testing
whether the token *right here* was a bare `'`-word, so anything one token deeper fell
through to the concrete field parser and reported `unknown type 'T`. Three layers behind
it were broken independently: `substitute_generic_field` handled only `Concrete`/`Var`,
a generic header was registered only *after* its own field list had parsed, and
`instantiate_struct`/`instantiate_enum` substituted fields before minting the memo key.

The headline consumer shape builds: `type: Ent 'K 'V k 'K v 'V ; type: Map 'K 'V slots [Ent['K 'V] 8] ;`
instantiated at two asymmetric `('K, 'V)` pairs. This removes the array-field-of-own-type-variable
blocker on `Map`; the `'N` length-variable gap (friction #7) and the `Default`/`Eq`/`Hash`
bound gap (friction #6) remain, so this slice does not unblock `Map`.

## Landed shape

**Parser (`src/parser.rs`).**

- `parse_generic_field_type_expr` captures the field span, calls the new recursive
  `parse_generic_field_shape`, then runs R8's growth walk once over the finished tree
  (split so the walk is per *field*, not per node visited). Arms: array (nested to any
  depth), `&`/`&!`, `^` (including a `^^` run), generic application (`Ent['K 'V]`,
  `L['T]`, `L[i64]`). Every leaf `'name` resolves against the header's `ty_vars` and marks
  `used[idx]`, at whatever depth, so `check_no_phantom_ty_var` needed no change. A fully
  concrete field still falls through to `parse_field_type_expr` unchanged.
- **Glued sigils are peeled explicitly.** `^'T`, `^^'T`, `&'T`, `&!'T` each lex as a
  single `Word`, and a sigil glued to a generic header that is then applied (`^L['T]`,
  `&Ent['K i64]`) must be routed into the application production by hand. Without that,
  only the spaced spelling worked and the glued one fell to the concrete parser, blaming
  `'K` as an unknown type: exactly the misreport the arm exists to prevent.
- R7: the `[`-arm replicates `quotation_type_ahead`'s top-depth `--` scan, and
  `quotation_effect_ty_var_ahead` (bracket-depth tracked, so a nested `[` cannot hide the
  scan's end) rejects a quotation field naming a header variable via
  `quotation_field_ty_var_error`. A concrete quotation field still parses; `~[` reports
  `tilde_quotation_position_error`.
- R2: `parse_generic_typedefs` is two-stage. Stage (a) walks the whole token stream,
  parses each header alone (`parse_generic_header`), pushes a placeholder decl with an
  empty field/variant list, and records where that header's list starts. Stage (b)
  revisits each position and parses only the list
  (`parse_generic_typedef_fields` / `parse_generic_enum_typedef_variants`), filling the
  placeholder in place. A single loop registering each header just before its own fields
  is not enough: a mutual cycle needs both headers registered before either list is read.
- R3: `RawTy::OwnedCell`, the `raw_to_poly_type` fold to `Concrete(Type::OwnedCell(..))`,
  a `^`-led `parse_poly_slot` arm (displacing its old `^` exclusion guard), and
  `owned_cell_no_payload_error` shared with the concrete `split_owning_cell_word` so both
  paths word the same defect the same way.

**AST (`src/ast.rs`).**

- `PolyType::OwnedCell(Box<PolyType>)`, mirroring `Ref`: no id minted until the payload
  grounds. The whole-crate exhaustive-match audit landed in the same commit as the variant
  (the crate does not compile in between), with explicit arms rather than new wildcards.
- `MutRegistries<'a>` (`structs`/`enums` immutable, `arrays`/`refs`/`cells` mutable) with
  `names()` and `reborrow()`. It replaces `NameRegistries` on the mutually recursive
  `substitute_generic_field` / `instantiate_struct` / `instantiate_enum` trio. In
  `parse_generic_typedefs` and `resolve_type_or_apply` it is built from disjoint parser
  fields at the call statement, so the borrow checker accepts it with no `mem::take`.
- `substitute_generic_field` is now a `GenericTypes` method with `Array`/`Ref`/`OwnedCell`/
  `Generic` arms; each recurses through `reborrow()`, and the `Generic` arm grounds its
  arguments and then re-enters the instantiator. The fallback `unreachable!` stays truthful:
  the three shapes reaching it (quotation field, `QuotLit`, `Len::Var` array) are each
  unconstructible.
- R2's deferral: `push_struct_placeholder`/`push_enum_placeholder`,
  `struct_pending`/`enum_pending`, `deferred_structs`/`deferred_enums`, drained by
  `fill_struct_fields`/`fill_enum_variants`. An instantiation minted against a still
  pending header (a concrete self-reference such as `L[i64]` inside `L`'s own field list
  goes through `resolve_type_or_apply` synchronously) gets its id and memo entry
  immediately with an empty field list, and its fields are recomputed in place when stage
  (b) fills the header. The `StructId`/`EnumId` already handed out never changes.
- R6: mint the id and push `struct_keys`, `struct_resolved` and `inst_structs` (three
  parallel vectors, enum twin likewise) **before** substituting, then fill in place. The
  declared field list is **cloned**, never `mem::take`n: taking it would strand a
  re-entrant instantiation at a different argument list with an empty list, reintroducing
  the fieldless-struct bug. `instantiate_enum` became an explicit loop over cloned
  variants for the same borrow reason.

**Checker.** `apply_subst` gained the owned-cell arm plus a plain `cells: &mut
Vec<OwnedCellDecl>` parameter, threaded through `back_edge_declared_shape` and its callers.
Its pre-existing `Generic` arm now builds `NameRegistries` over the live cells and refs
instead of `&[]`/`&[]`.

## Rulings as implemented

**R8, growing self-reference, rejected at declaration time.** At every `PolyType::Generic`
node *anywhere* in a field's tree (under an array, a ref, a cell, or another application),
each argument must be fully concrete at any depth or a bare `Var`. A compound argument
mentioning a header variable (`L[^'T]`, `L[['T 2]]`, `L[&'T]`) is rejected at the field's
span by `growing_generic_self_reference_error`, which names the *restriction* rather than
calling the type recursive. Termination (N2) is then a theorem, not a cap: no constructor
is ever applied to an instantiation argument, so the reachable `(header, module, args)` set
is finite and R6's memo visits each member once. Permutation cycles (`type: A 'K 'V next ^A['V 'K] ;`)
and constant cycles still work.

*Accepted over-rejection:* a non-recursive wrapping application (`Ent[['T 2] i64]` where
`Ent` never names back) is refused too. Admitting it needs an SCC pass over a header
dependency graph; nothing currently wanted has a compound generic argument, so the pass was
not built and the diagnostic names the restriction so a later slice can lift it.

**R8 does not inherit D5.** `generic_nesting_depth_error` (nesting depth > 1) is a
word-signature restriction living in `raw_to_poly_type`'s fold; the field parser never
calls that fold, so `L[Ent[i64 str]]` in a field is accepted by design, asymmetric with the
signature path. A test using `L[Ent['T i64]]` as R8's witness would be a placebo: D5 fires
first. R8's witness is `^L[^'T]`, which grows with no `Generic`-in-`Generic` in sight.

**R9 and R10 are reachability, not new rules.** A by-value or array-wrapped generic
self-reference, once instantiated at a concrete type, now reaches the existing
`recursive struct definition (infinite size)` diagnostic; a `&'T` (or composite `&Ent['K i64]`)
field now reaches `a reference cannot be stored` instead of `unknown type 'T`. Neither
fires on a bare generic declaration: `check_recursion` only walks post-instantiation
concrete decls. Consequently `^` is the only field-storable indirection, which is why the
owned-cell variant was a prerequisite and not a bonus.

**N3, no length variable.** A generic header binds only `'`-prefixed variables, so
the array arm sees `Len::Concrete` only; a `Len::Var` path would be dead code.

## Deviations from the plan

- **`MutRegistries` landed in phase 1, not phase 2.** The plan asserted stage (b) could
  reuse `NameRegistries`; it cannot. `fill_enum_variants` rebuilds whole `VariantDecl`s
  (a monomorphized variant's *name* carries the argument spelling, and a placeholder had
  no variants at all), so it interns.
- **Pending headers are `Vec<usize>` index lists**, not the planned `Vec<bool>` flags.
- **No timeout machinery.** The plan wrapped the cell-cycle goldens in a watchdog thread on
  the assumption that an ordering regression hangs. It aborts instead (see Exit findings),
  and a watchdog would have died with the process.
- **R4's `Ref` arm must recurse into the `Generic` arm.** A composite referent
  (`&Ent['K i64]`, in both spellings phase 1 admits) panicked at `substitute_generic_field`
  exactly as the bare `&'T` did; a `&'T`-only fixture cannot witness it, so R10's coverage
  was widened.

## Tests

Parser unit tests beside the production cover each of the five field shapes and their
`PolyType` trees, the nested array, the enum variant twin, the mixed concrete/variable
argument list, an unbound `'name` inside an array (error names the declaration, not
`PolyBuilder`), the not-phantom claim for a variable used only inside a wrapper, the
concrete self-reference `L[i64]`, a still-rejected duplicate generic header, R8's reject
and its concrete-nested accept case, R7's reject and the concrete quotation accept case,
the glued and stacked sigil spellings, and R7's bracket-depth scan.

`tests/phase7_slice3n.rs` carries the build-level goldens: the four instantiating field
shapes, `^'T` in a word signature (build and run), the stored-reference rejection for both
a bare and a composite referent, the two infinite-size rejections, the cell-wrapped
self-reference (struct and enum), the mutual cycle, the permuting cycle, a generic with a
cell argument constructed in a poly body, and the pinned attributeless-variant parse gap.
`src/driver.rs` holds `map_shaped_backing_storage_instantiates_at_two_key_value_pairs`
(asymmetric `(i64, str)` / `(str, i64)`, asserting field types and two distinct `StructId`s)
and `map_shaped_field_instantiation_renders_through_live_cells_and_refs`.

R6's ordering, R7's and R8's guards, and the distinct-`StructId` assertion were all
mutation-tested.

## Out of scope (pinned, not silent)

A struct-header length variable; a quotation-typed generic field (R7's located rejection);
the pre-existing attributeless/positional variant array-field parse gap
(`type: Foo | Some [i64 2] | None ;`, pinned at its current `expected a word, found LBracket`).
Untouched: `check_recursion`'s logic, the no-stored-reference rule, `check_no_phantom_ty_var`.

## Exit findings

- **The termination failure mode is an abort, not a hang.** Measured by reverting R6's
  ordering: the recursion is a call chain, so it overflows the compiler's stack
  (`fatal runtime error: stack overflow`). Residual cost is classification only: an aborted
  binary emits no terminal `test result:` line, so a `test result: FAILED` classifier scores
  such a mutation SURVIVED. Both R6 mutations were classified on the abort string, and both
  are caught.
- **N1 probed, not assumed.** No field shape phase 1 admits reaches
  `substitute_generic_field`'s `unreachable!`; the one candidate (a quotation naming a type
  variable) is rejected by R7 under array, cell, ref, nested array, generic argument and
  enum variant wrappers.
- **R8 is uniform across headers and kinds.** Cross-header growth (`A['T]` whose field names
  `^B[^'T]`) and the enum equivalent are both rejected; struct-to-enum and enum-to-struct
  cell cycles terminate in either declaration order. Cross-kind cycles were probed directly,
  not covered by a test.
- **`MutRegistries::names` over the live registries is load-bearing.** `type_arg_key`
  renders a `Struct`/`Enum` argument from the carried `name` but *indexes* `refs`/`cells`/
  `arrays` by id, so the old `cells: &[]` / `refs: &[]` throwaway renders a cell- or
  ref-payload argument wrong, or panics, as soon as one exists to look up.
- **`src/ast.rs` split signals (CLAUDE.md phase-exit re-run): 3 of 5 fire.** Not firing:
  import divergence (the file has zero top-level `use` statements) and no would-be circular
  dependency. Firing: three responsibilities, high- and low-level code mixed, functions that
  never call each other. **Deferred, but the cut is clean**, unlike `poly.rs`'s: the generics
  unit (`GenericStructDecl`/`GenericEnumDecl`/`GenericVariantDecl`/`GenericTypes`/`PolyType`/
  `NameRegistries`/`MutRegistries`/`type_arg_key`/`type_instantiation_name`/
  `generic_surface_name`) depends only downward and moves as one. **Recommended as its own
  change ahead of P7.S3e**, which grows `GenericTypes` again.
- **`growing_generic_self_reference_error` renders a doubled `error:` prefix**, joining a
  pre-existing unowned set across the crate. A sweep, not a drive-by fix.
- The roadmap's S3n entry is closed out in `docs/roadmap/P7-language-prereqs.md`; its
  "rejects with `unknown type 'T`" claim and its "not yet recon'd" paragraph were both
  falsified by this slice.

## Exit criteria (met)

1. Each wrapped-variable field shape parses to its stated `PolyType` tree, including the
   glued and stacked sigil spellings and the enum variant twin.
2. `type: L 'T next L[i64] ;` resolves its self-reference, and a duplicate generic header is
   still rejected.
3. `: idc ( ^'T -- ^'T ) ;` builds and runs.
4. A growing generic argument is a located parse-time rejection naming the restriction; a
   fully concrete nested argument is accepted.
5. A variable-quotation field is a located rejection; a concrete quotation field still builds.
6. The `Map`-shaped stand-in builds at two asymmetric instantiations with distinct `StructId`s.
7. `&'T` and `&Ent['K i64]` fields error with `a reference cannot be stored`, not `unknown type`.
8. By-value and array-wrapped generic self-references, instantiated concretely, error with
   `recursive struct definition (infinite size)`; the `^`-wrapped ones build and terminate,
   single-header, mutual and permuting.
9. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green at each phase exit.No response requested. (I returned the condensed document as text and did not write to `slice3n-spec.md`; if you want it applied to the file, say so and I'll re-read the autofixed copy first.)
