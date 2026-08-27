# P7.S9 — phase notes

Where this slice's phases record what their focus text says to "say in the phase report":
classifications, rulings, and findings a later phase must not rediscover. One section per
phase, added as each lands.

## Phase 1 (R1) — relocate `Library` into `src/driver.rs`

- **`compile_so` was not moved, and no motion was manufactured to satisfy prose.** It was
  already in `driver.rs` (`src/driver.rs:948` at HEAD), so the roadmap's "`compile_so`
  moves into `driver.rs`" is satisfied by construction.
- Relocated from `repl.rs` to `src/driver.rs:971-1032`, beside `compile_so`: the
  `RTLD_NOW`/`RTLD_GLOBAL` constants (both `cfg` arms), the `dlopen`/`dlsym`/`dlerror`
  extern block, `Library`/`open`/`symbol`, and `last_dlerror`. `fflush` stayed behind: it
  flushes the interactive prompt and dies with `repl.rs`.
- **Two doc comments were de-sessioned, not one.** The spec granted the rewrite for
  `Library`'s own doc ("The session keeps every handle resident … callable by later
  ones"). `open`'s doc said "objects loaded by later lines" — the same REPL-line prose, in
  a phrasing E2's grep cannot see, since it matches `session`/`repl` and not `line`. It now
  reads "objects loaded later". Leaving it would have handed phase 10 an unlisted edit.
- `library_opens_and_resolves_a_compiled_symbol` (`src/driver.rs:2494`) is the surviving
  witness. It also carries `repl.rs:3804`'s transmute-and-call (`sq(5) == 25`), so the
  "the resolved symbol is a usable fn pointer" fact does not die with `repl.rs` in phase
  5a; a non-null assertion alone would not have carried it. Mutation-proved in an isolated
  copy: deleting `symbol`'s null check fails the test on "a bad symbol name should error";
  returning `self.handle` in place of the resolved export segfaults.
- Behaviour-neutrality, proved while the REPL is still alive to prove it against:
  `--test repl_ux` 16/16, `--test symbol_hijack` 3/3, full suite green.

### Carried forward, for later phases

- **E2 is already unsatisfiable as worded; phase 11 should amend the criterion, not the
  code.** E2 expects the grep to return "only `src/driver.rs`'s relocated
  `dlopen`/`dlsym` extern declarations". Post-move, `driver.rs` legitimately says `dlopen`
  in five more places: `open`'s error message (`:1002`), `symbol`'s SAFETY comment
  (`:1010`), the pre-existing `TempDir` scratch-dir doc (`:1036`) and the round-trip test
  (`:2506`) — plus the call itself (`:999`). Only the `dlopen` line of the extern block
  matches at all: `dlsym` is not one of E2's alternatives, so the criterion's own prose
  names a string it never greps for. The residual set to expect at slice end is
  `driver.rs`'s `Library` plumbing plus `TempDir`'s doc, six hits; `driver.rs`'s other
  current hits (`:510`, `:890`, `:1264`, `:2021` prose, `:940`/`:943` `driver::repl`) die
  in phases 5a and 10 as scheduled.
- **`cargo clippy --all-targets -- -D warnings` is already red at HEAD**, in three places
  this slice does not own: `tests/phase4_combinators.rs:2499` (`507e0b7`) and `:2561`
  (`e4d43a2`), both `needless_borrow`, and `src/parser.rs:10351` (`1ccf370`),
  `bool_comparison`. CLAUDE.md's green is `clippy -- -D warnings`, which is clean, so this
  is not a regression and not this slice's to fix — but phases 2 through 4 edit tests
  heavily and will hit it if they widen the command.

## Phase 2 (R2) — migrate the check-side bare-line unit-test harness

Nothing in production `src/` was deleted: `Ctx::Line`, `infer_line` and `parse_line` all
still exist, so every claim below was proved against the live REPL. Lib tests 1708 → 1700
(the eight whole `#[test]` fns retired below).

### The shared migration target

