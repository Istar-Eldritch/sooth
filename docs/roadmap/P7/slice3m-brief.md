# P7.S3m brief — A declared quotation effect with two or more outputs cannot be lowered

## Problem, confirmed live against current `main`

`intern_output_bundles` (`src/check.rs:1005-1015`) is the only place a multi-output bundle
struct gets interned:

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

It walks only `module.words` — a *declared word*'s own top-level output list. A quotation
value's effect (`[ i64 -- i64 i64 ]`) is never inspected here, even when it appears as a
word's parameter or output type. `bundle_of` (`src/ir/func_builder/mod.rs:32-38`) looks the
tuple up by exact type list (`structs.bundle_for(&tys)`) and returns `None` when nothing
was interned; `lower_indirect_call` (`src/ir/func_builder/quotation.rs:203-`) uses that
`None` to skip pushing a return value at all, so the quotation's second (and later) output
is simply never produced. The first consumer that reads it underflows the stack.

**Confirmed live:** `: call_it ( [ i64 -- i64 i64 ] -- ) 3 swap call add print ;` panics at
`src/ir/func_builder/calls.rs:451` (`rhs = self.stack.pop().expect("bin: rhs")` inside the
`add` handler) — identically whether `call_it` is concrete or, per S3f's R3, polymorphic.

## Existing precedent (what's already there to build on)

**`intern_bundle_struct`, `bundle_of`, and `word_ret_ty` are shape-agnostic.** None of the
three cares whether the tuple it's given came from a word's own outputs or a quotation's —
they just take `&[Type]`/`&[TypedSlot]` and look up or mint a bundle. The gap is entirely in
*discovery*: nothing ever hands them a quotation's output tuple. Widening
`intern_output_bundles`'s walk is additive; no downstream consumer needs to change.

**A poly word's signature isn't in `w.effect` at all.** `w.effect` is populated only for a
concrete word; a polymorphic word's declared shape lives in `w.poly` as a `PolySig` of
`PolyType`s (`ast.rs:1583` region). A walk that only reads `w.effect` silently misses every
poly word's quotation-typed parameters and outputs — this is not a corner case, it's the
entire polymorphic half of the probe's confirmed repro.

## Probe result (2026-08-24): the discovery-scope question is closed, no design fork survives

A hand-patched build (widened `intern_output_bundles`, reverted after; `git status --short`
clean) confirmed the fix and closed both discovery-scope questions this brief would
otherwise have had to leave open for spec-writer:

- **The straightforward widen is sufficient and it is exactly a widen, not a redesign.**
  Recursively collect every `Type::Quotation`'s output tuple (len >= 2) reachable from:
  `module.words[].effect.inputs`/`.outputs`, `module.structs[].fields`,
  `module.enums[].variants[].fields`, and — separately, since `effect` is empty there — each
  poly word's `w.poly.inputs`/`.outputs` (recursing through `PolyType::Array`/`Ref`/
  `Quotation` wrappers to reach any nested `Concrete(Type::Quotation(..))`). With this in
  place both the concrete and polymorphic `call_it` probes ran correctly (printed the right
  values, not merely "didn't panic"), and the full suite (57 test binaries) stayed green.

- **No orphan quotation effect exists — probed directly, not assumed.** The natural worry
  (an inferred-only quotation effect that never appears in any module-level declaration,
  making a signature walk incomplete by construction) does not survive contact with an
  existing, unrelated checker invariant. A quotation literal called in the same branch it's
  built in is *spliced* at compile time and never reaches `lower_indirect_call`'s runtime
  path at all. Any quotation that *does* reach that runtime path — because it was merged at
  a branch join, or stored into an array/ref/struct field, and so must be *materialized* —
  is already rejected by the checker unless it has a declared home:
  `different_quotations_at_join_error` (`src/check/terms.rs:1636-1640`) requires "a declared
  type (a word output or field) so it can be materialized" at the join, and the array/struct
  storage boundary is gated the same way. So every quotation effect capable of reaching the
  panic site is, by construction, already visible to a signature walk — there is no second
  discovery pass to design.

- **No per-instantiation poly shape escapes the walk either — also probed directly.** A poly
  quotation parameter whose *own* output row still mentions the type variable
  (`[ 'T -- 'T 'T ]`) is statically rejected for `call` inside a poly body
  (`poly_op_on_variable_error`, e.g. `poly.rs:1372`); only a fully-ground
  `PolyType::Concrete(Type::Quotation(..))` is callable at all (`poly.rs:1367` region). By
  the time a `call` on a quotation-typed operand is legal, its effect is already concrete —
  the widened walk over `w.poly`'s declared (possibly-variable-bearing) signature already
  finds the ground case; there is no later, only-visible-post-instantiation shape that could
  slip past it.

## Exit criteria (from the roadmap, unchanged)

A declared quotation effect with two or more outputs interns an output bundle the same way
a declared word does, and `call`ing one on either the concrete or polymorphic path pushes
all declared outputs rather than panicking.

## Sizing

Small — one function's discovery walk widened to four additional sites (word inputs, struct
fields, enum variant fields, poly signatures), no other file touched. The design fork this
brief would otherwise have needed to leave open (does discovery need a second pass over
inferred-only quotation literals or per-instantiation poly shapes) is closed by the probe:
neither case is reachable, per two independent existing invariants cited above. Recommend
spec-writer state the widened walk as the mechanism directly, and cite the join-materialization
invariant (`terms.rs:1636-1640`) and the ground-only `call` guard (`poly.rs:1372`/`:1367`) as
the reason no second discovery pass is in scope, so a reviewer doesn't reopen either question.

## Ready to spec: yes, probe-validated, no open design questions

Verify every citation above against live `main` before writing — `src/check.rs`,
`src/check/poly.rs`, and `src/check/terms.rs` are active files other in-flight slices also
touch.
