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
**migrated** form confirmed to fail. All 30 mutations were killed. `engine.rs`'s `infer_src`
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

## Phase 3 (R3) — migrate the ir/backend-side bare-line unit-test harness

Nothing in production `src/` was deleted: `lower_line`, `parse_line` and `ast::Line` all
still exist, so every claim below was proved against the live mechanism. Lib tests
1700 → 1691 (twelve `#[test]` fns retired, three added).

### The gate, done first

`src/ir/test_helpers.rs` ends the phase with **zero** `parse_line` dependency: both the
`crate::parser::parse_line` and `crate::ast::Line` imports are gone. `line_terms` was
replaced in place by `body_terms`, which lexes `": probe ( -- ) {src} ;"`, runs
`crate::parser::parse` and returns `module.words.remove(0).body` — the same module-parse
recipe phase 2 used for `engine.rs`'s `releasable_into` migration. No `check` call, since
both consumers want a term list, not a typed one.

`grep -rn 'parse_line\|ast::Line\|Line::Expr\|line_terms\|lower_line' src/` now returns,
outside `repl.rs`/`parser.rs`, only phase 8's own targets: `ir/driver.rs:513` (the `pub fn`),
`ir.rs:49` (the `pub use`) and two doc-comment mentions (`ir/driver.rs:2`,
`ir/func_builder/mod.rs:1112`).

### A count correction

The spec says `line_terms` has "9 call sites across 8 test fns" in `ir/driver.rs`. It is
**9 sites across 9 fns**, one apiece. The eleven-consumer total (9 + `calls.rs` ×2) is right.

### Migrated, with the per-test mutation result

