# `check.rs` modularisation — technical spec

Source brief: [check-modularisation-brief.md](./check-modularisation-brief.md).

This is **pure structural refactoring**: code motion plus the minimum visibility
adjustments (`pub(crate)` / `pub(super)`) required to compile across module boundaries.
No behaviour changes to checking logic, diagnostics, or error text. Every existing golden,
example, and unit test must pass byte-for-byte after each phase, unmodified in intent.

## Goal

`src/check.rs` is 16664 lines (11320 non-test, `#[cfg(test)] mod tests` from L11321 to EOF).
Its own `module_report` outline exceeds the tool token cap, making it hard for humans and
agents to navigate. Split it into `src/check/*.rs` submodules grouped by the dependency
clusters the brief measured, following the existing `src/backend.rs` → `src/backend/qbe.rs`
precedent (Rust 2021, no `mod.rs`).

After the split, `src/check.rs` is a thin shim: `mod` declarations plus `pub(crate) use`
re-exports that keep the exact `crate::check::*` surface reachable, so **no call site outside
`check.rs` changes**.

## Precondition (re-check before starting)

The brief requires this run only after in-flight implementation work has merged and no
`impl/*` worktree exists. Verified at spec time: `main` is at `ab14a9f`, `git worktree list`
shows only the primary tree. Re-run `git worktree list` before Phase 1; if a new `impl/*`
worktree exists, wait for it to merge.

## The concrete acceptance check (behaviour-preservation proof)

The single objective test that the split preserved behaviour is that the external
`crate::check::*` surface is unchanged. Before Phase 1, snapshot it:

```sh
grep -rhoE "check::[a-zA-Z_][a-zA-Z0-9_]*" src/repl.rs src/driver.rs src/ir.rs \
  src/backend/qbe.rs | sort -u > /tmp/check-surface-before.txt
```

At spec time this yields 35 lines (34 distinct names plus `check::tests`):
`audit_quotation_type_registries`, `audit_word_quotation_positions`, `check`,
`check_combinator_cycles`, `check_def`, `check_def_collecting_drop_sites`,
`check_drop_overload_reachability`, `check_exported_signatures`, `check_poly_body`,
`check_poly_combinator_repl`, `check_selective_imports`, `check_types`, `CombinatorEnv`,
`combinator_of`, `drop_overload_struct_id`, `effect_str`, `enum_generated_sigs`,
`find_drop_overloads`, `has_self_tail_call`, `infer_line`, `is_combinator`, `is_copy`,
`is_linear`, `Overload`, `PolyEnv`, `poly_type_str`, `selective_collides_with_local_error`,
`selective_collision_error`, `SelectiveName`, `selective_not_exported_error`, `sig_of`,
`struct_generated_sigs`, `word_declares_quotation_parameter`, `word_span` (+ `tests`).

This is the authoritative list, not the brief's prose approximation. Re-run the grep after
each phase and diff against the snapshot; it must stay empty. If a name would disappear from
the snapshot, the fix is a `pub(crate) use` re-export in `check.rs`, never a call-site edit.

Because the surface is entirely `pub(crate)` (not `pub`), do **not** promote anything to
`pub`. Each moved item gets the minimum visibility to satisfy its callers: `pub(super)` when
only `check.rs` and sibling submodules use it, re-exported by `check.rs` only when an
external module names it.

## Target module shape

`src/check.rs` becomes `mod` declarations + `pub(crate) use` re-exports only. Content moves
into one file per cluster (working names from brief section 3; the dependency star is what
matters, not the labels):

