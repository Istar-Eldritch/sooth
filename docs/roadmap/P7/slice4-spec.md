# Spec: P7.S4 — Generic `impl:` targets with a specificity chain

**Status:** Draft  
**Created:** 2026-08-26  
**Discovery:** `docs/roadmap/P7/slice4-brief.md`

## Problem Statement

A trait's `impl:` target must today name exactly one concrete type: the whole-program
registry P7.S3e built keys on exact `(TraitId, Type)` equality (`resolve_user_bound`,
`poly.rs:5313`, scans for `i.trait_id == trait_id && i.target_ty == ty`), so a trait
conforming over a family of shapes (e.g. `Show`/`Eq` over every array shape `['T N]`)
needs one hand-written `impl:` block per shape, all with identical bodies. Authors of
`core` and stdlib-shaped families (the consumer P7.S3s first made real) pay an N-for-1
cost and a maintenance hazard: add a shape, forget an `impl:`, get a located
"unsatisfied bound" far from the cause. Sooth is unusually well placed to fix this —
dispatch is whole-program and monomorphizing with no cross-unit coherence question and
no runtime cost, and the language has neither lifetimes nor associated types (the two
soundness holes specialization opens elsewhere) — but the registry currently has no
notion of a *pattern* target, no way to choose among several matching ones, and no
orphan rule for a target that names no single struct/enum.

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
  (A doesn't relax B's concreteness); (2) B's equivalence classes refine A's — every B equivalence class is a subset of
  some A equivalence class (A is coarser/more-merged/more-constraining; B is
  finer/more-fragmented); and (3) A is strictly more constrained somewhere (A has
  `Concrete` where B has `Var`, or B has a finer partition (A has a coarser
  partition)). Concrete positions are singletons (each concrete position is its own
  equivalence class). This handles `Map['T 'T]` ≺ `Map['T 'U]` (B's partition {{0},{1}}
  refines A's {{0,1}} (A is coarser, linking the positions B keeps separate)),
  `Map[i64 'T]` ⊥ `Map['T 'T]` (incomparable because A is more constrained at
  position 0 (Concrete vs Var) but B is more constrained via sharing (positions
  linked vs independent) — neither pattern's constraints subsume the other's), and
  the existing examples (`[i64 N]` ≺ `['T N]`, `['T 4]` ≺ `['T N]`, `[i64 N]` ⊥
  `['T 4]`). Two candidates whose constraints neither subsume the
  other are incomparable; an unordered candidate set is a located error. No tiebreak
  rule (declaration order, arity, leftmost-concrete) is introduced.
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

- [ ] `impl: Show for ['T N]` (and `[i64 N]`, `['T 4]`) parses; `impl: Show for 'T`
  resolves the type variable instead of erroring "unknown type `'T`".
- [ ] A polymorphic word `shows ( &'T: Show -- )` called at `[i64 4]` dispatches to a
  single generic `impl: Show for ['T N]` and the program runs identically to one with a
  hand-written `impl: Show for [i64 4]`.
- [ ] A concrete `impl: Show for [i64 4]` overrides a generic `impl: Show for ['T N]`
  at `[i64 4]`; the generic covers every other array shape.
- [ ] Two incomparable matching targets (`[i64 N]` vs `['T 4]` at `[i64 4]`) produce a
  located error naming both targets and `[i64 4]`.
- [ ] A generic `impl:` declared outside the trait's module is rejected with a located
  orphan error; the same `impl:` in the trait's module is accepted.
- [ ] `examples/traits.sth`, `lib/cmp.sth`, and `core`'s `sort` compile and run
  unchanged; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
- [ ] Each new function (target parser, one-way matcher, specificity, generic-orphan)
  has unit tests beside it: a happy path plus at least one error/edge case.

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
  with no bound on the variable, `poly_trait_member_call` (`poly.rs:949`, the
  `for (v, bound) in &sig.bounds` loop) does not recognize the call as trait dispatch
  and it falls through to ordinary word lookup, which fails to resolve. This is the
  deferred follow-up slice (element-wise `Show`/`Eq` over a shape family via new
  impl-bound declaration grammar + recursive per-instantiation dispatch).
- `drop` as a trait (synthesized field-wise glue, never a writable default body; an
  owning closure's disposer keys on the construction site, **P7.S3v**).
- Trait objects / runtime dispatch (**P7.S3u**, parked); default member bodies and
  supertraits, still unforced; multi-type-variable traits.
- The REPL path — a REPL session declares no `impl:` (the parser's REPL/`parse` path is
  unchanged; `impl:` remains a build-time, whole-program-assembly feature, sequenced
  after **P7.S3s** which first gave the registry a real multi-`impl:` consumer in
  `core`).

## Solution Approach

The registry stops keying on exact `(TraitId, Type)` equality and instead stores each
`impl:` target as a `PolyType` pattern (the shape constructor family `ast.rs:1853`
already provides — `Var`, `Array`, `Generic`, `Ref`, `OwnedCell`, `Quotation` — plus
`Len`'s `Concrete`/`Var`). The `ImplDecl` gains an `ImplTarget` wrapper holding the
`PolyType` and the impl's own `ty_var_names`/`len_var_names`, since an impl target has
its own variable namespace (per-signature, exactly as `PolySig` does) for diagnostics
and for the member word's signature. `resolve_user_bound` then one-way-matches the
concrete instantiation `Type` against each candidate's pattern, collecting a `Subst`
per match, instead of the linear exact-equality scan. This matching is the reverse of
`apply_subst` (`poly.rs:5774`, which grounds a `PolyType` against a `Subst` to a
`Type`) and structurally a sibling of `unify_poly_input` (`poly.rs:5471`, which matches
a declared input `PolyType` against a concrete slot `Type` to extend a `Subst`) — but
keyed to the impl's own variable namespace rather than a callee `PolySig`, and producing
a whole `Subst` rather than extending one in place.

The deep consequence the brief leaves implicit — confirmed against the codebase and
pinned by supervisor decision — is that "a trait with one generic `impl:` behaves
identically to the hand-written concrete `impl:` blocks it replaces" forces the generic
impl's **synthesized member word to be polymorphic**. Today `parse_impl_member_body`
(`parser.rs:2597`) grounds the trait member's signature at a concrete `target: Type`
via `ground_member_type` (`ast.rs:1771`) and emits a monomorphic `WordDef { poly: None,
effect: <concrete> }`. For a generic target the trait's single self-variable
(`'T`, always id 0 in a member `PolySig`, per the `TraitDecl` doc at `ast.rs:1740`) must bind to the *whole*
target `PolyType`, yielding a `PolySig` over the impl's variables, and the word must
carry `poly: Some(...)`. It then flows through the existing machinery unchanged:
`check_poly_body` (the pre-pass at `check.rs:801`) walks its body once and records its
obligations/cross-calls; the call-site loop monomorphizes it per concrete instantiation;
`discover_transitive_instantiations` (`check.rs:1021`) composes its generic-calls-generic
cross-calls. The eager-expansion alternative (generate N concrete `ImplDecl`s from one
generic one) has a circular dependency — instantiations are discovered *during* `check`,
which already needs the impl registry — so reusing the existing monomorphization
pipeline is the architecturally forced path, not a choice.

Dispatch selection is the specificity partial order: pattern A is more specific than B
when B's equivalence classes refine A's (A is coarser/more-constraining) and A doesn't
relax B's concrete positions — an equivalence-class refinement relation, not a pointwise
field comparison (a shared variable in A linking positions B keeps separate makes A
more specific; a shared variable in B that A breaks apart would make A less specific).
The winning candidate is the unique maximal element; an unordered candidate set is a
located error naming every competing target and the concrete type. No tiebreak is
introduced — the user resolves ambiguity by writing the more specific `impl:` (e.g.
`[i64 4]`), exactly as the brief rules. The orphan rule for a generic target is the
one the brief proposes: a generic target names no single struct/enum, so the impl must
live in the trait's own module — the only module with a stake in the trait. The existing
orphan check already enforces this: `impl_target_module` returns `None` for a
non-struct/enum target, so the condition `impl_module != trait_decl_module &&
Some(impl_module) != target_module` reduces to `impl_module != trait_decl_module` — a
generic impl must live in the trait's module. Phase 3 updates only the diagnostic
rendering to name the `PolyType` target.

The slice is delivered in three phases. Phase 1 lays the spine — the data-model change,
the polymorphic member word, one-way matching, single-candidate dispatch, and the
monomorph recording that makes lowering emit a generic impl's member-word bodies — and
is independently shippable: a single generic `impl:` (no overlap) compiles, dispatches,
and runs identically to its hand-written concrete counterparts. Phase 2 adds the
specificity partial order and the ambiguity error (run in parallel with Phase 3).
Phase 3 renders the orphan error for generic targets (the rule is already
enforced in Phase 1). Phases 2 and 3 both depend only on
Phase 1 and are independent of each other.

## Codebase Map

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/ast.rs:1827` | `ImplDecl` struct (`target_ty: Type` at `:1829`) | Gains an `ImplTarget` (PolyType + `ty_var_names` + `len_var_names`) replacing `target_ty: Type` (R1) |
| `src/ast.rs:1853` | `PolyType` enum (`Var`, `Array`, `Generic`, `Ref`, `OwnedCell`, `Quotation`) | Reused as the target pattern type — already has every shape constructor S4 needs (R1) |
| `src/ast.rs:1844` | `Len` enum (`Concrete`/`Var`) | Reused for array-length patterns (R1) |
| `src/ast.rs:1740` | `TraitDecl` struct (doc: single self-variable `'T`, id 0 in each member `PolySig`) | The trait's `'T` binds to the whole target `PolyType` when grounding a member signature (R5) |
| `src/ast.rs:1771` | `ground_member_type(pty, target: Type) -> Type` | Add a poly variant grounding the trait's `Var(0)` to the whole target `PolyType`, returning a `PolyType` (R5) |
| `src/ast.rs:1962` | `Subst` struct (`ty: Vec<(u32,Type)>`, `len: Vec<(u32,u32)>`) | The matcher's output, in the impl's own variable space (R2) |
| `src/ast.rs:2065` | `instantiation_symbol(word, subst, generation)` | Mints the generic-impl dispatched symbol from the matched `Subst` (R6) |
| `src/parser.rs:2540` | `parse_impl_decl` (calls `parse_type_expr` at `:2544`) | Call a new `parse_impl_target` returning `ImplTarget` instead of `parse_type_expr` (R1) |
| `src/parser.rs:3682` | `parse_type_expr` (concrete; dispatches `[` to `parse_array_type_expr` at `:3825`) | The concrete reader — NOT reused for the target (rejects `'T` as "unknown type") |
| `src/parser.rs:2936` | `parse_poly_slot` (parses a poly slot with type vars via `parse_poly_ty_var`) | The reader to adapt for the target — admits `'T`; the target reader must forbid the bound syntax `parse_poly_ty_var` accepts (`:3161`) |
| `src/parser.rs:3161` | `parse_poly_ty_var` (parses `'T` **and** an optional `: Capabilities` bound) | Reference for variable parsing; the target reader must NOT admit the `:`-bound arm (bounds out of scope) |
| `src/parser.rs:1422` | `PolyBuilder` (`intern_ty_var`, `intern_len_var`, `finish`) | Reused to build the impl's variable namespace and the member word's `PolySig` (R1, R5) |
| `src/parser.rs:2597` | `parse_impl_member_body` (signature `target_ty: Type`; `ground` closure at `:2625`; `poly: None` at `:2648`) | Branch on concrete vs generic target: concrete keeps the existing path; generic builds a `PolySig` from the grounded member signature and sets `poly: Some` (R5) |
| `src/parser.rs:462` | `synth_member_word_name(member, trait, module, target: Type)` | Encode the target's `PolyType` shape (reuse `poly_type_str`, `poly.rs:6815`) so two generic impls for one trait get distinct synth names (R5/R6) |
| `src/check/declarations.rs:403` | `impl_target_module(ty: Type, module) -> Option<u32>` | Return `None` for a non-concrete `PolyType` target; the existing orphan arm already enforces the trait-module rule for generic targets (`Some(impl_module) != None` is always true, so the condition reduces to `impl_module != trait_decl_module`); Phase 3 updates only the diagnostic rendering (R4) |
| `src/check/declarations.rs:416` | `check_impl_decls` (exact duplicate scan at `:424` `*ty == imp.target_ty`; orphan check at `:444`) | Duplicate check compares the `ImplTarget`'s `PolyType` (structural equality → alpha-equivalence); orphan arm already enforces the generic-target rule from Phase 1 (R7, R4) |
| `src/check/poly.rs:5313` | `resolve_user_bound` (exact `.find(\|i\| i.trait_id == trait_id && i.target_ty == ty)` at `:5334`; symbol via `word_symbols.get(*idx)` at `:5356`) | Replace exact match with one-way matching + candidate set; mint `instantiation_symbol` for a generic winner; thread the monomorph recorder (R2, R3, R6) |
| `src/check/declarations.rs:1408` | `poly_admits` (`.any(|i| i.trait_id == ord && i.target_ty == *ty)`) | Overload-overlap: decides if a concrete type has an `impl:` for a trait (e.g. Ord); must switch from exact `target_ty` equality to `match_impl_target` (R10) |
| `src/check/poly.rs:4466` | `poly_sig_could_match` (`.any(|imp| imp.trait_id == ord && imp.target_ty == stack[base + i].ty)`) | Overload-overlap: decides if a stack type has an `impl:` for a trait; must switch from exact `target_ty` equality to `match_impl_target` (R10) |
| `src/check/poly.rs:5471` | `unify_poly_input(sig, pty, slot_ty, ..., subst, seeded)` | The structural model for the new one-way matcher — same shape recursion, but keyed to the impl's namespace and returning a fresh `Subst` (R2) |
| `src/check/poly.rs:5774` | `apply_subst(sig, pty, subst, ...) -> Type` | The reverse direction (PolyType→Type); reference only — S4 needs matching, not grounding |
| `src/check/poly.rs:949` | `for (v, bound) in &sig.bounds` (inside `poly_trait_member_call`) | Evidence that trait-member dispatch is gated on a declared bound — justifies bounds-on-impl-variables being out of scope |
| `src/check/poly.rs:4813` | `Bound::User(trait_id) =>` arm calling `resolve_user_bound` | The call site to thread the monomorph-recorder handle into `resolve_user_bound` (R6) |
| `src/check/poly.rs:6815` | `poly_type_str(pt: &PolyType, sig: &PolySig) -> String` | The `PolyType` renderer to reuse for the synth name and the ambiguity/orphan diagnostics (R5, R8) |
| `src/check.rs:572` | `check_module` (poly pre-pass `check_poly_body` at `:831`; call-site loop building `insts`; `discover_transitive_instantiations` at `:1021`; `module.instantiations = insts` at `:1023`) | Where the generic-impl member-word monomorphs are recorded so lowering emits them (R6) |
| `src/check.rs:1021` | `discover_transitive_instantiations(module, &mut insts, &symbols, &trait_obligations)` | The composition pass the member-word monomorphs reuse (their bodies' poly cross-calls are grounded here) (R6) |
| `src/ir/driver.rs:261` | the `distinct` monomorph emission loop (`module.instantiations.values().chain(&module.transitive_instantiations)`, dedup by `instantiation_symbol` at `:265`) | Consumes the member-word monomorphs — likely NO change if they surface through the existing tables (R6) |
| `src/ir/func_builder/calls.rs:278` | `self.trait_calls.get(&span)` → `lower_resolved_word_call(&sym_name)` | The call-site dispatch via the resolved symbol — NO change (the symbol is now a monomorph name for a generic winner) |
| `src/driver.rs:736-768` | whole-program `impls` assembly (`impls.extend(bodies.impls)`) | NO change — `ImplDecl` carries `ImplTarget` through assembly |
| `src/driver.rs:831` / `src/test_support.rs:86` | `check_impl_decls(&mut module)` pre-pass call sites | Run before `check_module`; the orphan rule is enforced here from Phase 1 (R4) |
| `src/check/declarations.rs:3618` | `impl_check_src` test helper (`parse` → `check_trait_decls` → `check_impl_decls`) | Pattern for the new parser/check unit tests (R11–R13) |
| `src/check/poly.rs:6900` | `checked_like_a_build` / `:6946` `SHOW` fixture / `:6935` `obligations_of` | Pattern for the dispatch/ambiguity golden + unit tests (R11, R12) |
| `examples/traits.sth` | the trait/`impl:` example | Regression corpus and the model for a new generic-`impl:` golden (R10, R11) |

## Open Questions

- [ ] **Monomorph recording vehicle.** The check pass must record each generic-impl
  dispatch as a `(member_word, impl_subst)` monomorph so lowering emits the member
  word's body. The natural homes are `module.instantiations` (keyed by call-site span)
  or `module.transitive_instantiations` (a `Vec<CallInst>` deduped by symbol, populated
  by `discover_transitive_instantiations`). Because a generic impl's member word is
  reached only through trait dispatch (never an ordinary call site), and its body's own
  generic-calls-generic cross-calls need the same composition, `transitive_instantiations`
  (or a sibling `Vec` merged at `check.rs:1021`) is the better fit — but the exact
  collection point (extend `discover_transitive_instantiations` vs a dedicated pass after
  the call-site loop) is an implementation decision for Phase 1 to settle. Flagged
  because it is the one integration point where the existing tables' shapes (span-keyed
  `instantiations`; symbol-deduped `transitive_instantiations`) do not obviously fit a
  dispatch-discovered monomorph.
- [x] ~~**Member-word model.** Polymorphic member words reusing the existing poly
  machinery, vs eager expansion into concrete `ImplDecl`s.~~ Resolved: polymorphic
  member words — eager expansion has a circular dependency (instantiations are
  discovered during `check`, which needs the impl registry). Supervisor-confirmed
  2026-08-26.
- [x] ~~**Bounds on the impl's own variables.** In or out of scope.~~ Resolved: OUT OF
  SCOPE this slice (deferred follow-up). A generic impl's member body treats impl
  variables as opaque rigid variables with no declared bounds; `poly_trait_member_call`
  (`poly.rs:949`) gates trait-member dispatch on a declared bound, so element-level
  dispatch is not writable this slice. Supervisor-confirmed 2026-08-26.

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| The member-word monomorph does not surface to lowering (link error / missing symbol) because the existing `instantiations`/`transitive_instantiations` shapes don't fit a dispatch-discovered monomorph | Med | Phase 1 pins the recording vehicle as its first sub-step (Open Question 1) and a golden test (R11) that links and runs is its exit gate; a missing symbol fails the golden loudly |
| `synth_member_word_name` collision: two generic impls for one trait (`['T N]` and `['T 4]`) synthesize member words with the same name | Med | Encode the target's `PolyType` shape in the synth name via `poly_type_str` (`poly.rs:6815`); Phase 2 adds a unit test that two coexisting generic impls for one trait produce distinct member-word names |
| Specificity partial order is subtle (equivalence-class refinement over shared variables, not just pointwise "more concrete fields"); an off-by-one makes a more-general target win silently | Med | Phase 2 implements `specificity` as a pure total function with unit tests for each sub-case including shared-variable scenarios (`[i64 N]`≺`['T N]`, `['T 4]`≺`['T N]`, `[i64 N]`⊥`['T 4]`, equal=not-subset, `Map['T 'T]`≺`Map['T 'U]`, `Map[i64 'T]`⊥`Map['T 'T]`, `[[i64 N] N]`≺`[[i64 N] M]`, `[['T N] N]`⊥`[['T 4] N]`) before the dispatch wiring; the ambiguity golden (R12) catches a silent wrong winner |
| A concrete `impl:` regresses because the `ImplTarget`/`PolyType` change ripples through `check_impl_decls`'s duplicate check and `resolve_user_bound`'s match | Low | Phase 1 keeps the concrete path byte-for-byte: a concrete target folds to `PolyType::Concrete(t)`, the matcher returns the exact-equal `Subst`, and the bare `word_symbols[idx]` path is unchanged; the corpus regression (R10) is the gate |
| The polymorphic member word's body, walked by `check_poly_body`, exercises a poly-path corner the existing member words (always `poly: None`) never hit (e.g. self-recursion, cross-calls) | Med | `rewrite_member_self_calls` (`parser.rs:475`) already rewrites member self-calls to the synth name; Phase 1 golden (R11) uses a non-recursive body first; recursion is an explicit later test, not the gate |

## Delivery Plan

### Phase 1: Generic `impl:` target parses, dispatches, and runs (single-match)

- **Goal**: A single generic `impl:` (e.g. `impl: Show for ['T N]`) parses, a
  `Bound::User` call site at a concrete instantiation (e.g. `[i64 4]`) dispatches to it,
  and the program compiles, links, and runs identically to one with a hand-written
  `impl: Show for [i64 4]`. (Zero or one matching candidate only; >1 is a basic error
  refined in Phase 2.)
- **Requirements Covered**: R1, R2, R5, R6, R7, R10, R11
- **Scope**:
  - Modify `src/ast.rs:1827` (`ImplDecl`): replace `target_ty: Type` (`:1829`) with an
    `ImplTarget` carrying `pattern: PolyType`, `ty_var_names: Vec<String>`,
    `len_var_names: Vec<String>`. Update every field access site.
  - Add to `src/ast.rs` near `ground_member_type` (`:1771`) a poly variant that grounds
    the trait member's `PolyType` signature by binding `PolyType::Var(0)` (the trait's
    self-var) to the whole target `PolyType`, recursing over
    `Array`/`Ref`/`Generic`/`OwnedCell`/`Quotation` (matching the matcher's coverage),
    returning a `PolyType` (model on `ground_member_type`'s `Var(_) => target` arm).
  - Modify `src/parser.rs:2540` (`parse_impl_decl`): replace `parse_type_expr()` at
    `:2544` with a new `parse_impl_target` that builds a `PolyBuilder`, parses one poly
    slot via the `parse_poly_slot` (`:2936`) machinery, **forbids** the bound syntax
    (`parse_poly_ty_var`'s `:`-bound arm at `:3161`) and row variables, and returns an
    `ImplTarget`. A concrete target folds to `PolyType::Concrete(t)`.
  - Modify `src/parser.rs:2597` (`parse_impl_member_body`): change the `target_ty: Type`
    parameter to `&ImplTarget`; for a concrete target (`PolyType::Concrete`) keep the
    existing `ground` closure (`:2625`) + `poly: None` (`:2648`) path; for a generic
    target, build the member word's `PolySig` from the trait member's signature grounded
    via the new poly grounding (inputs/outputs as `PolyType`, `ty_var_names`/`len_var_names`
    from the `ImplTarget`, `bounds: []`) and set `poly: Some(...)`.
  - Modify `src/parser.rs:462` (`synth_member_word_name`): for a generic target, encode
    the `PolyType` shape via `poly_type_str` (`poly.rs:6815`) against the impl's `PolySig`
    so two generic impls for one trait produce distinct names; for a concrete target keep
    `target.name()`.
  - Add to `src/check/poly.rs` a one-way matcher `match_impl_target(pattern: &PolyType,
    ty: Type, arrays, refs, ...) -> Option<Subst>`, modeled on `unify_poly_input`
    (`:5471`) but keyed to the impl's variable namespace (no `PolySig`/`seeded`), returning
    a fresh `Subst` (binding `Var`→`Type`, `Len::Var`→`u32`, recursing over `Array`/
    `Generic`/`Ref`/`OwnedCell`/`Quotation` (a `Quotation` target matches a concrete
    quotation slot row-pointwise, as `unify_poly_input`'s `Quotation` arm does —
    concrete quotations are already valid impl targets today); `PolyType::Concrete(t)`
    matches only on `t == ty`). A
    `Var` or `Len::Var` already bound in a prior position must match consistently — if
    the subst already maps the variable to a different value, the match fails (modeled on
    `unify_poly_input`'s consistency check at `poly.rs:5493`).
  - Modify `src/check/poly.rs:5313` (`resolve_user_bound`): replace the exact
    `.find(|i| i.trait_id == trait_id && i.target_ty == ty)` (`:5334`) with a scan that
    runs `match_impl_target` over each `impl:` and collects `(impl, subst)` candidates.
    Exactly one match → dispatch. Zero → the existing `unsatisfied_user_bound_error`
    (`:5379`) unchanged. **More than one** → a basic "multiple `impl:` targets match"
    error (Phase 2 replaces this with specificity/ambiguity). For a generic winner (target
    not `PolyType::Concrete`), mint `instantiation_symbol(word_symbols[idx], &subst)`
    (`ast.rs:2065`) as the dispatched symbol in place of the bare `word_symbols[idx]`
    (`:5356`); a concrete winner keeps the bare path.
  - Modify `src/check/declarations.rs:1408` (`poly_admits`) and
    `src/check/poly.rs:4466` (`poly_sig_could_match`): both use exact
    `i.target_ty == *ty` / `i.target_ty == stack[...]` equality to decide whether a
    concrete type has an `impl:` for a trait (e.g. Ord) for overload-overlap resolution.
    Switch both from exact equality to `match_impl_target`; for concrete-only impls the
    matcher's `Concrete(t)` arm returns a match iff `t == ty` — identical to current
    behavior (R10 parity). A generic `impl: Ord for ['T N]` now correctly satisfies Ord
    for `[i64 4]` via the matcher, a behavior change (previously missed).
  - Modify `src/check.rs:572` (`check_module`): record each generic-impl dispatch's
    `(member_word, impl_subst)` as a `CallInst` so lowering emits the member-word body.
    Settle Open Question 1 here first — extend `discover_transitive_instantiations`
    (`:1021`) or a sibling pass to seed these monomorphs (their bodies' poly cross-calls
    get composed by the existing fixpoint), deduped by symbol into
    `module.transitive_instantiations`. Thread the recorder handle into
    `resolve_user_bound` from the `Bound::User` arm (`poly.rs:4813`).
  - Modify `src/check/declarations.rs:416` (`check_impl_decls`): duplicate check compares
    `target.pattern` (the `PolyType`) via structural `PartialEq`, NOT the whole
    `ImplTarget` — comparing `ty_var_names`/`len_var_names` would make `['T N]` and
    `['U M]` unequal (different name strings) and break alpha-equivalence; do not derive
    `PartialEq` on `ImplTarget` for the duplicate check. `['T N]` and `['U M]` both fold
    to `Array(Var(0),Var(0))` and compare equal. `impl_target_module` (`:403`) returns
    `None` for a non-concrete target so the existing orphan arm already enforces the
    orphan rule for generic targets via the existing None path (`Some(impl_module) != None`
    is always true, so the condition reduces to `impl_module != trait_decl_module`);
    Phase 3 updates only the diagnostic rendering. Update the duplicate-error rendering
    (`duplicate_impl_error`, `:493`) to render the `PolyType`.
  - Files to create: none (new functions live in `ast.rs` / `check/poly.rs` per CLAUDE.md's
    "start in one file, split only under pressure").
  - **Out of scope for this phase**: the specificity partial order and the ambiguity
    error (Phase 2) — `resolve_user_bound` errors on >1 match with a basic diagnostic;
    the orphan diagnostic rendering for generic targets (Phase 3); lowering's
    `ir/driver.rs:261` and `ir/func_builder/calls.rs:278` (no change expected — verify the
    monomorph surfaces through the existing tables); bounds on impl variables (slice
    out-of-scope).
- **Entry Conditions**: P7.S3s done (the multi-`impl:` consumer in `core` exists); the
  supervisor-confirmed scope decision (bounds out of scope; polymorphic member words in
  scope). No prior phase of this slice.
- **Exit Criteria / Verifiable Artifacts**:
  - [ ] `impl: Show for ['T N]` and `impl: Show for 'T` parse without "unknown type `'T`"
    (unit test beside `parse_impl_decl`).
  - [ ] `match_impl_target` unit tests: `['T N]` matches `[i64 4]` →
    `Subst{ty:[(0,i64)], len:[(0,4)]}`; `[i64 N]` matches `[i64 4]` not `[u32 4]`;
    `['T 4]` matches `[i64 4]` not `[i64 2]`; `Point` matches only `Point`.
  - [ ] A golden test (R11): a program with `impl: Show for ['T N]` and a `shows` call at
    `[i64 4]` compiles, links, runs, and produces output identical to the same program
    with `impl: Show for [i64 4]`.
  - [ ] `examples/traits.sth`, `lib/cmp.sth`, and `core`'s `sort` compile and run
    unchanged; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
  - [ ] `check_impl_decls` unit test: two `impl:` with alpha-equivalent generic targets
    (`['T N]` and `['U M]`) are a duplicate error; two with unequal overlapping targets
    (`['T N]` and `['T 4]`) are accepted as declarations.
  - [ ] A generic `impl:` declared outside the trait's module is rejected in Phase 1
    already (the existing orphan arm rejects it via the `None` path), not deferred to
    Phase 3.
- **Parallelism**: SEQUENTIAL — this is the spine; Phases 2 and 3 build on its
  `ImplTarget`, `match_impl_target`, and the `resolve_user_bound` candidate-set shape.
- **Relative Effort**: L — multi-file AST change rippling through parser → check →
  lowering-consumption, plus the polymorphic-member-word integration with the existing
  poly machinery and the monomorph-recording wiring (the one genuine integration point).
- **Difficulty**: hard — cross-cutting data-model change touching shared dispatch control
  flow (`resolve_user_bound` is every trait-dispatch call site's resolution path) plus
  the polymorphic-member-word integration with the poly check/lower pipeline; the
  monomorph-recording vehicle (Open Question 1) is an ambiguous integration point.
- **Open Questions / Blockers**: Open Question 1 (monomorph recording vehicle) must be
  settled as the first sub-step; if the existing tables cannot fit a dispatch-discovered
  monomorph cleanly, a sibling `Vec<CallInst>` merged into lowering's emission set is the
  fallback.

### Phase 2: Specificity partial order and the ambiguity error

- **Goal**: Where more than one `impl:` target matches a concrete type, the most
  specific wins; two incomparable matching targets produce a located error naming every
  competing target and the concrete type.
- **Requirements Covered**: R3, R8, R12
- **Scope**:
  - Add to `src/check/poly.rs` a total function `specificity(a: &PolyType, b: &PolyType)
    -> Ordering` (or `is_strictly_more_specific(a, b) -> bool`): the equivalence-class
    refinement relation. The function builds equivalence classes from shared variables
    (both type vars and length vars — positions sharing the same variable form one
    class, concrete positions are singletons), checks that B's classes refine A's
    (every B class is a subset of some A class; A is coarser/more-constraining; B is
    finer), verifies A doesn't relax B's concrete positions, and confirms A is strictly
    more constrained somewhere (concrete where B is variable, or B has a finer
    partition (A is coarser)). Type-variable and length-variable equivalence classes
    are separate namespaces — type var id 0 and length var id 0 are distinct and must
    not be merged into one partition (the `Subst` already separates them via
    `ty`/`len` maps). It recurses over `Array`/`Generic`/`Ref`/`OwnedCell`
    sub-positions and `Len::Concrete` vs `Len::Var`. Equal patterns are **not** strictly
    more specific (handled by the duplicate check, not here).
  - Modify `src/check/poly.rs:5313` (`resolve_user_bound`): replace Phase 1's basic
    >1-match error with — compute the candidate set's maximal element under
    `specificity`; a unique maximal → dispatch it; **no** unique maximal (two or more
    incomparable maxima) → a located ambiguity error naming the trait, every competing
    target (rendered via `poly_type_str`, `:6815`), and the concrete `ty`, at the dispatch
    `span`. Keep Phase 1's single-match and zero-match paths unchanged.
  - Files to create: none.
  - **Out of scope for this phase**: the orphan diagnostic rendering (Phase 3); any
    declaration-time change to `check_impl_decls` (overlapping impls are legal
    declarations, R7 — already accepted by Phase 1's duplicate check); the matcher
    itself (Phase 1).
- **Entry Conditions**: Phase 1 complete — `match_impl_target` produces the candidate
  set; `resolve_user_bound` already collects `(impl, subst)` candidates; `poly_type_str`
  renders targets.
- **Exit Criteria / Verifiable Artifacts**:
  - [ ] `specificity` unit tests: `[i64 N]`≺`['T N]`, `['T 4]`≺`['T N]`, `[i64 N]`⊥
    `['T 4]` (neither more specific), equal-not-strict, and a nested case
    (`Box[i64]`≺`Box['T]`).
  - [ ] `specificity` unit tests for shared-variable scenarios: `Map['T 'T]`≺
    `Map['T 'U]` (B's partition {{0},{1}} refines A's {{0,1}}; A is coarser),
    `Map[i64 'T]`⊥`Map['T 'T]` (A more constrained at position 0 via Concrete, B more
    constrained via sharing — incomparable), `[[i64 N] N]`≺`[[i64 N] M]` (inner length
    = outer length, linked vs separate — the linked version is
    coarser/more-constraining, so more specific), and `[['T N] N]`⊥`[['T 4] N]`
    (length variable N shared in A vs concrete in B; concrete inner length in B vs
    variable in A — incomparable).
  - [ ] A golden test (R12): a concrete `impl: Show for [i64 4]` overrides a generic
    `impl: Show for ['T N]` at `[i64 4]`; the generic covers `[i64 2]`.
  - [ ] A golden test (R12): `impl: Show for [i64 N]` and `impl: Show for ['T 4]`
    together, called at `[i64 4]`, produce the located ambiguity error naming both
    targets and `[i64 4]`.
  - [ ] A golden test (R12): `Map['T 'T]` and `Map['T 'U]` both match `Map[i64 i64]` and
    the more specific `Map['T 'T]` wins (its partition forces both arguments equal,
    which is the more constrained match).
  - [ ] `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green; corpus
    unchanged.
- **Parallelism**: PARALLEL with Phase 3 — both depend only on Phase 1; Phase 2 touches
  `resolve_user_bound`'s selection, Phase 3 touches `check_impl_decls`'s orphan arm;
  they share no file's control flow.
- **Relative Effort**: M — one new pure function plus the selection policy and the
  ambiguity diagnostic in `resolve_user_bound`, with thorough unit/golden coverage.
- **Difficulty**: standard — a new localized algorithm feeding an existing call; the
  partial order is subtle (equivalence-class refinement over shared variables, not
  field-count) but pure and unit-testable in isolation before wiring.
- **Open Questions / Blockers**: None identified. (Phase 1's `resolve_user_bound`
  candidate-set shape is the contract this phase builds on.)

### Phase 3: Orphan rule for generic targets

- **Goal**: A generic `impl:` (a target whose `PolyType` is not a single concrete
  `Type::Struct`/`Type::Enum`) declared outside the trait's own module is rejected with a
  located orphan error; the same `impl:` in the trait's module is accepted. Concrete
  targets keep the existing rule.
- **Requirements Covered**: R4, R9, R13
- **Scope**:
  - Modify `src/check/declarations.rs:508` (`impl_orphan_error`): the orphan rule logic
    is already correct from Phase 1 — `impl_target_module` returns `None` for a generic
    target, so the existing `impl_module != trait_decl_module && Some(impl_module) !=
    target_module` check already rejects a generic impl outside the trait's module.
    Phase 3 is diagnostic-rendering-only: update `impl_orphan_error` to render a
    `PolyType` target (via `poly_type_str`, `poly.rs:6815`) instead of a `Type` target,
    so the error message names the shape family (e.g. `['T N]`) rather than a scalar.
    The "declares no module of its own" branch already reads correctly for a generic
    target.
  - Files to create: none.
  - **Out of scope for this phase**: dispatch, matching, specificity (Phases 1–2); any
    change to `resolve_user_bound`.
- **Entry Conditions**: Phase 1 complete — `ImplTarget` exists and `impl_target_module`
  already returns `None` for a generic target (so the orphan rule is already enforced);
  the `ImplDecl` carries the trait id and module for the rule.
- **Exit Criteria / Verifiable Artifacts**:
  - [ ] A unit test (beside `check_impl_decls`, pattern on `impl_check_src`,
    `declarations.rs:3618`): a generic `impl: Show for ['T N]` declared in a module that
    is not `Show`'s module is rejected with the located orphan error.
  - [ ] A unit test: the same generic `impl:` in `Show`'s module is accepted.
  - [ ] A unit test: a concrete `impl: Show for Point` keeps the existing rule
    (accepted in `Point`'s module or `Show`'s module; rejected elsewhere) — no
    regression.
  - [ ] A golden test (R13): a two-module program where a generic `impl:` sits in the
    wrong module fails to compile with the located orphan error.
  - [ ] `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green; corpus
    unchanged.
- **Parallelism**: PARALLEL with Phase 2 — both depend only on Phase 1; Phase 3 touches
  only `check_impl_decls`'s orphan arm, which Phase 2 does not touch.
- **Relative Effort**: S — one diagnostic-rendering update and tests; localized to
  `declarations.rs`.
- **Difficulty**: standard — a small, localized declaration check; no shared dispatch
  control flow, no concurrency or migration.
- **Open Questions / Blockers**: None identified.

### Parallelism Summary

- Phase 1 is sequential (the spine).
- Phase 2 and Phase 3 both run after Phase 1 and are independent of each other
  (Phase 2 = `resolve_user_bound` selection; Phase 3 = `check_impl_decls` orphan arm;
  disjoint files/control flow). They may run concurrently.

### Effort Summary

- Phase 1: L
- Phase 2: M
- Phase 3: S
- Total: one L + one M + one S; the L is the architectural spine, the M and S run in
  parallel after it.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "generic impl target parses, dispatches, runs (single-match)", "effort": "L", "difficulty": "hard" },
    { "phase": 2, "focus": "specificity partial order and ambiguity error", "effort": "M", "difficulty": "standard" },
    { "phase": 3, "focus": "orphan rule for generic impl targets", "effort": "S", "difficulty": "standard" }
  ]
}
```
