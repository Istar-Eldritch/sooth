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

## Phase 5 (R4 part b) — `tests/phase1.rs`

`tests/phase1.rs` is deleted in full. It was a whole-file REPL suite: all 49 tests ran
through its own `run_session` (`:11`) / `run_session_traced` (`:30`), both of which spawn
`sooth repl`, and it had no non-REPL member. Nothing in `src/` changed, so the REPL still
exists at the end of this phase; `--test phase1` no longer exists to spawn it.

**Nothing was migrated.** Every language fact underneath the suite already had a witness
off the REPL path, so all 49 are classified below as retired-mechanism or covered, each
covering test named. One migration *was* written and then reverted after measurement, and
that is recorded as a finding below rather than quietly dropped.

### The seven named criteria

Six are Phase 1 dogfood **Exit** criteria. The spec's rule is "rewrite against `sooth run`
over the same `examples/` source, unless an equivalent `run` golden already exists". For
all six it already exists over the *same file*, so each is a duplicate deletion. Verified
green as a set before deleting.

| retired test | covering `run` golden |
| --- | --- |
| `sign_definable_and_callable_in_repl` (`:168`) | `tests/phase0.rs::sign_compiles_and_runs` over `examples/sign.sth` (the same `0 gt ~[ 1 ] ~[ 0 ] if` word). The REPL form also called `sign` at a *positive* input, which `examples/sign.sth` does not; that both-arms fact is pinned natively by `tests/phase0.rs::eliminator_arm_containing_if_joins_correctly`, whose `0 gt ~[ 1 ] ~[ -1 ] if` runs at `5` and `-5` and asserts both results |
| `vectors_dogfood_runs_in_repl` (`:354`) | `tests/phase0.rs::vectors_dogfood_compiles_and_runs` over `examples/vectors.sth` (same `25` / `6`) |
| `shapes_dogfood_runs_full_program_in_repl` (`:459`) | `tests/phase0.rs::shapes_dogfood_compiles_and_runs` over `examples/shapes.sth` (same `12.5664` / `12` / `5` / `7`) |
| `stack_dogfood_runs_in_repl` (`:511`) | `tests/phase0.rs::stack_dogfood_compiles_and_runs` over `examples/stack.sth` (same `3` / `3` / `2` / `1` / `16`). The REPL form hand-rolled a `Popped` bundle struct because a REPL line cannot take a multi-output return; `examples/stack.sth` uses the real `( Stack -- Stack i64 )` ABI, so the covering golden is the *stronger* form of the same exercise |
| `self_tail_recursive_word_completes_in_constant_stack_in_repl` (`:603`) | `tests/phase0.rs::countdown_dogfood_runs_in_constant_stack` over `examples/countdown.sth`, whose `sum-to` is the same word at the same `0 1000000` for the same `500000500000` |
| `vm_dogfood_runs_in_repl` (`:635`) | `tests/phase0.rs::vm_dispatch_loop_runs_in_constant_stack` over `examples/vm.sth`, same N = 100_000, same `5000050000`. The REPL form flattened each definition onto one line; the golden runs the committed file |

**The `#[ignore]` sweep touched exactly these and nothing else.** Re-counted with attribute
lines only (`grep -nE '^[[:space:]]*#\[ignore' tests/phase1.rs`): **3**, at `:159`, `:600`,
`:632`, all on exit-criterion tests, as the spec says. Their notes cite a REPL-only gap
(`check.rs`'s two REPL check sites hardcode `TraitResolveCtx::scratch()`, so a session that
imports `core::cmp` ICEs). The gap does not exist off the REPL path and the three covering
goldens run green and un-ignored, which is the proof the criteria survive. Corpus-wide the
attribute count went 13 → 10: the 7 REPL notes phase 6 owns
(`phase4_combinators.rs` ×5, `phase3_strings.rs` ×1, `phase4_slice10c_tail_splice.rs` ×1)
plus `phase7_slice3b_follow.rs`'s 3 non-REPL notes, untouched.

**The seventh, `calculator_session_dogfood` (`:202`), is RETIRED, per the spec's ruling.**
Its subject is the interactive session: seven lines fed one at a time, asserting the
per-line `defined …` / `stack: …` echo after each. There is no `run` form of "a tiny
interactive calculator session". Its ordinary language facts, confirmed covered by named
`run` tests before deleting:

| fact | named covering test |
| --- | --- |
| a `\| n \|`-bound local in a word body | `tests/phase0.rs::gcd_compiles_and_runs` (`examples/gcd.sth`'s `\| a b \|`); `tests/phase3_locals.rs::mid_body_binding_consumes_from_the_stack`, `::mid_body_binding_leftmost_name_takes_deepest_value` |
| defining a word and calling it | `tests/phase0.rs::factorial_compiles_and_runs`, `::gcd_compiles_and_runs` |
| `mul` | `tests/phase0.rs::factorial_compiles_and_runs` |
| `add` | `tests/phase3_locals.rs::mid_body_binding_consumes_from_the_stack` (`a b add .` → `5`) |
| `sub` | `tests/phase0.rs::countdown_dogfood_runs_in_constant_stack` (`n 1 sub` a million times) |
| `swap` | `tests/phase4_generics.rs::core_shuffles_are_polymorphic_over_i64_bool_and_a_struct` |
| `.` | every golden above |

### The other 42, classified individually

Classified against what each **asserts**, not its name.

**Retired: the session boundary itself (16).** No native analogue exists, because a
whole-program build has no line boundary, no carried stack and no `:quit`.

| test | subject |
| --- | --- |
| `alloc_trace_stays_empty_in_a_session_that_never_allocates` (`:64`) | that each REPL `.so` carries its own allocator-shim copy and still prints no trace. Shared fact (trace silent with no allocation) is `tests/phase0.rs::alloc_trace_stays_empty_for_a_program_that_never_allocates` |
| `define_then_call_across_lines` (`:75`) | a definition on one line reachable from the next |
| `stack_persists_across_lines` (`:82`) | the session stack surviving a line |
| `redefinition_takes_effect_for_later_lines` (`:92`) | generation swap |
| `failed_redefinition_keeps_old_generation_resident` (`:135`) | a rejected redefinition leaving gen0 resident. Its diagnostic half (the stack-effect mismatch naming the word, "body leaves 2 values" / "declares 1 outputs") is pinned natively by `src/check.rs::check_declared_output_mismatch_is_error` (`:4181`); `tests/phase4_combinators.rs:2281`'s declared-outputs row only asserts the substring `` "stack effect mismatch in `w`" `` |
| `dot_output_interleaves_before_stack` (`:388`) | the flush-before-`stack:` discipline across the host's stdout and the loaded object's C stdio |
| `subword_carried_value_survives_line_boundary` (`:232`) | the carried `u8` cell staying 8 bytes and being relabelled on reload. Shared fact: `tests/phase0.rs::narrowing_conversion_truncates_and_widens_back_correctly` |
| `carried_float_survives_line_boundary_and_displays_as_float` (`:251`) | the carried `f64` marshalling (R20/R21). Shared facts: `tests/phase0.rs::float_arithmetic_runs_on_both_widths_end_to_end`, `::print_float_f64_and_f32_via_dot` |
| `carried_struct_survives_line_boundary` (`:264`) | the size-aware carried stack + the `<Vec2>` residual placeholder. Shared fact: `tests/phase0.rs::struct_flat_construct_get_destructure_native` |
| `carried_struct_and_scalar_offsets_stay_correct` (`:278`) | byte offsets into the carried buffer past a multi-cell slot |
| `carried_struct_with_non_eight_multiple_size_survives_line_boundary` (`:301`) | the carried cell count rounding up |
| `array_and_usize_cross_repl_line_boundary_and_render` (`:497`) | the carried array slot + `<[i64 4]>` render. Shared facts: `tests/phase0.rs::usize_arithmetic_comparison_and_conversion_native`, `::nested_array_shapes_construct_and_read_back_native` |
| `enum_large_payload_survives_line_boundary` (`:558`) | a three-`i64` payload blitted out of and back into the buffer |
| `enum_constructs_and_displays_placeholder_across_lines` (`:402`) | the `<Shape>` / `<MaybeInt>` residual placeholders and multi-cell slot sizing. Shared fact: `tests/phase0.rs::enum_crosses_word_call_boundary_with_scalar_underneath_native` |
| `enum_declared_then_eliminating_word_defined_on_later_lines` (`:433`) | R18's variant-set seeding from `Session.enums`, which exists only because the parser pre-pass sees one line at a time. Shared fact: `tests/phase0.rs::shapes_dogfood_compiles_and_runs` |
| `bool_residual_displays_as_true_or_false` (`:194`) | the `stack:` line's rendering of a residual `Bool`. `.`'s own `True`/`False` semantics: `tests/phase0.rs::leap_year_dogfood_compiles_and_runs` |

**Retired: `:quit` residual disposal (6).** "A live session can never prove you forgot to
dispose this" is the REPL relaxation these guard; a compiled `main` has the opposite rule.

| test | shared fact, and its native witness |
| --- | --- |
| `repl_quit_disposes_residual_linear` (`:682`) | top-first disposal order at session end. Ordering of drops generally: `tests/phase0.rs::drop_of_linear_struct_runs_field_glue_in_declaration_order` |
| `repl_explicit_drop_not_redisposed_at_quit` (`:716`) | exactly-once across the boundary. `tests/phase0.rs::explicit_drop_runs_destructor_once` |
| `repl_quit_disposes_residual_linear_struct` (`:827`) | field-order disposal of a residual struct. `tests/phase0.rs::drop_of_linear_struct_runs_field_glue_in_declaration_order` |
| `repl_quit_disposes_residual_linear_enum` (`:879`) | tag-dispatched disposal of a residual enum. `tests/phase0.rs::drop_of_linear_enum_dispatches_on_tag` |
| `repl_quit_frees_residual_owned` (`:908`) | `alloc 8` / `free 8` around a residual `^i64`. `tests/phase0.rs::owned_alloc_and_drop_traces_one_pair` |
| `repl_quit_frees_residual_recursive_value` (`:925`) | the fused disposal loop reached from `dispose_residual`. `tests/phase0.rs::recursive_list_disposes_in_expected_order`, `::deep_list_disposes_in_constant_stack` |

**Covered natively: linear discipline (5).** Each pairs one REPL-emission claim (the
synthesized destructor must be emitted into *every* REPL module: a bare line's, a
definition's, and `:quit`'s) with one ordinary language fact. The emission claim dies with
the multi-module REPL; the language fact is covered.

| test | covering test |
| --- | --- |
| `repl_within_one_line_create_and_drop_prints_once` (`:700`) | `tests/phase0.rs::explicit_drop_runs_destructor_once` |
| `repl_word_definition_keeps_strict_linear_rule` (`:733`) | `tests/phase0.rs::surplus_linear_on_stack_is_error` (same "linear value left on the stack" / `` `Spy` `` diagnostic); the "unknown word after a rejected definition" half is the session's rollback |
| `repl_bare_line_drops_linear_struct` (`:779`) | `tests/phase0.rs::drop_of_linear_struct_runs_field_glue_in_declaration_order` |
| `repl_word_definition_drops_linear_struct` (`:803`) | same |
| `repl_word_definition_drops_linear_enum` (`:856`) | `tests/phase0.rs::drop_of_linear_enum_dispatches_on_tag` |

**Covered natively: diagnostics that report and leave the session intact (4).** The
"session survives" half is REPL; the diagnostic is covered.

| test | covering test |
| --- | --- |
| `bad_line_reports_and_session_survives` (`:107`) | `tests/phase4_combinators.rs`'s `unknown-word` row (`:2293`, pins ``unknown word `nosuchword` in `w` ``); `src/check.rs::check_unknown_word_is_error` |
| `type_error_line_reports_and_session_survives` (`:121`) | `tests/phase7_slice3g.rs::self_call_concrete_operand_mismatch_is_located_type_error` (`:185`) pins a mismatch naming both `` `i64` `` and `` `Bool` `` verbatim; `tests/phase4_combinators.rs`'s `type-mismatch` row. This test is also on phase 6's classify/delete list; it is not REPL-driving (it drives `check_err`, not a session), so phase 6 must keep it — carried forward explicitly here since nothing on that list says so |
| `struct_declaration_errors_report_and_session_survives` (`:324`) | duplicate: `src/check/declarations.rs::check_struct_duplicate_type_name_is_error`, `tests/phase5_slice1.rs::generic_header_colliding_with_a_concrete_type_is_a_duplicate`. Recursive: `src/check/declarations.rs::check_recursion_by_value_self_cycle_is_error` (same `type: Loop next Loop ;` fixture), `tests/phase7_slice3n.rs::concrete_generic_self_reference_resolves_and_reaches_recursion_check` |
| `enum_declaration_errors_report_and_session_survives` (`:571`) | duplicate: `src/check/declarations.rs::check_enum_duplicate_type_name_across_two_enums_is_error`, `tests/phase5_slice1.rs::generic_enum_header_colliding_with_a_concrete_type_is_a_duplicate`. Recursive: `src/check/declarations.rs::check_enum_direct_recursion_is_error_not_hang` (same `Loop`/`Wrap` shape) |

**Covered natively: polymorphic definition and instantiation (4).**

| test | covering test |
| --- | --- |
| `polymorphic_repl_definition_with_clean_body_is_accepted_not_rejected` (`:963`) | `tests/phase7_slice3k.rs::a_non_inline_generic_callee_is_monomorphized_once_per_reached_instantiation` builds the same empty-bodied `: id ( 'T -- 'T ) ;` |
| `polymorphic_repl_definition_with_ill_typed_body_is_the_real_x1_not_the_old_blanket_rejection` (`:973`) | `src/check/poly.rs::check_x7_dup_of_unbounded_variable_names_missing_copy_bound` (same `dup` of an unbounded `'T`, names `'T` and `Copy`); the message's exact shape is pinned verbatim on the same formatter at `src/check/poly.rs:13159`/`:13174` for the `!` and `@` operands. The negative assertions (`!out.contains("REPL")`, `!out.contains("declared ( -- )")`) guarded against a Phase-1 blanket rejection that no longer exists on any path |
| `polymorphic_repl_word_instantiates_at_two_different_types_across_lines` (`:1000`) | `tests/phase7_slice3k.rs::a_non_inline_generic_callee_is_monomorphized_once_per_reached_instantiation` instantiates `id` at `i64` and `str` in one build; `tests/phase4_generics.rs::copy_bounded_type_variable_word_runs_at_two_concrete_types` runs both instantiations |
| `polymorphic_repl_word_instantiated_at_linear_type_without_copy_bound_is_x2` (`:1071`) | `src/check/poly.rs::check_x5_copy_bound_on_linear_type_names_variable_type_and_reason`, which uses the byte-identical fixture (`{SPY}: idc ( 'T: Copy -- 'T ) ;\n: main ( -- ) 0 Spy idc drop ;`). See the finding below |

**Retired: REPL-only generation and instantiation-freezing mechanism (7).**

| test | subject |
| --- | --- |
| `polymorphic_repl_word_instantiated_twice_at_one_type_prints_both_values` (`:1016`) | the session's `exported_insts` dedup. Its native form is stronger and already exists: `tests/phase7_slice3k.rs::a_non_inline_generic_callee_is_monomorphized_once_per_reached_instantiation` asserts the exact monomorph symbol set, i.e. one `IrFunc` per reached type |
| `poly_instantiation_freezes_callee_arity_across_a_differing_redefinition` (`:1033`) | a poly word's instantiation resolving its callee against the defining-*line* snapshot |
| `poly_instantiation_freezes_callee_value_across_a_same_arity_redefinition` (`:1054`) | the same, witnessed by value |
| `polymorphic_repl_definition_resolving_to_two_outputs_is_a_located_x3` (`:1095`) | the "resolves to N outputs, which is not yet supported at the REPL" deferral, whose message lives at `src/repl.rs:2895` and has no other producer |
| `redefined_polymorphic_word_freezes_earlier_call_while_new_call_rebinds` (`:1126`) | gen0/gen1 freezing across a redefinition |
| `ordinary_word_redefined_across_a_poly_definition_does_not_remint_the_old_symbol` (`:1175`) | the shared per-name generation counter, and `RTLD_GLOBAL` first-loaded-wins |
| `consolidated_exit_session_covers_define_instantiate_dedup_and_redefine` (`:1212`) | the consolidated session of the six above |

### Mutation proofs

No test was migrated, so E7's "every migrated test is proved live" is vacant here. In its
place, the three retirements whose covering test was least self-evident were proved by
mutation: break the rule, confirm the *named covering test* fails. Each mutation was
applied to a `git archive` copy of HEAD (never `cp -r` of the worktree, which would share
the real gitdir) and assessed corpus-wide with `cargo test --no-fail-fast`.

| mutation | named covering test | result | also killed |
| --- | --- | --- | --- |
| `drop_level_fields` iterates `.rev()` (`src/ir/func_builder/word_families.rs:709`) | `drop_of_linear_struct_runs_field_glue_in_declaration_order` | KILLED | 6 others, incl. `drop_of_nested_linear_struct_recurses_into_the_synthesized_destructor` and `an_owning_field_disposes_alongside_its_siblings_exactly_once` |
| every enum-destructor arm drops variant 0's fields (`emit_branch`, `layouts[vi]` → `layouts[0]`) | `drop_of_linear_enum_dispatches_on_tag` | KILLED | 27 others, incl. `recursive_list_disposes_in_expected_order` (the twin named for `repl_quit_frees_residual_recursive_value`) and `enum_variant_with_owned_frees_on_drop` |
| `visit_recursion`'s `InProgress` arm stops reporting a cycle whose repeated node is an enum (`src/check/declarations.rs:1670`) | `check_enum_direct_recursion_is_error_not_hang` | KILLED (stack overflow, SIGABRT: the exact failure its name guards) | none; it is the sole witness for the enum half of the cycle check |

### Findings

- **A migration was written, measured, and reverted.** Grepping for the *message text* of
  `poly_copy_bound_error` (`src/check/poly.rs:8285`) found no test outside `phase1.rs`, so
  `polymorphic_repl_word_instantiated_at_linear_type_without_copy_bound_is_x2` was migrated
  into `tests/phase4_generics.rs` beside its positive twin. The mutation that was supposed
  to prove it (both `Bound::Copy` arms at `:2199`/`:5149` collapsed to `None`) killed it —
  **and also killed `check::poly::tests::check_x5_copy_bound_on_linear_type_names_variable_type_and_reason`**,
  a pre-existing unit test with a byte-identical fixture. The grep missed it because its
  three assertions (`'T`, `Spy`, `linear`) are all *looser* than the message. The new test
  was reverted: it duplicated an existing witness on the same `check_src` path and closed
  no hole. **Method note for later phases: grepping a diagnostic's format string does not
  find the tests that guard it, because a test may assert substrings the format string
  spans. Mutate, or grep the fixture shape.**
- **Two message *shapes* the deleted tests pinned have no surviving verbatim witness.**
  "Covered natively" in the poly table above is true of the *rule* and looser about the
  *text*, and this phase applied a stricter standard to
  `tests/phase0.rs::surplus_linear_on_stack_is_error` (whose two carried lines it pinned).
  Neither gap is one this phase opened, so neither is a code change here:
  - `poly_copy_body_error`'s `dup` op label (`src/check/poly.rs:7619`).
    `polymorphic_repl_definition_with_ill_typed_body_is_the_real_x1_not_the_old_blanket_rejection`
    pinned `` cannot `dup` the type variable `'T` `` and `` `twice` `` verbatim; the
    covering `check_x7_dup_of_unbounded_variable_names_missing_copy_bound` asserts only
    `'T` and `Copy`. ``grep -rn 'cannot `dup` the type variable' src/ tests/`` is empty
    corpus-wide: that format string is pinned verbatim only at `:13159` and `:13174`, for
    the `!` and `@` operands. So the `{op}` slot has two witnesses, and `dup` is not one.
  - `poly_copy_bound_error`'s callee name and `Copy` clause (`:8285`).
    `polymorphic_repl_word_instantiated_at_linear_type_without_copy_bound_is_x2` asserted
    `id` and `Copy`; `check_x5_copy_bound_on_linear_type_names_variable_type_and_reason`
    asserts `'T`, `Spy` and `linear`, so it pins neither. The deleted assertions were on
    the `Ctx::Line` arm (`:8298`), which phase 7 deletes with `infer_line`, so nothing is
    carried forward for that arm. The surviving `Ctx::Word` arm's (`:8294`) looser pin
    pre-dates this slice. **Recommendation for phase 7**: while collapsing that match it
    will be editing this function anyway, so tighten `check_x5` onto the callee name and
    `Copy` then, rather than leaving the `Ctx::Word` text guarded by three substrings the
    format string spans — the same blind spot the finding above records.
- **`alloc_trace_stays_empty_for_a_program_that_never_allocates` is one-directional**, like
  the REPL test it replaces: it fails if the trace prints spuriously, but survives a
  mutation that disables the trace entirely. That direction is covered separately by
  `tests/phase0.rs::owned_alloc_and_drop_traces_one_pair`. Recorded because the pair, not
  either test alone, is the witness.
- **Corpus counts after this phase.** `#[ignore]` attributes 13 → 10 (the 7 REPL notes
  phase 6 owns, plus `phase7_slice3b_follow.rs`'s 3 non-REPL). REPL-driving test files
  16 → 15. `tests/common/mod.rs`'s `repl_core_import` / `repl_core_lines` /
  `REPL_CORE_ECHO` callers 4 → 3 (`phase3_strings.rs`, `phase4_combinators.rs`,
  `phase4_slice10c_tail_splice.rs`), exactly phase 6's stated set.
- Lib tests unchanged at 1691; `src/` untouched.

### Carried forward

- **Six doc comments now point at a deleted file.** `tests/phase3_locals.rs:12`,
  `phase3_refs.rs:90`, `phase3_resources.rs:11`, `phase3_strings.rs:16`,
  `phase4_generics.rs:14` and `phase4_slice10c_tail_splice.rs:111` each document their
  local `run_session` spawn helper as "mirroring `tests/phase1.rs`'s harness". Five of the
  six helpers are on phase 6's deletion list, so the dangling text dies with them.
  **`tests/phase3_refs.rs:91`'s `run_session` is not on that list** — the same omission
  phase 4 recorded for `tests/phase7_slice3v.rs:316`, and for the same reason: the file's
  named test (`times_def_hand_copy_is_pinned_to_the_library`) *is* listed but its helper is
  not. Phase 6 should delete both helpers with their last callers.
  `tests/phase3_resources.rs:4` also names `tests/phase1.rs` in a module-level provenance
  comment; it goes with that file's REPL surface in phase 6.
- **Twenty-two roadmap/doc files still name `tests/phase1.rs`**, not five as this bullet
  first recorded, and are phase 10's, not this phase's. `grep -rln 'tests/phase1\.rs'
  docs/` returns 24; two of those are this slice's own `P7/slice9-spec.md` and this file.
  Nineteen are out of scope by R8's own wording, which excludes
  `docs/roadmap/P{0..8}/slice*-{brief,spec}.md` and `docs/repl-ux-spec.md` by name:
  eighteen `slice*-{brief,spec}.md` files under `P2` (9), `P3` (6), `P4` (2) and `P7` (1),
  plus `docs/repl-ux-spec.md` (`:8`, `:63`, `:75`).
  **Three fall outside R8's literal enumeration, and phase 10 should rule on them
  explicitly rather than assume the exclusion reaches them**:
  `docs/roadmap/P1/spec.md` (`:65`, `:69`), Phase 1's own spec — the same historical class,
  but `P1/spec.md` is not matched by the `slice*` glob;
  `docs/roadmap/P3-linear-spine.md:259`, a *phase* file rather than a slice spec (the
  mention is a migration note inside slice 8c's "✅ done" record); and
  `docs/roadmap/operator-words-spec.md:33`, top-level in `docs/roadmap/`, whose
  `tests/phase1.rs:333` is a line reference that was already stale before this phase.
  All three were read: each sits in a past-tense record of landed work, so the *historical*
  conclusion holds for all 22. The earlier description of the set as "slice specs/briefs"
  did not — four of the 22 are not slice specs or briefs.
  `docs/repl-ux-spec.md` is the most stale of them: alongside `tests/phase1.rs` it cites
  `tests/phase4_repl_imports.rs` (F2 at `:8`, exit-criterion row at `:75`) and
  `tests/repl_ux.rs` (`:3`, `:63`, `:105`, `:107`), both retired by phase 4 (`c0bb5dd`).
  Every test file that spec names as a witness is now a deleted file.
- **`docs/roadmap/P1-repl-and-liveness.md`'s opening paragraph is unswept and is not on any
  phase's list.** It still states the removed design as current: "The REPL runs on the
  **backend** via `dlopen`: each new word is compiled to a shared object and loaded into
  the live session … redefinition loads a new object and swaps the name→symbol entry."
  This phase's scope is the file's two criterion lines (`:12-13`, `:14-15`), both updated.
  R8's stated-design sweep enumerates CLAUDE.md, DESIGN.md, ROADMAP.md,
  `P7-language-prereqs.md`, `docs/design/`, `P8` and `P12`, and this paragraph is the
  source of CLAUDE.md's load-bearing-invariant wording, so **phase 10 should sweep it in**
  rather than leave the invariant asserted in the phase file it came from.
- Full gate green: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test
  --no-fail-fast` (66 binaries, 0 failures).

## Phase 6 (R4 part c) — the remaining 14 single-surface test files, then the `#[ignore]` sweep

- **Three non-REPL exit-criterion tests were swept out of `tests/phase4_combinators.rs` by
  a contiguous deletion hunk, and restored in `2dcaecc`.** `combinators_dogfood_matches_
  hand_threaded` (`P4/slice6a-spec.md:276`, criterion 18), `filter_while_dogfood_matches_
  hand_threaded` (`P4/slice6b-spec.md:94`, criterion 16) and `combinator_in_times_dogfood_
  matches_hand_threaded` (R9's dogfood) sat inside the 380-line hunk that removed the
  file's `repl_*` block; all three drive `common::build_example` over a committed
  `examples/*.sth` and assert byte-identical stdout against a hand-threaded twin, with no
  REPL content at all. `tests/qbe_baseline.rs` pins those examples' IL text, not run-level
  combinator/hand equivalence, so nothing else carried the fact.
- **What hid it: group-level accounting.** "All 20 of `phase4_combinators.rs`'s `repl_*`
  tests" was true and still three short of what the file lost. The check that catches it is
  per-file, against git: `git show <commit> -- <file> | grep -c '^-fn '`, reconciled
  against the classification list one file at a time. Reconciled that way for every file
  this phase touched (helpers included, so each count is one or two above its test count):

  | File | Deleted `fn`s | Helpers | Tests | All REPL-driving? |
  | --- | --- | --- | --- | --- |
  | `phase3_locals.rs` | 6 | 1 | 5 | yes |
  | `phase3_refs.rs` | 2 | 1 | 1 | yes |
  | `phase3_resources.rs` | 10 | 1 | 9 | yes |
  | `phase3_strings.rs` | 9 | 1 | 8 | yes |
  | `phase4_combinators.rs` | 24 | 1 | 23 | **no — 20 of 23** |
  | `phase4_generics.rs` | 2 | 1 | 1 | yes |
  | `phase4_slice10c_tail_splice.rs` | 2 | 1 | 1 | yes |
  | `phase4_slice11_inline.rs` | 3 | 1 | 2 | yes |
  | `phase4_slice12_partd.rs` | 5 | 1 | 4 | yes |
  | `phase7_slice1.rs` | 2 | 1 | 1 (migrated) | yes |
  | `phase7_slice3g.rs` | 2 | 1 | 1 | yes |
  | `phase7_slice3i.rs` | 7 | 1 | 6 | yes |
  | `phase7_slice3t.rs` | 1 | 0 | 1 | yes |
  | `phase7_slice3v.rs` | 3 | 1 | 2 | yes |

  `phase4_combinators.rs` is the only file that lost a non-REPL test; the other thirteen
  reconcile exactly.
- **The spec's own "small REPL surface" framing does not hold for two of the fourteen
  files, and the per-file named-test lists in the phase focus text are not exhaustive
  inventories of each file's REPL tests.** `tests/phase4_combinators.rs` alone carried
  **20** `repl_error`-driving tests (re-grepped: `grep -c 'fn repl_' tests/phase4_combinators.rs`
  at phase start), not the seven the focus text names — the seven are exactly the
  `#[ignore]`-sweep set, not the file's whole REPL surface. `tests/phase7_slice3i.rs`
  carried **6** REPL tests under "R2: the REPL seeds `core::bool` itself", none of them the
  one test the focus text names for that file (`not_on_a_three_variant_enum_named_bool_is_
  an_error`, which is a plain `check_error`/`build_error` test with no REPL in it at all).
  Re-grepping `fn repl_|repl_error\(|repl_session\(|run_session\(|repl::run\(` per file,
  not trusting the focus text's per-file lists, is what surfaced both.
- **Classification rule applied throughout: a REPL test whose subject is session
  redefinition, generation/epoch freezing, cross-line splice hygiene, or the bare-line
  boundary itself is retired-mechanism by construction** — the same rule phase 5 applied to
  `tests/phase1.rs`. This covers the bulk of the corpus: all 20 of
  `tests/phase4_combinators.rs`'s `repl_*` tests (define-then-call-on-a-later-line,
  redefinition freezing, combinator/ordinary-word store eviction, cross-line cycle
  detection, import hygiene), 8 of `tests/phase3_resources.rs`'s 9 (destructor
  generation/epoch behaviour across session lines; the ninth is a covered-elsewhere case,
  below), all 6 of `tests/phase7_slice3i.rs`'s
  (session-only `core::bool` seeding with no package import), both of
  `tests/phase4_slice11_inline.rs`'s and all 4 of `tests/phase4_slice12_partd.rs`'s (the
  REPL's own splice-vs-lower retention gate, distinguished only by session redefinition
  freshness), both of `tests/phase7_slice3v.rs`'s (a session's non-PIC `.so` link limit),
  7 of `tests/phase3_strings.rs`'s 8 REPL tests (carried-slot marshalling across a REPL
  line boundary — phase 3 migrated the *lowering* half to `ir::lower`/`backend::qbe`
  fixtures, but not the `storel` mnemonic these carried, see the coverage-hole entry
  below), 5 of `tests/phase3_locals.rs`'s (REPL-line binding/frame-floor/transactionality facts),
  and one each in `tests/phase3_refs.rs` (a reference surviving a REPL line boundary) and
  `tests/phase4_generics.rs` (a quotation left on a REPL line, R19 — the REPL's "no
  declared outputs" boundary has no `build` analogue since every word declares an effect).
- **Two tests were retired as covered, not as mechanism-retired**, each with its covering
  native test named and verified present:
  - `tests/phase3_strings.rs::bool_print_dispatches_to_library_overload_same_line`'s own
    comment names `tests/phase0.rs::leap_year_dogfood_compiles_and_runs`; deleted rather
    than migrated since the fact it pins (`.` on `True`/`False` resolves through the
    library overload) is exercised there already.
  - `tests/phase3_resources.rs::repl_resource_field_is_disposed_through_the_overload` is
    **not** in the generation/epoch bucket its `repl_` prefix suggests: its own comment
    calls it "R7's ordinary composition", an enclosing struct disposing a resource field by
    calling that field's own destructor instead of inlining glue. Its native twin is
    `tests/phase0.rs:2136 drop_of_linear_struct_runs_field_glue_in_declaration_order`,
    where `SPY_DEF`'s user `: drop ( Spy -- )` overload printing `drop 1`/`drop 2` is the
    proof the glue calls the overload rather than inlining it. Only the REPL-specific half
    (the enclosing struct declared on a *later session line* than the override) dies:
    `build` assembles every declaration before synthesizing glue, so source order cannot
    reproduce it.
- **Two tests were embedded inside otherwise-surviving non-REPL test functions, not whole
  functions**, and needed surgical excision rather than whole-function deletion:
  `tests/phase4_combinators.rs::quotation_type_is_rejected_at_every_audited_position`'s
  `repl_rows`/`item1_rows` loops (the REPL's own `check_types`-only chokepoint and its
  session-bricking-registry-rollback regression) and
  `::stale_phase6_diagnostics_are_reworded`'s trailing REPL assertion (R19's residual-line
  rejection). Both retired as REPL-only in the same functions whose non-REPL `rows`/
  `checked_rows` loops survive unchanged.