`infer_probe_body` (`src/check/engine.rs`, `#[cfg(test)] pub(super)`) walks a source string
as the body of a synthetic one-word module and returns the residual typed stack. Elevated
to `engine.rs` because it needs `Ctx`/`word_ctx`/`check_terms`/`PolyCtx` and two sibling
test modules consume it; `pub(super)` in `check/engine.rs` reaches all of `check`. It is
the replacement for the "assert a *type* the walk produces" half of the three `infer_src`
copies. The other Ctx-only sites got `word_ctx` directly over a `probe_word()` `WordDef`
(`poly.rs`, `word_families.rs`) or the file's existing `bare_word` (`captures.rs`,
`engine.rs`).

`check::check`'s missing trait/impl pre-passes never bit: no migrated fixture carries a
`Bound::User`, and every migration stayed at the `check_terms`/`Ctx::Word`/helper level.
`parse_with_core` was not reached for anywhere.

### Retired as coverage of a mechanism that dies with the REPL (8 tests + 2 helpers + 1 row)

Classified against what each **asserts**, not its name.

| Test | Why it is retired, not migrated |
| --- | --- |
| `infer_line_rejects_a_quotation_left_on_the_residual` (`check.rs`) | Asserts the line-residual quotation error, which is *inside* `infer_line` (`check.rs:1399-1407`) and dies with it in phase 7. The word-exit twin (`check_outputs`' "leaves a quotation on the stack") is separately tested and untouched. |
| `infer_line_net_effect_expected` | A bare line's net effect. |
| `infer_line_carries_entry_depth` | The carried entry stack's depth. |
| `infer_line_carries_slot_types_expected` | The carried result *type*. |
| `line_underflow_against_carried_stack_is_error` | Asserts the `Ctx::Line` **arm's wording** ("stack underflow: needs 2 values, but the stack holds 1"). Checked before retiring, per the spec's instruction: the underflow *rule* has Word-path witnesses — `tests/phase7_slice3r.rs:588` pins the Word arm verbatim (`` `add` needs 2 values, but the stack holds 0``). `tests/phase4_combinators.rs:2283`/`:2288` pin the sibling `stack effect mismatch in \`w\`` diagnostic (declared-outputs and locals-exceed-inputs), not underflow, so they are not part of this witness. Only the Line arm's spelling is lost, and it dies with the arm. |
| `infer_line_unknown_word_is_error` | The `Ctx::Word` twin exists and asserts the same two substrings: `check_unknown_word_is_error` (`check.rs`, through `check_src`). |
| `infer_line_consumes_a_carried_linear_slot_ok` | "A residual linear slot can be dropped by a *later line*" — the session boundary itself. |
| `ctx_line_is_module_zero` (`engine.rs`) | Asserts `Ctx::Line`'s placeholder `module() == 0`. Dies with the variant. |
| the `Ctx::Line` half of `check_dup_of_drop_overload_type_names_the_cause` (`engine.rs`) | **Superset confirmed before deleting**, as instructed: the surviving `Ctx::Word` half pins three substrings — "cannot dup", "File is linear because it defines drop", and the negative "no bits to copy" — while the Line half pinned only the middle one. Deleted with its inline parse_line call, its "the Ctx::Line arm" comment, and the `struct_ty` test helper it was the last caller of. |
| the quotation audit's `is_line` row (`word_families.rs`) | The audited *site* is "end of a line". Row, `Row::is_line` field and the loop's `match is_line` all gone; the doc comment lost its `is_line` sentence and its "residual" family. The other 17 rows are untouched. |

The two orphaned helpers, `check.rs`'s `infer_src` (7 consumers, all above) and
`word_families.rs`'s `infer_src` (1 consumer, the audit row), went with their last callers.

### Migrated, with the per-test mutation result

Every migrated test was individually mutation-proved: the rule it guards was broken and the
**migrated** form confirmed to fail. All 27 mutations were killed. `engine.rs`'s `infer_src`
was inlined into its single consumer rather than kept as a two-line wrapper under a name
that no longer describes it.

| Test | Mutation applied | Result |
| --- | --- | --- |
| `operator_dispatch_resolves_the_exact_row_type` (engine) | `add` row output → `i64` | KILLED |
| " | comparison-primitive row output → `i64` | KILLED |
| " | `.` row output → `vec![ty]` | KILLED |
| `quotation_survives_dup_swap_and_bind` (engine) | `swap` clears the `quot` marker | KILLED |
| " | `Scope::bind` drops `slot.quot` | KILLED |
| `back_edge_index_map_is_bottom_aligned` (engine) | aligned position maps to `None` | KILLED |
| `releasable_into_withholds_a_name_used_in_a_back_edge_body` (engine) | drop `live.dead(..)` from the ancestor branch | KILLED |
| `check_capture_admission_gates_each_capture_kind` (captures) | drop the escaping frame-rooted rejection | KILLED |
| `past_owning_frame_names_owning_only_for_a_linear_capture` (captures) | always emit the `owning` remedy | KILLED |
| `check_capture_admission_rejects_captured_inline_quotation` (captures) | drop the `Type::InlineQuotation` arm | KILLED |
| " (the second, preserved witness) | `captured_inline_quotation_error` stops consulting `ctx` | KILLED |
| `dot_printable_set_slice_decision` (operators) | add the interned slice to `printable_types()` | KILLED |
| " | drop `found` from the Word-arm message | KILLED |
| `variant_whole_destructure_types_by_sig_dispatch` (word_families) | reverse a destructure's output order | KILLED |
| `zero_field_variant_destructures_to_nothing_and_mints_no_getter` (word_families) | skip zero-field variants in `variant_generated_sigs` | KILLED |
| `projection_on_variant_receiver_ok` (word_families) | drop the `resolved_variant_fields` insert | KILLED |
| `poly_term_admits_a_quotation_literal_as_a_marker_slot` (poly) | `PolySlot::quotation` drops the marker | KILLED |
| `poly_quotation_identity_moves_with_the_slot_under_swap` (poly) | same | KILLED |
| `poly_walk_arms_truncates_arm_locals_before_joining_moves` (poly) | drop the `moves.states` truncation | KILLED |
| `poly_walk_arms_rejects_an_arm_local_left_unconsumed` (poly) | drop the leak rejection | KILLED |
| `polyslot_int_val_folds_lits` (poly) | `IntLit` stops setting `int_val` | KILLED |
| `unify_poly_input_finding_a_seeded_variable_names_the_instantiation` (poly) | drop the `seeded` redirect | KILLED |
| `quotation_effect_unifies_and_binds_variable` (poly) | drop the quotation arity check | KILLED |
| `poly_is_copy_mutable_slice_is_not` (poly) | `PolyType::Concrete(Slice)` → always `Copy` | KILLED |
| `poly_len_over_a_slice_ok` (poly) | `len` stops consuming the slice slot | KILLED |
| `check_poly_slice_offset_admits_usize_and_literals_only` (poly) | admit a bare variable offset | KILLED |
| `poly_copy_gate_rejects_a_mutable_reference` (poly) | `PolyType::Ref` → always `Copy` | KILLED |
| `unify_poly_input_matches_a_declared_reference_slot` (poly) | drop the mutability check | KILLED |
| `apply_subst_grounds_a_reference_by_interning` (poly) | return the referent instead of interning | KILLED |
| `check_poly_array_index_bounds_checks_a_literal_and_requires_conversion_otherwise` (poly) | drop the literal bounds check | KILLED |

`call_variant_ref_word` (`word_families.rs`) is a migrated *helper*, not a test; its one
caller is `projection_on_variant_receiver_ok`, which is proved above.

### The special case, handled as the spec's corrected form requires

`check_capture_admission_rejects_captured_inline_quotation` ran the call twice. The
redundant second call and the four-line comment explaining why a second `Ctx` flavour was
needed are gone; **both** content assertions survive on the one surviving `Ctx::Word`
call — the ``"`~`"``/`"captured"` witness *and* the ``"`outer`"`` witness — and each was
mutation-proved separately (rows 10 and 11 above). The earlier draft's "collapse to a
single assertion" would have retired the first.

### Findings and one scope deviation

- **One doc comment moved forward from phase 7 to here.** `engine.rs:1580` (spec's line;
  now `engine.rs:1618` after this phase's own edits) read "R2: `Ctx::Word` carries its
  word's owning module; `Ctx::Line` denotes 0" and documented *both* tests. This phase
  deletes the second, so the clause named a test that no longer exists — the same
  reasoning the spec uses to assign `engine.rs:1745` to phase 2. It now reads "R2:
  `Ctx::Word` carries its word's owning module". Phase 7 will find this item already
  done; its other four (spec's `engine.rs:1129`/`:1300`, now `:1129`/`:1376` after this
  phase's `infer_probe_body` addition and helper dedup; `word_families.rs:1174`, `:1274`,
  unmoved) are untouched and still its. Re-grep at phase 7 start rather than trusting
  either set of numbers: this phase's own line count will have shifted again by then.
- **`quotation_survives_dup_swap_and_bind` is live, but three of its five iterations are
  inert, and that is pre-existing.** The assertion is
  `out.iter().any(|s| s.quot == marker)`. For `dup`, `over` and `rot` the *original* marked
  slot is still on the returned stack, so clearing the marker on the pushed copy survives
  the assertion — measured: the `dup`-arm mutation SURVIVED. The fixture is byte-identical
  to its pre-migration form, so this is not migration damage; it is recorded because it is
  exactly the green-and-inert shape this slice warns about, and because a future reader
  should not take the loop's five names as five witnesses. The test is genuinely live
  through `swap` and the bind half. Tightening it (assert the *moved* slot, by index)
  is out of this phase's scope.
- **Seven unit expectations changed from the Line arm's text to the Word arm's, and phase 7
  must not read them as goldens it may not touch.** `dot_printable_set_slice_decision` and
  six `poly.rs` tests asserted `Ctx::Line`-arm strings; under `Ctx::Word` they now assert
  the Word arm's — the enclosing word (`` `probe` ``), the line number and
  `note: declared ( -- )`. This is *stronger* pinning, and the text change is itself proof
  the `Ctx` is consulted rather than discarded. Phase 7's "no diagnostic text on the `Word`
  path may change" still holds over these: they are already on the Word path.
- **`check.rs:1348` is now the only `Ctx::Line` construction in all of `src/`.**
  `grep -rn 'Ctx::Line' src/ | grep -v '=>'` returns it plus four production doc comments
  (`engine.rs:1129`, now `:1376` after this phase's `infer_probe_body` addition and
  helper dedup; `word_families.rs:1174`, `:1274`, unmoved). Phase 7's premise holds as
  written.
- **Review round 1 collapsed five near-identical `WordDef` test constructors into one.**
  `probe_word`/`bare_word` were defined verbatim in `poly.rs`, `word_families.rs`,
  `engine.rs`, `captures.rs` and inlined a third time in `operators.rs` — CLAUDE.md's
  elevate-to-lowest-common-ancestor rule applies to test helpers too. Moved to
  `crate::test_support::bare_word(name, module)`, which `check/captures.rs`'s tests already
  depended on via `parse_with_core`. `probe_word()` in `poly.rs`/`word_families.rs` is now a
  one-line wrapper (`bare_word("probe", 0)`); `engine.rs`/`captures.rs` import the shared fn
  directly. This is what shifted `engine.rs`'s post-phase-2 line numbers a second time,
  noted above rather than left for phase 7 to rediscover.
- **`parse_line` in `src/` is down to what phases 3 and 8 own**: `ir/test_helpers.rs:8`/
  `:247` and `backend/qbe.rs:1458`/`:1479`/`:1900` (phase 3), plus `parser.rs`'s own
  definition and its `parse_line_src` test (phase 8). No check-side reference remains, and
  neither helper-bypassing site does — including `engine.rs:2152`, the general Liveness
  test, which was **migrated** to a module parse with every assertion intact (the `unused`
  control included) and is the sharp precondition for phase 8's `cargo test` compile.
