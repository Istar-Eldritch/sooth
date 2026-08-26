# `check.rs` modularisation (brief)

Not a ROADMAP slice — internal structural hygiene, triggered by `check.rs` becoming hard
for both humans and agents to navigate (its own `module_report` outline exceeded the tool's
token cap while writing this brief). No behaviour change is in scope: every existing golden,
example, and unit test must still pass, unmodified in intent, after the split. Timing: do
this only after the current in-flight implementation work has merged to `main` (checked at
brief time: it has — `main` is at `ab14a9f`, no open `impl/*` worktree exists). Re-check this
condition before starting; if a new `impl/*` worktree exists, wait for it.

## Recon: measured against the built compiler

**1. `check.rs` is 16304 lines; 11158 of those are non-test code.** `#[cfg(test)] mod tests`
starts at line 11159 and runs to EOF. The non-test portion alone is bigger than most other
source files in the crate combined (`ir.rs` non-test: 5287; `repl.rs` non-test: 3242;
`parser.rs` non-test: 2235).

**2. There is a precedent for the target file shape already in the tree.** `src/backend.rs`
is 4 lines (`pub mod qbe;`) with the real content in `src/backend/qbe.rs`. Rust 2021 edition
needs no `mod.rs`; `src/check.rs` becomes the thin `mod` + re-export shim, `src/check/*.rs`
holds the split content, same pattern.

**3. The file is not a flat bag of unrelated functions — it is one shared stateful engine
plus several genuinely independent early passes.** Measured clusters, by line range in the
current file:

- **Independent (no dependency on the borrow/scope engine below), used only by the
     `check()` driver or by other clusters as leaves:**
  - `builtins`: `Sig`, `Overload`, `resolve_overload`, `BuiltinRow`/`BuiltinLower`,
       `builtin_table`, `numeric_types`/`int_types`/`float_types`/`printable_types`,
       `is_copy`, `is_linear`, `contains_reference` (L23–666, ~640 lines).
  - `audits`: `find_drop_overloads`, `drop_overload_struct_id` + its error builders,
       `audit_quotation_type_registries`, `audit_word_quotation_positions`,
       `reject_poly_quotation_anywhere`, `reject_quotation_type_position` (L1929–2254,
       ~330 lines).
  - `declarations`: `check_extern_decls`, `check_exported_signatures`,
       `check_selective_imports` (+ `SelectiveName` and its error builders), `check_types`,
       `check_no_stored_references`, `check_slice_element_gate`, `check_structs` (+
       duplicate-name/type/word checks), `check_generic_concrete_overlap`,
       `check_duplicate_poly_signatures`, the struct/enum recursion checker
       (`VisitState`/`TypeNode`/`check_recursion`/…), `struct_generated_sigs`,
       `enum_generated_sigs` (L2525–3825, ~1300 lines).
  - `drop_graph`: `check_drop_overload_reachability`, `check_main_effect`,
       `tail_position_calls`, `has_self_tail_call`, `check_tail_call_cycles`,
       `check_drop_overload_recursion`, `drop_reachability_graph`, `collect_drop_targets`,
       `all_calls`/`reaches_start` (L3825–4600, ~780 lines).
- **The shared engine — everything below depends on this, nothing above needs it:**
  - `engine` (working name): `RegionId`, `Alias`, `AliasSetId`, `DerivId`, `Deriv`,
       `Provenance`, `MoveState`/`Moves`, `Scope`/`Binding`, `Liveness` and its
       `live_*`/`capture_*`/`peek_region` helpers, `AliasOrigin`/`aliasing_origin`,
       `BlockEnd`, `Ctx`/`word_ctx` (L667–1928, ~1350 lines). This is the linear/borrow
       state every checker function below threads through its arguments.
