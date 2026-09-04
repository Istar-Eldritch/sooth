# P7b.S9 brief — module-aware trait-impl matching (recon round, probes + paper complete)

Scope input for the S9 spec. Produced by a recon round against the clean tree
(worktree `p7b-s9`, HEAD `600bc1b`), then adjudicated by a probe round
(verbatim log and verdict: [slice9-probes](./slice9-probes.md)) and a
paper-test round (validated golden designs:
[slice9-paper-tests](./slice9-paper-tests.md)). Repo untouched throughout
(`git status --porcelain` shows only these three docs). Baseline at HEAD:
`cargo test --no-fail-fast` **82 binaries, 3150 tests, 0 failed**.

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
   source-determined). Mechanism: V3 below (bound-word monomorphization
   identity + span-keyed obligation wiring). The roadmap's `1 1` sentence
   needs correcting at slice exit.
3. **Real ambiguity, not a guess** — the probe/paper rounds found the
   *constructible* ambiguity shape (two modules each declaring a blanket
   `impl: Sized for 'T`) **already errors today** — declaration-time
   `duplicate impl:` check (`check_impl_decls`, `src/check/declarations.rs:544`,
   P7.S4 R7), module-blind by design because a bare `PolyType::Var` carries no
   header identity. Concrete-target dispatch-time ambiguity is structurally
   unconstructible (import cycles / placement rule / selective-import
   collision all fire first — probes P5b/P5c). The remaining genuinely open
   behaviour is the **third-module mono caller** (P5a/G4: silently prints `2`
   today); its post-fix expectation is a spec ruling (see D3).
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
  (`src/check/declarations.rs:4349`). Confirmed live by the probe traces
  (`pattern_id=Some((false, 0, 3))` vs `Some((false, 1, 4))`).
- **F4 — golden #10 (same shape, different payloads) passes.**
  `tests/phase7b_slice2.rs:649` prints `1` `2` with `i64`/`str` payloads
  through the shared bound word. Whatever breaks pb2 does not break
  per-operand dispatch when the two groundings' substitutions differ — the
  probe confirmed dispatch is correct per-operand in the `mk` variant (V3).
- **F5 — dispatch results are memoized span-keyed.** The mono member path
  records `span -> symbol` into `builtin_overloads` (`src/check/poly.rs:2396`;
  map at `:929/:1046/:1116`), and `resolve_user_bound`'s obligation routing is
  likewise keyed by the dispatching body span (`:8352` doc). The probe named
  the concrete map on the live path: `trait_calls: HashMap<Span, String>`
  (`trait_calls.insert(ob.span, symbol)`), one fixed span for every caller of
  a shared bound word.
- **F6 — true ambiguity already errors.** `select_most_specific`
  (`src/check/poly.rs:8073`) raises `ambiguity_error` on 2+ incomparable
  maxima; equal patterns are never strictly more specific
  (`:19466`). No silent first-match pick can come from the tie-break.
- **F7 — declaration-time duplicate-impl check is module-blind structural
  equality** (`check_impl_decls`, `src/check/declarations.rs:544`, P7.S4 R7):
  two blanket `for 'T` impls anywhere in one program collide (paper G3
  validates the error text); two `for Widget` impls in different modules do
  not, because `Generic` patterns carry `(idx, module)`.
