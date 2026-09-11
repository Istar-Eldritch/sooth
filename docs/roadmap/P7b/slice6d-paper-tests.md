# P7b.S6d paper tests — validated golden designs (S6d proper)

Designed against the clean tree at HEAD `390b1b2` (worktree `soo-38`, branch
`soo-38`; suite re-verified this round: **3480 passed, 0 failed**). Companion
docs: [slice6d-brief](./slice6d-brief.md) (problem, DQ1–DQ5),
[slice6d-probes](./slice6d-probes.md) (rounds S6d-1..S6d-10; S6d-7..S6d-10 are
the evidence base),
[probes/s6d_baseline.md](../../../probes/s6d_baseline.md) (byte-exact
clean-tree behavior of every committed fixture),
[slice6d-prereq](./slice6d-prereq.md) (REQ-1..6, Rulings A–F, NFRs). Harness
conventions from `tests/phase7b_slice8.rs`: `single_file_hosted` auto-prepends
`import: intrinsics * ;` + `import: hosted::show | . | ;` (so a golden's
inline source must NOT re-import `hosted::show` — a duplicate collides in the
seen-map), `build_ok`/`build_run_keep`/`build_error_located` (the last asserts
non-zero exit, no `panic`, and — caller-side — an `error:` in stderr),
`thing_condition_expected` naming. Probe fixtures run with
`--manifest tests/fixtures/sooth.pkg`. Import spells for `core` consumers
(measured, `tests/phase7b_slice8.rs`): `import: core::list | List Nil Cons | ;`

+ `import: core::iterator | Step Done More Iterator | ;` for hand-written
dispatch drains; `core::iterator` deliberately does not export bare `next`.

Every "today" byte below was reproduced this round against the clean-tree
binary (`target/debug/sooth`, rebuilt at HEAD) — the round's runs are quoted
in the desk-check section and per golden. The "post-fix" column is the
expected golden; exact diagnostic bytes for messages that only exist on the
patched tree (the Ruling A sweep text, the member output ban, the placement
gate) are pinned at implementation time from the live binary per house
convention — their content is already captured verbatim in
[slice6d-probes](./slice6d-probes.md) rounds S6d-8.4(iii), S6d-9 and S6d-8.6.

## The design frame, and amendments from this round's desk-checks

The frame under test (orchestrator decisions from the probe evidence):
**(1)** shared `Slice[i64]` target only; the mutable `!Slice[i64]` target
deferred (Ruling A's enum-payload sweep fires first — S6d-9; deferral points
at SOO-45). **(2)** the fence lift is the two-half sentinel patch exactly as
spiked in S6d-8.1 (parser half: `SLICE_SENTINEL_IDX` + `slice_sentinel` +
`rewrite_slice_sentinel` at `src/parser.rs:855-964`, the slice branch at
`:4629-4691` inside `parse_impl_member_body`, preempting the `is_concrete`
branch; dispatch half: `src/check/poly/ground.rs:1319-1333`, the slice guard
arm in `resolve_mono_member_call` reading the already-grounded member word);
negative controls (i)/(ii)/(iii) are regression pins. **(3)** per-impl
`inline` spelling; `declares_inline` = the impl's spelling when present, else
the trait member's (currently inherited unconditionally at
`src/parser.rs:4449-4453`). **(4)** the Step-monomorph clobber fix (a
PRE-EXISTING S8b-class bug, clean-tree-reproducible), REQUIRED because the
placement gate forces the slice impl into `core/iterator.sth` and co-location
mints the instantiation for every importer. **(5)** borrow-join union +
`back_edge_outs` deriv forwarding, TOGETHER. **(6)** the library impl in
`lib/core/iterator.sth`, `next inline`, natural body once 5(a) lands;
for_each/fold untouched (SOO-60 territory).

Amendments the desk-checks force (each argued in full in the verdicts):

+ **A-amend 1 — there are TWO join sites, and the union must land in both.**
  The `if`/`branch` join (`check_branch_join`, `src/check/terms.rs:3607`, error
  at `:3613`) and the eliminator arm merge (`merge_arm_output_slot`,
  `src/check.rs:2718`, deriv match at `:2725-2736`) carry the *same* refusal rule.
  The frame says "the branch-join rule"; the spec must patch both, or a
  `Step?`-dispatch-shaped state merge (exactly the while drain's inner
  shape) still refuses.
+ **A-amend 2 — the union's condition is `owned_root: None` on the one-sided
  deriv**, the same predicate the back-edge guard uses for its accept-case
  (`src/check.rs:1690-1700`) and `carried_borrow` uses for its no-contention
  case (`src/check.rs:3164`). Rooted one-sided asymmetries and two-root
  disagreements stay refused (goldens G4/G5 pin the bytes).
+ **C-amend — 5(b) alone suffices for the while-drain fixture**; the
  together-landing is justified by the soundness argument (the mask must not
  become a silent channel), not by this fixture. Both halves alone make
  `probes/s6d_h_while_drain.sth` build at the traced joins; the spec must
  still land them together (see verdict C).
