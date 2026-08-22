# P7.S3g-follow: the self-tail loop transform for a polymorphic body

## Summary

S3g lowered a self-call inside a non-inline generic body to an ordinary recursive
`Instr::Call` (D3, deliberately deferred): correct, but one real stack frame per
recursion level, where a monomorphic self-tail word lowers to a loop back-edge. This
slice closes that gap. Target program, S3g's own golden, whose recursive call is in
tail position (last term of the `if` combinator's recursive-arm quotation) yet got
`self_tail: false` unconditionally:

```sooth
: iszero ( i64 -- Bool ) 0 eq ;
: loopg ( 'T: Copy i64 -- 'T )
  dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;
```

Three pieces, delivered in this order so the checker guard lands before the loop
transform can reach an unguarded program (a reference to a frame-local riding the
back-edge into a header that rebinds that local):

1. **Checker**: tail-position threading through the poly walker (which carried no tail
   state), plus one located rejection on what may cross the back-edge. The reachable
   hazard is the reference one (1c); the linear hazard the brief scoped needs no guard
   (1b).
2. **Lowering plumbing**: compute `self_tail` from `has_self_tail_call` at the
   `lower_word_parts` call sites that hardcoded `false`.
3. **Lowering**: a back-edge dispatch branch in the poly self-call arm of
   `src/ir/func_builder/calls.rs`, sourcing arity from `self.cur_poly_callee`.

## Load-bearing facts

- `has_self_tail_call` / `tail_position_calls` (`src/check/drop_graph.rs`) is a purely
  syntactic name-walk and is already poly-aware (`declared_input_count` branches on
  `word.poly`). It returns `true` for `loopg` through the `if` splice with no change to
  `poly.rs`: no new tail-*detection* machinery was needed.
- The poly self-call arm in the checker (`poly.rs`, `ctx.mangled_name() == Some(name)`)
  is a structural pointwise match against `sig.inputs`/`sig.outputs` (S3g D1). It fires
  regardless of tail position and runs `stack.truncate(base)`, keeping everything below
  the argument window.
- The lowering poly self-call arm is gated on `self.cur_poly_callee` and runs *before*
  R7, which is structurally unreachable from a poly self-call (`name ==
  self.cur_word_name` never holds for a bare poly name, and its `self.env.get(name)`
  would panic).
- `self.cur_poly_callee` (`func_builder/mod.rs`) already stores this instantiation's
  `(callee, arity)` built from its own concrete `effect`: the correct phi/arg-count
  source.
- `begin_loop` inspects nothing about polymorphism, and `lower_word_parts` already
  calls it whenever `self_tail` is true. Threading a correct `self_tail` builds a
  working header with no change to `begin_loop`.
- The old `driver.rs` comment blamed `has_self_tail_call` for "only recognizing a
  plain-name `Call`, never a `CallInst` lookup". Disproven; the real reason was D3's
  lowering deferral. Rewritten.

## Locked decisions carried from S3g (untouched)

- The self-call check stays structural and non-unifying (S3g finding 5/9). This slice
  guards and lowers an already-typechecked call; no new type check, no interaction with
  the termination argument.
- An ordinary recursive `Instr::Call` remains the fallback (S3g finding 7). A non-tail
  self-call, or a word with no header, lowers exactly as S3g shipped it; this slice only
  adds a branch ahead of that fallback.

## Design

### Piece 1a: tail-position threading through the poly walk

`tail: bool` added to `poly_walk`, `poly_term`, `poly_call_term`,
`poly_combinator_call`, and `poly_eliminator_call`, mirroring the concrete precedent,
which is a plain parameter computed per term as `tail && i == last` in
`check_terms_relaxed`, not `Ctx`-carried state. `poly_walk` passes `tail && at == last`
into each `poly_term`.

Arm tail-ness is *per arm* (as `LiteralBoundary::is_arm` is concretely: `if`'s arms
inherit the call's tail position, `times`' body never does), so it rides `PolyArm.tail`
rather than a parameter of `poly_walk_arms`. `poly_combinator_call` sets it from the
shared `tail_called_param_slots` accessor; `poly_eliminator_call` sets it from its own
`tail` for every arm (an eliminator arm runs at most once, in place, in the call's
position); `poly_ground_quotation_literal`'s walk takes `false` (a materialized
quotation argument is not spliced in place).

