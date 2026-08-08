# Phase 4 Slice 6f: liveness ends at last use (brief)

Naming a borrow keeps it alive longer than using it does. A reference left on the stack dies
when a term consumes its slot; a reference *bound to a local* stays live for the whole
block. So the natural shape — borrow a place, write through the borrow, then consume the
place — is rejected, and the workaround is to spell the whole projection as one chain.
Confirmed by compiling against the built compiler, in a straight-line word body with no loop
and no quotation involved, so this is not about iteration scoping:

```
\ named: rejected today
&!acc &!Acc>arr | f |  f 0 >usize &!> 5 !  acc drop
=> error: cannot consume the borrowed local `acc` of type `Acc` (line 8, col 3)
     the mutable borrow taken at line 6, col 3 is still live

\ chained: identical semantics, compiles and runs
&!acc &!Acc>arr 0 >usize &!> 5 !  acc drop
=> builds, prints 5
```

The consumer that surfaced it is Phase 4's combinator work: an in-place accumulator body
(`| acc v | &!acc &!Acc>arr ... acc`) is the shape a `fold` over a linear aggregate wants,
and it only compiles chained. But the rule is not combinator-specific and the fix is not in
the combinators; the probe above is a plain word body.

**This is not a lifetime system**, by DESIGN.md's own definition (Memory model): no lifetime
variables, no regions, nothing binding a reference's validity to a named scope. It is a rule
about when a borrow ends inside one block, and the anonymous case *already works that way*.
The slice makes named references behave like the stack values the language is otherwise
built around, rather than adding a concept.

## Recon: measured against the built compiler and read against the current checker

**1. The rule is one function, and its doc comment already states the asymmetry.**
`live_derivs` (`src/check.rs:759`) chains the virtual stack's derivations with the scope's
bindings, documented as: "A reference is live from the term that creates it until the term
that consumes its slot; a reference *local* is live for the whole block." Seven call sites
read it — `:5844`, `:5858`, `:5881`, `:6002`, `:6041`, `:7275`, plus `aliasing_origin`'s own
scan at `:7292` — via `live_deriv`/`live_borrow_of`/`live_mutable_borrow_of`, so they all
inherit whatever it decides. This is a single choke point, not a rule scattered across the
checker. Two corrections to an earlier count of five, both caught while reviewing the spec:
`:5881` (`live_mutable_borrow_of` → `naming_aliases_borrowed_place_error`, the naming-side
twin of `aliasing_origin`) was missed because the call-site grep omitted that wrapper; and
`:5844` is the *reborrow-suspension* check (`suspended_place_error`), not the naming-side one.
`:5844` is also structurally immune to this slice — `Scope::bind` mints every binding deriv
with `reborrow: false` (`:583`), so the `scope.bound` half can never satisfy its predicate.

**2. Only the `scope.bound` half is lexical; the stack half is already use-based.** Naming a
reference local is a *reborrow*, not a move (`src/check.rs:5841`-`:5849`): it mints a fresh
`Deriv` chained to the binding's held one and pushes it on the stack, where it dies when a
term consumes it. The binding's own `deriv` (`Binding.deriv`, `:693`) is the thing that
outlives every use. **So the change is one half of one function**, and the half that already
behaves correctly must not be touched.

**3. The rejection path.** `TermKind::Call` on a *linear* local (`:5858`) asks
`live_borrow_of` (`:780`) for any live derivation rooted at that place and reports
`consume_of_borrowed_place_error` (`:7013`). A `Copy` local is merely read, never consumed,
so it never reaches the check — which is why the probes above need a linear type to
reproduce at all.

**4. The position needed to decide "last use" is already in hand.** `check_terms`
(`:5734`) walks `for (i, term) in terms.iter().enumerate()` (`:5748`) over the very slice a
last-use scan would read. No new plumbing is needed to know where the walk is.

**5. Last use is computable from the term list, and the walk already exists.** A read of a
local is `TermKind::Call(name)` — the enum documents it as "a word invocation, or a
reference to a named local" (`src/ast.rs:970`). `alpha_rename_locals` (`src/ast.rs:1002`)
already performs exactly this analysis for the inliner: track the names a block binds,
rewrite the `Call`s that refer to them, honouring that a bind's extent is the rest of its
block and that a nested quotation or `if` arm inherits the outer binds. **It is a working
precedent for the scan, not a new analysis to invent**, and the spec should model on it.

**6. There is no shadowing case to handle.** Re-binding a name already in scope is rejected
outright: `: main ( -- ) 1 | a | 2 | a | a . ;` fails with "`a` is already bound in `main`
... a name may not be re-bound while it is in scope". So within a block a name maps to one
binding, and a forward scan for the last `Call(name)` needs no stop-at-rebind rule.

