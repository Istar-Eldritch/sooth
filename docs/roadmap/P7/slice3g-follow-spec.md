# P7.S3g-follow: the self-tail loop transform for a polymorphic body

## Summary

S3g lowers a self-call inside a non-inline generic body to an ordinary recursive
`Instr::Call` (D3, deliberately deferred): correct, but one real stack frame per
recursion level, where a monomorphic self-tail word instead lowers to a loop
back-edge. This slice closes that gap for polymorphic bodies.

The target program is `loopg`, S3g's own golden, whose recursive call is in tail
position (the last term of the `if` combinator's recursive-arm quotation) yet gets
`self_tail: false` unconditionally today:

```sooth
: iszero ( i64 -- Bool ) 0 eq ;
: loopg ( 'T: Copy i64 -- 'T )
  dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;
```

The slice has three pieces, located by the brief's recon and corrected/completed by a
round-1 review pass (the concrete precedent of "locate-and-reject, don't yet implement
back-edge disposal" is mirrored throughout; one sub-piece, 1c, is a required
investigation rather than a locked design — see "Design, Piece 1"):

1. **A checker-side guard, larger than first scoped**: net-new tail-position threading
   through the poly walker (`poly_walk`/`poly_term`/`poly_call_term`/
   `poly_walk_arms`/`poly_combinator_call` carry no tail state today), plus a poly-side
   equivalent of the concrete `check_linear_across_back_edge`
   (`src/check/terms.rs:1058`) for a linear value stranded below the recursive call's
   argument window (typechecks clean at HEAD; the loop transform would otherwise reach
   it with no destructor plan), plus a required investigation into whether the
   concrete `check_reference_across_back_edge`'s hazard (a reference to a local
   crossing the back-edge) is even reachable given `PolySlot` carries no
   `deriv`/owned-root data. Both guards, if needed, are *located rejections*, mirroring
   the concrete guards, which are themselves still only located rejections, not full
   back-edge disposal.
2. **Two small plumbing changes** at the `lower_word_parts` call sites (native
   `driver.rs`, REPL `lower_instantiation`), each of which already holds everything
   `has_self_tail_call` needs and hardcodes `false`.
3. **A back-edge dispatch branch** inside the existing poly self-call arm in
   `src/ir/func_builder/calls.rs`, sourcing arity from `self.cur_poly_callee` rather
   than `self.env.get(name)` (which panics on a poly name).

The checker guard (piece 1) must land **before** the lowering pieces (2, 3), so the
loop transform never reaches a stranded-linear program without a rejection in front
of it. That is the exact regression the brief's open-question-1 probe found.

## Load-bearing facts (verified against HEAD by the brief)

- `has_self_tail_call` / `tail_position_calls` (`src/check/drop_graph.rs:377-382`, its arity via
  `declared_input_count`, `drop_graph.rs:135-140`) is a purely syntactic name-walk and is **already poly-aware**
  (`declared_input_count` has an explicit `word.poly.as_ref()` branch). Probed
  directly: it returns `true` for `loopg`'s body through the `if` combinator splice,
  with zero changes to `poly.rs`. No new tail-*detection* machinery is needed.
- `ctx.is_self_tail_call()` inside `check_poly_body` is dead: the `Ctx` is built with
  `&CombinatorIndex::new()` (`poly.rs:420-428`) and nothing in `poly.rs` reads
  `Ctx::Word.self_tail_call`. Fixing the empty index there changes nothing observable;
  the guard's tail gating must be threaded explicitly (see Phase 1).
- The poly self-call arm in the checker is `poly.rs:1297` (`ctx.mangled_name() ==
  Some(name)`): a pure structural pointwise match against `sig.inputs`/`sig.outputs`
  (S3g, D1). It fires for a self-call regardless of tail position and today runs
  `stack.truncate(base)` — which *keeps* `stack[..base]` (everything below the argument
  window) and drops the consumed argument window itself — with no linearity check on
  what it kept.
