# Spec: P7.S3e user-declarable trait bounds

**Status:** Draft
**Created:** 2025-07-22
**Discovery:** /root/code/ordfruma/sooth/docs/roadmap/P7/slice3e-brief.md

## Problem Statement

Polymorphic words today can only state bounds `Copy` (via `poly_is_copy`) or `Ord` (via `is_ord`, which is a hardcoded numeric predicate). These primitives cannot express user-defined relationships between types for collection keys or sorting behavior (e.g., a user `Order` trait for string comparison). The `Bound` variant set is closed and hardcoded; extending it requires either a new parser branch per predefined trait or a general trait table. A user-declarable trait system must introduce: (1) a trait‑decl grammar that lists required member signatures over a single type variable, (2) a bound syntax on that variable (e.g., `'T: Show`), (3) nominal satisfaction via explicit `impl:` blocks with an orphan rule, (4) a private implementation registry and lookup, (5) body-side dispatch of bounded method calls (`Show.show`) to deliver intensional polymorphism before monomorphization, (6) call-site bound-satisfaction checking to verify a concrete type actually has the required member, and (7) integration with the existing instantiation symbol table such that each monomorph of a polymorphic word gets its own bounded method symbol rather than a shared one.

Failure to ship a first consumer that compiles means all `Map`/`Vec`-driven sketches and the postponement strategy in the brief's "Ready to spec?" remain hypothetical—the bounds feature has no testable shape exposed to the ecosystem. The cost is widening `Sort.sort( [ 'T: Copy Order N ] -- [ 'T N ] )` from a typeable stub to a green consumer that emits working QBE.

## Requirements

- **R1.** The parser must accept a top-level `trait: TraitName` declaration with a list of one or more required member signatures. Each member signature must be of the form `( &'T ... -- ... )` where `'T` refers to the unique type variable scoped to the trait (single-type-variable traits only). A trait declaration is module-scoped and exportable, like `type:`/`extern:` declarations.

- **R2.** The parser must expand the two hardcoded keywords `Copy` and `Ord` in `parse_capabilities` into a single lookup against a pre-seeded trait table (`Copy` and `Ord` as predicate‑kind entries). Any attempt to declare a user `trait: Copy` or `trait: Ord` must fail with an ordinary duplicate‑declaration error matching the existing word/type/static error message style, not a reserved-word check. A user trait name colliding with a prelude trait remains shadowed at parse time (user declaration is rejected before the owned-validation pass).

- **R3.** Each `impl: Trait for Type ; ... ;` block must be syntactically validated at the implementation site, named the trait, the target type, the implementing module (or the trait's module, or the type's module), and the orphan violation (if the block belongs to neither). The block's body must be semantically checked against the trait's declared member signatures under the resolved type. The resulting satisfaction record must be stored in a private registry keyed by `(TraitId, Type)`. No two `impl:` blocks for the same `(TraitId, Type)` pair may exist, and a colliding block must receive the same duplicate‑declaration style message.

- **R4.** Bounds on type variables must follow the existing grammar: a variable's bound list appears on the first occurrence of that variable in the signature (e.g., `( &'T: Show &'T -- )`, not `( 'T: Show &'T &'T -- )`). The bound syntax is a simple capability list attached to the variable's first slot: `'T: Capability1 Capability2 ...`. The capability list is greedy over recognized trait names and accepts only those traits from the traversal order trait table → module‑scoped exports.

- **R5.** A bound constraint must be desugared in the parser into a `Vec<(u32, Bound)>` entry on the `PolySig.bounds` field, where the `u32` is the variable's index into `PolySig.ty_var_names`. Existing `Bound::Copy` and `Bound::Ord` variants must keep their existing meaning (calls to `is_copy`/`is_ord` on the concrete type). At this stage, the only `Bound` variants are `{Copy, Ord, User(trait_id)}`; no other bound kinds exist yet.

- **R6.** During body checking (`poly_term` in `src/check/poly.rs`), when a term calls a word on the stack and the top input slot is `PolyType::Ref(PolyType::Var(v), mutable)`, the checker must check whether the variable `v` has any `Bound::User(trait_id)` in its bound set. If so, it must lookup whether that `trait_id` requires a member with the called name, and if so, compose the required signature in the same shape a concrete `Overload` would produce. The output types are the trait's declared abstract outputs, not the symbol table entries (concrete resolution cannot run yet). This satisfying branch must fire before the existing env‑lookup fallthrough that rejects a bare `Var` in a concrete argument, preserving the existing `poly_var_to_concrete_error` barrier.

- **R7.** During call-site checking (`check_poly_call`'s R6 loop), when a polymorphic call is evaluated over an instantiated substitution `θ`, the checker must verify that each bound on the instantiated types holds. For `Bound::Copy`/`Bound::Ord`, reuse `poly_is_copy(..., sig)` and P8.S2's `is_ord` predicate. For a `Bound::User(trait_id)`, the checker must lookup the trait's satisfaction record for the concrete type `θ(T)` (where `T` is the instantiated type) in the private implementation registry. Only when the concrete type is satisfied may the call site proceed; otherwise, it must emit a located error naming the trait, the missing member signature, and the concrete type (e.g., `` `i64` does not satisfy `Show`: no `( &i64 -- )` found ``).

- **R8.** Member signatures within a bound list across all traits that constrain a single variable must be pairwise name-conflict-free. If two traits in a variable's bound set both require a member named `eq`, and at least one of them is a user trait (`Bound::User`), the checker must reject the declaration at the bound declaration site (the `traits` table or the module's exported bounds) with a message naming both traits, the colliding member name, and the bound's location (the signature's span), rather than leaving the conflict unresolved.

