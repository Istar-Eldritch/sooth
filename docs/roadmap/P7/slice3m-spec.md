# Spec: P7.S3m a declared quotation effect with two or more outputs interns an output bundle

**Status:** Ready to implement
**Discovery:** `docs/roadmap/P7/slice3m-brief.md` (probe-validated, no open design questions)

## Problem

A multi-output return bundle is minted in exactly one place: `intern_output_bundles`
(`src/check.rs:1005`, called from `:972`). It walks `module.words` alone and interns a
bundle struct for each *declared word* whose own top-level output list has length >= 2:

```rust
fn intern_output_bundles(module: &mut Module) {
    let tuples: Vec<Vec<Type>> = module
        .words
        .iter()
        .filter(|w| w.effect.outputs.len() >= 2)
        .map(|w| w.effect.outputs.iter().map(|s| s.ty).collect())
        .collect();
    for outputs in tuples {
        intern_bundle_struct(&mut module.structs, &outputs);
    }
}
```

A *quotation* value's own effect (`[ i64 -- i64 i64 ]`) is never inspected here, even when
it appears as a word's parameter or output type. At lowering, `lower_indirect_call`
(`src/ir/func_builder/quotation.rs:203`) asks `bundle_of(&outs, self.structs)`
(`src/ir/func_builder/mod.rs:32`) for the tuple; `bundle_of` returns `None` when nothing
was interned, so `ret` is set to `None`, the `CallIndirect` produces no value, and the
`unpack_bundle` that would push the outputs is skipped. The quotation's second (and later)
output is therefore never pushed. The first consumer that reads it underflows the stack.

**Confirmed live against `main`:**
`: call_it ( [ i64 -- i64 i64 ] -- ) 3 swap call add . ;` panics at
`src/ir/func_builder/calls.rs:451` (`rhs = self.stack.pop().expect("bin: rhs")` in the
`add` handler) -- identically whether `call_it` is concrete or, per S3f's R3, polymorphic.

## Mechanism (probe-validated, not an open question)

`intern_bundle_struct`, `bundle_of`, and `word_ret_ty` are already shape-agnostic: each
takes `&[Type]`/`&[TypedSlot]` and mints or looks up a bundle without caring whether the
tuple came from a word's outputs or a quotation's. The entire gap is in *discovery* --
nothing hands them a quotation's output tuple. The fix is a strict widening of
`intern_output_bundles`'s walk; no downstream consumer changes, and the interned bundle is
keyed by its exact type list, so a quotation output tuple that coincides with a word's is a
no-op re-intern.

**R1 -- widen the discovery walk to four additional sites.** `intern_output_bundles`
recursively collects every `Type::Quotation`'s output tuple of length >= 2 reachable from:

1. **Word inputs and outputs.** `module.words[].effect.inputs` and `.outputs`
   (`StackEffect`, `ast.rs:2146`; each `TypedSlot.ty` is a `Type`). The existing walk already
   reads `.outputs` for the word's own bundle; it now also descends *into* each slot's
   `Type` looking for a nested `Type::Quotation`, and adds `.inputs`.
2. **Struct fields.** `module.structs[].fields` (`StructDecl.fields: Vec<(String, Type)>`,
   `ast.rs:411`). A quotation is legal as a struct field (a materialization boundary),
   unlike an *enum variant field*, which the checker rejects outright
   (`audit_quotation_type_registries`, `src/check/audits.rs`) -- so enum variant fields are
   not a discovery site.
3. **Array elements.** A quotation is legal as an array element (the same materialization
   boundary), but `Type::Array(ArrayId, _)` (`ast.rs:2180`) carries only an `ArrayId`, not
   the element type inline, so this site is reached by resolving `module.arrays[id].element`
   (`ArrayDecl.element: Type`, `ast.rs:1242`) -- a registry lookup -- and recursing into the
   resolved `Type` (see R2).
4. **Poly signatures.** For each `w.poly` (`WordDef.poly: Option<Box<PolySig>>`,
   `ast.rs:1592`), walk `w.poly.inputs` and `.outputs`
   (`PolySig.inputs`/`.outputs: Vec<PolyType>`, `ast.rs:1930`). This site is separate
   because a polymorphic word's declared shape lives in `w.poly`, not `w.effect` --
   `w.effect` is empty for a poly word, so a walk that read only `w.effect` would miss every
   poly word's quotation-typed parameters and outputs, which is the entire polymorphic half
   of the confirmed repro.

