# P7b.S8c — Per-site binding for member signatures with free input type variables

Delivery plan for S8c. Recon inputs, frozen: [slice8c-brief](./slice8c-brief.md)
(scope + F1-F5 mechanism + D1/D2), [slice8c-probes](./slice8c-probes.md)
(two-round verbatim log, route taxonomy, w3r no-churn verdict),
[slice8c-paper-tests](./slice8c-paper-tests.md) (golden designs G1-G6, measured
HEAD behaviour). Roadmap scope: `P7b-higher-kinded-types.md:540`. Recon-docs
base HEAD `62a928d`; the S8b merge itself is `8985d6c` (`62a928d` is the commit
that rebased the recon docs on top of it). Baseline gate green (89 binaries,
3334 tests, 0 failed).

## Why

The P7b.S8 integrated review surfaced a pre-existing ICE: a bound-dispatched
trait member whose signature carries a **free input type variable** (not the
trait header variable, not bound by any bound bracket) passes checking and
panics at IR lowering. The panic is `subst_polytype`'s unification `expect`
(`src/ir/driver.rs:579`, "checked: unification bound every input type
variable"), reached through `concrete_effect` (`:555`) from lowering's R9
instantiation loop (`:327`). `sooth build` exits 101 and produces **no binary**
(the roadmap's "the build succeeds and the PANIC fires at run/IR time" wording
is corrected at slice exit — the *check* succeeds; the panic is a **build-time
lowering** panic).

The recon round rewrote the mechanism story into a five-route dispatch taxonomy
keyed in `resolve_user_bound` (`src/check/poly.rs:8981`) on the obligation's
`ty` (probes, "Mechanism"):

- **Route A — CtorImage** (App-headed operand, `'F['U]`): the arm at
  `poly.rs:9161-9314` composes per site — re-grounds `ob.slots` through the
  caller θ, unifies each candidate's member signature, mints that site's
  θ_call. Every member variable grounds. This is what every shipped S6/S7/S8
  dogfood rides (probes P3/P6/P7/P8).
- **Route B — non-CtorImage, generic winner** (the bound variable itself in a
  non-App position — plain slot, `&'T`, quotation row, array element): the
  P7.S4 mint arm (`poly.rs:9315-9323`) records `(member_word, subst)` where
  `subst` is `find_bound_impl`'s impl-target match substitution **alone** — the
  member row's own variables never enter θ. Lowering substitutes the whole
  member signature through that θ and hits the unbound `expect`. **This is the
  ICE** (probes P1/P5/P9/P10, plus round-2 array-element and bare-var-target
  cells).
- **Route C — non-CtorImage, concrete winner** (`poly.rs:9324-9331`): bare
  symbol, no mint, **no site-slot check**. `ground_member_type`'s
  `PolyType::Var(_) => target` arm (`src/ast.rs:2241`; round 1 cited 2239)
  collapses every member local to the target at registration, so the member
  emits as one mono function typed wholly at the target, and any type flows
  through the local slot unchecked. **A silent wrong-typing hole**, not a
  panic (probes P11/P12).
- **Route D — mono direct member call**: full call-site unification binds
  everything; healthy, and it *does* check operand slots
  (`trait_member_operand_error`; measured by round-2 worker w2, `slice8c-probes.md`'s
  "w2" verdict — the earlier `mono.sth`/P2 fixture is a clean build with no
  diagnostic to observe, so it is not the citation for the operand check).
- **Route E — lifted mono ctor-app target**: bare symbol, no signature to
  substitute; untouched (S8's shipped Range golden).

F3/P14: output-position member locals are already fenced at the impl body check;
only input/ref/quotation/array positions are in play. F5: the IR `expect`'s
invariant holds for call-site-minted instantiations (Routes A/D) and is violated
**by construction** for Route B's match-only mint — the checker never enforced
that a member row's variables are all determined by the impl-target pattern.

## Decisions

### D1 — fix direction: **option B, the per-site binding rule** (REQ-1)

Extend the non-CtorImage generic-winner mint arm to compose per site: at the
member call site, re-ground `ob.slots` through `caller_subst`, unify the member
word's signature, and **extend** (never replace) the impl-target match subst
with the resulting bindings. The repro then *works* — it grounds like `'B`
(Bifunctor's row locals) already does through Route A — rather than being
rejected.

Evidence for B over A and C:

- **Option A (declaration-time plain-slot fence) breaks green tests.** Round 2
  refuted the round-1 "zero victims" claim (probes w4): the committed suite
  carries ≥7 green test functions in 5 files riding plain-slot member locals
  through *working* Route B dispatches — including **S6's exit-criterion
  golden** `Foldable::fold` (`tests/phase7b_slice6.rs:337/372/469`), `Take::take`
  (`tests/phase7b_slice3.rs:426`), `Functor::pick`
  (`tests/phase7b_slice2.rs:146`), plus two src unit twins
  (`declarations.rs:3860`, `poly.rs:22301`). A declaration-time fence outlaws a
  shape the machinery supports. Disqualified.
- **Option B has measured nil symbol churn** (probes w3r, three reasons): (1)
  the shipped mint-arm dispatches — slice8b's `Monoid` `combine`/`empty`
  through bounds (`tests/phase7b_slice8b.rs:632`,
  `monoid_for_list_combine_through_bound_grounds_and_is_stable`, dispatching
  `combine ( 'T 'T -- 'T )` through `merge['T: Monoid]` against `impl: Monoid
  for List`, symbol `sooth_mono_combine_Monoid_0_List__T0___m0__t0_i64`) — are
  **trait-var-only members**: no member-local variable beyond the trait header
  var, so per-site unification contributes no bindings and `theta == subst`
  byte-identically. (The 49-symbol inventory *does* contain this mint-arm
  dispatch; the corrected claim is not "no mint-arm dispatch" but "the
  mint-arm dispatch present is trait-var-only, hence non-churning by
  construction" — see Phase 1's explicit no-churn pin.) (2) the len-pair
  encounter-order hazard has zero victims (no `Len::Var` in the shipped
  dispatch surface); (3) where both substitutions exist the binding sets
  agree, so a per-site symbol is byte-identical to today's. **Predicted**
  post-fix repro monomorph (measured ingredients, not yet built):
  `sooth_mono_odd_Odd_0_Box__T0___m0__t0_i64_t1_i64` (member word registered
  `odd;Odd;0;Box['T0]__m0`; θ binds the target var and the local, both `i64`)
  — Phase 1 pins this as a unit assertion on the recorded `(word, theta)`
  pair; it is measured against the live binary once built, not taken as
  given.
- **Option C (mint-site fence) is subsumed by absorption, not by D2.** C's
  safety property was "a located error when a member variable remains unbound
  after the match subst, in the mint arm itself." REQ-1's fail-closed tail (see
  below) *is* that same check, now running in the mint arm after per-site
  unification rather than instead of it — B does not need C as a separate
  option because REQ-1 carries C's guarantee inline. (D2's concrete-arm check,
  by contrast, is a different arm entirely and does not backstop the mint arm;
  the two are independent closures over independent holes.)

The evidence leans clearly to B; the spec owns the call and picks **B**.

### D2 — concrete-target hole: **fold in** (REQ-2)

Route C's silent no-site-check dispatch (`poly.rs:9324-9331`) is the same
obligation loop, one arm below the mint arm. Round 2 raised its urgency: it is
**bound-dispatch-specific** (the direct mono call is checked, the bound dispatch
is not) and **exploit-confirmed** to produce unambiguous wrong output — a member
body computing on the local slot prints the operand's **raw slot word verbatim**
(measured: a `List[i64]`'s head `41` prints `41`; a `Bool`'s `True` prints `1` —
not, as an earlier draft of this record claimed, `+1`; probes w2, P12). No
type-respecting execution explains it.

Folding in costs one slot-compatibility check in the concrete-winner arm — the
member word here is registered `poly: None` with an already-grounded
`StackEffect` (no `PolySig`, so this is plain-type equality after re-grounding,
not `unify_poly_input`) — and keeps the size-S character: one obligation loop,
one diagnostic family (Route D's `trait_member_operand_error` text is the
template). It closes the hole while B is open in the arm directly above; carving
it out would leave a live silent-wrong-typing hole behind a slice that already
has the re-grounding machinery in hand. **Fold in.**

## Requirements

- **REQ-1 (D1, Phase 1).** In `resolve_user_bound`'s non-CtorImage generic-winner
  mint arm (`src/check/poly.rs:9315-9323`), compose the member instantiation
  **per site**:
  - Start `theta` as a **clone** of `subst` (`find_bound_impl`'s impl-target
    match substitution) — never a fresh/empty `Subst`. Re-ground `ob.slots`
    through `caller_subst` (`apply_subst`, the same call the CtorImage arm makes
    at `poly.rs:9169-9173`), then unify each grounded slot against the member
    word's `PolySig` input via `unify_poly_input` (`poly.rs:10431`), passing
    `theta` as the substitution to unify **into**, with **`seeded: &[]` and
    `seeded_len: &[]`** — empty seed lists. `seeded`/`seeded_len` are wording
    selectors only, not a channel for prior bindings: their non-forwarding
    use is `match seeded.contains(v)` (`poly.rs:10458-10463`; the same
    shape recurs at the CtorImage-head fork `poly.rs:10797` and the length
    forks `poly.rs:10494`/`10701`) and the analogous
    `seeded_len.contains(ln)` wording fork
    (`explicit_len_instantiation_conflict_error`, `poly.rs:12200-12216`),
    choosing `explicit_instantiation_conflict_error`/its length twin over
    `poly_var_conflict_error`/`poly_len_conflict_error`. The impl-target
    bindings ride in `theta` itself (cloned from `subst`), so
    `unify_poly_input` still detects a conflict against an already-bound
    variable regardless of what the seed lists contain — passing them empty
    only selects which wording renders the conflict, which is what the
    provenance ruling below requires. `unify_poly_input` already implements
    "a bound variable in conflict is a named, located error" (the mechanism
    the P7.S3t redirect added — see its own unit test
    `unify_poly_input_finding_a_seeded_variable_names_the_instantiation`,
    `poly.rs:16567`): reuse that conflict *detection* path rather than writing
    a new one. A conflict propagates as a located error naming the member and
    the call site regardless (the CtorImage arm's own unification failure has
    no fallback candidate to try next in this arm — see the failure-branch
    note below).
  - **Conflict-diagnostic provenance ruling.** With `seeded`/`seeded_len`
    empty above, a conflict against a theta-bound variable already routes
    through the unseeded wording (`poly_var_conflict_error`) by construction —
    an empty list never contains the conflicting variable. This is the
    correct wording: on this route the prior binding comes from the
    impl-target **match**, not a written instantiation the user typed, so
    `explicit_instantiation_conflict_error`'s "was instantiated at" phrasing
    (`poly.rs:12169-12172`) would misdescribe provenance — and so would its
    near-fit sibling `impl_target_seed_conflict_error` (`poly.rs:12184-12198`):
    per its own doc comment, both of its ends are the caller's own written
    `[...]` instantiation list (one entry the caller wrote, the other fixed
    by the dispatch type that list's first entry names), whereas here the
    prior binding comes from the impl-target **match** on the bound variable,
    not a written instantiation at all — neither existing sibling fits;
    `poly_var_conflict_error` (or an equivalent message naming the member,
    the variable, and the two competing types with the impl-target match as
    the prior-binding source) is the right call. The identical false-provenance
    reasoning applies to the length twin: `seeded_len` selects
    `explicit_len_instantiation_conflict_error` (`poly.rs:12200-12216`) on the
    same basis, so it is passed `&[]` for the same reason. REQ-6's byte-exact
    conflict-case pin measures the unseeded wording, not the seeded-arm
    default.
  - **Fail-closed tail (absorbs option C).** After unification, scan the member
    word's full variable space (inputs **and** outputs) for any variable `theta`
    does not bind. If one remains, raise a located error naming the member, the
    unbound variable, and the call site — do not proceed to mint. This is what
    keeps `driver.rs:579`'s `expect` from ever firing on this route; REQ-3
    forbids touching that `expect` directly, so this check is the only fence.
  - **Widened guarantee.** The prior draft of this requirement promised binding
    for "every member **input** variable"; that quantifier is wrong.
    `concrete_effect` substitutes `sig.outputs` through the same closure as
    `sig.inputs` (`src/ir/driver.rs:556-560`), and P14's existing fence only
    covers output-position *locals* at the impl-body check, not the header
    variable reaching an output position through the mint arm. The guarantee
    and the fail-closed tail above both cover **every member variable, input
    and output alike**.
  - **Nullary-member witness (why "extend", never "replace").**
    `TraitObligation.slots` holds one entry per declared member *input*, so a
    nullary member (e.g. `trait: Monoid['T] : empty ( -- 'T ) ; ;`, dispatched
    as `mkempty[List[i64]]` through a user word `: mkempty ['T: Monoid] ( --
    'T ) empty ;`) has **zero** slots. Per-site unification over zero slots
    contributes no bindings. If `theta` started empty, the header variable's
    binding — which lives only in `subst`, the impl-target match — would be
    lost, and `driver.rs:579`'s `expect` would fire on the member's **output**.
    Because `theta` starts as `subst.clone()` and is only ever *extended*, the
    nullary case reduces to `theta == subst`, byte-identical to today: the
    symbol `sooth_mono_empty_Monoid_0_List__T0___m0__t0_i64` does not churn.
    Phase 1's regression unit test pins this case explicitly.
  - **QuotLit cell.** A written quotation literal can reach a plain member
    slot: `unify_member_operand`'s `(Var(v), found) => bind` arm
    (`poly.rs:1414`) accepts a `PolyType::QuotLit` operand against a declared
    plain slot (unlike a declared `Quotation` slot, which is deliberately
    rejected — the precedent unit test is
    `unify_member_operand_rejects_a_literal_quotation_operand`, `poly.rs:18067`),
    and the obligation clones that slot verbatim (`PolySlot::quotation`,
    `poly.rs:379`; `TraitObligation.slots`, `poly.rs:2948`). Repro: `: consume
    ['T: Odd] ( &'T -- ) [ drop ] swap odd ;` against `odd ( 'U &'T -- )` —
    passes today's check and panics **today**, measured, at `driver.rs:579`
    (the same unification-bound-every-input-variable `expect` every Route B
    repro hits, since today's match-only mint arm never grounds `ob.slots`
    through `apply_subst` at all). The reason this cell needs its own fence,
    rather than falling out of the residual-unbound-variable tail alone: once
    REQ-1's re-grounding calls `apply_subst` on `ob.slots`, a QuotLit slot
    would drive straight into `apply_subst`'s `QuotLit` arm, which is
    `unreachable!("a quotation-literal marker never reaches a signature")`
    (`poly.rs:10917`) — a **post-fix hazard** REQ-1's own re-grounding step
    would newly expose, not today's panic site. That is the fence's real
    motivation. Before re-grounding any slot, check for `PolyType::QuotLit`
    and reject it **located, at the dispatch site** if found — the marker
    must never reach `apply_subst`. This
    is a new check-stage fence this slice adds (unit test + golden twin, see
    REQ-5/REQ-6). The identical hazard applies to REQ-2's re-grounding step
    (a QuotLit obligation slot there must be rejected the same way, before its
    own `apply_subst` call).
  - **Failure-branch note.** Route A keeps a candidate list because a pinned
    candidate can be disqualified at one site and still serve at another
    (`poly.rs:9139-9165`). Route B has exactly one winner from `find_bound_impl`
    — there is no fallback candidate to try. A unification conflict, a QuotLit
    slot, or a residual-unbound variable are each the terminal, located error
    for that call site; there is no partial-match retry.
  - Route A is untouched by construction.

- **REQ-2 (D2, Phase 2).** In the concrete-winner arm (`poly.rs:9324-9331`), add
  a site-slot compatibility check, applying **only when `imp.target.is_concrete()`
  is true** (a genuinely concrete impl target, e.g. `impl: Odd for i64`). The
  shared arm's other tenant — a lifted mono ctor-app target (`imp.target
  .is_mono_ctor_app() && tr.words[*idx].poly.is_none()`, the same test the
  arm's own `is_generic` guard already makes at `poly.rs:9315-9317`) — is
  **Route E and stays untouched**; S8's shipped `Range[i64]` golden is the
  regression canary proving it.
  - **Mechanism.** A concrete-target member word is registered `poly: None`
    with an already-grounded `StackEffect` (`src/parser.rs:4589-4608`) — there
    is no `PolySig`, so `unify_poly_input` does not apply here. Re-ground
    `ob.slots` through `caller_subst` exactly as REQ-1 does (rejecting a
    `PolyType::QuotLit` slot located, before grounding, per REQ-1's fence),
    yielding a `Vec<Type>`. Compare each grounded slot against the member
    word's registered `effect.inputs[i].ty` by plain `Type` equality, modelled
    on the mono branch's `match_slot` + `PolyType::Concrete` wrapper usage
    (`poly.rs:2560-2598`) and the lifted-mono arm's own equality check
    (`poly.rs:9209-9211`, `if ins != site_slots`).
  - **Diagnostic.** On mismatch, reuse `trait_member_operand_error`'s message
    family (`poly.rs:11505-11527`), rendering the grounded `Type`s wrapped as
    `PolyType::Concrete(..)` exactly as the mono branch already does — this is
    a deliberate reuse of the existing message, not a new diagnostic string.
  - **Semantic ruling.** `ground_member_type`'s `PolyType::Var(_) => target`
    arm (`ast.rs:2241`) collapses every free member local to the target type
    at registration — a free member variable at a concrete target **stays
    pinned to the target type**. This check REJECTS an operand that disagrees
    with that pinned type; it does **not** make Route C polymorphic, and it
    introduces no new inference.
  - **Accept-side exit criterion.** The concrete-arm blast radius this check
    actually gates is the shipped `Ord` (12 impls), `Show` (11), and `Write`
    dispatches (plus any shipped concrete-target members) — all stay green
    under the new check (`cargo test`, zero new failures). This is the
    blast-radius measurement round 2 never took for D2 specifically (it only
    measured option A's blast radius and option B's symbol churn). slice8b's
    `Monoid` is green-suite evidence too, but it exercises Phase 1's mint-arm
    surface (`combine`/`empty` through bounds), not this arm — `Monoid`'s
    dispatches never reach the concrete-winner arm, so it is not part of D2's
    blast radius.
  - The bare-symbol dispatch path is otherwise unchanged (no mint — the member
    is already a mono word).

- **REQ-3 (scope, all phases).** `src/check/` only. **No `src/ir/` change** —
  `driver.rs:579`'s `expect` stays as the backstop, and G2/G4 double as the
  proof it never fires on the already-healthy routes. No new trait/impl surface,
  no `lib/` change (`lib/` is the regression net only).

- **REQ-4 (caller rule unchanged, Phase 3).** P4's caller-side rule ("output
  variable that no input binds / supply it explicitly") is pre-existing and
  stays byte-identical. The quotation-row twin (P5) reaches Route B *after* the
  caller supplies the explicit instantiation, so REQ-1 grounds it; P4's own
  located message gets no new golden.

- **REQ-5 (cells, Phase 3).** REQ-1's binding must cover all Route B cells the
  same union-var mechanism admits (`build_member_var_union`,
  `parser.rs:768/817-827`): plain slot, `&'T`, quotation-row local,
  array-element (`array['U 2]`), and the bare-var catch-all impl target
  (`impl: Odd for 'T`) all **ground** under direction B. The written-quotation-
  literal cell (QuotLit, see REQ-1) is the one cell that does **not** ground —
  it is rejected located at the dispatch site. Goldens cover the cheapest twin
  of each grounding cell plus the QuotLit rejection; the plain-slot cell gets
  its own end-to-end golden (not just a unit pin) because it is the shape S6's
  `Foldable::fold` rides in production.

- **REQ-6 (unit coverage, Phases 1-2).** Unit tests beside the changed checker
  code (`src/check/poly.rs`'s `#[cfg(test)] mod tests`): message bytes, member/
  variable naming, the mint-arm binding (REQ-1, including the nullary-no-churn
  case, the shipped `combine`/`Monoid` no-churn pin, and the QuotLit-rejection
  case), and the concrete-arm compatibility check (REQ-2, including its own
  QuotLit-rejection case). Byte-exact diagnostic pins measured from the live
  binary (diagnostics are behaviour). Phase 1 additionally asserts the
  predicted post-fix monomorph symbol as a unit test over the recorded
  `(word, theta)` pair passed to `instantiation_symbol`, using the
  **asymmetric** case — target var `str`, free local `i64` (the shape G1a
  ships end-to-end; see below for why a symmetric case cannot serve this
  witness) — this is a `src/check` unit assertion, not a built-binary `nm`
  capture; the end-to-end `nm`/`emit_ssa_with_manifest` pin is Phase 3's job
  (the G1a and G4 rows).
  **Grounding-correctness witness.** The recorded `(word, theta)` pair *is*
  the correctness witness, not merely a symbol-string check: if per-site
  unification bound the local to the wrong type, the recorded `theta` and the
  rendered symbol would differ from the predicted value above — but only
  when the target and the local are different types. G1r/G1p and REQ-5's
  array-element/bare-var-target twins all bind the target and the local to
  the *same* type (`i64`), so a per-site θ that wrongly collapsed the local
  onto the impl-target match would render an identical symbol there too;
  those four are **grounding-existence** pins (they prove a binding happens,
  and the flip from panic to grounded), not correctness witnesses. G1a's
  asymmetric twin (target `str`, local `i64`) is the one fixture whose types
  differ, so it is the fixture that actually discriminates slot-sourced from
  target-sourced binding. No runtime golden observes this instead — a
  generic-target member body cannot print a free member local (member rows
  carry no bounds, so there is no `Show`-style obligation to route through;
  round 2 measured both failure modes directly: the printed-word operator is
  unresolvable against a free local, and routing through a helper hits
  "expects 'Bool', but the type variable 'U is not a concrete type") — so the
  theta-content assertion here and the Phase 3 symbol capture (G1a) are the
  grounding-correctness witness end to end; no runtime type-observing twin is
  attempted.

- **REQ-7 (goldens both routes, Phase 3).** End-to-end goldens in
  `tests/phase7b_slice8.rs` for the mono call-site route (Route D) and the
  bound-dispatch route (Route B), per roadmap scope. S8c is an S8 follow-up; the
  file header comment gains an S8c line.

- **REQ-8 (repro twins land grounded, not located, Phase 3).** Retitled from an
  earlier draft's "panicked→located": under direction B the S8-review repro
  twins land **panicked→grounded** (they build and run — G1r and the new
  plain-slot golden), while the residual-unbound-variable case, the QuotLit
  cell, and D2's concrete-arm mismatch land **panicked/silent→located**. The
  fence-direction halves (an earlier design's G1/G3a) do not ship. The phase
  commit message names which direction won and which cells are grounded vs.
  located.

## Observable success criteria

Goldens (`tests/phase7b_slice8.rs`, table below), measure-then-pin:

- Route B's repro twins (`plainslot.sth`, `noref.sth`, quotation-row,
  array-element, bare-var-target) build and run to exit 0 instead of
  panicking at `driver.rs:579` (G1r/G1p/G3b + REQ-5's twins) —
  grounding-existence pins (they prove a binding happens; their symmetric
  i64/i64 types cannot discriminate binding provenance).
- G1a's asymmetric-twin monomorph-symbol capture is the grounding-correctness
  witness (REQ-6): per-site unification bound the local to the *right* type,
  not merely *a* type — something the symmetric pins above cannot show.
- The QuotLit cell is rejected located, non-panic, at the dispatch site (the
  `quotlit.sth` golden).
- D2's concrete-arm hole is rejected located (`concrete4.sth`, G6); the
  shipped `Ord`/`Show`/`Write` concrete-arm surface and slice8b's `Monoid`
  mint-arm surface both stay green (REQ-2/REQ-1 accept-side criteria).
- No shipped dispatch symbol churns (G4, G5, and Phase 1's `combine`/`Monoid`
  pin).
- Gate: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`,
  green.

## Goldens

Designs and measured HEAD behaviour: [slice8c-paper-tests](./slice8c-paper-tests.md).
Fixtures below are pasted **without** their own `import: intrinsics *
;`/`import: hosted::show | . | ;` header lines: `single_file_hosted`
(`tests/phase7b_slice8.rs:57-61`) already prepends both; pasting a fixture's own
copy duplicates the qualifier (measured: `error: duplicate import qualifier
'show'`).

| Golden | Test name | Fixture (probes) | Post-fix (direction B) |
| --- | --- | --- | --- |
| G1r | `bound_dispatch_grounds_member_locals_from_the_call_site` | `plainslot.sth` (P1, the `&'T` cell) | builds+runs, exit 0; monomorph `sooth_mono_odd_Odd_0_Box__T0___m0__t0_i64_t1_i64` captured via `nm`/`emit_ssa_with_manifest` — **predicted (w3r); measured at implementation**. Target var and local are both `i64` (symmetric): this is a **grounding-existence** pin (the flip from panic to grounded), not a correctness witness — the identical symbol would render even if per-site unification wrongly collapsed the local onto the impl-target match. G1a discriminates. |
| G1p | `bound_dispatch_grounds_a_bare_plain_slot_member_local` | `noref.sth` (P9, the bare plain-slot cell — the shape `Foldable::fold` rides) | builds+runs, exit 0 — grounding-existence pin (both `i64`, symmetric, same reasoning as G1r; see G1a) |
| G1a | `bound_dispatch_discriminates_slot_sourced_from_target_sourced_binding` | asymmetric twin (below; `mkbox` takes `str`, the free local operand is `i64`) | builds+runs, exit 0; monomorph `sooth_mono_odd_Odd_0_Box__T0___m0__t0_str_t1_i64` captured via `nm`/`emit_ssa_with_manifest` — **predicted (round 3); measured at implementation**. This is the end-to-end **grounding-correctness** witness (REQ-6): target and local are different types, so a per-site θ that wrongly sourced the local's binding from the impl-target match (instead of the site slot) would render a distinct, wrong symbol, not this one. |
| G2 | `mono_call_site_member_local_stays_working` | `mono.sth` (P2) | build+run exit 0 (positive pin, Route D) |
| G3b | `quotation_row_member_local_grounds_with_explicit_instantiation` | `qrow3.sth` (P5) | builds+runs, exit 0 |
| G4 | `app_headed_member_local_keeps_grounding_per_site` | `mixed3.sth` (P8) | build+run exit 0 **and** symbol `sooth_mono_w2_W2_0_Box__T0___m0__t0_i64_t1_e0_Bool` byte-identical (Route A regression pin) |
| G5 | (checklist item, not a fixture — see Phase 3 scope) | S6/S7/S8 dogfoods (existing suite) | `cargo test` green; guards are the existing suite's own symbol/IL pins — `functor_for_list_map_lowers_as_one_non_inline_frame` (`tests/phase7b_slice8b.rs:531`) and the REQ-11 pin (`tests/phase7b_slice8.rs:742`) — plus the escape hatch: any retouched baseline IL justified in the Phase 3 commit message |
| G6 | `concrete_target_member_dispatch_checks_site_slots` | `concrete4.sth` (P12) | `build_error_located` naming the member and the mismatched slot (D2 folded in) |
| — | `bound_dispatch_rejects_a_quotation_literal_in_a_plain_member_slot` | `quotlit.sth` (new, below) | `build_error_located`, non-panic, names the member and the site |

Plus REQ-5's remaining cell twins (array-element, bare-var-target), each a
G1r-style build+run pin — grounding-existence, symmetric `i64`/`i64` types
like G1r/G1p, not discriminating witnesses — followed by G1a's asymmetric
twin, the discriminating witness:

- **Array-element**: `array['U 2]` (note `array` is not a callable word; the
  working constructor spelling is `0 2 fill` — see `tests/phase4_slice10b.rs:511`
  for the pattern):

  ```sth
  type: Box['T] v 'T ;
  : mkbox ( i64 -- Box[i64] ) Box ;
  trait: Odd['T] : odd ( array['U 2] &'T -- ) ; ;
  impl: Odd for Box['T] : odd drop drop ; ;
  : consume ['T: Odd] ( array['U 2] &'T -- ) odd ;
  : main ( -- ) 7 mkbox | b | 0 2 fill &b consume ;
  ```

- **Bare-var-target**: `impl: Odd for 'T` (the catch-all impl target):

  ```sth
  type: Box['T] v 'T ;
  : mkbox ( i64 -- Box[i64] ) Box ;
  trait: Odd['T] : odd ( 'T 'U -- ) ; ;
  impl: Odd for 'T : odd drop drop ; ;
  : consume ['T: Odd] ( 'T 'U -- ) odd ;
  : main ( -- ) 7 mkbox | b | b 7 consume ;
  ```

- **Asymmetric twin (G1a)**: same `&'T` cell as `plainslot.sth`, but with
  `mkbox` typed at `str` (the target var) while the free local operand stays
  `i64` — the type mismatch is what lets the rendered symbol discriminate a
  correctly-sourced binding from a collapse onto the target's binding:

  ```sth
  type: Box['T] v 'T ;
  : mkbox ( str -- Box[str] ) Box ;
  trait: Odd['T] : odd ( 'U &'T -- ) ; ;
  impl: Odd for Box['T] : odd drop drop ; ;
  : consume ['T: Odd] ( 'U &'T -- ) odd ;
  : main ( -- ) "x" mkbox | b | 7 &b consume ;
  ```

- **QuotLit rejection** (`quotlit.sth`), the reviewer's repro:

  ```sth
  type: Box['T] v 'T ;
  : mkbox ( i64 -- Box[i64] ) Box ;
  trait: Odd['T] : odd ( 'U &'T -- ) ; ;
  impl: Odd for Box['T] : odd drop drop ; ;
  : consume ['T: Odd] ( &'T -- ) [ drop ] swap odd ;
  : main ( -- ) 7 mkbox | b | &b consume ;
  ```

**Type-observing runtime twin: not attempted.** A generic-target member body
cannot print a free member local — member rows carry no bounds, so there is
no `Show`-style obligation to route through a print call. Both failure modes
were measured directly (round 2): the printed-word operator is unresolvable
against a free `'U` local inside a generic-target member body, and routing
through a helper word instead hits "expects 'Bool', but the type variable 'U
is not a concrete type." REQ-6/REQ-7 substitute the theta-content unit
witness (Phase 1) and the G1a monomorph-symbol capture (Phase 3) for the
grounding-correctness proof a runtime golden would have provided.

Harness: `single_file_hosted` / `build_run_keep` / `build_error_located` (the
latter asserts `!stderr.contains("panicked")` — the admission-safety sweep's
standing rule). G4's symbol assert rides `emit_ssa_with_manifest` capture
(`tests/phase7b_slice8.rs:742`; `slice8b.rs:177/565` already filter
`sooth_mono_*`).

## Codebase map

| Path:line | Symbol | Role in S8c |
| --- | --- | --- |
| `src/check/poly.rs:8981` | `resolve_user_bound` | route dispatch on `ob.ty`; the changed function |
| `src/check/poly.rs:9161-9314` | CtorImage arm | per-site composition; REQ-1's source machinery (`unify_poly_input`, slot re-grounding via `apply_subst`) |
| `src/check/poly.rs:9315-9323` | P7.S4 generic-winner mint arm | **REQ-1 edit site** — extend the match-only subst with per-site θ, plus the fail-closed tail |
| `src/check/poly.rs:9324-9331` | concrete-winner / lifted-mono shared arm | **REQ-2 edit site** — add the site-slot compatibility check, gated on `imp.target.is_concrete()` only |
| `src/check/poly.rs:10431` | `unify_poly_input` | REQ-1's unification tool; already supports seeded-conflict diagnostics (`poly.rs:16567`'s unit test is the precedent) |
| `src/check/poly.rs:10917` | `apply_subst`'s `QuotLit` arm | `unreachable!` — the panic REQ-1/REQ-2's QuotLit fence must prevent from ever being reached |
| `src/check/poly.rs:1414`, `379`, `2948` | `unify_member_operand`'s `(Var(v), found)` bind arm; `PolySlot::quotation`; `TraitObligation.slots` | how a `QuotLit` operand reaches an obligation slot unfiltered |
| `src/check/poly.rs:18067` | `unify_member_operand_rejects_a_literal_quotation_operand` | precedent unit test for rejecting a `QuotLit` against a *declared-Quotation* slot; REQ-1/REQ-2 add the plain-slot counterpart |
| `src/check/poly.rs:2560-2598` | mono branch `match_slot` + `PolyType::Concrete` usage | REQ-2's template for the equality check and diagnostic reuse |
| `src/check/poly.rs:9209-9211` | lifted-mono (Route E) type-equality check | REQ-2's second template — the same shared arm's *other* tenant already does an equality check this way |
| `src/check/poly.rs:11505-11527` | `trait_member_operand_error` | REQ-2's reused diagnostic family |
| `src/check/poly.rs` (`mod tests`, near `:22301`) | checker unit tests | REQ-6 unit twins live here |
| `src/ir/driver.rs:579` | `subst_polytype`'s `expect` | the backstop; **unchanged** (REQ-3) |
| `src/ir/driver.rs:555-560,327` | `concrete_effect` (inputs **and** outputs), `lower` R9 loop | where the panic is reached today; unchanged |
| `src/ast.rs:2241` | `ground_member_type`'s `Var` arm | why Route C collapses locals to the target (context for REQ-2); unchanged |
| `src/parser.rs:391` | `member_shape_is_supported`'s `Var` arm | admits plain-slot member locals (context); unchanged |
| `src/parser.rs:768,817-827` | `build_member_var_union` | the union-var mechanism spanning all REQ-5 cells; unchanged |
| `src/parser.rs:4589-4608` | concrete-target member registration | why REQ-2 has no `PolySig` to unify against (context); unchanged |
| `tests/phase7b_slice8.rs` (header; `:742`) | S8 goldens | REQ-7/REQ-8 end-to-end goldens + G4 symbol capture |

## Delivery Plan

### Phase 1: Per-site binding in the mint arm (D1/REQ-1)

- **Goal**: `plainslot.sth` (the S8-review repro) and `noref.sth` (the bare
  plain-slot cell `Foldable::fold` rides) build and run to exit 0 instead of
  panicking at `driver.rs:579`, with no churn to any shipped dispatch symbol.
- **Requirements Covered**: REQ-1, REQ-6 (mint-arm unit tests)
- **Scope**:
  - Modify `src/check/poly.rs:9315-9323` (the P7.S4 generic-winner mint arm
    inside `resolve_user_bound`): replace the bare `subst`-only mint with
    per-site composition — re-ground `ob.slots` via `apply_subst` (rejecting a
    `PolyType::QuotLit` slot located first), unify via `unify_poly_input`
    against `theta` (cloned from `subst`) with empty seed lists (`seeded:
    &[]`, `seeded_len: &[]` — the seed lists select conflict wording only,
    per REQ-1's provenance ruling), then the fail-closed residual-unbound-
    variable check, before minting.
  - Add unit tests beside the change (`src/check/poly.rs`'s `mod tests`):
    binding correctness for the plain-slot and `&'T` shapes (both binding arms,
    `Box['T0]` vs `Box[i64]`), the nullary-member no-churn case
    (`empty`/`Monoid`), the shipped `combine`/`Monoid` no-churn pin
    (`sooth_mono_combine_Monoid_0_List__T0___m0__t0_i64`, the trait-var-only
    shape `tests/phase7b_slice8b.rs:632` rides through this phase's own edit
    site), the QuotLit-rejection case, a unification-conflict case (asserting
    the `poly_var_conflict_error`-style wording, per the provenance ruling
    above), and a residual-unbound-variable case.
  - Add the unit assertion for the predicted post-fix monomorph symbol over the
    recorded `(word, theta)` pair, using the asymmetric case (target `str`,
    local `i64` — REQ-6's discriminating witness, not a symmetric case).
  - Explicitly out of scope: the concrete-winner arm (`poly.rs:9324-9331`,
    Phase 2's edit site) and anything in `src/ir/`.
- **Entry Conditions**: spec approved at HEAD `dddb938` or later; baseline gate
  green.
- **Exit Criteria / Verifiable Artifacts**: new unit tests pass under `cargo
  test`; the symbol-string unit assertion matches the predicted
  `sooth_mono_odd_Odd_0_Box__T0___m0__t0_str_t1_i64` (the asymmetric case);
  the nullary-member unit
  test proves `theta == subst` (no churn) for a zero-slot obligation; the
  shipped `combine`/`Monoid` pin proves `theta == subst` byte-identically for
  the one committed test that dispatches through this phase's own mint arm
  (`tests/phase7b_slice8b.rs:632`); existing suite stays green (no regression
  to Route A/D/C paths otherwise untouched by this phase).
- **Parallelism**: SEQUENTIAL (first phase; nothing precedes it).
- **Relative Effort**: M — two edits (re-grounding, unification with empty
  seed lists) plus a
  new fail-closed check and a QuotLit fence, all inside one shared function's
  control flow, plus five-plus new unit tests.
- **Difficulty**: hard — a cross-cutting edit to the shared obligation loop at
  an ambiguous integration point (the mint arm's failure semantics were
  previously unspecified; this phase adjudicates them).
- **Open Questions / Blockers**: None identified.

### Phase 2: Concrete-arm compatibility check (D2/REQ-2)

- **Goal**: `concrete4.sth` (a `List[i64]` flowed through an `i64`-typed member
  local via a concrete-target bound dispatch) is rejected with a located error
  instead of silently printing wrong output; the shipped `Ord`/`Show`/`Write`
  concrete-arm dispatch surface is unaffected (`Monoid` rides Phase 1's
  mint-arm surface, not this arm — see REQ-2's accept-side criterion).
- **Requirements Covered**: REQ-2, REQ-6 (concrete-arm unit tests)
- **Scope**:
  - Modify `src/check/poly.rs:9324-9331` (the shared concrete-winner/lifted-mono
    arm): add the site-slot compatibility check, gated on `imp.target
    .is_concrete()` so Route E's lifted-mono tenant is untouched. Re-ground
    `ob.slots` via `apply_subst` (rejecting a `PolyType::QuotLit` slot located
    first, mirroring Phase 1's fence), compare against the member word's
    registered `effect.inputs`, and raise `trait_member_operand_error` (reused,
    rendered over `PolyType::Concrete`-wrapped grounded types) on mismatch.
  - Add unit tests beside the change: the mismatch case (a `List[i64]` in an
    `i64`-typed local slot) is now located; the matching case still builds; the
    QuotLit-rejection case for this arm.
  - Explicitly out of scope: the mint arm (Phase 1, already landed), Route E's
    lifted-mono equality check (`poly.rs:9209-9211`, untouched), `src/ir/`.
- **Entry Conditions**: Phase 1 merged — this phase reuses the QuotLit-rejection
  helper Phase 1 introduces rather than writing a second copy.
- **Exit Criteria / Verifiable Artifacts**: new unit tests pass; **accept-side
  criterion** — the full existing suite's `Ord` (12 impls), `Show` (11), and
  `Write` dispatch tests (this phase's actual concrete-arm surface) stay green
  (zero new failures); `cargo test` overall green.
- **Parallelism**: SEQUENTIAL after Phase 1 (same function, and reuses Phase 1's
  QuotLit-rejection helper).
- **Relative Effort**: S — one arm, one check, reusing Phase 1's re-grounding and
  fence machinery.
- **Difficulty**: hard — a shared arm serving two routes (C and E); getting the
  `is_concrete()` discriminator wrong silently regresses S8's shipped
  `Range[i64]` golden.
- **Open Questions / Blockers**: None identified.

### Phase 3: End-to-end goldens (REQ-4/5/7/8)

- **Goal**: the full golden set in the table above builds and passes against
  the live binary, proving both fixes end-to-end and pinning the exact bytes
  (diagnostics, symbols) the unit tests only predicted.
- **Requirements Covered**: REQ-4 (verified, no new code), REQ-5, REQ-7, REQ-8
- **Scope**:
  - Add to `tests/phase7b_slice8.rs` (header comment gains an S8c line, after
    `:742`'s REQ-11 pin): G1r, G1a (with its `nm`/`emit_ssa_with_manifest`
    symbol capture — the grounding-correctness witness, REQ-6), G1p, G2, G3b,
    G4, G6, the QuotLit-rejection golden, and the array-element/bare-var-target
    twins — all fixtures as designed in the Goldens section above, pasted
    without their own header import lines.
  - G5 is a checklist item, not a new fixture: run `cargo test`, confirm zero
    retouched baseline ILs; justify any retouch in the phase's commit message.
  - Explicitly out of scope: any change to `src/check/` or `src/ir/` beyond
    what Phases 1-2 already made (this phase is goldens-only).
- **Entry Conditions**: Phases 1 and 2 merged (both fixes must be present for
  the positive goldens to pass and the negative ones to be located rather than
  panicked).
- **Exit Criteria / Verifiable Artifacts**: `cargo test --test phase7b_slice8`
  green with every new golden present; G4's symbol assertion byte-identical to
  today's measured value (regression proof); the byte-exact diagnostic/symbol
  pins in the new goldens are measured from the live binary, not copied from
  this spec's predictions.
- **Parallelism**: SEQUENTIAL after Phases 1-2.
- **Relative Effort**: M — nine-plus new fixtures and goldens, several needing
  hand-verification against the live binary.
- **Difficulty**: standard — mechanical golden-writing against an already-fixed
  checker, following the existing harness conventions.
- **Open Questions / Blockers**: None identified.

### Phase 4: Roadmap corrigenda and gate

- **Goal**: the roadmap's S8c paragraph accurately reflects what shipped
  (direction B, D2 folded in, corrected wording), and the full green gate
  passes.
- **Requirements Covered**: REQ-3 (verified — confirm no `src/ir/` diff)
- **Scope**:
  - Apply corrigenda to the S8c paragraph (`P7b-higher-kinded-types.md:540`):
    1. "the build succeeds and the PANIC fires at run/IR time" → the check
       succeeds; the panic fires during **lowering, at build time** (no
       binary).
    2. The shape is one cell of a five-route taxonomy, with the
       quotation-row/array-element/bare-var-target/QuotLit cells and the
       concrete hole recorded.
    3. Carry-forward wording (note S8c shipped direction B and folded in D2).
    4. Citation drift (line numbers as they now stand).
    5. The entry's title ("Located fence for member signatures with
       unbindable free type variables") and its exit criterion ("the S8-review
       repro twins panicked→located") are both amended — under direction B the
       repro twins panicked→**grounded**; "located" survives only for the
       residual-unbound-variable tail, the QuotLit cell, and D2's concrete-arm
       mismatch.
    6. **Size correction**: the roadmap's original coarse estimate was `S`;
       the measured per-phase efforts (M/S/M/S) are the corrected figure. One
       sentence in the roadmap paragraph notes this as the slice's own size
       corrigendum, per the project's "no history" convention for
       ROADMAP/DESIGN (state the corrected figure, not a before/after
       narrative).
  - Re-run the growth-structure signals against `poly.rs` (per CLAUDE.md); the
    split stays deferred unless a phase's diff actually trips two or more
    signals.
  - Run the full green gate: `cargo fmt --check && cargo clippy -- -D warnings
    && cargo test`.
- **Entry Conditions**: Phase 3 merged (goldens exist and are measured, so the
  corrigenda can cite exact, verified outcomes rather than predictions).
- **Exit Criteria / Verifiable Artifacts**: roadmap paragraph updated with all
  six corrigenda; full green gate exits 0.
- **Parallelism**: SEQUENTIAL after Phase 3.
- **Relative Effort**: S — documentation and a gate run, no new code.
- **Difficulty**: standard.
- **Open Questions / Blockers**: None identified.

### Parallelism Summary

Fully sequential: Phase 1 → Phase 2 → Phase 3 → Phase 4. Phases 1 and 2 both
edit the same shared function (`resolve_user_bound`) and Phase 2 reuses Phase
1's QuotLit-rejection helper; Phase 3's goldens need both checker fixes
present to exercise the grounded/located outcomes; Phase 4's corrigenda cite
Phase 3's measured results. No phase can start before its predecessor lands.

### Effort Summary

M (Phase 1) + S (Phase 2) + M (Phase 3) + S (Phase 4). The roadmap's original
coarse `Size: S` is corrected to this measured M/S/M/S profile at slice exit
(Phase 4's corrigendum 6's sentence).

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
| --- | --- | --- |
| Theta-order symbol churn: extending θ with per-site bindings reorders construction relative to today's bare `subst` mint, which could in principle mangle a different symbol for a shipped dispatch. | Low (leg (3): where both substitutions exist the binding sets agree, so a per-site symbol is byte-identical to today's; Phase 1's explicit `combine`/`Monoid` no-churn pin measures this against the one shipped mint-arm dispatch the suite carries — round 2 confirmed a literal replace-subst implementation would have gone red against that committed test, so the pin is load-bearing, not decorative) | The P7.S3t sort invariant (`theta.ty.sort_by_key`/`theta.len.sort_by_key`) is preserved by construction (the extension is sorted, same as today); G5 is the escape hatch — any retouched baseline IL must be justified in the Phase 3 commit message, the way `d2e8123` did for S9. |
| Phase 2's new rejection has an unmeasured blast radius on the shipped concrete-target dispatch surface (`Ord`, `Show`, `Write` — `Monoid` rides Phase 1's mint-arm surface, not this one). | Low (these impls' member locals are already correctly typed at every call site in the shipped suite) | Phase 2's accept-side exit criterion requires the full existing suite for these traits to stay green; if it does not, the check's ruling ("pin to target type, reject mismatch") needs revisiting before merge, not after. |
| A residual-unbound-variable shape slips past Phase 1's fail-closed tail and still reaches `driver.rs:579` as a panic. | Low, but the consequence is severe (a raw panic reappears) | REQ-3 forbids touching `driver.rs:579` directly, so the fail-closed tail in the mint arm is the *only* fence; Phase 1's unit tests must include a residual-unbound-variable case that would panic without the tail, and G1r/G1p/the new goldens serve as the end-to-end proof the tail is reached before lowering, never after. |

## Open questions

None remain open. D1 (option B, per-site binding) and D2 (fold-in concrete-arm
check) are adjudicated in Requirements. Residual uncertainties — theta-order
churn, Phase 2's unmeasured blast radius, a residual-unbound-variable shape
slipping past the fail-closed tail — are tracked in Risks & Mitigations above,
not left open here.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Per-site binding in the mint arm (poly.rs:9315-9323): re-ground + per-site unification (empty seed lists) + fail-closed tail + QuotLit fence, with unit tests", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "Concrete-winner site-slot compatibility check (poly.rs:9324-9331), gated on imp.target.is_concrete(), with unit tests", "effort": "S", "difficulty": "hard" },
    { "phase": 3, "focus": "End-to-end goldens (G1a with symbol-capture witness, G1r/G1p/G2/G3b/G4/G6, QuotLit rejection, array-element/bare-var twins) in tests/phase7b_slice8.rs", "effort": "M", "difficulty": "standard" },
    { "phase": 4, "focus": "Roadmap corrigenda 1-6 on the S8c paragraph, growth re-check, full green gate", "effort": "S", "difficulty": "standard" }
  ]
}
```
