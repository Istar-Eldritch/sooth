# P7b.S9 spec — module-aware trait-impl matching

Scoped against worktree `p7b-s9`, HEAD `600bc1b`, baseline `cargo test
--no-fail-fast` **82 binaries, 3150 tests, 0 failed**. Discovery input:
[slice9-brief](./slice9-brief.md) (recon round) and its two completed evidence
rounds — the adjudicated mechanism in [slice9-probes](./slice9-probes.md)
(verbatim probe log + verdict) and the validated golden designs in
[slice9-paper-tests](./slice9-paper-tests.md). S9 is the
[post-ship correction](./slice5-spec.md) that closed S5: the tier policy S5
built governs `env`/`select_overload` ctor-*construction* selection only, and
the cross-pick it was motivated by lives in two other mechanisms, both upstream
or downstream of the (sound) trait-impl matcher.

> **Mechanism correction (carried from the probe round; supersedes the
> roadmap).** The roadmap's S9 entry claims "no module-identity check anywhere
> in `match_impl_target` or the `select_most_specific` tie-break". **That is
> false.** The probe round proved (V1/F1/F3) that `match_impl_target_rec`'s
> `Generic` arm *does* compare header identity `(idx, module)` and resolves
> per-module correctly in every run; the matcher and pattern resolution are
> sound. The real defects are two, both off the matcher:
>
> - **V2 (operand provenance, deterministic `pb2` `2\n2`).** A bare,
>   un-annotated ctor call silently borrows an *unrelated* module's
>   eagerly-minted instantiation. In `pb2` only `b`'s `usesize` spells
>   `Widget[i64]`, so exactly one `Widget[i64]` mint exists program-wide
>   (`StructId(2)`, provenance `gi=1, module=4` — b's header). a::run's bare
>   `Widget` ctor call applies *that* mint, so both callers dispatch with
>   provenance `(1, 4)` and only b's pattern matches. The S5 tier policy never
>   fires — `ctor-select` shows a single candidate at a's call site (a's own
>   ctor is absent from the candidate set there).
> - **V3 (monomorphization identity + span-keyed wiring, nondeterministic `mk`
>   variant `1\n1`/`2\n2`).** The shared bound word `sized`'s specialization key
>   renders the grounded type's *name* (`Widget_i64_`), identical for a's and
>   b's distinct `StructId`s, so both callers share one compiled `sized` body.
>   That body's one internal `size` call is wired through `trait_calls:
>   HashMap<Span, String>` keyed on one fixed source span — last-writer-wins
>   across the two groundings, order randomized per compiler process by Rust's
>   `RandomState`. The race is in the **checker** (a compile-time HashMap): a
>   built binary reruns deterministically; rebuilds flip. (`H1`/`H5` are the
>   same mechanism seen from the specialization side and the wiring side — one
>   finding, not two.)
>
> This spec **rules** on all five decisions the brief hands the spec-writer.
> The roadmap's own sentences are corrected at slice exit (R7).

---

## Rulings

### R1 — V2 fix site: pin the missing candidate first, then ground bare ctors at the caller's own header. (Phase 1 → Phase 2)

**R1.0 (Phase 1, blocking gate).** Before choosing the V2 fix site, pin *why*
a's own header's ctor is absent from the single-candidate set at a's bare
`Widget` call in `pb2`. The probe recorded the symptom (`ctor-select` fires with
one candidate, provenance `(gi=1, module=4)` = b's) but not the registration
cause. The candidate set for a bare ctor application is fed from
`struct_generated_sigs` registration (`src/check/declarations.rs:1815`) into the
overload arm (`src/check/terms.rs:915-928`). The Phase-1 deliverable is a
recorded verdict answering: does a's own `Widget[i64]` instantiation *exist* in
the registry at that call site (and the candidate scan filters it out), or is it
*never minted* because a's construction is bare/inferred and only b's `usesize`
forces an eager mint? The two answers point at different fix sites (R1.1a vs
R1.1b); Phase 1 chooses between them with evidence, not prose.

**R1.1 (Phase 2).** A bare ctor application must ground at the **caller's own
resolved header** — the same module-scoped resolution the parser already applies
to type positions (F3, `poly_generic_header` `src/parser.rs:7101` via
`bare_generic_owner` `:7133`) — minting its own instantiation when none exists
for that header, and **never** silently substituting another module's
eagerly-minted instantiation. Two candidate sites, chosen by R1.0's verdict:

- **R1.1a (registration/application, if a's mint is absent):** the bare-ctor
  candidate registration/application path must mint (or select) the instantiation
  keyed to the caller's own header, so a's call sees a's own `Widget[i64]`
  candidate. Neighbourhood: `struct_generated_sigs`
  (`src/check/declarations.rs:1815`) and the single-candidate overload arm
  (`src/check/terms.rs:915-928`).
- **R1.1b (operand normalization, if a's mint exists but is filtered):** the
  operand handed to `find_bound_impl` must carry the caller's own header
  provenance rather than the borrowed mint's. Neighbourhood: the operand
  normalization feeding `find_bound_impl` (`src/check/poly.rs:8235`).

**The matcher is not touched.** `match_impl_target`/`..._rec` (`:8833`/`:8846`)
and `find_bound_impl`'s registry scan (`:8235`) are sound (F1/F2/V1) and stay
as-is: with identity-correct operands, distinct headers have distinct
`(idx, module)` and exactly the right pattern matches.

### R2 — V3 fix shape: distinguish groundings, key wiring per grounding. (Phase 3)

V3 has two coupled halves. The spec's ruling is that **one provenance-aware
identity discipline covers both**, but the phase must verify both goldens (G1
resolves through V2; G2 resolves through V3) rather than assuming a single edit
suffices.

**R2.1 — monomorphization identity.** The bound word's specialization key
(`sized`'s, per the `nm` evidence `sooth_mono_sized__m2__t0_Widget_i64_`) must
distinguish same-rendered-name / different-`StructId` groundings. Key on
`(StructId, module)`-aware identity, following the minting-side precedent
`instantiation_symbol` (`src/ast.rs:2869`), which already keys ctor images on
`GenericId` (`:5368` unit-pinned). The asymmetry between the rendered-name mono
key and the `GenericId`-keyed `instantiation_symbol` is the design opening: bring
the mono key up to the same identity discipline.

**R2.2 — obligation/dispatch wiring.** The obligation routing must key **per
grounding**, not per source span, so two groundings of one shared bound word
record two independent dispatch decisions. The live site is `trait_calls:
HashMap<Span, String>` with `trait_calls.insert(ob.span, symbol)` in
`resolve_user_bound` (`src/check/poly.rs:8352`, dispatch at `:8408`); the
same-shaped span-keyed maps `builtin_overloads: HashMap<Span, String>`
(`:929`/`:1046`/`:1116`; write `:2396`) are in scope for the same widening if the
fix routes through them.

**R2.3 — coupling ruling.** If R2.1 (a provenance-aware mono key) makes the two
groundings compile to two distinct `sized` bodies, each body has its own single
`size` span and the last-writer-wins race disappears without a separate map
change — one mechanism. If the mono key cannot be widened without unacceptable
blast radius, R2.2 (grounding-keyed maps) is the fallback and stands alone. The
implementer picks per Phase-3 evidence; **both G1 and G2 must resolve correctly
under whichever is chosen**, and a committed test must **never assert a
run-count ratio** (the pre-fix flip is nondeterministic; see R-NFR3).

### R3 — D3: third-module mono caller resolves to a construction-time ambiguity error. (Phase 4)

Ruling: **option (a)**. The V2 fix gives the third module (`c`, which is neither
a's nor b's module) its own 2-candidate ctor collision at its bare `Widget`
call. That collision lands in S5's `select_overload` tier policy; `c` is neither
declaring module, so it falls to the ambiguity tier → **compile-time error**.
This is consistent with the roadmap's "real ambiguity error, not a silent guess"
intent and reuses S5's tier vocabulary rather than inventing new dispatch-time
machinery.

The post-fix assertion for G4 is `build_error` pinning a **located** message
that names `Widget`, both candidate modules, and `c`'s call site. The exact text
is the S5 `select_overload` ambiguity-tier diagnostic — Phase 4 pins the byte
text against what the fixture actually produces (diagnostics are behaviour), it
is not re-derived here. If Phase-2/3 evidence shows the V2 fix does **not**
surface `c`'s call as a 2-candidate collision (e.g. `c`'s bare ctor grounds
against no visible header at all and errors earlier with `unknown type`), Phase 4
records that as the ruling instead and pins the message it does produce; option
(a) is the target, the pinned text follows the mechanism.

### R4 — D4: no new dispatch-time visibility/tier machinery in the registry scan. (design ruling; no code)

Ruling: **no new visibility filter or dispatch-time tier machinery in
`find_bound_impl`.** With identity-correct operands (R1), distinct modules'
headers never both match one operand — their `(idx, module)` identities differ —
and the import system prevents a caller from ever holding an operand whose header
identity is ambiguous to it (probes P5b/P5c: import cycle / placement rule /
selective-import collision all fire first). The one **constructible**
concrete-target ambiguity — two modules each declaring a blanket `impl: Sized
for 'T` — already errors at declaration time via `check_impl_decls`
(`src/check/declarations.rs:544`, P7.S4 R7), module-blind by design because a
bare `PolyType::Var` carries no header identity. `ImplDecl.module`
(`src/ast.rs:2492`) stays read only by the placement rule (`:588`); the scan
stays `trait_id` + pattern-match only. Revisit only if R3's ruling forces
candidate-set awareness at dispatch (it does not — R3 routes through the ctor
`select_overload` path, not `find_bound_impl`).

