# P7b.S12 spec — poly-body App-dispatch output rendering (SOO-39)

**Status: shipped.** Phase 1 `6c75be0` (Generic arm + two-tier unbound rule,
`src/check/poly/ground.rs`, 8 units). Phase 2 `f6576f0` (goldens G1–G8+G11 in
`tests/phase7b_slice12.rs`, P7b/ROADMAP housekeeping). Suite 3480 / 0. This
file is the lean reference; the how lives in those commits. Discovery lives in
[slice12-brief](./slice12-brief.md), [slice12-probes](./slice12-probes.md),
[slice12-paper-tests](./slice12-paper-tests.md).

## Problem

A bound-generic body (`['It: Cursor]`) that dispatches a trait member whose
declared output row is App-headed
(`next ( 'It['T] -- Option['T] 'It['T] )`) either failed at check with a
stack-effect mismatch or, worse, checked with a silently mis-typed value.

Root cause: a single missing match arm. `render_member_decl`
(`ground.rs`, pushed member outputs through bindings for
`poly_trait_member_call`) rendered `PolyType::App` head-and-args but had **no
`PolyType::Generic` arm**: a concrete ctor applied to variables
(`Option['T]`, `Step['T 'It['T]]`) fell into `other => other.clone()`, so its
`args` stayed in **member-variable space** — the member row's element variable
landed verbatim in the caller's id space.

Three faces of the same leak (probes B/C/F/G):

1. **Out-of-range id** — renders `'?1`; every downstream concrete-shaped op
   chokes and reports the operator underflowing ("holds 1"/"holds 0"). The
   S6d-4 "loses its outputs" symptom (outputs present but non-concrete).
2. **In-range, wrong id** — renders a different caller variable, a silent
   mis-typing at check time (a `Some` payload that must be `'E` types as `'It`,
   a `* -> *` ctor variable). **A correctness gap, not a diagnostics gap.**
3. **Id coincidence** (the mask) — when the caller's element variable sits at
   id 1 and is the operand's App argument (`lib/core/iterator.sth`'s
   `fold`/`for_each`), the leaked member id 1 reads as the right caller
   variable and the program checks. This coincidence is the only reason the
   shipped Iterator consumers worked.

Enduring fact the analysis rests on (probe F/G): a poly word's `ty_var_names`
are interned in the **effect's mention order**, not the bound-list's written
order. `['It: Cursor 'T] ( 'T 'It['T] -- … )` has `'T`=0, `'It`=1.

The diagnostics twin `substitute_member_var` (`ground.rs`) had the same missing
arm, so member sigs with concrete-ctor applications rendered wrong in error
text.

The roadmap sentence names a second, independent facet — poly bodies
**cross-calling** poly words over compound receivers — which is a *located*
fence today (probe E, S1-17.i in `crosscall.rs`), not a loss. It does not gate
the drain-fold/`fold` shapes and was **not** this slice's fix (R4).

## Rulings (adjudicated)

- **R1 (direction).** Fix the render, not the diagnostics. Face 2 is a
  correctness gap; sharpened errors alone would leave the mis-typing silent or
  merely rejected.