- The lowering poly self-call arm is `calls.rs:667-687`, gated on
  `self.cur_poly_callee`; it runs *before* R7 (`calls.rs:688-701`) and today
  unconditionally falls to `emit_user_call`. R7 is structurally unreachable from a
  poly self-call (its `name == self.cur_word_name` can never hold for a bare poly
  name, and its `self.env.get(name)` would panic).
- `self.cur_poly_callee` (`func_builder/mod.rs:759`) already stores this
  instantiation's `(callee, arity)`, built from its own concrete `effect` — the
  correct phi/arg-count source for the back-edge.
- `begin_loop` (`func_builder/mod.rs:543-570+`) inspects nothing about polymorphism;
  `lower_word_parts` (`mod.rs:779-783`) already calls it whenever `self_tail` is
  `true`, for any caller. Threading a correct `self_tail` builds a working header with
  no change to `begin_loop`.
- The stale comment at `driver.rs:264-272` claims `has_self_tail_call` "only
  recognizes a plain-name `Call`, never a `CallInst` lookup" as the reason `self_tail`
  stays `false`. Contradicted by the recon probe; the real reason is D3's lowering
  deferral. Rewrite it as part of this slice.

## Locked decisions carried from S3g (untouched)

- The self-call check is structural and non-unifying (S3g finding 5/9). This slice is
  a lowering-and-guard change to an already-typechecked self-tail call, not a new
  type check, and has no interaction with the termination argument.
- An ordinary recursive `Instr::Call` remains the fallback (S3g finding 7): a
  non-tail self-call, or a word with no loop header, keeps lowering exactly as S3g
  shipped it. This slice only adds a branch ahead of that fallback, never removes it.

## Design

### Piece 1 — poly-side back-edge guards (checker)

**Round-1 review correction:** the first draft of this piece under-specified the tail-
position plumbing and misdescribed the concrete precedent (it does not carry tail state
on `Ctx` — verified below), and omitted a second sibling guard the concrete side runs
at the same site. Both are corrected here with a concrete, source-verified design.

#### 1a. Tail-position threading through `poly_walk` (net-new plumbing, not a tweak)

Verified against HEAD: **no poly-walk function carries or computes tail position
today.** `poly_walk`, `poly_term`, `poly_call_term`, `poly_walk_arms`, and
`poly_combinator_call` (`poly.rs:475, 527, 658, 1798, 2148`) take no `tail` parameter.
The concrete precedent is **not** `Ctx`-carried state; it is a plain `tail: bool`
function parameter, computed per term as `tail && i == last` inside
`check_terms_relaxed`'s loop (`terms.rs:11-90`, specifically the `tail && i == last`
call at the `check_term` call site), and threaded verbatim into a combinator arm's own
re-entry (`inline_combinator(...,  tail)` at `terms.rs:693`, and the quotation-splice
`call` site at `terms.rs:363` which passes `tail` unchanged into the spliced body's own
`check_terms_relaxed`). `ctx.is_self_tail_call()` is a separate, word-level predicate
(is *this word* self-tail-recursive at all), independent of the per-term `tail` flag.

The mechanical, source-verified port:

- Add `tail: bool` to `poly_walk`, `poly_term`, `poly_call_term`, `poly_walk_arms`, and
  `poly_combinator_call`'s signatures.
- `poly_walk` (`poly.rs:475-520`) already iterates `for (at, term) in
  terms.iter().enumerate()` — add `let last = terms.len().wrapping_sub(1);` and pass
  `tail && at == last` into each `poly_term` call, exactly mirroring
  `check_terms_relaxed`'s existing `tail && i == last`.
- `poly_walk_arms` (`poly.rs:1798-1850+`) calls `poly_walk(&body, arm.input, ...)` once
  per arm (`poly.rs:1837`) with no tail argument today; thread the caller's `tail`
  straight into that call, mirroring `inline_combinator`'s pass-through of `tail` into
  each concrete arm's `check_terms_relaxed`. `poly_combinator_call` (the `if`/`times`
  dispatch itself, `poly.rs:2148`) takes the same new `tail` parameter and forwards it
  unchanged into `poly_walk_arms`.
