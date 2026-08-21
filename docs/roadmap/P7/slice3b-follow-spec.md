# Phase 7 Slice 3b-follow: row-typed quotation consumers in a polymorphic body (spec)

## Goal

A **non-inline** polymorphic word body may consume a **row-typed inline combinator**
(`if`, `unless`, `times`, and any library/user word declaring `inline` with `~[ ]`
parameters), so a generic word can branch and loop **without forcing every call site to
splice its whole body**. P7.S3b shipped the eliminator consumer;
this slice ships the remaining tier: a consumer whose declared quotation parameter carries
a row (`~[ ..a -- ..b ]`) matched against the poly walk's abstract stack.

Probe-verified at HEAD, the gap this closes:

```sooth
: mymax ( 'T: Copy Ord 'T -- 'T ) over over gt ~[ drop ] ~[ swap drop ] if ;
```

```text
error: `if` on a quotation in the polymorphic body of `mymax` (line 1) is not yet supported
  only an enum eliminator consumes a quotation in a generic body today (P7.S3b-follow)
```

The same word with `inline` compiles today (`examples/poly_if.sth`): an `inline` poly word
is spliced into a concrete caller and the row grounds against a real stack. The gap is a
generic word that wants to branch or loop as a **monomorphized function**, one copy per
type, not a splice per call site.

**Two guards reject this family today, not one, and the new dispatch must precede both.**
Review fix: an earlier draft claimed a single located rejection. Probe-verified at HEAD:

- `if`, `times` and `tag` hit the **name guard** at `src/check/poly.rs:922`
  (`matches!(name, "call" | "branch" | "if" | "times" | "tag")`, comment at `:916`), which
  emits `poly_quotation_combinator_unsupported_error` (`:3201`) and names this slice.
- `unless` — and every other library or user `inline` word with `~[ ]` parameters — never
  reaches that guard. It hits the **operand-window `QuotLit` arm** at `src/check/poly.rs:1997`
  and emits `poly_op_on_variable_error`:
  `` error: `unless` is not permitted on a quotation literal in `mymin` (line 1) ``, a
  different diagnostic that does **not** name this slice.

So R4's "nothing falls through to `unknown word`" conclusion holds today only by accident of
two separate guards. The dispatch R2 introduces must sit ahead of **both** `:922` and `:1997`,
or `unless` stays rejected after this slice ships.

**Anchors in this spec are stated against `d1592ae`.** P7.S3c's implementation merged into
`main` mid-review and shifted `poly.rs` by ~74 lines; it did not change the guard, the join,
or `poly_eliminator_call`'s shape, but every line number below was re-verified after it.

## Honest framing of the consumer (do not re-argue, do not inflate)

This slice is scheduled on **code size**, not on an unwritable-program witness. Two
candidate motivating programs were tried in the brief and **both turned out writable today
with `inline`**, and that record stands:

- A recursive generic word has an inline escape: `examples/experiments/arrays.sth`'s `bin_search` is
  self-*tail*-recursive, still `inline`, still generic, and lowers to a loop back-edge
  (verified in disassembly). Tail recursion is **not** blocked.
- `lib/binary_search.sth`'s sketch was withdrawn: strictly worse than what `examples/experiments/arrays.sth`
  already ships, and its `if` is inside a word that could be `inline`.

What is actually left is a real cost, not a capability gap: every caller of a generic
row-consuming word splices its entire body, so `sort`'s merge sort is spliced in full at
each call site per instantiation. A non-inline generic word is monomorphized **once per
type** instead. The secondary consequence is that a non-inline generic word cannot take a
`~[ ]` parameter at all (probe-verified), so every generic word wanting a quotation
argument is forced to splice; this slice does not by itself lift that, but nothing can lift
it while row-typed consumers are rejected in a non-inline body. The user has ruled to
proceed on the code-size argument.

## Recon (verified against the source at HEAD)

1. **The representation already exists.** `PolyType::Quotation(ins, outs, is_inline,
   row_in, row_out)` (`poly.rs:3470` (a `poly_type_str` render arm; the 5-field variant itself is the `PolyType` definition, not this line)) has carried both row fields since slice 10a; they are
   rendered by `poly_type_str` (`poly.rs:3470-3491`) and are **never unified**. This slice
   adds row grounding, not representation.

