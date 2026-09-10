# P7b.S12 paper tests — validated golden designs (recon round)

Designed against the clean tree at HEAD `d7559bb` (worktree `soo-39`,
fast-forwarded to main tip; suite 3463/0). Fixtures are committed:
`probes/s12_*.sth`; frozen pre-fix stderr: `probes/s12_baseline.md`
(byte-exact). Companion docs: [slice12-brief](./slice12-brief.md),
[slice12-probes](./slice12-probes.md). Harness conventions from
`tests/phase7b_slice8.rs`: `single_file_hosted`/`build_ok`/`build_run_keep`/
`build_error_located`, `thing_condition_expected` naming (`build_error_located`
asserts non-zero exit and no `panic`; the `error:`-in-stderr check is
caller-side — `tests/phase7b_slice8.rs:119-135`). Probe H needs the shared
manifest (`--manifest tests/fixtures/sooth.pkg`) for `core::list`.

Every fixture below was built at HEAD `d7559bb` and its measured "today"
behaviour is recorded in the baseline. The "post-fix" column is the expected
golden; exact diagnostic bytes for any *new* message get pinned at
implementation time from the live binary, per house convention. No new
diagnostic text is expected anywhere in this slice (the fix renders types;
it writes no new rejections) — G7's byte pin is the one exception, and its
today-content is itself the garbled rendering.

## G1 — `bound_generic_drain_fold_option_row_checks_clean`

Fixture: `probes/s12_a_drain_fold_option_row.sth` (probe A). Today:
underflow at `add` ("holds 1"), exit 1. Post-fix: the body checks with
correct types (the `Some` payload is `i64`, the remainder `'It[i64]`) —
patch-validated. The committed fixture has no `main`, so the *link* step
fails by design (the golden canary: if a future change makes body-check
depend on lowering, this golden fails loudly instead of silently passing);
the golden's fixture adds `: main ( -- ) ;` and asserts `build_ok`.

## G2 — `add_directly_on_some_payload_reports_holds_zero_no_more`

Fixture: `probes/s12_a2_add_holds_zero.sth` (probe A2). Today: S6d-4's
exact message ("holds 0"), exit 1. **Post-fix: still a check error, and
deliberately so** (patch-validated): the add sits on the remainder and the
payload — genuinely ill-typed — so the message shifts to the honest
underflow shape for real operands ("holds 1"). The golden is
`build_error_located` asserting no `'?1`/garbled id anywhere and no
`panic`. Kept as its own golden (not folded into G1) because it is the
byte-provenance anchor to the S6d-4 record — the *pre-fix* bytes pin the
recorded incident; the *post-fix* pin is the invariant (located, non-panic,
no leaked id), per house convention.

## G3 — `member_output_residual_renders_caller_space_option`

Fixture: `probes/s12_b_member_output_residual.sth` (probe B). Today: the
mismatch names `Option['?1]`. Post-fix (patch-validated): byte-pinned error
text — `body leaves \`i64 'It[i64] Option[i64]\`, but the declared outputs
are \`i64 Option[i64]\``. The fixture's declared outputs deliberately omit
the remainder (that is how the residual is exposed), so the golden pins the
**corrected rendering** (no`'?1`), not a clean build; the build_ok twin is
G5, whose declarations match its residual.

## G4 — `some_payload_types_as_the_element_variable_not_the_constructor`

Fixture: `probes/s12_c_arm_payload_mistype.sth` (probe C). Today: arms
disagree (`'E` vs `'It`), exit 1. Post-fix (patch-validated): checks clean —
the `Some` payload is `'E` (element var 0), both arms exit `'E`. The
golden's fixture adds `: main ( -- ) ;` and asserts `build_ok`. This is the
correctness-gap face: the pre-fix failure was accidental (arms
disagreeing), so this golden guards the silent-mis-typing class, not just
the diagnostic.

## G5 — `leaked_member_id_renders_through_bindings_in_range`