- `poly_call_term`'s self-call arm (`poly.rs:1297`) gates the new scan (1b below) on
  its own received `tail` parameter directly — no separate `self_tail`-vs-per-term
  distinction needed beyond what's already there: `has_self_tail_call(word,
  combinators)` (computed once in `check_poly_body`, via
  `combinators.tail()` — `check_poly_body` receives `combinators: &CombinatorEnv`,
  not `&CombinatorIndex`; `CombinatorEnv::tail()` (`combinators.rs:30`) is the accessor
  that produces the `&CombinatorIndex` `has_self_tail_call` needs) decides whether this
  *word* ever back-edges at all, and the per-call `tail` flag (threaded per 1a) decides
  whether *this* self-call is the one back-edge site. Do not read from `poly.rs:420`'s
  dead `&CombinatorIndex::new()` — that call site is unrelated (it feeds
  `Ctx::Word.self_tail_call`, which nothing in `poly.rs` reads) and must not be reused
  as the source of this new flag.

#### 1b. Linear value stranded below the self-call's argument window

Add a poly analogue of `check_linear_across_back_edge` (`terms.rs:1058`, not
`1041-1075` as an earlier draft of this section cited), adapted to `PolySlot` / the
poly stack representation. Location: the poly self-call arm at `poly.rs:1297`. Gate on
`tail && has_self_tail_call(word, combinators)` (1a). Before truncating `stack[..base]`,
scan the stranded slots (everything below the `n`-wide argument window the self-call
consumes) for a linear `PolyType`, using the poly linearity predicate (`poly_is_copy`
over `sig`, `structs`, `enums`, `arrays`). If one is found, return a located rejection:
the poly-side analogue of `linear_across_back_edge_error`, naming the word, the
stranded type, and the self-tail callee, wording mirroring the concrete message
("linear values across a loop are not supported yet ... consume it before the
recursive call"). The argument-window suffix is *forwarded into* the call, not live
across the edge, so it stays legal — only slots below the window are the hazard.

**Scope correction on `frame_floor`:** the concrete guard's `frame_floor` parameter is
`Some` at a *spliced* self-tail combinator site and `None` at the *whole-word* TCO
site (`terms.rs:1041-1056`'s doc). `loopg`'s self-call is spliced through `if`, so on
the concrete side this is the `frame_floor = Some(..)` shape, not the whole-word one —
an earlier draft of this section named the wrong site. This distinction only matters
for the concrete guard's *second* clause (an unconsumed linear **local** bound below
the floor, not a stack-stranded value); it does not affect 1b's stack-stranded check,
which fires identically either way. **Whether that second clause needs its own poly
port is a required Phase 1 sub-task, not assumed here**: the poly checker already
rejects any unconsumed linear local at end-of-scope via its own general local-tracking
(`poly_arm_local_not_consumed_error` or equivalent) independent of self-tail-call
location, which may already subsume it. Confirm with a probe (a linear local bound
inside a self-tail `if` arm, left unconsumed past the recursive call) before deciding
whether 1b needs a second clause or the existing general check already covers it; do
not silently drop this without checking.

#### 1c. Reference derived from a local, passed as a back-edge argument (new finding, unresolved — required Phase 1 investigation)

The concrete checker runs a **second** guard at the identical gate
(`tail && ctx.mangled_name() == Some(name) && ctx.is_self_tail_call()`,
`terms.rs:822-828`): `check_reference_across_back_edge` (`check.rs:1548-1568`), which
rejects a reference-typed *argument to the call itself* (not a stranded value —
`stack[base..]`, the args, not `stack[..base]`) whose `Deriv::owned_root` is a local of
this frame: the loop header rebinds locals each iteration, so a reference derived from
one would alias a reused slot. An earlier draft of this spec ported only 1b and
omitted this sibling entirely.

**This cannot be ported mechanically the way 1b can, and is left as an open
investigation for whoever implements Phase 1, not resolved here.** `PolySlot`
(`poly.rs:116-127`) carries only `pt`, `int_val`, `quot` — no `deriv`/`owned_root`
equivalent at all; the poly walk's only borrow-provenance tracking found
(`PolyBorrow`, `poly.rs:102-111`: `place`/`mutable`/`span`) is a side-table
(`arm_borrows` in `poly_walk_arms`) unrelated to a stack slot's own derivation history.
So `check_reference_across_back_edge`'s exact mechanism has no poly-side data to read.
Before writing any guard here, **probe whether the hazard is even constructible**: does
a poly self-tail word's declared signature admit a concrete reference-typed parameter
(recall the standing note that a poly body's *local* borrows already work for concrete
types, per prior P7 slices), and if so, can a reference derived from a local bound
*inside* the loop body be passed into that parameter at the recursive call? If
unreachable today (e.g., because nothing in the poly walk currently lets a call
argument be a borrow of a same-body local at all), state that plainly, cite the check
that forecloses it, and this sub-piece is a no-op. If reachable, it needs its own
guard built from whatever borrow-tracking the poly walk does carry (`PolyBorrow`'s
`place` is the closest available handle), not a literal port of `check.rs:1548`'s
`Deriv`-based one.

Both 1b and 1c are **located rejections**, not disposal/soundness-repair support: no
back-edge destructor and no aliasing-safe representation is built, exactly as the
concrete guards defer both.

Probe program that must be rejected after this phase (open question 1, verified clean
at HEAD):

```sooth
type: Spy tag i64 ;
: drop ( Spy -- ) | s | s Spy> drop ;
: iszero ( i64 -- Bool ) 0 eq ;
: loopg ( Spy 'T: Copy i64 -- Spy 'T )
  dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;