- **`tests/phase7_slice3v.rs::an_owning_cell_payload_of_a_plain_quotation_is_still_
  rejected`, named in the focus text, is a plain `build_error` test with no REPL content**;
  the file's actual two REPL tests
  (`explicit_repl_override_epoch_disposal_is_blocked_by_the_repl_link_limit` and its
  control `a_plain_quotation_value_hits_the_same_repl_link_limit`) were unnamed in the
  focus text. Same pattern as `phase7_slice3g.rs`'s and `phase7_slice3i.rs`'s named tests
  (`self_call_concrete_operand_mismatch_is_located_type_error`,
  `not_on_a_three_variant_enum_named_bool_is_an_error`) and two more:
  `phase4_combinators.rs::combinator_and_hand_threaded_loops_agree_across_stack_limits`
  (`:1351`, a `build`-level equivalence witness for criterion 14) and
  `phase3_refs.rs::times_def_hand_copy_is_pinned_to_the_library` (`:79`, a
  `assert_pinned_to_combinators_lib` source-pin). Both survive untouched. So five of the
  focus text's named tests carry no REPL at all: not every name the focus text lists is
  itself REPL-driving, and not every REPL test in a file is named.
- **One migration, `tests/phase7_slice1.rs::repl_session_projects_struct_fields` →
  `projection_in_a_called_word_body_matches_an_inline_one`**: rewritten over `run_program`
  as an ordinary program binding a local and projecting through it, keeping the same
  read/write/getx/bump sequence and asserted output (`2\n9\n1\n2\n`). Mutation-proved:
  forcing `check_field_projection`'s field lookup (`src/check/word_families.rs`) to always
  resolve index 0 regardless of name breaks this test, confirmed, then reverted;
  `git diff --stat -- src/` empty before commit.

  **The migrated test is near-redundant, and its first name overclaimed.** That mutation
  kills its three siblings in the same file too, so it discriminates nothing they do not.
  Its only content the siblings lack is a projection written inside a *callee's* body
  against a `&`/`&!` parameter (`getx`/`bump`) — and even that has twins in
  `phase3_refs.rs:557` (`: bump ( &!Counter -- ) &!n 1 +! ;`) and `phase4_modules.rs:558`.
  It was named `projections_reach_every_lowering_path` after the criterion it came from,
  P7.S1's **R5, whose title is literally "the REPL path"** (`P7/slice1-spec.md:130`): the
  two paths R5 meant were `build` and the session, so with the REPL gone there is one path
  and the name asserted a coverage claim no test can hold. Renamed and its doc comment
  rewritten to state the callee-body fact instead. **R5 goes vacuous with the REPL** — a
  later phase should not read it as a live requirement needing a witness.
