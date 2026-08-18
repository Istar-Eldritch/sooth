## Design as shipped

- **R1 — parse-time recognition: name-only, module-scoped, generic-aware, type-precedent.**
  `parse_leading_variant_slot` returns `Option<VariantTag>` and resolves no `Type`. The
  sigil-stripped leading token of an annotation is a routing tag iff (1) it does *not*
  resolve as an ordinary type name in scope (`resolve_type_name_in_module`: a struct or enum
  of the same name wins, in both the lone `( Circle )` and escalated `( Circle -- i64 )`
  spellings), **and** (2) `variant_name_is_visible` finds it as a variant of a concrete or
  generic enum in this module, or in the target module of a selective import of that name.
  Module scoping is deliberate and not `is_variant_name`, which matches any enum in any
  module and would let an unimported module's variant capture this module's leading slot.
  `module_declares_variant` matches `name_static` on the concrete registry (REPL import-epoch
  tags) and `name` on `generics.enums`. `parse_quot_annotation` no longer synthesizes a
  leading input: `annot.inputs` for a tagged arm holds only the ordinary post-`--` slots.
- **R2 — the mode is explicit data, not an interned `Type::Ref`.**
  `QuotAnnot`/`AnnotEffect` carry `variant_tag: Option<VariantTag>`, where
  `VariantTag { name: String, mode: VariantTagMode }` and
  `VariantTagMode ∈ { Owning, Ref, RefMut }`. `name` is always the bare surface spelling.
- **R3 — check-time synthesis of the leading input.** `check_eliminator_call` builds each
  arm's slot from `(variant_type(ctx.enums(), id, vi), tag.mode)` through the same
  `intern_ref_type` as the call's `narrowed`, and `insert(0, …)`s it into
  `prov.quotations[qid].annot.inputs` (insert, not overwrite: the escalated spelling's
  declared slots follow it). `reconcile_annotation_with_parameter` then compares the
  user-written mode against the call's resolved mode with no new branch: identical interned
  types when they agree, the existing `annotation_parameter_mismatch_error` when they don't.
- **R4 — one tag spelling on both sides.** Recognition records the bare name; routing keeps
  comparing `generic_surface_name(&v.name) == tag.name`, which strips an instantiation's
  `[...]` (stored `Ok[i64 bool]` → `Ok`). Resolve never `__mN`-mangles variant names.
- **R5 — operand-driven `EnumId`, registry as a base-family gate.** `eliminator_registry`
  stays keyed by `generic_surface_name(&decl.name)` (the call site resolves to the bare
  mangled base `Enum__mN?`, so re-keying by instantiation would make the call unrecognizable).
  The operative id comes from the scrutinee's own `Type::Enum(id, _)` via `ref_parts`; the
  registry entry is only a gate, checked as
  `generic_surface_name(enums[scrutinee_id].name) == generic_surface_name(enums[gate_id].name)`.
  Two asymmetric instantiations therefore eliminate independently in one word, with no
  last-write-wins dependence. A non-enum or wrong-family scrutinee is `type_mismatch_error`,
  whose "expected" side renders the family surface name (`Result`, not an arbitrary retained
  instantiation).
- **Diagnostic rendering.** Eliminator errors name the surface enum via
  `demangle_word(generic_surface_name(&enum_decl.name))`: the strip handles instantiations
  (`Result[i64 bool]__m0`, mangle after the arguments), the demangle handles the concrete
  case (`Shape__m0`). Variants render through `generic_surface_name`.
- **R6/R7 — the IR reads the mode from `VariantTag`.** `arm_tag` no longer reads
  `annot.inputs` (its `unreachable!` is gone); `quot_arm_tags` holds `Option<VariantTag>`.
  `lower_eliminator` maps `Owning -> None`, `Ref -> Some(false)`, `RefMut -> Some(true)` and
  passes `(EnumId, Option<bool>)` to `lower_clauses`, whose parameter is now that pair rather
  than a `Type` it destructured itself; the clause-path caller does the destructuring in a
  shared helper. No `Type::Ref` is manufactured and `refs` is not consulted on this path.

## Guards

- `check_eliminator_call_mode_mismatch_is_error` (`src/check.rs`) pins the mode-mismatch
  wording byte-for-byte and fails if R3's synthesis is removed (`tails_agree` is vacuously
  true against an empty `annot.inputs`).
- Parser: `parse_leading_variant_slot_struct_of_same_name_takes_precedence`
  (+`_with_outputs`, `_may_not_elide_the_arrow`) for type precedence;
  `parse_leading_variant_slot_other_module_variant_is_not_visible` and
  `..._other_module_generic_variant_is_not_visible` for module scoping;
  `parse_quotation_annotation_variant_tag_owning_ok`/`_mut_ref_ok` for the `VariantTag`
  shape and the empty `inputs`.
- `tests/phase6_slice3b.rs`: `generic_enum_eliminator_runs_both_arms`;
  `generic_enum_eliminator_by_reference_reads_and_mutates_in_place` (arms in reverse
  declaration order, so it also witnesses tag-based routing);
  `stray_generic_arm_tag_outside_an_eliminator_call_is_error`;
  `non_exhaustive_generic_eliminator_names_the_surface_variant`;
  `wrong_family_scrutinee_names_the_generic_surface_family`;
  `two_asymmetric_instantiations_eliminate_independently_in_one_word` (`Result[i64 bool]` vs
  `Result[bool i64]`; also covers the differing stored variant name).
- `lower_eliminator_call_over_a_reference_to_a_scalar_enum_loads_the_tag`
  (`src/ir/func_builder/control_flow.rs`) is the only guard on R7's mode mapping:
  `ref_mutable` reaches codegen only through `scrutinee_is_value`, which needs `is_scalar`.
  No generic analogue exists — the phantom-parameter rule rejects an all-unit generic enum.
  For a payload-carrying enum the value is a codegen no-op, so a mutation check against the
  runnable goldens would be a placebo.
- Unchanged regressions: all of `tests/phase5_generic_enum_elimination.rs`, the
  `tests/phase6_slice3.rs` concrete suite, `examples/eliminator_ref.sth`.

## Known gaps (not fixed here)

- **Scalar enum by reference dies in the backend.** `type: Dir | North | South ;` with
  `: label ( &Dir -- i64 ) ~[ ( &North ) drop 1 ] ~[ ( &South ) drop 2 ] Dir? ;` fails as
  `qbe: invalid type for first operand %v0 in add`. Pre-existing (reproduces on `main`
  before this slice); owning-mode scalar elimination runs. Wants its own slice.
- **R5's family gate is not module-scoped for generics.** `generic_surface_name` splits at
  the first `[`, taking an instantiation's module tag with it, so two modules each declaring
  a generic `Result` share one registry key and pass one another's gate. Unreachable today:
  a generic instantiation cannot cross a module boundary (`export:` will not parse
  `Result[i64 bool]`, and exporting the base fails the private-type check on an exported
  word's effect). Whichever slice enables that export must scope the gate and the registry
  key by module; no stopgap here.

## Out of scope

`WordBody::Clauses`/`parse_clauses` deletion and the phase-5 test migration (Slice 4); any
change to `Type::Variant`, accessors, `ArmBinding`, clause-path behaviour, or generic-enum
construction; inferring an instantiation from an arm rather than the scrutinee. The REPL adds
no declaration form here — the stray-tag guard and the mode mismatch are widened to generic
tags and both are covered through the batch compiler.