- **F8 — S5's shipped goldens bound the blast radius.**
  `cross_module_same_shaped_ctor_dispatches_callers_own_impl`
  (`tests/phase7b_slice5.rs:90`, prints `15\n25`) pins the ctor tier policy;
  golden #10 pins distinct-subst dispatch. Both must pass unchanged.

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
- **V3 — H1/H5 LIVE (mk variant, nondeterministic): the bound word's
  monomorphization identity collapses distinct headers.** `nm` shows ONE
  `sooth_mono_sized__m2__t0_Widget_i64_` (keyed on the grounded type's
  *rendered name*, identical for a's and b's distinct `StructId`s) but TWO
  `size` specializations (`..._m3_...`, `..._m4_...`). Dispatch is *correct*
  per-operand (winners `impl_idx=0` for a's `StructId(2)`/`(gi=0,m3)` operand,
  `impl_idx=1` for b's `StructId(3)`/`(gi=1,m4)` — trace in slice9-probes.md),
  but both callers share the single compiled `sized` body, whose one internal
  `size` call is wired through `trait_calls: HashMap<Span, String>` at one
  fixed span — last-writer-wins across caller groundings, order randomized per
  compiler process (Rust `RandomState`). Hence `1 1`/`2 2` per compilation.
  Nondeterminism locus note: the race is in the *checker* (a compile-time
  HashMap), so a built binary reruns deterministically and rebuilds flip; the
  probe log's "10x clean binary" and the paper's "rebuild+run cycles" both
  observed the flip and disagree only on the rebuild discipline. Committed
  tests must never assert a ratio.
- **H5 is not a separate finding** — the "member-word identity collapse" and
  the "span-keyed memo" are the same mechanism seen from the specialization
  side and the wiring side.

## Machinery map (verified anchors, this tree, HEAD `600bc1b`)

- **Registry scan.** `find_bound_impl` (`src/check/poly.rs:8235`); candidate
  loop `:8262-8276`; `candidate_bounds_discharge` recursion `:8192`;
  `select_most_specific` `:8073`, `ambiguity_error` `:8099`, `specificity`
  `:9640`. Call sites: inline-splice path `:1722`, mono-member viability
  `:2239` (inside `resolve_mono_member_call`, `:2165`), where-clause discharge
  `:8192`, bound dispatch `:8408` (inside `resolve_user_bound`, `:8352`).
- **Pattern matcher.** `match_impl_target` `:8833` / `..._rec` `:8846`;
  `Generic` arm identity comparison (F1). Sound; likely untouched by the fix.
- **Memoization / wiring.** `builtin_overloads: HashMap<Span, String>`
  (`:929`, `:1046`, `:1116`; write `:2396`); `trait_calls: HashMap<Span,
  String>` with `trait_calls.insert(ob.span, symbol)` in `resolve_user_bound`
  (V3's live site); obligation routing keyed by the dispatching body span.
- **Monomorphization identity.** `sized`'s specialization key renders the
  grounded type's name (`sooth_mono_sized__m2__t0_Widget_i64_` — nm evidence);
  the rendered name strips the per-module mangle tag exactly like
  `Type::name()`/`generic_surface_name` do elsewhere (S5's F3 lineage). The
  minting-side twin `instantiation_symbol` (`src/ast.rs:2869`) already keys
  ctor images on `GenericId` (`:5368` unit-pinned) — the asymmetry between the
  two is the spec's design opening for V3's fix.
- **Impl record + placement + dedup.** `ImplDecl` (`src/ast.rs:2492`);
  placement rule `src/check/declarations.rs:575-588`; blanket-impl duplicate
  check `:544` (module-blind structural equality — keep as-is, G3 pin).
- **Target resolution.** `poly_generic_header` (`src/parser.rs:7101`),
  `bare_generic_owner` (`:7133`), `parse_impl_target_pattern` (`:4112`).
- **Instantiation identity.** `GenericTypes::struct_instantiation_of`
  (`src/ast.rs:1118`), `enum_instantiation_of` (`:1128`), `GenericId`
  (`:2521`), `mangle` (`src/resolve.rs:36`).
- **Whole-program env.** `poly_env` built once over the assembled program
  (`src/check.rs:684-698`); `overload_symbols` `$$N` suffixing (Slice 8a fix 1,
  `src/check.rs:697`).
- **Ctor application path (V2's fix neighbourhood).** The single-candidate
  overload arm (`src/check/terms.rs:915-928`) and the minted-candidate
  registration (`struct_generated_sigs`, `src/check/declarations.rs:1815`);
  in pb2, a's bare ctor call sees exactly one candidate (b's minted
  `Widget[i64]`) — why a's own header's ctor is absent from that candidate set
  at that site is the first implementation-time pin for the V2 fix.
- **Goldens in play.** #10 `tests/phase7b_slice2.rs:649`; S5 tier-1
  `tests/phase7b_slice5.rs:90`; `pb2` fixture text verbatim in
  [slice5-probes.md](./slice5-probes.md) and preserved in
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
- **V3 (mono identity + wiring).** Two coupled halves: (a) the bound word's
  monomorphization key must distinguish same-rendered-name/different-`StructId`
  groundings (key on `(StructId, module)`-aware identity — the
  `instantiation_symbol`/`GenericId` precedent); (b) the obligation wiring
  (`trait_calls`, and the `builtin_overloads`/span-keyed shapes) must key per
  grounding, not per source span, so two groundings of one shared bound word
  record two independent dispatch decisions. The spec rules whether one
  mechanism covers both halves (e.g. grounding-keyed maps with a
  provenance-aware mono key) or they are two changes; both G1/G2 shapes must
  resolve correctly either way.
- **D3 (third-module mono caller, G4's after-column).** Recommended ruling:
  the V2 fix gives the third module its own 2-candidate ctor collision (it is
  neither a's nor b's module), which lands in S5's tier policy → compile-time
  ambiguity error, consistent with the roadmap's "real ambiguity error" intent
  and S5's tier vocabulary. The spec must rule this explicitly (option (a) vs
  a deterministic-pick alternative) before the golden's after-column is
  pinned.
- **D4 (visibility/tier dimension for the registry).** With identity-correct
  operands, distinct modules' headers never both match one operand and the
  constructible ambiguity surface is already covered by the declaration-time
  duplicate check (G3). Recommend: no new visibility filter or dispatch-time
  tier machinery in `find_bound_impl`; revisit only if the V2 fix's ruling
  (D3) requires candidate-set awareness at dispatch. The spec records the
  ruling either way.
- **Roadmap corrections at slice exit (Q5).** The S9 entry's mechanism
  sentence ("no module-identity check anywhere in `match_impl_target`...") is
  falsified: rewrite to the adjudicated two-defect story (operand provenance;
  monomorphization identity + span-keyed wiring), correct "silent `1 1`" to
  "nondeterministic `1 1`/`2 2`", and reword exit item 3's ambiguity clause to
  the evidence (declaration-time duplicate check covers the constructible
  shape; third-module caller ruled per D3).

## Paper tests (validated designs; full detail in slice9-paper-tests.md)

- **G1** `cross_module_same_shaped_impls_dispatch_each_callers_own_impl` —
  verbatim pb2; before `2\n2` (deterministic, 3 cycles), after `1\n2`.
- **G2** `..._via_named_instantiation_...` — mk variant; before
  nondeterministic `1 1`/`2 2` (10 cycles; never assert a ratio in code), after
  `1\n2`. The flakiness itself is the pre-fix evidence.
- **G2r** `..._eager_minter_wins_regardless_of_caller` — provenance mirror
  (a eager / b bare); before deterministic `1\n1` (7 cycles) — the cleanest
  isolated V2 witness; after `1\n2`. Recommended primary regression pin.
- **G3** `duplicate_blanket_impl_across_modules_is_a_declared_error` — pins
  the existing declaration-time duplicate error text
  (`error: duplicate`impl:` for `'T`...`); regression pin, not new work.
- **G4** `third_module_mono_caller_sees_the_wrong_impl_silently` — before:
  silent `2` (3 cycles); after-column = D3's ruling (recommended: located
  ambiguity error at construction).
- **G5** regression pins: #10 must stay `1\n2`; S5 tier-1 must stay `15\n25`.
- **Unit sketches** (behaviour-level, in the paper doc): mono identity
  distinguishes same-rendered-name/different-`StructId` groundings; obligation
  routing keyed per grounding; lazily-minted bare-ctor provenance is never
  borrowed; the blanket duplicate check survives the fix untouched.

## Spec decisions this brief hands to the spec-writer

1. **V2 fix site** — ctor-candidate registration/application vs operand
   normalization feeding dispatch (first implementation task: pin why a's own
   ctor is absent from the single-candidate set in pb2).
2. **V3 fix shape** — mono-key widening, grounding-keyed obligation maps, or
   both; one mechanism or two.
3. **D3 ruling** — G4's after-column (recommended: construction-time ambiguity
   error via S5's tier policy).
4. **D4 ruling** — no new dispatch-time visibility/tier machinery
   (recommended) vs adding one.
5. **Roadmap edit** — the corrections listed above, applied at slice exit.

## Signals re-check at phase exit (CLAUDE.md growth structure)

`src/check/poly.rs` is ~21k lines; the split remains deferred (recorded
residual from S5, 3/5 signals, no clean cut). S9 is expected to touch
`poly.rs` (obligation wiring / mono identity), the ctor path in
`terms.rs`/`declarations.rs`, possibly `ast.rs` (identity helpers), and
`tests/phase7b_slice9.rs` (new). Re-run the split signals at phase exit
against the files as they then stand; do not preemptively split.