- **R9.** Bounded trait member calls must not increase the IR lowering cost beyond the existing instantiation overhead. The `Module::builtin_overloads: HashMap<Span, String>` record must be keyed by instantiation- and symbol-specific keys, ensuring each monomorph of a polymorphic word that bounds a user trait gets its own per‑instantiation record for that bounded member. This may require surfacing the instantiation symbol into the call site during body checking and using it as part of the lookup key, or lifting the lowering runtime symbol resolution into a separate per‑instantiation lookup (still keeping the invariant that lowering never re‑runs resolution, only reads an existing record).

- **R10.** All stage functions (`lex`, `parse`, `check`, `lower`) must have accompanying unit tests: the happy path for each new parse construct/grammar rule, the happy path for each new check path (bound-satisfaction, trait-member dispatch), and at least one error/edge case per branch (e.g., duplicate impl, multi-bound member name collision, user trait named `Copy`/`Ord`, missing member in bound satisfaction, body-side dispatch of a bounded method with `&'T` args).

- **R11.** Every roadmap phase exit criterion must be a golden test (source input file → expected output or diagnostic, both read from source in `tests/phase7_slice3e.rs`). Golden tests must assert the exact wording of diagnostics, not just their presence, to validate the "sharp error" behavior goals of this project.

- **R12.** The feature must be twinned against the array `sort` consumer from the dogfood (Program 2 in `slice3-dogfood.md`). The golden test accepts a trait declaration (e.g., `trait: Order 'T cmp ( &'T &'T -- Ordering ) ;`), an array `sort` word with `'[ 'T: Copy Order 'N ] -- [ 'T 'N ]'`, two `impl:` blocks for that trait on concrete types (e.g., `i64` and `f64`), and an array literal of those concrete elements. The test verifies that the program builds and runs correctly with an in-place insertion sort comparing values via the bounded `cmp` method.

- **R14.** A bound-satisfying trait member that is itself a polymorphic word must be rejected at the call site inside a bounded poly body -- restricted to a leaf (monomorphic) member, not propagated. This is not new machinery: `poly_calls_poly_word_error` (P8.S2's R6b, `src/check/poly.rs:1533-1546`) already rejects a poly word calling another poly word, same-module or cross-module, with the identical wording regardless of caller; a bound-satisfying member dispatched through R6 (`src/check/poly.rs:3406-3419`) that resolves to a poly word inherits this existing rejection for free. The spec must state this explicitly and add a golden proving it (a trait member implemented as a poly word, called from a bounded poly body, produces exactly R6b's message) rather than leaving it to be rediscovered as a bug during Phase 2/3 implementation.

- **R13.** Multi‑type‑variable and compiler‑known third‑trait-kind requirements (e.g., `bool`‑shaped or `Fallible`‑returning traits) are explicitly out of scope for this slice. The design focuses on single‑variable user traits and predicate‑kind traits supplied by the language (the existing `Copy`/`Ord`), leaving more general bound kinds for follow‑up work that has no forcing consumer yet.

## Success Criteria

- [ ] The parser accepts `trait: Show 'T show ( &'T -- ) ;` definitions and reports them as module-level declarations comparable to `type:`/`extern:` declarations.
- [ ] The parser rejects a second `trait: Show 'T show ( &'T -- ) ;` in the same module with a duplicate‑declaration error message.
- [ ] The parser accepts ``: foo ( 'T: Show -- ) a show ;` and body‑checks successfully against a user `Show` trait.
- [ ] The checker validates an `impl: Show for i64 ; : show ( &i64 -- ) "." ; ;` block against the trait and registers it, with correct orphan rule violation messages when the impl lives outside the trait's or the type's module.
- [ ] A polymorphic word with `': bar ( 'T: Show -- ) vec a show ;` fails bound‑satisfaction at a call site passing a concrete type that lacks a `show` member, emitting a diagnostic like `` `String` does not satisfy `Show`: no `( &String -- )` found (line 12, col 3) ``.
- [ ] A polymorphic word with bounds `'T: Show Eq` fails to parse if both `Show` and `Eq` require a member named `eq`, emitting a multi‑bound collision diagnostic naming both traits and the method.
- [ ] A polymorphic word with `': sort ( [ 'T: Copy Order 'N ] -- [ 'T 'N ] )` and a concrete integer array demonstrates successful bound composition and trait‑member dispatch via an `Order.cmp` call inside the insertion‑sort body.
- [ ] Unit tests exist for every new stage function invocation (`parse_trait_decl`, `check_impl_block`, `trait_satisfied_by`, `check_poly_bound_satisfaction`, `poly_trait_member_dispatch`) and include both happy and error cases.
- [ ] Golden tests for the array `sort` consumer in `tests/phase7_slice3e.rs` pass (`cargo test`).
- [ ] A bound-satisfying trait member implemented as a polymorphic word, called from inside a bounded poly body, fails with exactly `poly_calls_poly_word_error`'s existing wording -- no new nested-dispatch diagnostic is introduced for this case.
- [ ] No `Map` or generic‑instantiation over a poly word's own variable (e.g., `Vec2`, `Box['T]`) appears in the slice's scope; those remain blocked by other phases.

## Scope & Boundaries

**In scope:**

- Parser addition for the `trait:` declaration form and `impl:` implementation blocks.
- New AST types: `TraitDecl` with fields `name, trait_id, members: Vec<TraitMember>`, `TraitMember` with `name, sig: Sig`, `ImplDecl` with fields `trait_id, target_type, module, body: Vec<Term>`.
- Private runtime registry throughout checking: a `HashMap<(TraitId, Type), ImplDecl>` stored alongside the `Module` struct (module‑scoped, not exported or used for symbol resolution directly; only used by the checker).
- Extending `Bound` to include `User { trait_id }` variants and seeding `Copy`/`Ord` as predicate‑kind entries.
- The bounded method dispatch path in `poly_term` that handles `&'T` bounded vars before the env‑lookup fallthrough.
- The bounded bound‑satisfaction loop in `check_poly_call` that resolves concrete satisfaction against the private registry.
- Multi‑bound member name conflict detection across all traits referencing a single type variable.
- Body‑side trait member resolution and per‑instantiation overload record generation for the consumer (`sort`).
- Unit and golden tests for all new constructs, covered by the existing `#[cfg(test)]` module convention.

