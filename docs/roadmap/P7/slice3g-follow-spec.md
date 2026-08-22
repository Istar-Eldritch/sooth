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

The slice has three pieces, located by the brief's recon, corrected/completed by a
round-1 review pass, and re-scoped by Phase 1's own probes (the concrete precedent of
"locate-and-reject, don't yet implement back-edge disposal" is mirrored throughout; the
guard the brief scoped is not the one that was needed — see "Design, Piece 1"):

1. **A checker-side guard, larger than first scoped**: net-new tail-position threading
   through the poly walker (`poly_walk`/`poly_term`/`poly_call_term`/
   `poly_walk_arms`/`poly_combinator_call` carry no tail state today), plus a poly-side
   guard on what may cross the back-edge. Phase 1 resolved *which* guard: the linear
   hazard the brief chased needs none (unreachable, or already rejected — see 1b), and
   the sibling `check_reference_across_back_edge` hazard is the reachable one, silent at
   HEAD (see 1c). The guard is a *located rejection*, mirroring the concrete guards,
   which are themselves still only located rejections, not full back-edge disposal.
2. **Two small plumbing changes** at the `lower_word_parts` call sites (native
   `driver.rs`, REPL `lower_instantiation`), each of which already holds everything
   `has_self_tail_call` needs and hardcodes `false`.
3. **A back-edge dispatch branch** inside the existing poly self-call arm in
   `src/ir/func_builder/calls.rs`, sourcing arity from `self.cur_poly_callee` rather
   than `self.env.get(name)` (which panics on a poly name).

The checker guard (piece 1) must land **before** the lowering pieces (2, 3), so the
loop transform never reaches an unguarded program: a reference to a local of the frame
riding the back-edge into a header that rebinds that local.

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

**Phase 1 outcome:** that second sibling guard (1c) is the whole of the checker work;
the guard the brief scoped (1b) is not needed at all. Both sub-sections below are
rewritten to what was probed and delivered.

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
- `poly_walk_arms` (`poly.rs:1798-1850+`) walks each arm body; arm tail-ness is *per
  arm*, exactly as the concrete `LiteralBoundary::is_arm` is (`if`'s arms inherit the
  call's tail position, `times`' body never does), so it rides `PolyArm.tail` rather
  than a parameter of `poly_walk_arms` itself. `poly_combinator_call` sets it from the
  shared accessor the concrete argument site uses (`tail_called_param_slots`), and
  `poly_eliminator_call` sets it from its own `tail` for every arm (an eliminator arm
  runs at most once, in place, in the call's position). Both take the new `tail`
  parameter; `poly_ground_quotation_literal`'s walk takes `false` (a materialized
  quotation argument is not spliced in place), matching the concrete non-arm boundary.
- `poly_call_term`'s self-call arm (`poly.rs:1297`) gates the guard on its own received
  `tail` **and** `ctx.is_self_tail_call()`, character for character the concrete gate at
  `terms.rs:823`. **Deviation from an earlier draft of this section**, which forbade
  reading `ctx` and asked for a second threaded bool: `word_ctx`'s `combs` argument
  feeds nothing but `self_tail_call`, `ctx.is_self_tail_call()` is read nowhere else
  reachable from the poly walk (`poly.rs` never enters `check_terms`), and
  `check_poly_body` already holds the `CombinatorEnv`. So passing `combinators.tail()`
  where the dead `&CombinatorIndex::new()` was costs one line, deletes a stale comment
  that claimed the empty index was *correct*, and spares a second bare `bool` at five
  14-argument call sites. The two halves are pinned separately:
  `poly_non_tail_self_call_carrying_a_local_reference_is_ok` fails if the per-term half
  is dropped, and `poly_self_tail_call_in_a_builtin_named_word_skips_the_back_edge_guard`
  if the word-level half is (`has_self_tail_call` refuses every builtin spelling, so a
  generic `lt` gets no loop header however its body is written).

#### 1b. Linear value across the back-edge — resolved in Phase 1: no guard needed

Phase 1 probed both clauses of the concrete `check_linear_across_back_edge`
(`terms.rs:1058`) against the poly walk. Neither needs a port, and the spec's original
probe program was misread.

**The stack-stranded clause is unreachable in a generic body.** A tail self-call is the
last term of a context whose exit row *is* the word's declared outputs (the body
residual, or an `if`/eliminator arm's exit, which the call's exit then is), and the
self-call pushes exactly `sig.outputs`. So `stranded ++ outputs == outputs`, which forces
`stranded` empty. A generic body also cannot reach the shape from below the way an inline
combinator can: `check_poly_body` seeds the walk stack at `sig.inputs`, so there is no
caller row underneath. Written, the shape presents as the two arms disagreeing on their
exit row, which is what
`poly_self_tail_linear_stranded_below_the_call_window_is_not_well_typed` pins — a
tripwire, so that if the exit-row rule ever loosens, the clause has to be written.