- **Coverage hole, measured not assumed: `Instr::Store`'s `_ => storel` arm
  (`src/backend/qbe.rs:1412`) now has exactly one witness left, and it is inside
  `repl.rs`.** Phase 3 measured this arm as killed only by REPL tests; this phase deletes
  the last of the *integration* ones, so the claim "already migrated to `ir::lower`
  fixtures" was wrong and is corrected above. Re-measured after this phase's deletions, in
  an isolated copy (`src tests lib examples Cargo.*`, baseline recorded first): mutating
  the arm to `stores` moves the suite from 2766 passed / 0 failed to 2765 / 1, and the one
  kill is `repl::tests::session_rich_rendering_shows_struct_contents_through_real_session`.
  Every integration test survives it. `emit_scalar_store_and_load_follow_the_value_type_
  not_the_slot`, which phase 3's report offered as the replacement, asserts `stored` and
  `loadl` and never `storel`; `tests/qbe_baseline/*.ssa`'s `storel`s come from
  `field_store_op`, a different function. **Phase 7 deletes that last witness with
  `repl.rs`**, leaving the arm uncovered — phase 8 must treat it as an uncovered width arm
  alongside the float/`w`-widen ones, not as covered.
- **One finding: `tests/phase7_slice3t.rs::explicit_instantiation_is_rejected_at_the_repl`
  cannot be migrated, contrary to its focus-text instruction.** Its guard
  (`error: explicit type instantiation is not available at the REPL`) lives only at
  `src/repl.rs:541`, inside `repl.rs`'s own `lower_instantiation` path; `grep -rn
  '"explicit type instantiation is not available'` confirms no second site. Its own doc
  comment's premise — "a session routes through `lower_instantiation` and skips the
  module-level checks this slice's correctness argument rests on" — names exactly why
  there is nothing to migrate it onto: `build` always assembles the whole-program impl
  registry the REPL cannot, so no non-REPL context reproduces the REPL's information
  deficit. The nearby `an_instantiation_inside_a_polymorphic_body_is_rejected` already
  covers the one context where `build` genuinely lacks enough information (a call checked
  symbolically inside a polymorphic word's own body, no `Subst` yet seeded) — recorded as
  the surviving member of the family in place of the unmigratable test.
- **`#[ignore]` sweep, re-grepped with `grep -rnE '^[[:space:]]*#\[ignore' tests/` (attribute
  lines only) after every deletion above: exactly `tests/phase7_slice3b_follow.rs`'s 3
  non-REPL notes (`:84`, `:736`, `:768`) remain.** The seven REPL notes named in the focus
  text (`tests/phase4_combinators.rs` ×5, `tests/phase3_strings.rs` ×1,
  `tests/phase4_slice10c_tail_splice.rs` ×1) all fell out of the retired-mechanism/covered
  classifications above with no separate sweep step needed — each of those seven tests was
  independently retired for its own reason (session redefinition, covered-by-native, or
  carried-slot marshalling respectively), which is the coincidence the focus text predicted
  ("the sweep adds no test beyond it").
- **`tests/common/mod.rs`'s `repl_core_import`/`repl_core_lines`/`REPL_CORE_ECHO`
  deleted**: their three remaining callers (phase 5's carried-forward count) all fell with
  this phase's deletions in `tests/phase3_strings.rs`, `tests/phase4_combinators.rs` and
  `tests/phase4_slice10c_tail_splice.rs`; re-grepped callerless before deleting.
- Exit witness, re-verified after `2dcaecc`'s restore: `grep -rn 'arg("repl")\|repl::run\|
  repl_core' tests/` empty. `grep -rnE '^[[:space:]]*#\[ignore' tests/` shows only the three
  non-REPL notes above. Full gate green: `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test --no-fail-fast` (0 failures). `cargo clippy --all-targets -- -D warnings` is
  red at HEAD (3 pre-existing errors, confirmed via `git stash`) and is not this phase's
  gate.

### Carried forward

- Phase 8's precondition re-grep (`parse_line\|ast::Line\|Line::Expr\|line_terms\|
  lower_line` in `src/`) is unaffected by this phase: nothing here touched `src/`.
- **Phase 8: `Instr::Store`'s `_ => storel` arm is uncovered once phase 7 lands**, per the
  measurement above. If phase 8 wants a witness for it, the cheapest one is an integration
  golden whose IL contains a scalar `Instr::Store` outside `field_store_op`; if it does not,
  the arm should be listed with the other unwitnessed width arms rather than assumed live.
- **P7.S1's R5 ("the REPL path") goes vacuous with the REPL.** Its witness has been
  repurposed, not preserved; do not schedule a replacement.
- The REPL still exists at the end of this phase (phase 7 deletes it). Every test file
  this phase touched still compiles and links against the live `sooth repl` binary having
  had only its REPL-driving tests and helpers removed.

---

## Phase 7 (R5a + R6) — delete the REPL, and flip `Ctx` to a struct

Commit: `5b8e68c`. One commit, as the spec requires: `clippy -D warnings` forces the
REPL's deletion, `infer_line`'s deletion and the `Ctx` flip into a single compile unit.

### R5a (a): the REPL surface

`src/repl.rs` and `src/editor.rs` deleted in full; `pub mod repl;` / `pub mod editor;`
out of `src/lib.rs`; `Some("repl") => driver::repl()` and the `usage()` subcommand line
out of `src/main.rs`; `driver::repl` out of `src/driver.rs`. E1 witness, run against the
rebuilt binary:

```text
$ ./target/debug/sooth repl
unknown command: repl

sooth — the Sooth compiler (bootstrap)

usage:
   sooth build <file.sth> [--manifest <path>]   compile to a native binary
   sooth run   <file.sth> [--manifest <path>]   compile and run
...
$ echo $?
2
```

### R5a (b): the forced cascade, which is larger than the spec's four items

The spec named four items that `dead_code` fires on the moment `repl.rs` dies:
`InferredLine`, `infer_line`, `check_poly_combinator_repl` and `Ctx`. That list is
incomplete. `cargo build` (no `cfg(test)`) enumerated the rest, and each was verified
REPL-only against `HEAD` before deletion (`git grep <name> HEAD -- src/` showing
`repl.rs` as the only non-definition caller):

| Deleted | Was reached from |
| --- | --- |
| `check::InferredLine`, `check::infer_line` (with its line-residual quotation error) | `repl.rs` |
| `check::check_def`, `check::check_def_collecting_drop_sites`, `check::ResolvedCalls` | `repl.rs` (`ResolvedCalls` fell with `check_def`) |
| `check::poly::check_poly_combinator_repl` | `repl.rs` |
| `check::drop_graph::check_drop_overload_reachability` | `repl.rs` |
| `check::combinators::CombinatorEnv::insert`, `combinator_of`, `word_declares_quotation_parameter` | `repl.rs:1411`/`:2958`/`:108`, `repl.rs:2017`/`:3067` |
| `ir::lower_word`, `ir::lower_instantiation` | `repl.rs` |
| `ir::layout::DropOverride` | `repl.rs` built its `AlreadyLoaded` entries |

Three items were **not** deleted but narrowed to `#[cfg(test)]`, because their only
surviving callers are unit-test harnesses and the alternative (deleting them and
open-coding the construction at each call site) would have been churn without a subject:
`driver::discover_closure` (the environment-configured wrapper; production reaches
`discover_closure_audited` directly with its own `ResolutionConfig`),
`check::poly::TraitCtx::scratch` and `TraitResolveCtx::scratch`. `ir::layout::empty_statics`
joined the existing `#[cfg(test)] pub(crate) use self::layout::{empty_slices, Refs}`
re-export, which is the pattern already established for `empty_slices`.

Two collateral effects worth naming:

- **`DropOverride` collapsed to `&WordDef`, not to a single-variant enum.** Deleting
  `AlreadyLoaded` left `DropOverride::Body(&WordDef)` alone in its enum, which is the exact
  shape the spec's own `Ctx` ruling rejects ("keeping a single-variant enum would keep
  match arms alive for no reason"). `DropOverrides<'a>` is now
  `HashMap<StructId, &'a WordDef>` and the six sites that named the type were updated.
- **An orphaned doc paragraph moved, it did not die.** The prose documenting
  `check::Overload` was attached to `check.rs`'s `ResolvedCalls` alias (a pre-existing
  misplacement — `Overload` lives in `check/builtins.rs:28` and carried no doc). Deleting
  the alias orphaned it and `clippy::empty_line_after_doc_comments` caught it. The
  paragraph was moved onto `struct Overload`, not deleted.

