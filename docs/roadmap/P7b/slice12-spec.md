# P7b.S12 spec — poly-body App-dispatch output rendering (SOO-39)

- Base: `d7559bb` (main tip; post-S13 poly.rs partition — `poly.rs` is split
  into `src/check/poly/{ground,unify,trait,crosscall,construction,instantiate,overload,tests}.rs`
  — and post-S6d-PREREQ). Suite green at base: 3463 passed / 0 failed.
- Provenance: Linear [SOO-39](https://linear.app/ordfruma/issue/SOO-39)
  (project P7b, Higher-kinded types). The defect was first recorded by probe
  round S6d-4 (260907) and deferred by `slice6d-prereq-spec.md` "Deferred /
  out of scope" item (3) — "its own slice". This is that slice.
- Discovery + evidence: [slice12-brief](./slice12-brief.md) (problem,
  adjudicated mechanism, rulings R1–R7, recon addendum),
  [slice12-probes](./slice12-probes.md) (rounds A–G plus probe H — A2 folded
  under A, so 9 fixtures, each byte-identical against
  `probes/s12_baseline.md`, independently re-verified on a scratch
  `git archive HEAD` extract; probe H has no round section — its analysis is
  the paper-tests exit criterion), [slice12-paper-tests](./slice12-paper-tests.md)
  (validated golden designs G1–G11 + the unit list, measured at HEAD).
- Size: **S**. Two functions in one band (`src/check/poly/ground.rs`) plus
  units, goldens, and roadmap housekeeping. Two phases.

## Problem

A bound-generic body (`['It: Cursor]`) that dispatches a trait member whose
declared output row is App-headed
(`next ( 'It['T] -- Option['T] 'It['T] )`) either fails at check with a
stack-effect mismatch or, worse, checks with a silently mis-typed value.

The root cause is a single missing match arm. `poly_trait_member_call`
(`ground.rs:1516`) unifies the member row's App-headed input against the
caller's operand — correctly binding the member header var and element vars
into `bindings` — truncates the operands, and re-pushes the declared outputs
through `render_member_decl` (`ground.rs:197`; sole production caller is the
output push at `ground.rs:1678`). `render_member_decl` renders
`PolyType::App` head-and-args through the bindings, but has **no
`PolyType::Generic` arm**: a concrete ctor applied to variables
(`Option['T]`, `Step['T 'It['T]]`) falls into `other => other.clone()`, so
its `args` stay in **member-variable space**. The member row's element
variable (member id 1; the trait header var is id 0) lands verbatim in the
caller's id space.

Three faces of the same leak, all reproduced (probes B/C/F/G):

1. **Out-of-range id** (caller has one variable): renders `'?1`. Every
   downstream concrete-shaped operation chokes; `poly_delegate_op` extracts
   the maximal concrete suffix of the operand run and reports the operator
   underflowing ("holds 1"/"holds 0"). This is the S6d-4 "loses its outputs"
   symptom — the outputs are present but non-concrete (probes A, A2, B, G).
2. **In-range, wrong id**: renders a different caller variable — a silent
   mis-typing at check time (probe C: a `Some` payload that must be `'E`
   types as `'It`, the `* -> *` constructor variable; caught only because the
   arms happened to disagree). **This face is a correctness gap, not a
   diagnostics gap.**
3. **Id coincidence** (the mask): when the caller's effect mentions its
   element variable second (id 1) — `lib/core/iterator.sth`'s `fold`/
   `for_each` do — the leaked member id 1 reads as the right caller variable
   and the program checks. This coincidence is the only reason the shipped
   Iterator consumers work (probe F).

Variable-id ordering fact the analysis rests on (probe F/G, verified in the
recon addendum at `parser.rs:2043-2049` first-mention interning,
`parser.rs:3344-3346` bracket-attached-post-effect, `parser.rs:4024-4046`
member header pre-interned at 0): a poly word's `ty_var_names` are interned
in the **effect's mention order**, not the bound-list's written order.
`['It: Cursor 'T] ( 'T 'It['T] -- … )` has `'T`=0, `'It`=1.

The diagnostics twin `substitute_member_var` (`ground.rs:254`; callers at
`ground.rs:433` and `:1600` build error-text shape strings) has the same
missing Generic arm, so member sigs with concrete-ctor applications render
wrong in those messages.

The same roadmap sentence names a second, independent facet — poly bodies
**cross-calling** poly words over compound receivers — which is a *located*
fence today (probe E, S1-17.i at `crosscall.rs:346-352`;
`poly_cross_output`'s compound rejection at `crosscall.rs:387-396`), not a
loss. It does not gate the drain-fold/`fold` shapes and is **not** this
slice's fix (Ruling R4).

## Rulings (adjudicated)

- **R1 (direction).** Fix the render, not the diagnostics. Face 2 is a
  correctness gap: a program can check with a constructor-variable-typed
  value. Sharpened errors alone would leave the mis-typing either silent or
  merely rejected.
- **R2 (scope).** `render_member_decl` gains a `PolyType::Generic` arm:
  render `args` recursively through the existing `render` closure; carry
  `is_enum`/`idx`/`module`/`name` verbatim; carry `len_args` verbatim (S2-5:
  a member row declares no length variables, so a `Len::Var` there is
  unreachable by construction — the member-grammar gate at `parser.rs:429`,
  enforced at parse on every declared row — though representable in the type
  language (`Len::Var` renders as `'N<v>`, `ast.rs:845`), and carrying it
  verbatim is sound for every shape that can reach the arm). `substitute_member_var` gains the
  mirror arm for diagnostics, recursing args through its own documented
  total-rewrite semantics (S2-16/F6) — the twin has no bindings, so R3's
  two-tier rule is render-side only. No other dispatch route changes — the
  splice route (`ground_member_sig_via_theta` → `apply_subst`, whose
  `Generic` arm at `unify.rs:709-727` already substitutes args; the impl-side
  renderer `ground_member_poly` at `ast.rs:2381-2404` likewise already has
  the complete Generic arm this fix mirrors) and the mono route
  (`ast::ground_member_type` + concrete-effect fallback, generic-impl winner
  through `check_poly_call`) never touch `render_member_decl`. Probe H
  confirmed an `impl: Cursor for List` member checks clean today; only the
  bound-generic consumer fails.
- **R3 (unbound vars — the two-tier rule).** `unify_member_operand` has no
  `Generic` arm (`ground.rs:96-189`; arms through :186): a ctor-headed
  declared **input** (`Option['U]`, admitted bare-Var by
  `member_shape_is_supported`, `parser.rs:429-431`) matches only by the
  tail's structural equality (`ground.rs:187`), which records **no
  binding**. An unbound member variable inside `Generic` args therefore has
  two defined answers, keyed by whether the operands pinned it:
  - **Input-occurring — pinned by the admitting equality.** The dispatch
    only fires when the caller's slot arg is structurally equal to the
    declared input, i.e. the caller's variable with that exact id flowed
    through. Render **verbatim** (`PolyType::Var(v)`): the id IS the
    caller-space answer. These programs **compile today** — at the mono
    call site the CtorImage mint arm (`trait.rs:479-560`) re-grounds the
    obligation's slots and per-site unifies the impl member's grounded sig
    via `unify_poly_input`, whose `Generic` arm (`unify.rs:343-374`) binds
    the declared args positionally, exactly as an ordinary `Var` arm does
    (the channel that makes `mapover` work, `tests/phase7b_slice2.rs:295`)
    — so the verbatim tier keeps today-compiling programs compiling
    **byte-stably**. The unscoped `Var(var)` fallback the round-1 review
    caught would instead re-type the pinned payload as the header variable
    (a `* -> *` variable in an element position): breaking today-green
    bodies with residual mismatches, or silently re-typing internal
    consumption — the face-2 class this slice exists to kill.
  - **Output-only — never pinned.** Render `PolyType::Var(var)` (the
    dispatched header variable), exactly as the `Var` arm's existing
    fallback does. Such a row never compiles on either route: the
    generic-mint route fails closed at `compose_member_theta`
    (`unify.rs:55-62`, `first_unbound_sig_var` across inputs **and**
    outputs → `member_unbound_variable_error`), and the mono call-site
    route surfaces the unbound output at mono-seed
    (`poly_unbound_output_ty_error`, `trait.rs:395-399`, :541-560) — so
    this tier only shapes diagnostics on programs that never compile.
  Implementation: the `render` closure's unbound-`Var` branch consults a
  small `poly_type_mentions_var` helper over the member row's declared
  inputs (which `render_member_decl` gains as a parameter — see Fix).
- **R3.5 (GenericVariant posture).** The new `Generic` arm does not add a
  `GenericVariant` case; `GenericVariant` stays under the retained
  `other => other.clone()` catch-all: no production route reaches one
  (unconstructible outside an eliminator arm's own scrutinee — the R3.5
  convention unreachables at `trait.rs:1157`, `:1294`, `crosscall.rs:469`;
  sole constructor `generic_variant_type`, called in production only by the
  `~[ ( … ) … ]` mint at `poly.rs:2740`), and no caller relies on the
  catch-all's behaviour for it — the kept posture is pinned by a unit rather
  than relied on by any caller (see Units). No new handling and no new
  assertion is introduced for it in these two functions (they retain their
  `other => other.clone()` tail exactly as today, which the Generic arm now
  shadows for the one reachable compound shape). Adjudged out of scope:
  adding a bespoke `unreachable!` in a pure-render helper would be
  untestable ceremony.
- **R4 (fences — what does NOT move).** The cross-call App fence (S1-17.i)
  and `poly_cross_output`'s compound-output rejection stay byte-identical,
  tests included; their lift is a follow-up slice (the remaining facet of the
  roadmap sentence; no S6d shape needs it). **No S6d work lands here**: no
  sentinel-substitution grounding, no slice impl, no `PolySlot` provenance
  (`Deriv`). The S6d-6 concrete fence, the S2-6 fence, and all S8/S8b/S8c
  member machinery are untouched.
- **R5 (diagnostics fence).** The `note: declared ( -- )` placeholder in
  poly-body underflow notes (`ctx.effect()` is the poly word's empty
  placeholder) is **not** fixed here — many existing goldens pin it
  byte-exactly (`tests/phase7b_slice8b.rs` ×5+, `tests/phase0.rs`,
  `tests/phase7_slice3c.rs`), so it is a deliberate multi-golden text change
  deferred to its own tiny slice (golden-churn inventory required first). No
  diagnostic text changes except where the fix makes a previously-garbled
  member-sig rendering correct (G7); those messages are today
  unreachable-with-correct-content, so no existing golden pins their Generic
  shape.
- **R6 (behavioral fence — the coincidence contract).** Programs that check
  today **by id coincidence** (`fold`/`for_each` over List and Range, and any
  consumer whose element variable sits at id 1 **and is the operand's App
  argument**, so `unify_member_operand`'s App arm binds `(1, Var(1))` —
  `lib/core/iterator.sth:64`, `:73`) must produce **byte-identical rendered
  types** post-fix: for those shapes the Generic arm renders member `Var(1)`
  through a bindings map that already holds `(1, Var(1))`, so the arm is a
  no-op relative to today's verbatim clone. A consumer whose element sits at
  id 1 but is NOT the operand's App argument is face-2 mis-typed today and
  gets no byte-identity promise. `tests/phase7b_slice8.rs`'s List and Range
  consumers are the named canaries; the full existing suite (3463/0) is the
  global regression gate.
- **R7 (no new capability).** The fix is a rendering correction inside the
  existing unification/bindings machinery. **No new `PolyType` variants, no
  new unification rules, no changes to `unify_member_operand` or the
  obligation record.** Every shape `member_shape_is_supported` admits
  renders (R3's two-tier rule covers the unbound cases). The render changes
  observable behaviour only where today's render was already the leak: a
  structurally-pinned compiling program renders byte-identically (tier 1 —
  the per-site mint binds the pinned var either way), an output-only row
  never compiles on either route (tier 2), and the one remaining delta —
  a hybrid shape whose App-typed output arg is a Generic-input-pinned var —
  renders **more** correctly post-fix (today's existing unbound-`Var`
  fallback re-types it as the header variable), which is a face-2 fix, not
  a regression; no existing golden pins the old rendering (round-2 sweep of
  `lib/` + `tests/`).

## Fix

`src/check/poly/ground.rs`, one band, no other files edited in phase 1.

1. **`render_member_decl` (`ground.rs:197`)** — add before the
   `other => other.clone()` tail:

   ```rust
   PolyType::Generic { is_enum, idx, module, args, len_args, name } => PolyType::Generic {
       is_enum: *is_enum,
       idx: *idx,
       module: *module,
       args: args.iter().map(render).collect(),
       len_args: len_args.clone(),
       name: *name,
   },
   ```

   `args` recurse through the existing `render` closure (so nested
   `Generic`-in-`Generic` and `App`-inside-`Generic` args render through the
   same bindings). Header identity and `len_args` are carried verbatim
   (R2/S2-5). (Field types per `ast.rs`'s `PolyType::Generic`: `is_enum:
   bool`, `idx`/`module: u32`, `name: &'static str` — all behind the match's
   shared reference, so the arms use `*`; `clone()` on the `Copy` fields
   would trip `clippy::clone_on_copy` under the gate.)

   **R3 plumbing.** `render_member_decl` gains the member row's declared
   inputs as a parameter (`inputs: &[PolyType]`); the sole production caller
   (`poly_trait_member_call`'s output push, `ground.rs:1678`) passes
   `&m.sig.inputs`, which it already has in hand; the test caller
   (`ground.rs:1933`) passes its row. The `render` closure's unbound-`Var`
   branch becomes the two-tier rule:

   ```rust
   // Unbound: pinned by the admitting structural equality on a
   // ctor-headed input (ground.rs:187) renders verbatim — the id IS the
   // caller's variable (R3, tier 1). Never pinned: the header-variable
   // fallback (R3, tier 2).
   PolyType::Var(v) => match bindings.iter().find(|(id, _)| id == v) {
       Some((_, pt)) => pt.clone(),
       None if inputs.iter().any(|i| poly_type_mentions_var(i, *v)) => {
           PolyType::Var(*v)
       }
       None => PolyType::Var(var),
   },
   ```

   (Sketch — the existing branch shape governs; the delta is the
   `None if` arm.) New helper beside the band, mirroring
   `first_unbound_sig_var`'s variant walk (`unify.rs:70+`):

   ```rust
   fn poly_type_mentions_var(pt: &PolyType, v: u32) -> bool
   ```

   — a structural walk over every variant (App head+args, Generic args,
   Quotation ins/outs, Ref/OwnedCell/Array elements, `Len::Var` inside
   Array/Generic).

2. **`substitute_member_var` (`ground.rs:254`)** — mirror arm for
   diagnostics. The twin has no bindings: its documented semantics are a
   total rewrite (every member variable renders as the dispatched variable,
   S2-16/F6, `ground.rs:246-252`), so the mirror arm recurses args through
   the twin itself and R3's two-tier rule does not apply here:

   ```rust
   PolyType::Generic { is_enum, idx, module, args, len_args, name } => PolyType::Generic {
       is_enum: *is_enum,
       idx: *idx,
       module: *module,
       args: args.iter().map(|a| substitute_member_var(a, var)).collect(),
       len_args: len_args.clone(),
       name: *name,
   },
   ```

   (Field names verified against `PolyType::Generic` at `unify.rs:709-714`:
   `is_enum`, `idx`, `module`, `args`, `len_args`, `name`.)

No change to `poly_trait_member_call`, the obligation record, or any fence.

## Goldens (`tests/phase7b_slice12.rs`)

Harness conventions from `tests/phase7b_slice8.rs`:
`single_file_hosted`/`build_ok`/`build_run_keep`/`build_error_located`
(non-zero exit, `error:` stderr, no `panicked`), naming
`thing_condition_expected`. Fixtures are committed (`probes/s12_*.sth`);
frozen pre-fix stderr in `probes/s12_baseline.md`. Any *new* diagnostic bytes
are pinned from the live binary at implementation time (G7 only).

| Golden | Fixture | Today | Post-fix |
| --- | --- | --- | --- |
| `bound_generic_drain_fold_option_row_checks_clean` (G1) | `s12_a_drain_fold_option_row.sth` | underflow at `add` ("holds 1") | checks clean — golden adds `: main ( -- ) ;` for the link step, then `build_ok` |
| `add_directly_on_some_payload_reports_holds_zero_no_more` (G2) | `s12_a2_add_holds_zero.sth` | underflow ("holds 0") — S6d-4's exact message | **still a check error, honestly so**: the add genuinely consumes the remainder (not a number), so the message shifts to the underflow shape for real operands ("holds 1", patch-validated). Golden: `build_error_located` asserting no `'?1`/garbled id anywhere and no `panic`; today's bytes stay the S6d-4 provenance anchor |
| `member_output_residual_renders_caller_space_option` (G3) | `s12_b_member_output_residual.sth` | mismatch names `Option['?1]` | byte-pinned error text: "body leaves `i64 'It[i64] Option[i64]`, but the declared outputs are `i64 Option[i64]`" — the point is the correct caller-space rendering (no `'?1`); the fixture's declared outputs deliberately omit the remainder, so the build_ok twin is G5 |
| `some_payload_types_as_the_element_variable_not_the_constructor` (G4) | `s12_c_arm_payload_mistype.sth` | arms disagree (`'E` vs `'It`) | checks clean — golden adds `: main ( -- ) ;`, then `build_ok` (payload is `'E`; arms agree) |
| `leaked_member_id_renders_through_bindings_in_range` (G5) | `s12_f_id_coincidence_space.sth` | `Option['It]` vs declared `Option['T]` | `build_ok` (Generic arm renders through bindings; patch-validated) |
| `end_to_end_list_backed_cursor_drain_prints_6` (G6, exit criterion) | golden copy of `s12_h_end_to_end_list_drain.sth` with the house spelling: `: mkempty ( -- List[i64] ) Nil ;` added and `main`'s bare `Nil` calls rewritten to `mkempty` (per `tests/phase7b_slice8.rs:164-171`). Do **not** add the `hosted::show` import: `single_file_hosted` prepends it (`tests/phase7b_slice8.rs:54-69`) and a duplicate import collides in the seen-map (`declarations.rs:894-899`). The committed probe fixture stays byte-frozen for its pre-fix baseline | `drain` fails (impl checks clean) | `build_run_keep`, stdout `6`, exit 0 — patch-validated |
| `member_sig_diagnostics_render_generic_args_in_caller_space` (G7) | member-dispatch error rendering a member sig with `Option['T]` (via `substitute_member_var` callers `ground.rs:433`/`:1600`); exact fixture chosen at implementation time so the error fires for an operand-shape reason, not the leak | garbled member-space rendering | member type renders in caller space, byte-pinned from the live binary |
| `cross_call_app_fence_stays_byte_identical` (G8, R4 pin) | `s12_e_cross_call_app_fence.sth` | S1-17.i located rejection | byte-identical (fence untouched) |
| `iterator_consumers_survive_the_render_fix` (G9, R6 canary) | `tests/phase7b_slice8.rs` List + Range `for_each`/`fold` | green | green (R6's mechanism argument is the symbol-stability argument for the `(1, Var(1))` class; the canaries' green is the assertion — no symbol-diff harness is wired) |
| `existing_suite_green` (G10, global gate) | full suite | 3463 / 0 | 3463+ / 0 |
| `structurally_pinned_member_input_stays_compiling_byte_identically` (G11, R3 tier-1 pin) | new fixture, shape given in the G11 section below | build_ok — the structurally-pinned program compiles today (the per-site mint binds the pinned var via `unify_poly_input`'s Generic arm, `unify.rs:343-374`) | build_ok, byte-identical behaviour — the golden pins that the fix does NOT re-type the pinned payload (the unscoped fallback would break exactly this program) |

G9 is asserted by the existing `tests/phase7b_slice8.rs` goldens staying
green (not duplicated into `phase7b_slice12.rs`); the slice12 file cites them
by name in a comment. G10 is `cargo test`.

### G11 — the structurally-pinned subclass (R3 tier-1 pin, review-round addition)

`unify_member_operand` has no `Generic` arm, so a ctor-headed declared input
(`Option['U]`) dispatches at the poly-body route only by the tail's
structural equality (`ground.rs:187`) — no binding. But such programs
**compile today**: at the mono call site the CtorImage mint arm
(`trait.rs:479-560`) re-grounds the obligation's slots to concrete types and
per-site unifies the impl member's grounded sig via `unify_poly_input`,
whose `Generic` arm (`unify.rs:343-374`) binds the declared args
positionally — the same channel that makes `mapover` work
(`tests/phase7b_slice2.rs:295`). G11 pins exactly that (round-2 review:
the originally-specced build_error_located premise was false — neither
`no_candidate_fits_operands` nor `member_unbound_variable` fires).
Fixture shape (exact spelling at implementation time): a trait
`Cursor['S: * -> *]` with a member row declaring a ctor-headed input over a
member local, e.g. `accept ( 'S['T] Option['U] -- Option['U] )` (member
space: header 0, 'T 1, 'U 2 — admitted bare-Var by
`member_shape_is_supported`, `parser.rs:429-431`; impl body can be `swap
drop`); a poly caller `: ch ['C: Cursor 'D 'E] ( 'C['D] Option['E] --
Option['E] )` (the bound bracket is required for member resolution — every
dispatching fixture carries one — and probe F proves it does not perturb
effect-mention id order, so 'C=0/'D=1/'E=2 hold) whose body is just
`accept` (caller 'E lands at id 2, coinciding with 'U); a `main` calling
`ch` concretely through the impl. Today: build_ok (the body checks clean on
the verbatim render; the mint binds 'U per-site). Post-fix: build_ok with
byte-identical behaviour — the R3 verbatim tier keeps the body render
byte-stable. The golden (`build_ok`, or `build_run_keep` exit 0) asserts
the fix does NOT re-type the pinned payload: the unscoped `Var(var)`
fallback would break exactly this program with a body-level residual
mismatch. (The non-coincidence variant — caller slot arg id ≠ member local
id — fails at the operand check pre-render on both sides with
`trait_member_operand_error` (`ground.rs:1662` → `poly.rs:5059`), a
message no other S12 golden pins byte-level; no golden needed here since
the twin fails identically pre-render on both sides.)

**Fix validated by throwaway patch (260911).** The spec's two arms were
applied as a scratch patch (reverted after; the committed tree carries only
recon docs): the full probe battery ran against it, probe H's exit-criterion
program built and printed `6`, and the full suite passed 3463/0 — the R6
coincidence contract holds with zero regressions. Measured corrections
folded into the table above: G1/G4/G5 need a bare `: main ( -- ) ;` for the
link step (check-level pass is the fix's verdict); G2's add is genuinely
ill-typed (remainder + payload) and stays a located error post-fix — the
"holds 0" bytes are the pre-fix anchor, not a target state; G3's fixture
declared outputs intentionally omit the remainder, so its post-fix golden is
the byte-pinned corrected-rendering error text.

## Units (beside the changed sites, `src/check/poly/ground.rs` `tests`)

- `render_member_decl_renders_generic_args_through_bindings` — `Option['T]`
  member output, bindings `(1, Concrete(i64))` → `Option` args
  `[Concrete(i64)]`.
- `render_member_decl_input_pinned_unbound_var_renders_verbatim` — R3 tier
  1: row with a ctor-headed input over member var 2, bindings lacking 2,
  output `Generic{args:[Var(2)]}` → args render `[Var(2)]` verbatim, not
  `Var(var)`.
- `render_member_decl_generic_falls_back_to_the_bound_variable` — R3 tier
  2: unbound, output-only member var in args → `PolyType::Var(var)`.
- `render_member_decl_generic_carries_len_args_verbatim` — concrete
  `len_args` unchanged (S2-5 unreachable `Len::Var` documented in the test).
- `render_member_decl_renders_app_inside_generic_args` — `Step['T 'It['T]]`:
  nested `App` inside Generic args recurses through the bindings.
- `render_member_decl_generic_variant_posture_stays_verbatim_clone` — pins
  the R3.5 posture: a hand-built `GenericVariant` passed directly to
  `render_member_decl` clones through verbatim (kept — no bespoke arm); the
  reachability argument (unconstructible outside an eliminator arm's
  scrutinee) lives in a code comment beside the arm, citing
  `trait.rs:1157`. The unit pins the verbatim-clone posture so a future arm
  addition cannot silently change it.
- `poly_type_mentions_var_walks_all_variants` — the R3 helper: detection
  through nested App/Generic/Quotation/Ref/Array positions and `Len::Var`.
- `substitute_member_var_renders_generic_args_for_diagnostics` — the twin
  arm, mirroring the first four (total-rewrite semantics, no two-tier).

Target ~8–12 units, naming `thing_condition_expected`.

## Housekeeping (phase 2)

- **`docs/roadmap/P7b-higher-kinded-types.md`** — the S6d / S6d-PREREQ
  entries' "standing S8-era limitation" language: replace it with the
  current-design statement (the symbolic member-output render is
  Generic-complete; bound-generic slice consumers dispatch through member
  rows without leaking member-space ids) and re-point the S6d gate
  accordingly. State current design only, no change-history narration (house
  convention).
- **`docs/roadmap/ROADMAP.md`** — the stale P7b row still marks S6d-PREREQ
  "specced" though it landed; correct it, and add the S12 row.
- Re-run the growth-structure signals at phase exit against
  `src/check/poly/ground.rs` (the only edited band). Two small render arms
  and a helper are not expected to trip a split; record the re-check either
  way.

## Open questions — adjudicated / scoped out

- **Cross-call App lift timing.** Deferred (R4) to its own slice,
  **[SOO-60](https://linear.app/ordfruma/issue/SOO-60)**, with its own probe round first. Becomes
  S6d-gating only if S6d's consumer work needs poly-word-to-poly-word calls
  over compound receivers; the hard part is the interaction with R6's
  growth ban (a bare-var callee input facing an App-typed operand), not
  App-vs-App matching. **Out of scope for S12.**
- **`GenericVariant` in a member output.** Adjudicated by R3.5: no bespoke
  arm, no assertion in these two pure-render helpers; unconstructible outside
  eliminator arms, unreachable here. Documented, not exercised.
- **`note: declared ( -- )` placeholder.** Deferred (R5) to its own tiny
  slice, **[SOO-61](https://linear.app/ordfruma/issue/SOO-61)**; needs a golden-churn inventory first
  (`phase7b_slice8b`'s five pins are the start). **Out of scope for S12.**
- **Poly-body borrow propagation for slice remainders** (`PolySlot` has no
  `Deriv`). Recorded for **S6d**, not S12; lands where S6d's slice consumers
  hit the exclusivity story.

## Phases (JSON)

```json
[
  {
    "phase": 1,
    "focus": "In src/check/poly/ground.rs: add the PolyType::Generic arm to render_member_decl and the mirror (total-rewrite) arm to substitute_member_var (R2); give render_member_decl the declared-inputs parameter and make the unbound-Var branch two-tier per R3 (new poly_type_mentions_var helper). Units per the spec Units list. Phase gate: new units green; existing suite stays green per R6 (R6 canaries byte-stable).",
    "effort": "S",
    "difficulty": "low"
  },
  {
    "phase": 2,
    "focus": "Goldens G1-G8 and G11 in tests/phase7b_slice12.rs (G9 = the named phase7b_slice8 canaries stay green; G10 = full suite); exact new-error bytes pinned from the live binary (G7, G11). Housekeeping: docs/roadmap/P7b-higher-kinded-types.md states the limitation as lifted (current design, no change narration; S6d gate re-pointed) and ROADMAP.md's stale P7b row corrected + S12 row added; growth-structure re-check on ground.rs recorded.",
    "effort": "S",
    "difficulty": "low"
  }
]
```

## Review loop

Round 1 (260911, two fresh-context read-only lanes — technical soundness,
implementability): soundness NEEDS_CHANGES, implementability APPROVED. All
findings adjudicated and folded in: R3 rebuilt as the two-tier rule (the
unscoped `Var(var)` fallback mis-renders the structurally-pinned subclass —
the soundness lane's MAJOR, verified against `unify_member_operand`'s tail
at `ground.rs:187`); G11 added to pin that subclass; R6's coincidence
parenthetical tightened (element var at id 1 AND the operand's App
argument); R3.5 prose repaired and its unit renamed; R2's `Len::Var` claim
corrected (unreachable by construction, not unrepresentable); G6's golden
assembly made harness-accurate (`single_file_hosted` prepends the
`hosted::show` import — the golden src must not repeat it); G9's un-wired
symbol clause dropped; G7's error-family attribution corrected; Phases JSON
made 1:1 with the narrative and phase gates explicit; round-count wobble
fixed (9 fixtures, rounds A–G + probe H).

Round 2 (260911, fresh-context verification of the amendments):
NEEDS_CHANGES on the additions — the two-tier **fix design** verified
cleanly (mechanics 1a–1e, JSON, cross-doc consistency, review-loop
accuracy all confirmed), but G11's original premise was refuted by a
mint-path trace: the CtorImage arm (`trait.rs:479-560`) binds the
structurally-pinned var per-site via `unify_poly_input`'s `Generic` arm
(`unify.rs:343-374`), so the described program **compiles today** and the
build_error_located design was rebuilt as a build_ok byte-stability pin;
R7's "cannot complete a mint" mechanism claim and R3 tier-2's "masks the
honest dispatch error" contrast were inverted and rewritten around the
pinning/byte-stability argument (the unscoped fallback would break a
today-compiling program, not mask a dispatch error); G9's symbol clause
purged from the remaining docs; route-specific tier-2 citation added
(`poly_unbound_output_ty_error`, `trait.rs:395-399`); citation
imprecisions and the G3 backtick glitch fixed. Folded in above.

Round 3 (260911, fresh-context consistency check of the rewrites):
**APPROVED** — all eight round-2 findings verified folded, no blocking or
major findings. Two informational NITs folded: G11's caller sketch gained
the required Cursor bound bracket (`['C: Cursor 'D 'E]`, id order
preserved per probe F) and the non-coincidence twin's error family named
(`trait_member_operand_error`); the build_ok byte-stability pin, phase
JSON, unit lists, and cross-doc consistency all re-verified. The spec is
cleared for `/implement`.