```

### Piece 2 — compute `self_tail` at the two `lower_word_parts` call sites (lowering plumbing)

- **Native** (`driver.rs:262-278`, the poly-instantiation monomorphization loop):
  replace the hardcoded `false` with `has_self_tail_call(word, combinator_bodies)`.
  `combinator_bodies` (the real populated `CombinatorIndex`) is already in scope,
  threaded to `lower_word_parts` as its `combinators` argument on the next line — it
  is exactly `has_self_tail_call`'s second argument. `word` is the full `WordDef`.
- **REPL** (`driver.rs:816-...`, `lower_instantiation`; sole caller `repl.rs:1554`):
  compute `has_self_tail_call(&entry.word, &bodies)` at the caller in `repl.rs` (which
  holds `entry.word`, the full retained `WordDef`, and already passes
  `&entry.word.body`), and pass the result in — matching how the native path computes
  it inline. No new plumbing into `lower_instantiation`'s own tail-detection is needed
  beyond passing the already-computed bool.

Also in this piece: **rewrite the stale comment** at `driver.rs:264-272` to name the
real, still-valid reason `self_tail` was previously `false` (D3's now-being-closed
lowering deferral), not the disproven `CallInst`-lookup claim.

### Piece 3 — back-edge dispatch inside the poly self-call arm (lowering)

Inside the existing poly self-call arm (`calls.rs:667-687`, gated on
`self.cur_poly_callee`), ahead of its `emit_user_call` fallback, add:

```text
if tail && self.header.is_some() {
    // arity from self.cur_poly_callee (this instantiation's own effect),
    // NOT self.env.get(name) which panics on a poly name.
    // pop the args as the back-edge phi operands, materialize any phantom
    // quotation args (R-D3), push the back-edge, seal with Jmp(header).
}
```

Reuse the same `materialize_quot_args` / `self.back_edges.push` / `Terminator::Jmp`
sequence R7 already runs, sourcing `in_arity` (and any `quot_inputs`) from
`self.cur_poly_callee`'s stored arity rather than the `env` lookup. When `tail` is
false or `self.header` is `None`, fall through to the existing `emit_user_call`
unchanged. `begin_loop` needs no change: it is already driven by
`lower_word_parts` whenever the threaded `self_tail` is true.

### θ-substitution non-interaction (open question 3)

A single instantiation is lowered under one fixed θ substitution for its whole body.
The loop-carried types are the current instantiation's own concrete SSA values, and
the self-call's structural match (`poly.rs:1297`) requires the recursive call's
operand window to structurally equal `sig.inputs` — so a self-call recursing at a
different type argument is an ordinary mismatch, not a new instantiation. Therefore no
loop-carried type can differ from one iteration to the next within one self-tail poly
loop. This is a one-line confirmation, not new work; the spec records it and no code
depends on re-deriving it. (No open sub-question remains here.)

## Test plan (per CLAUDE.md conventions)

Unit tests beside the stage code, `thing_condition_expected` naming, diagnostics
tested by exact message.

### Checker (Phase 1)

- `poly_self_tail_linear_stranded_below_call_window_is_error` — the `Spy`/`loopg`
  program above is rejected, asserting the exact located message (word name, stranded
  `Spy`, self-tail callee). Beside `poly.rs`.
- `poly_self_tail_linear_forwarded_into_call_window_is_ok` — a linear value *moved
  into* the recursive call's argument window (forwarded, not stranded) stays legal.
- `poly_non_tail_self_call_with_linear_below_is_ok` — a non-tail-position self-call
  in a self-tail word does not trigger the guard (it lowers as an ordinary call).
  Mutation-guard: this test must fail if the tail gating is dropped.
- Existing `loopg` (`'T: Copy`, no linear) still typechecks clean — the guard fires
  only on a genuinely linear stranded slot.
