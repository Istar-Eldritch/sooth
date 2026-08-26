# Spec: P7.S4b — bounds on impl variables

**Status:** Draft
**Created:** 2026-08-26
**Discovery:** `docs/roadmap/P7/slice4b-brief.md`

## Problem Statement

**P7.S4** shipped generic `impl:` targets with a specificity chain, but deliberately left
bounds on an impl's own type variables out of scope: a generic impl's member word carries
`PolySig { bounds: vec![], .. }`, so `poly_trait_member_call` (`src/check/poly.rs:911`, the
`for (v, bound) in &sig.bounds` loop at `:952`) never recognizes a trait-member call on an
impl variable — it falls through to ordinary word lookup, which fails to resolve. This blocks
the element-wise `Show`/`Eq` forms S4 named as its motivating consumer:
`impl: Show for ['T N]` whose `show` iterates and calls `show` on each element needs
`'T: Show` declared on the impl, and recursive per-instantiation dispatch (when the member
word is monomorphized at `[Point 4]`, each element's `show` call must resolve to `Point`'s
`impl: Show`, discovered by discharging `'T: Show` against the concrete `Point`). This slice
closes that gap.

## Requirements

- **R1.** An `impl:` target may carry a `where`-clause after the target pattern —
  `impl: Show for ['T N] where 'T: Show` — parsed by a new `parse_impl_bounds`
  function called from `parse_impl_decl` after `parse_impl_target` returns. This
  function reads each variable name token (e.g. `'T`), resolves its ID by lookup
  against the target's already-parsed `ty_var_names`/`len_var_names` tables (erroring
  on an unknown name), expects `:`, then calls `parse_capabilities`
  (`src/parser.rs:3505`) to parse the bound list — reusing the existing bound-list
  parser but NOT `parse_poly_ty_var`, which enforces a binding-vs-use check via
  `intern_ty_var` (`src/parser.rs:1592`) that rejects bounds on any non-first
  (already-interned) variable occurrence. Since `parse_impl_target` already
  interns `'T` while parsing the target pattern, a `where`-clause re-mention is a
  *use*, not a binding, and `parse_poly_ty_var` would raise `bound_on_use_error`
  (`src/parser.rs:3334`, the call site inside `parse_poly_ty_var`). Multiple bounded variables are space-separated:
  `where 'T: Show 'V: Eq`. A target with no `where`-clause behaves exactly as
  today (`bounds: vec![]`). `where` is a new keyword with no existing lexer
  collision.