`tests/phase4_combinators.rs` and `tests/phase7_slice3r.rs` were touched only to correct
comments naming items this phase deleted; no test body or assertion changed.

### R6: `Ctx` is a struct

`enum Ctx<'a>` → `pub(super) struct Ctx<'a>` carrying the former `Ctx::Word` fields, which
are now `pub(super)` — the same visibility the enum's variant fields already had, so this
is not a widening. `word_ctx` builds it; `with_module` is a single `Ctx { module, ..*self }`
rebuild.

Method fates, as shipped:

| Method | Before | After |
| --- | --- | --- |
| `structs`, `enums` | shared match arm | field read |
| `static_type` | `Option<Type>` | **`Option<Type>`** (a name may genuinely not be a static) |
| `module` | `0` on `Line` | `u32`, infallible |
| `modules` | `None` on `Line` | **`Option<&[ModuleInfo]>`** — kept, see the finding below |
| `rendered_word_or(fallback)` | borrowed `fallback` on `Line` | **`rendered_word()`**, no parameter |
| `mangled_name` | `Option<&str>` | `&str` |
| `effect` | `Option<&StackEffect>` | `&StackEffect` |
| `declared_outputs` | `Option<&[TypedSlot]>` | `&[TypedSlot]` |
| `is_self_tail_call` | `false` on `Line` | `bool`, infallible |
| `generics` | `None` on `Line` | **`Option<&RefCell<GenericTypes>>`** (a real production `None`, below) |

**61 `Ctx::Line` match arms collapsed**, not 73: the spec's 73 counted
`check/engine.rs`'s own 12-arm method block, which this phase rewrote as field reads
rather than collapsed. Re-derived per file with `grep -rnE 'Ctx::Line[^=]*=>'` before the
edit: `check.rs` 23, `check/operators.rs` 12, `check/word_families.rs` 11,
`check/poly.rs` 11, `check/terms.rs` 4, `check/engine.rs` 12 (the method block),
`check/captures.rs` 0. 23+12+11+11+4 = 61 collapsed arms + 12 method-block arms = 73.

Each collapse kept the `Word` arm's body verbatim and replaced the pattern bindings with
accessor calls: `mangled` → `ctx.mangled_name()`, `effect` → `ctx.effect()`, and the
resulting `crate::resolve::render_word(ctx.mangled_name())` → `ctx.rendered_word()` (the
two are definitionally the same expression).

The four `mangled_name` consumers that compared against `Some(..)` shed the wrapper
(`check.rs`'s `is_own_drop_body`, `check/terms.rs:922`'s self-tail guard,
`check/terms.rs`'s `unwrap_or("the branch")` whose fallback is now dead, and
`check/poly.rs`'s self-call guard), as did `effect`'s and `declared_outputs`' one each.
`rendered_word_or`'s `fallback` died across **41** call sites, re-derived with
`grep -rn 'rendered_word_or(' src/`: `check/poly.rs` 39 (all `"<line>"`),
`check/captures.rs` 1 (`"<line>"`), `check/word_families.rs` 1 (`"this line"`).

**No diagnostic text on the surviving path changed.** Proved by diffing every changed
string literal: each `+ "error: …"` line in the diff has a byte-identical `- "error: …"`
twin (they are re-indentation moves out of the match arm), and every `-`-only literal is a
`Ctx::Line` arm's own text. `tests/qbe_baseline` is untouched — zero golden diffs.

One collapse defect was caught and fixed before commit: the scripted `effect` →
`ctx.effect()` substitution reached inside two string literals, turning
`"error: stack effect mismatch in …"` into `"error: stack ctx.effect() mismatch in …"`.
`tests/phase7_slice3r::impl_body_underflow_names_readable_member` failed on it, which is
the guard working. Both sites were reverted and the whole tree re-swept for the same
class (`grep` for the two accessor spellings inside quoted literals: zero remaining).

### R6 (d): doc prose naming a signature this phase changed

The labelled exception was applied to more than the six sites the spec listed:
`check/word_families.rs` ×2 (`Ctx::Line` prose on `intrinsic_is_gated_out` and
`drop_import_visibility_error`), `check/engine.rs` ×3 (the `modules`/`generics`/`mangled`
field docs, `infer_probe_body`'s `Ctx::Word` mention, `drop_res`'s), `check/poly.rs` ×1
(`Ctx::Word.effect`), `check.rs` ×2 and `test_support.rs` ×1 (`infer_line`),
`ir/destructors.rs`, `ir/driver.rs` and `ir/func_builder/mod.rs` ×2 (`lower_word`),
`check/word_entry.rs` (`word_declares_quotation_parameter`), plus the two test-side
mentions above. The test `ctx_word_carries_owning_module` was renamed
`ctx_carries_owning_module`: its name asserted a variant that no longer exists.

Three further sites were caught in review and fixed here, all of them (d) proper -- a
comment asserting a control-flow fact this phase falsified:

- `check/engine.rs`'s `BlockEnd` doc said the `Body` arm serves "a word body or REPL
  line". Only a word body reaches it now (`check/word_entry.rs:311` is the sole
  `BlockEnd::Body` construction).
- `check.rs`'s `in_word` doc said "empty for a bare REPL line". The empty branch died
  with `rendered_word_or`'s fallback; the body is now an unconditional `format!`.
- `check/poly.rs:489` said `check_poly_body`'s `modules` is "`Some` from `check::check`,
  `None` from `repl.rs`". Both halves are false: `repl.rs` is gone, and
  `grep -rn 'check_poly_body(' src/` shows exactly one caller (`check.rs:811`, passing
  `Some(modules)`), so no `None` reaches it at all. An earlier revision of this report
  claimed this line named a fact the phase did not change; that was wrong.

