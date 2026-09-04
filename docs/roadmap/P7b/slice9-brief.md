# P7b.S9 brief — module-aware trait-impl matching (recon round, probes + paper complete)

Scope input for the S9 spec. Produced by a recon round against the clean tree
(worktree `p7b-s9`, HEAD `600bc1b`), then adjudicated by a probe round
(verbatim log and verdict: [slice9-probes](./slice9-probes.md)) and a
paper-test round (validated golden designs:
[slice9-paper-tests](./slice9-paper-tests.md)). Repo untouched throughout
(`git status --porcelain` shows only these four docs). Baseline at HEAD:
`cargo test --no-fail-fast` **82 binaries, 3149 passing + 1 known-flaky, 0
unconditionally failed** — see item 2 below; `tests/phase7b_slice4.rs:427-490`
is G2's same-shape twin (different trait and text), hard-pinned to the pre-fix
coin-flip, and reds ~3/8 on rerun.

S9's scope is the [post-ship correction](./slice5-spec.md) that closed S5: the
tier policy S5 built governs `env`/`select_overload` ctor-construction
selection only, and the collision it was motivated by lives elsewhere — plus
one *newly reachable* wrong output the same correction recorded. **The probe
round has rewritten the roadmap's mechanism story** (see "The adjudicated
mechanism"): the roadmap's "no module-identity check anywhere in
`match_impl_target`" is false as stated, and the real defects are two, both
upstream or downstream of the matcher, which is sound.

## What the slice must close (roadmap exit, `P7b-higher-kinded-types.md:221`, as amended by evidence)

1. **`pb2` cross-pick** — deterministic `2` `2` at HEAD; each caller's own impl
   would print `1` `2`. Mechanism: V2 below (operand provenance), *not*
   impl-target matching.
2. **The same-payload `mk` variant** — the roadmap/post-ship records "a silent
   `1 1` at exit 0"; the probes measured **nondeterministic** `1 1`/`2 2`
   (probe: 6/4 over 10 runs; paper: 8/2 over 10 rebuild+run cycles — the
   compiler's own HashMap iteration order decides, so the outcome is not
   source-determined). Mechanism: V3 below (`instantiation_symbol`'s
   non-injective fall-through arm collapsing two groundings to one symbol,
   collapsed further by lowering's symbol-keyed dedup — not a span-keyed
   wiring defect). The roadmap's `1 1` sentence needs correcting at slice
   exit.
3. **Real ambiguity, not a guess** — the probe/paper rounds found the
   *constructible* ambiguity shape (two modules each declaring a blanket
   `impl: Sized for 'T`) **already errors today** — declaration-time
   `duplicate impl:` check (`check_impl_decls`, `src/check/declarations.rs:544`,
   P7.S4 R7), module-blind by design because a bare `PolyType::Var` carries no
   header identity. Concrete-target dispatch-time ambiguity is structurally
   unconstructible (import cycles / placement rule / selective-import
   collision all fire first — probes P5b/P5c). The remaining genuinely open
   behaviour is the **third-module mono caller** (P5a/G4: silently prints `2`
   today); its post-fix expectation is a spec determination (see D3 — Phase 4
   must build the fixture and observe, not assume the outcome).
4. **Goldens pin the resolved shape(s)** — G1/G2/G2r in the paper doc; the
   existing ambiguity and regression pins stay green (G3/G5).

## What the recon round established (static, confirmed by the probe round)

- **F1 — the `Generic` target arm already compares header identity.**
  `match_impl_target_rec`'s `PolyType::Generic` arm (`src/check/poly.rs:8995`
  onward) matches a `Type::CtorImage` operand on
  `GenericId { is_enum, idx, module }` equality, and a `Type::Struct`/`Enum`
  operand by recovering the mint's provenance via
  `GenericTypes::struct_instantiation_of` (`src/ast.rs:1118`) and comparing
  `(found_idx, found_module)` against the pattern's `(idx, module)`. The probe
  traces confirm it: with correct operands, exactly the right pattern matches
  (H3 ruled out).