Fixture: `probes/s12_f_id_coincidence_space.sth` (probe F). Today:
`Option['It]` vs declared `Option['T]`. Post-fix (patch-validated):
`build_ok` — member Var(1) renders through the bindings map (`(1, Var(0))`
from the operand unification) to the caller's `'T`. Pins the in-range
rendering path that G3's out-of-range case cannot reach. The golden's
fixture adds `: main ( -- ) ;` for the link step.

## G6 — `end_to_end_list_backed_cursor_drain_prints_6` (exit criterion)

Base fixture: `probes/s12_h_end_to_end_list_drain.sth` (probe H), manifest
`tests/fixtures/sooth.pkg`. Today: the `impl: Cursor for List` member
checks clean (mono route) and the bound-generic `drain` fails (leak) —
baseline records the exact stderr. Post-fix (patch-validated):
`build_run_keep`, stdout `6`, exit 0. The golden's fixture copy uses the
house spelling `tests/phase7b_slice8.rs` established: add
`: mkempty ( -- List[i64] ) Nil ;` (a bare `Nil` in a mono body with no
declared `List[i64]` in scope is the zero-mint `unknown word`, unrelated to
this slice) and rewrite `main`'s `Nil` calls to `mkempty`. Do **not** add
the `hosted::show` import: `single_file_hosted` prepends it
(`tests/phase7b_slice8.rs:54-69`) and a duplicate import collides in the
seen-map (`declarations.rs:894-899`). The committed probe fixture stays
byte-frozen for its pre-fix baseline. This is the slice's
exit criterion: the full S6d-shaped program — HKT trait, Option row with a
bare App-headed remainder, impl over a generic ctor target, bound-generic
draining fold with self-tail recursion — compiles and runs without any S6d
work.

## G7 — `member_sig_diagnostics_render_generic_args_in_caller_space`

Fixture: a member-dispatch error that renders the member sig through
`substitute_member_var` (its callers at `ground.rs:433` and `:1600` feed
`no_candidate_fits_operands_error` / `ambiguous_trait_member_error` shape
strings). Today: a member sig naming `Option['T]` renders the ctor with a
member-space variable (garbled against the caller's table). Post-fix: the
member type renders in caller space; byte-pinned from the live binary at
implementation time. The exact fixture spelling is chosen during
implementation (the error must fire for an operand-shape reason, not the
leak — the leak's own diagnostics disappear with the fix); the pin is the
rendered *member type*, which the fix changes.

## G8 — `cross_call_app_fence_stays_byte_identical` (regression pin)

Fixture: `probes/s12_e_cross_call_app_fence.sth` (probe E). Today and
post-fix: the S1-17.i located rejection, byte-identical (baseline is the
pin). R4's untouched-fence assertion; if this golden moves, the slice has
overrun its scope.

## G9 — `iterator_consumers_survive_the_render_fix` (the coincidence contract)

Existing goldens, named canaries: `tests/phase7b_slice8.rs`
`for_each_drains_a_list_through_the_iterator_bound`,
`fold_sums_a_list_through_the_iterator_bound`,
`for_each_and_fold_drain_a_range_through_the_iterator_bound`. Post-fix:
green (R6's mechanism argument is the symbol-stability argument for the
`(1, Var(1))` class; the canaries' green is the assertion — no symbol-diff
harness is wired). The claim being guarded:

## G10 — `existing_suite_green`

`cargo test` at the fix commit: 3463+ passed, 0 failed (3463 at base plus
this slice's new tests). Any red existing golden is a real signal per R6 —
the fix must explain it (a coincidence case the analysis missed), not
patch the golden.

## G11 — `structurally_pinned_member_input_stays_compiling_byte_identically`

Review-round addition (R3 tier-1 pin; premise corrected by the round-2
mint-path trace). `unify_member_operand` has no `Generic` arm, so a
ctor-headed declared input dispatches at the poly-body route only by the
tail's structural equality (`ground.rs:187`) — no binding. But such
programs **compile today**: at the mono call site the CtorImage mint arm
(`trait.rs:479-560`) re-grounds the obligation's slots and per-site
unifies the impl member's grounded sig via `unify_poly_input`, whose
`Generic` arm (`unify.rs:343-374`) binds the declared args positionally —
the channel that makes `mapover` work (`tests/phase7b_slice2.rs:295`).
Fixture shape (exact spelling at implementation time): trait
`Cursor['S: * -> *]` with member
`accept ( 'S['T] Option['U] -- Option['U] )` (admitted bare-Var by
`member_shape_is_supported`, `parser.rs:429-431`; member space header 0,
'T 1, 'U 2; impl body can be `swap drop`); poly caller
`: ch ['C: Cursor 'D 'E] ( 'C['D] Option['E] -- Option['E] )` (the bound
bracket is required for member resolution; probe F proves it does not
perturb effect-mention id order, so 'C=0/'D=1/'E=2 hold) whose body is
just `accept` (caller 'E lands at id 2, coinciding with 'U); a `main`
calling `ch` concretely through the impl. Today: build_ok (the body
checks clean on the verbatim render; the mint binds 'U per-site). Post-fix: build_ok with
byte-identical behaviour — the R3 verbatim tier keeps the body render
byte-stable. The golden (`build_ok`, or `build_run_keep` exit 0) asserts
the fix does NOT re-type the pinned payload: the unscoped `Var(var)`
fallback would break exactly this program with a body-level residual
mismatch. (The non-coincidence variant — caller slot arg id ≠ member local
id — fails at the operand check pre-render on both sides; no golden
needed, the dispatch-error family is already pinned by G2/G3.)

## Units (beside the changed sites, `src/check/poly/ground.rs` tests)

- `render_member_decl_renders_generic_args_through_bindings` — `Option['T]`
  member output, bindings `(1, Concrete(i64))` → `Option` args `[Concrete(i64)]`.
- `render_member_decl_input_pinned_unbound_var_renders_verbatim` — R3 tier
  1: row with a ctor-headed input over member var 2, bindings lacking 2,
  output `Generic{args:[Var(2)]}` → args render `[Var(2)]` verbatim, not
  `Var(var)`.
- `render_member_decl_generic_falls_back_to_the_bound_variable` — R3 tier
  2: unbound, output-only member var in args → `PolyType::Var(var)`.
- `render_member_decl_generic_carries_len_args_verbatim` — concrete
  `len_args` unchanged (and the S2-5 unreachable documented).
- `render_member_decl_renders_app_inside_generic_args` — `Step['T 'It['T]]`
  shape: nested `App` inside Generic args recurses.
- `render_member_decl_generic_variant_posture_stays_verbatim_clone` — a
  hand-built `GenericVariant` passed directly to `render_member_decl`
  clones through verbatim (R3.5 adjudication: no bespoke arm; the
  reachability argument lives in a code comment beside the arm, citing
  `trait.rs:1157`). The unit pins the posture so a future arm addition
  cannot silently change it.
- `poly_type_mentions_var_walks_all_variants` — the R3 helper: detection
  through nested App/Generic/Quotation/Ref/Array positions and `Len::Var`.
- `substitute_member_var_renders_generic_args_for_diagnostics` — the twin
  arm, mirroring the first four (total-rewrite semantics, no two-tier).
- Naming `thing_condition_expected`; total target ~8-12 units.
