# `check.rs` modularisation — technical spec (delivered)

Pure structural refactoring of the 16.6k-line `src/check.rs` into `src/check/*.rs`
submodules grouped by dependency cluster (Rust 2021, no `mod.rs`, following the
`backend.rs` → `backend/qbe.rs` precedent). Code motion plus minimum visibility
adjustments only; no behaviour change to checking logic, diagnostics, or error text.
All existing goldens, examples, and unit tests pass unmodified.

## Outcome

`src/check.rs` is the hub of a dependency star: `mod` declarations + `pub(crate) use`
re-exports that keep the exact `crate::check::*` surface reachable (no call site outside
`check.rs` changed), plus the code genuinely shared by two or more cluster files — per
CLAUDE.md's lowest-common-ancestor rule, shared code belongs at the LCA, not pushed into
one arbitrary consumer. It is not a fully-emptied "thin shim": ~1.9k non-test lines remain
by design (the `check()` driver, shared types like `Slot`/`PolyCtx`, and helpers/error
builders used by 2+ submodules). Everything used by exactly one submodule lives in that
submodule instead. The twelve cluster files hold everything else:

| File | Cluster | Engine-dep |
|------|---------|-----------|
| `builtins.rs` | `Sig`, `Overload`, `resolve_overload`, `builtin_table`, `*_types`, `is_copy`, `is_linear`, `contains_reference` | no |
| `audits.rs` | `find_drop_overloads`, `drop_overload_struct_id`, quotation/poly-position audits | no |
| `declarations.rs` | extern/exported-signature/selective-import checks (+ `SelectiveName`), `check_types`, struct/enum + recursion checks, `*_generated_sigs` | no |
| `drop_graph.rs` | `check_drop_overload_reachability`, `check_main_effect`, tail-call cycle checks, `drop_reachability_graph`, `collect_drop_targets` | no |
| `engine.rs` | `RegionId`, `Alias`, `Deriv`, `Provenance`, `Moves`, `Scope`/`Binding`, `Liveness` + helpers, `aliasing_origin`, `BlockEnd`, `Ctx`/`word_ctx` | hub |
| `word_entry.rs` | `check_word`, `check_reference_free_signature`, `check_terms_word`, `check_clause_word`, `check_clause_body` | yes |
| `terms.rs` | `check_terms`, `check_terms_relaxed`, `check_term` (~1040-line fn) | yes |
| `poly.rs` | `PolyScope`, poly combinator/body/call checkers, `resolve_poly_overload`, `unify_poly_input`, `apply_subst`, ~25 `poly_*_error` builders | yes |
| `combinators.rs` | `collect_combinators`, cycle detection, `inline_combinator`, `check_poly_combinator_args`, `combinator_of`, `is_combinator`, `word_declares_quotation_parameter`, `CombinatorEnv` | yes |
| `captures.rs` | `classify_capture`, `CaptureClass`, `check_capture_admission`, `materialize_quotation_at_boundary`, `ref_root_is_in_frame` + capture/borrow error builders | yes |
| `operators.rs` | `OpDispatch`, `check_operator` + operator-exclusive error builders | yes |
| `word_families.rs` | `check_*_word` checkers, `is_name_visible_to_module`, `scoped_operator_overloads`, `check_drop_import_visibility` | yes |

`pub fn check(module)` and shared helpers not owned by an obvious cluster (`effect_str`,
`infer_line`, `word_span`, `sig_of`, `check_def`, `check_def_collecting_drop_sites`,
`check_poly_combinator_repl`) stayed in `check.rs` alongside the re-export shim.

## Resolved open questions (as delivered)

1. **`captures` vs `poly`.** Kept as separate files; `cargo build` at Phase 10 proved no
   hard dependency into `poly` internals. They share only `engine` types.
2. **`word_families` granularity.** Single file for all `check_*_word` checkers (same
   shape, change together), not one file per checker.
3. **Test relocation.** All tests moved in the final phase into their subject cluster's
   module (grep each `#[test]` body for destination-cluster function names; tie-break to
   the more specific checker under test, not the engine primitive it exercises). Tests
   keep `use super::*`; cross-module needs are imported explicitly. `check.rs` retains
   only tests for items that stayed there.
4. **Phase granularity.** Strict one cluster per phase (13 phases, one commit each);
   maximises revertibility and isolates any accidental cross-cluster cycle.

## Behaviour-preservation proof

The authoritative test is that the external `crate::check::*` surface is byte-identical.
Snapshot before Phase 1 and diff after every phase (must stay empty):