- **F2 — the registry scan itself is module-blind, and `ImplDecl.module` is
  never read at dispatch.** `find_bound_impl` (`src/check/poly.rs:8235`)
  filters candidates on `trait_id` + pattern match only. `ImplDecl` carries
  `module: u32` (`src/ast.rs:2492`) but its only reader is the *placement*
  rule in `check_impl_decls` (`src/check/declarations.rs:588`). With
  identity-correct operands this is harmless (distinct headers have distinct
  `(idx, module)`); whether S9 still owes the scan a module/visibility
  dimension is D4.
- **F3 — the parser resolves impl-target ctor names module-scoped.**
  `poly_generic_header` (`src/parser.rs:7101`) via `bare_generic_owner`
  (`src/parser.rs:7133`); unit-pinned at
  `impl_target_module_generic_ctor_target_names_the_ctor_module`
  (`src/check/declarations.rs:4355`). Confirmed live by the probe traces
  (`pattern_id=Some((false, 0, 3))` vs `Some((false, 1, 4))`).
- **F4 — golden #10 (same shape, different payloads) passes.**
  `tests/phase7b_slice2.rs:655` (assert `:696`) prints `1` `2` with
  `i64`/`str` payloads through the shared bound word. Whatever breaks pb2 does
  not break per-operand dispatch when the two groundings' substitutions differ
  — the probe confirmed dispatch is correct per-operand in the `mk` variant
  (V3).
- **F5 — `trait_calls` is per-instantiation, not shared; the collapse is in
  lowering's dedup.** `resolve_user_bound`'s `trait_calls.insert(ob.span,
  symbol)` (`src/check/poly.rs:8663`) writes into a `HashMap<Span, String>`
  created fresh per instantiation (`:7235`) and moved onto that
  instantiation's own `CallInst` (`:7384`; field `CallInst.trait_calls`,
  `src/ast.rs:2808`) — two groundings own two separate maps and never
  overwrite each other. The actual collapse is one stage later, in lowering's
  instantiation dedup (`src/ir/driver.rs:350-373`): a `HashSet<String>` keyed
  on `instantiation_symbol(&inst.callee, &inst.subst)`, iterating a randomized
  `HashMap`; when two groundings' substitutions render the same symbol (see
  F5b), the `HashSet::insert` keeps only the first `CallInst` reached, and the
  losing grounding's entire `CallInst` — `trait_calls` map included — is
  discarded whole, not overwritten entry-by-entry. `builtin_overloads`
  (`:929/:1046/:1116`, write `:2396`) is a different, span-keyed memo for a
  different call shape (mono member calls); it is not implicated in this
  defect and needs no change.
- **F5b — `instantiation_symbol` is already the mono key, and its fall-through
  arm is non-injective.** `instantiation_symbol` (`src/ast.rs:2869`) mints the
  `sooth_mono_...` symbol directly (`nm` evidence:
  `sooth_mono_sized__m2__t0_Widget_i64_`); its own doc comment (`:2864-2866`)
  states it is the single source of truth for both "the checker's call-site
  table and the lowered `IrFunc.name`". Its `Type::CtorImage` arm already keys
  on `GenericId` (`:2521`); its fall-through arm for every other `Type`
  (`other => other.name().to_string()`, `:2886`) renders only the type's
  name, which is identical for `sized`'s two `Widget[i64]` groundings
  (`StructId(2)`/module 3, `StructId(3)`/module 4) despite their distinct
  provenance.
- **F6 — true ambiguity already errors.** `select_most_specific`
  (`src/check/poly.rs:8073`) raises `ambiguity_error` on 2+ incomparable
  maxima; equal patterns are never strictly more specific
  (`:19466`). No silent first-match pick can come from the tie-break.