- **R2.** `ImplTarget` (`src/ast.rs:1873`) carries the parsed bounds (`Vec<(u32, Bound)>`,
  keyed to the impl's own `ty_var_names`/`len_var_names` indices) alongside the existing
  `pattern`/name tables.
- **R3.** `parse_impl_member_body` (`src/parser.rs:2694`) constructs the generic member
  word's `PolySig` with `bounds` populated from the target's declared bounds (replacing the
  `bounds: Vec::new()` literal at `:2777`), rather than always empty. No other field of the
  member word's `PolySig` changes. The existing `poly_trait_member_call`,
  `check_poly_body`, and obligation-recording machinery need no modification: they already
  treat `PolySig.bounds` as bound-source-agnostic.
- **R4.** Two generic `impl:` blocks for one trait with the same target pattern but
  different bound sets are distinct declarations, not a duplicate error. The duplicate key
  in `check_impl_decls` (`src/check/declarations.rs:431-443`, today `(TraitId, PolyType)`)
  widens to include the bound set: `(TraitId, PolyType, BoundSet)`, where a `BoundSet` is
  compared as an unordered set of `(var_name, Bound)` pairs (using the impl's own
  `ty_var_names`/`len_var_names` to normalize variable identity across alpha-equivalent
  targets, mirroring R7's existing pattern-equality normalization from P7.S4). Two blocks
  with the same pattern and the same bound set remain a duplicate error.
- **R5.** `specificity` (`src/check/poly.rs`) widens its comparison domain to include
  bound sets: for two candidates with comparable (non-incomparable) patterns under the
  existing pattern order, a strictly-more-constrained bound set at an equal pattern is a
  specificity tiebreaker — `impl: Eq for ['T N] where 'T: Eq` is more specific than
  `impl: Eq for ['T N]` with no bounds, at any concrete instantiation where both match
  (i.e. where the bound is discharged; see R6). "Strictly more constrained" is defined as
  proper-superset containment: bound set A is strictly more constrained than B iff every
  `(var, Bound)` pair in B is also in A and A has at least one pair not in B (A ⊃ B).
  Variable identity is normalized across alpha-equivalent targets using the impl's own
  `ty_var_names`/`len_var_names` (same normalization as R4). If neither pattern nor bound
  set strictly dominates the other, the candidates remain incomparable and an unordered
  candidate set is the existing located ambiguity error (P7.S4 R8). A bound only
  participates in matching when it can be discharged (R6) — a candidate whose bound
  fails to discharge at the concrete instantiation does not match at all, and is excluded
  from the candidate set entirely (not merely deprioritized). This exclusion happens
  before `select_most_specific` is called, so `specificity` only sees candidates whose
  bounds have already discharged.
- **R6.** When a generic impl's member word is monomorphized at a concrete instantiation,
  each bound on its own variables becomes a concrete obligation discharged against the
  impl registry, recursively. This requires factoring a side-effect-free helper (e.g.
  `find_bound_impl`) out of `resolve_user_bound` (`src/check/poly.rs`): the helper takes
  `(trait_id, ty, tr, ...)` and returns the winning `(ImplDecl, Subst)` or an error,
  performing candidate-finding via `match_impl_target` and selection via
  `select_most_specific` — the first ~40 lines of `resolve_user_bound`'s body, without
  the obligation-routing tail (`trait_calls`/`impl_monos` mutation). The existing
  `resolve_user_bound` calls this helper then does obligation routing; the recursive
  discharge calls the helper directly to check existence without side effects,
  avoiding the need for rollback on non-winning candidates. For `impl: Show for
  ['T N] where 'T: Show` instantiated at `[Point 4]`, the `'T: Show` bound becomes
  `Point: Show`, and the helper finds `impl: Show for Point` in the registry. A
  candidate whose declared bound fails to discharge (no matching `impl:` for the
  concrete type) is excluded from the match set at that instantiation (R5); if that
  leaves no matching candidate, the existing "unsatisfied bound" diagnostic fires.
  The recursion happens through the existing `impl_monos` → `cross_calls_of` →
  `compose` → `resolve_user_bound` chain: the winning impl's member word is
  monomorphized, `compose` iterates its `sig.bounds`, and `resolve_user_bound`
  discharges each — the new cycle guard (R7) wraps this chain.
- **R7.** A bound-discharge cycle — an impl whose bound requires itself at the same type,
  directly or transitively — is a located error, not infinite recursion. Detected via a
  path-scoped visited-set of `(TraitId, Type)` pairs threaded through the discharge:
  a pair is inserted on entry to `find_bound_impl` and removed on back-track (return),
  so the set tracks only the current DFS path, not all visited nodes. This prevents
  false-positives on diamond-shaped shared resolutions (two impls whose bounds both
  require the same `(TraitId, Type)` via different paths) while catching true cycles
  (a pair already in the path-set on entry is a back-edge = cycle). A self-edge (a
  bound that immediately requires its own `(TraitId, Type)`) is a cycle of length
  one. The cycle error is reported at the span of the impl declaration whose bound
  creates the cycle edge (`ImplDecl.span`). The 3-colour DFS shape in
  `check_combinator_cycles` (`src/check/combinators.rs:207`) is the closest existing
  pattern; the path-scoped set follows that shape rather than inventing a new
  cycle-detection idiom.
- **R8.** (NFR, parity) Every existing `impl:` (generic and concrete, bounded and
  unbounded) dispatches identically to before this slice; `examples/traits.sth`,
  `lib/cmp.sth`, and `core`'s `sort` compile and run unchanged; `cargo fmt --check &&
  cargo clippy -- -D warnings && cargo test` stays green.
- **R9.** (NFR, golden) A golden test demonstrates `impl: Show for ['T N] where 'T: Show`
  instantiated at `[Point 4]` (where `Point` has its own `impl: Show`) compiling and
  running identically to a hand-written per-element concrete counterpart, and that omitting
  `Point`'s `impl: Show` produces the located unsatisfied-bound error at the array's `show`
  call site.
- **R10.** (NFR, golden) A golden test demonstrates a self-referential bound cycle (an
  `impl:` whose declared bound on its own variable requires the same `(TraitId, Type)`
  pair being defined) produces the located cycle error rather than a stack overflow or
  hang.
- **R11.** (NFR, golden) A golden test demonstrates a bounded generic impl overriding an
  unbounded generic impl with the same pattern at instantiations where the bound is
  satisfied, per R5.
- **R12.** (NFR) Each new function (`parse_impl_bounds`, the widened duplicate key,
  the factored `find_bound_impl` helper, the widened `specificity`/`select_most_specific`,
  the cycle detector) has unit tests beside it: a happy path plus at least one error/edge
  case.

## Success Criteria

- `impl: Show for ['T N] where 'T: Show` parses; a body that calls `show` on each element
  type-checks (recognized as trait dispatch via `poly_trait_member_call`, not ordinary word
  lookup).
- Monomorphized at `[Point 4]` where `Point` has `impl: Show`, the member word's element
  `show` calls resolve to `Point`'s impl and the program runs identically to a hand-written
  per-element concrete counterpart.
- Omitting `Point`'s `impl: Show` produces the existing located unsatisfied-bound
  diagnostic at the array's `show` call site.
- `impl: Eq for ['T N] where 'T: Eq` and `impl: Eq for ['T N]` (no bounds) coexist as
  distinct declarations; the bounded one dispatches where the bound is satisfied.
- A self-referential bound cycle is a located error, not a hang.
- `examples/traits.sth`, `lib/cmp.sth`, and `core`'s `sort` compile and run unchanged;
  `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.

## Scope & Boundaries

**In scope:**

- `where`-clause grammar attached to an `impl:` target, threading declared bounds into the
  generic member word's `PolySig`, widening the duplicate-impl key and `specificity` to
  account for bound sets, and the recursive per-instantiation bound-discharge mechanism
  (with cycle detection) that lets a generic impl's body dispatch trait-member calls on its
  own bounded variables.

**Out of scope:**

- `poly_trait_member_call`, `resolve_user_bound`'s existing (non-recursive) discharge path,
  `TraitResolveCtx`, and the obligation recording/resolution mechanism itself — reused
  unmodified. The new work is populating `PolySig::bounds` for impl member words and adding
  the recursive discharge call plus its cycle guard, not changing how bound dispatch works.
- S4's matching, specificity's *pattern* comparison, and orphan rule — inherited as-is;
  this slice only adds a bound-set dimension on top.
- Inline-in-pattern bound syntax (`['T: Show N]`) and a binder-bar form (`| 'T: Show |`) —
  the `where`-clause is the sole attachment point (see brief for why: the former reopens
  the `forbid_bounds` ambiguity `parse_impl_target` resolves; the latter overloads
  `Token::Pipe` a third time in one declaration).
- `drop` as a trait, trait objects (S3u, parked), default member bodies, supertraits,
  multi-type-variable traits.
- The REPL path — `impl:` remains a build-time, whole-program-assembly feature.

## Design Decisions & Rationale

**`where`-clause over inline or binder-bar bounds.** The impl target's variables are bound
by the target pattern itself, not by a `PolySig` slot, so the existing bound grammar
(bounds at a variable's *binding site*) has no attachment point on a target. A trailing
`where`-clause introduces one unambiguous keyword and reuses the existing bound-list
parser `parse_capabilities` to read each variable's bound list. The clause does NOT reuse
`parse_poly_ty_var`: that function interns the variable via `intern_ty_var` and rejects
bounds on non-binding (already-interned) occurrences (`bound_on_use_error`), but
`parse_impl_target` has already interned every target variable, so a `where`-clause
re-mention is a use, not a binding. The new `parse_impl_bounds` resolves variable names
against the already-parsed `ty_var_names`/`len_var_names` tables instead. The two
alternative attachment points were rejected on collision grounds, not aesthetics: inline
bounds (`['T: Show N]`) would reopen the `forbid_bounds: true` ambiguity
`parse_impl_target` deliberately set to keep a member body's leading `:` from being
misread as a bound colon; a binder bar (`| 'T: Show |`) overloads `Token::Pipe` a third
time within one declaration.

**Distinct-with-satisfaction-dispatch, not duplicate-rejection.** The zero-effort fallback
— keeping the duplicate key as `(TraitId, PolyType)` alone, so a bounded and unbounded impl
at the same pattern collide — was rejected because it makes the textbook specialization
case (`impl: Eq for ['T N] where 'T: Eq` refining `impl: Eq for ['T N]`) permanently
unrepresentable, cutting against the entire point of S4's specificity chain. Widening the
key to `(TraitId, PolyType, BoundSet)` and widening `specificity` to treat a
strictly-more-constrained bound set as a tiebreaker is the more work, but it is the only
option that keeps specialization possible.

**Cycle detection mirrors `check_combinator_cycles`.** `resolve_user_bound` has no
existing visited-set; the recursive discharge this slice introduces is the first place a
bound resolution can call back into itself. A path-scoped (insert-on-entry,
remove-on-backtrack) `(TraitId, Type)` visited-set prevents false-positives on
diamond-shaped shared resolutions while catching true cycles. The 3-colour DFS shape in
`check_combinator_cycles` (`src/check/combinators.rs:207`) is the closest existing pattern
(its self-edge-is-a-cycle variant matches a bound requiring itself at the same type
directly), so the new visited-set follows that shape rather than inventing a new
cycle-detection idiom.

## Open Questions

None — the brief's two open items (grammar attachment point, specificity vs.
duplicate-rejection for bounded/unbounded pairs) were resolved during probe verification
(see brief).

## Implementation

### Phase 1 — `where`-clause grammar, bounds threading, duplicate-key widening (R1, R2, R3, R4, R8 partial, R12 partial)

**Scope.** `src/parser.rs` (new `parse_impl_bounds` function called from
`parse_impl_decl` at `:2608` after `parse_impl_target` returns, reading `where` then
resolving each variable name against `target.ty_var_names`/`len_var_names` and calling
`parse_capabilities` for the bound list; `parse_impl_member_body` at `:2694`, replacing
the `bounds: Vec::new()` literal at `:2777` with the target's declared bounds),
`src/ast.rs` (`ImplTarget` at `:1873` gains a `bounds: Vec<(u32, Bound)>` field; the single
construction site at `src/parser.rs:2682` is the only place to update),
`src/check/declarations.rs` (`check_impl_decls` duplicate check at `:431-443` widened to
`(TraitId, PolyType, BoundSet)`, comparing bound sets by `(var_name, Bound)` normalized
against the impl's own name tables).

**Out of bounds.** Recursive discharge (phase 2), `specificity`'s bound-set tiebreak
(phase 3) — this phase only threads bounds through parsing and declaration-time dedup; it
does not make bound-carrying member words dispatch correctly yet (that requires phase 2's
discharge, since `poly_trait_member_call` will record an obligation but nothing resolves
it against a concrete instantiation until phase 2 lands).

**Entry.** P7.S4 landed at HEAD (generic `impl:` targets with `bounds: vec![]`).

**Exit.** `impl: Show for ['T N] where 'T: Show` parses; the member word's `PolySig`
carries the bound; a body calling `show` on an element is recognized as trait dispatch by
`poly_trait_member_call` (type-checks, even though nothing resolves it end-to-end yet); a
bounded and unbounded impl at the same pattern are accepted as distinct declarations (not
a duplicate error); two impls with the same pattern and same bound set are still a
duplicate error; unit tests for the `where`-clause parse path and the widened duplicate
key (happy path + a duplicate-with-same-bounds edge case); full green.

### Phase 2 — recursive per-instantiation bound discharge with cycle detection (R6, R7, R9, R10, R12 remainder)

**Scope.** `src/check/poly.rs` (factor a side-effect-free `find_bound_impl` helper out of
`resolve_user_bound`: the candidate-finding + `select_most_specific` portion, returning
`(ImplDecl, Subst)` or error, without the obligation-routing tail; `resolve_user_bound`
calls it then does routing as before; the recursive discharge and the R5 candidate
filtering both call `find_bound_impl` directly; a path-scoped `(TraitId, Type)` visited-set
inserted on entry / removed on back-track is threaded through the helper, catching
cycles without false-positives on diamond-shaped shared resolutions; the cycle error is
reported at `ImplDecl.span`). `src/check/declarations.rs` may need a minor change if the
`compose` path (which calls `resolve_user_bound`) needs the candidate-filtering step
before `select_most_specific`.

**Out of bounds.** `specificity`'s bound-set tiebreak (phase 3) — this phase makes
discharge work and excludes non-discharging candidates from the match set (a minimal
form of R5: "fails to discharge → excluded"), but the *tiebreak* between two
successfully-discharging candidates with different bound sets is phase 3.

**Entry.** Phase 1 landed and green.

**Exit.** `impl: Show for ['T N] where 'T: Show` instantiated at `[Point 4]` (with
`Point`'s own `impl: Show` present) resolves each element's `show` call to `Point`'s impl
and runs identically to a hand-written per-element concrete counterpart (R9's golden);
omitting `Point`'s `impl: Show` produces the located unsatisfied-bound error at the
array's `show` call site (R9's negative golden); a self-referential bound cycle produces
the located cycle error, not a hang (R10's golden); unit tests for the recursive
discharge (happy path, unsatisfied-bound edge case) and the cycle detector (happy path,
self-edge cycle); full green.

### Phase 3 — specificity bound-set tiebreak and goldens (R5, R11, R12 remainder)

**Scope.** `src/check/poly.rs` (`specificity` widened to accept bound-set data and apply
the proper-superset tiebreak (R5) when patterns are equal; `select_most_specific`
widened to pass each candidate's bound set to `specificity` — it is the sole caller and
currently passes only `&target.pattern`, so it must also pass `&target.bounds`; candidates
that fail to discharge per phase 2 are already excluded from the candidate set before
reaching `select_most_specific`).

**Out of bounds.** Nothing beyond the tiebreak comparison and its goldens — grammar,
threading, and discharge are all phases 1-2.

**Entry.** Phase 2 landed and green.

**Exit.** `impl: Eq for ['T N] where 'T: Eq` dispatches over `impl: Eq for ['T N]` (no
bounds) at instantiations where the bound is satisfied (R11's golden); two candidates
where neither pattern nor bound set strictly dominates remain incomparable, producing the
existing located ambiguity error; unit tests for the widened `specificity` (happy path:
bounded beats unbounded at equal pattern; edge case: incomparable bound sets stay
incomparable); full green (`cargo fmt --check && cargo clippy -- -D warnings && cargo
test`); `examples/traits.sth`, `lib/cmp.sth`, and `core`'s `sort` unchanged (R8).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "where-clause grammar (new parse_impl_bounds, not parse_poly_ty_var) on impl: targets, threading declared bounds into the generic member word's PolySig, and widening the duplicate-impl key to (TraitId, PolyType, BoundSet)", "effort": "M", "difficulty": "standard" },
    { "phase": 2, "focus": "factor side-effect-free find_bound_impl helper from resolve_user_bound, recursive per-instantiation bound discharge with a path-scoped (TraitId, Type) visited-set for cycle detection", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "specificity bound-set tiebreak via proper-superset containment (bounded impl beats unbounded impl at equal pattern), widening select_most_specific to pass bound data, plus the full R9/R10/R11 goldens", "effort": "S", "difficulty": "standard" }
  ]
}
```