2. **The arm-walk and N-arm join already exist**, built for the eliminator.
   `poly_eliminator_call` (`poly.rs:1080-1387`) computes `row = stack[..base]`, clones the
   enclosing `PolyScope` per arm, recursively `poly_walk`s each arm body, and joins the
   exits with a **borrow-table union** (`poly.rs:1329-1387`), a `Scope::leave` analogue
   (arm-local-leak rejection + truncation), and `Moves::join`. Its own comment states the
   one difference: an eliminator arm has no declared `~[ ..a -- ..b ]` effect, its input is
   the concrete narrowed variant this dispatch computes.

3. **The concrete path is a direct model to port.** `check_poly_combinator_args`
   (`combinators.rs:652-730` (grounding at `:676`, `shape_baseline` at `:652`)) grounds a row-bearing declared quotation parameter to
   `stack[..base]`, derives `shape_changing` from `row_in != row_out`, and cross-checks
   sibling arms via a `shape_baseline: HashMap<u32, Vec<Type>>` keyed by the **output row
   id**. The poly analogue is the same split over `PolySlot`/`PolyType` instead of
   `Slot`/`Type`.

4. **`poly_call_term` has no combinator dispatch and no combinator-env access.** It routes
   `poly_construct_generic` → `env` (monomorphic candidates only; poly words and combinators
   are not registered) → `poly_delegate_op` → `unknown word` (`poly.rs:920-985`). The guard
   at `poly.rs:848` is currently the **only** thing catching `if`/`times`; removing them
   from it without adding a dispatch would drop them to `unknown word`. Dispatching a
   library combinator therefore requires threading its declared `PolySig` source into the
   poly walk (new plumbing, analogous to how `enums`/`structs`/`arrays` are threaded).

5. **`tag` is not a quotation consumer.** `check_tag_word` (`word_families.rs:615-643`)
   takes an all-unit **enum** operand and produces `u32`; it carries no `~[ ]` parameter.
   It sits in the `poly.rs:848` guard only because the guard is name-based and `if`'s inline
   library body (`lib/core.sth:42`) expands through `tag`. It is not this slice's mechanism.

## Rulings on the brief's open questions

### OQ1 (the sizing question): is `poly_eliminator_call`'s arm-walk extractable? — RULED: extract the join unconditionally; extract the arm-walk if it reads cleanly, with a stated fallback

**Decision.** Factor out a shared helper (working name `poly_walk_arms`) that owns the
per-arm mechanics both consumers share: per-arm `PolyScope` clone, recursive `poly_walk`,
the arm-exit escape checks (variant escape, unconsumed nested quotation), the `Scope::leave`
analogue (arm-local-leak rejection **then** truncation to the enclosing key set), the
`Moves::join` reduction, and the **borrow-table union**. It is parameterized by (a) how each
arm's *input* row is built (narrowed variant for the eliminator; the grounded declared row
for the combinator) and (b) the cross-arm *output* rule (rigid structural equality for the
eliminator; entry-row equality or output-row `shape_baseline` for the combinator, per R3).

**The borrow-table union is shared unconditionally, non-negotiable.** `PolyScope`'s borrow
table is name-keyed and a **missing** record reads as "no conflict" (`live_borrow_of`
returns `None`), so a second join that intersects or picks one arm is a **false accept**,
not a false reject. There must be exactly one join in the file after this slice; a second
copy is a soundness regression, not a style choice.

