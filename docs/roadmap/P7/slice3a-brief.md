# Phase 7 Slice 3a: generic instantiation over a poly word's own type variable (brief)

Split out from P7.S3b (`docs/roadmap/P7/slice3b-brief.md`) as its own prerequisite:
discovered as a compiled blocker during that slice's paper dogfood
(`docs/roadmap/P7/slice3-dogfood.md`, finding #5), not planned work. A polymorphic
word cannot today name a generic type applied to its own type variable —
`Box['T]`, `Option['T]`, `Map['K 'V]` inside a poly signature all fail — while a
concrete generic application (`Result[i64 str]` in a monomorphic word) and an
array carrying a poly variable (`['T: Copy 4]`) both already work.

## Recon (verified against the built compiler by the dogfood worker; parser side

only — unification/monomorphization/lowering not yet traced)

1. **The failure is confirmed by compiling, not inferred.**
   `: unbox ( Box['T] -- 'T )` and `: or-default ( 'T Option['T] -- 'T )` both fail
   `` error: unknown type 'T ``. Control: `: setat ( ['T: Copy 4] 'T -- ['T 4] )`
   (`examples/poly_borrow_setat.sth`) builds green — an *array* type carrying a
   poly variable in a signature works; a *named generic* carrying one does not.
   This isolates the gap to generic-type-argument resolution specifically, not to
   poly signatures containing type variables in general.

2. **Root cause traced to one function: generics monomorphize eagerly, at parse
   time, and only accept concrete arguments.** `resolve_type_or_apply`
   (`src/parser.rs:3129-3172`) is the single path a generic name goes through. For
   a struct or enum generic it calls `parse_type_arguments` to collect the
   argument list, then immediately calls `instantiate_struct`/`instantiate_enum`
   (`parser.rs:3153`, `3167`) to produce a concrete `Type`. Each argument is
   itself resolved through the ordinary `resolve_type` path
   (`parser.rs:2735-`), which only knows registered concrete type names — a bare
   `'T` is not one, hence "unknown type `'T`."

3. **Every existing use of a generic type is a concrete argument to a
   monomorphic word.** `Result[i64 str]`, `Option[i64]`, etc. all resolve fine
   because nothing upstream of `resolve_type_or_apply` is itself abstract. This
   slice's gap only appears the first time a generic is nested inside *another*
   generic word's own signature — a case Phase 5 never needed and never built.

4. **The likely shape of a fix, by analogy to the last time a type had to become
   nameable-but-abstract.** Phase 4 Slice 6a gave quotations a `Type`/`PolyType`
   variant, unification, and `apply_subst`, deferring real resolution to
   monomorphization; Slice 7a then gave it a runtime representation. A generic
   application over an abstract argument plausibly needs the same shape one level
   in: a `PolyType` variant carrying "generic X applied to (possibly abstract)
   arguments," participating in unification and `apply_subst` the same way a bare
   `PolyType::Var` does today, and resolved to a real, concrete, monomorphized
   `Type` only once `check_poly_call` (`src/check/poly.rs:1430-`) has a concrete
   `Subst` for the enclosing word — the same point in the pipeline where
   everything else about the poly word becomes concrete. **Not yet verified**:
   this recon has only read the parser's failure point; unification,
   `apply_subst`, the monomorphization walk, layout, and lowering all need their
   own pass before this shape is trusted, on this project's own standing rule
   (verify claims of verification; the last two probes both falsified a brief's
   "should be free" claim on exactly this kind of reasoning-not-compiling gap).

## Open questions

1. **Does this need a new `PolyType` variant, or can an existing one be
   repurposed?** Recon 4 assumes a new variant by analogy; not confirmed against
   `src/ast.rs`'s actual `PolyType` enum and what already exists there.

2. **Interaction with monomorphization identity.** A generic struct/enum today is
   keyed by its concrete instantiation (`Box[i64]` and `Box[usize]` are distinct
   monomorphs). Once `'T` inside `Box['T]` can vary per outer instantiation, does
   `Box['T]`'s *own* monomorph get minted once per outer `Subst`, or does it need
   to unify with an already-existing concrete `Box[Sprite]` monomorph if one
   exists elsewhere in the program? Get this wrong and two call sites could mint
   duplicate, incompatible monomorphs of the same concrete generic.

3. **Does this interact with the asymmetric-instantiation hazard already on
   record** (`workflow_symmetric_instantiation_placebo`)? A generic applied to a
   poly variable, itself instantiated at two different concrete types, is exactly
   the shape that hazard warns about — the spec/tests for this slice should
   deliberately include a multi-type-variable generic (`Map['K 'V]`, not just
   `Box['T]`) instantiated asymmetrically, not `Box[i64]` proven twice.

4. **Scope: does this need to support a generic applied to a *poly variable of
   a poly variable*, or nesting depth > 1?** (`Box[Box['T]]`). No consumer forces
   this yet; recommend explicitly scoping to depth 1 unless the S3b/S4 dogfood
   needs more.

5. **Relationship to P7.S3b.** Independent in mechanism (this is a parser/type-
   system change; S3b is a checker-whitelist-and-lowering change), but S3b's
   `Map['K 'V]` consumer needs this slice to land first. The array form of `sort`
   in S3b does not depend on this at all and can proceed regardless of this
   slice's timeline.

## Out of scope

- Trait bounds (P7.S3b's concern entirely).
- Nesting depth beyond 1 (OQ4), unless a real consumer forces it.
- Any change to how a *concrete* generic argument resolves — that path is
  unaffected and stays exactly as it is.

## Ready to spec?

Not yet. The parser-side root cause is solid (compiled and confirmed), but nothing
downstream (unification, `apply_subst`, monomorphization, layout, lowering) has
been probed. Recommend the same discipline that caught S3b's two false claims:
before locking a spec, write and compile the smallest real probe — a poly word
applying an existing generic (`Option['T]`) to its own variable, instantiated
asymmetrically at two real types — and read what actually happens at each stage,
rather than reasoning from the parser fix alone.
