# Phase 7 Slice 4: generic `impl:` targets, with a specificity chain (brief)

An `impl:` target must name one concrete type: the whole-program registry S3e built keys on
`(TraitId, Type)` and discharges a bound per concrete instantiation, so a trait with N
conforming shapes needs N hand-written `impl:` blocks with identical bodies. S3e listed
blanket impls as out of scope with no consumer forcing them; `Show` and `Eq` over a shape
family are that consumer. Both ends reject a non-concrete target today, at different layers:
`impl: Show for 'T` resolves no type ("unknown type `'T`"), and `impl: Show for [i64 2]` does
not parse (`parse_impl_decl` takes the target through `parse_type_expr`, which does not admit
an array form here).

An `impl:` target may name type variables and shape constructors over them
(`impl: Show for ['T N]`), and where several targets match a concrete type the most specific
wins. Sooth is unusually well placed for this: specialization's soundness holes in other
languages come from lifetimes and associated types, and Sooth has neither, while dispatch is
whole-program and monomorphizing, so the winning `impl:` is chosen statically per instantiation
with no cross-unit coherence question and no runtime cost.

**Specificity is a partial order, and this slice rules that ambiguity is an error.** Targets
`['T N]`, `[i64 N]` and `['T 4]` all match `[i64 4]`, and the last two are incomparable: neither
matches a strict subset of what the other does. Selecting one requires a tiebreak (declaration
order, arity, leftmost-concrete) and every such rule is invisible at the use site. So an
unordered candidate set is a located error naming the competing targets, and the user resolves
it by writing the more specific `impl:` (here `[i64 4]`). No tiebreak rule is introduced.

Out of scope: `drop`, which is not a trait and is not becoming one -- its blanket behaviour is
synthesized field-wise glue, not a writable default body, and an owning closure's disposer keys
on the construction site rather than the type (**P7.S3v**); trait objects (**P7.S3u**, parked);
default member bodies and supertraits, still unforced. Sequence after **P7.S3s**, which is the
first slice to give the impl registry a real multi-`impl:` consumer in `core`.

**Exit:** an `impl:` target may name type variables and shape constructors over them; a bound is
discharged against it for every matching concrete instantiation with no per-shape `impl:`
written; a more specific target overrides a more general one at the instantiations they share;
an unordered candidate set is a located error naming the competing targets; and a trait with one
generic `impl:` behaves identically to the same trait with the hand-written concrete `impl:`
blocks it replaces.

## Paper pre-check (verified against `main` 2026-08-26)

The brief's two rejection claims were probed live. One is stale, one is confirmed.

**`impl: Show for 'T` → "unknown type `'T`" — CONFIRMED.** The type variable is
rejected at `resolve_type_or_apply` (parser.rs), the same path any type name takes. The
generic array form `impl: Show for ['T N]` also rejects here: the `[` is admitted by
`parse_type_expr` (parser.rs:3682, dispatches `LBracket` to `parse_array_type_expr`), but
the `'T` element hits the same "unknown type `'T`" rejection. So the gap is type-variable
*resolution*, not array parsing.

**`impl: Show for [i64 2]` — STALE, fully compiles.** The brief says this "does not parse
(`parse_impl_decl` takes the target through `parse_type_expr`, which does not admit an array
form here)." Live test: `impl: Show for [i64 2]` with a member body builds, links, and
produces a 17KB executable. `parse_type_expr` already dispatches `[` to
`parse_array_type_expr`; the concrete array form has been admitted since P4. The brief's
wording should read "a concrete array target parses and compiles; a type-variable-bearing
target does not resolve" rather than claiming the array form itself is rejected.

### Registry key confirmed: exact `(TraitId, Type)` equality

`resolve_user_bound` (poly.rs:5308) scans `tr.impls` linearly for an exact match:
`i.trait_id == trait_id && i.target_ty == ty` (poly.rs:5333). `ImplDecl::target_ty` is a
plain `Type` (ast.rs:1829), not a `PolyType`. The duplicate check in `check_impl_decls`
(declarations.rs:421) does the same exact `(TraitId, Type)` equality.

### `PolyType` infrastructure already exists — the pattern half is missing

`PolyType` (ast.rs:1853) already has every shape constructor S4 needs:
`Var(u32)`, `Array(Box<PolyType>, Len)`, `Generic { .. }`, `Ref`, `OwnedCell`,
`Quotation`. `Len` (ast.rs:1844) has `Concrete(u32)` and `Var(u32)`. `apply_subst`
(poly.rs:5774) already grounds a `PolyType` against a `Subst` to produce a concrete
`Type` — the substitution direction. **S4 needs the reverse**: given a concrete `Type` and a
`PolyType` pattern (the impl target), determine if the pattern matches and extract the implied
`Subst`. This is one-way matching (the concrete type has no variables), not full unification.

### Three design questions the spec must answer

1. **`ImplDecl::target_ty` type change.** It must move from `Type` to `PolyType` (or a
   new `ImplTarget` wrapper) to admit `Var` and `Array` patterns. The parser's
   `parse_impl_decl` (parser.rs:2540) currently calls `parse_type_expr` returning `Type`;
   it needs a variant that returns `PolyType` (admitting type variables, like
   `parse_poly_type_expr` if one exists, or a new one).

2. **Orphan rule for generic targets.** `impl_target_module` (declarations.rs:403)
   extracts the defining module from `Type::Struct`/`Type::Enum`. A generic
   `impl: Show for ['T N]` has no single struct/enum, so the orphan check needs a new
   rule — likely "the impl lives in the trait's module" (the only module with a stake in
   the trait), which is stricter than the current rule but sound for blanket impls.

3. **Specificity as a partial order.** The matching function produces a candidate set.
   Specificity is "does pattern A match a strict subset of what pattern B matches?" —
   computed structurally: `['T N]` is more general than `[i64 N]` (the element is a
   variable vs concrete) and than `['T 4]` (the length is a variable vs concrete). Two
   patterns are incomparable when neither's constraints subsume the other's. The
   winning candidate is the unique maximal element; an unordered set is a located error.
   This is not the same as the duplicate check — two impls can overlap without being
   identical, and overlapping impls are legal as long as one is more specific.
