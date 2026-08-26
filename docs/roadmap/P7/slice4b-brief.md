# Phase 7 Slice 4b: bounds on impl variables (brief)

**Sequence after P7.S4.** S4 shipped generic `impl:` targets with a specificity chain, but
deliberately left bounds on an impl's own type variables out of scope: a generic impl's
member word carries `PolySig { bounds: vec![], .. }`, so `poly_trait_member_call`
(`src/check/poly.rs:949`, the `for (v, bound) in &sig.bounds` loop) does not recognize a
trait-member call on an impl variable, and it falls through to ordinary word lookup, which
fails to resolve. This slice closes that gap.

**The consumer.** S4's brief named "Show and `Eq` over a shape family" as the motivating
consumer, but S4 could only deliver the shape-only forms (print the length, compare lengths).
The element-wise forms — `impl: Show for ['T N]` whose `show` iterates and calls `show` on
each element, or `impl: Eq for ['T N]` whose `eq` compares element-by-element — require
`'T: Show` / `'T: Eq` declared as bounds on the impl's variable, and recursive
per-instantiation dispatch: when the generic impl's member word is monomorphized at a concrete
`[Point 4]`, the body's `show` call on each element must resolve to `Point`'s `impl: Show`,
discovered by discharging the `'T: Show` bound against the concrete `Point` — the same
`resolve_user_bound` lookup that discharged the *caller's* bound, now applied to the impl's
*own* variable.

## What S4 landed (the foundation S4b extends)

S4's spec (R5/R6) made the generic impl's synthesized member word polymorphic:
`WordDef.poly = Some(PolySig)` over the impl's own variables, with the trait's single
self-variable bound to the whole target `PolyType`. The member word flows through the existing
poly machinery unchanged — `check_poly_body` walks it once, the call-site loop monomorphizes
it per concrete instantiation, and `resolve_user_bound` mints
`instantiation_symbol(member_word_symbol, matched_subst)` for the winning candidate. The
`ImplDecl` gained an `ImplTarget` wrapper holding the `PolyType` pattern and the impl's own
`ty_var_names`/`len_var_names`.

**What S4 left empty: `PolySig::bounds`.** The member word's `PolySig` is constructed with
`bounds: vec![]`. The existing `poly_trait_member_call` machinery already iterates `sig.bounds`
to dispatch trait-member calls — if those bounds were populated, the body check would
recognize `show` as a trait dispatch on `'T` and record an obligation, exactly as it does for
an ordinary poly word with a `'T: Show` bound. The gap is purely that no bounds are declared
or threaded through.

## The three pieces

**1. Grammar for impl-bound declarations.** Sooth has no `where`-clause or impl-bound syntax
today; the existing bound grammar (`'T: Copy Ord` at a variable's binding site in a word
signature, parsed by `parse_poly_ty_var` at `src/parser.rs:3161` and resolved by
`parse_capabilities` at `src/parser.rs:3357`) attaches bounds to type-variable *binding
occurrences* in `PolySig` slots. An impl target's variables are bound by the target pattern
itself, not by a signature slot, so the bound grammar needs a new attachment point. The
natural shapes, to be resolved in spec:

- A `where`-style clause after the target: `impl: Show for ['T N] where 'T: Show`.
- Bounds inline in the target pattern, reusing the existing `:` grammar:
  `impl: Show for ['T: Show N]`.
- A binder bar mirroring member-body binders: `impl: Show for ['T N] | 'T: Show |`.

The grammar must compose with S4's specificity chain: two generic impls with the same target
pattern but different bounds (e.g. `impl: Eq for ['T N] where 'T: Eq` vs `impl: Eq for ['T N]`
with no bounds) are distinct declarations, and a body that needs the bound dispatches through
the bounded one. Whether bounds participate in specificity (a bounded target is more specific
than an unbounded one with the same pattern) or whether boundedness is a declaration-time
rejection (two impls with the same pattern but different bounds is a duplicate) is an open
question for the spec.

**2. Threading bounds into the member word's `PolySig`.** Once parsed, the impl's bounds
populate the member word's `PolySig::bounds: Vec<(u32, Bound)>` — the same field
`poly_trait_member_call` already reads. The `ImplTarget` (or `ImplDecl`) carries the declared
bounds alongside the `PolyType` pattern and variable name tables, and `parse_impl_member_body`
(`src/parser.rs:2597`) constructs the member word's `PolySig` with those bounds filled in
rather than empty. This is a wiring change, not a new mechanism: every downstream consumer
(`check_poly_body`, `poly_trait_member_call`, the obligation recorder) already handles bounded
variables in a `PolySig`.

**3. Recursive per-instantiation bound discharge.** This is the genuinely new mechanism. When
a generic impl's member word is monomorphized at a concrete instantiation, the bounds on its
variables become concrete obligations that must be discharged against the impl registry. For
`impl: Show for ['T N] where 'T: Show`, instantiated at `[Point 4]`: the `'T: Show` bound
becomes `Point: Show`, and `resolve_user_bound` (`src/check/poly.rs:5308`) must look up
`impl: Show for Point` to resolve the body's `show` calls on each element. This is the same
`resolve_user_bound` that discharges the *caller's* bound, now applied recursively to the
*impl's own* bound — but keyed to the impl's variable namespace and the matched `Subst`,
not the caller's `PolySig`.

The recursion terminates because the type shrinks each step: `[Point 4]` → `Point` → (no
bounds on `Point`'s impl, if any). A cycle (an impl whose bound requires itself at the same
type) is a located error, not infinite recursion — detected by a visited-set of
`(TraitId, Type)` pairs carried through the discharge.

## What this does not touch

- `poly_trait_member_call`, `resolve_user_bound`, `TraitResolveCtx`, and the whole obligation
  recording/resolution mechanism — reused, not modified. The new work is populating the
  `PolySig::bounds` field and adding the recursive discharge call, not changing how bounds
  dispatch.
- S4's matching, specificity, and orphan rule — inherited as-is. S4b adds bounds *on top of*
  a generic impl target; it does not change how the target pattern matches a concrete type.
- `drop` (still not a trait, still synthesized field-wise glue); trait objects (S3u, parked);
  default member bodies and supertraits (still unforced).
- The REPL path — `impl:` remains a build-time, whole-program-assembly feature.

**Exit:** a generic `impl:` target may declare bounds on its own type variables; the member
word's `PolySig` carries those bounds; a body that calls a trait member on an impl variable
type-checks and dispatches correctly; the bound is discharged recursively at each concrete
instantiation (so `impl: Show for ['T N] where 'T: Show` instantiated at `[Point 4]` resolves
each element's `show` to `Point`'s impl); a self-referential bound cycle is a located error;
and the element-wise `Show`/`Eq` programs that S4 explicitly deferred compile and run
identically to their hand-written per-element concrete counterparts.