- **Depend on `engine`, roughly independent of each other:**
  - `word_entry`: `check_word`, `check_reference_free_signature`, `check_terms_word`,
       `check_clause_word`, `check_clause_body` (L4600–5005, ~400 lines) — dispatch layer.
  - `poly`: `PolyScope`, `check_poly_combinator_standalone`, `check_poly_body`,
       `poly_walk`/`poly_term`/`poly_call_term`, `resolve_poly_overload`, `check_poly_call`,
       `unify_poly_input`, `apply_subst`, plus ~25 `poly_*_error` message builders
       (L5005–7181, ~2180 lines — the single largest cluster after the core term checker).
  - `combinators`: `collect_combinators`, `find_combinator_cycle`, `check_combinator_cycles`,
       `back_edge_outs`/`back_edge_declared_shape`, `inline_combinator`,
       `check_poly_combinator_args`, `check_literal_against_declared_effect`,
       `combinator_of`, `is_combinator`, `word_declares_quotation_parameter` (L7182–7864,
       ~680 lines).
  - `captures`: `classify_capture`, `check_capture_admission`,
       `materialize_quotation_at_boundary`, `ref_root_is_in_frame`, plus the capture/borrow
       error builders that precede them (L7864–8428, ~560 lines).
  - `terms`: `check_terms`, `check_terms_relaxed`, `check_term` — `check_term` alone is
       ~1040 lines, the single biggest function in the crate (L8428–9556, ~1130 lines). This
       is the actual per-term type/linearity checker; everything else is either setup for it
       or a specialised sub-checker it dispatches into.
  - `operators`: `OpDispatch`, `check_operator` (~290 lines) + its error builders
       (L9556–10046, ~490 lines).
  - `word_families`: `check_reference_word`, `check_access_word`, `check_str_word`,
       `check_array_word`, `check_owned_cell_word`, `check_struct_peek_word`,
       `check_struct_get_word`, `is_name_visible_to_module`, `scoped_operator_overloads`,
       `check_drop_import_visibility` (L10046–11158, ~1110 lines).
  - `pub fn check(module)` itself (L2255–2524, ~270 lines): the top-level driver that
       calls into every cluster above in sequence. This is the natural home for the
       re-exported top-level entry point, or it can stay directly in `check.rs`.

   Net shape: a dependency **star**, not a chain — `engine` is the hub every one of
   `word_entry`/`poly`/`combinators`/`captures`/`terms`/`operators`/`word_families` needs,
   and none of those seven need each other except through `engine`. `builtins`, `audits`,
   `declarations`, and `drop_graph` need nothing from `engine`. No cluster observed importing
   from a cluster that would need to import it back — no circular-dependency risk found, but
   this was traced by hand from call sites in one pass, not exhaustively; the spec's
   extraction phases should re-verify each boundary with `cargo build` at each step, which
   will simply fail loudly if a cycle exists.

**4. The crate's actual external `check::` surface is small and `pub(crate)`, not `pub`.**
Grepping every `check::X` reference from outside `check.rs` gives ~34 distinct names (`check`,
`check_def`, `check_def_collecting_drop_sites`, `check_poly_body`,
`check_poly_combinator_repl`, `check_types`, `check_selective_imports`,
`check_exported_signatures`, `check_drop_overload_reachability`, `builtin_table`, `is_copy`,
`is_linear`, `sig_of`, `Overload`, `CombinatorEnv`, `PolyEnv`, `combinator_of`,
`is_combinator`, `word_declares_quotation_parameter`, `infer_line`, `word_span`,
`effect_str`, `poly_type_str`, `has_self_tail_call`, `find_drop_overloads`,
`drop_overload_struct_id`, `struct_generated_sigs`, `enum_generated_sigs`,
`audit_quotation_type_registries`, `audit_word_quotation_positions`,
`selective_collision_error`, `selective_collides_with_local_error`,
`selective_not_exported_error`, `SelectiveName`), used from `repl.rs`, `driver.rs`, `ir.rs`,
and `backend/qbe.rs`. After the split, `src/check.rs` must still expose exactly this surface
at `crate::check::*` (via `pub(crate) use` re-exports from the submodules) so **no call site
outside `check.rs` needs to change**. This is the single concrete acceptance check for "did
the split preserve behaviour."

