# Spec: P7.S4 — Generic `impl:` targets with a specificity chain

**Status:** Draft  
**Created:** 2026-08-26  
**Discovery:** `docs/roadmap/P7/slice4-brief.md`

## Problem Statement

A trait's `impl:` target must today name exactly one concrete type: the whole-program
registry P7.S3e built keys on exact `(TraitId, Type)` equality, so a trait conforming
over a family of shapes (e.g. `Show`/`Eq` over every array shape `['T N]`) needs one
hand-written `impl:` block per shape, all with identical bodies. Authors of `core` and
stdlib-shaped families pay an N-for-1 cost and a maintenance hazard: add a shape, forget
an `impl:`, get a located "unsatisfied bound" far from the cause. Sooth is unusually well
placed to fix this — dispatch is whole-program and monomorphizing with no cross-unit
coherence question and no runtime cost, and the language has neither lifetimes nor
associated types (the two soundness holes specialization opens elsewhere) — but the
registry currently has no notion of a *pattern* target, no way to choose among several
matching ones, and no orphan rule for a target that names no single struct/enum.

## Requirements

- **R1.** An `impl:` target parses as a `PolyType` pattern over the impl's own type and
  length variables — `['T N]`, `[i64 N]`, `['T 4]`, `Box['T]` — not a concrete `Type`,
  and the `ImplDecl` carries the pattern together with the impl's variable name tables
  (mirroring `PolySig`'s `ty_var_names`/`len_var_names`). A concrete target
  (`Point`, `[i64 4]`) still parses and behaves exactly as today.
- **R2.** At a `Bound::User` call site, the registry one-way-matches the concrete
  instantiation `Type` against each `impl:` target `PolyType`, producing a `Subst` (in
  the impl's own variable space) for each match; a concrete target matches only the
  equal concrete type (the existing exact path, unchanged).
- **R3.** Where more than one target matches a concrete type, the most specific wins.
  Specificity is an equivalence-class refinement partial order: a pattern's shared
  variables define an equivalence relation on positions (positions sharing the same
  variable must have the same concrete value). Pattern A is more specific than B
  (A ≺ B) iff (1) every position where B has `Concrete(t)`, A also has `Concrete(t)`
  (A doesn't relax B's concreteness); (2) B's equivalence classes refine A's — every B
  equivalence class is a subset of some A equivalence class (A is coarser/more-merged/
  more-constraining; B is finer/more-fragmented); and (3) A is strictly more constrained
  somewhere (A has `Concrete` where B has `Var`, or B has a finer partition (A has a
  coarser partition)). Concrete positions are singletons. This handles `Map['T 'T]` ≺
  `Map['T 'U]`, `Map[i64 'T]` ⊥ `Map['T 'T]` (incomparable), and the existing examples
  (`[i64 N]` ≺ `['T N]`, `['T 4]` ≺ `['T N]`, `[i64 N]` ⊥ `['T 4]`). Two candidates whose
  constraints neither subsume the other are incomparable; an unordered candidate set is
  a located error. No tiebreak rule is introduced.
- **R4.** A generic target — one whose `PolyType` is not a single concrete
  `Type::Struct`/`Type::Enum` — declares no struct/enum module of its own, so the
  `impl:` must live in the trait's own module; a generic `impl:` declared outside the
  trait's module is a located error. Concrete targets keep the existing rule (the
  trait's module **or** the target's struct/enum module).
- **R5.** A generic impl's synthesized member word is a polymorphic word
  (`WordDef.poly = Some(PolySig)` over the impl's own variables, with **no** bounds on
  them), its signature the trait member's signature grounded by binding the trait's
  single self-variable to the *whole* target `PolyType`; it is checked by the existing
  poly-body pass and monomorphized per concrete instantiation. A concrete impl's member
  word stays monomorphic (`poly: None`) with a concrete `StackEffect`, exactly as today.
- **R6.** For a generic-impl winner, `resolve_user_bound` mints the dispatched symbol as
  `instantiation_symbol(member_word_symbol, matched_subst)` (not the bare
  `word_symbols[idx]`), and the check pass records the member-word monomorph so lowering
  emits its body under that symbol. A concrete-impl winner keeps the existing bare
  `word_symbols[idx]` dispatch path.
- **R7.** Two `impl:` blocks for one trait with structurally-equal (alpha-equivalent)
  targets are a located duplicate error; two with overlapping-but-unequal targets are
  accepted as declarations (the overlap is resolved by specificity at the dispatch site,
  not rejected at declaration time). The duplicate check compares `target.pattern` (the
  `PolyType`), NOT the whole `ImplTarget` — comparing `ty_var_names`/`len_var_names`
  would make `['T N]` and `['U M]` unequal (different name strings) and break
  alpha-equivalence. Do not derive `PartialEq` on `ImplTarget` for the duplicate check.
- **R8.** (NFR, diagnostics) An ambiguity — incomparable matching candidates — is a
  located error at the dispatch site naming the trait, **every** competing target, and
  the concrete instantiation.
- **R9.** (NFR, diagnostics) A generic `impl:` outside the trait's module is a located
  orphan error at the `impl:` declaration naming the trait and the target.
- **R10.** (NFR, parity) Every existing concrete `impl:` dispatches byte-for-byte
  identically; the existing corpus (`examples/traits.sth`, `lib/cmp.sth`, `core`'s
  `sort`) compiles and runs unchanged, and `cargo fmt --check && cargo clippy -- -D
  warnings && cargo test` stays green. The overload-overlap path (`poly_admits` and
  `poly_sig_could_match`) is covered: concrete impls behave identically via the
  matcher's `Concrete(t)` arm (which matches iff `t == ty`), so the existing
  `(TraitId, Type)` equality semantics are preserved for concrete-only impls.
- **R11.** (NFR, golden) A golden test demonstrates that a single generic `impl:`
  compiles and runs identically to the hand-written concrete `impl:` blocks it
  replaces (same lowered symbols, same output).
- **R12.** (NFR, golden) A golden test demonstrates that a more-specific target
  overrides a more-general one at their shared instantiations, and that incomparable
  matching candidates produce the located ambiguity error.
- **R13.** (NFR, golden) A golden test demonstrates that a generic `impl:` outside the
  trait's module is rejected with the located orphan error.

## Success Criteria

- `impl: Show for ['T N]` (and `[i64 N]`, `['T 4]`) parses; `impl: Show for 'T` resolves
  the type variable instead of erroring "unknown type `'T`".
- A polymorphic word `shows ( &'T: Show -- )` called at `[i64 4]` dispatches to a single
  generic `impl: Show for ['T N]` and the program runs identically to one with a
  hand-written `impl: Show for [i64 4]`.
- A concrete `impl: Show for [i64 4]` overrides a generic `impl: Show for ['T N]` at
  `[i64 4]`; the generic covers every other array shape.
- Two incomparable matching targets (`[i64 N]` vs `['T 4]` at `[i64 4]`) produce a
  located error naming both targets and `[i64 4]`.
- A generic `impl:` declared outside the trait's module is rejected with a located
  orphan error; the same `impl:` in the trait's module is accepted.
- `examples/traits.sth`, `lib/cmp.sth`, and `core`'s `sort` compile and run unchanged;
  `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
- Each new function (target parser, one-way matcher, specificity, generic-orphan) has
  unit tests beside it: a happy path plus at least one error/edge case.

## Scope & Boundaries

**In scope:**

- The `ImplDecl` target type change from `Type` to a pattern (`PolyType` + the impl's
  variable namespace), the parser path that admits type variables in an `impl:` target,
  one-way pattern matching (concrete `Type` vs `PolyType` pattern → `Subst`), the
  specificity partial order with ambiguity-as-error, the orphan rule for generic
  targets, and the polymorphic-member-word + per-instantiation monomorphization that
  makes a generic impl behave identically to its hand-written concrete counterparts.

**Out of scope (per the brief and supervisor decision 2026-08-26):**

- **Bounds on the impl's own variables.** A generic impl's member body treats the
  impl's variables as opaque rigid variables with no declared bounds. A body that needs
  element-level trait dispatch (e.g. `impl: Show for ['T N]` whose `show` iterates and
  calls `show` on each element, requiring `'T: Show`) is **not** writable this slice:
  with no bound on the variable, `poly_trait_member_call` does not recognize the call as
  trait dispatch and it falls through to ordinary word lookup, which fails to resolve.
  This is the deferred follow-up slice (element-wise `Show`/`Eq` over a shape family via
  new impl-bound declaration grammar + recursive per-instantiation dispatch).
- `drop` as a trait (synthesized field-wise glue, never a writable default body; an
  owning closure's disposer keys on the construction site, **P7.S3v**).
- Trait objects / runtime dispatch (**P7.S3u**, parked); default member bodies and
  supertraits, still unforced; multi-type-variable traits.
- The REPL path — a REPL session declares no `impl:` (the parser's REPL/`parse` path is
  unchanged; `impl:` remains a build-time, whole-program-assembly feature, sequenced
  after **P7.S3s** which first gave the registry a real multi-`impl:` consumer in
  `core`).

## Design Decisions & Rationale

**Polymorphic member words over eager expansion.** A generic impl's synthesized member
word must be polymorphic (`poly: Some(PolySig)` over the impl's own variables). The
trait's single self-variable (`'T`, always id 0 in a member `PolySig`) binds to the
*whole* target `PolyType`, yielding a `PolySig` over the impl's variables. The word then
flows through the existing machinery unchanged: `check_poly_body` walks its body once
and records its obligations/cross-calls; the call-site loop monomorphizes it per
concrete instantiation; `discover_transitive_instantiations` composes its
generic-calls-generic cross-calls. The eager-expansion alternative (generate N concrete
`ImplDecl`s from one generic one) has a circular dependency — instantiations are
discovered *during* `check`, which already needs the impl registry — so reusing the
existing monomorphization pipeline is the architecturally forced path, not a choice.
Supervisor-confirmed 2026-08-26.

**Specificity as equivalence-class refinement.** The specificity order is not a
pointwise field count ("more concrete positions = more specific"). It is an
equivalence-class refinement: a shared variable in A linking positions B keeps separate
makes A more specific; a shared variable in B that A breaks apart would make A less
specific. The winning candidate is the unique maximal element; an unordered candidate
set is a located error. No tiebreak is introduced — the user resolves ambiguity by
writing the more specific `impl:`.

**Orphan rule follows from the existing check.** A generic target names no single
struct/enum, so `impl_target_module` returns `None` for it, and the existing orphan
condition (`impl_module != trait_decl_module && Some(impl_module) != target_module`)
reduces to `impl_module != trait_decl_module` — a generic impl must live in the trait's
module. The rule is already enforced; only the diagnostic rendering needed updating to
name the `PolyType` target.

**Monomorph recording.** The check pass records each generic-impl dispatch as a
`(member_word, impl_subst)` monomorph so lowering emits the member-word body. Because a
generic impl's member word is reached only through trait dispatch (never an ordinary
call site), and its body's own generic-calls-generic cross-calls need the same
composition, `transitive_instantiations` is the natural home (deduped by symbol,
populated via `discover_transitive_instantiations`).

## Open Questions

- [x] ~~**Member-word model.** Polymorphic member words reusing the existing poly
  machinery, vs eager expansion into concrete `ImplDecl`s.~~ Resolved: polymorphic
  member words — eager expansion has a circular dependency (instantiations are
  discovered during `check`, which needs the impl registry). Supervisor-confirmed
  2026-08-26.
- [x] ~~**Bounds on the impl's own variables.** In or out of scope.~~ Resolved: OUT OF
  SCOPE this slice (deferred follow-up). A generic impl's member body treats impl
  variables as opaque rigid variables with no declared bounds; `poly_trait_member_call`
  gates trait-member dispatch on a declared bound, so element-level dispatch is not
  writable this slice. Supervisor-confirmed 2026-08-26.

## Implementation

### Phase 1 — Generic `impl:` target parsing, dispatch, and monomorphization (R1, R2, R5, R6, R7, R10, R11)

Commit `1ccf3706`. Files: `src/ast.rs`, `src/parser.rs`, `src/check/poly.rs`,
`src/check.rs`, `src/check/declarations.rs`, `src/ir/driver.rs`,
`tests/phase7_slice4.rs`.

- `src/ast.rs` — `ImplDecl.target_ty: Type` replaced with `ImplTarget` (`pattern:
  PolyType` + `ty_var_names`/`len_var_names`); poly variant of
  `ground_member_type` binding the trait's self-var to the whole target `PolyType`.
- `src/parser.rs` — new `parse_impl_target` building a `PolyBuilder` and parsing a poly
  slot (admitting `'T`, forbidding bound syntax); `parse_impl_member_body` branches on
  concrete vs generic target (concrete keeps existing path; generic builds a `PolySig`
  and sets `poly: Some`); `synth_member_word_name` encodes the `PolyType` shape for
  distinct names.
- `src/check/poly.rs` — new `match_impl_target` one-way matcher (concrete `Type` vs
  `PolyType` pattern → `Option<Subst>`, modeled on `unify_poly_input` but keyed to the
  impl's own namespace); `resolve_user_bound` replaced exact-equality scan with
  candidate-set matching, minting `instantiation_symbol` for generic winners;
  `poly_admits` and `poly_sig_could_match` switched from exact equality to
  `match_impl_target`.
- `src/check.rs` — generic-impl dispatch monomorphs recorded via
  `discover_transitive_instantiations` so lowering emits member-word bodies.
- `src/check/declarations.rs` — duplicate check compares `target.pattern` (`PolyType`
  structural equality → alpha-equivalence); orphan rule already enforced via the
  existing `impl_target_module` `None` path.
- `tests/phase7_slice4.rs` — golden: generic `impl: Show for ['T 'N]` runs identically
  to `impl: Show for [i64 4]`; `impl: Show for 'T` parses and runs; overlapping-unequal
  targets accepted; alpha-equivalent targets are a duplicate error; generic impl in
  trait's module accepted; generic impl outside trait's module is an orphan error.

### Phase 2 — Specificity partial order and ambiguity error (R3, R8, R12)

Commit `08184072`. Files: `src/check/poly.rs`, `tests/phase7_slice4.rs`.

- `src/check/poly.rs` — `specificity` function implementing the equivalence-class
  refinement partial order; `resolve_user_bound` candidate selection replaced basic
  >1-match error with maximal-element dispatch and located ambiguity error for
  incomparable candidates.
- `tests/phase7_slice4.rs` — golden: concrete `[i64 4]` overrides generic `['T 'N]` at
  shared instantiation; incomparable `[i64 'N]` vs `['T 4]` at `[i64 4]` produces
  ambiguity error; shared-variable `[['T 'N] 'N]` beats `[['T 'N] 'M]` at `[[i64 4] 4]`.

### Phase 3 — Orphan rule diagnostic for generic targets (R4, R9, R13)

Commit `aa3ce403`. Files: `src/check/declarations.rs`, `tests/phase7_slice4.rs`.

- `src/check/declarations.rs` — `impl_orphan_error` rendering updated to display the
  `PolyType` target shape (e.g. `['T 'N]`) instead of a scalar `Type`.
- `tests/phase7_slice4.rs` — golden: two-module program with generic `impl:` in wrong
  module fails with orphan error naming the trait, the "declares no module of its own"
  explanation, and the target shape family.