**Fallback (the brief's escape hatch).** If threading both output modes through a single
`poly_walk_arms` visibly distorts `poly_eliminator_call`'s readability, extract only the
soundness-critical primitives (borrow-union, leave-truncation, `Moves::join`) as shared free
functions and let the two arm-walk drivers stay separate, both calling the shared join. The
implementer makes this readability call in Phase 1 against this stated fallback; either way
the join is shared. This is why Phase 1 is the extraction and nothing else.

### OQ2: do all four consumers land together, or does `tag` separate? — RULED: deliver row-typed inline combinators (`if`/`times`/library) via one signature-driven dispatch; keep `call`/`branch`/`tag` as narrowed located rejections

**Decision.** This slice delivers **one mechanism**: signature-driven dispatch of a
row-typed *inline combinator* (a `WordDef` with `declares_inline` and `~[ ]` parameters).
That single mechanism covers `if`, `unless`, `times`, and any library or user inline
combinator, because it is driven by the combinator's declared `PolySig`, not by name. It
exercises **both** row cases: shape-changing (`if`/`unless`, `row_in != row_out`) and
non-shape-changing (`times`, `row_in == row_out`).

**`call`, `branch`, `tag` are NOT delivered and keep a located rejection**, with the
`poly.rs:848` guard narrowed to exactly those three names:

- `tag` is not a row-typed consumer at all (recon 5): an enum→`u32` scalar primitive. Making
  it work is a trivial, separate scalar-primitive port with no arm-walk; bundling it here
  would widen the slice past its one mechanism for no shared code. Deferred, located.
- `call` and `branch` are compiler-known **primitives**, not combinator-env words, so each
  needs its own hand-written row grounding (a second and third code path), not the
  signature-driven dispatch. `call`/`branch` on a *parameter* is unreachable in a non-inline
  body anyway (it cannot declare a `~[ ]` parameter); on a *literal* they are low-level and
  the `if`/`unless`/`times`/`each` surface subsumes them. Deferred, located.

Any name left out **keeps `poly_quotation_combinator_unsupported_error`** rather than falling
through to `unknown word`; its message is re-pointed at the follow-up that would take the
primitives. No consumer silently becomes `unknown word`.

### OQ3: what happens to `poly.rs`'s deferred split? — RULED: re-run the signals at exit, do not decide now

P7.S3b deferred the split and named **this slice** as the trigger (a second quotation
consumer). Per CLAUDE.md, re-run the five signals at phase exit against `poly.rs` as it then
stands, and record the decision in the exit findings (Phase 4). Do not pre-decide here; the
extraction in Phase 1 changes the shape the signals see. Note for the exit re-run: with two
consumers now sharing `poly_walk_arms`, the responsibility-shaped split (eliminator +
combinator arm machinery into one module) is more defensible than it was at S3b, when the
split would have cut the `poly_call_term → poly_eliminator_call → poly_walk` recursion for a
single consumer.

### OQ4: does an erased (non-literal) quotation reach this path? — RULED: require a splice-consumed literal at each arm; reject a non-literal operand located, never inherit the panic

**Decision.** Each combinator arm operand must be a **splice-consumed quotation literal**
(a `PolySlot` carrying `PolyType::QuotLit` with a live `PolyQuotRef`), the same admission
S3b's L2 fixed for the eliminator. A non-literal quotation operand at an arm position, an
abstract/forwarded quotation, or an **erased** one materialised from a word return, is a
**located rejection** here, reusing the S3b materialisation diagnostic family
(`poly_quotation_not_consumed_error` / `poly_quotation_combinator_unsupported_error` as
fits), **not** an inherited backend panic. This forecloses the pre-existing
`while`-over-an-erased-quotation ICE (`ir/func_builder/control_flow.rs`) from being reached
through the new path. Materialised/escaping quotations in a poly body remain out of scope
(carried L2); the arm is a literal or it is an error. Phase 3 must include a test that a
non-literal arm operand hits the located rejection and does not panic.

## Locked rules (carried from P7.S3b, unchanged and for the same reasons)

- **L1 Type variables stay rigid; no mid-body `Subst`.** An arm leaving `Var(0)` against a
  sibling's `Var(1)`, or against `Concrete(i64)`, is a located
  `poly_arm_output_disagreement_error`, never a bind. Admitting it would need a mid-body
  unifier with ripples into mangling. No `apply_subst` in the term walk.
- **L2 Row *variables* stay rigid too.** `..a` grounds **once**, to `stack[..base]` at the
  dispatch site, and is not solved for. Nothing in this slice infers a row.
- **L3 The arm merge UNIONS the borrow table.** Restated because it is the false-accept
  surface (see OQ1). A merge that intersects or picks one arm silently accepts a later use
  of a place only one arm borrowed. `poly_eliminator_call` already unions; the shared join
  must be the only join.
- **L4 Splice-consumed quotations only.** The arms are `~[ ]` inline-only parameters by
  declaration (`lib/core.sth:42`), so an ordinary `[ ]` arm is the wrong bracket and must
  produce the eliminator's existing `ordinary_literal_at_inline_param_error`, not a new one.

## Delivered shape

### R1 Shared arm machinery (OQ1)

Extract `poly_walk_arms` (or, per the fallback, the join primitives), owning the per-arm
clone, recursive walk, arm-exit escape checks, `Scope::leave` analogue, `Moves::join`, and
the **single** borrow-table union. `poly_eliminator_call` is refactored to call it with no
behaviour change; its existing goldens and unit tests stay green as the regression gate for
the extraction.

### R2 Combinator-signature plumbing and dispatch

Thread the row-typed inline combinator's declared `PolySig` source into the poly walk. A
combinator's `PolySig` is derivable from its `WordDef` (as `check_poly_combinator_standalone`
already does); thread either the `CombinatorEnv` or a pre-built name→`PolySig` view through
`poly_walk`/`poly_term`/`poly_call_term`, one new parameter, added **at first use** (no
pre-staged plumbing). In `poly_call_term`, **before** the narrowed guard, intercept a call
whose name resolves to a row-typed inline combinator with its quotation arms present on the
stack and dispatch to `poly_combinator_call`. Verify the call's name (a prelude word such as
`if` is exempt from mangling) matches the `CombinatorEnv` key (`word.name`); if a mangled
name is observed, resolve it before lookup. Narrow the `poly.rs:848` guard to
`matches!(name, "call" | "branch" | "tag")`.