**Out of scope:**

- Trait objects or runtime dispatch (`dyn Show`, `^Any`; verbatim Soth principle of compile‑time only).
- Associated types, default method bodies, blanket impls, supertraits, generic constants.
- Multi‑type‑variable traits (e.g., `Zip` or `Coerce` constraints over two type arguments).
- The prelude `Fallible` trait or other compiler‑known third‑trait‑kind mechanisms (bool‑shaped/Option/Result signatures, as purely speculative).
- Changing core layout, lowering of user trait types beyond symbol‑record persistence, or adding a new `CallInst` variant unless directly required to emit distinct symbols per instantiated poly word.
- Maps or any generic data structure using this feature beyond the array `sort` golden; treat those as future work (deferred to P9.S1 and P7.S3a).
- Re‑use of the existing `Module::instantiations` table (instantiation key is `Span`, not instantiation‑symbol‑aware unless needed); the spec does not mandate using it as a key for overloads at this slice.

## Solution Approach

This slice introduces three orthogonal layers: surface syntax (`trait:`/`impl:`), compile‑time polymorphism enforcement (bounds checking and method dispatch), and lowering integration (per‑instantiation symbol records). The recommended approach is to build these in order: first the parser and AST extensions, then the body‑side checker that stitches trait members into the existing `poly_term` flow, then the call‑site bound checker that validates a concrete type against a user trait, and finally the consumer golden test and lowering preparation to confirm the instantiation overhead does not exceed a small fraction of the existing IR.

**Ast layer:** Extend `src/ast.rs` to add `TraitDecl` and `ImplDecl` structs that mirror the structure of `StructDecl`/`EnumDecl` (both module‑scoped, with `module` and `span` fields). These types are owned by the module and only visible to the checker, not exposed to the resolver or lowering directly (the resolver sees type names, not trait bodies). Add `TraitMember` as a lightweight description of a required method: `{ name: String, sig: Sig }`. Optionally expose `TraitTable` as a public thin wrapper, but the implementation registry can be a private module‑level `HashMap`.

**Parser layer:** In `src/parser.rs`, before parsing top‑level words, add a `parse_trait_decl` function that recognizes the `trait:` keyword, consumes the trait name, expects exactly one type variable binding (syntactically `'T` in a single bracket position), then parses a semicolon‑terminated list of member definitions. Each member is a parenthesized `( ... -- ... )` effect under the same `'T`. The `parse_impl_brace` function recognizes `impl:` and validates that the required fields (trait name, target type declaration, braces and body) are present and parse correctly. Both may reuse the existing `parse_effect` scaffolding and line‑number tracking.

**Check layer:** The checker must validate each `trait:` declaration independently from any implementation blocks. Register the trait in the module's `traits` table (keyed by `trait_id: usize`, unique per module) and store the member list for later lookup. For each `impl:` block, mark the target type (a concrete `Type` from the type table) and the trait ID. Store the satisfaction record in a new private field `Module::impl_satisfactions: HashMap<(TraitId, Type), ImplDecl>`. During body checking, extend `poly_term`'s fallthrough handling (currently a direct env lookup after `&`‑shuffles and before `len`, etc.) with a new branch that matches `PolyType::Ref(PolyType::Var(v), mutable)` in a slot that is about to be treated as a ground operand to another word. This branch threads the variable's bound list and looks up which trait(s) provide the called name, composing the expected signature in place of the registry lookup (the function must not perform concrete symbol resolution yet—only forward the trait's abstract signature).