**7. Two existing tests encode the lexical rule and flip meaning; a third must not.** In
`tests/phase3_refs.rs`: `move_of_place_borrowed_in_locals_is_error` (`:750`) binds `&b | r |`
and never uses `r` again, and its comment says outright "a reference local is live for the
whole block"; `dispose_of_borrowed_place_is_error` (`:765`) has the same shape with `&!b`.
Both must **accept** after this slice, because a reference that is never used again is dead.
`move_of_place_borrowed_on_stack_is_error` (`:735`) is the control and must keep rejecting:
its borrow is left unconsumed on the stack, which is genuinely live.

**8. The decisive probe pair, both rejected today with the same message.** After the slice
they must diverge, which makes them a mutation-testable pair rather than a pair of assertions
that could both pass for the wrong reason:

```
B (must accept):  &!acc &!Acc>arr | f |  f 0 >usize &!> 5 !  acc drop
A (must reject):  &!acc &!Acc>arr | f |  acc drop  f 0 >usize &!> 5 !
```

**9. The `times` identity check becomes position-sensitive.** `:6002`/`:6041` snapshot
`live_derivs` before the body splice and compare against after, requiring equality (a borrow
is idempotent per iteration, so a well-formed body leaves the set unchanged). Once liveness
depends on a term index, "before" and "after" are evaluated at different positions and the
comparison stops being apples-to-apples on its own terms. This is the one existing consumer
the change can silently break.

**10. Checker-only, verified.** `Deriv`/`DerivId` appear in no file but `src/check.rs` (zero
hits elsewhere in `src/`), so nothing in `ir.rs` or the backend reads borrow liveness. This
slice cannot change lowering even by accident.

**11. The sibling table, and what relaxing it is worth — measured, not argued.**
`aliasing_origin` (`:854`, sole caller `:7292`) rejects a mutable borrow of a place a second
live *name* denotes, scanning `scope.bound` for lexically-in-scope names with an overlapping
alias set (plus a stack half, `:876`, which is already positional). It is the same
lexical-vs-last-use question pointed at names instead of borrows, and it is what forces `dup`
on a `Copy` aggregate accumulator. Stubbing it to `None` and rebuilding:

```
\ Copy Acc, in-place body, aliasing_origin stubbed
[| acc v | &!acc &!Acc>arr v >usize &!> v 2 * ! acc ] c::fold
=> compiles, prints 6 (correct), and the loop body emits ZERO blits:
   one index phi, a direct `storel` into the stable slot,
   both remaining blits in the preheader (once per call)
```

Against **four** 32-byte blits per iteration for the `dup` form it forces today (the dup, the
reconstruct, and the two-phase back-edge staging). So the payoff is real: an expensive
implicit memcpy where a reader expects a move. The caveat on this measurement is that the
stub disables the check entirely, proving the codegen *upper bound*, not that a correct
last-use relaxation reaches it — but `fold`'s body names `acc` exactly once
(`acc count [ ... ] times`) and never again, so it is genuinely dead where the spliced body
borrows it, and the result transfers for this shape.

**12. The guard surface `aliasing_origin` actually provides: 17 tests.** With the stub in
place the suite fails exactly 17, all in `tests/phase3_refs.rs`, in three groups: name/peek/
getter aliases (`mutable_borrow_of_name_aliased_place_is_error` and kin), stack aliases
(`..._aliased_on_the_stack_is_error`, the positional half that does not change), and **six
branch/merge cases** (`mutable_borrow_aliased_by_one_if_arm_only_is_error`,
`..._of_a_merge_of_two_aliased_arms_is_error`, and kin). Three were sampled in full
(`:823` name alias, `:1060` `over`-duplicate, `:974` one-arm merge) and **all three use the
aliasing name after the borrow** — they were written to demonstrate the mutation is
observable, which requires the later use — so they stay live and keep failing for the reason
they state. The risk therefore concentrates in the merge group, where region unioning meets
per-arm liveness (Q3).

Probes live in `/tmp/sooth-repro/t20.sth` (named, rejected), `/tmp/sooth-repro/t21.sth`
(chained, builds and prints `5`), `/tmp/sooth-repro/t22.sth` (use-after-consume, rejected),
`/tmp/sooth-repro/t23.sth` (shadowing, rejected). All disposable, rebuilt from the snippets
above.

## Locked decisions

- **D1.** Checker-only. No new `Instr`/`Terminator`, no lowering change, no `Type`/`IrType`
  change (recon 10). The slice touches `src/check.rs`, `tests/phase3_refs.rs`, and dogfood
  examples.
- **D2.** Within `live_derivs`, only its `scope.bound` half becomes use-bounded (recon 2);
  its stack half is already correct and must be left alone, and a "fix" that touches it is
  out of scope and probably a regression. The same split applies to `aliasing_origin` (D8):
  scope half relaxed, stack half untouched, in both tables.