**Do not read the (d) application as exhaustive.** It is not, and an earlier revision of
this report claimed it was. Comments elsewhere in `src/` still assert the REPL as a live
path -- `check.rs:3114` and `check/word_families.rs:1198` ("`ctx.modules()` is `None` on
the REPL path"), the six `PolyCtx` field docs at `check.rs:120`-`:165` ("scratch on the
REPL paths"), `check/terms.rs:50` and `:518`, `check/poly.rs:4457` and `:5720`,
`check/operators.rs:360` and `:506`. They are left to phase 10, which derives its list
from E2's grep and will find them; they are named here so phase 10 does not have to
rediscover the reasoning, not to pre-empt its scope.

**Correction to the spec's (d) list.** The spec names `src/driver.rs:1103` and
`src/check/poly.rs:2046` as "two stale doc comments that describe `ctx.mangled_name()`'s
old `Option` return". They do not: neither mentions `Option` or `Some`. What both actually
do is contrast `ctx.mangled_name()` with `ctx.word_name()` — a method that does not exist
anywhere in the tree, at HEAD or before this phase. Both were corrected to name
`ctx.rendered_word()`, which is the demangled accessor the comments mean, but the spec's
stated reason for listing them was wrong and is recorded here rather than silently
satisfied.

### The `resolve` note, for a future slice

There was no REPL call site of `resolve::resolve_modules` to delete; the brief's claim
(`slice9-brief.md:57`, `:102`) is wrong and the spec already corrects it. The REPL's
`always_mangle: false` caller was `driver::assemble_module(&closure, false)` in
`repl.rs`, which died with (a). `resolve_modules`, its `always_mangle` parameter and its
single-module forcing are untouched. **After this phase `always_mangle` is
unreachable-but-retained**: every surviving non-test `assemble_module` caller passes
`true` (`driver.rs:890`, plus `backend/qbe.rs:1767` and `driver.rs`'s own unit tests),
and the `false` path is exercised only by `resolve.rs`'s four unit tests. Do not "tidy"
it in a later slice: the forcing it selects is what closes the QBE symbol-hijack class.
`cargo test --test symbol_hijack` is green (3 passed).

### Findings

- **`modules: Option` survives the ruling, but its stated justification did not.** The
  spec keeps `modules` optional because "a retained poly word still passes `None`" — a
  REPL mechanism. Measured, not assumed: an `assert!(modules.is_some())` in `word_ctx`,
  run over the whole suite in an isolated copy, fires **27** times and every one is in the
  lib's own unit-test binary; all 40 integration binaries pass. So after this phase
  `Ctx.modules` is `Some` on every production path, and `None` reaches only unit-test
  harnesses. `Option` is kept as the spec rules (collapsing it would arm
  `intrinsic_is_gated_out` and the drop import-visibility gate for those harnesses), and
  no behaviour changed here — production already always passed `Some`, the REPL's `None`
  came through `infer_line`, not `word_ctx`. But the ruling's *reason* is now false, and a
  later slice deciding this question should re-derive it rather than quote the spec.
- **`generics: Option` does keep a real production `None`**, unlike `modules`:
  `check_poly_combinator_standalone` (`check/poly.rs:376`) hardcodes `None`, and
  `check::check` reaches it for every `inline` polymorphic combinator. `static_type`
  trivially keeps its `Option`. Neither was collapsed.
- **Phase 6's carried-forward claim is confirmed, and worse than "uncovered":
  `Instr::Store`'s `_ => storel` arm (`src/backend/qbe.rs:1412`) was **unreached**.**
  Measured in an isolated copy after this phase's deletions (`src tests lib examples
  Cargo.*`, baseline recorded first, never a `cp -r` of the worktree): mutating `storel`
  → `stores` gives 2686 passed / 0 failed, and so does replacing the arm's body outright
  with `panic!` -- so no test in the suite compiled a program that reached it, rather
  than reaching it and not observing the result. The probe is sound rather than a stale
  binary: with the `panic!` in place, `./target/debug/sooth run` on a cell-local program
  does panic, while `examples/refs.sth` (whose cells are struct fields, blitted) builds
  clean. Its last witness was
  `repl::tests::session_rich_rendering_shows_struct_contents_through_real_session`, which
  died with `repl.rs` in this phase.

  **Re-witnessed here**, rather than handed on: an earlier revision of this report passed
  it to phase 8, which owns `parse_line`/`ast::Line`/`lower_line` and has no reason to
  open `backend/qbe.rs`. The new witness is
  `backend::qbe::tests::emit_scalar_store_of_a_word_wide_value_uses_storel`, beside the
  `stored`/`loadl` test that pins the arm's float sibling. It borrows a cell *local*
  (`: w ( -- i64 ) 7 ^ | c | &c &^ @ c ^> drop ;`): the cell pointer has no address of
  its own, so `&^` allocs a one-word place and stores the pointer into it through this
  arm. That producer is production code (`ir/func_builder/word_families.rs:152`) and
  survives phase 8; the arm's other feeder, `lower_line`'s residual store
  (`ir/driver.rs:675`/`:689`), does not.

  The assertion is pinned to the `alloc8` place, not to any `storel` in the body, and the
  first draft of it was a placebo that this caught: `^`'s own payload store emits
  `storel` from `store_op`'s **separate** width table (`qbe.rs:514`), so a
  `contains("storel")` form passed with the arm mutated. Re-measured after the fix,
  `storel` → `stores` gives 2686 passed / **1 failed**, killed by the new test and by
  nothing else -- confirming both that the witness works and that it was the only one.

- **`quotation_taking_word_mints_no_symbol`'s control now open-codes its predicate, and
  that is deliberate.** Deleting `word_declares_quotation_parameter` (REPL-only:
  `repl.rs:2017`, `:3067`) left that test's control asserting
  `!w.effect.inputs.iter().any(is_quotation_type)` inline. Review flagged it as drift
  risk. Left as it is: the open-coded form calls `crate::ast::is_quotation_type`, the
  live shared primitive, which has its own test
  (`ast.rs:2986 is_quotation_type_accepts_all_three_variants_only`, covering the inline
  `~` flavour the deleted `word_declares_quotation_parameter_recognizes_inline` covered),
  and the poly half is covered by
  `check/poly.rs:9589 poly_input_is_quotation_recognizes_inline` against the surviving
  `poly_input_is_quotation`. So no coverage was lost, and retaining a `#[cfg(test)]`
  copy of a predicate no production path consults would tie the control to a dead item
  rather than to the live rule.

### Deletion proof (E5, this phase's items)

`grep -rnE '<name>' src/ tests/` for each deleted item, after the commit. All zero except
one, which is a retirement note phase 6 wrote and not a live reference:

| Item | `src/` | `tests/` |
| --- | --- | --- |
| `infer_line`, `InferredLine`, `check_poly_combinator_repl`, `Ctx::Line`, `rendered_word_or` | 0 | 0 |
| `check_def`, `ResolvedCalls`, `check_drop_overload_reachability` | 0 | 0 |
| `combinator_of`, `word_declares_quotation_parameter` | 0 | 0 |
| `lower_word`, `DropOverride` | 0 | 0 |
| `lower_instantiation` | 0 | 1 — `tests/phase7_slice3t.rs:260`, phase 6's note recording why `explicit_instantiation_is_rejected_at_the_repl` was unmigratable |
| `editor.rs` | 0 | 0 |
| `repl.rs` | 1 — `backend/qbe.rs:303`'s `sooth_line_{seq}` note; `check/poly.rs:489`'s was the second and is corrected above | 4 — comment prose in three test retirement notes, which phase 10's E2 sweep owns |

`docs/roadmap/` hits for every one of these are in this slice's own spec/brief and in the
historical implementation specs the spec puts out of scope; none is a statement of current
design.

### Carried forward

- `parse_line`, `ast::Line` and `ir::lower_line` all still **exist** at the end of this
  phase, and that is correct: each is `pub` in a `pub` module, so nothing warns. Phase 8
  removes them. `grep -rn 'parse_line\|ast::Line\|Line::Expr\|line_terms\|lower_line' src/`
  is phase 8's precondition and this phase did not change its answer for the test tree.
- Phase 10's E2 sweep inherits **235 hits across 25 files** in `src/`, measured with E2's
  own command at the end of this phase:
  `grep -rniE 'repl|session|dlopen|override_epoch|drop_generation' src/ | grep -viE 'replac|replic'`
  (the spec measured 275 across 28 at spec time, on the same exclusion of `repl.rs` and
  `editor.rs`), plus the three test-side retirement notes listed above. Earlier revisions
  of this report said "six", which matched no measurement: the `repl.rs` literal is now 1,
  a case-insensitive `session` is 38, and the bare word `REPL` is 148. Much of the 235
  dies with phases 8/9's own deletions -- `is_repl` alone accounts for most of
  `parser.rs`'s 79 -- so phase 10 must still derive its list from a fresh grep rather than
  scope from this figure.
- Phase 11's split-signal re-run inherits four files that shrank here: `src/check.rs`
  (−236 lines), `src/check/engine.rs`, `src/check/poly.rs`, `src/parser.rs` (unchanged
  this phase; phase 8 shrinks it).

### Gate

`cargo fmt --check` clean, `cargo clippy -- -D warnings` clean,
`cargo test --no-fail-fast`: **2687 passed, 0 failed**, 3 ignored (the three non-REPL
`phase7_slice3b_follow.rs` notes). Zero `tests/qbe_baseline` diffs. 2687 rather than the
2686 this phase first reported: the review-fix round added the `storel` witness above.

## Phase 8 (R5b) — the bare-line entry-point family

Re-grepped every item before deleting, as the spec instructs, rather than trusting its
list. All matched: `parse_line`'s only non-test callers were gone with `repl.rs`;
`parse_line_with_structs`'s were `repl.rs:1418`/`:1694` only; `lower_line`'s was
`repl.rs:3414` only; `reject_generic_typedef_in_repl` had **two** call sites, not the
spec's one — `parser.rs:1353` (inside `parse_typedef_line`) and `:1436` (inside
`parse_enum_typedef_line`) — both inside the family, so the extra site changes nothing
about scope; `repl_unknown_capability_error` was called only from the two `is_repl`
readers. Deleted: `ast::Line`; `parse_line`, `parse_line_with_structs`;
`reject_generic_typedef_in_repl`; `repl_unknown_capability_error`; the `is_repl` field
with all 8 assignments and both readers; `ir::lower_line` and its `ir.rs` re-export; and
`Parser::declares_type` (its only caller was the deleted REPL `Ord` arm above) —
omitted from this section in the phase's first pass and recorded here on review.

### `parse_typedef_line` and its family are in scope by the spec's own range clause

`parse_typedef_line`, `parse_enum_typedef_line`, `typedef_line_is_enum` and
`enum_variant_names` are REPL-only entry points in exactly the sense R5b describes, and
all four construct `is_repl: true` and (the first two) call
`reject_generic_typedef_in_repl`. An earlier pass of this section called these
"unnamed" by the spec — wrong: the spec's R5b bullet reads "`parse_line` and
`parse_line_with_structs` (both `pub fn`) and *the rest of the family through `:1436`*",
and old `parser.rs:1436` is the `reject_generic_typedef_in_repl` call site *inside*
`parse_enum_typedef_line`. The range clause covers the family; only `ImportCtx` (the
import/selective/exports bundle threaded only to these four parsers and their tests) and
`Parser::declares_type` were truly unnamed by the spec. `grep -rn` for each, over `src/`
and `tests/`, found zero callers anywhere once `repl.rs` was gone — not even a test.
Leaving them would have meant either resurrecting `is_repl`/`reject_generic_typedef_in_repl`
for their sake (defeating the phase) or open-coding a stub, so all were deleted alongside
the named items.

### A second unnamed forcing: `carried_slot_bytes`

Deleting `lower_line` left `ir/layout.rs`'s `carried_slot_bytes` with no caller but its
own four unit tests. Unlike `parse_line`/`ast::Line`/`lower_line` — each `pub` in a `pub`
module, hence R5b's whole premise that `-D warnings` does not force this phase — `ir.rs`
re-exports it via `pub(crate) use`, not `pub use`, which is crate-visible only and *does*
trigger `dead_code` once its last caller dies (the same rule that made `InferredLine`
and `check_poly_combinator_repl` fire in phase 7). `cargo build --lib` confirmed this:
`carried_slot_bytes` warned the moment `lower_line` was gone. Deleted the function, its
`ir.rs` re-export, and its four REPL-carried-slot unit tests
(`carried_slot_bytes_scalar_is_eight_struct_is_aligned_aggregate`,
`carried_slot_bytes_array_is_aligned_aggregate`,
`carried_slot_bytes_slice_is_aligned_aggregate`,
`carried_slot_bytes_enum_is_aligned_aggregate`) — each tested only the carried-buffer
marshalling fact ("a slot this wide rounds up to an 8-byte cell"), which is REPL-only
subject matter, not a shared layout fact: `StructLayout`/`EnumLayout`/`ArrayLayout`'s own
`size`/`align` fields keep their existing non-REPL tests untouched. `round_up` stays; it
has five other production callers in `ir/layout.rs`.

### The parser unit tests

`parse_line_src` and its five tests were classified per test, not deleted as a block:

- `parse_line_float_lit_is_float_lit` tests a general parsing fact with no REPL content
  (a float literal token) that had no other witness in the tree (`grep -n 'FloatLit'`
  outside it found nothing test-level). **Migrated**, not deleted:
  `parse_float_lit_term_is_float_lit` re-expresses the fact over a one-word module body
  via `parse_src`/`terms_body`.
- `parse_line_bare_expression_is_expr` is equally REPL-free (a multi-term sequence) but
  was **not** a sole witness: `parse_mid_body_binding_produces_bind_term` already asserts
  a stronger form of the same shape (a 3-term body, `IntLit` at `[0]`, a trailing `Call`
  at `[2]`, and the `Bind` between them). It is therefore **retired as covered**; its
  migrated form `parse_bare_term_sequence_is_a_multi_term_body` is kept as redundant
  restatement, not as the fact's only guard. Both migrated tests were mutation-proved
  individually in an isolated edit-and-revert (not a worktree copy): relabelling
  `TermKind::IntLit(n)` to `IntLit(n + 1)` fails the migrated multi-term test;
  relabelling `FloatLit(v)` to `FloatLit(v + 1.0)` fails the migrated float test. Both
  reverted after.
- `parse_line_colon_is_def` tests the `Line::Def`/`Line::Expr` split itself, which is the
  REPL mechanism this phase deletes — **retired**, no twin needed.
- `parse_line_trailing_tokens_after_def_is_error` tests "one line is one complete unit",
  a rule with no file-parsing counterpart (a module is an unbounded sequence of
  definitions; trailing tokens after one `;` are simply the next definition) —
  **retired**.
- `parse_line_unterminated_def_is_error` pins "unexpected end of input … `;`" for an
  unterminated `:` body. `parse_missing_semicolon_is_error` (`parser.rs`, module-level)
  already pins the identical fact (`: w ( -- ) 1` with no closing `;`) — **retired as a
  duplicate**, named here as the covering test.

The four `parse_capabilities_at_repl_scope_*` tests (`_is_a_located_repl_specific_error`,
`_unknown_name_keeps_the_generic_error`, `_reports_ord_after_a_folded_predicate`,
`_yields_ord_to_a_declared_type`) pinned `is_repl`'s `Ord`-specific wording, which no
longer exists once `is_repl` and `repl_unknown_capability_error` are gone — **retired**.
The surviving non-REPL golden, `parse_capabilities_unknown_name_is_still_an_error`,
already pins the generic `unknown capability` wording the collapsed `Ord` case now falls
into; built and ran `sooth build` over `: f ( 'T: Ord 'T -- 'T ) drop ;` to confirm the
collapse actually produces `error: unknown capability \`Ord\` at line 1, col 11 (a bound
names \`Copy\` or a trait in scope)` — the generic arm, byte-identical to the file-parsing
wording, not a retyped REPL string.

### Deletion proof (E5, this phase's items)

`grep -rn 'parse_line\|InferredLine\|lower_line\|is_repl\|ast::Line\|Line::Def\|Line::Expr\|ImportCtx\|reject_generic_typedef_in_repl\|repl_unknown_capability_error\|carried_slot_bytes\|declares_type' src/` and the same over `tests/`: **both empty.** `docs/roadmap/` still names
several of these (this slice's own spec/brief/phase-notes, plus historical `P2`/`P3`/`P4`/
`P7` implementation specs recording what was built at the time) — out of scope per the
spec, not a statement of current design.

### Carried forward

- Phase 9 (R7) is unaffected: `generation`'s field and its one remaining unit witness
  (`lower_call_uses_resolved_generation_symbol`) were untouched here.
- Phase 10's E2 sweep should re-derive its file list fresh; this phase's deletions
  (`is_repl` alone was most of `parser.rs`'s share of phase 7's 235-hit count) shrink it
  further, and phase 10 must not assume the count from phase 7's report still holds.

### Gate

`cargo fmt --check` clean, `cargo clippy -- -D warnings` clean,
`cargo test --no-fail-fast`: **2676 passed, 0 failed**, 3 ignored (unchanged — the three
non-REPL `phase7_slice3b_follow.rs` notes). Net change from phase 7's 2687: −11
(parser.rs: −9 tests / +2 migrated = −7; ir/layout.rs: −4 `carried_slot_bytes` tests).
Zero `tests/qbe_baseline` diffs.

## Phase 9 (R7) — narrow the incremental-compile state

The whole of R7 landed; the stop condition did not fire. Deleted, in one compile unit
because each is a field or parameter every surviving path passes `None` to:

- `CallInst::generation` (`ast.rs`) and `instantiation_symbol`'s third parameter. The
  spec calls the field `ast::Instantiation::generation`; the struct is `CallInst` — same
  field, and `Instantiation` is not a type in this tree.
- The `(PolySig, Option<u64>)` pairing in `check.rs`'s `PolyEnv`, now
  `HashMap<String, Vec<PolySig>>`, plus the `PolyCtx` doc paragraph (R2b) that existed
  only to explain the pairing.
- `StructLayout`/`EnumLayout`/`ArrayLayout::drop_generation`, `Cells::drop_generations`,
  the `epoch` parameter of all four minters (`struct_`/`enum_`/`cell_`/`array_drop_symbol`),
  the R11/R11.2/R11.3 override-epoch doc block and its four mirrors, and every `None` seed.
- The `drop_generation` argument at every minting site in `ir/destructors.rs`,
  `ir/func_builder/quotation.rs` and `backend/qbe.rs`, plus the `None` argument at the
  ~27 `*_drop_symbol(id, None)` call sites in those files' unit tests.

### Four consumer sites the spec did not name

The spec's site list is a lower bound; `cargo build` enumerated the rest, which is the
whole reason the type flip goes first.

- **`check/terms.rs:731` and `:1130`** destructure `poly.env`'s candidates as
  `|(sig, _)|`. Neither is in R7's reader list (which names `check/poly.rs` only), and
  both are on the live monomorphic-dispatch path, not in a test.
- **`check/poly.rs:2060` `poly_cross_call` and `:4567` `no_poly_overload_matches_error`**
  both take `candidates: &[(PolySig, Option<u64>)]` in their own signatures. R7 named the
  `resolve_poly_overload` reader and the four mint sites but not these two, so the
  `Option<u64>` would have survived in two public-in-module signatures.
- **Three further `instantiation_symbol(.., None)` call sites** in `check/poly.rs`
  (`:5272`, `:5415`, `:6152` — the P7.S4 generic-impl member-word monomorph seeds and the
  trait-call dispatch mint), against the two the spec's `4937-4938`/`5198`/`5231` range
  implied.

### The surviving `instantiation_symbol` test is renamed

`instantiation_symbol_none_reproduces_native_spelling_expected` becomes
`instantiation_symbol_reproduces_native_spelling_expected`. The spec says its "only edit
is dropping the now-absent `None` argument"; the name, however, is the name of a
parameter this phase deletes, and a test named after a dead argument reads as coverage of
a mechanism that no longer exists. Content is untouched: it still pins
`sooth_mono_id__t0_i64`.

### Retired tests (3), each retired-mechanism

- `ast.rs::instantiation_symbol_some_appends_gen_component_expected` — asserted only that
  `Some(0)` appends `__gen0`. With the parameter gone there is nothing left to assert.
- `ast.rs::instantiation_symbol_distinct_generations_are_distinct_symbols_expected` —
  asserted `Some(0)`'s symbol differs from `Some(1)`'s. Same.
- `ir/driver.rs::lower_call_uses_resolved_generation_symbol` — phase 3 migrated it off
  `line_terms` and kept it green expressly so this phase would have a witness that fires
  when the field goes. It did: deleting the field is what makes the test unwritable, not a
  narrowing that leaves it green-and-inert.

### Stop condition: not triggered

`REGEN_QBE_BASELINE=1 cargo test --test qbe_baseline` regenerated the baseline and
`git status --porcelain tests/qbe_baseline/` came back **empty**. Not "changed only by
losing a generation suffix" — *no* baseline symbol changed at all, which is the stronger
form of the spec's condition: off the REPL path the suffix was never emitted, so removing
its source is byte-identical. The field was dead, not live-but-quiet.

### Mutation proofs

Two guards, each proved live *after* the parameter removal (isolated edit-and-revert on
one file at a time, restored and `git diff --stat` checked; no worktree copy).

- `instantiation_symbol`'s `sooth_mono_` prefix → `sooth_monoX_`: **1588 passed / 10
  failed** in `cargo test --lib`, among them
  `ast::tests::instantiation_symbol_reproduces_native_spelling_expected` and
  `check::poly::tests::transitive_discovery_dedups_repeated_instantiation_symbol`; the
  other eight are transitive-discovery and monomorphization tests that assert on minted
  symbol names. The surviving test is not a placebo.
- `struct_drop_symbol`'s `sooth_struct_drop_` prefix → `sooth_struct_dropX_`: kills
  `ir::destructors::tests::lower_appends_one_destructor_func_per_linear_struct_only`
  (1597 passed / 1 failed). The un-parameterised minter is still witnessed.

`lower_call_uses_resolved_generation_symbol` was not mutation-proved because it is
deleted; its mutation proof is the deletion itself.

### Three doc-prose calls

- **`src/driver.rs:961` stripped here**, out of the spec's literal R7 list. The comment
  inside `compile_so` read "allow them (earlier generations, printf) to resolve at load
  under RTLD_GLOBAL" — "earlier generations" names the concept this phase deletes, and
  **phase 10 would not have found it**: E2's grep is `repl|session|dlopen|override_epoch|
  drop_generation`, and that line matches none of them (`RTLD_GLOBAL` is not in the word
  list). Left standing it survives the whole slice. Now reads "(printf and friends)".
- **`src/ir/types.rs:552` left for phase 10**, deliberately: the `Resolver` doc says "the
  REPL supplies generation-mangled symbols". It contains `REPL`, so E2's grep does catch
  it, and it describes no signature this phase changes.
- **`docs/roadmap/P7-language-prereqs.md:811-817` fixed here** (commit `08700c8`), out of
  the spec's literal R7 list, on the same reasoning as `driver.rs:961`. The S3v entry's
  `REPL:` paragraph asserted that the closure disposer "resolves them against the live
  `drop_generation`, so it carries the session-wide override epoch" — a design claim about
  a field this phase deletes. R8 names `P7-language-prereqs.md`'s **S9** entry, not S3v,
  and phase 10 derives its sweep list from a grep over `src/` only, so neither would have
  reached a `docs/` line: left standing it survives the whole slice. It now describes what
  `emit_drop` actually does, resolve by the aggregate's positional id symbol.

### Deletion proof (E5, this phase's items)

`grep -rni 'generation\|override_epoch\|__gen' src/` returns exactly one line —
`ir/types.rs:552`, the phase-10 item above. `grep -rn 'drop_generation\|override_epoch'
src/ tests/` returns one line, `tests/phase7_slice3v.rs:313`, which is phase 6's own
retirement note recording the two REPL override-epoch tests it deleted (it names them in
order to record them as gone; that is the note working, not a survival).

### Carried forward

- The `always_mangle` note from phase 7 is unaffected: nothing here touches `resolve`.
- Nothing else is carried forward beyond `src/ir/types.rs:552`, recorded above as a
  phase-10 item.

### Gate

`cargo fmt --check` clean; `cargo clippy -- -D warnings` clean;
`cargo test --no-fail-fast`: **2673 passed, 0 failed**, 3 ignored (unchanged — the three
non-REPL `phase7_slice3b_follow.rs` notes). Net from phase 8's 2676: **−3**, exactly the
three retired tests above. Zero `tests/qbe_baseline` diffs.

`cargo clippy --all-targets` still reports the pre-existing `bool_comparison` warning at
`src/parser.rs:9892` (an `== false` in a parser unit test) and two
`needless_borrow`s in `tests/phase4_combinators.rs`. All three predate this phase and are
outside the `cargo clippy -- -D warnings` gate; not touched, per phase scope.

## Phase 10 (R8) — docstring, design-doc and invariant sweep

### Derived sweep list, not hardcoded

`grep -rniE 'repl|session|dlopen|override_epoch|drop_generation' src/ | grep -viE
'replac|replic'` at the start of this phase (post phase 9) returned **115 hits across 23
files** — far fewer than the spec's spec-time snapshot of 275/28, because phases 4-9 already
retired most of the REPL surface incrementally as they went (phase 9's carry-forward note
above already flagged `ir/types.rs:552`). Files actually touched this phase: `ast.rs`,
`backend/qbe.rs`, `check.rs`, `check/{audits,builtins,combinators,declarations,drop_graph,
engine,operators,poly,terms,word_families}.rs`, `driver.rs`, `ir.rs`, `ir/{driver,layout,
types}.rs`, `ir/func_builder/{calls,mod}.rs`, `lexer.rs`, `parser.rs`, `resolve.rs`. No hits
remained in `lib.rs`, `main.rs`, `test_support.rs`, `ir/destructors.rs`, or
`ir/func_builder/quotation.rs` — already clean from earlier phases.

Every hit was read in context, not pattern-substituted blind. Three shapes of edit:

- **Drop a stale alternative from a list** (`"Empty on the REPL/destructor/test paths"` →
  `"Empty on the destructor/test paths"`) where the REPL was one of several callers that
  left a field empty and the others still do.
- **Rename to the actual current caller** where the REPL was the *only* thing that made a
  doc's claim true and something else now plays the same structural role — `check.rs`'s
  `PolyCtx` fields now name the unit test that probes a body through `infer_probe_body`,
  not the REPL.
- **Explain the surviving mechanism** where the REPL was the claim's only witness and
  *nothing* replaces it. `ir/func_builder/calls.rs:821`'s "the REPL's env derives a
  multi-output `ret_ty` from the first output alone" had no surviving producer to rename
  to, so the comment now says why the branch tests the bundle flag rather than
  `out_arity >= 2` at all. An intermediate draft named `extern:` declarations as the
  surviving producer of that shape; `extern_multi_output_error` rejects a multi-output
  `extern:` outright, so that was a counterfactual, corrected in `b5e43f6`.
- **Delete the clause outright** where the REPL was the sole reason a rule existed and
  nothing replaces it — `check/audits.rs`'s per-word quotation audit no longer contrasts
  against "rejected separately at the REPL (R23)"; that second rejection path died with the
  REPL and nothing stands in for it.

One correction made mid-sweep: `ast.rs:3013`/`:3027`/`:3040` and `parser.rs:4040` invoked "the
REPL splice's epoch-tagged `.name`" as the mechanism that makes a decl's `.name` diverge from
its `.name_static`. That import-epoch tagging never existed as production code (`grep -rn
'epoch' src/resolve.rs` is empty); the actual mechanism is `resolve::mangle`'s `__m{module}`
suffix. The fixture literal `"q::Point__import3"` was renamed to `"Point__m1"` to match what
`mangle` actually produces, not left as a REPL-flavoured string with the REPL word stripped
out around it.

`backend/qbe.rs:303`'s `sooth_line_{seq}` carve-out (named in the spec's verified-anchors
table) is dead in full, not just its REPL clause: `grep -n 'sooth_line' src/` returns only
that one comment line pre-edit. Dropped from the list of symbols `qbe_name`'s escape
character has to leave alone, rather than reworded.

### `src/backend/qbe.rs:303`'s `__import{epoch}` half also didn't exist

The same comment's `__m{module}`/`__import{epoch}` mangle-suffix list named a second suffix
shape that, like the REPL's epoch tagging above, is not real: `grep -n '__import' src/` (pre-
edit) matched only doc-comment prose, never a format string. Dropped that half too, leaving
only the real `__m{module}` suffix `resolve::mangle` emits.

### E2, re-derived after the sweep

```sh
grep -rniE 'repl|session|dlopen|override_epoch|drop_generation' src/ | grep -viE 'replac|replic'
```

returns exactly six lines, all in `src/driver.rs`: the `dlopen`/`dlsym` extern declaration,
its two call sites, one `SAFETY` comment, one doc comment on `Library::open`, and one unit
test's `.expect("dlopen should succeed")`. Word-boundary form (`grep -rniwE`) returns the
same six. This matches E2's expected witness exactly (the relocated `Library` primitive is
the sole permitted survivor).

### Design-doc sweep (current-state only, no removal narrative)

- **`CLAUDE.md`**: the load-bearing-invariants bullet now reads "`driver::Library` loads a
  compiled `.so` in-process via `dlopen`" — states the surviving primitive, not the REPL.
- **`DESIGN.md`**: three mentions fixed (codegen summary, the "Liveness and craft discipline"
  bullet, the `Decided` backend bullet) plus two `Open/deferred` items (the ISR/dlopen
  question dropped its now-answered REPL clause; "owning a native backend" dropped
  "sub-millisecond REPL" as a motivating reason; "REPL late binding for redefinition" renamed
  to "Late binding for redefinition", now framed against `driver::Library`'s retained `dlopen`
  primitive rather than a session that no longer exists).
- **`docs/roadmap/ROADMAP.md`**: the guiding-principles "A REPL and immediate feedback arrive
  in Phase 1" bullet now reads "Fast local iteration (`sooth run`)"; the paragraph describing
  the REPL's hand-rolled line editor (prompt, history, tab completion) under "Cross-cutting"
  was deleted outright — it described `src/editor.rs`, which no longer exists, in present
  tense. The **P1** row's existing "interactive criteria retired with the REPL" note (written
  by phase 5) was left as-is: it is a correct statement about what happened, not a stale claim
  about current REPL behaviour.
- **`docs/roadmap/P7-language-prereqs.md`'s S9 entry**: rewritten from `[ planned ]`,
  ~50 lines of "what to delete and why", to `[ done ]`, ~10 lines stating the post-removal
  state (`Ctx` is a struct; `driver::Library` retained; `always_mangle` unreachable-but-kept)
  plus the still-open book-doc follow-up. **The file's six other REPL mentions were ruled on
  as a set**, not one at a time — an earlier draft named only the S3s one as an exception,
  which reads as though the set were exhaustive. R8 names the S9 entry specifically; the
  rest split by whether the mention asserts a *live premise* or *records history*:
  - **Swept, because a live premise went false.** A "standing hazard" or open-deferral item
    claims something about current behaviour, so losing the REPL makes it wrong rather than
    merely dated. `:423` (S3h's `__quot0` non-PIC session-link hazard), `:811` (S3v's
    owning-closure disposer, whose "no session line can link one" unreachability claim also
    cited a blocked-state tripwire in `tests/phase7_slice3v.rs` that phase 6 deleted — that
    file records the deletion and its reason at `:313-319`), and `:1109` (S8's "REPL
    trait/impl checking" deferral). Each now carries the moot marker instead of the claim.
  - **Left untouched, because they record history.** `:366` (S3g), `:384` (S3g-follow),
    `:514` (S3k) and `:709-714` (S3s) all sit inside an `**Exit:**`-bearing body describing
    how a landed slice was built — the class R8 puts out of scope as "what was built at the
    time, not current design". `:384` cites `src/repl.rs`'s `lower_instantiation`, a path
    that no longer exists; that is stale as a *file reference*, and re-anchoring the
    roadmap's historical bodies against deleted paths is a doc-hygiene pass over all of
    `docs/roadmap/`, not R8's stated-design sweep. **Recommended for phase 11's exit sweep
    to schedule or explicitly decline**, not silently inherit.
- **`docs/roadmap/P1-repl-and-liveness.md`**: the opening paragraph, handed to this phase by
  phase 5's notes and on no phase's derived list, still stated the removed design in present
  tense ("The REPL runs on the **backend** via `dlopen`: each new word is compiled to a
  shared object and loaded into the live session … redefinition loads a new object and swaps
  the name→symbol entry"), three lines above the `**Exit (retired):**` note phase 5 wrote —
  so the file contradicted itself. It is also the source of CLAUDE.md's load-bearing-invariant
  wording, which this phase rewrote, so leaving it would have left the invariant asserted in
  the file it came from. Rewritten to the surviving rule (no in-process JIT, no comptime
  interpreter, `driver::Library`'s `dlopen`/`dlsym` over a `compile_so` output, nothing
  calling it today, whole-program `run` via compile-to-binary + subprocess), matching CLAUDE.md
  and `codegen.md`. The redefinition and Factor in-image sentences were dropped rather than
  reworded: both describe the session's design, and DESIGN.md's `Open/deferred` "Late binding
  for redefinition" item already carries what survives of the first. The heading and phase 5's
  retirement notes are untouched.
- **`docs/design/`**: `control-flow.md` (7 word-boundary mentions, now 0) — the two REPL-
  specific rejection sentences in the 6a/6b narrative were dropped (the whole rejection they
  described no longer exists to contrast against), and the entire "Phase 4 Slice 6c" paragraph
  (REPL `Session`-level combinator store: 9 sentences describing `Session::combinators`,
  `infer_line`, `lower_line` entry points) was condensed to the two sentences of it that are
  still true design facts independent of the REPL (combinators have no compile event to
  freeze against; a poly word's frozen-resolver precedent doesn't transfer). `modules.md`
  (6 → 0): the whole "REPL imports (Slice 5b)" paragraph, entirely about a now-deleted
  mechanism, was deleted outright. An intermediate draft left a two-sentence retirement
  pointer to P7.S9 in its place; `b5e43f6` removed that too, since a pointer to the removal
  is removal history, which R8 bars from these files. `codegen.md` (4 → 0): the "REPL runs
  on the backend" bullet and "REPL's stack buffer is a driver artifact" paragraph were
  rewritten to describe `driver::Library`'s retained primitive as the still-true "no
  in-process JIT" mechanism, not a session. That bullet initially preserved the old
  paragraph's "the process holds live, natively-compiled code it can call at once", which
  invents a consumer: nothing calls the primitive. It now states the capability and says so,
  matching the wording `b5e43f6` had already settled on in DESIGN.md's `Open/deferred` item. `memory-model.md` confirmed **0** word-boundary REPL mentions
  both before and after, matching the spec's own correction of a review round's false claim.
- **`docs/roadmap/P8-packages-modules.md`**: `:111` and `:176`'s "also read by the REPL for a
  session" clauses dropped (the fallback chain reads the same regardless); `:291-292`'s
  declined "REPL exemption in the compiler" note rewritten to say the question is moot
  because the REPL no longer exists, rather than describing REPL behavior that no longer
  happens.
- **`docs/roadmap/P12-self-hosting.md`**: `:14` — plain mention strip, `REPL/` dropped,
  "No metacircular JIT: the self-hosted build path still runs on the backend" survives
  verbatim otherwise. `:19` — rewritten to name `driver::Library`'s `dlopen` over a
  `compile_so` output as the host→Sooth FFI direction, per R1's decision; the sentence's
  other half (Sooth calling host code, pulled into Phase 8) is untouched, and the rewrite
  states this answers the direction rather than leaving it recorded as open.

### Follow-up recorded, not started

`docs/book/preface.md` (5 REPL mentions), `getting-started.md` (6), `words.md` (2), and
`the-stack.md` (1 — correctly included this time, the item the spec flagged as omitted from
the roadmap entry's own list) all still teach the REPL. `docs/book/SUMMARY.md:55`'s
`the-interactive-book.md` entry (unwritten, planned) still stands. P7.S9's phase-notes doc
section was corrected mid-phase: an early draft of this phase's edit said the interactive-
book `SUMMARY.md` entry "drops" as if done; it does not, and the entry now says the drop is
tracked follow-up work alongside the three prose files, not a completed action. None of the
four book files were touched.

### Gate

`cargo fmt --check` clean; `cargo clippy -- -D warnings` clean; `cargo test --no-fail-fast`:
all suites green, zero failures (unit-test count and `tests/qbe_baseline` unaffected — no
*production* code changed this phase; the `src/` diff is comments, plus one unit-test
fixture literal (`"q::Point__import3"` → `"Point__m1"`, above) and one test rename, both in
`#[cfg(test)]` modules).

Nothing in the toolchain catches prose reflow: `cargo fmt` does not rewrap comments and the
repo has no markdownlint config, so the sweep's rewrites left ragged mid-sentence breaks
behind. Swept at the end of the phase, in `control-flow.md`, `modules.md`,
`P8-packages-modules.md` (×2), `src/ast.rs` (×2), `src/ir/layout.rs` (×2), `src/driver.rs`,
`src/check/engine.rs` and `src/check/poly.rs`. The check is a scan of the phase's own added
lines for a short line followed by more prose, not a whole-repo pass — plenty of unrelated
ragged paragraphs predate this slice. `cargo clippy --all-targets`
still reports the same three pre-existing warnings from phase 9 (`src/parser.rs`'s
`bool_comparison`, two `needless_borrow`s in `tests/phase4_combinators.rs`), confirmed
unchanged by diffing against a `git stash` baseline — outside this phase's scope, not fixed.