The self-call arm gates the guard on its received `tail` **and**
`ctx.is_self_tail_call()`, character for character the concrete gate. Reading `ctx` is
safe and cheaper than a second threaded bool: `word_ctx`'s `combs` argument feeds
nothing but `self_tail_call`, nothing else reachable from the poly walk reads it, and
`check_poly_body` already holds the `CombinatorEnv`, so passing `combinators.tail()`
where a dead `&CombinatorIndex::new()` sat costs one line and spares a bare `bool` at
five 14-argument call sites. The two halves are pinned separately:
`poly_non_tail_self_call_carrying_a_local_reference_is_ok` fails if the per-term half is
dropped, `poly_self_tail_call_in_a_builtin_named_word_skips_the_back_edge_guard` if the
word-level half is.

### Piece 1b: linear value across the back-edge, no guard needed

Both clauses of the concrete `check_linear_across_back_edge` were probed against the
poly walk; neither needs a port.

**The stack-stranded clause cannot be well-typed in a generic body.** A tail self-call
is the last term of a context whose exit row *is* the word's declared outputs, and the
call pushes exactly `sig.outputs`, so `stranded ++ outputs == outputs` forces `stranded`
empty. `check_poly_body` seeds the walk at `sig.inputs`, so unlike an inline combinator
there is no caller row underneath. Written out, the shape presents as two arms
disagreeing on their exit row, pinned as a tripwire by
`poly_self_tail_linear_stranded_below_the_call_window_is_not_well_typed`: if the
exit-row rule ever loosens, the clause has to be written.

**The unconsumed-linear-local clause is already rejected** by general end-of-body/arm
local tracking (`poly_local_unconsumed_error` / `poly_arm_local_not_consumed_error`).
The concrete clause's own doc says it makes nothing safe, it only relocates the same
rejection to the back-edge, so a second poly rule that only relocates a message is not
worth it. Pinned by `poly_self_tail_unconsumed_linear_local_is_error`.

**The brief's probe program is legal, not a rejection.** In `: loopg ( Spy 'T: Copy i64
-- Spy 'T ) dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;` the declared window is all
three inputs, so the `Spy` is moved *into* the call and forwarded as a back-edge
operand: the concrete guard's own accept case, pinned by
`poly_self_tail_linear_forwarded_into_the_call_window_is_ok`. No rejection golden exists
for it.

### Piece 1c: reference derived from a local as a back-edge argument (the reachable hazard)

The concrete side runs a second guard at the identical gate,
`check_reference_across_back_edge`, rejecting a reference-typed *argument to the call*
(`stack[base..]`, not `stack[..base]`) whose `Deriv::owned_root` is a local of this
frame: the header rebinds locals each iteration, so such a reference would alias a
reused slot. The hazard is reachable in a generic body and was silent:

```sooth
: iszero ( i64 -- Bool ) 0 eq ;
: loopg ( 'T: Copy &!['T 4] ['T 4] ['T 4] i64 -- i64 )
  | r a b n |
  n iszero ~[ drop r drop 0 ] ~[ r drop &!a b dup n 1 sub loopg ] if ;
```

A body borrow always yields `PolyType::Ref(..)`, while a fully concrete `&!Cell`
parameter folds to `Concrete(Type::Ref(..))` at parse time, and the pointwise self-call
match never equates the two (it reports the memorable `expected &!Cell, found &!Cell`).
So the referent must stay variable-bearing, and an *array* local is then the only
borrowable one a generic body admits (a bare `'T` might instantiate to a scalar; a
`Generic` application is not borrowable). `&!a` above crossed the back-edge clean at
HEAD while the monomorphic twin of the same body was already rejected.