- **D3.** Last use is computed per `check_terms` invocation, over the slice being walked.
  A spliced combinator body, an inlined quotation body, and an `if` arm each get their own
  scan of their own term list. No global or cross-body index, which is what keeps this
  composable with the inliner's alpha-renaming (recon 5).
- **D4.** No shadowing rule is needed (recon 6). If the spec finds a path where a name *can*
  be re-bound in scope, that is a separate defect to report, not something to handle here.
- **D5.** The two flipping tests (recon 7) are **rewritten to use-after-consume**, not
  deleted: their intent — a held reference blocks consuming its place — is correct and worth
  keeping, and today it is being proved by accident of lexical scoping rather than on
  purpose. The rewrite makes each prove the rule for its stated reason. The stack control
  test is left untouched.
- **D6.** Inside a loop body, any use of a reference local means live for the whole body: a
  use "earlier" in the body is a *later* use on the next iteration. The spec must not treat a
  loop body as straight-line code.
- **D7. Ending a borrow disposes nothing and emits nothing; no implicit drop is introduced,
  and none may be added to make this slice work.** A borrow ending is not a value being
  dropped: a reference is neither `Copy` nor linear and owns nothing, so it carries no drop
  obligation. `Scope::bind` registers a move-state entry only for a linear value
  (`src/check.rs:719`) and `Scope::leave` reports a leak only where such an entry exists
  (`:745`), so a reference local going out of scope already runs nothing and reports nothing
  today. `Deriv`/`DerivId` never leave `src/check.rs` (recon 10), so lowering cannot observe
  borrow liveness at all and no emitted code can change. A `drop` is always an explicit term
  the programmer wrote — an unconsumed linear local is an *error* precisely because nothing
  auto-drops — and this slice only changes the point at which the checker stops refusing such
  a term. It moves entirely within what the checker permits, never within what the compiler
  emits.
- **D8. `aliasing_origin`'s scope half is in scope, and gets the same treatment**
  (recon 11/12): a name that is never used again does not count as a second denoting name,
  exactly as a reference local that is never used again does not count as a live borrow. One
  last-use analysis, two scope tables, both stack halves untouched. Splitting it would answer
  one question twice, and the payoff is a measured removal of a per-iteration memcpy, not a
  hypothetical.
- **D9. The alias half is mutation-tested, the borrow half is merely tested.** The two differ
  in failure mode: a wrong answer on the borrow half accepts or rejects a *program*, while a
  wrong answer on the alias half silently produces a wrong *value* — which DESIGN.md names as
  the class this language exists to turn into a compile error. Every one of the 17 guard tests
  must be shown to fail when the guard it names is removed, not merely to pass. Reading them
  is not sufficient evidence and has not been on this project before.
- **D10. The exclusivity relaxation is intended, not incidental** (was Q4). Two sequential
  mutable borrows of one place where the first is never used again become legal. This is a
  visible relaxation of the per-place exclusivity rule DESIGN.md states, so it is recorded
  here as deliberate and pinned with its own test. It is not two live mutable references:
  `| f |` *pops* the reference off the stack into the binding table
  (`stack.split_off(stack.len() - names.len())`, `:5825`), and a binding never named again can
  be read through by nothing, so neither a mutation nor an observation through it is
  expressible.

## Open questions (for the spec)

- **Q1. Where does last-use live?** Three shapes, and the spec should pick one and say why.
  (a) Compute it at the `Bind` term by scanning forward in the current slice, and store an
  expiry index on `Binding`; `live_derivs` then needs only the current index. (b) Scan
  forward from the query point each time `live_derivs` runs, with no stored state. (c) A
  pre-pass building a table for the whole body before the walk. (a) looks smallest and keeps
  the knowledge next to the binding that owns it, but it adds a field to `Binding` and
  requires the position at bind time; (b) adds no state at all but re-scans per query. This
  is a craft project, so the honest default is the one with the least machinery that is not
  quadratic in practice.
- **Q2. What position do the `times` before/after snapshots use** (recon 9)? Evaluating both
  at the index of the `times` term keeps the comparison apples-to-apples and is probably
  right, but it needs stating explicitly and a test that a body genuinely leaving a borrow
  live is still caught.
- **Q3. Branches.** For a reference bound *before* an `if` and last used inside one arm only,
  is the expiry the max across arms (conservative, live until the `if` ends) or per-arm? Max
  is simpler and cannot be unsound; per-arm is more precise and probably unnecessary. Pick
  the conservative one unless a dogfood needs otherwise. Recon 12 makes this the highest-risk
  question in the slice: six of the 17 alias guards are merge cases.
- **Q4. Does `leave_block` (`:5625`) need anything?** It truncates `scope.bound` at block
  end, which already retires a reference local's deriv. Probably untouched, but the spec
  should confirm rather than assume, since it is the other place bindings disappear.
