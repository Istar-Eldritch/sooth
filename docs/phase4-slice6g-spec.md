# Phase 4 Slice 6g spec: combinator splices learn 6f's granting rule

Derives from [`docs/phase4-slice6g-brief.md`](./phase4-slice6g-brief.md). The brief's recon is
ground truth **except where this spec says otherwise**: probing the built compiler falsified
one premise and found one missing site, and both are corrected here rather than inherited.
`file:line` anchors are pre-implementation, taken against `491cf54`.

**Checker-acceptance-only.** Edits are confined to `src/check.rs`'s liveness/granting layer
plus tests, one example line, and two stale comments. No `Instr`/`Terminator`, no `Type`/
`IrType`, no lowering, no `qbe.rs`. A program that compiles today and still compiles after
this slice lowers byte-for-byte, because nothing this slice touches survives into the IR.

## What probing changed, and why the brief's D1 is not shippable alone

Every claim below was built and run, not read. Programs `P1`-`P9` are listed in
[Probe programs](#probe-programs); the matrix is the measured result of five compiler
variants against them.

| | today | D1 only | R1 only | D1+R1 | D1+R1+R2 |
| --- | --- | --- | --- | --- | --- |
| P1 recon-1 minimal (`filter` after a bind) | reject | **ok** | reject | **ok** | **ok** |
| P2 bind-then-pass, straight-line splice | reject | **ok** | reject | **ok** | **ok** |
| P3 split-out merge word from a swapping loop | reject | **ok** | reject | **ok** | **ok** |
| P4 splice inside a `times`, literal reads the alias | reject | **ok** | reject | reject | reject |
| P5 same, self-tail combinator | reject | reject | reject | reject | reject |
| P5b P5 with a literal that captures nothing | reject | **ok** | reject | **ok** | **ok** |
| P6 `if` arm inside a `times`, same shape | **ok** | **ok** | reject | reject | reject |
| P7 `while` nested inside a combinator body | reject | reject | reject | reject | **ok** |
| P8 `lib/arrays.sth`'s `sort` called with bound locals | reject | **ok** | reject | **ok** | **ok** |
| P9 `sort` restructured to use `while` | reject | reject | reject | reject | **ok** |

Bold is "accepted". P4/P5/P6 are the rows where acceptance is **wrong**: P4 compiled under
"D1 only" and printed `0` then `9`, a mutation through an alias silently visible through the
name the caller still reads on the next iteration. That is the exact class the diagnostic
exists to catch.

P5 rejects in every column, but **not** because of anything this slice does: its literal
captures the caller's array and is still reachable through the combinator's own quotation
parameter at the back-edge, so `capture_alive_names` (`:1376`) keeps the name live. P5b is
the same program with a non-capturing literal, and it compiles under the fix. Do not read
P5's row as evidence for R1.

Three findings follow.

**C1. The brief's D1, alone, trades a false rejection for a silent wrong value.** Granting
the caller's name into the splice is right; the machinery that decides *which* names may be
granted is not loop-aware. `releasable_into` (`src/check.rs:1338`) grants an ancestor name on
`!references(rest, name)`, where `rest` is only the *remaining* sibling terms. Inside a
back-edge body the remaining terms are not the whole story: execution wraps around to the
body's first term, so a name used *earlier* in the same body is still live at the point the
grant is handed down. `Liveness` already knows this (a granted name used anywhere in a
`back_edge = true` body gets `IMMORTAL_IN_BODY`, `:1168`); `releasable_into` never asks it.

**C2. The same hole is already open today, without any 6g change.** P6 is an `if` arm inside
a `times` body, and it compiles on unmodified `491cf54` and prints `0` then `9`. So this is a
6f defect, not one 6g introduces. What 6g does is multiply its blast radius: every
combinator call sited inside a loop (legal since 6d) becomes a fourth doorway into it, and
10b/10c then turn `times` and `if` themselves into combinator splices. Fixing the rule is
therefore in scope here, and only here: shipping D1 while leaving `releasable_into` wrong
means shipping a regression (P4 goes from a correct rejection to a wrong answer) in exchange
for a false-rejection fix. That is not a trade this slice is allowed to make.

**C3. `inline_combinator`'s body splice is not the only wrong-side `check_terms`.**
`check_literal_against_declared_effect` (`:7425`, the `check_terms` at `:7444`) runs the
caller's quotation *literal* against the declared parameter effect, in the caller's own
scope, through the plain root entry point. It is reached from both of `inline_combinator`'s
argument paths (the mono loop at `:7230`, `check_poly_combinator_args` at `:7396`). Recon 6
missed it because it is an argument check, not a body splice, but it rejects first: P7's
`cannot borrow cs__inl0 mutably: it is aliased by s0` was traced to this call, not to the
splice. Without it, D3's `while` half is not discharged (P7 and P9 stay rejected), so the
brief's own D3 cannot be closed by D1 alone either.

## Locked decisions

- **D1 (from the brief).** `inline_combinator`'s body-check (`:7310`) becomes
  `check_terms_relaxed` with a `releasable_into`-computed `outer_releasable` set. Unchanged
  in substance; the brief's recon 3/6 stand.
- **D2 (from the brief).** No change to `Moves`, `aliasing_origin`, or Copy-array
  move-blindness. Recon 2 is a correct property of the language, not a gap.
- **D3 (from the brief, now dischargeable).** `lib/arrays.sth`'s `sort` header comment
  documenting the inline-everything/no-`while` workaround is deleted, because the rationale
  is measured false: the split-out-merge-word shape (P3) and the `while`-in-the-loop shape
  (P7/P9) both compile under this fix, and P9 (a `while`-restructured `sort`) sorts
  correctly. `sort`'s code is **not** restructured (brief, out of scope); only the stale
  paragraph goes, and the three shapes it blamed become accept goldens.
- **R1 (new, forced by C1/C2).** `releasable_into` gains `live: &Liveness, at: usize`. An
  ancestor name (`idx < base_depth`) is granted only if the caller's own liveness says it is
  dead *there*, which under a back-edge body means "used nowhere in this body". A name bound
  in the current invocation (`idx >= base_depth`) keeps the existing `!references(rest, ...)`
  rule verbatim: it is rebound on every iteration, so a later-index use reads the fresh
  binding and the wrap-around concern does not apply.

  ```rust
  .filter(|(idx, b)| {
      if *idx >= base_depth {
          !references(rest, &b.name)
      } else {
          outer_releasable.contains(&b.name) && live.dead(&b.name, at)
      }
  })
  ```

  This is a tightening in one direction only. Measured: the full suite (18 targets, 1457
  tests) is green with R1 applied alone, and no shipped example, library word, or golden
  changes shape. Its only behavioural effect on today's compiler is P6, which stops
  compiling.
- **R2 (new, forced by C3).** `check_literal_against_declared_effect` takes the same
  `granted` set and uses `check_terms_relaxed`. Its three non-combinator callers
  (`materialize_quotation_at_boundary` `:7790`, the two `if`-arm quotation-merge sites
  `:8930`/`:8934`) pass `&HashSet::new()`, preserving their behaviour byte-for-byte. Whether
  *those* three shapes deserve a grant is 7b's and 10c's question, not this slice's.
- **D4.** Both new relaxed calls pass `back_edge = true`. See Q1: the flag is unobservable at
  the body splice and unpinnable at the literal check, so it is chosen by argument, and the
  argument is that a combinator splice can re-execute (its call site may sit inside a loop)
  and a literal is spliced for real inside a `call`/`times`, which is exactly the
  justification `call` (`:8302`) already gives for its own `true`.

## Resolved questions

- **Q1 (brief: the exact `back_edge` for a splice). Rejected: recon 7's `self_tail`
  candidate. Answer: constant `true`, chosen by argument because the flag is unobservable.**
  The brief framed this as a false-rejection risk; that framing is wrong in both directions.
  `back_edge = true` is the *conservative* value (a granted name used inside is pinned live
  for the whole body), and both values are strictly more permissive than today's empty grant,
  so neither can add a rejection. Measured: flipping the body splice's `back_edge` between
  `true` and `false` changes nothing at all, not one probe and not one test in the suite.
  The reason is structural: `Liveness::scan` only records a use of a name it is tracking, the
  granted names are the *caller's* locals, and the spliced body is the callee's own terms
  with its own locals alpha-renamed (`:7305`), so a granted name can never appear as a `Call`
  in the scanned list. `self_tail` is therefore a condition on a flag with no observable
  effect, and it conditions in the wrong direction (it would relax the loop case and tighten
  the one-shot case). A constant, matching `call`/`times`, is what a future slice that *does*
  make granted names visible inside a splice (7b's erased closures, 10b/10c) wants already in
  place.
- **Q2 (brief: `granted` parameter vs `siblings`/`at`/`base_depth`/`outer_releasable`).
  `granted: &HashSet<String>`, computed by the caller.** Not the style choice the brief
  expected: C3 decides it. The set has to reach `check_literal_against_declared_effect`,
  which sits two frames down (`inline_combinator` -> `check_poly_combinator_args` ->
  literal check) and has neither a sibling list nor an index of its own. One `HashSet`
  threaded down beats re-deriving position in a function that has no position.
- **Q3 (brief: is the reversion the mutation test, or is a unit test also warranted).
  Both, and three reversions, not one.** The brief anticipated one mutation with a
  false-rejection failure mode; there are now two independent relaxations and one tightening,
  and the tightening's failure mode is a silent wrong value. See
  [Mutation-required](#mutation-required-criteria). R1 additionally gets a direct
  `#[cfg(test)]` unit test on `releasable_into`, because the end-to-end reject goldens
  discriminate the *rule* only through four nested invocations, and a unit test that
  constructs a `Scope` plus a `Liveness` with a known `IMMORTAL_IN_BODY` entry pins the rule
  itself. Precedent: 6f's own R6 walk-stop test, which had to become a direct unit test for
  the same reason.
- **Q4 (brief: does an existing golden change shape). Yes, one line.**
  `examples/filter_while.sth` binds `scores` to a local before passing it to `filter`, and
  the sentence in its header comment explaining the avoidance is deleted. Measured: the
  bound form is rejected today and, under the fix, prints the example's existing golden
  output unchanged (`3`, `5`). This converts an existing example from "documents a
  limitation" into a positive pin at the cost of one line and no new file. The stronger pin
  is T5 (`sort` with bound array locals), which is the shape a user actually hits.

## Mechanism

1. `releasable_into` (`:1338`) gains `live`/`at` and R1's split filter. Its four call sites
   (`call` `:8311`, `times` `:8397`, `if` `:8797`, and the new one) already have both in
   scope, in `check_term`'s own parameters (`:8136`/`:8137`).
2. `inline_combinator` (`:7194`) gains `granted: &HashSet<String>`; its body-check (`:7310`)
   becomes `check_terms_relaxed(..., granted, true)`.
3. `check_poly_combinator_args` (`:7350`) and `check_literal_against_declared_effect`
   (`:7425`) gain the same parameter; the latter's `check_terms` (`:7444`) becomes
   `check_terms_relaxed(..., granted, true)`. The other three callers pass an empty set.
4. The sole `inline_combinator` call site (`:8640`) computes
   `releasable_into(scope, base_depth, outer_releasable, &siblings[at + 1..], live, at)`,
   exactly as its three neighbours do.

No new struct, no new field, no new diagnostic, no signature change to `check_terms`/
`check_terms_relaxed`/`Liveness`. Measured size of the whole mechanism: 90 insertions, 16
deletions in `src/check.rs`, `cargo fmt --check` and `cargo clippy -- -D warnings` clean.

## Sanctioned files

- `src/check.rs` (the four functions above) plus its `#[cfg(test)] mod tests` for the R1 unit
  test.
- `tests/phase4_slice6g.rs` (new): the goldens below.
- `examples/filter_while.sth`: bind `scores` first, delete the avoidance sentence (Q4). Its
  existing golden output must not change.
- `lib/arrays.sth`: delete the stale workaround paragraph from the header comment (D3). No
  code change. **Note:** this file is currently untracked in the working tree; if it has not
  landed when 6g starts, T5 and the D3 edit move to whichever commit brings it in, and the
  rest of the slice is unaffected.
- `ROADMAP.md`: mark 6g implemented; record the two defects under [Found, not
  fixed](#found-not-fixed).

## Exit criteria (goldens in `tests/phase4_slice6g.rs`)

| ID | Test | Kind | Phase | Source in -> expected out |
| --- | --- | --- | --- | --- |
| U1 | `releasable_into_withholds_a_name_used_in_a_back_edge_body` | unit | 1 | ancestor name with `IMMORTAL_IN_BODY` in the caller's `Liveness` is absent from the grant; the same name with a passed last use is present |
| T1 | `combinator_splice_inside_a_loop_reading_an_alias_is_still_an_error` (P4) | reject | 1 | `cannot borrow ... mutably` + `aliased by` |
| T2 | `self_tail_combinator_splice_over_a_captured_alias_is_still_an_error` (P5) | reject | 2 | same. Guards `capture_alive_names`, **not** R1 (see the matrix note) |
| T2b | `self_tail_combinator_splice_over_a_bound_array_is_accepted` (P5b) | accept | 2 | P5 with a non-capturing literal builds and runs |
| T3 | `if_arm_inside_a_loop_reading_an_alias_is_an_error` (P6) | reject | 1 | same. **Behaviour change**: accepted today |
| T4 | `bound_array_passed_to_filter_is_accepted` (P1) | accept | 2 | recon-1 minimal builds and runs |
| T5 | `bound_array_passed_to_a_borrowing_combinator_is_accepted` (P2) | accept | 2 | builds, prints the mutated element |
| T6 | `quotation_taking_word_called_from_a_swapping_loop_is_accepted` (P3) | accept | 2 | builds |
| T7 | `while_nested_in_a_combinator_body_over_bound_arrays_is_accepted` (P7) | accept | 2 | builds, prints the copied element |
| T8 | `sort_called_with_bound_array_locals_runs` (P8) | accept | 3 | `lib/arrays.sth`'s shipped `sort` over `[4 1 3 2]` prints `1 2 3 4` |
| T9 | `filter_while_example_binds_first` | accept | 3 | `examples/filter_while.sth` still prints `3`, `5` |
| T10 | whole suite green | regression | 1-3 | `cargo fmt --check && cargo clippy -- -D warnings && cargo test` |
| T11 | ROADMAP 6g implemented, both found defects recorded | doc | 3 | prose |

T8 is the dogfood. It is also the honest measure of the bug's cost: today you cannot call the
library's own `sort` on arrays you have named.

## Mutation-required criteria

Run each in a **throwaway copy of the worktree**, never the shared one (a concurrent reviewer
has previously mistaken an in-place mutation for a real bug).

- **M1 (D1).** Revert `inline_combinator`'s body check to plain `check_terms` -> **T4 must go
  red** (measured: the recon-1 rejection is raised inside the spliced body, at `filter`'s own
  `&!arr`). This is the brief's asked-for mutation test. T2b/T5/T6/T8 are expected to go red
  with it, but that is not measured for this exact build: record which ones do, and any that
  stays green is pinning R2 rather than D1 and must say so in its own test comment.
- **M2 (R2).** Revert `check_literal_against_declared_effect` to plain `check_terms` -> T7
  goes red and T4 stays green. Both halves matter: without the second, R2 looks redundant
  with D1; without the first, M2 could pass by accident.
- **M3 (R1, the soundness one).** Revert `releasable_into`'s filter to
  `(idx >= base_depth || outer_releasable.contains(name)) && !references(rest, name)` -> T1
  and T3 go red. **T2 is not an M3 witness** and must not be listed as one: it rejects
  identically with and without R1. Stronger assertion required: under the reverted build T1's
  program must not merely compile, it must **run and print `0` then `9`**. A reject test that
  only checks "compiles" would not distinguish this from a type error, and the wrong value is
  the point.
- **M4 (U1).** Delete the `live.dead(...)` conjunct -> U1 red. Delete the `idx >= base_depth`
  fast path (route every name through the ancestor branch) -> T4 red, proving the two branches
  are not interchangeable. Expected by construction, not measured: a name bound in the current
  invocation is in no `outer_releasable`, so the ancestor branch grants it nothing.
- **Not pinnable, stated so it is not faked (D4).** Flipping either new `back_edge` from
  `true` to `false` changes no test and no probe. Do not write a test that claims to pin it;
  do not treat a green suite as evidence the value is right. If a later slice makes it
  observable, that slice owns the test.

## Found, not fixed

Both are recorded in `ROADMAP.md`, not addressed here.

- **A caller local shadows a word name inside a splice.** `0 4 fill | len | len [ 4 > ]
  c::filter drop drop` fails with `>i64 requires a numeric source, found [i64 4]`: the
  spliced body's own `len` call resolves to the caller's local. `alpha_rename_locals`
  (`:7305`) renames the callee's locals, but the caller's stay visible in the spliced scope,
  so any library combinator can be broken by a caller's choice of local name. Pre-existing,
  orthogonal to liveness, and a hygiene defect rather than a granting one. It is also why Q1
  is unobservable today: the only way a granted name could appear in a spliced body is this
  collision, and it fails earlier.
- **The `releasable_into` hole's other three doorways.** R1 closes `if`/`call`/`times` along
  with the splice because it fixes the shared function, and T3 pins the `if` case. No further
  audit of 6f's granting sites was done, and none is claimed.

## Out of scope

Restructuring `lib/arrays.sth`'s `sort` to use `while` or a split-out merge word (D3 requires
only the stale rationale to go; P9 exists to falsify that rationale, not to ship). Any change
to `Moves`/move-tracking for `Copy` types (D2). The polymorphic-body `if` gap (`:3664`/
`:3672`) and `poly_call_term` (a combinator's resolution is intercepted at `:8629` and never
reaches it). The `PolyType::Ref` gap. Whether the three non-combinator
`check_literal_against_declared_effect` callers deserve a grant (R2). Any lowering, IR, or
diagnostic-text change. **Sequencing: after 10a lands, before 10b/10c begin** (ROADMAP Phase 4
item 6g); C2 sharpens the reason, since 10b/10c convert `times` and `if` into combinator
splices and would inherit the C1 hole through a fourth and fifth doorway.

## Probe programs

Sources are abbreviated; each was built and run against five compiler variants.

- **P1** recon-1 minimal: `0 4 fill | a | a [ 4 > ] c::filter drop drop`.
- **P2** a mono combinator `run2 ( [i64 4] [ -- ] -- [i64 4] )` = `| q | | arr | q call
  &!arr 0 >usize &!> 9 ! arr`, called as `0 4 fill | a | a [ a drop ] run2 drop`.
- **P3** `mergepass ( [i64 4] [i64 4] [ -- ] -- [i64 4] [i64 4] )` binding both arrays,
  called from a `times` loop that rebinds and swaps the two row items.
- **P4** P2's `run2` called from inside `2 [ | i | a [ &a 0 >usize &> @ . ] run2 drop ]
  times`. **Must reject.** Accepted, it prints `0` then `9`.
- **P5** P4's shape with a self-tail combinator (`spin`, recursing on a countdown) instead of
  a `times`. Rejects in every variant, by the capture rule, not by R1.
- **P5b** P5 with `[ ]` for the literal, so nothing captures the caller's array.
- **P6** no combinator at all: `2 [ | i | a | arr | &a 0 >usize &> @ . true if &!arr 0
  >usize &!> 9 ! else end arr drop ] times`. **Must reject.** Accepted on`491cf54` today,
  where it prints `0` then `9`.
- **P7** a combinator whose body nests `c::while`, with the mutable borrow inside the
  `while`'s quotation, over arrays bound by the caller.
- **P8** `lib/arrays.sth`'s shipped `sort`, called as `b s0 [ - ] a::sort` with `b` and `s0`
  bound locals.
- **P9** a throwaway copy of `arrays.sth` with `sort`'s per-block `times` rewritten as a
  `c::while`. Compiles and sorts correctly only under D1+R1+R2. Probe only, never shipped.

## Phased delivery

**Phase 1 (hard) - R1, the tightening, alone.** `releasable_into` gains `live`/`at` and the
split filter; four call sites updated; U1 unit test; T1/T3 reject goldens; M3/M4. Lands
first and independently: measured green on the existing suite with no relaxation present, so
if anything in it is wrong the failure is visible before the relaxation can mask it.

**Phase 2 (standard) - D1 + R2, the relaxation.** `granted` threaded through
`inline_combinator`, `check_poly_combinator_args`, `check_literal_against_declared_effect`;
both `check_terms` calls become `check_terms_relaxed(..., granted, true)`; T2/T2b and T4-T7;
M1/M2. T1/T3 must stay red across this phase.

**Phase 3 (standard) - dogfood, D3, docs.** T8; `examples/filter_while.sth` bound form (T9);
delete the stale workaround paragraph in `lib/arrays.sth` and the avoidance sentence in the
example; ROADMAP 6g marked implemented with both found-not-fixed defects recorded (T11).

```json
{
  "phases": [
    { "phase": 1, "focus": "releasable_into loop-aware grant, unit test, three reject goldens, mutation M3/M4", "difficulty": "hard" },
    { "phase": 2, "focus": "grant threaded into inline_combinator and the literal check, four accept goldens, mutation M1/M2", "difficulty": "standard" },
    { "phase": 3, "focus": "sort dogfood golden, filter_while bound form, stale workaround comments deleted, ROADMAP", "difficulty": "standard" }
  ]
}
```