**Call‑site bound layer:** The R6 loop in `check_poly_call` already checks `PolyType::Var` for `Copy`/`Ord`. Extend this loop to also check `Bound::User(trait_id)`'s satisfaction against a concrete type. The concrete type is obtained by applying the ground substitution `subst` to the variable: `subst.ty_of(v).expect(...)` (checked before the loop). The satisfaction lookup uses the private `impl_satisfactions` table. If missing or the concrete type asserts it cannot satisfy the trait, emit a unique diagnostic that names the trait, the missing member signature in full (as a visible skeleton like `( &'T -- )`), and the concrete type with location (the source span of the bound declaration, available from the `PolySig` attached to the word). Provide full tracing: the checker can walk the module's trait list to cite the trait's name and member signatures at the call site to improve the diagnostic.

**Member‑name uniqueness layer:** When registering a bound in `PolySig` or when checking a new bound declaration for conflict, iterate over all traits that have contributed to the variable's bound list and collect the set of required members per trait. If two traits contribute the same member name, reject the declaration (or detected bound) at the earliest point detecting the conflict: the point where the extra bound is added to a variable that already has a bound requiring that name. The diagnostic must name both traits (by their display names), the colliding member, and the bound's location (the signature containing the repeated ability list).

**Lowering and instantiation layer:** For the consumer, `sort` will call `Order.cmp` in the concrete monomorph bodies. The existing mechanism records a per‑span symbol for any resolved user overload via `builtin_overloads.insert(span, symbol)`. To avoid symbol collision across instantiations, this record must be span‑and‑instantiation‑aware. Reading the real probing evidence in the brief, `Module::builtin_overloads` stays a `HashMap<Span, String>` spanning the whole `Module`. The cost claim there forces a per‑instantiation overload record using a composite key such as `(instantiation_symbol, member_name)` or `(subst, call_site_span)`—or a scoped lookup at each `lower_instantiation` pass that reads the instantiation symbol and the user overload symbol together to mint fresh records during lowering, without re‑resolving at each call. This maintains the "lowering never re‑runs resolution" invariant because the lookup is cheap (hash table indexed by instantiation and trait member) and only strings are concatenated, no trait walking. The IR output is identical to the current code for trait‑free words; per‑instantiation records produce slightly more linear overhead exactly where the bounded method appears in each monomorph.

**Project convention compliance:** All new constructs and stage functions receive unit tests using the existing `#[cfg(test)] mod tests` style, with at least one happy path plus one error/edge case per branch. Every phase exit criterion is a golden test reading a source file and asserting expected diagnostics; the golden directory follows the `tests/phase7_slice*.rs` pattern using `common::fixture_source`. The growth structure rule is respected: for the first slice, `src/ast.rs` and `src/check/trait.rs` (new file for trait‑related check logic) stay single-ish; each subsequent slice can split out from there if complexity emerges (e.g., trait registry into its own module).