**The guard** (`check_poly_reference_across_back_edge`) is not a literal port: `PolySlot`
carries no `Deriv`, so nothing traces which argument a recorded borrow flowed into. It is
the conjunction the available data supports: a reference among the call's arguments, and
a live `PolyScope::borrows` record not rooted in a static (the concrete R3 exemption: a
static's data-segment storage survives every iteration). A reference *parameter* is
exempt for free, since a body that borrows nothing records nothing.

Local-vs-static is decided at the **borrow site** and carried on the record
(`PolyBorrow::static_rooted`), never re-derived at the self-call. Two probed shapes make
a lookup there wrong: a borrow taken inside a `call`-splice or an eliminator arm outlives
the locals of the block that took it (each exit `retain`s enclosing locals while keeping
the borrow records, and `poly_walk_arms` unions each arm's borrows into the parent), so
`scope.locals` misses a real frame-local; and a local shadowing a static of the same name
resolves to the local at the borrow site but to the static under a name-keyed
`ctx.static_type` test. Either reading exempts a live hazard.

The rule is deliberately coarser than the concrete one: it can reject a program the
concrete side accepts (a dead local borrow beside a forwarded parameter reference). That
is the liveness coarseness `prune_dead_borrows` already documents, stated in the message
via `POLY_BORROW_LIVENESS_NOTE`, and pinned as a real rejection by
`poly_self_tail_dropped_borrow_then_forwarded_ref_is_over_conservative` rather than left
as prose.

Both 1b and 1c are **located rejections**: no back-edge destructor and no aliasing-safe
representation, exactly as the concrete guards defer both.

**One conjunct is unwitnessable by a source program**: the per-arm refinement in
`poly_combinator_call` (`tail && tail_slots.contains(&i)`, so `times`' body does not
inherit tail position while `if`'s arms do). Kept for lockstep with
`tail_position_calls`/`lower_terms`; its cost is the over-conservative rejection above.

### Piece 2: compute `self_tail` at the `lower_word_parts` call sites

- **Native poly instantiations** (`driver.rs`): `has_self_tail_call(word,
  &combinator_bodies)`, the same populated index already threaded to `lower_word_parts`.
- **Native monomorphic words** (`driver.rs`): the same predicate, additionally
  conjoined with `symbols[idx] == w.name`. A word sharing its name with another
  candidate is not self-tail-recursive on a bare name match, since the name in its body
  may resolve to the other candidate: the same reasoning that excludes builtin-named
  words inside `has_self_tail_call`.
- **Drop-override bodies** (`destructors.rs`): the real `combinator_bodies` index
  replaces `lower_word`'s hardcoded `empty_combinators()`, which was silently wrong here
  (an override body can call a combinator like any other body).
- **REPL** (`repl.rs`, sole caller of `lower_instantiation`): `has_self_tail_call(&entry.word,
  &bodies)` computed at the caller, which holds the retained `WordDef`, and passed in as
  a bool.

Both paths must derive `self_tail` from an index of the same contents the checker used
(`combinator_bodies` natively, `checker_combinators(&self.combinators)` at the REPL), or
guard and transform disagree about whether a word back-edges: a splice the checker's
index cannot follow makes it treat the self-call as ordinary recursion, and lowering must
reach the same conclusion or it back-edges past an unrun guard.

The stale `driver.rs` comment is rewritten to the real reason (`CallInst`/`env` hold no
poly self-name; the predicate is poly-aware and is now the shared source of truth).

### Piece 3: back-edge dispatch inside the poly self-call arm

Inside the poly self-call arm (gated on `self.cur_poly_callee`), ahead of the
`emit_user_call` fallback: when `tail` and `self.header.is_some()`, pop the args as
back-edge phi operands, materialize any phantom quotation args (R-D3), push the back-edge
and seal with `Jmp(header)`. `in_arity` and `quot_inputs` come from
`cur_poly_callee`'s stored arity (the effect that seeded the header phis), never
`self.env.get(name)`, which panics on a poly name. The sequence is factored into a shared
`emit_back_edge` helper with R7's. When `tail` is false or there is no header, the
existing `emit_user_call` runs unchanged; `begin_loop` needed no change.

### θ-substitution non-interaction

One instantiation is lowered under one fixed θ for its whole body, and the structural
self-call match requires the recursive operand window to equal `sig.inputs`, so a
self-call at a different type argument is an ordinary mismatch, not a new instantiation.
No loop-carried type can differ between iterations of one self-tail poly loop. No code
depends on re-deriving this.

## Tests

### Checker (`src/check/poly.rs`)

All built from one fixture helper (`self_tail_ref_loop`) whose doc records why `&!['T 4]`
is the only reference parameter a body borrow can match.

- `poly_self_tail_reference_to_a_local_across_the_back_edge_is_error`: 1c's hazard, exact
  message; `..._through_an_eliminator_arm_is_error` reaches it via
  `poly_eliminator_call`'s per-arm `tail` (`Bool?` swapped in for `if`).
- `..._rooted_in_a_spliced_block_local_is_error` and
  `..._rooted_in_a_local_shadowing_a_static_is_error`: the two shapes forcing
  `static_rooted` to be recorded at the borrow site. Each is accepted by exactly one of
  the two wrong lookup formulations and by neither real clause.
- `poly_self_tail_reference_rooted_in_a_static_is_ok`,
  `poly_self_tail_reference_parameter_forwarded_across_the_back_edge_is_ok`,
  `poly_self_tail_call_with_no_reference_argument_ignores_a_live_local_borrow`: the
  exemptions and the second precondition.
- `poly_non_tail_self_call_carrying_a_local_reference_is_ok` and
  `poly_self_tail_call_in_a_builtin_named_word_skips_the_back_edge_guard`: the two halves
  of the gate.
- `poly_self_tail_linear_forwarded_into_the_call_window_is_ok`,
  `poly_self_tail_unconsumed_linear_local_is_error`,
  `poly_self_tail_linear_stranded_below_the_call_window_is_not_well_typed`: 1b's three
  conclusions, pinned rather than asserted in prose.
- `poly_self_tail_dropped_borrow_then_forwarded_ref_is_over_conservative`: the
  `tail_slots` refinement's documented cost (poly body rejects, monomorphic twin
  accepts).

Mutation-tested: deleting the guard call, either half of the gate, the reference
precondition, the static-rooted filter (both wrong formulations plus forging the flag at
the borrow site), scanning `stack[..base]` instead of the args, dropping `tail && at ==
last`, and forcing an eliminator arm's `tail` to `false` are each killed by a named test.
The one surviving mutation (the per-arm `tail_slots` refinement) is unwitnessable by a
source program and is pinned by its cost.

### Lowering (`src/ir/driver.rs`)

- `poly_self_tail_call_lowers_to_loop_back_edge`: `loopg` lowers with a header block and
  a `Jmp(header)` back-edge, not an `Instr::Call` to the instantiation symbol. Poly
  instantiation is unobservable at runtime, so this asserts IR shape.
- `poly_self_call_lowers_to_ordinary_recursive_call`: the one migrated S3g test (name
  unchanged, fixture and doc moved onto a self-call kept out of tail position), the
  negative regression asserting absence of a header.
- `poly_non_tail_self_call_in_a_self_tail_body_stays_an_ordinary_call`: a non-tail
  self-call inside a word that *does* back-edge elsewhere still emits a call into the
  header-carrying func.

### Goldens (`tests/phase7_slice3g.rs`)

- `self_recursive_poly_word_runs_a_large_counter_in_constant_stack`: the exit criterion.
  The same `loopg`, counted from one million under `ulimit -s 1024`, where one frame per
  level would overflow long before the base case. Extends
  `self_recursive_poly_word_runs_to_base_case` (a run-and-assert-stdout golden with no
  header assertion, so nothing needed migrating) rather than replacing it.
- `repl_self_tail_poly_word_runs_a_deep_counter_in_constant_stack`: the REPL call site
  has no IR to assert against, so the witness is behavioural (a 2-million-deep counter;
  hardcoding `self_tail: false` aborts the session).
- `run_at_stack_limit` elevated from `tests/phase4_*` into `tests/common` as the shared
  `ulimit -s` runner (`exec` so a signal death is the binary's, `SOOTH_TRACE_ALLOC`
  cleared so its trace cannot pollute a pinned transcript).
- Every other existing S3g golden, mangled-name and mismatch test passes unchanged.

## Out of scope

- The self-call check itself (S3g, shipped).
- P7.S3k (generic-calls-generic): a self-tail loop is specific to a call to the word
  being lowered; no interaction traced.
- `resolve::mangle`, `env`, and the `instantiations`/`CallInst` machinery.
- Back-edge destructor disposal for a linear loop-carried value (the concrete side defers
  it too).

## Delivered phases

- **Phase 1**: `tail` threaded through the five poly-walk functions, per-arm via
  `PolyArm.tail`, with the word-level half off `ctx.is_self_tail_call()` and the dead
  empty combinator index in `check_poly_body` fixed; 1b resolved as no guard;
  `check_poly_reference_across_back_edge` added for 1c.
- **Phase 2**: `self_tail` computed at every `lower_word_parts` call site; back-edge
  dispatch added to the poly self-call arm and factored into `emit_back_edge`; stale
  `driver.rs` comment rewritten.
- **Phase 3**: the constant-stack goldens (native and REPL), `run_at_stack_limit`
  elevated to `tests/common`, roadmap entry marked done.

```json
{
  "phases": [
    { "phase": 1, "focus": "Thread tail-position through poly_walk/poly_term/poly_call_term/poly_combinator_call/poly_eliminator_call; guard the poly self-tail back-edge against a reference derived from a local (1c) gated on has_self_tail_call and tail; record why the linear hazard (1b) needs no guard", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "Compute self_tail at every lower_word_parts call site and add the poly self-call back-edge dispatch in func_builder/calls.rs; fix the stale driver.rs comment", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "Extend the loopg run golden into a large-counter constant-stack golden plus a REPL twin, elevate run_at_stack_limit to tests/common, confirm all other existing goldens pass", "effort": "S", "difficulty": "standard" }
  ]
}
```
