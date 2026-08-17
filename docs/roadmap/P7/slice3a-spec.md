# Phase 7 Slice 3a: generic instantiation over a poly word's own type variable (implemented)

## Goal

A polymorphic word can name a generic type applied to *its own* type variables
(`Result['T 'E]`, `Box['T]`, `Option['T]`) in its signature, and its body can
*construct* such a value. Delivered by a deferred `PolyType::Generic`
application in the type language plus keeping the `GenericTypes` instantiator
alive and mutable through check and lowering, so a monomorph is minted on demand
at the point a substitution grounds it.

Trait bounds are P7.S3b and appear nowhere here.

## Delivered shape

```sooth
type: Result 'T 'E | Ok 'T | Err 'E ;

: reorder ( 'T Result['T 'E] -- Result['T 'E] 'T ) swap ;
: wrap    ( 'T -- Result['T i64] ) Ok ;
```

Before this slice the signature of `reorder` was rejected at parse time
(`error: unknown type 'T`), and `wrap`'s body failed with ``unknown word `Ok` ``
because the constructor's target mentions `'T`. Both now work, at two asymmetric
instantiations (`[i64 str]` and its swap `[str i64]`), each minting its own
monomorph. The already-working concrete case
(`: wrap ( 'T -- Result[i64 i64] ) drop 1 Ok ;`) is byte-for-byte unchanged:
an all-concrete argument list folds to `PolyType::Concrete` at the parse fold.

## Design decisions

- **D1 A new `PolyType::Generic` variant.** Nothing existing carries
  (header identity, argument list): `Concrete` demands a real `StructId`/`EnumId`,
  which is exactly what does not exist yet; `Array`/`Ref` are shape-specific.
  The precedent is `PolyType::Ref`, which carries no `RefId` for the same reason.
- **D2 Shape:** `Generic { is_enum: bool, idx: u32, module: u32, args: Vec<PolyType>, name: &'static str }`
  (`src/ast.rs:1195`). `idx` indexes `GenericTypes::structs`/`enums` per
  `is_enum`; `module` is the *instantiating* module, the third dedup-key
  component, captured at the naming site. `name` is the header's declared
  spelling, cached for diagnostics only (mirroring `StructDecl::name_static`)
  and carrying no identity: header sameness is `is_enum`/`idx`/`module` alone.
- **D3 One instantiator, one key table, one id space.** Grounding routes through
  `GenericTypes`' own `struct_keys`/`enum_keys`, never an independent downstream
  interner, which would mint a second `Result[i64 str]` under a different
  `StructId` and make the checker's `Type` equality answer "different".
- **D4 Keep `GenericTypes` alive rather than pre-materializing.** Whole-program
  pre-materialization would need every poly word crossed with every call site's
  substitution before checking has produced those substitutions. The
  arrays/refs registries are the working precedent.
- **D5 Depth 1 only.** `Box[Box['T]]` is representable but rejected at the parse
  fold with a located error; no consumer forces it.
- **D6 A variable-bearing generic is conservatively linear (never `Copy`).**
  `Copy`-ness of `Result['T 'E]` depends on the args' bounds, and a per-argument
  derivation is a new rule with its own drop-obligation consequences. Rejecting
  `dup` is consistent with the linear spine and relaxable later without
  invalidating any accepted program.
- **D7 Construction is one narrow arm.** Quotation literals
  (`src/check/poly.rs:466`) and array constructors (`:487`) stay rejected in poly
  bodies; only a generic variant/struct constructor gains an arm.

## Requirements

### R1 `PolyType::Generic` plus a deliberate arm at every match site

Parse route: `RawTy::Generic` (`src/parser.rs:893`), produced by a
`parse_poly_slot` arm ahead of the `parse_type_expr` fallthrough
(`src/parser.rs:2064`), folded in `raw_to_poly_type` (`src/parser.rs:2366`).
The fold mirrors the array fold: all-`Concrete` arguments call
`instantiate_struct`/`instantiate_enum` and yield `Concrete`, otherwise the
`Generic` stays symbolic. The arm reuses the existing header lookup and privacy
gate (`bare_generic_owner`, `generic_is_declared`, `type_is_exported`) and only
the *arity check* of `parse_type_arguments`, not its concrete-only argument
parser; arguments parse as poly slots. Arity mismatch keeps
`generic_arity_error`. Depth > 1 is rejected at the fold, naming outer and inner
headers.

No site takes a `_ =>` catch-all:

| Site | Arm |
| --- | --- |
| `poly_is_copy` (`check/poly.rs:43`) | `false` (D6) |
| `is_reference_slot` (`:130`) | `false` |
| `poly_copy_gate` (`:1411`) | `poly_copy_generic_error`, rendered via `poly_type_str` |
| `unify_poly_input` (`:2205`) | positional recursion over `args` after matching `is_enum`/`idx`/`module` and arity; a concrete instantiation of the same header matches through the dedup key |
| `apply_subst` (`:2038`) | substitute args, then mint-or-find through the live instantiator, returning a concrete `Type` |
| `poly_op_on_variable_error` (`:2333`) | `` "a generic type `…`" `` |
| `receiver_is_aggregate_projection` (`:2406`) | `true`, matching the concrete struct/enum answer |
| `poly_type_str` (`:2728`) | `Name['A 'B]` in the signature's own spellings, from the cached `name` |
| `contains_poly_reference` (`check/audits.rs:365`) | recurse into `args` |
| `audit_poly_input_quotation` (`:407`) | recurse into `args` |
| `reject_poly_quotation_anywhere` (`:438`) | recurse into `args` |
| `collect_poly_concrete` (`check/declarations.rs:367`) | recurse into `args` only |
| `subst_polytype` (`ir/driver.rs:677`) | lookup-only through `lookup_struct`/`lookup_enum` |
| `remap_poly_type` (`repl.rs:289`) | remap args, header identity passes through |

**Open gap (export privacy).** `collect_poly_concrete` carries only concrete
`Type`s, so it cannot carry an ungrounded generic *header* named in an exported
poly word's signature, and the parse-time `type_is_exported` gate fires only for
*qualified* names. An exported poly word can therefore name a bare,
module-private generic header with no privacy check. Closing it needs a
dedicated header-privacy channel (`Vec<(usize, u32)>`) in
`check_exported_signatures`. Recorded, not closed.

### R2 `GenericTypes` lives through check and lowering

`src/driver.rs:316-318` flushes the staged parse-time instantiations onto the
live registries and then **rebases** the instantiator to their new length
(`flush_structs_into` / `flush_enums_into` / `rebase`, `src/ast.rs:570-585`),
rather than consuming and dropping it. The instantiator then rides on `Module`
(`src/driver.rs:335`) and reaches the grounding arms through `Ctx::Word`'s
`Option<&RefCell<GenericTypes>>` (`src/check/engine.rs:1133`, accessor at
`:1316`).

- The id invariant: a mint's returned id index equals the decl's position in the
  final merged registry. The trap the rebase closes is a flush with no rebase, so
  a later downstream mint counts from a stale base and lands on an id a
  parse-time instance already occupies (two `Type::Struct`s, one id, different
  field layouts). Already-minted ids are read back from
  `struct_resolved`/`enum_resolved`, never recomputed from the advanced base.
- Check mints (`apply_subst`), lowering only looks up (`subst_polytype`), the
  same division the array/ref arms draw.
- `Ctx::Line` and the REPL yield `None`: those paths can never carry a
  `PolyType::Generic` (the REPL declares no generic `type:`), and a `None` there
  produces `poly_generic_not_yet_groundable_error` rather than minting through an
  absent table.
- A program with no variable-bearing generic mints exactly the same monomorph
  set as before.

### R3 generic construction in a poly body

`poly_call_term` calls `poly_construct_generic` (`src/check/poly.rs:978`) ahead
of the ordinary `env` dispatch, deliberately before it: a single registered
concrete candidate under the bare name would otherwise commit and error on a
`'T` operand.

- The header comes from the variant/struct base name, searched over every
  generic header the module declares (`poly_construction_header`), own module
  preferred over an imported same-named one. Arguments come from unifying the
  operand slots against the header's declared payload `PolyType`s; undetermined
  arguments fall back to the enclosing word's declared output slot at that stack
  position when it is a `Generic` over the same header
  (`poly_construction_fallback`, `:884`).
- Still-undetermined argument: located error naming constructor and variable.
  Operand/payload mismatch: reported at the call site through
  `poly_rendered_type_mismatch_error`, never deferred into synthesis.
- The pushed slot is `Generic`, folding to `Concrete` when every argument is
  concrete, which is today's behaviour for the concrete case.
- Soundness: an undetermined argument is *phantom* for the constructed variant
  (`substitute_generic_field`, `src/ast.rs:505-511`, substitutes only fields that
  exist), so adopting a concrete `E` from the output slot cannot create a
  runtime/static mismatch. The backstop for *determined* arguments is
  `unify_poly_input`'s `Generic` arm at word exit, which is why the off-tail test
  exists rather than being assumed.

### R4 one independent monomorph per instantiation

Two substitutions over the same poly word yield two distinct monomorph symbols
with positionally-correct types, proven by `nm` over the asymmetric pair
`[i64 str]` / `[str i64]` (`Result[i64 i64]` cannot tell `Ok 'T | Err 'E` from
its swap).

### R5 soundness rejections

Located errors, each asserted on message text: depth > 1 nesting; an
undetermined constructor argument; a constructor operand/payload mismatch;
`dup`/`over` on a variable-bearing generic slot; arity mismatch (reusing
`generic_arity_error`).

## Implementation

Two commits.

**Phase 1 (`41b488b`)** the variant, the parse route, and the non-grounding
arms: `src/ast.rs`, `src/parser.rs`, `src/check/poly.rs`, `src/check/audits.rs`,
`src/check/declarations.rs`, `src/ir/driver.rs`, `src/repl.rs`. The three
grounding arms exist for exhaustiveness but return
`poly_generic_not_yet_groundable_error` (`src/check/poly.rs:2098`): there is no
check-side registry for a named generic before R2, so no grounding and no
build+run golden here.

**Phase 2 (`eb474d8`)** registry lifetime, grounding, and construction:
`src/driver.rs`, `src/ast.rs`, `src/check.rs`, `src/check/captures.rs`,
`src/check/declarations.rs`, `src/check/engine.rs`, `src/check/poly.rs`,
`src/check/word_entry.rs`. The grounding arms and R2 are one unit; none of
`apply_subst`, `unify_poly_input`, `subst_polytype` can resolve a generic
without the live table.

### As-built deviations from the plan

- The instantiator reaches check through `Ctx`'s
  `Option<&'a RefCell<GenericTypes>>`, not a `&mut GenericTypes` threaded beside
  the array/ref registries. `Ctx` otherwise borrows only immutably, and
  grounding mints mid-body-walk. `apply_subst`/`unify_poly_input` signatures are
  unchanged; the plumbing lands on `word_entry.rs`, `engine.rs`, `captures.rs`.
- `GenericTypes` is flushed-then-rebased rather than made the sole writer of
  `structs`/`enums` with the one-shot `extend` deleted. Same invariant, smaller
  blast radius.
- `PolyType::Generic` carries a `name` for diagnostics, which the plan's shape
  omitted; `poly_type_str` needs no registry as a result.
- Lowering's `subst_polytype` takes `&GenericTypes` (lookup-only, `expect` on a
  miss), matching the array/ref arms, rather than sharing the mint.