### R3 `poly_combinator_call`

Port `check_poly_combinator_args`'s row logic over `PolySlot`/`PolyType`:

- `n = sig.inputs.len()`; underflow if `stack.len() < n`; `base = stack.len() - n`.
- Ground the declared row: a `PolyType::Quotation(_, _, _, Some(_), _)` parameter grounds
  its `..a` to `row = stack[..base].to_vec()` (L2); a parameter with no row grounds against
  the empty region.
- Collect each arm literal off the stack by its `PolyQuotRef`; a non-literal operand is the
  OQ4 located rejection; an ordinary `[ ]` bracket is L4's existing diagnostic.
- **Non-shape-changing** (`row_in == row_out`, e.g. `times`): **pre-seed the cross-arm
  baseline with the grounded entry row (`row`) before walking any arm**, then compare each
  arm's exit to it structurally (L1 rigid).
  **This is a soundness requirement, not a stylistic one.** Review fix (blocker): an earlier
  draft delegated this to "the shared join's structural per-slot check". The join extracted
  from `poly_eliminator_call` is **cross-arm only** — `baseline` is seeded `None`
  (`poly.rs:1231`) and set from the *first arm's* exit (`:1329`) — because an eliminator's
  arms are *supposed* to change the stack shape. `times` is **single-arm**: with a `None`
  seed the comparison never fires and nothing checks exit against entry. The unseeded design
  wrongly accepts:

  ```sooth
  : bad ( 'T: Copy 'T i64 -- 'T ) ~[ dup ] times ;
  ```

  whose arm has effect `'T -- 'T 'T` against `times`'s declared `~[ ..a -- ..a ]`, producing
  a loop whose back-edge stack depth does not match its entry. The concrete path rejects it
  via `check_literal_against_declared_effect` under
  `LiteralBoundary { shape_changing: false }` (`combinators.rs:~700`). Pre-seeding is what
  ports that rejection. `bad` is a required rejection test (see Testing).
- **Shape-changing** (`row_in != row_out`, e.g. `if`/`unless`): no fixed exit-row check;
  sibling arms sharing one output-row id are cross-checked against a poly `shape_baseline:
  HashMap<u32, Vec<PolySlot>>` keyed by that id, the first arm setting the baseline and each
  later one compared structurally (L1) with `poly_arm_output_disagreement_error` on
  disagreement.
- Join via the shared machinery (R1): borrow-union (L3), `Moves::join`, leave-truncation.
- Exit row is the arms' common exit; type variables are never bound (L1/L2), so no per-arm
  clone diverges on `Subst`.

### R4 Narrowed located rejection, no `unknown word` fallthrough

`poly_quotation_combinator_unsupported_error` remains for `call`/`branch`/`tag`, its message
re-pointed at the follow-up that would take the primitives (not at "P7.S3b-follow", which is
this slice). No name that reaches this family falls through to `unknown word`.

**`tag`'s guard arm is reachable — probe-verified, so it stays in the mutation list.**
Review fix: the reachability was queried on the grounds that a guard whose deletion causes no
test failure looks tested and is not. It is genuinely reached:

```text
: tg ( 'T: Copy 'T -- 'T ) ~[ dup ] tag ;
error: `tag` on a quotation in the polymorphic body of `tg` (line 1) is not yet supported
```