### R5 — Regression pins stay green, unchanged. (every phase)

- **Golden #10** `same_named_ctors_in_two_modules_dispatch_distinct_impls`
  (`tests/phase7b_slice2.rs:649`) — the only existing golden exercising
  per-operand trait-impl dispatch with **distinct** substitutions (`i64`/`str`)
  across modules. Must keep printing `1\n2`. The fix repairs the
  *same*-substitution (`pb2`) case without regressing the distinct-substitution
  case.
- **S5 tier-1** `cross_module_same_shaped_ctor_dispatches_callers_own_impl`
  (`tests/phase7b_slice5.rs:90`) — pins S5's ctor/destructure tier policy
  (`select_overload`), a different registry from S9's. Must keep printing
  `15\n25`.
- **G3** `duplicate_blanket_impl_across_modules_is_a_declared_error` — the
  declaration-time duplicate check (`check_impl_decls`, `:544`) must survive the
  fix untouched. New regression golden in `tests/phase7b_slice9.rs` (not new
  work; it pins pre-existing behaviour).

### R6 — Growth-structure re-check at phase exit. (final phase)

`src/check/poly.rs` is ~21k lines; its split remains deferred (3/5 signals, no
clean cut — recorded S5 residual). S9 is expected to touch `poly.rs` (obligation
wiring / mono identity), the ctor path in `terms.rs`/`declarations.rs`, possibly
`ast.rs` (identity helpers), and `tests/phase7b_slice9.rs` (new). Re-run the
CLAUDE.md split signals at phase exit against the files as they then stand; do
**not** preemptively split.