- **Q5. The REPL path.** A REPL line is a block; a reference local bound on a line has no
  next line to be used from. Recon did not exercise this. Confirm the rule degrades sensibly
  there or state that it is unchanged.
- **Q6. Does the alias half need its own last-use rule, or does it share the borrow half's?**
  Both ask "is this name used again", so one scan should serve both (D8). But `aliasing_origin`
  keys on alias-set *overlap* rather than name identity, and a region can be denoted by a name
  whose own last use has passed while a *different* overlapping name is still live. The spec
  should confirm that filtering each binding by its own last use composes correctly with
  overlap, rather than assuming name-wise deadness implies region-wise deadness.

## Dogfood

The motivating program, and the reason the slice exists: an in-place `fold` accumulator
written the way a reader would write it, with the accumulator named rather than chained.

```
type: Acc arr [i64 4] ;

: main ( -- )
  scores 0 4 fill Acc
  [| acc v | &!acc &!Acc>arr | f |  f v >usize &!> v 2 * !  acc ] c::fold
  Acc>arr | a | &a 3 >usize &> @ . ;
```

Rejected today at `| f |` (the borrow outlives its use) and, for a `Copy` `Acc`, rejected
again at `&!acc` (aliased by `fold`'s own `acc__inl0`). It must compile after this slice at
**both** a `Copy` `Acc` and a linear one, and lower to a loop with no per-iteration `blit` in
either case — the second half being the measurable claim, checked against emitted QBE rather
than asserted.

## Out of scope

- **Declarable linearity.** `is_copy` (`src/check.rs:239`) derives linearity structurally and
  the only way to opt a struct out of `Copy` is to give it a `drop` overload, so "thread this
  in place" is spelled "give this a destructor". Real, recorded in ROADMAP 6f and DESIGN.md's
  Memory model, and entangled with slice 1's parked question of whether `Copy` is a
  privileged constraint and with slice 8's polymorphic `drop`. Do not settle it here, do not
  foreclose it.
- **Closure captures** (slice 7). This slice must land first so slice 7 points a settled rule
  at a new carrier, but nothing about capture is decided here.
- **The `times` reference-across-back-edge rejection** (`times_body_borrow_across_loop_error`,
  `:5589`). Carrying a reference *as loop-carried state* is a different feature with a
  different justification; this slice only ensures its guard still means what it says (Q2).
- **Any change to what a borrow *is*** — projection, reborrow chaining, `owned_root`
  propagation, or the per-place exclusivity rule's shape. Only *when a borrow ends* changes.

## Citations (verified against current `main`)

`live_derivs`: `src/check.rs:759` (definition and the doc comment stating the asymmetry).
`live_deriv`: `:769`. `live_borrow_of`: `:780`. `live_mutable_borrow_of`: `:793`.
Call sites: `:5844`, `:5858`, `:6002`, `:6041`, `:7275`. `Scope`/`Binding`: `:680`/`:689`,
`Binding.deriv` at `:693`. `Scope::depth`: `:702`. `Scope::local` (front-first): `:706`.
`Scope::leave`/`leave_block`: `:742`/`:5625`. Reference-local reborrow path: `:5841`-`:5849`.
Linear-local consume check: `:5858`. `consume_of_borrowed_place_error`: `:7013`.
`suspended_place_error` reborrow guard: `:5846` (message at `:6999`). `times` identity snapshots: `:6002`/`:6041`;
`times_body_borrow_across_loop_error`: `:5589`. `aliasing_origin`: `:854`, sole caller
`:7292`. `is_copy`: `:239`. `check_terms` and its enumerate loop: `:5734`/`:5748`.
`TermKind::Call` documented as word-or-local: `src/ast.rs:970`. `alpha_rename_locals`:
`src/ast.rs:1002`. Flipping tests: `tests/phase3_refs.rs:750` and `:765`; stack control that
must not flip: `:735`. `aliasing_origin` guard surface: 17 tests in `tests/phase3_refs.rs`,
sampled at `:823` (name alias), `:974` (one-arm merge), `:1060` (`over`-duplicate), all three
using the aliasing name after the borrow; `aliasing_origin`'s own stack half at `:876`.
The stub experiment (`aliasing_origin` → `None`, rebuilt, reverted): the `Copy`-`Acc`
in-place `fold` compiles, prints `6`, and emits zero blits in the loop body, against four per
iteration for the `dup` form; suite fails exactly those 17 and nothing else. `Deriv`/`DerivId` confined to `src/check.rs`: zero hits elsewhere in
`src/`. Probes run against the built compiler on `main` at `a5a4180`: named form rejected,
chained form builds and prints `5`, use-after-consume rejected, re-bind rejected.