| File | Cluster | Approx lines | Depends on `engine`? |
|------|---------|--------------|----------------------|
| `builtins.rs` | `Sig`, `Overload`, `resolve_overload`, `BuiltinRow`/`BuiltinLower`, `builtin_table`, `*_types`, `is_copy`, `is_linear`, `contains_reference` | ~640 | no |
| `audits.rs` | `find_drop_overloads`, `drop_overload_struct_id`, quotation/poly-position audits | ~330 | no |
| `declarations.rs` | `check_extern_decls`, `check_exported_signatures`, `check_selective_imports` (+ `SelectiveName`), `check_types`, struct/enum checks, recursion checker, `*_generated_sigs` | ~1300 | no |
| `drop_graph.rs` | `check_drop_overload_reachability`, `check_main_effect`, tail-call cycle checks, `drop_reachability_graph`, `collect_drop_targets` | ~780 | no |
| `engine.rs` | `RegionId`, `Alias`, `Deriv`, `Provenance`, `Moves`, `Scope`/`Binding`, `Liveness` + helpers, `aliasing_origin`, `BlockEnd`, `Ctx`/`word_ctx` | ~1350 | is the hub |
| `word_entry.rs` | `check_word`, `check_reference_free_signature`, `check_terms_word`, `check_clause_word`, `check_clause_body` | ~400 | yes |
| `terms.rs` | `check_terms`, `check_terms_relaxed`, `check_term` (~1040-line fn) | ~1130 | yes |
| `poly.rs` | `PolyScope`, poly combinator/body/call checkers, `resolve_poly_overload`, `unify_poly_input`, `apply_subst`, ~25 `poly_*_error` builders | ~2180 | yes |
| `combinators.rs` | `collect_combinators`, cycle detection, `inline_combinator`, `check_poly_combinator_args`, `combinator_of`, `is_combinator`, `word_declares_quotation_parameter` | ~680 | yes |
| `captures.rs` | `classify_capture`, `check_capture_admission`, `materialize_quotation_at_boundary`, `ref_root_is_in_frame` + capture/borrow error builders | ~560 | yes |
| `operators.rs` | `OpDispatch`, `check_operator` + error builders | ~490 | yes |
| `word_families.rs` | `check_reference_word`, `check_access_word`, `check_str_word`, `check_array_word`, `check_owned_cell_word`, `check_struct_peek_word`, `check_struct_get_word`, `is_name_visible_to_module`, `scoped_operator_overloads`, `check_drop_import_visibility` | ~1110 | yes |

`pub fn check(module)` (the ~270-line top-level driver) and small shared helpers named on the
external surface but not owned by an obvious cluster (`effect_str`, `infer_line`, `word_span`,
`sig_of`, `check_def`, `check_def_collecting_drop_sites`, `check_poly_combinator_repl`) stay
directly in `check.rs` alongside the re-export shim, unless they read more naturally inside a
cluster, in which case `check.rs` re-exports them.

**Line ranges are stale recon.** The brief's ranges predate `ab14a9f`; `mod tests` has moved
from L11159 to L11321. Re-locate each cluster by symbol at extraction time (`documentSymbol`
or grep for the item name), not by copying line numbers.

## Resolved open questions

1. **`captures` vs `poly` independence.** Verify at Phase 10 (captures extraction) whether
   `captures` truly needs nothing from `poly`. Default: separate files. Both touch quotation
   capture, but `poly` handles stack-polymorphic *unification* and `captures` handles
   linear-region *admission at boundaries*; they are expected to share only `engine` types.
   If Phase 10's `cargo build` reveals a hard dependency from `captures` into `poly`
   internals, co-locate them in `poly.rs` rather than adding cross-cluster `pub(super)`
   leakage, and note the merge in the phase commit. Do not pre-merge on suspicion.

2. **`word_families` granularity.** One file (`word_families.rs`), not one file per checker.
   The `check_*_word` functions are the same shape (a dispatch-target per builtin word family)
   and change together; splitting them further contradicts CLAUDE.md's "keep code that changes
   together in the same place" and would produce ten sub-200-line files.