### R7 — Roadmap correction at slice exit. (final phase)

Rewrite the S9 entry (`docs/roadmap/P7b-higher-kinded-types.md:221`):

1. Replace the mechanism sentence ("no module-identity check anywhere in
   `match_impl_target` or the `select_most_specific` tie-break…") with the
   adjudicated two-defect story: operand provenance (a bare ctor borrows another
   module's eager mint) and monomorphization identity + span-keyed wiring (the
   shared bound word collapses distinct-`StructId` groundings). State explicitly
   that the matcher and pattern resolution are sound.
2. Correct the mono/`mk` variant's "silent `1 1`" to **nondeterministic**
   `1\n1`/`2\n2` (compiler-HashMap-seed dependent, rebuild-time flip).
3. Reword exit item 3's ambiguity clause: the constructible ambiguity shape is
   covered by the **declaration-time** duplicate check (no new dispatch-time
   mechanism); the third-module mono caller is ruled per R3 (construction-time
   ambiguity error). Do not imply "add a dispatch-time ambiguity error".

Per [[feedback_roadmap_design_no_history]]: state the current design only; no
"was X, now Y" narration in the roadmap prose itself.

---

## Requirements (traceable)

- **REQ-1** (R1.0): a recorded Phase-1 verdict on why a's own ctor is absent from
  the single-candidate set at a's bare `Widget` call in `pb2` (absent-mint vs
  filtered-mint), chosen with instrumentation evidence.