- `poly_self_tail_unconsumed_linear_local_in_arm_is_error` (or a note explaining why
  an existing test already covers it) — resolving 1b's `frame_floor`/second-clause
  question: a linear local bound inside a self-tail `if` arm, left unconsumed past the
  recursive call, must be rejected either by this guard's own second clause or by an
  identified pre-existing general check. Do not skip writing this probe.
- Piece 1c's outcome, either way: if the reference-across-back-edge hazard is
  reachable, a `poly_self_tail_reference_to_local_across_back_edge_is_error` test
  (or equivalent); if unreachable, a short comment at the investigation site citing
  what forecloses it, and no test is required for a shape that cannot be constructed.

### Lowering (Phase 2/3)

- `poly_self_tail_call_lowers_to_loop_back_edge` — `loopg` lowers with a loop header
  and a back-edge `Jmp` for the recursive call (assert against the IR: a header block
  and a `back_edges` entry / `Jmp(header)` terminator), not an `Instr::Call` to the
  instantiation symbol. Poly instantiation is unobservable at runtime, so this asserts
  IR shape, not program output.
- `poly_non_tail_self_call_lowers_to_ordinary_recursive_call` — the migrated negative
  regression (see open question 2): a *non*-tail-position self-call still declines the
  loop and lowers as `emit_user_call`.

### Golden (exit criteria)

- **The golden**: `loopg` compiled and run over a large counter (large enough that the
  old stack-consuming lowering would blow or visibly deepen the stack), demonstrating
  constant-stack behavior — the roadmap's framed exit "a generic countdown over a
  large counter runs in constant stack". Source-in → expected-output golden.
- **Regression**: the existing S3g golden and mangled-name/mismatch tests keep passing
  unchanged, **with one correction from round-1 review**: only
  `poly_self_call_lowers_to_ordinary_recursive_call` (`src/ir/driver.rs`) actually
  asserts the *absence* of a loop header, and only that one test migrates to a
  non-tail-self-call fixture (becoming the negative regression above). The
  `b49ef63` change to `tests/phase7_slice3g.rs` was a doc-comment edit on
  `self_recursive_poly_word_runs_to_base_case`, a behavioral run-and-assert-stdout
  golden with **no header assertion** — `loopg` keeps producing the identical
  transcript once it lowers to a loop, so this test is not migrated; it is *extended*
  into the large-counter constant-stack golden below (verified against `git show
  b49ef63 -- tests/phase7_slice3g.rs`; an earlier draft of this spec miscounted this
  as two tests needing migration).
- **Rejection golden**: the `Spy`/`loopg` program produces the poly-side
  linear-across-back-edge located diagnostic (source-in → expected-diagnostic-out).

## Out of scope

- The self-call check itself (S3g, shipped) — untouched.
- P7.S3k (generic-calls-generic) — a self-tail loop is specific to a call to the word
  being lowered; no interaction traced.