```sh
grep -rhoE "check::[a-zA-Z_][a-zA-Z0-9_]*" src/repl.rs src/driver.rs src/ir.rs \
  src/backend/qbe.rs | sort -u > /tmp/check-surface-before.txt
```

35 lines (34 distinct names + `check::tests`):
`audit_quotation_type_registries`, `audit_word_quotation_positions`, `check`,
`check_combinator_cycles`, `check_def`, `check_def_collecting_drop_sites`,
`check_drop_overload_reachability`, `check_exported_signatures`, `check_poly_body`,
`check_poly_combinator_repl`, `check_selective_imports`, `check_types`, `CombinatorEnv`,
`combinator_of`, `drop_overload_struct_id`, `effect_str`, `enum_generated_sigs`,
`find_drop_overloads`, `has_self_tail_call`, `infer_line`, `is_combinator`, `is_copy`,
`is_linear`, `Overload`, `PolyEnv`, `poly_type_str`, `selective_collides_with_local_error`,
`selective_collision_error`, `SelectiveName`, `selective_not_exported_error`, `sig_of`,
`struct_generated_sigs`, `word_declares_quotation_parameter`, `word_span`.

No item was promoted from its pre-refactor visibility. `check_structs` was already `pub`
(not `pub(crate)`) before the refactor and is re-exported via a bare `pub use`, unchanged;
everything else on the moved/re-exported surface is `pub(crate)`. Moved items got the
minimum visibility (`pub(super)` when only `check.rs`/siblings use them; a `pub(crate) use`
re-export in `check.rs` when an external module names them). A name that would drop from
the snapshot was fixed with a re-export, never a call-site edit.

## Per-phase checkpoint (every phase, in order)

```sh
cargo build
cargo fmt --check
cargo clippy -- -D warnings
cargo test
# then the surface diff must be empty:
grep -rhoE "check::[a-zA-Z_][a-zA-Z0-9_]*" src/repl.rs src/driver.rs src/ir.rs \
  src/backend/qbe.rs | sort -u | diff - /tmp/check-surface-before.txt
```

One commit per phase; no next phase on a red tree. A cyclic-dependency or unresolved-import
build failure means the boundary was wrong: pull the offending item back to its dependency
root (or co-locate the two clusters) within the same phase, never paper over with a wider `pub`.

## Delivery record

Order followed the dependency star: four engine-independent clusters, then `engine` (the
hub), then the seven engine-dependent clusters (any order), then test relocation.

1. `builtins.rs` — `fa3d9614`
2. `audits.rs` — `218dc4d8`
3. `declarations.rs` — `51a8fd90` (+ `c37c4558` tightened selective-import error builders to private)
4. `drop_graph.rs` — `a8df7868`
5. `engine.rs` (hub) — `05981228`
6. `word_entry.rs` — `8d0d2a04`
7. `terms.rs` (incl. ~1040-line `check_term`, moved whole) — `c9c8ebe5`
8. `poly.rs` — `18d5704c`
9. `combinators.rs` — `6a5df9ba` (+ `abf00e60` restored `inline_combinator` doc comment, `95de0d9c` tightened `Combinator.terms` to private)
10. `captures.rs` — `a564e6ba` (+ `a01fab29` review feedback; kept separate from `poly`)
11. `operators.rs` — `b93ee732` (+ `7492c9c1` restored `operand_pair_mismatch_error` doc comment)
12. `word_families.rs` — `dc884f5c`
13. test relocation into each cluster module — `b0c4a610`
14. (review-round fix) relocated 4 single-consumer items + 6 private error builders to
    `terms.rs`, and `check_array_index` + 3 error builders to `word_families.rs` — `23ba13e`
15. (review-round fix) relocated 19 more single-consumer items to their sole-consuming
    cluster (12 to `word_families.rs`, 6 to `terms.rs`, 1 to `word_entry.rs`); restored a
    doc comment on the back-edge witness test dropped in step 14; this doc reconciliation

## Out of scope

- Any behavioural change to checking logic, error messages, or diagnostics.
- Splitting any other file (`ir.rs`, `repl.rs`).
- Promoting or restructuring the `crate::check` surface (stays `pub(crate)`, same paths).
- Adding new tests: existing tests in their new homes are the proof.

## Exit criteria (met)

- `src/check.rs` is the dependency-star hub: `mod` + `pub(crate) use` shim plus code shared
  by 2+ cluster files per the LCA rule; single-consumer code lives across the twelve
  cluster files instead.
- Every relocated test lives in the module of the function it tests.
- `cargo build && cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- Surface grep diffs empty against `/tmp/check-surface-before.txt`: `crate::check::*`
  exposes exactly the pre-refactor names at the same paths, with zero changes outside
  `src/check.rs` and `src/check/`.