- **REQ-2** (R1.1): a bare ctor application grounds at the caller's own resolved
  header; it never substitutes another module's eagerly-minted instantiation.
- **REQ-3** (R2.1): the bound word's monomorphization key distinguishes
  same-rendered-name / different-`(StructId, module)` groundings.
- **REQ-4** (R2.2): obligation/dispatch routing records one decision per
  grounding, not one last-writer-wins entry per source span.
- **REQ-5** (R2.3): both G1 and G2 resolve to deterministic `1\n2` under the
  chosen fix shape; no committed test asserts a run-count ratio.
- **REQ-6** (R3): G4's after-column is a located compile-time ambiguity error
  naming `Widget`, both candidate modules, and `c`'s call site (or the earlier
  error the mechanism actually produces, recorded).
- **REQ-7** (R4): `find_bound_impl` gains no visibility filter or dispatch-time
  tier machinery; the declaration-time duplicate check is untouched.
- **REQ-8** (R5): #10 stays `1\n2`; S5 tier-1 stays `15\n25`; G3 pins the
  existing duplicate-impl error text.
- **REQ-9** (R7): the roadmap S9 entry is corrected to the adjudicated story.
- **REQ-10** (CLAUDE.md): every stage function edited gets a unit test beside it
  (happy path + one error/edge); every phase exit criterion is a golden.

## Non-functional requirements

- **R-NFR1 — check-stage only.** No IR change, no backend change, no lowering
  change. The linear spine, `Ptr[T]` opacity, QBE-only backend invariants are
  untouched (nothing here reaches lowering). If any candidate fix appears to need
  an IR/lowering edit, stop and escalate — that is out of scope.
- **R-NFR2 — matcher untouched.** `match_impl_target`/`..._rec` and
  `find_bound_impl`'s scan are expected to have **zero** behavioural diff. A
  Phase-exit check confirms no edit landed there (or, if one did, justifies it
  against F1/F2/V1).
- **R-NFR3 — never assert nondeterminism.** G2's pre-fix flip is HashMap-seed
  nondeterminism. The pre-fix evidence is noted in a scratch check (run N× before
  the fix), never encoded as a ratio assertion in committed tests. The committed
  G2 asserts only the post-fix deterministic `1\n2`.
- **R-NFR4 — green gate.** `cargo fmt --check && cargo clippy -- -D warnings &&
  cargo test` green at every phase exit; no dead parameters (clippy `-D
  warnings`).
- **R-NFR5 — baseline anchor.** Every phase is independently verifiable against
  the paper goldens with the baseline stated: **3150 tests, 0 failed at HEAD
  `600bc1b`**. A phase that adds N goldens/units expects `3150 + N` passing, 0
  failed.

## Success criteria (anchored in the validated goldens; before/after are measured facts, not re-derived)

