# P7b.S7 spec — quotation effects over type constructors (the `call` extension)

> Delivery spec. Companion frozen docs: [slice7-brief](./slice7-brief.md)
> (problem, adjudicated mechanism, scope), [slice7-paper-tests](./slice7-paper-tests.md)
> (complete fixtures G1-G5 + regression pin, measured-before columns),
> [slice7-probes](./slice7-probes.md) (verbatim probe log P1-P5 + precedent
> check). Read [slice2-spec](./slice2-spec.md) "Deliberate limitations" (S2-15.d)
> for the residual this closes, and [slice9-spec](./slice9-spec.md) /
> [slice10-spec](./slice10-spec.md) for the sibling house style. Base: `a9eca84`
> (P7b.S10 recon docs merged; suite green).

## Why

S2 shipped `Functor.map` (a quotation whose declared effect mentions a plain-slot
HKT output, `'F['U]`, grounded via `apply_subst`'s `App` arm) but fenced the
richer container traits at **parse time**. Two independent, pre-S7 fences block
the next rung of the ladder:

1. **`Monad.bind`** — `bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] )` puts a
   `PolyType::App` *inside* a quotation row. `app_in_member_quotation_row_error`
   (`src/parser.rs:452`) is raised at `src/parser.rs:3966`, once
   `member_shape_is_supported` (`src/parser.rs:379`, called from the
   post-parse loop at `src/parser.rs:3950`) returns false and the row-scoped
   dispatch at `src/parser.rs:3965` isolates the row-nested case — this
   rejects the whole `trait:` declaration before `check`, before dispatch,
   before `call` is reached (probes P1/P2). The trait never becomes a
   `TraitDecl`; there is no dispatch-time behaviour to fix, only a shape to
   admit and then ground.
2. **`Applicative.ap`** — `ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] )` puts a
   quotation *as the argument* of a variable-headed `App`.
   `app_arg_quotation_error` (defined `src/parser.rs:2630`, raised in
   `parse_poly_app_arg` at `src/parser.rs:5213`)
   rejects this unconditionally — an S1-era fence unrelated to traits (probes
   P3/P4). A **named** constructor's application list (`Box[[i64 -- i64]]`) has
   no such fence (probe P5): the restriction is specific to a type-*variable*
   head, not to "constructor applied to a quotation" in general.

**Revised finding (this pass, superseding the original framing below).** The
original premise — that `poly_call_abstract_quotation_param`
(`src/check/poly.rs:4426`) needs new logic, reusing `PolyType::App`'s
structural `Eq` at `src/check/poly.rs:1352` — is wrong on both counts.
`src/check/poly.rs:1352` sits inside `unify_member_operand`
(`src/check/poly.rs:1335`), a one-way *unifier* with a mutable bindings
accumulator whose own doc comment says it **replaces** a prior
structural-equality check; it is not structural equality, and it is not what
`poly_call_abstract_quotation_param` uses. More importantly,
`poly_call_abstract_quotation_param` never sees an unresolved App at all: an
`impl:` member's body is grounded against its target **before** any
body-check runs. For a generic target (e.g. `impl: Monad for Option`, whose
desugared pattern is `PolyType::Generic`, not `Concrete`), that grounding is
`ground_member_poly` (`src/ast.rs:2262`), whose existing `App` arm
(`src/ast.rs:2316`, the head-0 dissolve) already recurses correctly through
its `Quotation` arm — so `'F['U]` inside `bind`'s row is dissolved into a
plain `Generic{Option, [Var(U)]}` at parse time, before the member word's own
body is ever poly-checked. By the time `poly_call_abstract_quotation_param`
runs (dispatched at `src/check/poly.rs:3084`, when `call` pops a
still-abstract, non-literal `PolyType::Quotation` operand), the row-slot is
already a `Generic`, never an `App` — and that function's existing input
check (`stack[base+i].pt != want`, `src/check/poly.rs:4441`) and output push
(`out.clone()`, `src/check/poly.rs:4453`) already treat any `PolyType`
uniformly; a `Generic`-shaped slot needs no special-casing. The "`'F` still
abstract" branch OQ-1 worried about is also structurally impossible:
`ground_member_poly`'s `App` arm rejects a non-`Generic` target outright
(`member_app_abstract_target_error`, `src/ast.rs:2054`) before any body-check
can run, so an impl target can never leave `'F` undissolved. **Net: zero new
code is needed in `poly_call_abstract_quotation_param`, `apply_subst`, or
`ground_member_poly`.** What genuinely needs verification — the original
adjudicated mechanism below, which the resolution keeps — is the *caller*
dispatch path: a generic body calling `bind` through a `'T: Monad` bound goes
through `unify_member_operand`/`render_member_decl`
(`src/check/poly.rs:1335`/`:1407`), which already has working `App`- and
`Quotation`-recursing arms, exercised today by a non-inline `twice['F: Functor
'T]` dispatching `map` through a shared bound — verified against two real
tests: `s2_shared_bound_golden_unchanged` (`tests/phase7b_slice3.rs:606`,
fixture-twin types) and
`shared_bound_poly_word_dispatches_over_the_real_core_option`
(`tests/phase7b_slice4.rs:199`, the real `core::option::Option`) — but never
yet by a shape where the App sits *inside* a quotation row, since that row
shape could not parse before now. See REQ-5 below.

---

**Original framing (retained for the citation trail; superseded above).** The
adjudicated grounding mechanism (brief, probe precedent check) was thought to
be **not** `apply_subst`'s `Subst`-based `App` arm but
`poly_call_abstract_quotation_param` (`src/check/poly.rs:4426`), reasoned to
ground an abstract quotation parameter (rows carrying unbound `PolyType::Var`s)
by **structural equality**, no `Subst` built or consulted (the L1 "variables
stay rigid" discipline), with `'F` still abstract at the member's own
body-level `call`. The verification round above found this function is not
even reached with an unresolved App, and needs no change regardless.

Scope is the quotation-effect grounding paths only: no new trait surface, no new
declaration syntax (brief). `Applicative.ap` ships **only if** the same extension
grounds it for free; it is not an independent exit criterion.

## Adjudicated mechanism (from the probe round)

- Fence #1 lives in the shape gate: `parse_trait_member_effect`
  (`src/parser.rs` ~3950) calls `member_shape_is_supported` (~379), whose
  `Quotation` arm (~391) rejects any row where `member_quotation_row_mentions_app`
  (~421) is true. It scans both `ins` and `outs` (~428), so it is row-position
  blind (probe P2).
- Fence #2 lives in `parse_poly_app_arg` (`src/parser.rs` ~5206): the moment a
  `[` appears as a type-application argument it errors (`src/parser.rs:5213`),
  upstream of `member_shape_is_supported` entirely. It fires for *any* `'F[...]`
  application, trait or not (probes P3/P4). There is a second call site at
  `src/parser.rs:7846`.
- The named-constructor path (`Box[...]`) goes through a different parser
  (`Generic`, not `App`) and already admits a quotation argument (probe P5); it
  is the existence proof that "constructor over a quotation" is representable —
  the gap is that the abstract-head grammar was never given the same freedom.
- Load-bearing invariants that assert the fenced shape *cannot exist* were
  audited (this pass, verified against source): `ground_member_type`
  (`src/ast.rs:2147`) has an `App` arm that is a hard `unreachable!()`
  (`src/ast.rs:2185`), not a comment — but it stays correctly unreachable, since
  `fence_member_app_against_concrete_target` (`src/ast.rs:2116`) already fences
  an App anywhere in a member's signature, rows included, before a concrete
  target ever reaches `ground_member_type` (its own doc says "the scan is
  total anyway", and `member_ty_mentions_app`, `src/ast.rs:2095`, does recurse
  into `Quotation` rows already — only its "Quotation rows never carry an App"
  comment line needs a one-line correction post-REQ-1, the code needs none).
  `apply_subst`'s two `sig.ty_var_names[*head as usize]` indexings
  (`src/check/poly.rs:10454`, `:10497`) are safe by construction: an `App`'s
  `head` id is always allocated within the same `PolySig`'s own variable space
  it is matched against (`apply_subst` is always called with the enclosing
  word's own `sig`), so the index can never point outside `ty_var_names` —
  this invariant was implicit and is now stated explicitly (REQ-3). Also
  audited: `poly_type_app_head`'s row-blind-by-design contract
  (`src/parser.rs:902-913`) is unaffected by REQ-1 (it must stay row-blind — it
  is the predicate that isolates the row-nested case in the first place) —
  but `app_in_member_quotation_row_error`'s message ("keep quotation rows
  App-free") becomes wrong for the one case that still fires post-REQ-1 (a
  member-*local*-headed App in a row, still unsupported per
  `member_shape_is_supported`'s own `App` arm, `*head == 0`) and needs
  correcting (REQ-3).
- **Resolved (this pass): no genuinely new logic is needed at all.** See the
  "Why" section's revised finding above — `ground_member_poly`'s existing `App`
  arm already dissolves a row-nested App into a `Generic` at parse time, before
  `poly_call_abstract_quotation_param` (or `apply_subst`) ever runs, and the
  "still fully abstract `'F`" case cannot occur (a non-`Generic` impl target is
  already a located error). What remains is verifying the already-shipped
  caller-dispatch path (`unify_member_operand`/`render_member_decl`) against
  this specific shape, not writing new grounding code. **OQ-1 is resolved, not
  an open risk** (see Open Questions).

## Requirements

- **REQ-1 (lift fence #1, brief scope 1).** `member_shape_is_supported`'s
  `Quotation` arm admits a `PolyType::App` inside a member's quotation row: the
  `member_quotation_row_mentions_app` rejection is narrowed so that a
  kind-correct App inside a row parses to a `TraitDecl` with the declared effect
  intact. The lift is **narrow**: it admits "App inside a row", not "anything
  inside a row" (see REQ-4). **Deliberately reverses S2-15.d**: two existing
  tests pin the exact shape this lifts and must be retired/inverted in the
  same change, not left to rot as false negatives —
  `app_inside_member_quotation_row_is_fenced`
  (`tests/phase7b_slice2.rs:204`) and
  `parse_trait_decl_app_inside_member_quotation_row_is_fenced`
  (`src/parser.rs:11861`). `docs/roadmap/P7b/slice2-spec.md`'s "Deliberate
  limitations" S2-15.d line is updated to note the supersession (a factual
  correction to a landed slice's own spec, not history narration —
  CLAUDE.md's "no history" rule targets ROADMAP/DESIGN prose, not a
  companion spec's accuracy). Traces to G1; unit
  `member_quotation_row_admits_app_expected` beside the changed parser code.
- **REQ-2 (lift fence #2 for the member-declaration path, brief scope 2).**
  `parse_poly_app_arg` admits a quotation-typed argument to a variable-headed
  `App` at the trait-member-declaration site. If the fix lands in the shared
  `RawTy::App` parser path (fence is unconditional, not trait-scoped per P4),
  that is acceptable and **not** scope creep — it is the same code path either
  way (brief "explicitly out of scope"). It is not a goal to lift it separately
  at every non-member signature site. Traces to G5 (bonus); unit
  `poly_app_arg_admits_quotation_at_member_site_expected`. **Gated by OQ-2** —
  if G5 does not ground for free, this requirement is deferred (see Open
  Questions / Phase 4).
- **REQ-3 (audit invariant-bearing guards, brief scope 3).** Every function
  whose comment/contract asserts the fenced shape cannot exist is audited and,
  where it now can, corrected so the invariant is not silently falsified.
  Audited this pass, with conclusions (no guesswork left for the implementing
  phase):
  - `ground_member_type`'s `App` arm (`src/ast.rs:2185`, a hard
    `unreachable!()`) stays correctly unreachable —
    `fence_member_app_against_concrete_target` (`src/ast.rs:2116`) already
    fences an App anywhere in a signature, rows included
    (`member_ty_mentions_app`, `src/ast.rs:2095`, already recurses into
    `Quotation` rows), before a concrete target ever reaches it. **No code
    change**; only its doc comment's "Quotation rows never carry an App"
    line (now stale prose, not a stale invariant) needs a one-line correction
    noting S7 lifts the row-nested case for *generic* targets (concrete
    targets are unaffected — the fence there is unconditional).
  - `apply_subst`'s two `sig.ty_var_names[*head as usize]` indexings
    (`src/check/poly.rs:10454`, `:10497`) are safe by construction: an App's
    `head` id is always allocated within the same `PolySig`'s own variable
    space it is matched against. **No code change**; state this invariant
    explicitly in the function's doc comment so a future reader does not have
    to re-derive it.
  - `poly_type_app_head`'s row-blind contract (`src/parser.rs:902-913`) is
    **unaffected** — it must stay row-blind; it is the predicate that
    isolates the row-nested case for REQ-1's own dispatch. No change.
  - `app_in_member_quotation_row_error`'s message ("keep quotation rows
    App-free", `src/parser.rs:452`), reached via the two-conjunct dispatch at
    `src/parser.rs:3965` (`member_quotation_row_mentions_app(t) &&
    poly_type_app_head(t).is_none()`), becomes **actively wrong** for the one
    shape that still fires post-REQ-1: a member-*local*-headed App inside a
    row (e.g. `[ 'T -- 'G['U] ]` where `'G` is not the trait's own var) is
    still unsupported (`member_shape_is_supported`'s `App` arm requires
    `head == 0`, `src/parser.rs:386`), but the row is no longer required to
    be "App-free" in general. **Needs a corrected message** (e.g. "an App
    inside a quotation row must be headed by the trait's own type variable").
  - New this pass, beyond the brief's original three: `ground_member_poly`'s
    `App` arm (`src/ast.rs:2316`) already recurses correctly when reached
    through its own `Quotation` arm (both existed pre-S7, but the combination
    — an App nested inside a row, grounded via the generic-target path — was
    unreachable and so untested). **No code change; a new unit is needed**
    (REQ-5) to cover the previously-impossible combination.

  Traces to G1/G4; units beside each corrected comment/message; a new unit for
  `ground_member_poly`'s row+App combination (see REQ-5).
- **REQ-4 (kind-correctness preserved, regression shape).** Lifting fence #1 is
  not a blanket admit: a kind-incorrect use inside a now-admitted quotation row
  (e.g. a bare `* -> *` variable used as a standalone type) still produces a
  **located** error, not silent acceptance. Whether this reuses an existing
  kind check re-run post-fence or needs new logic is **OQ-3** (resolved: it
  reuses `arrow_var_used_bare_error`, unaffected by the row fence). Traces to
  **G4a** (kind-check regression pin; G4a alone cannot witness the row-fence's
  narrowness, since it fails on kind-checking before the row fence is ever
  reached) **and G4b** (the actual row-fence-narrowness witness: a
  member-local-headed App in a row, still rejected post-REQ-1 via
  `member_shape_is_supported`'s `head == 0` gate); unit
  `kind_incorrect_app_in_quotation_row_is_located_error`.
- **REQ-5 (verify the grounding — resolved, no new logic, brief scope 4).**
  **Revised (this pass): this requirement no longer introduces new grounding
  code.** The verification round found `poly_call_abstract_quotation_param`
  (`src/check/poly.rs:4426`) never sees an unresolved App at all —
  `ground_member_poly`'s existing `App` arm (`src/ast.rs:2316`) dissolves a
  row-nested App into a plain `Generic` at parse time, before the member's
  own body is ever poly-checked, and the "`'F` still fully abstract" case is
  structurally impossible (a non-`Generic` impl target is already a located
  error, `member_app_abstract_target_error`, `src/ast.rs:2054`). REQ-5 is now
  a **verification-only** requirement, covering two already-shipped
  mechanisms against the one shape they have never been exercised on (an App
  nested inside a quotation row):
  1. The **callee** path: `ground_member_poly`'s `Quotation` arm
     (`src/ast.rs:2305`) recursing into its own `App` arm (`src/ast.rs:2316`)
     correctly dissolves `bind`'s row-nested `'F['U]` into
     `Generic{Option, [Var(U)]}` when grounding a generic `impl:` target.
     Unit: `ground_member_poly_quotation_row_app_dissolves_to_target_generic`.
  2. The **caller** path: `unify_member_operand`/`render_member_decl`
     (`src/check/poly.rs:1335`/`:1407`, already shipped, exercised today by
     `s2_shared_bound_golden_unchanged`, `tests/phase7b_slice3.rs:606`, and
     `shared_bound_poly_word_dispatches_over_the_real_core_option`,
     `tests/phase7b_slice4.rs:199`) correctly unify/render a `bind`-shaped
     App-in-quotation-row member signature when a generic caller dispatches
     through a `'T: Monad` bound — the same arms, a new shape. Units:
     `unify_member_operand_app_row_slot_binds_dispatched_ctor`,
     `render_member_decl_app_row_slot_renders_into_caller_space`.

  No `Subst`-based path is introduced (none was ever reachable here); L1
  rigidity holds throughout, unexercised by any new code. Traces to G2/G3.
- **REQ-6 (dispatch + IR, brief scope 5 / roadmap exit).** `bind` through a
  shared `Monad` bound type-checks, dispatches per constructor
  (`Option`/`Result`), and splices to IR **equivalent to a hand-written inline
  `and_then`** (no call frame beyond it). The `Option` and `Result` impls are
  distinct, proving constructor-keyed dispatch, not a single fallthrough. Traces
  to G2/G3. The exact `bind` body idiom in the impls is **OQ-4**.
- **REQ-7 (regression pin).** The named-constructor-of-quotation path
  (`Box[[i64 -- i64]]`, probe P5) is byte-identical to today — it never touched
  fence #2's variable-headed grammar. Traces to the regression pin.
- **REQ-8 (Applicative.ap — bonus, not exit).** If REQ-1..REQ-5 admit and ground
  `ap`'s shape for free, G5 ships as a bonus golden; if it needs new grounding
  machinery beyond `bind`'s, it is recorded as future work and does **not** block
  the slice (roadmap: "if shape (b) grounds for free in the same extension").
  Traces to G5. **OQ-2.**
- **REQ-9 (guardrails).** No new trait/`impl:` declaration syntax; no new kinds
  or kind polymorphism; no `Applicative.pure` (S6/return-type-polymorphism
  territory). Diagnostics are behaviour: G4's kind error and G1's accept are both
  pinned. Growth signals (CLAUDE.md) re-run on every touched file at phase exit,
  with special attention to `src/parser.rs` and `src/check/poly.rs` (both large,
  both touched).
- **REQ-10 (roadmap + growth + gate).** Roadmap S7 entry / S2-15.d residual
  updated to current-design prose stating `bind` grounds (no history narration);
  growth signals (R6-style) re-run at phase exit on every touched file with the
  outcome recorded; final full gate ×2. Also satisfies the roadmap's own S7 exit
  clause "the probe round's rulings on which effect shapes ground are recorded in
  the brief": `slice7-brief.md`'s "Adjudicated mechanism" section carries the
  final ruling (no new grounding code needed; two already-shipped mechanisms
  verified, not extended) with its full citation trail, superseding the
  originally-probed (and incorrect) mechanism, which is kept in the same section
  for the record rather than deleted.
- **REQ-11 (literal-quotation operand vs. declared `Quotation` — Phase 3
  measurement, not a pre-`/implement` gate).** `unify_member_operand`'s
  fallback arm (`_ => declared == found`, `src/check/poly.rs:1397`) has no
  case pairing a declared `Quotation` against a caller's `PolyType::QuotLit`
  (the marker a literal `[ ... ]` quotation argument carries on the poly
  stack). `bind`'s real dogfood call shape (`4 Some [ half ] bind`) is
  exactly a literal-quotation operand at the caller's dispatch site, so this
  gap must be measured during Phase 3 (the first point a real fixture
  exists) before G2/G3 can be trusted — not probed further now (there is no
  real `Monad.bind` declaration to test it against until REQ-1 lands). If the
  fallback rejects a literal operand, Phase 3 adds the missing arm as part of
  its own scope; this is a measurement step recorded live, not a blocking
  open question (decided; see Open Questions).

## Goldens

New behavioural goldens in `tests/phase7b_slice7.rs`. Complete fixture text lives
in [slice7-paper-tests](./slice7-paper-tests.md); the error golden (G4) pins
byte-exact text once the implementing phase measures the rendered output
(measure-then-pin). Names follow `thing_condition_expected` (CLAUDE.md).

| Golden | Test name | Behaviour |
| --- | --- | --- |
| G1 | `monad_bind_declares_app_in_quotation_row` | before: `app_in_member_quotation_row_error`, exit 1 → after: parses to a `TraitDecl`, `bind` effect intact, `'F` kind `* -> *`; exit 0 |
| G2 | `option_bind_dispatches_and_short_circuits` | `4 Some [ half ] bind` → `Option[i64]::Some(2)`; `3 Some [ half ] bind` → `Option[i64]::None`; IR matches hand-written `and_then` |
| G3 | `result_bind_dispatches_and_short_circuits_on_err` | `Ok 4 [ half_result ] bind` (even) → `Ok 2`; odd input → `Err "odd"`; distinct `impl:` from Option's (constructor-keyed) |
| G4a | `kind_incorrect_app_in_quotation_row_is_error` | regression pin (not the row-fence witness): the pre-existing kind check (`arrow_var_used_bare_error`) fires independently of the row fence for a bare-post-App-head-mention var; located, exit 1 (byte-exact, measure-then-pin) |
| G4b | `member_local_headed_app_in_quotation_row_is_still_unsupported` | the real narrowness witness: a member-local-headed (not trait-var-headed) App inside a row is **still** rejected post-REQ-1; located, exit 1 (byte-exact, measure-then-pin) |
| G5 | `applicative_ap_declares_app_of_quotation_argument` | **bonus** — before: `app_arg_quotation_error`, exit 1 → after (only if grounds for free): parses to a `TraitDecl`; otherwise recorded as future work |
| Reg | `named_ctor_of_quotation_argument_unchanged` | `Box[[i64 -- i64]]` path byte-identical (probe P5) |

## Units (beside the changed code)

- `member_quotation_row_admits_app_expected` (parser) — REQ-1.
- `poly_app_arg_admits_quotation_at_member_site_expected` (parser) — REQ-2/OQ-2.
- `kind_incorrect_app_in_quotation_row_is_located_error` (parser/check) — REQ-4.
- `ground_member_poly_quotation_row_app_dissolves_to_target_generic`
  (`src/ast.rs`),
  `unify_member_operand_app_row_slot_binds_dispatched_ctor`,
  `render_member_decl_app_row_slot_renders_into_caller_space`
  (`src/check/poly.rs`) — REQ-5 (verification only, no new logic).
- Comment-correction units beside each guard touched under REQ-3
  (`ground_member_type`'s doc, `apply_subst`'s doc, `app_in_member_quotation_row_error`'s
  message).
- A `unify_member_operand`-`QuotLit` unit under REQ-11, only if Phase 3's
  measurement finds the gap real.

## Guardrails

- No new trait/`impl:` declaration syntax; no new kinds; no kind polymorphism; no
  `Applicative.pure` (brief "explicitly out of scope").
- Backend stays QBE; IR stays backend-neutral; the linear spine holds (`bind`'s
  quotation consumes its input exactly once — the errors-as-values idiom is the
  point). No in-process JIT / no comptime interpreter (CLAUDE.md invariants; not
  directly exercised here, kept in mind).
- Diagnostics are behaviour: G1's accept and G4's located kind error are both
  pinned; no existing diagnostic text churns except where an audited invariant
  comment (REQ-3) is corrected.

## Open questions

Only one item below is a genuine decision for the user before `/implement`; the
other three were lookups or measurements this pass resolved or correctly
reclassified.

- **OQ-1 (grounding extension) — RESOLVED, no longer an open risk.** The
  original premise (REQ-5 needs new logic in
  `poly_call_abstract_quotation_param`, separating a caller-bound-θ case from
  a still-abstract case) does not hold: that function never sees an
  unresolved App (`ground_member_poly` dissolves it into a `Generic` first),
  and the "still abstract" branch cannot occur (a non-`Generic` impl target
  is already rejected before body-check). See the "Why" section's revised
  finding and REQ-5 for the full citation trail. Kept here only as a resolved
  record, not a live gate.
- **OQ-2 (G5 / Applicative.ap — in-phase or deferred). Genuine user decision
  — the only one left.** Should `ap` be attempted in-phase (REQ-2 + REQ-8) or
  explicitly deferred if it does not ground for free? Default (taken):
  attempt fence #2's lift in Phase 4; ship G5 only if it grounds under the
  same mechanism, else record it as future work and close the slice on
  `bind` alone. The roadmap permits either. **User: confirm whether `ap` is
  worth a Phase-4 attempt or should be deferred outright.**
- **OQ-3 (G4 kind check) — RESOLVED, a lookup, not a decision.** Measured this
  pass: the kind-incorrectness check the original G4 fixture needs already
  exists and fires independently of, and earlier than, the row-shape gate.
  `mark_ty_star` (`src/parser.rs:1950`) raises `arrow_var_used_bare_error`
  (`src/parser.rs:2658`, message at `:2660`) the moment an already
  `Arrow`-established variable (from an earlier application-head mention) is
  later used bare — this runs per-mention while `raw_to_poly_type` builds
  each `PolyType`, strictly before `member_shape_is_supported`'s post-hoc
  row-shape gate (`parse_trait_member_effect`, `src/parser.rs:3950`) ever
  sees the finished signature. No new kind-check logic is needed for the
  *original* G4 fixture (`'F` used bare after an App-head mention); see the
  paper-tests correction for why a *second* fixture is needed as the real
  row-fence-narrowness witness.
- **OQ-4 (`bind` body idiom) — reclassified as a Phase 3 measurement, not a
  pre-`/implement` decision.** The paper-test G2/G3 fixtures leave the exact
  `bind` body idiom unfrozen (`dup Some?` / `Ok?` + destructure + `call`).
  This cannot be answered before Phase 1 lands — there is no real `Monad`
  trait declaration to write a body against yet. Measured during Phase 3
  against the live mechanism (migrate the fixture, don't guess it in
  advance); not a user decision blocking `/implement`.

## Baseline (measured at a9eca84)

- Suite: green, all crates, no failures (brief/probes). Re-measure with
  `--no-fail-fast` at implementation start.
- S7 adds `tests/phase7b_slice7.rs` → integration-file count +1. The implementing
  phase re-measures the exact pass count from the base.
- Green = `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
  Baseline `clippy --all-targets` separately with `git stash` — it is red at HEAD
  independent of this slice.

## Delivery plan

Small, independently-committable phases (sibling-slice granularity). Fence lifts
land before the grounding extension, since the grounding logic is unreachable
until the shape parses.

### Phase 1 — lift fence #1 + retire the two S2-15.d tests + audit row invariants + kind-correctness (difficulty: standard)

**Changes.** Narrow `member_shape_is_supported`'s `Quotation` arm
(`src/parser.rs:391`) by deleting its `&& !member_quotation_row_mentions_app(t)`
conjunct — the arm's existing per-element recursion into `member_shape_is_supported`
already gates an App by `head == 0` (`src/parser.rs:386`), so a row-nested App
headed by anything else stays rejected without extra logic (REQ-1). Retire/invert
`app_inside_member_quotation_row_is_fenced` (`tests/phase7b_slice2.rs:204`) and
`parse_trait_decl_app_inside_member_quotation_row_is_fenced` (`src/parser.rs:11861`),
and correct `docs/roadmap/P7b/slice2-spec.md`'s S2-15.d line to note the
supersession (REQ-1). Correct comments only (no code change) on
`fence_member_app_against_concrete_target` (`src/ast.rs:2116`) and `apply_subst`'s
`head`-indexing invariant (`src/check/poly.rs:10454`/`:10497`); correct the
`app_in_member_quotation_row_error` message for the surviving
member-local-headed-App-in-row case (REQ-3). Confirm (already measured this
pass, per OQ-3) that `arrow_var_used_bare_error` already covers the
bare-post-App-head-mention kind error independently of the row fence (REQ-4).

**Goldens.** G1 (declaration parses to a `TraitDecl`), G4 (two fixtures: the
original bare-`'F`-after-App-head-mention kind-error regression pin, plus a new
member-local-headed-App-in-a-row fixture as the actual row-fence-narrowness
witness).

**Units.** `member_quotation_row_admits_app_expected`,
`kind_incorrect_app_in_quotation_row_is_located_error`, plus one unit per
corrected comment/message under REQ-3.

**Exit.** G1 parses; both G4 fixtures error byte-exact; the two retired tests
are inverted/replaced, not left red; `slice2-spec.md`'s S2-15.d line is
corrected; no invariant comment survives that the lift makes untrue; full gate
green. Growth signals noted on `src/parser.rs` / `src/ast.rs` (formal re-check
deferred to the final phase).

### Phase 2 — verify the already-shipped grounding against the row-nested-App shape (difficulty: standard)

**Changes.** No new grounding code (REQ-5, OQ-1 resolved). Add units proving:
(a) `ground_member_poly`'s `Quotation` arm (`src/ast.rs:2305`) recursing into its
own `App` arm (`src/ast.rs:2316`) correctly dissolves `bind`'s row-nested
`'F['U]` into a `Generic` when grounding a generic `impl:` target; (b)
`unify_member_operand`/`render_member_decl` (`src/check/poly.rs:1335`/`:1407`)
correctly unify/render the same shape when a generic caller dispatches through a
`'T: Monad` bound. If either check surfaces a real gap (contradicting this
phase's premise that no new code is needed), stop and escalate rather than
bolting on a fix outside REQ-5's now-resolved scope.

**Goldens.** None new (the mechanism is exercised end-to-end by Phase 3's
dispatch goldens; this phase is unit-covered only).

**Units.** `ground_member_poly_quotation_row_app_dissolves_to_target_generic`,
`unify_member_operand_app_row_slot_binds_dispatched_ctor`,
`render_member_decl_app_row_slot_renders_into_caller_space`.

**Exit.** All three units pass with **zero changes** to
`poly_call_abstract_quotation_param`, `apply_subst`, or the grounding logic in
`ground_member_poly`/`unify_member_operand`/`render_member_decl` beyond the
comment corrections already made in Phase 1; full gate green. **If any unit
requires a real code change, that contradicts this phase's premise — stop and
escalate rather than silently expanding scope.**

### Phase 3 — dispatch + IR: `bind` over Option/Result (difficulty: standard)

**Changes.** Wire `bind` through a shared `Monad` bound so it dispatches per
constructor and splices to IR equivalent to a hand-written inline `and_then`
(REQ-6). Freeze the `bind` body idiom in the Option/Result impls against the live
mechanism (OQ-4, now a measurement, not a pre-decision). Measure whether
`unify_member_operand`'s fallback arm (`src/check/poly.rs:1397`) correctly pairs a
declared `Quotation` against a caller's literal `PolyType::QuotLit` operand
(REQ-11); if not, add the missing arm as part of this phase's own scope. Keep the
regression pin green (REQ-7).

**Goldens.** G2 (`Option`), G3 (`Result`, distinct impl / early-exit on `Err`),
Reg (`Box[[i64 -- i64]]` unchanged).

**Units.** Beside any new dispatch/splice code touched; a
`unify_member_operand`-`QuotLit` unit if REQ-11's gap turns out real.

**Exit.** G2/G3 produce the expected values and IR (no frame beyond `and_then`);
distinct impls prove constructor-keyed dispatch; Reg byte-identical; REQ-11's
measurement is recorded either way; full gate green.

### Phase 4 — Applicative.ap (bonus, gated by OQ-2) (difficulty: standard)

**Changes.** Lift fence #2 (`app_arg_quotation_error`, `src/parser.rs:2630` and
`:7846`) for the trait-member-declaration path — landing in the shared
`RawTy::App` parser path is acceptable (REQ-2). Attempt to ground `ap` under the
same already-shipped mechanism Phase 2 verified for `bind` (REQ-8) — no new
grounding code is expected here either, per REQ-5's resolution, but `ap`'s
mirror shape (App-of-quotation, not quotation-containing-App) is untested and
must be measured, not assumed.

**Goldens.** G5 — **only if** `ap` grounds for free; otherwise this phase records
`ap` as future work and ships no golden.

**Units.** `poly_app_arg_admits_quotation_at_member_site_expected`.

**Exit.** Either G5 parses/grounds/dispatches and is pinned, or `ap` is recorded
as future work with the measured reason; the regression pin (Reg) and all prior
goldens stay green; full gate green. **This phase is optional per OQ-2 and does
not block slice exit.**

### Phase 5 — roadmap + growth re-check + final gate (difficulty: standard)

**Changes.** Update the roadmap S7 entry / S2-15.d residual in current-design
prose stating `bind` grounds (no history narration, per feedback). No code change.

**Goldens.** None (documentation phase).

**Units.** None.

**Exit.** Roadmap no longer describes `bind` as fenced; growth signals (R6-style)
re-run on every file S7 touched (`src/parser.rs`, `src/ast.rs`,
`src/check/poly.rs`, `tests/phase7b_slice7.rs`, roadmap docs) with the outcome
recorded — split only if 2+ signals fire together; final full gate ×2 green.

**Notes.** ROADMAP/DESIGN carry current design only, never history narration.

## Phases (JSON)

```json
{
  "phases": [
    {
      "id": 1,
      "name": "lift fence #1 + retire S2-15.d tests + audit row invariants + kind-correctness",
      "difficulty": "standard",
      "requirements": ["REQ-1", "REQ-3", "REQ-4"],
      "changes": [
        "Delete the '&& !member_quotation_row_mentions_app(t)' conjunct from member_shape_is_supported's Quotation arm (src/parser.rs:391); its existing per-element recursion already gates an App by head == 0 (src/parser.rs:386), so the lift is narrow with no extra logic",
        "Retire/invert app_inside_member_quotation_row_is_fenced (tests/phase7b_slice2.rs:204) and parse_trait_decl_app_inside_member_quotation_row_is_fenced (src/parser.rs:11861); correct slice2-spec.md's S2-15.d line to note the supersession",
        "Comment-only corrections (no code change): fence_member_app_against_concrete_target's stale doc (src/ast.rs:2113-2114, not ground_member_type's), and apply_subst's head-indexing invariant (src/check/poly.rs:10454/:10497)",
        "Real code change: rewrite app_in_member_quotation_row_error's message (src/parser.rs:452-455) so it no longer claims 'App may not appear inside a quotation row' -- App is now admitted for the trait-var-headed case; the surviving rejection is member-local-headed only. This is golden-visible (G4b pins the corrected text), not comment-only",
        "Confirm (already measured, OQ-3) that arrow_var_used_bare_error (src/parser.rs:2658) already covers G4a's kind error independently of the row fence; G4a is a regression pin, not the narrowness witness. G4b (member-local-headed App in a row) is the real row-fence-narrowness witness and must pin the corrected message from the bullet above"
      ],
      "goldens": [
        "monad_bind_declares_app_in_quotation_row",
        "kind_incorrect_app_in_quotation_row_is_error",
        "member_local_headed_app_in_quotation_row_is_still_unsupported"
      ],
      "units": [
        "member_quotation_row_admits_app_expected",
        "kind_incorrect_app_in_quotation_row_is_located_error"
      ],
      "exit": "G1 parses to a TraitDecl with bind's effect intact; both G4 fixtures error byte-exact (measure-then-pin); the two retired tests are inverted/replaced, not left red; slice2-spec.md's S2-15.d line is corrected; no invariant comment survives that the lift makes untrue; full gate green; growth signals noted on parser.rs/ast.rs."
    },
    {
      "id": 2,
      "name": "verify the already-shipped grounding against the row-nested-App shape",
      "difficulty": "standard",
      "requirements": ["REQ-5"],
      "changes": [
        "No new grounding code (REQ-5/OQ-1 resolved: poly_call_abstract_quotation_param never sees an unresolved App; ground_member_poly already dissolves it into a Generic first, and the still-abstract case cannot occur)",
        "Add units proving ground_member_poly's Quotation arm (src/ast.rs:2305) recursing into its own App arm (src/ast.rs:2316) correctly dissolves bind's row-nested App into a Generic when grounding a generic impl: target",
        "Add units proving unify_member_operand/render_member_decl (src/check/poly.rs:1335/:1407) correctly unify/render the same shape when a generic caller dispatches through a bound; if either surfaces a real gap, stop and escalate rather than expanding scope"
      ],
      "goldens": [],
      "units": [
        "ground_member_poly_quotation_row_app_dissolves_to_target_generic",
        "unify_member_operand_app_row_slot_binds_dispatched_ctor",
        "render_member_decl_app_row_slot_renders_into_caller_space"
      ],
      "exit": "All three units pass with zero changes to poly_call_abstract_quotation_param, apply_subst, or the grounding logic beyond Phase 1's comment corrections; full gate green. If any unit requires a real code change, stop and escalate."
    },
    {
      "id": 3,
      "name": "dispatch + IR: bind over Option/Result",
      "difficulty": "standard",
      "requirements": ["REQ-6", "REQ-7", "REQ-11"],
      "changes": [
        "Wire bind through a shared Monad bound so it dispatches per constructor (Option/Result) and splices to IR equivalent to a hand-written inline and_then (no extra call frame)",
        "Freeze the bind body idiom in the Option/Result impls against the live mechanism (OQ-4, a measurement here, not a pre-decision); keep the named-constructor-of-quotation regression pin green",
        "Measure whether unify_member_operand's fallback arm (src/check/poly.rs:1397) correctly pairs a declared Quotation against a caller's literal PolyType::QuotLit operand (REQ-11); add the missing arm here if not"
      ],
      "goldens": [
        "option_bind_dispatches_and_short_circuits",
        "result_bind_dispatches_and_short_circuits_on_err",
        "named_ctor_of_quotation_argument_unchanged"
      ],
      "units": [],
      "exit": "G2/G3 produce the expected values and IR (no frame beyond and_then); distinct impls prove constructor-keyed dispatch; Reg byte-identical; REQ-11's measurement is recorded either way; full gate green."
    },
    {
      "id": 4,
      "name": "Applicative.ap (bonus, gated by OQ-2)",
      "difficulty": "standard",
      "requirements": ["REQ-2", "REQ-8"],
      "changes": [
        "Lift fence #2 (app_arg_quotation_error, raised at src/parser.rs:5213 in parse_poly_app_arg and at src/parser.rs:7846; the error constructor itself is defined at src/parser.rs:2630) for the trait-member-declaration path; landing in the shared RawTy::App parser path is acceptable",
        "Attempt to ground ap under the same already-shipped mechanism Phase 2 verified for bind; ship G5 only if it grounds for free, else record ap as future work with the measured reason"
      ],
      "goldens": [],
      "units": [],
      "exit": "Optional per OQ-2; does not block slice exit. If attempted: either G5 parses/grounds/dispatches and applicative_ap_declares_app_of_quotation_argument plus poly_app_arg_admits_quotation_at_member_site_expected are added and pinned, or ap is recorded as future work with the measured reason and the fence-#2 lift is reverted (no ungroundable shape ships silently parseable); Reg and all prior goldens stay green; full gate green."
    },
    {
      "id": 5,
      "name": "roadmap correction + growth re-check + final gate",
      "difficulty": "standard",
      "requirements": ["REQ-9", "REQ-10"],
      "changes": [
        "Update the roadmap S7 entry / S2-15.d residual (docs/roadmap/P7b-higher-kinded-types.md, docs/roadmap/P7b/slice2-spec.md) to current-design prose stating bind grounds (no history narration)",
        "Re-run growth signals on every file S7 touched (parser.rs, ast.rs, check/poly.rs, tests/phase7b_slice7.rs, roadmap docs) and record the outcome; split only if 2+ signals fire together"
      ],
      "goldens": [],
      "units": [],
      "exit": "Roadmap no longer describes bind as fenced; growth signals re-run with outcome recorded; final full gate x2 green."
    }
  ]
}
```

## Requirement → phase map

| REQ | Phase | Goldens / units |
| --- | --- | --- |
| REQ-1 lift fence #1 + test-flip | 1 | G1; member_quotation_row_admits_app_expected |
| REQ-3 audit invariants | 1 | G1/G4; guard audit units |
| REQ-4 kind-correctness | 1 | G4 (both fixtures); kind_incorrect_..._located_error |
| REQ-5 verify grounding (no new code) | 2 | ground_member_poly/unify_member_operand/render_member_decl units |
| REQ-6 dispatch + IR | 3 | G2, G3 |
| REQ-7 regression pin | 3 | Reg |
| REQ-11 QuotLit measurement | 3 | (measurement; unit only if gap is real) |
| REQ-2 lift fence #2 | 4 | G5; poly_app_arg_admits_quotation_at_member_site |
| REQ-8 Applicative.ap bonus | 4 | G5 |
| REQ-9 guardrails | 1-5 | (across all goldens) |
| REQ-10 roadmap + growth + gate | 5 | (docs; final gate x2) |