**The unconsumed-linear-local clause is already rejected**, by the general
end-of-body/arm local tracking (`poly_local_unconsumed_error` /
`poly_arm_local_not_consumed_error`), with `error: linear value `s` is never consumed`.
The concrete clause's own doc says that clause is not what makes disposal safe — its only
job is to *locate* the same rejection at the back-edge — so a second poly rule that only
relocates a message is not worth the second rule.
`poly_self_tail_unconsumed_linear_local_is_error` pins the subsumption.

**The spec's own probe program is legal, not a rejection.** In

```sooth
: loopg ( Spy 'T: Copy i64 -- Spy 'T )
  dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;
```

the self-call's declared window is all three inputs, so the `Spy` is *moved into* the
call and forwarded as a back-edge operand — the concrete guard's own accept case. It
stays clean, pinned by `poly_self_tail_linear_forwarded_into_the_call_window_is_ok`;
open question 1's premise (a linear value stranded *below* the window) does not hold of
it. **Phase 3 must not add a rejection golden for this program.**

#### 1c. Reference derived from a local, passed as a back-edge argument — reachable, guarded in Phase 1

The concrete checker runs a **second** guard at the identical gate
(`tail && ctx.mangled_name() == Some(name) && ctx.is_self_tail_call()`,
`terms.rs:822-828`): `check_reference_across_back_edge` (`check.rs:1548-1568`), which
rejects a reference-typed *argument to the call itself* (`stack[base..]`, the args, not
`stack[..base]`) whose `Deriv::owned_root` is a local of this frame: the loop header
rebinds locals each iteration, so a reference derived from one would alias a reused slot.

**Phase 1 finding: the hazard is reachable, and was silent.** One shape reaches it, and
it is the shape both representations allow to meet:

```sooth
: iszero ( i64 -- Bool ) 0 eq ;
: loopg ( 'T: Copy &!['T 4] ['T 4] ['T 4] i64 -- i64 )
  | r a b n |
  n iszero ~[ drop r drop 0 ] ~[ r drop &!a b dup n 1 sub loopg ] if ;
```

A poly-body borrow always yields `PolyType::Ref(..)`, while a fully concrete `&!Cell`
parameter folds to `Concrete(Type::Ref(..))` at parse time, and the self-call's pointwise
match never equates the two (it reports the memorable `expected &!Cell, found &!Cell`).
So the referent has to stay variable-bearing, and an *array* local is then the only
borrowable one a generic body admits (a bare `'T` might instantiate to a scalar; a
`Generic` application is not on the borrowable list). Above, `&!a` crosses the back-edge
and typechecked clean at HEAD, while the monomorphic twin of the same body was already
rejected.

**The guard** (`check_poly_reference_across_back_edge`) is not a literal port: `PolySlot`
carries no `Deriv`, so nothing traces *which* argument a recorded borrow flowed into. It
is the conjunction the available data supports — a reference among the call's arguments,
and a live `PolyScope::borrows` record whose place is a **local** (a static's
data-segment storage survives every iteration, the concrete R3 exemption). A reference
*parameter* is exempt for free: a body that borrows nothing records nothing. The rule can
therefore reject a program the concrete side accepts (a dead local borrow beside a
forwarded parameter reference), which is the coarseness `prune_dead_borrows` already
documents and the message states (`POLY_BORROW_LIVENESS_NOTE`).

Both 1b and 1c are **located rejections**, not disposal/soundness-repair support: no
back-edge destructor and no aliasing-safe representation is built, exactly as the
concrete guards defer both.

**One conjunct is unwitnessable by a source program**: the per-arm refinement in
`poly_combinator_call` (`tail && tail_slots.contains(&i)`, so `times`' body does not
inherit tail position while `if`'s arms do). Kept for lockstep with
`tail_position_calls`/`lower_terms`, and its cost of being unpinned is recorded at the
site: the coarse borrow liveness rejects any body that borrows a local in *one* arm and
tail-recurses with a reference argument in *another*, so no program can tell which arm
the borrow sat in.

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

**Phase 1 note for whoever does this:** the checker's word-level gate now reads
`has_self_tail_call` off the same `CombinatorEnv` each path already passes to
`check_poly_body` (`combinator_bodies` natively, `checker_combinators(&self.combinators)`
at the REPL). Piece 2's `self_tail` must come from an index of the same contents on both
paths, or the guard and the transform disagree about whether a word back-edges: a splice
the checker's index cannot follow (an `if` missing from a session's retained combinators,
say) makes the checker treat the self-call as ordinary recursion, and lowering must reach
the same conclusion or it back-edges past an unrun guard.

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