| Golden | Name | Before (measured) | After |
| --- | --- | --- | --- |
| **G1** | `cross_module_same_shaped_impls_dispatch_each_callers_own_impl` (verbatim `pb2`) | `2\n2`, exit 0, deterministic (3 cycles) | `1\n2` |
| **G2** | `..._via_named_instantiation_dispatch_each_callers_own_impl` (`mk` variant) | **nondeterministic** `1\n1`/`2\n2` (8/2 paper, 6/4 probe — never asserted) | deterministic `1\n2` |
| **G2r** | `..._eager_minter_wins_regardless_of_caller` (a eager / b bare — cleanest V2 witness; **primary provenance regression pin**) | `1\n1`, exit 0, deterministic (7 cycles) | `1\n2` |
| **G3** | `duplicate_blanket_impl_across_modules_is_a_declared_error` | `error: duplicate`impl:` for `'T`(line 3, col 1); first declared at line 3, col 1` | unchanged (regression pin) |
| **G4** | `third_module_mono_caller_sees_the_wrong_impl_silently` | silent `2`, exit 0 (3 cycles) | located ambiguity `build_error` (R3) |
| **G5** | #10 (`tests/phase7b_slice2.rs:649`); S5 tier-1 (`tests/phase7b_slice5.rs:90`) | `1\n2`; `15\n25` | unchanged |

Fixture text for G1/G2/G2r/G3/G4 is preserved verbatim in
[slice9-paper-tests](./slice9-paper-tests.md); use it as-is (built and validated
at HEAD `600bc1b`).