**R2 -- `Type` recursion.** A `Type::Quotation(eff)` (`ast.rs:2248`) carries
`QuotEffect { inputs: Vec<Type>, outputs: Vec<Type> }` (`ast.rs:2269`). When such a type is
reached and `eff.outputs.len() >= 2`, intern `intern_bundle_struct(&mut module.structs,
&eff.outputs)`. The recursion also descends through `eff.inputs`/`.outputs` (a quotation
whose own parameter is itself a multi-output quotation).

The one composite `Type` constructor that can legally wrap a `Type::Quotation` yet is not a
top-level walk site of its own is the array element. `Type::Array(ArrayId, _)` (`ast.rs:2180`)
stores no element type inline -- only an `ArrayId` -- so reaching a quotation nested in an
array element is a distinct step, not inline recursion: resolve `module.arrays[id].element`
(`ArrayDecl.element: Type`, `ast.rs:1242`), a registry lookup, then recurse into the resolved
`Type`. R2 does *not* recurse into a reference referent or an owned-cell payload: the checker
rejects a `Type::Quotation` in either position outright (`audit_quotation_type_registries`,
`src/check/audits.rs`), so those are dead code, not materialization sites. Struct fields are
covered by the top-level walk over `module.structs`; enum variant fields cannot carry a
quotation (same audit), so neither is recursed into here.

**R3 -- site 4 needs only the top-level `PolyType::Concrete` arm, not a recursive walk.** A
ground quotation can only ever reach a poly signature as a top-level
`PolyType::Concrete(Type::Quotation(..))`: `audit_poly_input_quotation`
(`src/check/audits.rs:366`) already rejects a quotation nested in every other
`PolyType` position -- `Array`, `Ref`, `OwnedCell`, a quotation-effect row, and a generic
type argument -- via its `reject_poly_quotation_anywhere` twin, and that audit
(`audit_quotation_type_registries`/`audit_poly_input_quotation`, both run from
`src/check.rs:570`) runs well before the discovery walk (`:972`). So a recursive descent
through those wrappers, as an earlier draft of this mechanism called for, is dead code: it
can never fire, because nothing that would make it fire survives to reach it. Site 4 is
therefore just `for pt in sig.inputs.iter().chain(&sig.outputs) { if let
PolyType::Concrete(ty) = pt { collect_quotation_bundles(*ty, &mut tuples); } }` -- the same
`Type` recursion (R2) applied to the one `PolyType` shape that can carry a ground quotation.
A fully concrete composite (`[ [ i64 -- i64 i64 ] 2 ]`) also folds to `Concrete` at parse
time, so it arrives here rather than as a `PolyType::Array`.

Ordering is unchanged: the widened `intern_output_bundles` still runs where the current call
sits (`src/check.rs:972`), after every type-level check and `struct_generated_sigs`, and
before the per-instantiation `inst.out_arity >= 2` bundle loop (`:978`) and the S3k
transitive-instantiation fixpoint (`:993`). A quotation parameter's effect is a fixed
concrete shape read straight off a declared signature, so it is discoverable statically and
needs no interaction with either later, instantiation-driven bundle pass.

## Why no second discovery pass is needed (do not reopen)

Two independent, already-present checker invariants make a signature walk complete for every
quotation effect that can reach the panic site. Both were probed directly, not assumed.

- **No orphan (inferred-only) quotation effect reaches the runtime path.** A quotation
  literal called in the same branch it is built in is *spliced* at compile time and never
  reaches `lower_indirect_call`'s runtime path at all. Any quotation that *does* reach that
  path -- because it was merged at a branch join, or stored into an array/ref/struct field,
  and so must be *materialized* -- is already rejected unless it has a declared home:
  `different_quotations_at_join_error` (`src/check/terms.rs:1637`, raised at `:1397`)
  requires the quotation be given "a declared type (a word output or field) so it can be
  materialized" at the join, and the array/struct storage boundary is gated the same way.
  So every quotation effect capable of reaching the panic is, by construction, already
  visible to a walk over word signatures, struct fields, and array elements -- there is
  no inferred-only shape for a second pass to find.