### Checker (Phase 1) — as delivered

All beside `poly.rs`, all built from one fixture helper (`self_tail_ref_loop`) whose doc
records why `&!['T 4]` is the only reference parameter a body borrow can match.

- `poly_self_tail_reference_to_a_local_across_the_back_edge_is_error` — 1c's hazard,
  exact message.
- `poly_self_tail_reference_parameter_forwarded_across_the_back_edge_is_ok` — the
  incoming reference parameter still crosses freely.
- `poly_non_tail_self_call_carrying_a_local_reference_is_ok` — the per-term half of the
  gate; the word's real back-edge sits in the sibling arm, so only `tail` tells the two
  self-calls apart.
- `poly_self_tail_call_in_a_builtin_named_word_skips_the_back_edge_guard` — the
  word-level half.
- `poly_self_tail_reference_rooted_in_a_static_is_ok` — the static exemption.
- `poly_self_tail_call_with_no_reference_argument_ignores_a_live_local_borrow` — the
  rule's other precondition: a live local borrow alone is not a hazard.
- `poly_self_tail_linear_forwarded_into_the_call_window_is_ok` — the spec's original
  probe program, which is this accept case and not a rejection.
- `poly_self_tail_unconsumed_linear_local_is_error` and
  `poly_self_tail_linear_stranded_below_the_call_window_is_not_well_typed` — the two
  reasons 1b needs no guard, pinned rather than asserted in prose.
- The existing `loopg` goldens still typecheck clean (`--lib` and the S3g suite).

Every clause was mutation-tested: deleting the guard call, either half of the gate, the
reference precondition, the locals filter, scanning `stack[..base]` instead of the args,
and dropping `tail && at == last` are each killed by a named test above. The one
surviving mutation (the per-arm `tail_slots` refinement) is documented at its site as
unwitnessable, with the reason.

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
- **Rejection golden**: the `&!['T 4]` program of 1c produces the poly-side
  reference-across-back-edge located diagnostic (source-in → expected-diagnostic-out).
  *Not* the `Spy`/`loopg` program: that one is legal and stays legal (see 1b).

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

All resolved. Phase 1 answered the two it left open, and both answers moved work:
**1c** (the reference-across-back-edge hazard) is reachable and is what Phase 1 guards;
**1b** (the linear hazard, open question 1) needs no guard at all — its stack-stranded
clause cannot be well-typed in a generic body and its unconsumed-local clause is already
rejected, and the program the brief offered as the witness is the *forwarded* accept
case, so **Phase 3 must not add a linear rejection golden for it**. Open question 2 (test
migration) stands as corrected to exactly one test; open question 3 (θ non-interaction)
needs no code.

## Phased delivery plan

Sequenced so the checker guards land before the loop transform can reach an
unguarded hazard. Phase 1 grew a third sub-piece (1c) after round-1 review; it stays
one phase since 1a/1b/1c share the same self-call-arm location and tail-plumbing
prerequisite.

- **Phase 1 (delivered) — tail-position plumbing plus the poly-side back-edge guard.**
  1a: `tail: bool` threaded through `poly_walk`/`poly_term`/`poly_call_term`/
  `poly_combinator_call`/`poly_eliminator_call`, per-arm via `PolyArm.tail`, and the
  word-level half read off `ctx.is_self_tail_call()` with the dead empty combinator
  index in `check_poly_body` fixed. 1b: no guard — both clauses are unreachable or
  already rejected, and the spec's probe program turned out to be the accept case. 1c:
  the hazard is reachable and was silent; `check_poly_reference_across_back_edge` is the
  rejection. Exit met: the `&!['T 4]` program is rejected with a located message, the
  forwarded-reference/forwarded-linear/non-tail/builtin-named/static cases stay legal,
  and existing `loopg` still typechecks.
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
    { "phase": 1, "focus": "Thread tail-position through poly_walk/poly_term/poly_call_term/poly_walk_arms/poly_combinator_call; guard the poly self-tail back-edge against a reference derived from a local (1c, the reachable hazard) gated on has_self_tail_call and tail; record why the linear hazard (1b) needs no guard", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "Compute self_tail at both lower_word_parts call sites and add the poly self-call back-edge dispatch in func_builder/calls.rs; fix the stale driver.rs comment", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "Migrate the one true S3g absence-of-header test to a non-tail fixture, extend the existing loopg run golden into the constant-stack golden, confirm all other existing goldens pass, add the rejection golden", "effort": "S", "difficulty": "standard" }
  ]
}
```