+ **Mechanism fact (corrected — naming does NOT always mint a rootless
  reborrow):** the name-read push's reborrow arm (`src/check/terms.rs:248`,
  `Provenance::reborrow`) *inherits* the held deriv's root when one exists
  (`src/check/engine.rs:418`); it mints rootless only when the binding
  carries no held deriv to begin with. A parameter-seeded slice slot is
  `Slot::computed` (deriv-free, `src/check/word_entry.rs:219-221`), so
  *that* naming is rootless — but a local rooted in a frame place preserves
  the root through the reborrow (see `s6d_j`/G4, which names `va` and is
  refused as "a borrow of `a`" — a rooted result). Rootless derivs are
  therefore *common* in slice code via parameter-rooted remainders (this is
  why S6d-8.2's prescribed body fails and why the while drain's inner
  `Step?` merge passes today — both its arms' states pass through named
  locals). The union rule will fire often; G4/G5 are the guardrails.
+ **D-amend — per-impl inline on the MONO member is confirmed safe at mono
  call sites** (the call site is a sig-check, not a body re-walk), with one
  documented caveat: a member call *inside a poly combinator splice* routes
  onto `inline_combinator` (`src/check/poly/ground.rs:608-620`, S3-1.c) and
  IS re-walked there, where the natural body's bare `Done` would hit the S11
  strict-grounding wall (`consumer_expected_type`'s spliced-body half
  declines for a mono member — `combinator_sig` is `None`,
  `src/check/combinators.rs:549-556`). No golden in this set calls `next`
  from inside a quotation; the times-hosted drain is closed regardless
  (S6d-10.4). Noted for the spec's non-goals.

## Desk-check verdicts

### A — the join-union taxonomy

**Verdict:** confirmed with amendments. (i) unioning a ROOTLESS deriv is
sound; (ii) the ROOTED one-sided asymmetry must stay refused and the
condition `owned_root.is_none()` is clean; (iii) the (Some, Some)
different-root case stays refused exactly as today.

**Method:** read the rule and every scan it feeds; pinned all three shapes
empirically on the clean tree with new fixtures (no impl needed — the join
rule is generic).

**Evidence.** The rule today (`src/check/terms.rs:3607-3620`):
`(None, None) => None`; `(Some(a), Some(b))` with equal `suspension()` =>
`Some(a)`; everything else → `borrow_join_disagreement_error` (`:3797`). The
eliminator twin: `src/check.rs:2725-2736`, same three arms, same error. The
scans a kept deriv feeds:

+ Exclusivity (`src/check/word_families.rs:252-256`): the predicate is
  `(d.owned_root.as_deref() == Some(rest) || d.place == rest) && (mutable ||
  d.mutable)`. A rootless **shared**-slice deriv (the only kind the protocol
  produces: `subslice`/`slice`/`&>` project the receiver's deriv,
  `word_families.rs:918/951/62`, and the naming mint is shared-and-rootless)
  matches neither arm effectively — `owned_root` is `None`, and
  `d.place == rest` requires mutability, which a shared view lacks. It
  conflicts with nothing. A rootless **mutable** deriv (a `!Slice` view of a
  parameter) still matches via `d.place` — and *should*: that is the
  exclusivity the PREREQ wants. Unioning keeps exactly the right conflicts.
+ The consume guard (`live_borrow_of`, gated on `is_linear` of the consumed
  type): a slice is `Copy`, so consuming the parameter root never fires; the
  rooted array case is carried by the rooted deriv, untouched by the union.
+ The back-edge guard (`src/check.rs:1690-1700`): rejects only
  `owned_root: Some(non-static local)`; `None` is the documented accept-case.
  A rootless deriv kept at the join is consistent with the loop story (5b).

Taxonomy (then-arm, else-arm → today → proposed):

| shape | today | proposed | pinned by |
| --- | --- | --- | --- |
| (None, None) | `None` | unchanged | every existing if golden |
| (Some, Some) equal suspension | `Some(a)` | unchanged | existing |
| (Some(rootless), None) / (None, Some(rootless)) | **refuse** | **union → Some(d)** | G3 (bytes today), G2 post-fix |
| (Some(rooted), None) / (None, Some(rooted)) | refuse | **stays refused** | G4 (bytes today = post-fix) |
| (Some, Some) different suspension | refuse | **stays refused** | G5 (bytes today = post-fix) |