Each retained name's rejection test **must assert the exact message text**. Asserting only
that the build fails is a placebo: it passes identically against an `unknown word`
fallthrough, which is the one regression these tests exist to catch.

## Lowering

**No lowering change.** A non-inline poly word is already monomorphized once per instantiation
by `lower_instantiation`, which splices the inline combinator's body at that single
per-type function (not per call site). The code-size payoff this slice is scheduled on is a
property of that **existing** lowering; this slice only makes an `if`-using non-inline body
reach it by unblocking the checker. If a combinator arm reaches an unlowered shape at
monomorphization, that is an OQ4 case the checker must have rejected first.

## The golden (a test fixture, not a `lib/` word)

**Review fix (blocker): the previous golden was unbuildable and has been replaced.** It was
an `Ord`-bounded `bin_search` with a **non-inline recursive helper**. A non-inline generic
word cannot call itself at all:

```text
: loopg ( 'T: Copy 'T i64 -- 'T ) 1 sub loopg ;
error: unknown word `loopg__m0` in `loopg`
```

That is the *generic-word-calls-generic-word* standing limit this spec already lists as out
of scope, so no amount of row machinery would have made that fixture compile. **Self-recursion
in a non-inline generic body is tracked separately as P7.S3g**
(`docs/roadmap/P7-language-prereqs.md`); **no phase's exit criteria in this spec may depend on
it.** Note the consequence for the rationale, recorded rather than smoothed over: the
code-size win applies only to generic words that loop *without* self-recursion until P7.S3g
lands.

The replacement loops with a **literal `times`** carrying an inner `if`, which needs no
self-call. Probe-verified to hit exactly this slice's rejection today, so it discriminates:

```text
: sumg ( 'T: Copy 'T i64 -- 'T ) ~[ | i | ] times ;
error: `times` on a quotation in the polymorphic body of `sumg` (line 1) is not yet supported
  only an enum eliminator consumes a quotation in a generic body today (P7.S3b-follow)
```

The fixture is a non-inline generic word that folds an accumulator over a fixed iteration
count, selecting per iteration with an inner `if` — exercising the shape-changing case (`if`)
nested inside the non-shape-changing case (`times`) in one non-spliced body, at two
instantiations so `'T` is carried rigidly rather than coincidentally matching:

```sooth
: clampsum ( 'T: Copy Ord 'T 'T i64 -- 'T )
```

A test fixture, **not** a library word: `examples/experiments/arrays.sth` already has a
capable comparator-driven `bin_search`/`sort` (an experiment, not shipped in `lib/`), and
an `Ord`-only fixture is strictly weaker anyway (`is_ord` is `is_numeric` and nothing else,
so it cannot reach a user struct). Nothing here belongs in `lib/`.

Two halves:

- **Behavioural.** The fixture's arithmetic result at two instantiations, with the `if`
  selecting different arms across iterations so both arms are exercised and arm-order
  routing is not vacuous. **Mutation-tested guard:** deleting the R2/R3 combinator dispatch
  makes the fixture fail to compile with the located rejection, so this golden guards this
  slice's new code. Because a compile failure is the *primary* failure mode, the behavioural
  assertions must also be able to fail *while compiling* — hence two instantiations and
  both `if` arms taken, which a wrong arm-routing or a wrong exit-row join can break without
  breaking the build.

- **Structural — a CHARACTERIZATION only, with no mutation-guard claim.** Review fix: the
  previous draft demanded a mutation in which a wrong R3 "splices the arms into the caller"
  and makes a copy count scale with call sites. That mutation cannot be written against R3:
  copy count is a **lowering** property, and this slice changes **no lowering** (see
  Lowering). The test therefore records, without claiming to guard R3, that the non-inline
  generic word is emitted as one definition per instantiation reached by real `call`s from K
  call sites. It is kept only because it pins the code-size claim the slice is scheduled on;
  it is **excluded from the mutation-tested guard list**, and must be labelled a
  characterization in the test file so a later reader does not mistake it for a guard.
  `nm` for a minted symbol remains a known placebo here regardless (`poly_indices` excludes
  poly template words from symbol minting), so the assertion is over emitted QBE, not `nm`.

## Testing