## Testing

`tests/phase7_slice3a.rs`:

- `poly_word_consuming_result_over_its_own_vars_runs_at_two_asymmetric_instantiations` (T1)
- `two_asymmetric_instantiations_mint_distinct_symbols_nm` (T2)
- `poly_word_constructs_a_monomorph_no_other_site_materializes` (T3: the mint
  can only come from downstream)
- `poly_body_constructor_off_tail_position_unifies_at_exit` (T-nontail)
- `generic_nested_depth_two_is_error`,
  `generic_constructor_undetermined_argument_is_error`,
  `generic_constructor_operand_mismatch_is_error`,
  `dup_on_variable_bearing_generic_slot_is_error` (R5)

Unit tests beside their stage: `src/parser.rs`
(`parse_poly_generic_over_own_type_variable_ok`,
`..._all_concrete_args_folds_to_concrete`, `..._nested_depth_two_is_error`,
`..._arity_mismatch_is_error`, `..._private_header_is_not_exported_error`);
`src/check/poly.rs` (`poly_generic_slot_is_not_copy`,
`poly_type_str_renders_a_generic_application`,
`poly_generic_receiver_is_aggregate_projection`,
`unify_poly_generic_binds_arguments_positionally`,
`poly_body_constructor_resolves_arguments_from_the_declared_output`,
`..._undetermined_argument_is_error`, `..._operand_mismatch_is_error`);
`src/check/audits.rs` (`quotation_smuggled_as_generic_arg_is_rejected`,
`ref_bearing_generic_in_copy_position_is_rejected`); `src/ast.rs`
(`interleaved_downstream_mint_id_differs_from_parsetime_instance`, an
*interleaved* mint/flush/mint, since a single mint in isolation passes under the
colliding implementation too).

