## Problem Statement

P7.S4 shipped generic `impl:` targets with a specificity chain, but deliberately left bounds on an impl's own type variables out of scope. A generic impl's member word carried `PolySig { bounds: vec![], .. }`, so `poly_trait_member_call` never recognized a trait-member call on an impl variable — it fell through to ordinary word lookup, which failed to resolve. This blocked the element-wise `Show`/`Eq` forms S4 named as its motivating consumer: `impl: Show for ['T N]` whose `show` iterates and calls `show` on each element needs `'T: Show` declared on the impl, and recursive per-instantiation dispatch (when the member word is monomorphized at `[Point 4]`, each element's `show` call must resolve to `Point`'s `impl: Show`, discovered by discharging `'T: Show` against the concrete `Point`).

## Requirements

- **R1.** An `impl:` target may carry a `where`-clause after the target pattern — `impl: Show for ['T N] where 'T: Show` — parsed by a new `parse_impl_bounds` function. This function resolves each variable name against the target's already-parsed `ty_var_names`/`len_var_names` tables (erroring on unknown names), then calls `parse_capabilities` for the bound list. It deliberately does NOT reuse `parse_poly_ty_var`, which enforces a binding-vs-use check via `intern_ty_var` that rejects bounds on already-interned variables. Since `parse_impl_target` already interns `'T` while parsing the target pattern, a `where`-clause re-mention is a use, not a binding. Multiple bounded variables are space-separated: `where 'T: Show 'V: Eq`. A target with no `where`-clause behaves exactly as today (`bounds: vec![]`). `where` is a new keyword with no existing lexer collision.
- **R2.** `ImplTarget` carries the parsed bounds (`Vec<(u32, Bound)>`, keyed to the impl's own `ty_var_names`/`len_var_names` indices) alongside the existing `pattern`/name tables.
- **R3.** `parse_impl_member_body` constructs the generic member word's `PolySig` with `bounds` populated from the target's declared bounds, rather than always empty. No other field of the member word's `PolySig` changes. The existing `poly_trait_member_call`, `check_poly_body`, and obligation-recording machinery need no modification.
- **R4.** Two generic `impl:` blocks for one trait with the same target pattern but different bound sets are distinct declarations, not a duplicate error. The duplicate key in `check_impl_decls` widens to include the bound set: `(TraitId, PolyType, BoundSet)`, where `BoundSet` is compared as an unordered set of `(var_name, Bound)` pairs, using the impl's own name tables to normalize variable identity across alpha-equivalent targets. Two blocks with the same pattern and same bound set remain a duplicate error.
- **R5.** `specificity` widens its comparison domain to include bound sets: for two candidates with comparable patterns under the existing pattern order, a strictly-more-constrained bound set at an equal pattern is a specificity tiebreaker — `impl: Eq for ['T N] where 'T: Eq` is more specific than `impl: Eq for ['T N]` with no bounds, at any instantiation where both match. "Strictly more constrained" means proper-superset containment (A ⊃ B). Variable identity is normalized as in R4. If neither pattern nor bound set strictly dominates, candidates remain incomparable and an unordered candidate set is the existing located ambiguity error. A candidate whose bound fails to discharge at the concrete instantiation does not match at all and is excluded from the candidate set entirely before `select_most_specific` is called.
- **R6.** When a generic impl's member word is monomorphized at a concrete instantiation, each bound on its own variables becomes a concrete obligation discharged against the impl registry, recursively. A side-effect-free helper (`find_bound_impl`) is factored out of `resolve_user_bound`: it takes `(trait_id, ty, tr, ...)` and returns the winning `(ImplDecl, Subst)` or error, performing candidate-finding via `match_impl_target` and selection via `select_most_specific` without obligation-routing side effects. The recursive discharge calls the helper directly to check existence without side effects, avoiding rollback. A candidate whose declared bound fails to discharge is excluded from the match set (R5); if that leaves no candidate, the existing "unsatisfied bound" diagnostic fires.
- **R7.** A bound-discharge cycle — an impl whose bound requires itself at the same type, directly or transitively — is a located error, not infinite recursion. Detected via a path-scoped visited-set of `(TraitId, Type)` pairs threaded through `find_bound_impl`: inserted on entry, removed on back-track, so the set tracks only the current DFS path. This prevents false-positives on diamond-shaped shared resolutions while catching true cycles. The cycle error is reported at the span of the impl declaration whose bound creates the cycle edge. The 3-colour DFS shape in `check_combinator_cycles` (`src/check/combinators.rs:207`) is the closest existing pattern.
- **R8.** (NFR, parity) Every existing `impl:` (generic and concrete, bounded and unbounded) dispatches identically to before this slice; `examples/traits.sth`, `lib/cmp.sth`, and `core`'s `sort` compile and run unchanged; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` stays green. The R11 bound-set-tiebreak goldens were failing at the point this spec was condensed: `synth_member_word_name` (`src/parser.rs`) rendered a member word's name from only its trait and pattern, so two impls sharing a pattern but differing by bound set collided on one name and `impl_mono_seed` (`src/check/poly.rs`) resolved the wrong word, panicking at lowering. Fixed by folding a deterministic, var-idx-sorted rendering of `target.bounds` into the synthesized name; `cargo test` is green as of that fix.
- **R9.** (NFR, golden) A golden test demonstrates `impl: Show for ['T N] where 'T: Show` instantiated at `[Point 4]` (where `Point` has its own `impl: Show`) compiling and running identically to a hand-written per-element concrete counterpart, and that omitting `Point`'s `impl: Show` produces the located unsatisfied-bound error.
- **R10.** (NFR, golden) A golden test demonstrates a self-referential bound cycle producing the located cycle error rather than a stack overflow or hang.
- **R11.** (NFR, golden) A golden test demonstrates a bounded generic impl overriding an unbounded generic impl with the same pattern at instantiations where the bound is satisfied, per R5.
- **R12.** (NFR) Each new function (`parse_impl_bounds`, the widened duplicate key, the factored `find_bound_impl` helper, the widened `specificity`/`select_most_specific`, the cycle detector) has unit tests beside it: a happy path plus at least one error/edge case.

## Success Criteria

- `impl: Show for ['T N] where 'T: Show` parses; a body that calls `show` on each element type-checks (recognized as trait dispatch via `poly_trait_member_call`, not ordinary word lookup).
- Monomorphized at `[Point 4]` where `Point` has `impl: Show`, the member word's element `show` calls resolve to `Point`'s impl and the program runs identically to a hand-written per-element concrete counterpart.
- Omitting `Point`'s `impl: Show` produces the existing located unsatisfied-bound diagnostic at the array's `show` call site.
- `impl: Eq for ['T N] where 'T: Eq` and `impl: Eq for ['T N]` (no bounds) coexist as distinct declarations; the bounded one dispatches where the bound is satisfied.
- A self-referential bound cycle is a located error, not a hang.
- `examples/traits.sth`, `lib/cmp.sth`, and `core`'s `sort` compile and run unchanged; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.

## Scope & Boundaries

**In scope:**

- `where`-clause grammar attached to an `impl:` target, threading declared bounds into the generic member word's `PolySig`, widening the duplicate-impl key and `specificity` to account for bound sets, and the recursive per-instantiation bound-discharge mechanism (with cycle detection) that lets a generic impl's body dispatch trait-member calls on its own bounded variables.

**Out of scope:**

- `poly_trait_member_call`, `resolve_user_bound`'s existing (non-recursive) discharge path, `TraitResolveCtx`, and the obligation recording/resolution mechanism itself — reused unmodified. The new work is populating `PolySig::bounds` for impl member words and adding the recursive discharge call plus its cycle guard, not changing how bound dispatch works.
- S4's matching, specificity's *pattern* comparison, and orphan rule — inherited as-is; this slice only adds a bound-set dimension on top.
- Inline-in-pattern bound syntax (`['T: Show N]`) and a binder-bar form (`| 'T: Show |`) — the `where`-clause is the sole attachment point.
- `drop` as a trait, trait objects (S3u, parked), default member bodies, supertraits, multi-type-variable traits.
- The REPL path — `impl:` remains a build-time, whole-program-assembly feature.

## Design Decisions & Rationale

**`where`-clause over inline or binder-bar bounds.** The impl target's variables are bound by the target pattern itself, not by a `PolySig` slot, so the existing bound grammar has no attachment point on a target. A trailing `where`-clause introduces one unambiguous keyword and reuses the existing bound-list parser `parse_capabilities`. The clause does NOT reuse `parse_poly_ty_var`: that function interns the variable via `intern_ty_var` and rejects bounds on non-binding occurrences, but `parse_impl_target` has already interned every target variable, so a `where`-clause re-mention is a use, not a binding. The two alternative attachment points were rejected on collision grounds: inline bounds (`['T: Show N]`) would reopen the `forbid_bounds: true` ambiguity `parse_impl_target` deliberately set; a binder bar (`| 'T: Show |`) overloads `Token::Pipe` a third time within one declaration.

**Distinct-with-satisfaction-dispatch, not duplicate-rejection.** Keeping the duplicate key as `(TraitId, PolyType)` alone would make the textbook specialization case (`impl: Eq for ['T N] where 'T: Eq` refining `impl: Eq for ['T N]`) permanently unrepresentable, cutting against the entire point of S4's specificity chain. Widening the key to `(TraitId, PolyType, BoundSet)` and widening `specificity` to treat a strictly-more-constrained bound set as a tiebreaker is the only option that keeps specialization possible.

**Cycle detection mirrors `check_combinator_cycles`.** `resolve_user_bound` had no existing visited-set; the recursive discharge this slice introduces is the first place a bound resolution can call back into itself. A path-scoped (insert-on-entry, remove-on-backtrack) `(TraitId, Type)` visited-set prevents false-positives on diamond-shaped shared resolutions while catching true cycles. The 3-colour DFS shape in `check_combinator_cycles` (`src/check/combinators.rs:207`) is the closest existing pattern, so the new visited-set follows that shape rather than inventing a new idiom.

## Open Questions

None — the brief's two open items (grammar attachment point, specificity vs. duplicate-rejection for bounded/unbounded pairs) were resolved during probe verification (see brief).

## Implementation

| Area | Commit | Key files |
|---|---|---|
| `where`-clause grammar, `ImplTarget.bounds` field, bounds threading into member `PolySig`, duplicate-key widening to `(TraitId, PolyType, BoundSet)` (R1–R4, R8 partial, R12 partial) | `ba6c318c` | `src/parser.rs` (`parse_impl_bounds`), `src/ast.rs` (`ImplTarget`), `src/check/declarations.rs` (`check_impl_decls`) |
| Recursive per-instantiation bound discharge via factored `find_bound_impl` helper, path-scoped cycle detection, R9/R10 goldens and unit tests (R6, R7, R9, R10, R12 remainder) | `40458be2` | `src/check/poly.rs` (`find_bound_impl`, `resolve_user_bound`), `tests/phase7_slice4b.rs` |
| `specificity` bound-set tiebreak via proper-superset containment, `select_most_specific` widened to pass bound data, R11 goldens and ambiguity golden (R5, R11, R12 remainder) | `e8531117` | `src/check/poly.rs` (`specificity`, `select_most_specific`), `tests/phase7_slice4b.rs` |

### Notable implementation detail

The R9 golden uses `Print` (a separate trait) rather than `Show` in the `where`-clause because `rewrite_member_self_calls` rewrites any call to the member's own name (`show`) to the synthesized self-word symbol, preventing `poly_trait_member_call` from recognizing it as trait dispatch. A different trait's member name (`print`) is not rewritten and is correctly recognized as trait dispatch on the bounded variable.