- **F7 — declaration-time duplicate-impl check is module-blind structural
  equality** (`check_impl_decls`, `src/check/declarations.rs:544`, P7.S4 R7):
  two blanket `for 'T` impls anywhere in one program collide (paper G3
  validates the error text, measured exit 1); two `for Widget` impls in
  different modules do not, because `Generic` patterns carry `(idx, module)`.
- **F8 — S5's shipped goldens bound the blast radius.**
  `cross_module_same_shaped_ctor_dispatches_callers_own_impl`
  (`tests/phase7b_slice5.rs:100`, assert `:129`, prints `15\n25`) pins the
  ctor tier policy; golden #10 pins distinct-subst dispatch. Both must pass
  unchanged. A third test, `tests/phase7b_slice4.rs:427-490`
  (`same_named_ctor_mk_ambiguity_resolves_but_impl_dispatch_still_cross_picks`),
  is G2's same-shape twin (different trait and text) and is currently a
  **flaky, not a stable**, regression
  pin — it hard-asserts the pre-fix `1\n1` coin-flip and reds ~3/8 on rerun; it
  must be re-pinned, not merely kept green.

## The adjudicated mechanism (probe round; log lines in slice9-probes.md)

Two live defects, one ruled-out hypothesis, one collapsed pair:

- **V1 — H3 (pattern resolution) RULED OUT.** Candidate patterns resolve
  per-module correctly in every run; `match_impl_target`'s identity comparison
  is sound.
- **V2 — H2 LIVE (pb2, deterministic): operand provenance is borrowed from an
  unrelated eager mint.** In pb2 only b's `usesize` spells `Widget[i64]`, so
  exactly one `Widget[i64]` mint exists program-wide (`StructId(2)`, provenance
  `gi=1 module=4` — b's header). a::run's bare, unannotated `Widget` ctor call
  applies **that** instantiation: both dispatch entries see
  `ty=Struct(StructId(2))` with provenance `(gi=1, module=4)`, so only b's
  pattern `(1, 4)` matches and both callers print b's constant. The S5 tier
  policy never fires — the `ctor-select` trace shows **only one candidate** at
  a's call site (a's own ctor is not in the candidate set at that site), so
  there is no collision to disambiguate; the bug is that a's construction
  silently reuses b's mint instead of minting its own header's instantiation.
  The `mk`-variant control proves the tier policy itself is sound: when both
  sides spell `Widget[i64]`, `ctor-select` fires with two candidates and picks
  tier-1 own-module for both callers.
- **V3 — H1/H5 LIVE (mk variant, nondeterministic): `instantiation_symbol` is
  non-injective on grounding identity, so lowering's dedup collapses two
  `CallInst`s into one.** `nm` shows ONE `sooth_mono_sized__m2__t0_Widget_i64_`
  (keyed on the grounded type's *rendered name*, identical for a's and b's
  distinct `StructId`s) but TWO `size` specializations (`..._m3_...`,
  `..._m4_...`). Dispatch is *correct* per-operand (winners `impl_idx=0` for
  a's `StructId(2)`/`(gi=0,m3)` operand, `impl_idx=1` for b's
  `StructId(3)`/`(gi=1,m4)` — trace in slice9-probes.md), and each grounding's
  `trait_calls` map (F5) is correct and separate. The collapse happens one
  stage later, in `src/ir/driver.rs:350-373`'s dedup loop: it keys a
  `HashSet<String>` on `instantiation_symbol(&inst.callee, &inst.subst)` while
  iterating a randomized `HashMap`; both groundings render the identical
  symbol (F5b), so `HashSet::insert` keeps only the first `CallInst` reached
  — discarding the other grounding's `CallInst` (and its correct `trait_calls`
  map) whole. Which grounding survives depends on `HashMap` iteration order,
  reseeded per compiler process (Rust `RandomState`). Hence `1 1`/`2 2` per
  compilation. Nondeterminism locus note: the race is in the *checker's own
  build of `module.instantiations`* (a compile-time HashMap), so a built
  binary reruns deterministically and rebuilds flip — measured directly: one
  binary stable across 5 reruns, 6/8 rebuilds → `1 1`, 2/8 → `2 2`. The probe
  log's "10x clean binary" reading ("the same binary flips… with no source
  change") was a mislabelled series of rebuild+run cycles, not same-binary
  reruns — see the errata in slice9-probes.md. Committed tests must never
  assert a ratio.