Mutation-tested guards (each must fail when the arm it guards is deleted): the
four R5 rejections, the interleaved id-invariant test, the two audit-arm
rejections, and `unify_poly_generic_binds_arguments_positionally` (mutation:
collapse positional order, which T1/T2 must then catch).

Regression, green and untouched: `tests/phase5_slice2.rs` (every concrete
generic application, the fold-to-`Concrete` guarantee),
`tests/phase5_generic_enum_elimination.rs` (clause elimination, the
`Result[i64 i64]` / `Result[bool bool]` non-collision test pinning the dedup
keys R2 keeps alive longer), `tests/qbe_baseline.rs`, the Slice 13
poly-reference suite (`parse_poly_slot` is edited directly above the `&`-led
arms), `tests/phase7_slice1.rs`, `tests/phase7_slice2.rs`.

## Out of scope / known gaps

- Trait bounds (P7.S3b entirely).
- Nesting depth beyond 1: representable, rejected, unblocked later if a consumer
  forces it.
- Any change to how a *concrete* generic argument resolves.
- Relaxing D6 into a per-argument `Copy` derivation.
- Quotation literals and array constructors in poly bodies: still rejected.
- **Open:** the export-privacy gap under R1, and the single-file
  `parser::parse` path, which drops its own `GenericTypes`, so D3's "one id
  space" is not globally true there.