**Unit-level success criteria** (behaviour-level; fix sites are R1/R2's choice):

- monomorphization identity distinguishes same-rendered-name / different-
  `StructId` groundings (near `instantiation_symbol`, `src/ast.rs:2869`);
- obligation routing keyed per grounding, not per span
  (`HashMap<Span, String>` shapes at `src/check/poly.rs:929/1046/1116/2396` and
  `trait_calls` in `resolve_user_bound` `:8352`);
- a lazily-minted, un-annotated bare-ctor operand's provenance is never borrowed
  from an unrelated eager mint (R1's fix site);
- `check_impl_decls`'s blanket-impl duplicate check is unaffected
  (`src/check/declarations.rs:544`).

## Scope and boundaries

**In scope:** check-stage repair of V2 (operand provenance) and V3
(monomorphization identity + obligation wiring); the D3 third-module ruling via
S5's existing `select_overload` tier policy; new `tests/phase7b_slice9.rs`; the
roadmap correction.

**Out of scope / untouched:**

- IR, lowering, backend — R-NFR1.
- `match_impl_target`/`match_impl_target_rec` and `find_bound_impl`'s registry
  scan — R-NFR2 / R4 (sound per F1/F2/V1).
- New dispatch-time visibility or tier machinery in the registry — R4.
- The declaration-time duplicate-impl check — R5/G3 (kept as-is).
- S5's `select_overload` tier *policy* — reused by R3, not re-authored.
- Any new trait surface, declaration syntax, or user-facing spelling.

## Codebase map (verified anchors, HEAD `600bc1b`, from the brief's machinery map)

- **V2 fix neighbourhood (ctor application).** single-candidate overload arm
  `src/check/terms.rs:915-928`; minted-candidate registration
  `struct_generated_sigs` `src/check/declarations.rs:1815`; module-scoped ctor
  name resolution `poly_generic_header` `src/parser.rs:7101`, `bare_generic_owner`
  `:7133`, unit-pinned
  `impl_target_module_generic_ctor_target_names_the_ctor_module`
  `src/check/declarations.rs:4349`.
- **V3 fix neighbourhood.** monomorphization key rendering (`sized` — nm
  evidence); minting-side precedent `instantiation_symbol` `src/ast.rs:2869`
  (keys on `GenericId`, `:5368` unit-pinned); `GenericId` `src/ast.rs:2521`;
  `GenericTypes::struct_instantiation_of` `src/ast.rs:1118`,
  `enum_instantiation_of` `:1128`; `mangle` `src/resolve.rs:36`. Obligation
  wiring `resolve_user_bound` `src/check/poly.rs:8352` (dispatch `:8408`),
  `trait_calls.insert(ob.span, symbol)`; span-keyed memos `builtin_overloads`
  `:929`/`:1046`/`:1116` (write `:2396`).
- **Matcher (untouched).** `find_bound_impl` `src/check/poly.rs:8235` (candidate
  loop `:8262-8276`, `candidate_bounds_discharge` `:8192`); `select_most_specific`
  `:8073` (`ambiguity_error` `:8099`, `specificity` `:9640`);
  `match_impl_target` `:8833` / `..._rec` `:8846` (`Generic` arm identity
  compare `:8995`). Call sites: inline-splice `:1722`, mono-member viability
  `:2239` (in `resolve_mono_member_call` `:2165`), where-clause discharge
  `:8192`, bound dispatch `:8408`.
- **D3 (reused tier policy).** S5's `select_overload` (in `src/check/builtins.rs`
  per S5 spec) and the ctor-select path feeding it from `terms.rs`.
- **Impl record + duplicate check (untouched).** `ImplDecl` `src/ast.rs:2492`;
  placement rule `src/check/declarations.rs:575-588`; blanket-impl duplicate
  check `:544`.
- **Whole-program env.** `poly_env` built once `src/check.rs:684-698`;
  `overload_symbols` `$$N` suffixing `src/check.rs:697`.
- **Goldens in play.** #10 `tests/phase7b_slice2.rs:649`; S5 tier-1
  `tests/phase7b_slice5.rs:90`; `pb2` fixture verbatim in
  [slice9-paper-tests](./slice9-paper-tests.md) G1.
- **Roadmap entry to correct.** `docs/roadmap/P7b-higher-kinded-types.md:221`.

## Open questions and risks

- **OQ-1 (resolved by Phase 1, gates Phase 2).** Absent-mint vs filtered-mint at
  a's bare ctor call (REQ-1). The V2 fix site (R1.1a vs R1.1b) depends on the
  answer; Phase 2 does not start the fix until Phase 1 records the verdict.
- **OQ-2 (resolved by Phase 3).** Does a provenance-aware mono key (R2.1) alone
  dissolve the V3 race (one mechanism), or is a grounding-keyed map (R2.2) also
  needed (two changes)? R2.3 rules the implementer picks per evidence; risk is
  scope creep if both land where one suffices — Phase 3 must justify the second.
- **OQ-3 (resolved by Phase 4).** Does the V2 fix actually surface `c`'s bare
  ctor as a 2-candidate `select_overload` collision (R3 option (a)), or does `c`
  ground against no visible header and error earlier? R3 pins whichever the
  mechanism produces; the risk is asserting a message the fix does not emit —
  pin against fixture output, not spec prose.
- **RISK-1.** Widening the mono key (R2.1) has blast radius across every
  monomorphized bound word, not just `sized`. Mitigation: R-NFR5 baseline anchor
  after each phase; if the widening reds unrelated goldens, fall back to R2.2
  (grounding-keyed maps, narrower).
- **RISK-2.** The V2 fix could perturb #10's distinct-substitution dispatch
  (F4). Mitigation: R5/G5 keeps #10 green as a per-phase gate.
- **RISK-3 (`poly.rs` size).** Editing the ~21k-line `poly.rs` again; the split
  stays deferred (R6). Do not preemptively split; re-run signals at exit.

---

## Phased delivery plan

Baseline for every phase: **3150 tests, 0 failed at HEAD `600bc1b`**. Each phase
is independently verifiable against the paper goldens and leaves the tree green
(`cargo fmt --check && cargo clippy -- -D warnings && cargo test`).

- **Phase 1 — pin the missing candidate (V2 diagnosis gate).** Instrument the
  bare-ctor candidate path in `pb2` and record the REQ-1 verdict
  (absent-mint vs filtered-mint). No src behavioural change; no golden. Exit: the
  verdict is written into the slice notes and selects R1.1a vs R1.1b; suite still
  3150/0.
- **Phase 2 — V2 fix + G1/G2r goldens.** Implement R1.1 at the site Phase 1
  chose: a bare ctor grounds at the caller's own header, never borrowing another
  module's mint. Add G1 (`pb2` → `1\n2`) and G2r (eager-mirror → `1\n2`, primary
  provenance pin) to `tests/phase7b_slice9.rs`, plus the R1 unit test
  (un-annotated bare-ctor provenance not borrowed). Matcher untouched (R-NFR2).
  Exit: G1 + G2r green; #10 and S5 tier-1 unchanged; suite 3150+N/0.