**5. The test module doesn't split cleanly along the same lines without checking first.**
`mod tests` (L11159–16304, ~5150 lines) is one flat block today. Per `CLAUDE.md`'s test
convention ("every stage function gets unit tests beside it"), tests should move with the
function they exercise into that function's new submodule, not stay as one 5000-line orphan
importing `use super::super::*` from a slim `check.rs`. This needs a real pass (not a guess)
to attribute each test to the cluster its subject function landed in — expect a many-to-one
relationship where some tests exercise multiple clusters at once (e.g. `terms` tests that
also exercise `engine`'s `Scope`/`Moves` directly) and have to pick the more specific home.

## What this brief treats as settled

- Target shape: `src/check.rs` (thin, `mod` declarations + `pub(crate) use` re-exports) +
  `src/check/{builtins,engine,audits,declarations,drop_graph,word_entry,poly,combinators,
  captures,terms,operators,word_families}.rs`, following the existing `backend.rs`/
  `backend/qbe.rs` precedent. Names above are working names; the spec may rename or merge
  adjacent small clusters (e.g. `audits` into `declarations`) if that reads better once the
  actual code is in front of the implementer — the boundary evidence in section 3 is what
  matters, not the label.
- No behaviour change: every existing test (in its new location) and every existing golden/
  example must still pass byte-for-byte on `cargo test`. This is a refactor, not a
  redesign — no new tests are required to prove the split beyond re-running what exists.
- Incremental, checkpointed extraction: one cluster moved per phase, `cargo build &&
  cargo fmt --check && cargo clippy -- -D warnings && cargo test` green before moving to the
  next, one commit per phase. Do not attempt this as a single diff.
- Extraction order follows the dependency star in section 3: the four `engine`-independent
  clusters (`builtins`, `audits`, `declarations`, `drop_graph`) first since they carry no
  risk of breaking anything downstream; `engine` next since everything else needs it in its
  new location before it can be extracted; then the seven `engine`-dependent clusters in any
  order (they don't depend on each other); test relocation and a final full-crate check last.
- No public API changes: `check::` stays `pub(crate)`, not promoted to `pub`, and the exact
  33-name surface in section 4 stays reachable at the same paths.

## Open questions for the spec

- **Exact cluster boundaries and file names** — section 3's clusters are evidence-grounded
  but the spec should verify a couple of the fuzzier ones by reading the actual code, not
  just this brief's line-range recon: in particular whether `captures` (L7864–8428) is truly
  independent of `poly` (L5005–7181) given both deal with quotation capture, and whether
  `word_families`' individual checkers (`check_reference_word` etc.) are similar enough in
  size/shape to justify one file vs. one file each.
- **Test relocation granularity** — move whole `#[test] fn` blocks by inspection of which
  cluster's functions they call, or do a mechanical first pass (grep each test body for which
  cluster's public fn names it references) and hand-fix ambiguous cases. The spec should pick
  one and say so; don't leave it to phase-time improvisation.
- **Phase granularity** — the settled order above implies roughly 6–9 phases (up to 4 for the
  independent clusters, 1 for `engine`, up to 7 for the dependent clusters if done
  individually, 1 for tests + final check). The spec should decide whether to bundle same-risk
  independent clusters into fewer, larger phases, or keep them one-per-phase for smaller
  diffs — no strong preference here, but state the choice explicitly.

## Out of scope

- Any behavioural change to checking logic, error messages, or diagnostics — pure code
  motion plus the minimum visibility (`pub(crate)`/`pub(super)`) changes required to compile.
- Splitting any other oversized file (`ir.rs`, `repl.rs`) — this brief is `check.rs` only.
- Renaming or restructuring the public `crate::check` surface — section 4's names stay put.

## Exit (sketch, spec settles the real one)

- `src/check.rs` is a thin `mod`/re-export shim; the ~11.2k lines of non-test logic live in
  `src/check/*.rs`, grouped by the clusters in section 3 (or the spec's refined version of
  them).
- Every relocated test lives beside the function it tests, in that function's new module.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green throughout, and
  `crate::check::*` still exposes exactly the section-4 surface with no external call site
  changed.
