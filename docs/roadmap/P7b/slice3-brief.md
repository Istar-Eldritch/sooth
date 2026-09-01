# P7b.S3 brief — inline + HKT bounds, the zero-cost splice (recon round 260901)

Scope input for the S3 spec. Produced by a recon round against the clean tree (worktree
`p7b-s3`, HEAD `403618f`, P7b.S2 landed): a read-only extension-point map, nine live
compile/run probes with non-inline twins plus four reverted mutation experiments under
`/tmp/p7bs3-probes/` (verbatim log: [slice3-probes.md](./slice3-probes.md)). Repo
untouched throughout (`git status --porcelain` empty after every mutation restore);
probe fixtures are disposable. Baseline `cargo test --no-fail-fast` at HEAD is green
(3044 passed), so every rejection below is a missing S3 surface, not baseline breakage.
Diagnostic texts below are drafts — they freeze at implementation, pinned by the
goldens (S12 precedent).

S3's exit criteria (from the [phase doc](../P7b-higher-kinded-types.md)): `Functor.map`
called through a bound on an inline word splices to the same IR as a hand-written
inline `map` would produce; no call frame; no runtime dispatch.

## What the round established

The headline is two-part, and the second part is the one that sets the slice's shape.

**First: the standing MEMORY note is stale.** "An `inline` trait member is checker-ready
after S3o but panics at lowering with `checked resolved call exists`" was true when it
was written and is not true on this tree. p1 — an ordinary `Sized` trait, a *concrete*
impl target, `size inline` — compiles, runs, prints `7`, and p7's symbol check confirms
the member was genuinely **spliced** (no `size;Sized;0;Boxi` symbol minted, against
`core`'s non-inline `Show` impls which do mint `show;Show;3;Bool`). The combinator
splice path the note prescribes already exists at `src/ir/func_builder/calls.rs:170-228`,
and it is more careful than the note's prescription: the uid comes from
`member_uid_seeds` first and falls back to `splice_uid_stack.last()` only on a lookup
miss (the documented `P7.S8 R1` uid rule). **S3 is not a lowering slice.** Any spec that
budgets its weight there is budgeting for work that landed.

**Second: the real blocker is a chain of three checker gates, and they only appear once
the impl target is *generic*.** That condition is not an edge case for S3 — S2's
bare-ctor desugar turns every `impl: Functor for Opt` into `for Opt['ctor0]`, so every
HKT member has a generic target by construction. The chain, in firing order:

1. **R3's grounding gate** (`check/poly.rs:566`) — "grounding a generic over its own
   type variable is not yet implemented". A declared top-level `PolyType::Generic` input
   slot is refused at the combinator's standalone check.
2. **The poly pre-pass skip** (`check.rs:837`) — a synthesized member word carries no
   `Bound::User` (S2-6 grounds the trait variable into the target), so as a combinator it
   takes the skip, is never recorded in `PolyCtx::recorded`, and `word_sig_of` misses at
   dispatch: "`impl: Functor for Opt` binds no word for member `map`".
3. **R1.5's per-splice gate** (`check/poly.rs:4151` elimination, `:5611` construction) —
   "a generic enum eliminated inside a combinator body is not yet supported: each splice
   would need its own resolution, and none is recorded".

Each was isolated by a mutation: m1 neutralizes (1) and p2/p3 then build and print
exactly their non-inline twins' output; m2's tagged build localized (2) to the
`word_sig_of` miss at `:7981`; m3 records member combinators and exposes (3). The
coupling between (2) and (3) was **predicted in the tree**: `check.rs:822-838` says the
skip is safe only because "that body shape is rejected first by the pre-existing
variable-bearing-application gate … **Revisit the skip if that restriction is lifted**".
m1 lifts exactly that restriction.

**The critical result is m4.** With all three gates lifted or stubbed, p2 and p4 build
and print **correct** values (`2`; `-1` / `3`, two impls dispatched from one `twice`
body). That looks like "the gates were merely conservative — delete them and ship". The
symbol evidence says otherwise:

```text
INLINE (m1+m3+m4) p4 : sooth_mono_map_Functor_0_Opt__T0__T1___m0__t0_i64…  (+ Res twin)
NON-INLINE control p0: sooth_mono_map_Functor_0_Opt__T0__T1___m0__t0_i64…  (+ Res twin)
$ objdump -d p4/main | grep -cE 'call.*map_Functor'  → 4
$ objdump -d p0/main | grep -cE 'call.*map_Functor'  → 4
```

Identical symbols, identical call counts. **The `inline` keyword produced no lowering
difference at all** — the member was monomorphized into a real function and called four
times, exactly as the non-inline control. So lifting the three gates yields a program
that runs correctly while delivering *none* of S3's exit criterion: there is still a call
frame and still a runtime function. Worse, it is a silent failure mode — green tests,
correct output, and the criterion unmet unless a golden asserts on symbols or call
counts. m4 also means R1.5's hazard is **untested**, not disproven: no splice occurred,
so the per-splice resolution problem the gate names was never exercised.

| # | Finding | Probes |
| --- | --- | --- |
| F1 | **The "inline member panics at lowering" note is stale.** A concrete-target `inline` member compiles, runs, and splices today. Mechanism: `src/ir/func_builder/calls.rs:170-228`, uid rule `P7.S8 R1`, `member_uid_seeds` first with `splice_uid_stack.last()` as fallback. | p1, p7 |
| F2 | **The `inline` keyword alone flips a working HKT program.** p2 (`map inline`) dies at R3's gate; p2b, byte-identical but for the dropped keyword, builds and prints `2`. Clean attribution: nothing about the HKT shape is at fault. | p2, p2b |
| F3 | **The blocker is not HKT-specific.** p3 — ordinary Star trait `Sized`, generic target `for Box['T]`, `inline` member, no application anywhere — dies with the *identical* R3 error; p3b prints `1`. S3's first gate is "`inline` member on a **generic** impl target"; HKT is one instance of it, and (via S2's bare-ctor desugar) always an instance. | p3, p3b |
| F4 | **Gate 2 is the pre-pass skip, and the tree predicted it.** Member words are combinators without a `Bound::User`, so `check.rs:837` skips recording them and `word_sig_of` (`check/poly.rs:181`) misses. m2's TAG narrowed the three identical-text call sites (`:7938`, `:7969`, `:7981`) to `:7981`. The in-tree comment at `check.rs:822-838` names the m1 lift as the trigger. | m2, m3 |
| F5 | **Gate 3 is R1.5, on both the elimination and construction sides** (`check/poly.rs:4151`, `:5611`). It fires as soon as member combinators are recorded, because the only writable `Functor` bodies eliminate a generic enum. | m3 |
| F6 | **The struct route to `Functor` is closed independently of S3.** p5 (`impl: Functor for Box`, struct payload, no enum elim) is rejected — "`Box>` is not permitted on a generic type" — and p5b, without `inline`, is rejected **identically**. Pre-existing, not S3's to fix, but load-bearing for scope: it means every writable `Functor` body is enum-eliminating, i.e. exactly the shape R1.5 fences. S3 cannot route around R1.5 by picking a different body idiom. | p5, p5b |
| F7 | **S4's module-identity gap is live.** p6's `impl: Functor for Option` over the real `core::option` gets "no `impl:` in this program dispatches on these operands". S3's goldens must use fixture twins, as S2's W3/W4 already do. | p6 |
| F8 | **Two positive controls define the target shape.** p7: a mono member word splices (no symbol). p8: `core::cmp`'s `: lt inline ['T: Ord]` (`lib/core/cmp.sth:145`) splices too — poly + `inline` + a user bound **can** splice, via the `poly.combinators` interception, which records no instantiation. That interception is the path S3's member dispatch has to reach; today member dispatch goes through `resolve_user_bound`/`impl_monos` and mints a monomorph instead. | p7, p8 |
| F9 | **m4 is a trap, not a green light.** All three gates lifted → correct output, identical symbols and call counts to the non-inline control. No splice happened; the exit criterion is unmet; R1.5's hazard is untested rather than disproven. Any S3 golden must assert on symbols or call counts, never on stdout alone. | m4, p0 |

Positive control p0 (S2's W4 shared-bound golden) re-passes on this tree.

## Machinery map (verified anchors, this tree)

All anchors re-derived on HEAD `403618f`.

- **Gate 1 — R3.** `check_poly_combinator_standalone` (`src/check/poly.rs:537`) seeds a
  stand-in `Subst` (every ty var at `Type::I64`, `:552`; len vars at `STANDALONE_LEN`),
  then refuses any top-level `PolyType::Generic { .. } | GenericVariant { .. }` input at
  `:566` — before any grounding is attempted, deliberately ("so it never depends on
  whether grounding would have succeeded"). m1 shows the stand-in grounding is
  sufficient for these body shapes once the refusal is dropped.
- **Gate 2 — the pre-pass skip.** `src/check.rs:837`,
  `if is_combinator(word) && !sig.bounds.iter().any(|(_, b)| matches!(b, Bound::User(_)))
  { continue; }`, under the comment block at `:822-838` that names this exact coupling
  and warns "Revisit the skip if that restriction is lifted." Widening it to
  `sig.bounds.is_empty()` is explicitly measured and rejected in that comment (40+ tests
  red, `lib/core.sth`'s Copy-bounded loop combinators). m3's discriminator was
  `word.name.contains(';')` — a probe expedient, not a proposal.
- **Gate 3 — R1.5.** Elimination side `src/check/poly.rs:4151`
  (`matches!(operative, Operative::Generic { .. }) && tctx.is_combinator_splice`, message
  at `:10637`); construction side `:5611` (`tctx.is_combinator_splice && matches!(result_pt,
  PolyType::Generic { .. })`). The in-code rationale is the representation limit:
  `splice_trait_calls` is keyed `(uid, span)`, and "widening the key is out of scope for
  this slice" — the B3 miscompile is what silence would reinstate.
- **The miss site.** `word_sig_of` (`src/check/poly.rs:181`) reads `PolyCtx::recorded`,
  the `WordObligations` table the pre-pass builds; the CtorImage candidate loop re-derives
  the member per candidate and reports through `unresolved_trait_obligation_error` at
  `:7938` / `:7969` / `:7981` (m2: `:7981`).
- **Lowering, already working.** `lower_resolved_word_call` (`src/ir/func_builder/calls.rs:229`)
  with the combinator-splice bracket documented at `:170-228`: push `member_uid_seeds`
  onto `splice_uid_stack`, *reset* `self.inline_uid` to that seed, raise
  `member_splice_depth` so the span-keyed `trait_calls` lookup stands aside (R1b), restore
  all three on the way out.
- **The precedent to reach.** `lib/core/cmp.sth:145` (`: lt inline ['T: Ord] ( 'T 'T -- Bool )`)
  splices via the `poly.combinators` interception in `check_term`'s `Call` arm, which runs
  before the `poly.env` one and records no instantiation. Member dispatch currently lands
  on the other side of that fork.

## Open questions and scope recommendation

**S3 is not a small lowering fix, and it is not "delete three gates" either.** The
lowering half already works (F1/p1, p7, p8). The slice is a three-gate checker unlock
**plus** the substantive part m4 exposed: making per-splice grounding actually reach the
combinator-interception path, so an HKT member with `inline` is spliced rather than
monomorphized. Until that happens, lifting the gates produces pre-S3 dispatch behavior
wearing an `inline` keyword — correct output, unmet criterion, and R1.5's hazard sitting
untested behind a deleted gate.

**Q1 (the core design question) — what replaces R1.5?** The gate's own message states
the requirement: "each splice would need its own resolution, and none is recorded". So
the spec must answer: what is the minimal per-splice `Subst` plumbing that lets the
generic enum elimination inside a member body resolve **per call site** instead of once
per word? The in-code note points at the shape of it — `splice_trait_calls` is keyed
`(uid, span)` and the key would have to widen (or a per-splice resolution record has to
be threaded alongside). This should be **probed and designed before R1.5 is deleted**,
per the paper-precheck convention; m4 deliberately does not license deletion, because it
never made a splice happen. Recommend the spec open with a mutation round on the
splice-record key specifically, using p2/p4 with a symbol assertion (not a stdout
assertion) as the measurement.

**Q2 — gate 2's replacement discriminator.** m3's `name.contains(';')` proves the
mechanism but is not shippable. The real question is whether member combinators should be
recorded by the pre-pass at all, or reach `word_sig_of` by a different route (they are
synthesized, and their obligations are known at desugar time). Note the in-tree warning:
the neighbouring widening (`sig.bounds.is_empty()`) is measured-red, so the discriminator
must stay narrow.

**Q3 — does S3 need S4 first?** No. p6 confirms S4's module-identity gap is live, so a
real `core::option` dogfood cannot work yet; but S2 set the precedent of running W3/W4
over fixture twins for exactly this reason, and S3's exit criterion (splice, no frame, no
runtime dispatch) is fully observable on twins via `nm`/`objdump`. **Recommendation: S3
proceeds on fixture twins, independent of S4's landing order**, and the phase doc's
real-`Option`/`Result`/`List` dogfood stays where it already sits — dependent on S4 or on
twin workarounds.

**Q4 — golden methodology, forced by F9.** Every S3 positive golden needs a symbol or
call-count assertion (`nm` for an absent member symbol, `objdump` call count against a
hand-written inline equivalent). A stdout-only golden passes under m4's non-splicing
build and would certify the slice done while the criterion is unmet. p7/p8 are the
assertion templates; p0 is the non-inline control to diff against.

**Q5 — scope fences to keep.** F6's struct-route rejection (`Box>` on a generic type) is
pre-existing and identical with or without `inline`: record it, do not fix it in S3.
Whether S3 also owes the *ordinary* Star-trait case a fix is a real scope question — p3
shows `inline` + generic target fails for non-HKT traits too, so the unlock is wider than
HKT and its goldens should include p3's shape as a non-HKT witness.