- **Phase 3 — V3 fix + G2 golden.** Implement R2 (mono identity, and/or
  grounding-keyed wiring per R2.3) so the shared bound word's two groundings
  dispatch independently and deterministically. Add G2 (`mk` variant → `1\n2`,
  no ratio assertion — R-NFR3) plus the two V3 unit tests (mono identity
  distinguishes groundings; obligation routing keyed per grounding). Exit: G2
  deterministic `1\n2`; the pre-fix flip noted only in scratch; suite green.
- **Phase 4 — D3 ruling + G3/G4 goldens.** Add G3 (regression pin for the
  existing declaration-time duplicate error text — R5) and G4 (third-module mono
  caller → located ambiguity `build_error` per R3, message pinned against fixture
  output). Confirm no new machinery in `find_bound_impl` (R4). Exit: G4 and G3
  green; suite green.
- **Phase 5 — roadmap correction + growth-structure re-check + final gate.**
  Apply R7 to `docs/roadmap/P7b-higher-kinded-types.md:221`. Re-run CLAUDE.md
  split signals against every file S9 touched (R6); record the verdict (expected:
  split still deferred). Final green gate; confirm all S9 goldens + #10 + S5
  tier-1 pass and no edit landed in the matcher (R-NFR2).

## Phases (JSON)