| Test | Migration | Mutation applied | Result |
| --- | --- | --- | --- |
| `quotation_literal_emits_no_instr_and_records_body` (`func_builder/calls.rs`) | `line_terms` → `body_terms`; the test drives `lower_term` directly and never wanted a `Line` | `lower_term`'s `Quotation` arm stops inserting into `quot_bodies` | KILLED |
| `self_tail_combinator_saves_and_restores_loop_state` (`func_builder/calls.rs`) | same | `restore_loop_state` stops restoring `alloca_home` (the mutation the test's own comment names) | KILLED |
| `lower_call_uses_resolved_generation_symbol` (`ir/driver.rs`) | off `lower_line` onto `ir::lower` — see below | the emit-loop mints the monomorph symbol with `None` instead of `inst.generation` | KILLED |
| `word_str_slot_keeps_its_own_ir_type` (`ir/driver.rs`, new) | replaces `lower_line_carried_str_slot_keeps_its_own_ir_type`'s shared half | `ir_type_of`'s `Type::Str` arm → `IrType::Ptr` | KILLED |
| `emit_comparison_of_word_width_operands_uses_an_l_suffixed_compare` (`backend/qbe.rs`) | replaces `emit_comparison_line_stores_bool_via_extension`'s compare half, over `emit_src` | `Instr::Cmp`'s width suffix follows the *result* type instead of the operand | KILLED |
| `emit_scalar_store_and_load_follow_the_value_type_not_the_slot` (`backend/qbe.rs`) | replaces `emit_float_slot_round_trips_with_float_load_store` **and** `emit_line_wrapper_has_load_and_store` | `Instr::Store`'s float arm emits `storel` | KILLED |
| " | " | `Instr::Load`'s default arm loads at `w` width | KILLED (**after a fix — see below**) |

**`lower_call_uses_resolved_generation_symbol` is a stronger witness after the migration,
and the spec's premise for keeping it was wrong.** It did *not* witness
`ast::Instantiation::generation` at all: it passed a `|name| format!("{name}__gen2")`
closure as the `Resolver` and asserted the emitted `Instr::Call` named `sq__gen2`, which
witnesses only that `lower_line` consults its `Resolver` — a fact
`extern_call_lowers_to_a_call_with_the_declared_symbol` already covers off the line path.
The migrated form checks a one-generic-word module, sets each `inst.generation` to `Some(2)`
and re-mints `inst.symbol` exactly as the REPL does, then asserts the emitted monomorph's
`IrFunc.name` and `main`'s call symbol agree and both end in `__gen2`. That is a real
unit-level witness for `driver.rs:322` reading `inst.generation`, which is what phase 9
deletes. **The name is deliberately unchanged** so phase 9 finds the test where its focus
text says it is. Post-`check` field mutation follows `Probe::with_overrides`' existing
precedent in `ir/test_helpers.rs`.

### Retired as coverage of the session stack-marshalling protocol (12 tests + 1 helper)

Classified against what each **asserts**, not its name. `ir/driver.rs`, eight:

| Test | Why it is retired, not migrated |
| --- | --- |
| `lower_line_marshals_all_inputs_and_outputs` | D buffer loads / M buffer stores. The protocol itself. |
| `lower_line_returns_advanced_top` | The wrapper's `Ret(top + delta)` contract. |
| `lower_line_scalar_only_uses_eight_byte_cells_and_no_blit` | The `PtrOffset` cadence `[0, 8, 0]` into the session buffer. |
| `lower_line_struct_slot_blits_in_and_out` | Carried-slot blit + `out_bytes == 16`. The shared "a struct aggregate is copied by `Alloc` + `Blit`" fact keeps its witness in `lower_dup_of_struct_allocs_and_blits` (`func_builder/calls.rs`), over `lower_src`. |
| `lower_line_enum_slot_blits_in_and_out` | Same, for enums; twin witness `lower_dup_of_enum_allocs_and_blits`. |
| `lower_line_carried_float_slot_loads_as_float` | The prologue loading a buffer cell at `d` width. The shared mapping `f64 → IrType::Float { bits: 64 }` is witnessed by `ir_type_of_float_widths_expected` (`ir/types.rs`). |
| `lower_line_carried_narrow_slot_relabels_after_load` | The prologue's `Conv` relabel of an `l`-width buffer load — a step that exists only because the buffer cell is 8 bytes wide. The shared mapping `u8 → IrType::Int { bits: 8, signed: false }` is witnessed by `ir_type_of_each_width_expected`. |
| `lower_line_carried_str_slot_keeps_its_own_ir_type` | The prologue's `_`-arm bug. **The only one of the eight whose shared fact had no surviving witness**: `ir_type_of` has no positive `Type::Str` case (`ir_type_of_slice_is_a_two_word_aggregate_not_a_pointer` only asserts `assert_ne!(ir, IrType::Str)`). Re-expressed as `word_str_slot_keeps_its_own_ir_type` over `ir::lower`, above. |

`backend/qbe.rs`, three, plus the `emit_line` helper they were the only callers of:

| Test | Why |
| --- | --- |
| `emit_wrapper_signature_takes_stack_and_top` | `export function l $sooth_line_0(l %v0, l %v1)` — the wrapper signature is the protocol. |
| `emit_line_wrapper_has_load_and_store` | `loadl`/`storel` in the wrapper. Both mnemonics are re-expressed on their surviving producer in `emit_scalar_store_and_load_follow_the_value_type_not_the_slot`. |
| `emit_comparison_line_stores_bool_via_extension` | Three assertions, split: `=w csgtl` migrated (above); `extuw` + `storel` are the epilogue's widen-before-8-byte-store step, and die with the epilogue. `extuw` as a mnemonic keeps `emit_print_on_subword_unsigned_widens_via_extuw`, which is the `Instr::Conv` path. |

### One IL expectation changed, and why

`emit_comparison_line_stores_bool_via_extension` lowered `5 3 ugt` as a bare line with no
word environment. The migrated form is `emit_src(": w ( -- u32 ) 5 3 ugt ;")` — the declared
output is **`u32`, not `Bool`**, because off the line path the flag has to satisfy a
declaration and `ugt` leaves a 32-bit flag (`body leaves u32 where the declaration requires
Bool`). The asserted IL, `=w csgtl`, is byte-identical. The primitive `ugt` is kept rather
than `lib/`'s `gt` for the reason the original comment gives: `gt` would splice its own body.

### The inert assertion this phase caught

`emit_scalar_store_and_load_follow_the_value_type_not_the_slot`'s `loadl` assertion **passed
with `Instr::Load`'s width dispatch broken** on first writing: the fixture calls `.`, and
`$.2e.` reads string descriptors through `Instr::StrPtr`, whose `loadl` is hardcoded in the
emitter. The whole-module `il.contains("loadl ")` matched those. Fixed by scoping both
assertions to `$w`'s own body, the pattern `emit_print_of_cstr_uses_string_format` already
uses; the mutation is KILLED against the scoped form. This is the exact green-and-inert
shape the slice warns about, and it only showed up because the mutation was run.

### Finding for phase 8: two emitter arms lose their last producer

Not acted on here (production code is untouched this phase), and worth knowing before
phase 8 deletes `lower_line` rather than after. `Instr::Load` and `Instr::Store` have
exactly three producers between them once the line prologue/epilogue
(`ir/driver.rs:583`, `604`, `609`, `630`, `679`, `693`) goes:
`ir/func_builder/word_families.rs:79` (a `Ptr`), `:152` (an `OwnedCell`) and
`ir/func_builder/control_flow.rs:63`/`:65` (`total_order_key`: stores a float, loads an
unsigned integer). So after phase 8:

- `Instr::Load`'s float arms (`backend/qbe.rs:1378-1379`, `loads`/`loadd`) have **no
  producer**. `loadd` itself stays reachable through `field_load_op` (`:484`), pinned by
  `tests/qbe_baseline/shapes.ssa:23`.
- `Instr::Store`'s `w`-width widen-then-`storel` branch (`:1404-1410`) has **no producer**.
- `Instr::Store`'s float arms keep one (`total_order_key`), which is what the new test pins.

Phase 8 or 10 should decide whether those arms become `unreachable!` or stay as defensive
width dispatch; this phase does not pre-empt it.

## Phase 4 (R4 part a) — retire the two whole-file non-Phase-1 REPL suites

`tests/repl_ux.rs` (16 tests) deleted in full: its subject is interactive UX (prompt,
banner, `:words`, line editing), which has no non-REPL counterpart.
`tests/phase4_repl_imports.rs` (23 tests, Phase 4 slice 5b's exit-criterion set) also
deleted in full. The retired 5b exit criterion at
`docs/roadmap/P4-polymorphism-quotations.md:295` was updated in place with a one-line reason
and a pointer at its native replacement, rather than removed.

### Per-test classification of `tests/phase4_repl_imports.rs` (all 23)

**Migrated (4)** — a module-system fact with no `run`/`build` twin, now in
`tests/phase4_modules.rs`:

| retired test | native form |
| --- | --- |
| `repl_import_type_resolves_in_signature_and_typedef_position` | `imported_type_resolves_in_signature_and_typedef_position:231` |
| `repl_imported_nested_struct_ids_remap` | `nested_struct_ids_remap_when_a_local_type_declares_first:208` |
| `repl_transitive_reexport_stays_closed` | `imported_third_file_stays_closed_behind_a_reexporting_module:175` |
| `repl_selective_type_import_aliases_one_struct_id` | `selective_type_import_aliases_one_struct_id:247` |

**Deleted as already covered natively (11)** — line numbers in `tests/phase4_modules.rs`
unless stated:

| retired test | covering test |
| --- | --- |
| `repl_import_word_is_callable_qualified` | `two_files_word_import_compiles_and_runs:69` |
| `repl_import_type_accessor_resolves` | `imported_type_is_nameable_and_runs:83` |
| `repl_qualified_private_name_is_not_exported` | `unexported_word_is_not_exported_error:311`, `absent_word_in_module_is_unknown_not_unexported:338` |
| `repl_import_path_is_relative_to_cwd` | `import_path_is_relative_to_importing_file:291` (see caveat below) |
| `repl_import_cycle_and_missing_are_located` | `import_cycle_is_located_error_naming_both:114`, `missing_import_file_is_located_error:137` |
| `repl_malformed_import_is_located_error` | `malformed_import_form_is_located_parse_error:412` |
| `repl_import_of_library_declaring_main_is_rejected` | `src/driver.rs`'s `check_no_main_in_closure_rejects_imported_module_main:1240`, `build_rejects_imported_module_declaring_main:1269` |
| `repl_selective_import_exposes_unqualified` | `selective_import_exposes_names_unqualified:795` |
| `repl_selective_import_of_private_is_error` | `selective_import_of_private_name_is_error:810` |
| `repl_selective_import_collides_with_local` | `selective_import_colliding_with_local_word_is_error:850` |
| `repl_modules_dogfood_session_runs` | `modules_example_builds_and_runs:962` |

Caveat on the one partial match: the retired test's subject was resolution against the
*process cwd*, which only the REPL has (a bare line has no importing file). The native rule
is resolution against the importing file, and that is what the covering test pins; the
cwd-relative half retires with the REPL rather than being covered.

**Deleted as REPL-only session mechanism (7)** — each tests the session boundary itself
(import epochs, reload, qualifier rebind, session survival), which has no native analogue
because a native build resolves every import once and aborts on failure:
`repl_failed_import_leaves_session_intact`, `repl_reimport_freezes_existing_caller`,
`repl_reimport_of_type_leaves_unrelated_typedef_unaffected`,
`repl_reimport_of_type_resolution_does_not_diverge`,
`repl_qualifier_rebind_frozen_and_rejudged`,
`repl_selective_reimport_same_qualifier_reloads`,
`repl_dispose_of_imported_override_without_selective_import_is_unaffected` (its own comment
states the rule it pins is unreachable off `Ctx::Line`).

**Recorded as a gap (1):** `repl_double_colon_in_declared_name_is_located_rejection`, below.

### Mutation proof for the four migrated tests

Each mutation was applied to a copy of the tree and assessed with `cargo test
--no-fail-fast`, so the "also killed" column is corpus-wide, not one binary.

| mutation | migrated tests killed | also killed |
| --- | --- | --- |
| `struct_base.push(structs.len())` -> `push(0)` (`src/driver.rs:518`) | `nested_struct_ids_remap_when_a_local_type_declares_first`, `imported_type_resolves_in_signature_and_typedef_position` | `same_named_types_in_two_modules_coexist` |
| qualified word lookup falls through to any module that declares the name, after the target and re-export branches both miss (`src/resolve.rs`) | `imported_third_file_stays_closed_behind_a_reexporting_module` | `a_name_the_hub_does_not_export_is_unreachable_through_it` (`tests/phase8_slice2.rs:255`) |
| a qualified type name resolved against the current module instead of the import target (`ast::resolve_type_name_in_module`) | `imported_type_resolves_in_signature_and_typedef_position`, `selective_type_import_aliases_one_struct_id` | 8 others, incl. `imported_type_is_nameable_and_runs` and both `unexported_type_*` tests |

All four migrated tests are live: each fails under at least one mutation of the rule it
asserts. **None of the four is a sole witness**, though — every mutation above is also
caught by a pre-existing test, so the migration adds redundancy rather than closing a hole.
What each migrated fixture does add is the positive and negative half of one rule in a
single program (e.g. the re-exporter's own result crossing *and* the third file's name not
crossing), which no single pre-existing test pairs up.

- **A committed mutation regressed `struct_base` and was caught, then wrongly accepted as a
  fix.** A later commit on this branch (`9b24f99`, titled as a refactor) changed
  `struct_base.push(structs.len())` to `push(0)` in `assemble_module`
  (`src/driver.rs:518`), claiming it "syncs behavior with enum_base". `enum_base` pushes
  `enums.len()`, so the claim was false; `struct_base[m]` is module `m`'s offset into the
  merged struct registry, consumed at `src/driver.rs:759` to write parsed fields back into
  the right slots. The zero-base mutation makes every module write into module 0's slots.
  `nested_struct_ids_remap_when_a_local_type_declares_first`, one of the four tests migrated
  above, killed this mutation on contact (`recursive struct definition (infinite size)`,
  and a field-value mismatch in the sibling struct-collision test). The commit reworded the
  test's comment to match the broken behaviour instead of reverting the code. Reverted here
  (`e948ea0`). Note the mutation table above: the pre-existing
  `same_named_types_in_two_modules_coexist` also kills this mutation, so the migrated tests
  were not the only thing standing between the branch and a silent miscompile.
- **One fact from the retired suite has no native twin and is not fixed by this phase.**
  `repl_double_colon_in_declared_name_is_located_rejection` guarded a REPL-only rule: a
  declared name containing `::` is a located rejection, closing off the internal module-tag
  separator's forgeability. Measured on the native build path:
  - `type: q::T x i64 ;` and `: q::foo ( -- ) ;` both build clean; the declaration is
    silently accepted.
  - The declared name is nevertheless unreachable. With **no** import bound, calling
    `q::foo` fails with ``unknown word `q::foo` ``: a qualified call site is always routed
    through module resolution, never to a local declaration of that literal spelling.
  - With an import binding `q` to a library exporting `foo`, calling `q::foo` runs the
    **imported** word. The import wins and the local declaration is dead code, not the
    other way round.
  - A local `type: q::T x i64 ;` alongside an import that binds `q` is not a collision
    error either; it builds clean.
  So the shape of the gap is a silently-accepted declaration that can never be called,
  rather than a shadowing hazard. Recording it for whichever future phase owns
  name-resolution/declaration validation on the native path; implementing the rejection is
  out of this retirement phase's scope.
- **Spec item (d) defers to phase 6, as the spec anticipated.** `tests/common/mod.rs`'s
  `repl_core_import:64`, `repl_core_lines:74` and `REPL_CORE_ECHO:85` are *not* callerless
  after this phase: `tests/phase1.rs`, `tests/phase3_strings.rs`,
  `tests/phase4_combinators.rs` and `tests/phase4_slice10c_tail_splice.rs` still call them
  (four files, one more than the spec's three — `phase4_combinators.rs` was not on its
  list). Deleting them here would break the build, so they stay for phase 6 (R4 part c),
  which already owns their deletion.
- **Carry-forward for phase 6.** `tests/phase7_slice3v.rs:316`'s `run_session` helper
  documents itself as "`tests/repl_ux.rs`'s harness", now a dangling reference to a file
  this phase deleted. Phase 6's named-test list includes that file's
  `an_owning_cell_payload_of_a_plain_quotation_is_still_rejected`, but its
  locally-defined-spawn-helper list does not include `tests/phase7_slice3v.rs:316`; the
  helper needs deleting with its last caller there, and the stale comment goes with it.
- Retirement-note pointer fixed: the "rejected imported `main`" fact this note claimed was
  covered by `tests/phase4_modules.rs` actually lives in `src/driver.rs`'s own test module
  (`check_no_main_in_closure_rejects_imported_module_main:1240`,
  `build_rejects_imported_module_declaring_main:1269`); the note now points there.
- Full gate green after the revert: `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test --no-fail-fast` (0 failures, all binaries).
