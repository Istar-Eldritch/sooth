# Phase 4 Slice 6g spec: combinator splices learn 6f's granting rule

Derives from [`docs/phase4-slice6g-brief.md`](./phase4-slice6g-brief.md). The brief's recon
was built and run against `e87bcae` (post-10a), not read; this spec takes it as ground truth.
All `file:line` anchors below are the brief's own, re-verified against `e87bcae`; the three
anchors this spec adds that the brief does not give (`lib/arrays.sth`'s stale paragraph,
`tests/phase4_combinators.rs`'s import helper, and the `filter_while` corpus row) are cited
where they appear.

**Checker-acceptance-plus-one-diagnostic.** Edits are confined to `src/check.rs`'s
liveness/granting layer and its two `Bind` arms, plus tests, one stale library comment, and
`ROADMAP.md`. The only new *behaviour* outside liveness is D5's bind-collision diagnostic. No
`Instr`/`Terminator`, no `Type`/`IrType`, no lowering, no `qbe.rs`. A program that compiles
today and still compiles after this slice lowers byte-for-byte, and this spec keeps that claim
true by declining to edit any corpus-pinned example (see Q-corpus below).

## The bug in three programs

Recon 4's three programs are the whole slice. Every combinator body is spliced at its call
site by `inline_combinator` (`:7359`), whose body-check (`:7506`) calls the plain
`check_terms` (`:8318`) — the root entry point 6f's own doc comment reserves for "a word body,
a REPL line, a `case` clause: nothing is ancestor to those" — instead of `check_terms_relaxed`
(`:8357`) with a `releasable_into`-computed grant. `call` (`:8595`), `times` (`:8683`) and an
`if` arm (`:9125`) all do the relaxed thing; the splice is the one nested-invocation shape on
the wrong side of the fork. Every array is `Copy`, so naming one never enters move-tracking
(D2), and `Liveness::dead` (`:1319`) is the only guard left — a guard the splice never grants
into.

The programs (verbatim from the brief; the `import:` line for `c::` names is harness, the
program body is unaltered):

```sooth
\ P-times-accept: the times doorway grants `a` into the loop body, so the aliased
\ mutable borrow of `arr` is allowed. Accepts today, must keep accepting.
: main ( -- )
  0 4 fill | a |
  a | arr |
  4 [ | i | &!arr i >usize &!> i ! ] times
  &arr 2 >usize &> @ .
  arr drop ;

\ P-times-reject: one later use of `a` withholds the same grant. Rejects today
\ (`aliased by a`). Proves the grant above is load-bearing, not incidental.
: main ( -- )
  0 4 fill | a |
  a | arr |
  4 [ | i | &!arr i >usize &!> i ! ] times
  arr drop
  &a 0 >usize &> @ .
  a drop ;

\ P-splice: the identical shape routed through a combinator splice, which
\ carries no grant. Rejects today; D1 makes it compile.
: main ( -- )
  0 4 fill | a |
  a [ 4 > ] c::filter drop drop ;
```

The doorways grant, the splice does not. D1 fixes the splice. But D1 alone is not shippable,
for two reasons the brief measured and this spec locks below.

## Why D1 alone is not shippable

**The granting rule is already wrong at a loop back-edge, with no 6g change (recon 9).** Run
on `e87bcae`, this compiles and prints `0` then `9` — a mutation through `arr` silently
visible through `a` on the next iteration, no combinator involved:

```sooth
\ P-wrap (verbatim): accepts today, prints 0 then 9. Must reject after R1.
: main ( -- )
  0 4 fill | a |
  2 [ | i | a | arr | &a 0 >usize &> @ . true if &!arr 0 >usize &!> 9 ! else end arr drop ] times ;
```

`releasable_into` (`:1353`) grants an ancestor name on `!references(rest, name)`, where `rest`
is only the *remaining* sibling terms. Inside a `back_edge = true` body execution wraps around
to the body's first term, so a name used *earlier* in the body is still live where the grant is
handed down. 6g multiplies this hole's blast radius: every combinator call sited inside a loop
(legal since 6d) becomes a fourth doorway, and 10b/10c turn `times`/`if` themselves into
splices. Shipping D1 while leaving this open trades a false rejection for a silent wrong value.
R1 closes it. **This is the sequencing constraint: R1 lands and is validated before the
relaxation can mask it.**

**The splice's `back_edge` is observable today, through a name-hygiene defect (recon 10).**
`alpha_rename_locals` (`src/ast.rs:1208`) renames the callee's locals; `rename_call`
(`src/ast.rs:1229`) leaves a call to a builtin untouched. A caller local sharing a builtin's
name is read *in place of that builtin* inside the spliced body, silently. On `e87bcae`:

```sooth
1 >usize | len |  9 4 fill [ 4 > ] c::filter . drop  len drop   → prints 1
                  9 4 fill [ 4 > ] c::filter . drop             → prints 4
```

No diagnostic either way. This is the one route by which a granted caller name can appear as a
`Call` inside a spliced body, so D4's "the splice's flag is unobservable" argument depends on
closing it. D5 closes it at the root. **D5 lands before or with the relaxation.**

**The argument path has its own wrong-side `check_terms` (recon 8).**
`check_literal_against_declared_effect` (`:7644`, the `check_terms` at `:7674`) runs the
caller's quotation *literal* against the declared parameter effect, in the caller's own scope,
through the plain root entry point. It is reached from both of `inline_combinator`'s argument
paths (the mono loop at `:7400`, `check_poly_combinator_args` at `:7612`) and rejects *before*
the body splice, so D1 alone cannot discharge the `while`-nested-in-a-combinator shape. R2
grants it the same set.

## Locked decisions

- **D1 (from the brief).** `inline_combinator`'s body-check (`:7506`) becomes
  `check_terms_relaxed` with a `releasable_into`-computed `outer_releasable` set. This is the
  one call site on the wrong side of 6f's contract; the three correct sites are the pattern.

- **D2 (from the brief).** No change to `Moves`, `aliasing_origin` (`:1609`), or Copy-array
  move-blindness. A `Copy` local never enters the move map, so `moved_site` is `None` forever;
  this is a permanent property of the language, not a gap.

- **D3 (from the brief).** `lib/arrays.sth`'s header paragraph blaming aliasing for the
  inline-everything/no-`while` shape (`lib/arrays.sth:18`–`28`, the block "`sort`'s merge logic
  is inlined … Inlining and dropping `while` are the only shapes found that dodge both.") is
  deleted, its rationale retested and found false. **`sort`'s code is not restructured**, and
  **`sort`'s own per-word doc comment justifying a fixed-bound `times` over stopping early on
  the `u32` length bound is unrelated to aliasing and must stay.** `lib/arrays.sth` is
  currently untracked (`git status`: `?? lib/arrays.sth`); if it has not landed in a tracked
  commit when 6g starts, the `sort` dogfood golden and this edit move to whichever commit
  brings it in, and nothing else in the slice is affected.

- **D4 (from the brief).** Both new relaxed calls pass `back_edge = true`.
  - At `check_literal_against_declared_effect` this is **required for soundness**: the terms
    scanned are the caller's own literal, which references caller locals directly, so a granted
    name provably appears in that scan; with `false` its last use would be treated as final
    even though the callee re-executes the literal per iteration.
  - At the body splice, `true` matches `call`/`times` and is the conservative value (a granted
    name used inside is pinned live for the whole body). Recon 10's hygiene defect is what would
    otherwise make the splice's flag observable; **D5 closes it, making the splice's value a
    uniformity choice rather than a soundness one.** Do **not** write a test claiming to pin the
    splice's flag, and do **not** treat a green suite as evidence it is right.

- **D5 (from the brief).** Reject binding a local whose name collides with a builtin, a word in
  `env`, a polymorphic word (`poly.env`), or a combinator (`poly.combinators`). Sites: the mono
  `TermKind::Bind` arm (`:8448`) and the poly one (`:5201`), alongside the existing
  `reject_variant_local` (`:3886`) and `reject_duplicate_local` (`:3906`). The predicate
  `is_builtin_word_name` (`:2527`) already exists; `extern_redeclaration_error` (`:2533`) is the
  precedent for rejecting a declaration that reuses a builtin/word name, and
  `reject_variant_local` is the precedent for rejecting a *local* that collides with another
  namespace. This turns "a granted caller name can never appear as a `Call` in a spliced body"
  from false-by-counterexample into true-by-construction. Measured (brief): enforcing D5 at both
  bind sites compiles every file in `examples/` and `lib/` unchanged and passes the whole suite,
  and rejects recon 10's shadowing program; blast radius zero. A regex over `|...|` is **not** a
  valid way to re-check this (`|` is overloaded with clause dispatch).

- **R1 (new, forced by recon 9).** `releasable_into` (`:1353`) gains `live: &Liveness, at:
  usize`, and its filter splits — a name bound in the current invocation (`idx >= base_depth`)
  keeps today's rule verbatim; an ancestor name is granted only if the caller's own liveness
  says it is dead there:

  ```rust
  .filter(|(idx, b)| {
      if *idx >= base_depth {
          !references(rest, &b.name)
      } else {
          outer_releasable.contains(&b.name) && live.dead(&b.name, at + 1)
      }
  })
  ```

  **The index is `at + 1`, not `at`, and this is the whole correctness of the rule.**
  `nested_uses` (`:1265`) attributes a use found inside `terms[at]` to `at` itself, and `dead`
  (`:1319`) is `last < at`, so `live.dead(name, at)` is always false whenever the granted-into
  term is itself the user of the name — the entire reason one grants. Asking at `at + 1`
  reproduces exactly what `!references(rest, name)` meant ("no residual use after this term")
  while still catching recon 9's wrap-around, because a use anywhere in a `back_edge = true`
  body is recorded as `IMMORTAL_IN_BODY` (`:1183`, via `record_granted_use` `:1254`), which is
  not `<` any index. Measured (brief): with R1 applied to the three existing call sites alone
  (no D1, no R2), recon 9 rejects; the two-level execute-once grant chain below still compiles;
  a wrap-around-through-an-earlier-sibling-literal shape rejects; the suite stays green.

  ```sooth
  \ P-nest2 (verbatim): accepts today (prints 9), must keep accepting. Two levels of
  \ execute-once nesting, `a` used only inside the innermost one. R1 written with `at`
  \ instead of `at + 1` rejects this.
  : main ( -- )
    0 4 fill | a |
    true if
      true if
        a | arr | &!arr 0 >usize &!> 9 ! &arr 0 >usize &> @ . arr drop
      else end
    else end ;
  ```

- **R2 (new, forced by recon 8).** `check_literal_against_declared_effect` takes the same grant
  and its `check_terms` (`:7674`) becomes `check_terms_relaxed(..., granted, true)`. Its three
  non-combinator callers — `materialize_quotation_at_boundary` (`:8058`) and the two `if`-arm
  quotation-merge sites (`:9261`, `:9277`) — pass an empty set, preserving their behaviour
  exactly (with an empty grant, `back_edge` only feeds `record_granted_use`, which fires only for
  names in `outer_releasable`). Whether those three shapes deserve a grant is 7b's and 10c's
  question, not this slice's.

- **Q2 (was open, now decided).** The parameter is `granted: &HashSet<String>`, computed by the
  caller. The set must reach `check_literal_against_declared_effect`, which sits two frames below
  `inline_combinator` and has neither a sibling list nor an index of its own. One `HashSet`
  threaded down beats re-deriving position in a function that has no position.

## Open questions the spec must answer

**Q-corpus — does any shipped `.sth` need editing to positively pin the fix, and what does it
cost?** `examples/filter_while.sth` passes `scores` straight from its producer word into
`filter`, never bound to a local, and its header comment says so to dodge this bug. Binding
first would convert it into a positive pin. **Decision: leave the example untouched.** It is a
row of `CORPUS` in `tests/qbe_baseline.rs` (`tests/qbe_baseline.rs:38`), whose golden asserts
the emitted `.ssa` is byte-identical to `tests/qbe_baseline/filter_while.ssa`. A `scores` bind
introduces a slot and changes the lowering, so editing the source forces a sanctioned baseline
regeneration and a review of a generated `.ssa` diff, and it falsifies this spec's "everything
lowers byte-for-byte" claim. **What the decision costs:** `filter_while.sth` stays a
documents-a-limitation example rather than becoming a positive pin, so the pin is carried
instead by the new accept goldens and the `sort` dogfood (T-splice, T-while, T-sort), which are
the shape a user actually hits. **What it buys:** the byte-for-byte invariant stays literally
true, and no corpus baseline is regenerated or reviewed. The example's comment is left as-is: its
code genuinely still binds-through-producer, so the comment still accurately describes the code.

**Q-witness — which accept goldens witness D1 versus R2?** Decided by recon 8: D1 is the body
splice, R2 is the argument-path literal check, and the `while`-in-a-combinator shape rejects at
R2 *before* the body splice runs. So:

- **T-splice** (P-splice: `filter` over a bound array) needs only the body grant. Reverting D1
  turns it red; reverting R2 leaves it green. It is the D1 witness.
- **T-while** (a combinator whose body nests `c::while` with the mutable borrow inside the
  `while`'s quotation, over caller-bound arrays) rejects at the literal check without R2.
  Reverting R2 turns it red; reverting D1 does not reach it. It is the R2 witness.

Each reversion turns a *different, named* test red, so R2 is not redundant with D1.

**Q-order — ordering of D5 against D1/R2.** D5 is what makes D4's splice-side argument hold, so
it lands **before** the relaxation (Phase 2, ahead of Phase 3's D1/R2). R1 lands **first**
(Phase 1), alone, so the tightening is validated green before any relaxation can mask a mistake
in it. See [Phased delivery](#phased-delivery).

## Invariant recorded (was true, nothing else records it)

Grants are handed out **capture-blind**: `releasable_into` never consults captures.
Capture-awareness lives at the *use* site — `live_derivs` (`:1495`) and `aliasing_origin`
(`:1609`) each compute `capture_alive_names` (`:1391`) and check `!live.dead(name, at) ||
captured.contains(name)`. `live.dead` has exactly four call sites: those two, plus two internal
to the capture machinery (`:1419`, `past_last_use_capture` `:1459`). A future slice adding a
third consumer of `live.dead` on an aliasing path without the capture disjunct would break this
silently. R1 adds a `live.dead(&b.name, at + 1)` call inside `releasable_into`, which is a
*grant* site, not an aliasing-use site, so it does not need the disjunct — but it is now a fifth
`live.dead` caller, and the count above is why that is safe.

## Mechanism

1. `releasable_into` (`:1353`) gains `live: &Liveness, at: usize` and R1's split filter. Its
   three existing call sites (`call` `:8595`, `times` `:8683`, `if` `:9125`) and the new fourth
   already have both in scope, in `check_term`'s own parameters.
2. `inline_combinator` (`:7359`) gains `granted: &HashSet<String>`; its body-check (`:7506`)
   becomes `check_terms_relaxed(..., granted, true)`. Its sole call site (`:8968`, inside
   `check_term`'s `TermKind::Call` dispatch) computes
   `releasable_into(scope, base_depth, outer_releasable, &siblings[at + 1..], live, at)`, exactly
   as its three neighbours do, and passes the result as `granted`.
3. `check_poly_combinator_args` (`:7546`) and `check_literal_against_declared_effect` (`:7644`)
   gain the same `granted` parameter; the latter's `check_terms` (`:7674`) becomes
   `check_terms_relaxed(..., granted, true)`. Its three non-combinator callers (`:8058`, `:9261`,
   `:9277`) pass `&HashSet::new()`.
4. The mono `TermKind::Bind` arm (`:8448`) and the poly one (`:5201`) reject a local name that is
   a builtin (`is_builtin_word_name`, `:2527`), a word in `env`, a poly word (`poly.env`), or a
   combinator (`poly.combinators`), modelled on `extern_redeclaration_error` (`:2533`) /
   `reject_variant_local` (`:3886`).

No signature change to `check_terms`/`check_terms_relaxed`/`Liveness`. One new diagnostic (D5).

## Sanctioned files

- `src/check.rs` — the four functions above and the two `Bind` arms, plus its `#[cfg(test)] mod
  tests` for the R1 unit test.
- `tests/phase4_slice6g.rs` (new) — the goldens below. Reuse the absolute-path import helper
  pattern from `tests/phase4_combinators.rs:69` (`combinators_import`), because `run_src` writes
  the source under `temp_dir()` so a relative `import:` does not resolve; the `sort` dogfood needs
  the same helper pointed at `lib/arrays.sth`.
- `lib/arrays.sth` — delete only the aliasing-workaround paragraph (`:18`–`28`, D3). No code
  change; `sort`'s fixed-bound-`times` rationale stays. (Untracked; see D3.)
- `ROADMAP.md` — mark 6g implemented; correct the two stale texts named in the brief: the "Next
  action" pointer (`ROADMAP.md:589`) still reads "Phase 4 Slice 10a", and the 6g entry still
  prescribes a `self_tail`-conditioned `back_edge` that D4 rejects (constant `true` is correct).

## Exit criteria (goldens in `tests/phase4_slice6g.rs`)

| ID | Test | Kind | Phase | Source in → expected out |
| --- | --- | --- | --- | --- |
| U1 | `releasable_into_withholds_a_name_used_in_a_back_edge_body` | unit | 1 | over a synthetic `Scope`+`Liveness` built by `scan`: an ancestor name with `IMMORTAL_IN_BODY` is **absent** from the grant; an ancestor name in `outer_releasable` that the body never mentions is **present** (shows the tightening does not over-tighten) |
| T-wrap | `if_inside_a_loop_reading_an_alias_is_an_error` (P-wrap) | reject | 1 | `cannot borrow … mutably` + `aliased by`. **Behaviour change**: accepted today, prints `0`/`9` |
| T-nest2 | `two_level_execute_once_grant_still_accepted` (P-nest2) | accept | 1 | builds, prints `9`. Discriminates `at + 1` from `at` (the `at` form rejects it) |
| T-doorway-ok | `times_doorway_grants_the_bound_alias` (P-times-accept) | accept | 1 | builds, prints `2` |
| T-doorway-no | `later_use_withholds_the_times_grant` (P-times-reject) | reject | 1 | `aliased by a` |
| T-shadow | `binding_a_local_named_after_a_builtin_is_rejected` (`len`) | reject | 2 | D5 diagnostic naming the collided builtin. **Behaviour change**: accepted today, prints `1` silently |
| T-splice | `bound_array_passed_to_filter_is_accepted` (P-splice) | accept | 3 | builds. **D1 witness** |
| T-while | `while_nested_in_a_combinator_body_over_bound_arrays_is_accepted` | accept | 3 | builds, prints the copied element. **R2 witness** |
| T-sort | `sort_called_with_bound_array_locals_runs` | accept | 4 | `lib/arrays.sth`'s shipped `sort` over a bound array + comparator prints it sorted |
| T-green | whole suite green | regression | 1–4 | `cargo fmt --check && cargo clippy -- -D warnings && cargo test` |
| T-roadmap | ROADMAP 6g implemented; both stale texts corrected | doc | 4 | prose |

T-sort is the dogfood and the honest measure of the bug's cost: today you cannot call the
library's own `sort` on arrays you have named.

## Mutation-required criteria

Run each in a **throwaway copy of the worktree**, never the shared one (a concurrent reviewer
has previously mistaken an in-place mutation for a real bug). Keep `target/` copies **off**
`/tmp` (a 32G tmpfs shared across sessions; an unrelated session has been ENOSPC'd by orphaned
scratch dirs). There is no tooling for this; copy the tree elsewhere and build there.

Each mutation names the phase it is runnable in and the exact test it turns red. **No witness
listed here needs a later phase's code to compile** — in particular the Phase 1 witnesses use no
combinator, so they compile in an R1-only tree.

- **M-R1 (Phase 1, the soundness one).** Revert `releasable_into`'s filter to the pre-R1
  `(idx >= base_depth || outer_releasable.contains(name)) && !references(rest, name)` →
  **T-wrap** and **T-nest2** go red. Additionally: change `at + 1` to `at` alone → **T-nest2**
  goes red (over-tightening), proving the index is load-bearing. Under the reverted build T-wrap's
  program must not merely compile: it must **run and print `0` then `9`**. A reject test that only
  checks "fails" cannot distinguish this from a type error, and the wrong value is the point.
- **M-U1 (Phase 1).** Delete the `live.dead(…)` conjunct → **U1** red. Delete the
  `idx >= base_depth` fast path (route every name through the ancestor branch) → a Phase-1
  accept golden that binds-and-uses within one invocation (T-doorway-ok) red, proving the two
  branches are not interchangeable.
- **M-D5 (Phase 2).** Remove the D5 rejection at both bind sites → **T-shadow** red: the program
  compiles and prints `1` again (the shadowing silent wrong value). Assert the value `1`, not
  just "compiles".
- **M-D1 (Phase 3).** Revert `inline_combinator`'s body-check to plain `check_terms` → **T-splice**
  red (the recon-1 rejection is raised inside the spliced body, at `filter`'s own `&!arr`).
  **T-while** must stay green under this reversion; if it goes red too, it is pinning R2 as well
  and must say so in its own test comment. This is the brief's asked-for mutation test.
- **M-R2 (Phase 3).** Revert `check_literal_against_declared_effect` to plain `check_terms` →
  **T-while** red and **T-splice** green. Both halves matter: without the second, R2 looks
  redundant with D1; without the first, M-R2 could pass by accident.
- **Not pinnable, stated so it is not faked (D4).** Flipping the *body splice's* `back_edge` from
  `true` to `false` changes no test and no probe once D5 has closed recon 10's hygiene defect. Do
  not write a test that claims to pin it; do not treat a green suite as evidence the value is
  right. The literal check's `back_edge = true` **is** load-bearing (D4) and is pinned by M-R2's
  value assertion, not by a flag-flip test.

## Out of scope

Restructuring `lib/arrays.sth`'s `sort` (D3 requires only the stale rationale to go). Any change
to `Moves`/move-tracking for `Copy` types (D2). Whether the three non-combinator
`check_literal_against_declared_effect` callers deserve a grant (R2). The `PolyType::Ref` gap.
Any lowering, IR, or diagnostic-text change beyond D5's new diagnostic. Editing any
corpus-pinned example (Q-corpus).

**Sequencing.** Now unblocked — 10a has merged (`e87bcae`) — and before 10b/10c, which is a
prerequisite relation, not a preference: both convert a doorway that grants into a splice that
does not (10b for every `times`, 10c for every `if`), so P-times-accept regresses at 10b unless
this slice has landed.

## Phased delivery

**Phase 1 (hard) — R1, the tightening, alone.** `releasable_into` gains `live`/`at` and the
`at + 1` split filter; the three existing call sites are updated; U1 unit test; T-wrap /
T-doorway-no reject goldens; T-nest2 / T-doorway-ok accept goldens; mutations M-R1, M-U1. Lands
first and independently: measured green on the existing suite with no relaxation present, so if
anything in it is wrong the failure is visible before the relaxation can mask it. No combinator
splice is touched here, so every Phase-1 golden compiles in this tree.

**Phase 2 (standard) — D5, the bind-collision reject.** The two `Bind` arms reject a local name
colliding with a builtin/word/poly/combinator; new diagnostic; T-shadow reject golden; mutation
M-D5. Lands before the relaxation so D4's splice-side argument holds by construction. Measured
blast radius (brief) is zero, so no existing golden changes.

**Phase 3 (standard) — D1 + R2, the relaxation.** `granted` threaded through
`inline_combinator`, `check_poly_combinator_args`, `check_literal_against_declared_effect`; both
`check_terms` calls become `check_terms_relaxed(..., granted, true)`; T-splice (D1 witness) and
T-while (R2 witness) accept goldens; mutations M-D1, M-R2. T-wrap / T-doorway-no / T-shadow must
stay red across this phase.

**Phase 4 (standard) — dogfood, D3, docs.** T-sort over `lib/arrays.sth`'s shipped `sort` with
bound array locals (absolute-path import helper); delete the stale aliasing-workaround paragraph
in `lib/arrays.sth`; correct ROADMAP's "Next action" pointer and the 6g entry's stale
`self_tail`-conditioned `back_edge` text; mark 6g implemented (T-roadmap).

```json
{
  "phases": [
    { "phase": 1, "focus": "releasable_into loop-aware grant with at+1 index; unit test; two reject and two accept goldens (no combinator, R1-only tree); mutations M-R1/M-U1", "difficulty": "hard" },
    { "phase": 2, "focus": "D5 reject a local name colliding with a builtin/word/poly/combinator at both Bind arms; T-shadow reject golden; mutation M-D5", "difficulty": "standard" },
    { "phase": 3, "focus": "D1+R2 grant threaded into inline_combinator and the literal check; T-splice (D1 witness) and T-while (R2 witness) accept goldens; mutations M-D1/M-R2", "difficulty": "standard" },
    { "phase": 4, "focus": "sort dogfood golden via absolute-path import; delete stale arrays.sth workaround paragraph; correct ROADMAP next-action and self_tail back_edge text", "difficulty": "standard" }
  ]
}
```