```json
{
  "baseline": { "head": "600bc1b", "tests": 3150, "failed": 0 },
  "phases": [
    {
      "id": 1,
      "name": "Pin the missing candidate (V2 diagnosis gate)",
      "requirements": ["REQ-1"],
      "changes": [
        "Instrument the bare-ctor candidate path (struct_generated_sigs src/check/declarations.rs:1815; overload arm src/check/terms.rs:915-928) on pb2",
        "Record verdict: is a's Widget[i64] mint absent (never minted, bare/inferred) or present-but-filtered from the candidate scan"
      ],
      "goldens": [],
      "units": [],
      "exit": "REQ-1 verdict recorded, selecting R1.1a (registration/application) vs R1.1b (operand normalization); no src behavioural change; suite 3150/0 green",
      "notes": "Blocking gate: Phase 2 fix site depends on this verdict (OQ-1). Matcher untouched (R-NFR2)."
    },
    {
      "id": 2,
      "name": "V2 fix (operand provenance) + G1/G2r goldens",
      "requirements": ["REQ-2", "REQ-8", "REQ-10"],
      "changes": [
        "Implement R1.1 at the Phase-1-chosen site: bare ctor grounds at caller's own resolved header (src/parser.rs:7101 module-scoped resolution precedent); never substitute another module's eager mint",
        "Matcher and find_bound_impl scan untouched (R-NFR2/R4)"
      ],
      "goldens": [
        "tests/phase7b_slice9.rs: cross_module_same_shaped_impls_dispatch_each_callers_own_impl (G1, pb2) -> 1\\n2",
        "tests/phase7b_slice9.rs: cross_module_same_shaped_impls_eager_minter_wins_regardless_of_caller (G2r, primary provenance pin) -> 1\\n2"
      ],
      "units": [
        "un-annotated bare-ctor operand provenance is not borrowed from an unrelated eager mint (R1 fix site)"
      ],
      "exit": "G1 + G2r print 1\\n2 deterministically; #10 stays 1\\n2, S5 tier-1 stays 15\\n25; fmt/clippy/test green; suite 3150+N/0",
      "notes": "V2 deterministic pb2 2\\n2 -> 1\\n2. Fix site per REQ-1 (R1.1a or R1.1b)."
    },
    {
      "id": 3,
      "name": "V3 fix (mono identity + obligation wiring) + G2 golden",
      "requirements": ["REQ-3", "REQ-4", "REQ-5", "REQ-10"],
      "changes": [
        "R2.1: widen the bound word's monomorphization key to distinguish same-rendered-name/different-(StructId,module) groundings (precedent instantiation_symbol src/ast.rs:2869)",
        "R2.2 (if R2.3 evidence requires): key trait_calls (src/check/poly.rs:8352) and span-keyed memos (:929/:1046/:1116/:2396) per grounding, not per source span"
      ],
      "goldens": [
        "tests/phase7b_slice9.rs: cross_module_same_shaped_impls_via_named_instantiation_dispatch_each_callers_own_impl (G2, mk variant) -> deterministic 1\\n2 (NEVER assert a run-count ratio, R-NFR3)"
      ],
      "units": [
        "monomorphization identity distinguishes same-rendered-name/different-StructId groundings (near src/ast.rs:2869)",
        "obligation/dispatch routing records one decision per grounding, not one last-writer-wins entry per span"
      ],
      "exit": "G2 prints 1\\n2 deterministically; pre-fix flip noted only in scratch, not committed; one-mechanism vs two-change choice (R2.3) justified against evidence; suite green",
      "notes": "V3 nondeterministic 1\\n1/2\\n2 -> deterministic 1\\n2. RISK-1: mono-key blast radius; fall back to R2.2 if it reds unrelated goldens."
    },
    {
      "id": 4,
      "name": "D3 ruling + G3/G4 goldens",
      "requirements": ["REQ-6", "REQ-7", "REQ-8"],
      "changes": [
        "Confirm the V2 fix surfaces c's bare ctor as a 2-candidate select_overload collision -> S5 ambiguity tier -> compile-time error (R3 option a)",
        "No new visibility/tier machinery in find_bound_impl (R4); declaration-time duplicate check untouched (src/check/declarations.rs:544)"
      ],
      "goldens": [
        "tests/phase7b_slice9.rs: duplicate_blanket_impl_across_modules_is_a_declared_error (G3) -> pins existing 'error: duplicate `impl:` for `'T` ...' text",
        "tests/phase7b_slice9.rs: third_module_mono_caller_sees_the_wrong_impl_silently (G4) -> build_error: located ambiguity naming Widget, both candidate modules, c's call site (or the earlier error the mechanism produces, recorded)"
      ],
      "units": [
        "check_impl_decls blanket-impl duplicate check unaffected by the fix (two impl: Trait for 'T in different modules still error)"
      ],
      "exit": "G3 + G4 green; G4 message pinned against actual fixture output (not spec prose); find_bound_impl gained no new machinery; suite green",
      "notes": "OQ-3: if V2 fix doesn't surface c's collision (grounds against no visible header), record that ruling and pin the error it does emit."
    },
    {
      "id": 5,
      "name": "Roadmap correction + growth re-check + final gate",
      "requirements": ["REQ-9", "REQ-10"],
      "changes": [
        "R7: rewrite docs/roadmap/P7b-higher-kinded-types.md:221 to the two-defect story (operand provenance; mono identity + span-keyed wiring), correct 'silent 1 1' to nondeterministic 1\\n1/2\\n2, reword exit item 3 (declaration-time duplicate check covers the constructible shape; third-module caller per R3). No history narration.",
        "R6: re-run CLAUDE.md split signals against every file S9 touched (poly.rs, terms.rs, declarations.rs, ast.rs); record verdict (expected: split still deferred)"
      ],
      "goldens": [],
      "units": [],
      "exit": "Roadmap corrected; growth-signal verdict recorded; final fmt/clippy/test green; all S9 goldens + #10 + S5 tier-1 pass; no edit landed in match_impl_target/find_bound_impl scan (R-NFR2)",
      "notes": "Documentation + verification phase; no new behaviour."
    }
  ]
}
```