- **H5 is not a separate finding** — the "member-word identity collapse" and
  the "dedup collision" are the same mechanism (`instantiation_symbol`'s
  non-injective fall-through arm) seen from the specialization side and the
  dedup side.

## Machinery map (verified anchors, this tree, HEAD `600bc1b`)

- **Registry scan.** `find_bound_impl` (`src/check/poly.rs:8235`); candidate
  loop `:8262-8276`; `candidate_bounds_discharge` recursion `:8192`;
  `select_most_specific` `:8073`, `ambiguity_error` `:8099`, `specificity`
  `:9640`. Call sites: inline-splice path `:1722`, mono-member viability
  `:2239` (inside `resolve_mono_member_call`, `:2165`), where-clause discharge
  `:8192`, bound dispatch `:8408` (inside `resolve_user_bound`, `:8352`).
- **Pattern matcher.** `match_impl_target` `:8833` / `..._rec` `:8846`;
  `Generic` arm identity comparison (F1). Sound; likely untouched by the fix.
- **Memoization (per-instantiation, untouched by the fix).**
  `builtin_overloads: HashMap<Span, String>` (`:929`, `:1046`, `:1116`; write
  `:2396`, a different call shape, not implicated); `CallInst.trait_calls`
  (`src/ast.rs:2808`), created fresh per instantiation `src/check/poly.rs:7235`,
  moved onto the instantiation `:7384`, written by `resolve_user_bound`'s
  `trait_calls.insert(ob.span, symbol)` (`:8663`) — correct and separate per
  grounding; consumed by lowering (`src/ir/driver.rs:414`,
  `src/ir/func_builder/calls.rs:375/385`, `src/ir/destructors.rs:379` via
  `empty_trait_calls` `src/ir.rs:130`), which is why re-keying it would be an
  IR/lowering edit, out of scope.
- **Monomorphization identity (the actual fix site) + lowering dedup (the
  actual collapse site).** `instantiation_symbol` (`src/ast.rs:2869`) already
  *is* the mono key (doc `:2864-2866`: "the checker's call-site table and the
  lowered `IrFunc.name`" share one source of truth); its `CtorImage` arm keys
  on `GenericId` (`:2521`, `:5368` unit-pinned) but its fall-through arm for
  every other `Type` (`other => other.name().to_string()`, `:2886`) renders
  only the rendered name, colliding `sized`'s two `Widget[i64]` groundings
  into `sooth_mono_sized__m2__t0_Widget_i64_` — nm evidence. The collision
  surfaces at `src/ir/driver.rs:350-373`'s dedup loop (`HashSet<String>` keyed
  on the same `instantiation_symbol`, iterating randomized
  `module.instantiations`): first `CallInst` reached wins, the other's is
  discarded whole. There is no separate "asymmetry to bring up to a shared
  discipline" — one function, one arm to widen.
- **Impl record + placement + dedup (impl registry, untouched).** `ImplDecl`
  (`src/ast.rs:2492`); placement rule `src/check/declarations.rs:575-588`;
  blanket-impl duplicate check `check_impl_decls` `:544` (module-blind
  structural equality — keep as-is, G3 pin).
- **Target resolution.** `poly_generic_header` (`src/parser.rs:7101`),
  `bare_generic_owner` (`:7133`), `parse_impl_target_pattern` (`:4112`).
- **Instantiation identity.** `GenericTypes::struct_instantiation_of`
  (`src/ast.rs:1118`), `enum_instantiation_of` (`:1128`), `GenericId`
  (`:2521`), `mangle` (`src/resolve.rs:36`).
- **Whole-program env.** `poly_env` built once over the assembled program
  (`src/check.rs:684-698`); `overload_symbols` `$$N` suffixing (Slice 8a fix 1,
  `src/check.rs:697`).