3. **Test relocation granularity.** Mechanical first pass, then hand-fix ambiguous cases.
   For each `#[test] fn` in the current flat `mod tests`, grep its body for the public
   function names of each destination cluster; move the test into the module of the cluster
   it references. When a test references functions from more than one cluster (e.g. a `terms`
   test that also drives `engine`'s `Scope`/`Moves` directly), tie-break to the **more
   specific** subject: the checker under test, not the engine primitive it happens to exercise.
   All test relocation happens in the final phase, as one pass, after all source clusters have
   moved (so every test's destination module already exists). Tests keep `use super::*`
   against their new parent module; a test that legitimately needs cross-module items imports
   them explicitly.

4. **Phase granularity.** **Strict one cluster per phase.** This matches the brief's settled
   "one cluster moved per phase" bullet, keeps each diff to a single reviewable/revertible
   code-motion, and lets each phase's `cargo build` isolate any accidental cross-cluster cycle
   to exactly the cluster that introduced it. The cost is more commits (13), which is cheap
   and desirable for pure code motion (trivially bisectable). The brief invited bundling the
   same-risk independent clusters and expressed no strong preference; strict one-per-phase is
   a valid answer that maximises safety, so I take it rather than trading it for a lower phase
   count.

## Per-phase checkpoint (every phase)

A phase is not done until, in order:

```sh
cargo build
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

all pass, **and** the surface diff is empty:

```sh
grep -rhoE "check::[a-zA-Z_][a-zA-Z0-9_]*" src/repl.rs src/driver.rs src/ir.rs \
  src/backend/qbe.rs | sort -u | diff - /tmp/check-surface-before.txt
```

Then one commit for that phase. Do not begin the next phase on a red tree. If a phase's
`cargo build` fails with a cyclic-dependency or unresolved-import error, that boundary was
wrong: pull the offending item back toward its actual dependency root (or co-locate the two
clusters) within the same phase rather than papering over it with a wider `pub`.

## Extraction mechanics (per source cluster)

1. Create `src/check/<cluster>.rs`.
2. Cut the cluster's items (types, fns, impls, and their private helpers) out of `check.rs`
   into it, verbatim. Do not edit bodies.
3. Add `mod <cluster>;` to `check.rs`.
4. Add `use` lines at the top of `<cluster>.rs` for what it needs: `use super::*;` for
   sibling/`engine` items and crate-level types (`ast`, `ir`, spans), plus explicit `std`/
   external imports the moved code used.
5. Set visibility: items used only within `check`'s module tree become `pub(super)`; items on
   the external surface (section-4 list) get a `pub(crate) use self::<cluster>::Name;`
   re-export line in `check.rs`.
6. Run the full checkpoint. Fix visibility until green; never touch call sites outside
   `check.rs`.

`super::*` glob imports are acceptable here (this is one module tree being reshaped, not a new
public API); prefer them over hand-maintaining long import lists, and let `cargo fmt` /
`clippy` flag genuinely unused ones.

## Delivery plan

Order follows the dependency star: the four `engine`-independent clusters first (no downstream
risk), then `engine` (every dependent cluster needs it in its new home before it can move),
then the seven `engine`-dependent clusters (independent of each other, any order), then test
relocation and a final full-crate green check.

- **Phase 1 — `builtins`.** Extract `builtins.rs`. Engine-independent leaf; `is_copy`,
  `is_linear`, `Overload`, `builtin_table`, `sig_of` are on the external surface, so re-export
  those.
- **Phase 2 — `audits`.** Extract `audits.rs`. `find_drop_overloads`,
  `drop_overload_struct_id`, `audit_quotation_type_registries`,
  `audit_word_quotation_positions` are external; re-export.
- **Phase 3 — `declarations`.** Extract `declarations.rs`. `check_types`,
  `check_exported_signatures`, `check_selective_imports`, `SelectiveName`, the three
  `selective_*_error` builders, `struct_generated_sigs`, `enum_generated_sigs` are external;
  re-export. Largest independent cluster.
- **Phase 4 — `drop_graph`.** Extract `drop_graph.rs`. `check_drop_overload_reachability`,
  `has_self_tail_call` are external; re-export. Watch for shared tail-call helpers also used
  by `combinators` (Phase 9): if so, they belong here and `combinators` imports them via
  `super`.
- **Phase 5 — `engine` (hard).** Extract `engine.rs`: the borrow/scope/liveness hub. Highest
  cross-reference count and the widest `pub(super)` surface (every dependent cluster threads
  these types). Nothing above (Phases 1-4) may depend on it; if `cargo build` says otherwise,
  the item was miscategorised — move it, don't widen visibility blindly.
- **Phase 6 — `word_entry`.** Extract `word_entry.rs` (dispatch layer into the term checker).
- **Phase 7 — `terms` (hard).** Extract `terms.rs`. Contains `check_term`, the single biggest
  function in the crate (~1040 lines); move it whole, unedited.
- **Phase 8 — `poly` (hard).** Extract `poly.rs`. Largest cluster (~2180 lines) with ~25
  `poly_*_error` builders. `check_poly_body`, `check_poly_combinator_repl` are external;
  re-export.
- **Phase 9 — `combinators`.** Extract `combinators.rs`. `check_combinator_cycles`,
  `combinator_of`, `is_combinator`, `word_declares_quotation_parameter`, `CombinatorEnv` are
  external; re-export.
- **Phase 10 — `captures`.** Extract `captures.rs`. Resolve open question 1 here: keep
  separate unless `cargo build` proves a hard `poly` dependency, in which case co-locate.
- **Phase 11 — `operators`.** Extract `operators.rs`.
- **Phase 12 — `word_families`.** Extract `word_families.rs` (one file for all `check_*_word`
  checkers).
- **Phase 13 — test relocation + final full-crate green.** Move each `#[test] fn` from the
  flat `mod tests` into its subject cluster's module per open question 3 (mechanical grep pass,
  hand-fix ambiguous, tie-break to the more specific subject). `check.rs` retains only tests
  for items that stayed in `check.rs` (the driver, shared helpers). Final gate: full checkpoint
  green **and** the surface diff empty, confirming `crate::check::*` is byte-identical to the
  pre-refactor snapshot with no external call site changed.

## Out of scope

- Any behavioural change to checking logic, error messages, or diagnostics.
- Splitting any other file (`ir.rs`, `repl.rs`).
- Promoting or restructuring the `crate::check` public surface (stays `pub(crate)`, same paths).
- Adding new tests: this is code motion; existing tests in their new homes are the proof.

## Exit criteria

- `src/check.rs` is a thin `mod` + `pub(crate) use` shim; the ~11.3k non-test lines live in
  `src/check/{builtins,audits,declarations,drop_graph,engine,word_entry,terms,poly,combinators,captures,operators,word_families}.rs`.
- Every relocated test lives in the module of the function it tests.
- `cargo build && cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- The surface grep diffs empty against `/tmp/check-surface-before.txt`: `crate::check::*`
  exposes exactly the pre-refactor names at the same paths, with zero changes outside
  `src/check.rs` and `src/check/`.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Extract builtins.rs (engine-independent leaf: Sig, Overload, resolve_overload, builtin_table, type predicates, is_copy, is_linear, contains_reference); re-export external names; full checkpoint green + surface diff empty; one commit" },
    { "phase": 2, "focus": "Extract audits.rs (find_drop_overloads, drop_overload_struct_id, quotation/poly-position audits); re-export external names; full checkpoint green + surface diff empty; one commit" },
    { "phase": 3, "focus": "Extract declarations.rs (extern/exported-signature/selective-import checks, SelectiveName + error builders, check_types, struct/enum + recursion checks, *_generated_sigs); re-export external names; full checkpoint green + surface diff empty; one commit" },
    { "phase": 4, "focus": "Extract drop_graph.rs (drop-overload reachability, main effect, tail-call cycle checks, drop_reachability_graph, collect_drop_targets); re-export external names; full checkpoint green + surface diff empty; one commit" },
    { "phase": 5, "focus": "Extract engine.rs, the shared borrow/scope/liveness hub (RegionId, Alias, Deriv, Provenance, Moves, Scope/Binding, Liveness helpers, aliasing_origin, BlockEnd, Ctx/word_ctx); verify nothing in phases 1-4 depends on it; full checkpoint green + surface diff empty; one commit", "difficulty": "hard" },
    { "phase": 6, "focus": "Extract word_entry.rs (check_word, check_reference_free_signature, check_terms_word, check_clause_word, check_clause_body dispatch layer); full checkpoint green + surface diff empty; one commit" },
    { "phase": 7, "focus": "Extract terms.rs (check_terms, check_terms_relaxed, and the ~1040-line check_term, the crate's biggest function) verbatim; full checkpoint green + surface diff empty; one commit", "difficulty": "hard" },
    { "phase": 8, "focus": "Extract poly.rs (PolyScope, poly combinator/body/call checkers, resolve_poly_overload, unify_poly_input, apply_subst, ~25 poly_*_error builders); re-export check_poly_body and check_poly_combinator_repl; full checkpoint green + surface diff empty; one commit", "difficulty": "hard" },
    { "phase": 9, "focus": "Extract combinators.rs (collect_combinators, cycle detection, inline_combinator, check_poly_combinator_args, combinator_of, is_combinator, word_declares_quotation_parameter, CombinatorEnv); re-export external names; full checkpoint green + surface diff empty; one commit" },
    { "phase": 10, "focus": "Extract captures.rs (classify_capture, check_capture_admission, materialize_quotation_at_boundary, ref_root_is_in_frame + capture/borrow error builders); keep separate from poly unless cargo build proves a hard dependency, then co-locate; full checkpoint green + surface diff empty; one commit" },
    { "phase": 11, "focus": "Extract operators.rs (OpDispatch, check_operator + error builders); full checkpoint green + surface diff empty; one commit" },
    { "phase": 12, "focus": "Extract word_families.rs (all check_*_word checkers, is_name_visible_to_module, scoped_operator_overloads, check_drop_import_visibility) in one file; full checkpoint green + surface diff empty; one commit" },
    { "phase": 13, "focus": "Relocate each #[test] fn from the flat mod tests into its subject cluster's module (mechanical grep pass, hand-fix ambiguous, tie-break to the more specific subject); final full-crate checkpoint green and surface grep byte-identical to the pre-refactor snapshot with zero external call-site changes; one commit" }
  ]
}
```