Goldens (`tests/phase7_slice3b_follow.rs`): the `mymax` shape-changing `if` exit criterion
at two instantiations (a carried `'T`, rigid across arms); the `clampsum` behavioural
matrix and structural characterization above; a non-shape-changing `times` body; **the
single-arm `times` entry-row rejection (`~[ dup ] times`, R3's pre-seeded baseline — this
one is a soundness test, not a diagnostic test)**; arm output
disagreement (rigid `'T` vs `i64`); arm depth mismatch; the borrow-union false-accept guard
asserting **both** single-arm-pick directions and a cross-arm mutability disagreement; a
bind-and-leak of a linear arm-local and the one-arm-binds-one-does-not no-ICE case; an
ordinary `[ ]` arm (L4); the OQ4 non-literal / erased arm operand (located, no panic); and
the narrowed `call`/`branch`/`tag` located rejections, **each asserting the exact message
text** rather than merely that the build fails (a bare failure assertion passes identically
against an `unknown word` fallthrough, the one regression these tests exist to catch).
`unless` gets its own golden proving it reaches the new dispatch rather than the
operand-window `QuotLit` arm at `poly.rs:1997`.

Unit tests in `src/check/poly.rs`: the extraction preserves the eliminator join
(`poly_walk_arms` under both an eliminator and a combinator caller unions borrows
identically); combinator dispatch precedes the narrowed guard; shape-changing vs
non-shape-changing routing; row grounds to `stack[..base]`; a rigid type variable is not
bound across arms.

Mutation-tested guards (delete/flip the guarded code, watch the test fail, then restore to a
clean `git status`): the borrow union (both single-arm picks); the combinator dispatch
intercept (deletion → located rejection on the golden); **R3's pre-seeded entry-row baseline
(revert the seed to `None` → `~[ dup ] times` must start compiling; if it still fails,
the test is a placebo and something else is rejecting it)**; the OQ4 non-literal rejection;
the shape-baseline cross-arm check; and the narrowed guard's remaining names
(`tag`'s arm is probe-confirmed reachable, so it belongs here). **The structural
characterization is deliberately NOT on this list** — it asserts a lowering property this
slice does not change, so no mutation of this slice's code can flip it. Commit before
mutation testing; a mutation copy needs `examples/`; touch sources after any rollback; end
each cycle on a clean `git status`.

Regression, green and untouched: `tests/phase7_slice3b.rs` (the extraction's gate),
`tests/phase7_slice3a.rs`, the `tests/phase6_*` eliminator suites, `tests/qbe_baseline.rs`.

## Exit findings (Phase 4, required)

- Re-run `poly.rs`'s five split signals (OQ3) and record the decision.
- Confirm the borrow join is unique in the file (no second, non-unioning join was introduced).
- Record any consumer still rejected and the follow-up its message points at.

## Out of scope

- `call`, `branch`, `tag` in a poly body (OQ2): narrowed located rejection, a separate
  follow-up.
- The rowless concrete consumer (P7.S3d): assumed, not re-done.
- Self-recursion in a non-inline generic body: **P7.S3g**, and the reason the golden loops
  with a literal `times` instead. No exit criterion here may depend on it.
- Slices (P7.S3c) and trait bounds (P7.S3e): recon found no interaction. **P7.S3c has since
  merged into `main`** (it shifted `poly.rs` by ~74 lines without touching the guard, the
  join, or `poly_eliminator_call`); re-verify once against the merged state, do
  not design around them.
- The three pre-existing row-combinator ICEs, except as OQ4 requires a located rejection
  rather than inheriting a panic.
- A generic word calling another generic word (`unknown word g__m0`): a standing limit that
  bounds what can be written against this slice.
- Materialised / escaping / erased quotations in a poly body (L2/L4).
- Mid-body unification of type variables or rows (L1/L2).
- Any lowering change.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Extract shared arm-walk and join from poly_eliminator_call (OQ1); borrow-union stays the single join; eliminator goldens are the no-behaviour-change gate",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Thread the combinator PolySig source into the poly walk and dispatch a non-shape-changing row-typed inline combinator (times); narrow the poly.rs:848 guard to call/branch/tag",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Shape-changing case (if/unless) via a poly shape_baseline keyed by output-row id; OQ4 located rejection of a non-literal or erased arm operand",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Golden clampsum fixture (literal-times loop with an inner if, no self-recursion; behavioural matrix plus structural characterization), mutation-test the guards, and exit findings including the OQ3 poly.rs split re-run",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