- **Ctor application path (V2's fix neighbourhood).** The single-candidate arm
  (`src/check/terms.rs:932`, `[only] =>`) and the tier-selection path
  (`src/check/terms.rs:968-991`, `select_overload_fallback_sourced`/
  `select_overload`); the minted-candidate registration (`struct_generated_sigs`,
  `src/check/declarations.rs:1824`); in pb2, a's bare ctor call sees exactly
  one candidate (b's minted `Widget[i64]`) — why a's own header's ctor is
  absent from that candidate set at that site is the first implementation-time
  pin for the V2 fix.
- **Goldens in play.** #10 `tests/phase7b_slice2.rs:655`; S5 tier-1
  `tests/phase7b_slice5.rs:100`; the pre-existing flaky pin
  `tests/phase7b_slice4.rs:427-490` (G2's same-shape twin — different trait and
  text — must be re-pinned, not
  merely kept green); `pb2` fixture text verbatim in
  [slice5-probes.md](./slice5-probes.md) and preserved complete in
  [slice9-paper-tests.md](./slice9-paper-tests.md) G1.

## Fix shape guidance for the spec (from the adjudicated mechanism)

- **V2 (provenance).** A bare ctor application must ground at the *caller's
  own resolved header* (the same module-scoped resolution the parser already
  applies to type positions, F3), minting its own instantiation when none
  exists for that header, and must never silently substitute another module's
  eagerly-minted instantiation. Candidate sites: the single-candidate overload
  arm / candidate registration in the ctor path (machinery map above) or
  operand normalization feeding `find_bound_impl`; the spec picks after
  pinning why a's ctor is missing from the candidate set (first
  implementation task). The matcher (F1) needs no change.
- **V3 (mono identity — one mechanism, one fix site, resolved).** Widen
  `instantiation_symbol`'s `Type::Struct`/`Type::Enum` fall-through arm
  (`src/ast.rs:2886`) to render the `StructId`/`EnumId` the matched variant
  already carries (globally unique across modules — no module component
  needed), the same way its `CtorImage` arm already keys on `GenericId`. `trait_calls` is
  never touched — it is already per-instantiation-correct (F5); re-keying it
  would be inert (the collision is in lowering's symbol-keyed dedup, not in
  this map) and out of bounds (it is lowering-consumed, an IR-adjacent edit).
  G1/G2 both resolve correctly under this one change.
- **D3 (third-module mono caller, G4's after-column) — a determination, not a
  confirmation.** R1.1's own rule ("ground at the caller's own resolved
  header") conflicts with assuming `c` gets a 2-candidate collision: `c`
  declares no `Widget` header at all, and probes P5a-ii already measured that
  a third module naming `Widget[i64]` explicitly with no header of its own is
  a hard `unknown type` error. The spec must have Phase 4 build the fixture
  against the landed fix and observe which of (a) 2-candidate collision →
  S5 ambiguity tier, or (b) no visible header → an earlier error, actually
  happens, then pin that — not assume (a) up front.
- **D4 (visibility/tier dimension for the registry).** With identity-correct
  operands, distinct modules' headers never both match one operand and the
  constructible ambiguity surface is already covered by the declaration-time
  duplicate check (G3). Recommend: no new visibility filter or dispatch-time
  tier machinery in `find_bound_impl`; revisit only if D3's determination
  requires candidate-set awareness at dispatch. The spec records the ruling
  either way.
- **Roadmap corrections at slice exit (five falsified targets, not one).** Beyond
  the S9 entry's first mechanism sentence ("no module-identity check anywhere
  in `match_impl_target`..."), a *second* sentence in the same entry
  ("`find_bound_impl`'s target-pattern matching itself being blind...") is
  equally falsified; the stale anchor `poly.rs:8218` (real: `:8235`) appears in
  both the roadmap and `slice5-spec.md:776`; and the "silent `1 1`" claim
  actually lives at `slice5-spec.md:783`, not in the roadmap entry at all
  (which never mentions `mk` or `1 1`). All five need correcting: the two
  roadmap sentences (adjudicated two-defect story, matcher sound), the stale
  anchor at both sites, and the two falsified `slice5-spec.md` claims (marked
  as corrections to the historical post-ship-correction record, not silently
  rewritten). Reword the roadmap's exit item 3 to the evidence
  (declaration-time duplicate check covers the constructible shape;
  third-module caller per D3's determination, whichever outcome landed).

## Paper tests (validated designs; full detail in slice9-paper-tests.md)

- **G1** `cross_module_same_shaped_impls_dispatch_each_callers_own_impl` —
  verbatim pb2; before `2\n2` (deterministic, 3 cycles), after `1\n2`.
- **G2** `..._via_named_instantiation_...` — mk variant; before
  nondeterministic `1 1`/`2 2` (10 cycles; never assert a ratio in code), after
  `1\n2`. The flakiness itself is the pre-fix evidence.
- **G2r** `..._eager_minter_wins_regardless_of_caller` — provenance mirror
  (a eager / b bare); before deterministic `1\n1` (5 cycles) — the cleanest
  isolated V2 witness; after `1\n2`. Recommended primary regression pin.
- **G3** `duplicate_blanket_impl_across_modules_is_a_declared_error` — pins
  the existing declaration-time duplicate error text (measured: `error:
  duplicate \`impl:\` for \`'T\` (line 3, col 1); first declared at line 3, col
  1`, exit 1); regression pin, not new work.
- **G4** `third_module_mono_caller_is_not_silently_cross_picked` — before:
  silent `2` (3 cycles); after-column = D3's Phase-4 determination (build
  first, pin whichever outcome (a)/(b) actually happens).
- **G5** regression pins: #10 must stay `1\n2`; S5 tier-1 must stay `15\n25`;
  the pre-existing flaky `tests/phase7b_slice4.rs:427-490` must be re-pinned
  deterministically (same shape as G2, different trait and text), not merely
  kept green.
- **Unit sketches** (behaviour-level, in the paper doc): `instantiation_symbol`
  distinguishes same-rendered-name/different-`StructId` groundings;
  lazily-minted bare-ctor provenance is never borrowed; the blanket duplicate
  check survives the fix untouched.

## Spec decisions this brief hands to the spec-writer (resolved in slice9-spec.md)

1. **V2 fix site** — ctor-candidate registration/application vs operand
   normalization feeding dispatch (first implementation task: pin why a's own
   ctor is absent from the single-candidate set in pb2). *Resolved:* R1,
   gated on a Phase-1 diagnosis.
2. **V3 fix shape** — *Resolved:* one mechanism, one fix site — widen
   `instantiation_symbol`'s fall-through arm (R2.1); no map-side change, no
   fallback (R2.2 withdrawn as both inert and out of bounds).
3. **D3 ruling** — G4's after-column. *Resolved as a procedure, not a fixed
   outcome:* R3, Phase 4 builds the fixture and determines which of two
   outcomes actually happens.
4. **D4 ruling** — *Resolved:* no new dispatch-time visibility/tier machinery
   (R4).
5. **Roadmap edit** — *Resolved:* R7, five edit targets (two roadmap
   sentences, one stale anchor at two sites, two falsified `slice5-spec.md`
   claims).

## Signals re-check at phase exit (CLAUDE.md growth structure)

`src/check/poly.rs` is ~21k lines; the split remains deferred (recorded
residual from S5, 3/5 signals, no clean cut). S9 is expected to touch the ctor
path in `terms.rs`/`declarations.rs` (V2) and `instantiation_symbol` in
`ast.rs` (V3, R2.1); `poly.rs` itself may end up untouched (V3's fix lives in
`ast.rs`, not the obligation-wiring code the brief originally suspected). Plus
`tests/phase7b_slice9.rs` (new). Re-run the split signals at phase exit
against the files as they then stand; do not preemptively split.