## Codebase Map

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/ast.rs:1283` | `pub enum Bound { Copy, Ord }` | Extend to `User { trait_id }` for user traits; seed `Copy`/`Ord` as pre‑mirrored entries |
| `src/ast.rs:18` | `Module` struct | Add `traits: Vec<TraitDecl>`, `impl_satisfactions: HashMap<(TraitId, Type), ImplDecl>` fields |
| `src/ast.rs` | `TraitDecl` (new) | New AST node representing a user trait; fields: name, `ty_var_id`, members list |
| `src/ast.rs` | `TraitMember` (new) | New struct: name, signature (`Sig`) for a required method |
| `src/ast.rs` | `ImplDecl` (new) | New node for an implementation block; fields: `trait_id`, `target_ty`, `module`, body |
| `src/parser.rs:2299` | `parse_capabilities` | Replace hardcoded "Copy"/"Ord" string matches with a lookup against the trait table; ensure user trait `Copy`/`Ord` is rejected via duplicate‑declaration flow |
| `src/parser.rs` | `parse_trait_decl` (new) | New parser function for `trait: Name 'T member semicolon ;` |
| `src/parser.rs` | `parse_impl_brace` (new) | New parser function for `impl: Trait For Type ; { body } ;` and validation (orphan rules) |
| `src/check/poly.rs:7` | `is_ord(ty: Type)` | Existing predicate; stays unchanged; user `Ord` bounds still fail here rather than using a trait satisfaction check |
| `src/check/poly.rs:37` | `poly_is_copy(...)` | Handles `Bound::Copy`; reused unchanged for bounded `Copy` semantics |
| `src/check/poly.rs:3406-3419` | `check_poly_call`'s R6 loop | Extend the `match bound { Bound::Copy => ..., Bound::Ord => ... }` arm with a `Bound::User(trait_id)` case to resolve against the concrete substitution; emit bound‑satisfaction failure diagnostics naming trait, member, and concrete type |
| `src/check/poly.rs:1170-1204` | `poly_term`'s `env.get(name)` lookup and its `PolyType::Var(v) => poly_var_to_concrete_error(...)` rejection arm (line 1197) | Insert a new branch *before* this lookup (verified via probe, R2 in this spec's own re-probe of the brief): check whether the top-of-stack `PolyType::Ref(PolyType::Var(v), _)` or bare `PolyType::Var(v)` has a `Bound::User(trait_id)` requiring the called name, and if so push the trait's declared abstract output types instead of falling through to this env lookup |
| `src/check/poly.rs:1533-1546` | `poly_calls_poly_word_error` (P8.S2's R6b) | Existing, general, located rejection for a poly word calling another poly word, same-module or cross-module. Cite this directly for a bound-satisfying member that happens to be a poly word rather than a leaf/monomorphic one -- do not build new nested-dispatch machinery for this case (see Requirements, new R14) |
| `src/check/poly.rs:3028` | `poly_delegate_op`'s handling of `OpDispatch::UserOverload` | Must continue using existing `builtin_overloads.insert(span, symbol)` for overload symbol records—no change needed for existing overloads, only for bounded user methods in the consumer |
| `src/ir/driver.rs:805` | `lower_instantiation` | Intersection point for per‑instantiation lookup: the function already receives `subst`, `symbol`, `body`, and `type` maps; here we thread the instantiation symbol into the bound‑method resolution path to index the per‑instantiation overload map (no runtime re‑resolution) |
| `src/ir/driver.rs:416` | `FuncBuilder::new`'s `builtin_overloads` field | Existing reference to the per‑span overload record; lowering consumers already pass this map; new bounded‑method symbols must be added to the map keyed by `(instantiation_symbol, member_name)` or `CallInst`'s instantiation side |
| `tests/phase7_slice3e.rs` (new) | Golden tests | One test for trait parsing/declaration (duplicate, member syntax errors), one for impl validation and orphan rules, one for bound composition on a variable, one for multi‑bound member name collision, one for the array `sort` consumer, plus per‑stage unit‑test functions in the same file or new test modules |

**Integration points:** The parser, AST, and checker work together but never share mutable global state. The trait registration lives in the module during the parse -> check pipeline; the list is immutable thereafter. Bound satisfaction queries hit the private `impl_satisfactions` map that builds up during `check_impl_block`; they never consult the resolver.

**Load‑bearing constraints:** Do **not** change `src/ast.rs:72`'s `builtin_overloads: HashMap<Span, String>` field to include trait‑specific data; preserve the existing span‑only key structure and instead add a per‑instantiation wrapper only in the lowering phase (OQ1 cost is real and must be reflected in effort sizing). The existing `module.exports` list defines which traits are module‑visible; trait declarations themselves are implicitly exported per module, similar to `type:` declarations.

## Open Questions

- [x] What is the scope of the trait table—module‑scoped, prelude‑global, or both? → **Resolved as module‑scoped per module exports**; prelude traits (`Copy`/`Ord`) appear as a bootstrapped predicate‑kind entry in every module's table.
- [x] Should bound satisfaction at call sites reject partial implementations (i.e., only some required members satisfied), or require all members to be satisfied? → **Resolved to reject on any missing member**, matching R6's language "no `( &'T -- )` found" (all members).
- [x] How should bound diagnostics distinguish a trait that is entirely missing from the module vs. a trait that exists but is not satisfied? → **Resolved**: the diagnostic names the trait and the specific missing member, and the checker walks the registered trait list at the call site if the trait is present, which is pleasant reading and implies missing is a violation of the declared requirement.
- [x] Should postfix bound syntax be allowed, or must the bound ride directly on the variable's first appearance? → **Resolved**: the bound rides on the first occurrence of the variable in the signature (e.g., `( &'T: Show -- )`), per the existing constraint on where bounds are allowed.
- [x] How should the trait `impl` registry be generated during module assembly vs. something that lives along the `check` path? → **Resolved**: use the existing `check` pipeline to populate a private `Module::impl_satisfactions: HashMap<(TraitId, Type), ImplDecl>` after declaration validation, before lowering; this keeps the data flow consistent and isolates runtime introspection.

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Adding a new branch in `poly_term` introduces a regression in the existing attribute handling (e.g., `const`, `inline`, `ret` edge cases) | Medium | Run all existing `poly_term` unit tests (phase 3b/g/f goldens) as part of the integration test suite before merging; the new branch is inserted in a well‑isolated position between the shuffle/len checks and the env lookup that currently processes concrete‑typed inputs. |
| The `impl` registry grows unbounded if many implementation blocks are declared, causing memory pressure during lowering | Low | The current project size for a single module is negligible; if it scales, precedents from `trait_table` usage and `Module::builtin_overloads` suggest external analyses (dead impl pruning) are far beyond P7. For now, keep the map per module and assume its size is tiny. |
| Extending `Bound` breaks binary compatibility or unintended regressions in existing bound checks | Low | The enum is crate‑internal and never serialized; existing check paths only inspect the enum variants, and adding a new variant does not affect codegen. Unit tests covering all `Bound` usage (including the new variant) guard against regressions. |
| The per‑instantiation overload record increases IR complexity and obscures the monomorphization story | Medium | Mirror the existing `builtin_overloads` pattern: lower builds a fresh record per `lower_instantiation` keyed by `(instantiation_symbol, member_name)`, exactly as the generic injunctive shadows already do. Document the intent in comments and keep the addition localized to lowering; the IR shape and increase over baseline are linear in the number of collided bounded calls in each instantiation. |
| Member name collisions across a bound set are silently ignored unless discovered at compile time | Medium | The check flows for bound composition in P7.S1 and P7.S3a enforce uniqueness; for this slice we explicitly verify at bind‑time (or at trait registration time if we choose to validate traits first) that a member name unique across all traits for a variable's bounds. Raise a precise diagnostic naming the conflicting traits and the method, and provide library guidance on renaming or merging traits. |
| OQ1's cost (per‑instantiation overload) was probed as non‑zero; treating it as zero would cause golden test failures at lowering | Medium | Accept the cost; increase estimated effort appropriately. The hydration phase (+1‑2 days spanning Phase 2 and 3) directly acknowledges this by adding a sub‑phase for "instantiation‑aware overload record minting". |
| The trait surface introduces many new error messages that could feel inconsistent with existing diagnostics | Medium | Model all new diagnostics on existing patterns: use the word "duplicate" for duplicate declarations (`duplicate_trait_decl_error`, `duplicate_impl_error`), use "does not satisfy" for bound failures with trait/member disambiguation, and align with existing phrase "line X, col Y" locations. Run the goldens through a unified review checklist. |

## Delivery Plan

### Phase 1: Trait declaration and parser surface

**Goal:** Users can declare `trait: Order 'T cmp ( &'T &'T -- Ordering ) ;` in a module, and the parser captures this data into a new AST representation that integrates cleanly with the existing `Module` struct and is validated against the duplicate‑declaration and module‑scoping rules.

**Requirements Covered:** R1, R2, R4, R5

**Scope:**

- Files to modify:
  - `src/ast.rs`: Add `TraitDecl`, `TraitMember`, `ImplDecl` structs (exactly matching the docstring skeleton). Add `Module::traits: Vec<TraitDecl>`. Update parsing of Declarations via the declaration dispatcher (or add a new `trait:` handler before or alongside `type:/extern:` callbacks).
  - `src/parser.rs`: Add `parse_trait_decl` (recognizes `trait:` token, trait name, one `'T` variable, then member list). Add `parse_impl_brace` (recognizes `impl:`, validates required tokens), with a helper for orphan detection (checks module match against trait's module and impl's target type's module). Extend `Declaration` variant to include `Trait(…)`, `Impl(…)`.
- Files to create: `tests/phase7_slice3e_trait_parsing.rs` as a new test file mirroring `phase7_slice3b.rs`'s pattern, with unit tests for duplicate trait declarations, illegal member signatures, malformed trait bodies, and orphan rule violations.
- Out of scope for this phase:
  - Any bound syntax or trait consumption (e.g., `'T: Show`)—only the declaration syntax.
  - Implementations (`impl:`) beyond parsing and schema validation; validation of impl blocks deferred to Phase 2.
  - Trait registry runtime usage; we just record them in the AST.

**Entry Conditions:** The module parsing stage must already accept `type:`, `static:`, and `extern:` declarations so that treating `trait:` as one of those is straightforward.

**Exit Criteria / Verifiable Artifacts:**

- `cargo test` passes for `tests/phase7_slice3e_trait_parsing.rs` (all new tests).
- Golden test `tests/phase7_slice3e_trait_parsing.golden.sth` reads a `trait:` declaration and asserts the parsed `TraitDecl` matches expected fields (name, variable, member signatures).
- All existing phase 7 slice and core tests continue to pass (no regressions).

**Parallelism:** SEQUENTIAL (must precede Phase 2).

**Relative Effort:** S (1‑2 days) — the pattern is well understood from `type:`/`static:` parsing; only a single new top‑level declaration form and a thin AST layer.

**Difficulty:** `standard`.

**Open Questions / Blockers:** None identified.

---

### Phase 2: Implementation block validation and body‑side dispatch

**Goal:** The checker can validate each `impl: Trait For Type ; ... ;` block against the trait, and inserts satisfaction records into the private registry. `poly_term` gains a new branch for `&'T` calls to bounded methods, stitching the trait's required signature into the call site before the existing env lookup.

**Requirements Covered:** R3, R6, R14

**Scope:**

- Files to modify:
  - `src/check/poly.rs`: Add `check_impl_blocks` that walks the module's `impl_satisifactions` and validates that each block's target type is concrete and that the impl's module matches either the trait's module or the target type's module (orphan rule). Insert a new function `trait_satisfied_by(trait_id, concrete_ty)` that returns `Some(ImplDecl)` or `None`.
  - `src/check/poly.rs`'s `poly_term`:
    - Locate the `env.get(name)` lookup (`poly.rs:1170`) and its `PolyType::Var(v) => poly_var_to_concrete_error(...)` rejection arm (`poly.rs:1197`). Insert a new branch immediately before this lookup.
    - In this branch, check that the top-of-stack slot is `PolyType::Ref(PolyType::Var(v), mutable)` (or a bare `PolyType::Var(v)`), retrieve `sig` for the variable, and iterate through its bound list, looking for a `Bound::User(trait_id)` member requiring the called word's name.
    - If found, push the trait's declared abstract output types (no symbol, no site table -- `'T` is still abstract at this point, matching the brief's Recon 4). Do **not** construct a synthetic `Overload` or forward a placeholder symbol.
    - **R14 (leaf calls only):** if the trait member the call resolves to is itself a polymorphic word (not a leaf/monomorphic one), this call site must fail with `poly_calls_poly_word_error`'s existing wording (`src/check/poly.rs:1533-1546`, P8.S2's R6b) -- this is the existing general "a poly word cannot call another poly word" rejection, already reachable from this call site once the body reaches an ordinary poly-to-poly call. Do not add new machinery for this case; add a golden proving the existing message fires.
- Files to create: `tests/phase7_slice3e_impl_and_dispatch.rs` with:
  - Happy‑path: a trait, multiple impls, and a poly word using `Show.show` on a `&'T` slot.
  - Error cases: orphan impl (impl in wrong module), mismatched target type (impl for one concrete type while body calls another), a call to a name that no trait bound provides, and a bound-satisfying member implemented as a poly word (R14, expect `poly_calls_poly_word_error`'s exact wording).
- Out of scope:
  - No actual bound‑satisfaction checking at the call site yet (defer to Phase 3).
  - No per‑instantiation overload record generation yet (defer to Phase 4).
  - No trait implementation of `impl` isolation between modules beyond orphan detection.
  - No obligation propagation for a nested poly call (R14 rejects it outright; propagation is not this slice's scope).

**Entry Conditions:** Phase 1 must have produced a module containing parsed `TraitDecl`/`ImplDecl` structures and valid AST, so that `check_branches` can iterate and validate them.

**Exit Criteria / Verifiable Artifacts:**

- Golden test (`tests/phase7_slice3e_impl_and_dispatch.golden.sth`) passes: a module with a trait, two implementations, and a polymorphic word using a bounded method compiles successfully.
- Unit tests pass for every `check_impl_blocks` and `trait_satisfied_by` invocation, including orphans and unsatisfied impls.
- The new branch in `poly_term` is exercised by the golden test, which calls `Show.show` and succeeds in body checking.
- A golden proves R14: a bound-satisfying member implemented as a poly word, called from a bounded poly body, fails with exactly `poly_calls_poly_word_error`'s wording.

**Parallelism:** SEQUENTIAL (must precede Phase 3).

**Relative Effort:** M (roughly a week) — this is the core logic for trait satisfaction and body‑side dispatch; both `impl` checking and the `poly_term` insertion points are sensitive and must be thoroughly unit‑tested.

**Difficulty:** `hard` — the body‑side branch must integrate without breaking existing checks, and the proxy signature forwarding must be robust (no unintended side effects like reusing the wrong local scopes).

**Open Questions / Blockers:** None identified.

---

### Phase 3: Call‑site bound satisfaction and multi‑bound member collision

**Goal:** `check_poly_call`'s R6 loop now checks user trait bounds against a concrete substitute, emitting precise diagnostics for missing members or type violations. The parser validates that the bound capability list is free of member name collisions across all traits constraining a single variable.

**Requirements Covered:** R4, R7, R8

**Scope:**

- Files to modify:
  - `src/check/poly.rs`'s R6 bound loop (starting around line 1470):
    - Extend the existing `Bound::Copy`/`Bound::Ord` arms with a new branch for `Bound::User(trait_id)`.
    - After verifying the concrete type `subst.ty_of(v)`, call `trait_satisfied_by(trait_id, concrete_ty)`. If missing or the impl does not cover all required members, raise a diagnostic that names the trait, the specific missing member signature, and the concrete type at the call site.
    - Ensure the diagnostic includes the bound's original span for location.
  - `src/parser.rs`'s `parse_capabilities`:
    - Track, for each variable's bound set, the full list of `(trait_id, member_name)` pairs contributed by each recognized trait.
    - After parsing a bound list, validate that no member name is duplicated across a single variable's entire bound set. If a collision is found, raise an diagnostic naming both traits and the colliding member, with location at the bound declaration (the span after the colon where the capability list appears).
- Files to create: `tests/phase7_slice3e_bound_satisfaction.rs` with:
  - Happy path: a poly word with `Order 'T cmp` bound and a concrete type with the required member (e.g., `i64 cmp` working copy) compiles successfully.
  - Error case: a call to `cmp` with a concrete type that lacks the `cmp` member produces a bound‑satisfaction diagnostic with full trait/member/type context.
  - Error case: a bound like `'T: Eq Hash` on a closure creates a multi‑bound name collision error before even reaching the call site.
- Out of scope:
  - Lowering support for this phase's diagnostics is incomplete; we only want to verify they are produced and formatted correctly.
  - No trait‑object / runtime support yet; this is a purely compile‑time check.

**Entry Conditions:** Phase 2 must have implemented `trait_satisfied_by` and the body‑side dispatch mechanism, so we have a working registrar to query.

**Exit Criteria / Verifiable Artifacts:**

- Golden tests pass for both bound‑satisfaction and multi‑bound collision cases (`tests/phase7_slice3e_bound_satisfaction.golden.sth`).
- Unit tests cover all `check_poly_bound_satisfaction` branches (both success and failure).
- All existing phase 3/4/5 tests remain passing.

**Parallelism:** SEQUENTIAL (must precede Phase 4).

**Relative Effort:** M (1‑1.5 weeks) — Lifting the R6 loop to handle user traits is similar in shape to adding more predicates to `is_copy`/`is_ord`; the main costs are in writing the precise diagnostic wording and ensuring multi‑bound collision detection runs early (ideally before any IR is generated).

**Difficulty:** `hard` — the diagnostics must be precise and follow existing patterns, and the multi‑bound collision check must distinguish between a legitimate duplicate (same trait provides both `Eq` and `Hash`) and a collision (different traits both provide `eq`). The default assumption in the spec is to forbid duplicate member names across a variable's bound set; any user wanting both `Eq` and `Hash` that share member names must merge the traits or rename members.

**Open Questions / Blockers:** None identified.

---

### Phase 4: Array `sort` consumer golden and instantiation‑aware overload record

**Goal:** Implement the array `sort` consumer from the dogfood with `Order` showing up as a bounded method. The body contains a `cmp` call on a `&'T &'T` pair; that call must be dispatched under the trait's abstract signature at body check, then appear as a distinct per‑instantiation symbol for each monomorph of `sort` (e.g., an `i64` monomorph gets a different symbol than an `f64` monomorph). Lowering passes must generate those symbols and record them in a new per‑instantiation lookup that does not break the existing "lowering never re‑runs resolution" rule.

**Requirements Covered:** R9, R12

**Scope:**

- Files to modify:
  - `src/ast.rs`: Consider exposing `impl_satisfactions` as a public thin view for lowering, or keep it private and export a function `get_implementations_for(trait_id: TraitId, ty: Type)` to `src/ir/driver.rs`. The spec chooses the latter to preserve encapsulation and avoid exposing the registry outside of checking.
  - `src/ir/driver.rs`:
    - In `lower_instantiation` (around line 805), after receiving the substitution `subst` and the instantiation `symbol`, extract the monomorphized type arguments (e.g., for `sort` taking `'[ 'T: Copy Order 'N ]`, the monomorphized `'T` is the underlying concrete type of `Vec2`). Use this map to index a new per‑instantiation overload table: a `HashMap<(String, String), String>` keyed by `(instantiation_symbol, member_name)`.
    - For each bounded method call in the monomorph body (e.g., the `cmp` call to `Order` on `&'T` and `&'T`), build a symbol like `${instantiation_symbol}_traits_${trait_name}_${member_name}` and insert it into the overload table. Use the same mechanism as `builtin_overloads.insert(span, symbol)` to record it at the call site.
    - Update `FuncBuilder::new` to accept an additional argument `per_instantiation_overloads: &HashMap<(String, String), String>` or to build it lazily from a passed `InstantiationContext` struct.
  - `tests/phase7_slice3e_sort_golden.rs`: Write a golden test that includes:
    - `type: Ordering | Less | Equal | Greater ;`
    - `trait: Order 'T cmp ( &'T &'T -- Ordering ) ;`
    - Multiple `impl:` blocks for that trait on specific struct/enum types (e.g., `i64`, `f64`, `String`).
    - `: sort ( [ 'T: Copy Order 'N ] -- [ 'T 'N ] )` with an in‑place insertion sort body that calls `cmp` on adjacent elements.
    - An array of elements of those types, demonstrating that `sort` works for each concrete monomorph (e.g., `sort`[ i64 2 1 ]` yields `[ i64 1 2 ]`).
  - `tests/phase7_slice3e_golden_golden.sth`: Assert both the runtime execution correctness (output matches expected sorted order) and the absence of re‑resolution at lowering (no tracing of trait lookup at runtime).

- Files to create: `src/check/trait.rs` (recommended, though you could keep it inside `poly.rs` for now), containing `check_trait_declarations`, `check_impl_blocks`, `trait_satisfied_by`, and helper diagnostic messages.

- Out of scope:
  - No constraint solving for trait type lower bounds (e.g., auto‑generating a `Copy` impl from a derived protocol).
  - No support for more complex trait membership rules (traversable, sized, etc.).
  - No trait objects or runtime dispatch; this is a compile‑time trait system only.

**Entry Conditions:**

- Phases 1‑3 must be complete, providing a working trait registry, bound checking, and body‑side dispatch.
- The module's trait and impl declarations must be parsed and validated in the `check` pass, so that `lower_instantiation` can consult them safely.

**Exit Criteria / Verifiable Artifacts:**

- Golden test passes: the array `sort` program with user `Order` compiles, lowers to QBE, and executes correctly for each concrete monomorph.
- Goldens pass that diagnostics produced by bound violation checks are exactly as expected (including trait/member typing).
- All existing phase 7 goldens remain stable (no unintended changes to existing `.sth` outputs).

**Parallelism:** SEQUENTIAL (must follow Phase 3).

**Relative Effort:** M (1.5‑2 weeks) — managing the per‑instantiation overload record is the load‑bearing integration point and must be carefully designed to preserve existing lowering invariants. The sort consumer adds complexity to the `check` side (body‑side dispatch of trait members) and the `lower` side (in‑body bounded method call resolution), each of which needs independent unit tests plus the green consumer golden.

**Difficulty:** `hard` — maintaining the "lowering never re‑runs resolution" invariant while surfacing a per‑instantiation record is inherently complex; the spec's recommended approach is to mint records from the `Module::builtin_overloads`-style hash map, keyed by `(instantiation_symbol, member_name)`, during `lower_instantiation` without mutating the trait registry. This approach avoids altering the call graph's lower‑time ordering, but it requires careful coordination between the check‑side symbol generation and the lower‑side record insertion. The Golden test makes regressive changes impossible to ignore.

**Open Questions / Blockers:** None identified.

---

### Parallelism Summary

- Phase 1, 2, and 3 must run sequentially: parsing + impl validation (P1) → body‑side dispatch (P2) → call‑site bound checking (P3). The phases rely on each other's artifacts (e.g., P2's `poly_term` branch relies on P1's `TraitDecl` structures; P3's R6 loop builds on P2's satisfaction cache).
- Phase 4 can proceed in parallel with a limited set of Phase‑3 changes (e.g., unit tests for bound‑satisfaction diagnostics can be written without blocking the lowering plumbing), but the core implementation (overload record) must wait until the trait registry and bounds are stable. In practice, run all phases sequentially to avoid integration spaghetti.
- Unit test suites for each phase (new `.rs` files) can be compiled in parallel earlier in the development cycle; but integration tests that touch `poly_term` or `lower_instantiation` must be gated by the correct Phase markers.

---

### Effort Summary

- Phase 1: S (1‑2 days)
- Phase 2: M (roughly 1 week)
- Phase 3: M (1‑1.5 weeks)
- Phase 4: M (1.5‑2 weeks)
- Total: M + M + M + M ≈ 4‑6.5 weeks depending on complexity and integration friction.

---

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Trait declaration and parser surface", "effort": "S", "difficulty": "standard" },
    { "phase": 2, "focus": "Implementation block validation and body‑side dispatch", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "Call‑site bound satisfaction and multi‑bound member collision", "effort": "M", "difficulty": "hard" },
    { "phase": 4, "focus": "Array sort consumer golden and instantiation‑aware overload record", "effort": "M", "difficulty": "hard" }
  ]
}
```