- **No per-instantiation poly shape escapes the walk.** A poly quotation parameter whose own
  output row still mentions a type variable (`[ 'T -- 'T 'T ]`) is statically rejected for
  `call` inside a poly body (`poly_op_on_variable_error`, `src/check/poly.rs:1372`). `call`
  admits a quotation operand only when it is a fully-ground
  `PolyType::Concrete(Type::Quotation(..))` (`src/check/poly.rs:1367`). By the time a `call`
  on a quotation operand is legal, its effect is already concrete, and R3's walk over
  `w.poly` finds exactly that ground case in the declared signature. There is no later,
  only-visible-post-instantiation quotation shape that could slip past the check-time walk,
  so the fixpoint at `check.rs:993` needs no widening.

## Out of scope

- The per-instantiation output bundle loop (`check.rs:978`) and the S3k transitive
  instantiation fixpoint (`check.rs:993`, `intern_composed_bundles`): a quotation
  parameter's effect is concrete at declaration and interned by the widened walk before
  either runs.
- Materialized runtime quotation values (branch-join merge, array/ref/struct storage): their
  materialization boundary is a separate, already-shipped mechanism; this slice only interns
  the bundle their declared effect needs, and relies on that boundary (above) for
  completeness.
- Any change to `bundle_of`, `word_ret_ty`, `intern_bundle_struct`, `lower_indirect_call`,
  or any lowering path: all are shape-agnostic and already correct once the bundle exists.

## Invariants

- The widened walk is purely additive: an interned bundle is keyed by its exact `&[Type]`
  list, so a quotation output tuple that coincides with an existing word/instantiation bundle
  re-interns to the same `StructId`, and IL for every program that did not use a multi-output
  quotation is byte-identical.
- Discovery stays at `check.rs:972`, ahead of the per-instantiation and transitive bundle
  passes; a quotation parameter's ground effect is found without any θ.
- A single-output (`len == 1`) or zero-output quotation effect is untouched: the `>= 2`
  filter is preserved at every site, so `lower_indirect_call`'s single-output path
  (`ret = Some(..)`, no bundle) is unchanged.
- No new diagnostic and no rejection: the two guards above already reject every quotation
  effect this walk cannot reach, so widening discovery never has to refuse anything.

## Tests

Per CLAUDE.md (a unit test beside the stage function; diagnostics/behaviour, not just
pass/fail; `thing_condition_expected` naming):