(i) is sound because the kept deriv is an over-approximation that is *true*
of the arm that produced it and conflicts with nothing on the other path.
(ii): the rooted asymmetry is a genuine disagreement — in the Step-protocol
shape one arm stores a view of a frame local the other arm drops — and the
refusal is long-standing checker behavior (measured today, G4's bytes);
an unconditional union would move existing rejections and mask the
disagreement class. The condition is clean: `owned_root.is_none()` is
exactly the guard's own rootlessness test. (iii): picking one arm's root
launders the other (Ruling F / SOO-41 territory at the join).

**New-fixture captures (clean tree, this round):**

```text
s6d_q (rootless one-sided): error: borrow state disagrees at the branch join in `rootless-asym` (line 22)
  the first arm leaves no live borrow, the second arm leaves a borrow with no local root: both arms must agree on which place, if any, stays borrowed past the join
  note: declared ( Slice[i64] -- Step[i64 Slice[i64]] )
s6d_j (rooted one-sided):   error: borrow state disagrees at the branch join in `rooted-asym` (line 32)
  the first arm leaves no live borrow, the second arm leaves a borrow of `a`: both arms must agree on which place, if any, stays borrowed past the join
  note: declared ( -- Step[i64 Slice[i64]] )
s6d_k (two roots):          error: borrow state disagrees at the branch join in `main` (line 20)
  the first arm leaves a borrow of `b`, the second arm leaves a borrow of `a`: both arms must agree on which place, if any, stays borrowed past the join
  note: declared ( -- )
```

**Confidence:** verified (rule read + all three shapes reproduced today).

### B — back_edge_outs forwarding

**Verdict:** confirmed. Forwarding `deriv` alongside `surviving` changes
nothing for any program whose loop-carried state has no deriv, keeps the
deriv live into the next iteration for accepted rootless states, and leaves
the PREREQ's guard test pinning exactly what it pinned.

**Method:** read `back_edge_outs` (`src/check/terms.rs:3773-3790`: it copies
`surviving` at `:3784` and nothing else — `Slot::computed(ty)` seeds
`deriv: None`), its callers (the self-tail back-edge arm in
`src/check/terms.rs:820-827`), the guard (`src/check.rs:1670`), and both
existing tests (`terms.rs:4388` surviving-forward white-box;
`terms.rs:4417` the guard rejection).

**Evidence.** For deriv-free carried state the forward is `None → None`
(byte-identical behavior; every existing List/Range/while golden carries no
deriv — measured: the suite is 3480/0 today with the drop in place). For the
while drain, the carried input at the self-call is the state slot itself
(`s next`'s output, deriv rootless — see verdict C), so the forwarded deriv
is the true crossing deriv and the manufactured asymmetry at
`lib/core/combinators.sth:80` disappears. The guard test
(`back_edge_rejects_a_deriv_carrying_aggregate_argument`) fires at the CALL
site — `check_reference_across_back_edge` scans the self-call's *arguments*
before `back_edge_outs` is ever reached — and 5(b) does not touch the guard,
so a rooted deriv crossing still rejects first; the test's premise ("the
mask is unreachable only because the guard rejects first") is exactly what
S6d-10.3 proved now has a rootless-shaped hole, which 5(b) closes. The
surviving white-box test still passes (surviving forwarding is unchanged); a
new unit pins the deriv forward (Units).

**Confidence:** verified (code read; the forward site is one assignment).

### C — the while-drain prediction

**Verdict:** with 5(a)+5(b) in place, `probes/s6d_h_while_drain.sth` builds
and runs; stdout `6\n6\n6\n`, exit 0. Notably, 5(b) alone also suffices for
THIS fixture, and 5(a) alone would also make it build — the two must still
land together (below).

**Method:** stage-by-stage walk against the current tree's code, with the
naming-mint fact calibrating every deriv.

**Evidence.** The walk (line numbers = the fixture's):

1. `|s|` binds the slice parameter — the seeded slot is `Slot::computed`
   (`word_entry.rs:219-221`), so the binding is deriv-free.
2. `s next` (line 37): naming `s` mints the rootless reborrow (`terms.rs:248`).
   The member call at a MONO site is a **sig-check, not a splice**
   (`resolve_mono_member_call`'s mono branch, `ground.rs:1273-1442`: slot
   match, the span-keyed record at `:1407`, `push_dispatch_outputs`) — so
   the state slot's deriv is the forward of the operand's (site 4,
   `poly.rs:4630`): rootless. Strict grounding S11 amendment: not consulted
   here (no bare ctors at this level).
3. The annotated quotation `p` (lines 38-41): its tags are eliminator arms,
   so the standalone check is skipped for the arms
   (`terms.rs:1417-1442`, `EliminatorArmDest::Reached`) and they are checked
   at the `Step?` call site against the received scrutinee
   (`check.rs:2600-2603`, `..scrutinee` — provenance included). Done arm:
   names `s` → a fresh rootless mint → the empty view → `as-done`'s site-4
   forward → Step@rootless. More arm: names `r` → a fresh rootless mint →
   `r More` → Step@rootless. Equal suspension `(None, None)` → the inner
   merge passes. **This is measured, not just predicted**: the isolated
   merge shape (a probe word with both arms, never called) checks clean on
   the CLEAN tree up to the link step — and the derivative probes pin each
   half (the as-done arm alone vs a deriv-free arm refuses at an `if` join;
   a plain repack arm alone vs a deriv-free arm refuses at the eliminator
   merge). The variant-escape rule: both arms consume their shells.
4. `while` spliced into `drain` (it is `inline`,
   `lib/core/combinators.sth:79-80`): `p call` → (state, Bool); the internal
   `if`'s then-arm `~[ p while ]` is the self-tail back edge. The guard
   (`check.rs:1670`) scans the self-call's arguments: the state's deriv is
   rootless → **accept** (S6d-10.1's measured accept-case). Then
   `back_edge_outs` re-derives the arm's outputs: today `deriv` is dropped
   (`terms.rs:3773-3790` copies only `surviving`), so the then-arm's state
   arrives deriv-free while the else-arm `~[ ]` keeps the state's deriv →
   `borrow_join_disagreement_error` at combinators.sth:80 — the S6d-10.3
   capture, and the exact join 5(b) heals (both arms then carry the same
   rootless deriv; equal suspension → keep).
5. Post-join: the state exits `while` with its deriv; `drop` consumes it
   (a shared-slice container is `Copy`); `drain`'s declared outputs are `--`;
   `main` drops `buf` last with no live borrow. Nothing else fires.

**Why together, precisely:** with 5(a) alone this fixture also builds — the
join unions `(None, Some(d))` back to `Some(d)`, and because the two arms'
states are the *same slot* (the else arm leaves `p call`'s state verbatim),
the recovered deriv happens to be the true one. But the mask itself
(`back_edge_outs` dropping the crossing deriv) would remain, and any shape
where the join *cannot* recover the deriv from a sibling arm becomes a
silent launder instead of today's loud error — the exact failure mode
`back_edge_rejects_a_deriv_carrying_aggregate_argument`'s doc comment warns
about. With 5(b) alone, `next`'s natural body (no `as-done`) still refuses
at its own join (S6d-8.2). Together: no silent path, and the natural body
checks. **Confidence:** verified for the pre-fix behavior (clean-tree probes)
and the S6d-10.3 capture; the post-fix run is a prediction from the traced
mechanics (no patched tree was built this round).

### D — the inline-mono-member splice path

**Verdict:** the S6d-8.3 evidence plus this round's code read DO support the
frame's claim for mono call sites — with one caveat the goldens avoid.

**Method:** read the mono member call path end-to-end and the poly-splice
member path; ran nothing new (the fence blocks any live exercise today).

**Evidence.** At a MONO call site (`main`/`drain` bodies),
`resolve_mono_member_call`'s mono branch sig-checks the slots against the
grounded member sig, records the span-keyed symbol (`ground.rs:1407`), and
pushes dispatch outputs (`:1441`) — it never re-walks the member body. The
body (including the natural body's bare `Done`) is checked exactly ONCE, at
the member word's declaration (`check_word` → `check_terms_word`,
`word_entry.rs:181-240`), where the tail channel
(`consumer_expected_type`, `terms.rs:2649`, tail half at `:2722-2727`) grounds
the arm-tail `Done` against the member's own declared output — the same
channel the List impl's non-inline member uses today. The S6d-8.5 disease is
specific to the POLY inline member path (trait-level inline makes the
List/Range members route through the poly combinator machinery, where the
nullary ctor mis-resolves — the round's two failure shapes); a mono member
never enters it at mono call sites. S6d-8.3's end-to-end drain (mono member,
inline, real `Step?` dispatch and `More>` destructure at the call sites) ran
green on the patched tree — consistent. **Caveat:** a member call inside a
poly combinator splice routes onto `inline_combinator`
(`ground.rs:608-620`) and re-walks the body with the splice's context, where
`consumer_expected_type`'s spliced-body half declines for a mono member
(`combinator_sig` is `None`, `combinators.rs:549-556`) — a bare arm-tail
ctor would hit the S11 wall there. No golden calls `next` from inside a
quotation; `next`-inside-a-quotation shapes are separately closed for slices
(S6d-10.4's abstract-row wall). The evidence therefore supports the frame
**for the shapes this slice ships**.

**Confidence:** verified by code read; the declaration-check grounding is
corroborated by the measured `done-empty` behavior (a bare ctor grounds from
a single-output word's declared output at declaration, and refuses against a
caller whose tail output differs — the probes behind G2/G4).

### E — the clobber fix's blast radius

**Verdict:** no existing golden moves under a per-monomorph keying — no
existing fixture in the suite has two monomorphs of one generated enum in one
program (measured: no slice8 test imports both `core::list` and
`core::range`; the prereq suite mints `Step[i64 Slice[i64]]` alone). The
clobber is reachable only from NEW programs, which is why it is unpinned
today and why G7/G8 must exist. Canaries: the 6 named below plus the suite.

**Method:** enumerate the resolution paths for generated enum words; measure
the suite's exposure; reproduce the clobber byte-exact as a committed fixture.

**Evidence.** The paths a bare `Done`/`More`/`More>`/`Cons>` resolution
touches: (1) the module `env`'s bare-key entries (built at assemble, before
check-time mints); (2) `mint_fallback_candidates` (`terms.rs:2249-2289`) —
every unflushed check-time monomorph's generated sig matching the name, so a
second mint turns one candidate into two; (3) the fallback picker
`select_overload_fallback_sourced` (`src/check/builtins.rs:180-199`:
operand-filter, then a tier-1 same-module preference, then first-match) —
the likely reason the entry-file-minted `Step[i64 Slice[i64]]` hijacks a
List consumer's site (the repro's "expected" side is the Slice monomorph
against a List operand); (4) the S11 strict-grounding ladder
(`terms.rs:1000-1100`, `ground_bare_generic_ctor` at `:2843`); (5) the
span-keyed records — the `[only]` arm's
`splice_enum_words[(uid,span)]`/`builtin_overloads[span]`
(`terms.rs:1124-1157`) and the multi-candidate arm's `builtin_overloads[span]`
(`:1210`) — the S8b mechanism the frame points at; (6) lowering's bare-key
last-write-wins map (`src/ir/func_builder/calls.rs:480` reads the span-keyed
record first, then the bare-key map) — the checker never reads
`builtin_overloads` (measured: only `src/ir/` does). A per-monomorph keying
of the bare-name channel changes behavior exactly where two monomorphs of
one family are live in one program — the single-monomorph case resolves
identically under any keying. The repro, committed as `probes/s6d_m_clobber_touch.sth`
(a mere signature mention minting `Step[i64 Slice[i64]]` before a List-drain
consumer):

```text
error: type mismatch in `drain` (line 19)
  `More>` expected `Step[i64 Slice[i64]].More`, found `Step[i64 List[i64]].More`
  note: declared ( List[i64] -- )
```

**Confidence:** verified (byte-exact repro; suite exposure measured; the
tier-1 hijack is a code-shape reading — the mechanism design stays with the
implementer per the frame).

## The golden set

G-numbers are the spec's pin list. "today" = the clean tree at HEAD
`390b1b2` (all re-run this round); "post-fix" = the expected golden once the
frame lands. Fixtures marked *frozen* are committed byte-stable probe
fixtures reused as-is; fixtures marked *new* were created this round under
`probes/s6d_<letter>_<slug>.sth` and are byte-frozen from today.

### G1 — `slice_impl_member_per_impl_inline_builds_and_runs`

Fixture: `probes/s6d_n_perimpl_inline_member.sth` (*new*). Trait member
NON-inline (the HEAD spelling everywhere in `lib/`), impl member spelled
`: next inline`. Today: the S2-6 fence at the member —

```text
error: trait member `next` of `Iterator` (line 26, col 5) applies the trait variable `'It`, but the impl target `Slice[i64]` is concrete
  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead
```

Post-fix: `build_run_keep`, stdout `3\n4\n` (one `next` step over a 5-element
view: element, remainder length). Pins frame items 1+2+3 (sentinel both
halves, per-impl inline desugar, the body spelled with the `as-done` Done arm
so this golden isolates the inline-spelling item from 5(a)).

### G2 — `natural_next_body_checks_with_the_rootless_join_union`

Fixture: `probes/s6d_p_natural_next_join.sth` (*new*). The protocol's own
straight-line Done arm (`~[ drop Done ]`, no helper). Today: the fence at the
member (line 25, col 5 — same shape as G1). On the sentinel-patched tree
without 5(a) this is S6d-8.2's refusal (the G3 message shape, at the member's
own line). Post-fix: `build_run_keep`, stdout `3\n4\n`. Pins frame item 5(a)
end-to-end plus the declaration-check grounding channel (verdict D): the
member body checks once at declaration (the tail channel grounds the arm-tail
`Done` against the member's declared output) and the mono call sites
sig-check only.

### G3 — `rootless_join_asymmetry_refused_today_carried_by_the_union`

Fixture: `probes/s6d_q_join_rootless_asymmetry.sth` (*new*). The join rule
pinned WITHOUT any impl, so today's bytes are the baseline: one `if`-arm
packs the word's seeded slice parameter directly (deriv-free — parameters
are `Slot::computed`), the other binds then names it (the naming mint —
rootless). Today (captured in verdict A): the refusal with S6d-8.2's exact
wording, `build_error_located`. Post-fix: `build_ok` (the union carries the
rootless deriv). Pins frame item 5(a)'s rule itself, at `check_branch_join`.

### G4 — `rooted_join_asymmetry_stays_refused_byte_identically`

Fixture: `probes/s6d_j_join_rooted_asymmetry.sth` (*new*). A `Step?` dispatch
inside a never-called inline word with a declared `Step` output (so the arms'
bare ctors ground via the tail channel): the Done arm rebuilds the state
deriv-free (`done-empty`), the More arm repacks the remainder of a view of
the frame local `a` (rooted deriv). Today (captured in verdict A): the
refusal naming `a`. Post-fix: **byte-identical** — the union is conditioned
on `owned_root: None` (desk-check A.ii). Also pins the eliminator merge site
(`merge_arm_output_slot`) — proof the second join site needs the same
treatment and keeps its rooted refusal.

### G5 — `two_root_join_asymmetry_stays_refused_byte_identically`

Fixture: `probes/s6d_k_join_two_roots.sth` (*new*). Two views of two
different frame locals, each arm packing its own. Today (captured in verdict
A): "the first arm leaves a borrow of `b`, the second arm leaves a borrow of
`a`". Post-fix: byte-identical (desk-check A.iii; Ruling F / SOO-41
territory at the join).

### G6 — `while_threaded_slice_drain_runs_once_back_edge_outs_forward_deriv`

Fixture: `probes/s6d_h_while_drain.sth` (*frozen*). Today: the fence at the
member (line 18, col 5). On the sentinel-patched tree WITHOUT 5(b): the
while-internal join refusal at `lib/core/combinators.sth:80` (S6d-10.3's
capture, quoted in the probes doc). Post-fix: `build_run_keep`, stdout
`6\n6\n6\n`, exit 0 (3 elements of 6, one print per `More`). Pins frame item
5(b) (verdict C is the golden's justification trail). *Note:* 5(b) alone
suffices for this fixture; the together-landing is the soundness argument,
not a fixture dependency (verdict C).

### G7 — `clobber_touch_mint_leaves_list_drain_resolutions_untouched`

Fixture: `probes/s6d_m_clobber_touch.sth` (*new*). The S6d-10.5 repro as a
committed fixture: a bare signature mention (`: touch inline ( Step[i64
Slice[i64]] -- ) drop ;`) before a List-drain consumer. Today (captured in
verdict E, byte-exact — this is the baseline pin):

```text
error: type mismatch in `drain` (line 19)
  `More>` expected `Step[i64 Slice[i64]].More`, found `Step[i64 List[i64]].More`
  note: declared ( List[i64] -- )
```

Post-fix: `build_run_keep`, stdout `1\n2\n3\n`. Pins frame item 4 (the
clobber fix).

### G8 — `list_and_slice_impls_drain_through_one_imported_protocol` (exit criterion)

Fixture: `probes/s6d_o_lib_slice_end_to_end.sth` (*new*). One program, both
lib impls, bare `next` dispatching to each: a List drain (1 2 3) and a slice
drain (3 3 3), two `Step` monomorphs coexisting. Today: the no-dispatch error
at the slice drain (the lib ships no slice impl yet) —

```text
error: `next` in `drain-slice` (line 21, col 3) is a trait member of Iterator, but no `impl:` in this program dispatches on these operands
  the operand types here are `Slice[i64]`; declare an impl of one of those traits for the operand's type, or import a word that claims this name
```

Post-fix: `build_run_keep`, stdout `1\n2\n3\n3\n3\n3\n`, exit 0. Pins frame
items 4+6 together with 1+2+3: the slice impl shipped in
`lib/core/iterator.sth` (per-impl inline), the clobber fix proven by
coexistence, the sentinel lift exercised through the exported surface. This
is the slice's exit criterion. For the *test-harness* copy: do NOT add
`import: hosted::show` (the harness prepends it); the probe fixture keeps
its own import for standalone runs.

### G9 — `mutable_slice_impl_first_firing_is_the_enum_payload_sweep`

Fixture: `probes/s6d_g_mut_impl.sth` (*frozen*), with the always-true twin
`probes/s6d_b3_mut_payload.sth` (*frozen*). Today: the fence with the mutable
target display (G-file line 17, col 5, `!Slice[i64]`); the twin (no impl)
already fires the Ruling A sweep on the clean tree, byte-exact (baseline
doc). Post-fix: the impl fixture transitions to `build_error_located`
asserting the Ruling A enum-payload sweep — first firing, at the `Step`
declaration's span (S6d-9's verbatim capture:

```text
error: a reference cannot be stored: payload field 1 of variant `More[i64 !Slice[i64]]` of type `Step[i64 !Slice[i64]]` has type `!Slice[i64]` (line 7, col 3)
  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow
```

); the twin stays byte-identical today/post. Pins frame item 1 (the mutable
deferral) and Ruling A.

### G10 — `noninline_slice_member_output_ban_holds_post_fix`

Fixture: `probes/s6d_next_noninline.sth` (*frozen*; trait member AND impl
member both non-inline). Today: the fence (line 37, col 3). Post-fix:
`build_error_located` asserting the member word's own output ban (S6d-8.4(iii)'s
verbatim capture:

```text
error: a reference cannot be stored: `next` (member of trait `Iterator` for `Slice[i64]`) declares the output `Step[i64 Slice[i64]]`
  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead
```

). Pins negative control (iii) and frame item 3's default rule (no impl
keyword → the trait member's `declares_inline`, which is false at HEAD).

### G11 — `fence_byte_stability_for_a_non_slice_concrete_target`

Fixture: `probes/s6d_l_fence_nonslice_target.sth` (*new*). A probe-local
one-field struct target (`Unit`) with the same App-headed member. Today
(captured this round):

```text
error: trait member `next` of `Iterator` (line 22, col 5) applies the trait variable `'It`, but the impl target `Unit` is concrete
  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead
```

Post-fix: byte-identical — the sentinel branch claims
`Concrete(Type::Slice(..))` only. Pins negative control (i)'s permanent
shadow (the parser half is load-bearing exactly for slice targets).

### G12 — negative control (ii): the dispatch half

Not expressible as a `.sth` golden post-fix (the panic it guards is
unreachable once the fix lands). Two pins carry it: (a) an
implementation-time revert control — with the ground.rs arm reverted, the
consumer's `next` call site panics at `src/ast.rs:2271` (S6d-8.4(ii)'s
capture; the harness's no-`panic` discipline in `build_error_located` is the
standing guard); (b) G8's call-site resolution — the slice member's effect at
a mono call site comes from the already-grounded word, which is the arm's
whole job. Recorded here so the negative control is not lost.

### G13 — `trait_level_inline_spelling_still_builds_for_a_probe_local_slice_impl`

Fixture: `probes/s6d_a_fence_baseline.sth` (*frozen*; trait-level `next
inline`, `as-done` body). Today: the fence (line 37, col 3). Post-fix:
`build_run_keep`, stdout `3\n4\n` — the trait-level spelling stays legal
where it always was (a probe-local trait has no List/Range impls to break);
the per-impl keyword is *additive*. Guards the desugar against accidentally
forbidding the old spelling. (The 6-golden S6d-8.5 regression is a
*library*-impl phenomenon — the canaries in G22 pin it.)

### G14 — `prereq_admissions_stay_byte_green`

Fixture: `probes/s6d_b_step_shared_inline.sth` (*frozen*). Today AND
post-fix: `build_run_keep`, stdout `41\n5\n` (verified today: build exit 0,
run exit 0). No frame item touches the PREREQ admissions; this golden fails
if any of them did.

### G15 — `monomorphic_consumer_drains_two_views_end_to_end`

Fixture: `probes/s6d_c_impl_consumer.sth` (*frozen*). Today: the fence (line
27, col 5). Post-fix: `build_run_keep`, stdout `3\n3\n3\n3\n3\n9\n` (the
second drain proves the `Done` path runs at runtime). Pins the S6d-8.3
end-to-end (the `resolve_mono_member_call` slice arm exercised for real) —
the evidence G12(b) leans on.

### G16 — `selftail_drain_passes_the_back_edge_guard_with_a_parameter_rooted_remainder`

Fixture: `probes/s6d_d_selftail.sth` (*frozen*). Today: the fence (line 17,
col 5). Post-fix: `build_run_keep`, stdout `3\n3\n3\n3\n3\n`. Pins
`check_reference_across_back_edge`'s accept-case (S6d-10.1: the prediction
was falsified — parameter-rooted remainders have no `owned_root` and cross
freely).

### G17 — `selftail_inline_drain_still_rejected_at_the_back_edge_guard`

Fixture: `probes/s6d_d2_selftail_inline.sth` (*frozen*). Today: the fence
(line 17). Post-fix: `build_error_located` asserting (S6d-10.1's sharper
twin, verbatim from the probes doc):

```text
error: a reference to a local cannot cross a loop in `main` (line 37)
  a reference derived from `buf`, a local of this frame, crosses the self-tail-call back-edge to `drain`: that local's storage does not survive to the next iteration
  note: declared ( -- )
```

Pins the guard's reject-case (SOO-42's gate, exactly at the root-visibility
boundary).

### G18 — `nontail_drain_runs_with_one_frame_per_element`

Fixture: `probes/s6d_e_nontail_drain.sth` (*frozen*). Today: the fence (line
17). Post-fix: `build_run_keep`, stdout `4\n4\n4\n4\n`. Pins the admissible
non-tail consumer shape (S6d-10.2; a bare `Slice[i64]` input to a non-inline
word stays admissible).

### G19 — `slice_impl_over_an_imported_trait_still_must_live_in_the_declaring_module`

Fixture: `probes/s6d_f0_libiter_gate.sth` (*frozen*). Today: the fence (line
9, col 5). Post-fix: `build_error_located` asserting the placement gate
(S6d-8.6 gate 1's verbatim capture:

```text
error: `impl: Iterator for Slice[i64]` at line 8, col 1 must live in the module declaring `Iterator` (`Slice[i64]` declares no module of its own)
```

). Pins frame item 6's placement premise: the co-declaration arm is
structurally unavailable for slices, so `core/iterator.sth` is the only home.

### G20 — `bound_generic_consumers_stay_closed_at_the_slot_unification`

Fixtures: `probes/s6d_f_for_each_slice.sth`, `probes/s6d_f2_fold_slice.sth`
(*frozen*). Today: the fence (line 17, col 5). Post-fix:
`build_error_located` asserting the bound-slot unification rejections
(S6d-8.6 gate 3's verbatim captures: `` `for_each` expected `'It['T]`,
found `Slice[i64]` `` / the `fold` twin). Pins the SOO-60 boundary:
for_each/fold are NOT touched by this slice — the `'It['T]` slot cannot
unify a bare slice (no ctor head), and the golden guards against accidentally
admitting it.

### G21 — `times_hosted_slice_drain_stays_closed_at_the_abstract_row`

Fixture: `probes/s6d_i_times_drain.sth` (*frozen*). Today: the fence (line
18, col 5). Post-fix: `build_error_located` asserting the abstract-row
no-dispatch error (S6d-10.4's capture — `mono_member_no_dispatch_error` with
the EMPTY operand list `the operand types here are ``` ` ```). The wall is
the times quotation's standalone check against the abstract row, not the
member spelling — unchanged by the frame.

### G22 — `existing_suite_green` and the canaries

`cargo test` at the fix commit: 3480 + this slice's new tests passed, 0
failed (3480/0 re-verified at HEAD this round). Any red existing golden is a
real signal. Named canaries (the S6d-8.5 six — the trait-level-inline
regression face; they must stay green with List/Range spellings unchanged):
`consuming_loop_over_range_is_one_frame_with_a_back_edge_and_next_is_a_real_frame`,
`core_iterator_module_drains_a_list_through_its_cross_module_impl`,
`fold_sums_a_list_through_the_iterator_bound`,
`for_each_and_fold_drain_a_range_through_the_iterator_bound`,
`for_each_drains_a_list_through_the_iterator_bound`,
`range_next_dispatches_at_a_mono_call_site` — all in
`tests/phase7b_slice8.rs`.

## Units (beside each changed site, `thing_condition_expected`)

+ `src/parser.rs` (sentinel helpers, beside `rewrite_slice_sentinel`):
  `rewrite_slice_sentinel_erases_every_sentinel_occurrence` (recursion over
  Generic/GenericVariant/Array/Ref/OwnedCell/Quotation positions);
  `slice_sentinel_is_recognizable_and_never_a_real_registry_index` (the
  `u32::MAX` marker, the invariant-exception comment's contract).
+ `src/parser.rs` (`parse_impl_member_body` slice branch):
  `slice_impl_member_grounds_app_headed_row_against_concrete_slice_target`;
  `slice_impl_member_non_var_free_grounding_reaches_the_standing_fences`.
+ `src/parser.rs` (per-impl inline desugar):
  `impl_member_inline_keyword_sets_the_member_words_declares_inline`;
  `impl_member_without_inline_keyword_inherits_the_trait_member_flag`.
+ `src/check/poly/ground.rs` (the slice arm in `resolve_mono_member_call`):
  `slice_mono_member_call_reads_the_already_grounded_word_effect`;
  `non_slice_concrete_member_call_still_rederives_via_ground_member_type`
  (byte-stability of the untouched path — G11's unit-level twin).
+ The join rule — BOTH sites (desk-check A-amend 1), taxonomy cases from
  verdict A: `branch_join_unions_a_rootless_one_sided_asymmetry`,
  `branch_join_still_refuses_a_rooted_one_sided_asymmetry`,
  `branch_join_still_refuses_two_different_roots` (`src/check/terms.rs`
  tests); `eliminator_arm_merge_unions_a_rootless_one_sided_asymmetry`,
  `eliminator_arm_merge_still_refuses_a_rooted_asymmetry`
  (`src/check.rs` tests, beside `merge_arm_output_slot`).
+ `back_edge_outs` (`src/check/terms.rs` tests):
  `back_edge_outs_forwards_deriv_along_the_index_map` (beside the existing
  `back_edge_outs_forwards_surviving_set_along_index_map`, which stays).
+ The clobber fix site (mechanism per the implementer; the S8b span-keyed
  channel is the named surface):
  `variant_word_resolution_survives_a_second_monomorph_of_the_same_enum` —
  two `Step` monomorphs, each site resolving to its own. Integration pins:
  G7/G8.

Target ~14 units.

## Risks and fallbacks (per frame item)

+ **F1 (shared target only).** No risk: the mutable target's rejection is
  Ruling A's sweep, first-firing (G9). If the sweep's span/wording shifts,
  G9's twin (s6d_b3) catches it. Deferral pointer: SOO-45.
+ **F2 (the two-half sentinel patch).** Negative controls (i)/(ii)/(iii) are
  the risk cover: (i) G11 (byte-stability for non-slice targets), (ii) G12's
  revert control, (iii) G10. If the sentinel's invariant exception is ruled
  unacceptable, the fallback is the `PolyType::SliceApp` variant — a bigger,
  unverified diff (S6d-2's ledger); not a spelling fallback.
+ **F3 (per-impl inline).** Hard requirement, no spelling fallback: the
  member MUST be inline (the output ban, G10), and trait-level inline is NOT
  an acceptable fallback — it breaks 6 List/Range goldens via the inline
  poly-member mis-resolution of nullary variant ctors (S6d-8.5; the
  canaries are the pin). If the desugar stalls, the slice stalls; escalate.
  The `as-done` fallback covers the BODY shape only (see F5).
+ **F4 (the clobber fix).** The gate leaves no other home for the impl
  (`Slice` declares no module — G19), so if the fix stalls, the lib
  placement stalls and the slice ships nothing user-visible. The fix surface
  is the S8b span-keyed mechanism; the risk is the OTHER paths (the
  fallback picker's tier-1 module preference, verdict E path 3) — the
  implementer should key the resolution, not widen one arm. Integration
  pins: G7, G8.
+ **F5 (join union + back_edge_outs, together).** Probe-proven fallback if
  either stalls: the `as-done` poly-helper spelling
  (`probes/s6d_a_fence_baseline.sth`'s impl body) with join/back_edge
  untouched — it checks and runs end-to-end on the sentinel-patched tree
  (S6d-8.2/8.3), at the cost of the over-conservative Done-path deriv. If
  5(a) stalls: G2/G3's post-fix expectations change to the as-done body
  (s6d_a's), and G4/G5 keep the join bytes frozen. If 5(b) stalls: G6's
  post-fix expectation is withdrawn (the while drain stays closed — it is
  not the exit criterion; G8 is). Trait-level inline is NOT a fallback for
  any of this (see F3).
+ **F6 (the lib impl).** for_each/fold stay out (G20). The natural body
  (G2) depends on 5(a) AND on verdict D's mono-caller finding; if the
  S11-splice caveat ever matters (a future `next`-inside-a-quotation
  shape), the as-done body is the fallback spelling there too.

## Commands run this round (evidence)

+ `cargo build` (clean tree at HEAD `390b1b2`) — fresh binary.
+ `cargo test` — 3480 passed, 0 failed.
+ `sooth build probes/<name>.sth --manifest tests/fixtures/sooth.pkg` for
  every committed s6d fixture (all 13) — bytes match
  `probes/s6d_baseline.md`; `s6d_b_step_shared_inline.sth` additionally run:
  stdout `41\n5\n`, exit 0.
+ The seven new fixtures (j,k,l,m,n,o,p,q) built and captured (bytes above;
  o/p/n/l reproduce today's fence/dispatch shapes, j/k/q pin the join
  taxonomy, m pins the clobber).
+ Five /tmp isolation probes (derivation of the naming-mint model; deleted).