- **R2 (scope).** `render_member_decl` gains a `PolyType::Generic` arm
  (render `args` recursively through the existing `render` closure; carry
  `is_enum`/`idx`/`module`/`name`/`len_args` verbatim — a member row declares
  no length variables, so a `Len::Var` there is unreachable by construction,
  the member-grammar gate at `parser.rs:429`). `substitute_member_var` gains
  the mirror arm. No other dispatch route changes: the splice route
  (`apply_subst`'s `Generic` arm, `unify.rs`) and the mono route
  (`ast::ground_member_type`) never touch `render_member_decl`; the impl-side
  renderer `ground_member_poly` already had the complete Generic arm this
  mirrors.
- **R3 (unbound vars — the two-tier rule).** `unify_member_operand` has no
  `Generic` arm: a ctor-headed declared **input** (`Option['U]`) matches only
  by the tail's structural equality (`ground.rs:187`), recording **no
  binding**. An unbound member variable inside `Generic` args thus has two
  answers, keyed by whether the operands pinned it:
  - **Input-occurring — pinned by the admitting equality.** Dispatch only
    fires when the caller's slot arg is structurally equal to the declared
    input, i.e. the caller's variable with that exact id flowed through.
    Render **verbatim** (`PolyType::Var(v)`): the id IS the caller-space
    answer. **These programs compile today** — at the mono call site the
    CtorImage mint arm (`trait.rs:479-560`) re-grounds the obligation's slots
    and per-site unifies the impl member's grounded sig via `unify_poly_input`,
    whose `Generic` arm (`unify.rs:343-374`) binds the declared args
    positionally (the channel that makes `mapover` work). **This is the
    enduring subtlety**: the verbatim tier keeps today-compiling programs
    compiling byte-stably. An unscoped `Var(var)` fallback would re-type the
    pinned payload as the header variable (`* -> *` in an element position) —
    the face-2 class this slice kills.
  - **Output-only — never pinned.** Render `PolyType::Var(var)` (the
    dispatched header variable). Such a row never compiles on either route
    (generic-mint fails closed at `compose_member_theta` →
    `member_unbound_variable_error`; the mono route surfaces
    `poly_unbound_output_ty_error`, `trait.rs:395-399`), so this tier only
    shapes diagnostics on never-compiling programs.
  Implementation: the `render` closure consults a `poly_type_mentions_var`
  helper over the member row's declared inputs (a new parameter).
- **R3.5 (GenericVariant posture).** The new arm adds no `GenericVariant`
  case; it stays under `other => other.clone()`: no production route reaches
  one (unconstructible outside an eliminator arm's own scrutinee; sole
  constructor `generic_variant_type` fires only at the `~[ … ]` mint,
  `poly.rs:2740`), and no caller relies on the catch-all for it. The posture
  is pinned by a unit, not relied on by a caller — a bespoke `unreachable!` in
  a pure-render helper would be untestable ceremony.
- **R4 (fences — what does NOT move).** The cross-call App fence (S1-17.i) and
  `poly_cross_output`'s compound-output rejection stay byte-identical, tests
  included; their lift is SOO-60. **No S6d work here**: no sentinel grounding,
  no slice impl, no `PolySlot` provenance (`Deriv`). S6d-6/S2-6 fences and all
  S8/S8b/S8c member machinery untouched.
- **R5 (diagnostics fence).** The `note: declared ( -- )` placeholder is
  **not** fixed here (many goldens pin it byte-exactly); deferred to SOO-61.
  No diagnostic text changes except where the fix makes a previously-garbled
  member-sig rendering correct (G7) — today unreachable-with-correct-content,
  so no existing golden pins their Generic shape.
- **R6 (behavioral fence — the coincidence contract).** Programs that check
  today **by id coincidence** (`fold`/`for_each` over List and Range — element
  var at id 1 AND the operand's App argument, so `unify_member_operand` binds
  `(1, Var(1))`) produce **byte-identical rendered types** post-fix: the
  Generic arm renders member `Var(1)` through a bindings map already holding
  `(1, Var(1))` — a no-op relative to today's clone. A consumer whose element
  sits at id 1 but is NOT the operand's App argument is face-2 mis-typed today
  and gets no byte-identity promise. `tests/phase7b_slice8.rs`'s List/Range
  consumers are the canaries; the full suite is the global gate.
- **R7 (no new capability).** A rendering correction inside the existing
  unification/bindings machinery. **No new `PolyType` variants, no new
  unification rules, no changes to `unify_member_operand` or the obligation
  record.** Observable behaviour changes only where today's render was already
  the leak: structurally-pinned compiling programs render byte-identically
  (tier 1), output-only rows never compile (tier 2), and the one remaining
  delta — a hybrid whose App-typed output arg is a Generic-input-pinned var —
  renders **more** correctly (a face-2 fix, no existing golden pins the old
  rendering).

## Goldens (`tests/phase7b_slice12.rs`, commit `f6576f0`)

Harness conventions from `tests/phase7b_slice8.rs`. G1–G8+G11 live in the
slice12 file; G9 is the named slice8 canaries staying green; G10 is `cargo
test`.

| Golden | Pins |
| --- | --- |
| G1 `bound_generic_drain_fold_option_row_checks_clean` | bound-generic drain/fold Option row checks clean (was underflow "holds 1") |
| G2 `add_directly_on_some_payload_reports_holds_zero_no_more` | add stays an **honest** located error post-fix (remainder genuinely ill-typed); asserts no `'?1`/garbled id, no panic. Pre-fix "holds 0" bytes are the S6d-4 provenance anchor, not a target |
| G3 `member_output_residual_renders_caller_space_option` | byte-pinned error: body leaves `i64 'It[i64] Option[i64]` vs declared `i64 Option[i64]` — correct caller-space render (no `'?1`) |
| G4 `some_payload_types_as_the_element_variable_not_the_constructor` | payload types as `'E` not `'It`; arms agree; build_ok |
| G5 `leaked_member_id_renders_through_bindings_in_range` | Generic arm renders through bindings; build_ok |
| G6 `end_to_end_list_backed_cursor_drain_prints_6` (exit criterion) | List-backed cursor drain, stdout `6`, exit 0 |
| G7 `member_sig_diagnostics_render_generic_args_in_caller_space` | `substitute_member_var` renders the member sig in the dispatched variable's name (e.g. `'V['V] Option['V]` for caller var `'V`) — caller space, no member-space ids; byte-pinned from live binary |
| G8 `cross_call_app_fence_stays_byte_identical` (R4 pin) | S1-17.i fence byte-identical |
| G9 `iterator_consumers_survive_the_render_fix` (R6 canary) | slice8 List+Range `for_each`/`fold` stay green (cited, not duplicated) |
| G10 `existing_suite_green` (global gate) | full suite green |
| G11 `structurally_pinned_member_input_stays_compiling_byte_identically` (R3 tier-1 pin) | the structurally-pinned subclass keeps compiling byte-identically — the fix does NOT re-type the pinned payload (the unscoped fallback would break exactly this program) |

**G11 subtlety (the enduring pin).** `unify_member_operand` has no `Generic`
arm, so a ctor-headed declared input (`Option['U]`) dispatches at the
poly-body route only by the tail's structural equality — no binding. But such
programs **compile today** via the CtorImage mint trace: the mono call site
re-grounds the obligation's slots and per-site binds the declared args through
`unify_poly_input`'s `Generic` arm (`unify.rs:343-374`, the `mapover` channel).
The round-1 build_error_located premise was false; G11 was rebuilt as a
build_ok byte-stability pin. Fixture: trait `Cursor['S: * -> *]` with a
ctor-headed member input (`accept ( 'S['T] Option['U] -- Option['U] )`), a poly
caller `: ch ['C: Cursor 'D 'E] ( 'C['D] Option['E] -- Option['E] )` whose
body is `accept` (caller `'E` at id 2 coincides with `'U`), and a `main`
calling `ch` concretely. The non-coincidence twin fails identically pre-render
on both sides with `trait_member_operand_error` — no golden needed.

## Units (`src/check/poly/ground.rs` `tests`, commit `6c75be0`)

8 units: Generic args through bindings; R3 tier-1 (input-pinned var renders
verbatim); R3 tier-2 (output-only var → bound variable); `len_args` verbatim;
App-inside-Generic recursion; R3.5 GenericVariant verbatim-clone posture (pins
the kept posture so a future arm can't silently change it); `poly_type_mentions_var`
walks all variants; the `substitute_member_var` twin arm.

## Housekeeping (phase 2, commit `f6576f0`)

- `docs/roadmap/P7b-higher-kinded-types.md` — S6d/S6d-PREREQ "standing S8-era
  limitation" language replaced with the current-design statement (symbolic
  member-output render is Generic-complete; bound-generic consumers dispatch
  without leaking member-space ids); S6d gate re-pointed to the residual pair.
- `docs/roadmap/ROADMAP.md` — P7b row corrected (S6d-PREREQ landed), S12 row
  added.
- Growth-structure re-check on `ground.rs`: no split, signals not tripped
  (recorded in the P7b doc).

## Deferred / out of scope

- **Cross-call App lift** — [SOO-60](https://linear.app/ordfruma/issue/SOO-60)
  (R4), its own probe round first. S6d-gating only if S6d's consumer work
  needs poly-word-to-poly-word calls over compound receivers; the hard part is
  the interaction with R6's growth ban, not App-vs-App matching.
- **`GenericVariant` in a member output** — R3.5: no bespoke arm,
  unconstructible here, documented not exercised.
- **`note: declared ( -- )` placeholder** —
  [SOO-61](https://linear.app/ordfruma/issue/SOO-61) (R5), golden-churn
  inventory first.
- **Poly-body borrow propagation for slice remainders** (`PolySlot` has no
  `Deriv`) — recorded for **S6d**, lands where S6d's slice consumers hit the
  exclusivity story.

## Review loop (outcome)

Three fresh-context rounds (260911). R3 was rebuilt as the two-tier rule after
the round-1 soundness lane caught that an unscoped `Var(var)` fallback
mis-renders the structurally-pinned subclass; G11's premise was corrected in
round 2 (a mint-path trace proved the subclass compiles today, so
build_error_located → build_ok byte-stability pin). Round 3 APPROVED.