- **Unit, beside `intern_output_bundles` in `src/check.rs`**
  (`#[cfg(test)] mod tests`): the widened walk finds each legal site. `bundle_for` lives on
  the IR-side `Structs` registry (`src/ir/layout.rs:370`, `pub(super)`) and is unreachable
  from a `src/check.rs` test, so every assertion instead scans `module.structs` (a
  `&[StructDecl]`) for an `is_bundle` struct whose `fields` match the expected tuple -- the
  shape `intern_bundle_struct_same_tuple_dedups_expected` (`src/ast.rs:3375`) already uses
  (`s.is_bundle && s.fields == vec![("f0".to_string(), Type::I64), ("f1".to_string(), Type::I64)]`).
  - `quotation_param_two_outputs_interns_a_bundle` -- a concrete word taking
    `[ i64 -- i64 i64 ]`: after `check`, `module.structs` contains an `is_bundle` struct
    with fields `[("f0", i64), ("f1", i64)]`.
  - `poly_signature_quotation_param_interns_a_bundle` -- a poly word whose `w.poly` carries
    a ground `PolyType::Concrete(Type::Quotation([i64 -- i64 i64]))` param: the same
    `is_bundle` struct is present, proving site 4 (`w.poly`, not `w.effect`) is walked. (Not
    a quotation *output*: a poly signature's output can never be a quotation --
    `reject_poly_quotation_anywhere` rejects one outright -- so only the param shape exists.)
  - `struct_field_quotation_two_outputs_interns_a_bundle` -- a struct with a
    `[ i64 -- i64 i64 ]` field carries the same `is_bundle` struct after `check`.
  - `array_element_quotation_two_outputs_interns_a_bundle` -- a word taking
    `( &[ [ i64 -- i64 i64 ] 2 ] -- )` (an array whose element is a two-output quotation,
    taken by reference): the walk resolves `module.arrays[id].element` to reach the nested
    `Type::Quotation` and interns the same `is_bundle` struct. This exercises the one live
    composite site (R2's array-registry step).
  - `quotation_param_single_output_interns_no_bundle` -- a `[ i64 -- i64 ]` param leaves
    `module.structs` with no `is_bundle` struct for `[i64]` (the `>= 2` filter holds; guards
    the walk against over-interning).
  - `variable_bearing_poly_quotation_interns_no_bundle` -- a poly word declaring a
    variable-bearing quotation parameter `[ 'T -- 'T 'T ]`: running the widened
    `intern_output_bundles` directly over the parsed module (as the drop-overload tests call
    `find_drop_overloads` directly) interns no `is_bundle` struct -- the parameter's output
    row is not ground, so R3 descends it for a *nested* concrete quotation but never hands
    it to R2 -- and a full `check` of a body that `call`s it is still rejected
    (`poly_op_on_variable_error`, `src/check/poly.rs:1372`). Pins that widening discovery
    neither accepts nor interns a bundle for a non-ground quotation shape.

- **Golden / integration, `tests/phase7_slice3m.rs`** -- both goldens follow the harness in
  `tests/phase7_slice3f.rs`/`tests/phase7_slice3p.rs`: a `Scratch` single-file program
  carrying its own `sooth.pkg` (`common::fixture_package`), an `import: intrinsics * ;`
  preamble (plus `import: core::bool * ;` where a `Bool` instantiation is used), built and
  run through the `build_and_run` helper, asserting exit code `0` and exact stdout. Both run
  and print the *right* values (not merely no panic):
  - `concrete_call_it_pushes_both_quotation_outputs` -- source:

    ```sooth
    import: intrinsics * ;
    : call_it ( [ i64 -- i64 i64 ] -- ) 3 swap call add . ;
    : main ( -- ) [ dup ] call_it ;
    ```

    `3 swap call` runs `[ dup ]` on `3` (`3 -> 3 3`), `add` sums them, `.` prints. Expected
    stdout: `6\n`.
  - `polymorphic_call_it_pushes_both_quotation_outputs` -- S3f's R3 `call_it`
    (`( 'T: Copy [ i64 -- 'T ] -- 'T )`) is single-output/variable-output and cannot type a
    two-output ground quotation, and `call` requires a fully-ground quotation operand
    (`src/check/poly.rs:1367`), so this golden instead uses a word polymorphic over an
    unrelated `'T` that takes a *fixed* two-output `[ i64 -- i64 i64 ]` parameter, run at two
    instantiations of `'T` so the variable is carried rigidly (the S3f golden shape). The
    word's own declared output stays a *single* `'T` -- giving it two outputs
    (`( ... -- 'T i64 )`) would make the golden a placebo, since the `'T = i64`
    instantiation's own return bundle happens to be `[i64 i64]`, the exact tuple the
    quotation needs, so the program would build and print correctly even with discovery
    unwidened. Source:

    ```sooth
    import: intrinsics * ;
    import: core::bool * ;
    : call_it ( 'T: Copy [ i64 -- i64 i64 ] -- 'T ) 3 swap call add . ;
    : main ( -- )
      9 [ dup ] call_it .
      True [ dup ] call_it .
    ;
    ```

    Entry stack `['T, q]`; `3 swap call` runs `[ dup ]` on `3` beside the untouched `'T`
    (`['T, 3, 3]`), `add` sums (`['T, 6]`), then `.` prints the sum before `call_it` returns
    the untouched `'T`, which `main` prints. Expected stdout: `6\n9\n6\nTrue\n`. This
    exercises the `w.poly` walk (site 4) through a build.

## Sizing

Small: one function's discovery walk widened to four additional sites; no other file
touched. The design fork this brief would otherwise leave open (a second pass over
inferred-only quotation literals or per-instantiation poly shapes) is closed by the two
invariants above, so it is stated as the mechanism, not a question.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Widen intern_output_bundles discovery to word inputs, struct fields, array elements, and poly signatures, recursing Type and PolyType to reach every ground multi-output Type::Quotation; unit tests beside it plus the concrete and polymorphic call_it end-to-end goldens", "effort": "S", "difficulty": "standard" }
  ]
}
```