- Any change to `resolve::mangle`, `env`, or the `instantiations`/`CallInst`
  machinery. This slice is confined to `has_self_tail_call`'s two call sites
  (`driver.rs`, `repl.rs`), the poly self-call arm in `func_builder/calls.rs`, and the
  new guard in `check/poly.rs`.
- Back-edge destructor disposal for a linear loop-carried value (concrete side defers
  it too; this slice only locates and rejects the shape).

## Open questions

One genuinely open item, deliberately left for Phase 1 rather than guessed here:
**piece 1c** (does a poly self-tail body ever admit a reference-to-a-local as a
back-edge argument, and if so what guard fits `PolySlot`'s representation) is an
explicit required investigation, not a locked design — see "Design, Piece 1c" above.
Everything else is resolved: open question 1's core hazard (linear-stranded) is
specified as Phase 1's 1b; the `frame_floor` second clause is scoped as a Phase 1
probe task (1b); open question 2 (test migration) is corrected to exactly one test,
not two, per round-1 review; open question 3 (θ non-interaction) needs no code.

## Phased delivery plan

Sequenced so the checker guards land before the loop transform can reach an
unguarded hazard. Phase 1 grew a third sub-piece (1c) after round-1 review; it stays
one phase since 1a/1b/1c share the same self-call-arm location and tail-plumbing
prerequisite.

- **Phase 1 — tail-position plumbing plus both poly-side back-edge guards.** 1a:
  thread `tail: bool` through `poly_walk`/`poly_term`/`poly_call_term`/
  `poly_walk_arms`/`poly_combinator_call` (net-new parameter on all five, per the
  concrete-precedent design above — this is real, nontrivial checker-walk work, not a
  one-line change). 1b: the linear-stranded-below-window located rejection, plus
  resolving the `frame_floor` second-clause question with a probe. 1c: the
  reference-across-back-edge investigation — resolve whether the hazard is reachable
  before deciding whether it needs a guard; either outcome is an acceptable Phase 1
  exit, but the investigation itself is not optional. Exit: the `Spy`/`loopg` program
  is rejected with the exact analogue message; the forwarded-linear and non-tail-self
  cases stay legal; existing `loopg` still typechecks; 1c's reachability question is
  answered and (if reachable) guarded.
- **Phase 2 — compute `self_tail` and add the back-edge dispatch.** Thread
  `has_self_tail_call` into the two `lower_word_parts` call sites (`driver.rs`,
  `repl.rs`, via `CombinatorEnv::tail()` where a `CombinatorIndex` is needed); add the
  back-edge branch inside the poly self-call arm in `func_builder/calls.rs` sourcing
  arity from `self.cur_poly_callee`; rewrite the stale `driver.rs:264-272` comment.
  Exit: `loopg` lowers to a loop header + back-edge (IR assertion) and the
  constant-stack golden runs.
- **Phase 3 — test migration and regressions.** Migrate exactly one existing test
  (`poly_self_call_lowers_to_ordinary_recursive_call`, `driver.rs`) onto a
  non-tail-self-call fixture (it becomes the negative regression); extend
  `self_recursive_poly_word_runs_to_base_case` (`tests/phase7_slice3g.rs`) into the
  large-counter constant-stack golden rather than migrating it (round-1 review
  correction — it asserts no header today and needs none migrated away); confirm every
  other existing S3g golden/mangled-name/mismatch test passes unchanged; record the θ
  non-interaction note. Exit: full suite green, the rejection golden and the
  constant-stack golden both present.

```json
{
  "phases": [
    { "phase": 1, "focus": "Thread tail-position through poly_walk/poly_term/poly_call_term/poly_walk_arms/poly_combinator_call; add the poly-side linear-stranded-below-window guard gated on has_self_tail_call and tail; investigate and resolve the reference-across-back-edge (1c) reachability question", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "Compute self_tail at both lower_word_parts call sites and add the poly self-call back-edge dispatch in func_builder/calls.rs; fix the stale driver.rs comment", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "Migrate the one true S3g absence-of-header test to a non-tail fixture, extend the existing loopg run golden into the constant-stack golden, confirm all other existing goldens pass, add the rejection golden", "effort": "S", "difficulty": "standard" }
  ]
}
```
