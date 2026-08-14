# Phase 4 Slice 10c: `if`/`cond` as ordinary words

Retires the last compiler-known control-flow construct. `if` stops being a
bespoke `TermKind::If` node with dedicated checker and lowering arms and becomes
an ordinary **clause-bodied combinator over `Bool`**, spelled with 10a's row and
`~` machinery:

```sooth
: if ( ..i Bool ~[ ..i -- ..o ] ~[ ..i -- ..o ] -- ..o )
```

The surface syntax `cond if T else E end` is retained and desugared by the parser
into `cond ~[ T ] ~[ E ] if`; `TermKind::If` is deleted. `cond` ships fixed-arity,
written as nested `if`.

**Gates on 10a phases 1–4 only** (the `~` type variant, its surface syntax, rows
inside a nested quotation effect, and grounding at the check sites), all shipped
on `main`. It does **not** gate on 10a phase 5/6 (the self-tail back-edge rewrite:
`if` has no back-edge of its own), nor on 10b, nor on 11 — all of which are also
done on `main` (`e2648e0`). Line anchors below were verified at `e2648e0`;
re-anchor before editing.

## Why `if` is asymmetric where `times` is symmetric

10a's `times` carries `~[ ..s i64 -- ..s ]`: one row, provably the same on both
sides, because a loop body is a fixed point over the carried region (N=0 leaves it
untouched, N≥2 feeds iteration i's output into i+1's input). 10a's
`check_literal_against_declared_effect` bakes that in: it takes a single `row`,
seeds the literal's entry as `row ++ eff.inputs` and requires the exit to equal
`row ++ eff.outputs` (`src/check.rs:1424-1451`, "the carried region is a fixed
point").

`if` runs each branch **once**, so its branch genuinely transforms the region:
`~[ ..i -- ..o ]`, `..i ≠ ..o`. This is 10a's own note ("this is why 10c's `if`
legitimately takes an asymmetric `~[ ..i -- ..o ]`: it runs once"). Generalising
the fixed `row` into a separate `row_in`/`row_out` pair is the first block of
10c work (R1), and it is a *restatement* of 10a decision 2, not new row algebra:
the quotation's input-side row is the signature's `row_in`, its output-side row
its `row_out`. There is still no abstract row-to-row unification, because `~`
guarantees every branch is concrete by splice time and the join does the real
verification.

## What `~` buys here, restated against the shipped mechanism

Verified against `main`: a `~[ ... ]` value has no runtime representation, cannot
reach `materialize_quotation_at_boundary` or an `if`-join erasure site (rejected
by `Type` inequality, 10a R2/R3), and `call` on a `~` is statically always a
splice (`check_abstract_quotation_call`, `src/check/terms.rs:1126`). Two
consequences 10c depends on:

- **No abstract row unification.** Both branches of an `if` are checked against
  the *same concrete* `..i` region at a real call site; whatever they leave is
  `..o`, and the two must agree. The verification is ordinary stack-effect
  checking plus the existing arm-join equality, exactly as 10a promised.
- **No erasure-boundary problem.** A branch's row region cannot escape into a
  runtime `QuotEffect` (which has no row field), because a `~` cannot be
  materialised at all.

## Verified anchors (the brief's claims re-checked at `e2648e0`)

1. **Clause dispatch is an independent primitive.** `lower_clauses`
   (`src/ir/func_builder/control_flow.rs:194`) loads the scrutinee's discriminant
   and dispatches N-way; it is not built on `if`. With `Bool` a real enum
   (`BOOL_ENUM_ID`, slice 9), a clause-bodied word dispatching on it is the same
   mechanism every user enum eliminator uses. **No circularity.** Confirmed.
2. **The guard blocking a quotation-taking clause body is stale.**
   `clause_bodied_quotation_word_error` (`src/check/audits.rs:342`) is raised from
   `audit_word_quotation_positions` (`:239`) for a `w.poly.is_none()` clause word
   whose inputs name a quotation. Its own comment cites slice 7's runtime
   quotation value as the intended lift; 7a/7b shipped. **Stale, must be
   removed/relaxed** (R5).
3. **Splicing must cover clause bodies or the termination guarantee breaks.**
   `tail_position_calls`/`collect_tail_calls` (`src/check/drop_graph.rs:74-104`)
   and lowering's `body_tail_calls_self` (`src/ir/func_builder/mod.rs:99`) recurse
   into `TermKind::If` branches to find a self-tail call. After the desugar the
   branches live inside `~[ ... ]` literal arguments to a terminal `if` call, so
   the tail passes must recurse there instead, and the inliner must splice the
   matching branch **in tail position** — not via `call`, which pins `tail =
   false` (`src/check/terms.rs:~300`). `examples/gcd.sth` and
   `examples/countdown.sth` (`sum-to`, 1M iterations) both self-tail through `if`;
   `lib/combinators.sth`'s `while` (`| p | p call if p while else end ;`)
   self-tails through `if` *inside a combinator body*. **Load-bearing**
   (R2/R3/R7).
4. **Clause dispatch scrutinises `inputs.last()`** (`src/check/word_entry.rs:277`
   and `src/ir/func_builder/control_flow.rs:203`), which for `if`'s signature is
   the topmost branch quotation, not the `Bool`. Must relax to the topmost
   **enum-typed** input (R6). For every existing clause word the two coincide, so
   it is additive.
5. **`cond`'s variadic form stays blocked.** Clause dispatch is N-way on one
   scrutinee's variants; `cond`'s `[ pred ] [ body ]`-list form is N independent
   predicates, blocked on quotations-in-collections (slice 4 D4). `cond` ships
   fixed-arity as nested `if` (R11). Confirmed.

Two anchors the brief flagged as *new integration work*, both re-verified:

1. **The run-once capture asymmetry.** `check_literal_against_declared_effect`
   enforces D3 on a combinator's quotation parameter: it rejects a literal that
   consumes a linear enclosing local or leaves an enclosing borrow on its exit
   row (`src/check.rs:~1370-1420`, `quotation_captures_local_error` /
   `quotation_borrows_place_error`). Correct for `times` (runs f 0..N times);
   **wrong for `if`**, whose branch runs exactly once and today may freely consume
   an enclosing linear. So an `if` branch cannot be checked as an ordinary
   run-many combinator parameter; it needs the run-once splice path plus an
   arm-join (R2/R4).
2. **Splicing a clause combinator must join move/borrow state across its arms.**
   `check_clause_word` today checks each clause in a **fresh** `Scope`/`Provenance`
   (`src/check/word_entry.rs:428-430`), because a top-level clause word has no
   enclosing scope to reconcile. When `if` is *spliced* into a caller, its True/
   False arms are two runtime paths over the caller's live scope, so a linear
   consumed on one arm but not the other must join to `MaybeMoved`, and a borrow
   suspended on one arm must agree with the other — exactly today's
   `TermKind::If` join (`src/check/terms.rs:~980-1080`). This move/borrow join is
   not in the current clause path and is the largest single piece of new
   integration (R4).

## Decisions

**D1 — `if` is a compiler-injected clause combinator, not free-floating library
source.** `Bool` is compiler-injected (`BOOL_ENUM_ID`) and `if` is used
everywhere with no `import:`; injecting a synthesized `WordDef` for `if`
alongside the `Bool` enum keeps it available with zero prelude machinery, in
every module and the REPL. It is an *ordinary* `WordDef` running through the
generalised clause-combinator splice — **no bespoke checker or lowering arm**. The
only residual name-recognition is in the parser desugar (D2) and the syntactic
tail passes (R3), both of which encode the surface tail rule, not `if`'s
semantics. Rejected alternative: keep `TermKind::If` and merely re-*describe* it
as a word. That is the half-measure the slice exists to kill (`if` stays special,
the guard/scrutinee/splice work is unmotivated), and it is exactly the kind of
"feels hacked in" outcome that gets binned.

**D2 — the parser desugars, `TermKind::If` is deleted.** `parse_term`'s `if` arm
(`src/parser.rs:2220`) stops producing `TermKind::If` and instead emits two
`TermKind::Quotation`-flavoured **inline** literals (the `~` marker) followed by a
`TermKind::Call("if")`: `cond if T else E end` → `cond ~[ T ] ~[ E ] if`. A
missing `else` desugars its else-branch to `~[ ]`. The `TermKind::If` variant and
all 23 of its match sites (`src/{parser,resolve,repl,ast}.rs`,
`src/check/{terms,poly,drop_graph,engine,captures}.rs`,
`src/ir/func_builder/{mod,calls}.rs`) are removed; each site's behaviour is
re-expressed on the desugared shape (a `Call("if")` whose two preceding operands
are `~` literals) or, where it was pure `if`-node bookkeeping, deleted.

**D3 — branch literals are spliced run-once, D3-exempt, join-reconciled.** An
`if` branch parameter is *not* checked through the run-many
`check_literal_against_declared_effect` path. It is spliced inline like a `call`
(move-state tracked, borrows live, capture free) but preserving the ambient
`tail` flag, and the two arms' move/borrow state is joined by the clause
dispatch (R4). The `~[ ..i -- ..o ]` signature documents the effect and drives
combinator recognition; the per-arm output check plus the join do the
verification. The run-once property is a declared combinator attribute (R4), not
inferred, so `times` (run-many, D3 enforced) and `if` (run-once, D3 exempt) never
collide.

**D4 — `row_in`/`row_out` generalisation is a restatement of 10a decision 2, not
new algebra.** `check_literal_against_declared_effect` and the row-grounding
sites take the input-side and output-side rows separately (R1). No abstract
row-to-row unification, no `Subst` extension, no mangling impact — `~` keeps every
row concrete at splice time.

**D5 — `if`'s standalone check is trivial and needs no new grounding.** At `if`'s
injected definition there is no caller, so `..i`/`..o` ground to the empty region
(10a's existing "size-zero during a body check" model). Each clause arm splices
its branch param, which is `~[ -- ]` there, leaving the empty declared output.
The brief's worry ("the row transformed because a trusted call to a `~` parameter
says so is new integration work") does **not** bite for `if`: `if`'s body never
touches the row directly, it only splices the branch params, so the standalone
check is the trivial one the brief predicted. The row transformation happens only
at real call sites, where the region is concrete and the arm-join derives `..o`
(D3). This is verified, not assumed: `: shrinks-row ( ..a i64 -- ..b ) drop drop
;` still fails (a body that touches the row), and `if`'s body is not that shape.

**D6 — `cond` is fixed-arity nested `if`, injected the same way** (R11). Its
variadic predicate-list form is out of scope (slice 4 D4).

**D7 — de-risk order: capability first on a user combinator, `if` migration
last.** Requirements R1–R4 (the row generalisation, the run-once splice, the
scrutinee relaxation, the arm-join) and R5 (the stale guard) land and are proven
on a **user-defined** clause combinator that dispatches on a user enum and
splices a quotation branch in tail position, before `if` itself is migrated and
`TermKind::If` removed (R7–R9). The migration then rests on a proven mechanism;
if it surfaces a blocker, the capability has still shipped and `if` can hold at
`TermKind::If` for one more slice. This is the brief's own paper-pre-check
discipline applied to sequencing.

## Requirements

"Located" = the diagnostic carries a span and names the offending row/argument/
word and the declared signature. Every new guard is mutation-tested (R13): prove
its golden flips when the guard is deleted.

### The mechanism (proven on a user combinator: phases 1–5)

**R1 — separate `row_in`/`row_out`.** `check_literal_against_declared_effect`
(`src/check.rs:1354`) takes the input-side row and output-side row as two
parameters. Seed `fresh = row_in ++ eff.inputs`; require the exit to equal
`row_out ++ eff.outputs`; strip `row_in.len()` slots before rendering a mismatch
(as today, so the caller's stack never leaks into the printed effect). The single-
`row` callers (times, the `if`-join erasure sites, the mono/empty-region sites)
pass `row_in == row_out`, byte-unchanged. `PolyType::Quotation`'s existing
`row_in`/`row_out` fields (10a R7) feed the two arguments; a differing pair is
legal now (10a's `a loop body cannot change the shape of the carried region`
rejection is lifted **only** for a combinator declared run-once, R4, and its note
`10c lifts this for a word without a back-edge` is discharged). A same-row
combinator still rejects a differing pair. Unit test: a run-once combinator with
`~[ ..a i64 -- ..a ]` (drops the `i64`) checks; the same as run-many rejects.

**R2 — the run-once branch splice, tail-preserving.** A quotation parameter of a
combinator marked run-once (R4) is spliced inline against the live stack via the
`call`-splice machinery (`check_terms_relaxed`, capture-free, move-tracked), with
the `tail` flag threaded from the enclosing position rather than pinned `false`.
Reached when a clause arm of a spliced clause combinator mentions the parameter
in terminal position. A self-tail call inside such a branch is therefore a
back-edge, not a re-splice. Lowering (`src/ir/func_builder`) splices identically,
preserving the loop shape. Witness on the user combinator: a self-tail clause
combinator whose recursion sits inside a spliced branch runs in constant stack at
`ulimit -s 1024` (`run_at_stack_limit`).

**R3 — the syntactic tail passes recurse through a terminal `if`'s branch
literals.** `collect_tail_calls` (`src/check/drop_graph.rs:88`) and
`body_tail_calls_self` (`src/ir/func_builder/mod.rs:99`): a terminal
`Call("if")` whose two preceding siblings are `~` literals hands tail position to
the last term of each literal's body, recursively (the exact rule the deleted
`TermKind::If` arm encoded). `has_self_tail_call` and `check_tail_call_cycles`
inherit it. Regression pin: `gcd`/`sum-to`/`while` keep their self-tail loop.
This is the one place `if` is recognised by name in a *pass* rather than the
parser; it encodes the surface tail rule, not `if`'s dispatch.

**R4 — clause-combinator splice: run-once attribute + arm-join.** Three parts:

- *Recognition.* `is_combinator`, `collect_combinators`, and `combinator_of`
  (`src/check/combinators.rs:44-78`) accept a `WordBody::Clauses` word that
  declares a quotation parameter (today all three are `WordBody::Terms`-only).
  `inline_combinator` (`:~300`) grows a clause-splice path beside its terms path.
- *Run-once splice with join.* Splicing a clause combinator lowers its scrutinee
  discriminant and splices each arm against a clone of the caller's live scope,
  then reconciles: `scope.moves = Moves::join(arm states)`, borrow suspension must
  agree across arms (reusing `borrow_join_disagreement_error`), and each arm's
  output stack must match (reusing the branch-length/type checks). A quotation
  parameter mentioned in an arm splices run-once (R2), D3-exempt. This is the
  `TermKind::If` join generalised to N clauses over a spliced scope.
- *Alpha-rename.* `alpha_rename_locals` / `rename_terms` (`src/ast.rs:1270`)
  descend into `WordBody::Clauses` bodies and their `| names |`, so a spliced
  clause combinator's locals cannot collide with caller locals under transitive
  inlining. (Today `rename_terms` handles `TermKind::If`; that arm goes, a
  clause-body arm arrives.)

**R6 — scrutinee is the topmost enum-typed input.** `check_clause_word`
(`src/check/word_entry.rs:277`) and `lower_clauses`
(`src/ir/func_builder/control_flow.rs:203`) select the deepest-to-shallowest
**enum-typed** (or `&Enum`/`&!Enum`) input as the scrutinee, and treat every
input below and above it (other than that one) as carried context. A word with no
enum-typed input keeps today's "top input is not an enum" located error. A word
with two enum-typed inputs picks the topmost and is unambiguous. For every
existing clause word the topmost input is the only enum, so the selection is
identical — pinned by an unchanged-output regression over `examples/shapes.sth`.

**R5 — remove the stale clause-bodied-quotation guard.** Delete
`clause_bodied_quotation_word_error` and its call in
`audit_word_quotation_positions` (`src/check/audits.rs:239-243, 342`). A
clause-bodied word taking a quotation is now a supported combinator (R4). The
`main`/output/nested-in-effect audits in the same function are retained. Mutation
evidence: the deleted rejection's former golden now *accepts*; a positive test
(the user combinator) compiles and runs where it previously errored.

### `if`/`cond` migration (phases 6–8)

**R7 — desugar and delete `TermKind::If`.** D2: the parser emits `~[ T ] ~[ E ]
if`; the `TermKind::If` variant and all 23 match sites are removed, each
re-expressed on the desugared shape or deleted. `resolve.rs` name-resolution and
`repl.rs`'s If-rewrite fold into the general `Call`/`Quotation` handling. Golden:
the parser produces the three desugared terms for `if T else E end` and `if T
end`.

**R8 — inject `if` and `cond`.** A synthesized `if` `WordDef` (the D1 clause
combinator, run-once, over `Bool`) is added to every module and the REPL session
store alongside the `Bool` enum, marked run-once. `cond` is a synthesized
fixed-arity nested-`if` combinator (D6). Both are ordinary combinators the splice
path already reaches after R4; neither adds a checker/lowering arm. The injected
`if` body is written so its standalone check is trivial (D5). REPL retention
(`eval_def` path) keeps them the way it keeps `times`.

**R9 — behavioural and codegen equivalence.** The whole corpus, `while`, and the
`lib/combinators.sth` combinators compile and run byte-identically in **stdout and
exit code**. `examples/gcd.sth` prints `5`, `examples/countdown.sth` prints
`500000500000` in constant stack at `ulimit -s 1024`, `examples/factorial.sth`
(Phase 0 golden) unchanged. Poly bodies that branch (`choose`/`mymax`/`mymax3`,
slice 6e) compile and run at each instantiation with `if` now a spliced
combinator in the poly walk rather than a `TermKind::If` arm — the poly-body If
handling (`src/check/poly.rs:372`) is removed and re-reached through the poly
combinator call path with `..i`/`..o` grounded to the poly stack region. The
`tests/qbe_baseline*` `.ssa` goldens are **regenerated** (the discriminant-dispatch
lowering of a `Bool` clause word is not guaranteed byte-identical to the deleted
`lower_if`'s direct `jnz`); the regeneration is the deliverable, pinned as such,
and diffed to confirm only the `if`-dispatch shape changed and no per-iteration
indirect call or `blit` appeared in a self-tail loop body.

**R10 — diagnostics preserved.** The `if` condition-type and
quotation-as-condition messages (`src/check/terms.rs:713-717`) re-emerge as the
injected `if`'s scrutinee mismatch (`Bool` expected) through the ordinary clause
scrutinee/argument checks; pinned by exact text so a `5 if ... end` still names
`if` and `Bool`, not a raw enum-scrutinee message.

**R11 — `cond` ships fixed-arity.** A witness `: pick3 ( i64 -- Str ) ... ;`
written with nested `cond`/`if` compiles and prints the right arm. The variadic
`[ pred ] [ body ]`-list form is a located "not yet" only if attempted, pointing
at slice 4 D4.

**R12 — no regression, `~` non-materialisation intact.** A `~` branch literal
still cannot be stored, returned, or reach an erasure boundary (10a R2/R3 unbroken
by the desugar); a `[ ... ]` (non-`~`) in an `if` position is a located
type-mismatch against the `~[ ... ]` parameter. `lib/combinators.sth` stays
byte-unchanged except where R9 requires (it should require nothing: `while`'s `if`
is surface syntax the parser desugars, the source text is untouched).

**R13 — mutation-test every new guard.** R1's differing-row acceptance, R4's
arm-join disagreement, R6's scrutinee selection, R5's removal, R10's scrutinee
message: each proven capable of failing.

## Phased delivery plan

1. **Row_in/row_out generalisation** *(standard)* — R1. Split the single `row`
   in `check_literal_against_declared_effect` and its grounding callers into
   input-side/output-side; same-row callers pass both equal; a run-once combinator
   may declare a differing pair. Unit tests for the differing-row accept and the
   run-many reject. Nothing user-visible ships yet; this is the substrate.
2. **Run-once attribute and D3-exempt splice** *(hard)* — R2 (check side) + R4's
   run-once attribute and splice-with-join skeleton, exercised through a
   *term-bodied* run-once combinator first (no clause dispatch yet), proving the
   tail-preserving splice and the D3 exemption in isolation.
3. **Clause-combinator recognition, splice, and arm-join** *(hard)* — R4 full
   (`is_combinator`/`collect_combinators`/`combinator_of`, `inline_combinator`'s
   clause path, `alpha_rename_locals` into clause bodies, the move/borrow/output
   arm-join) + R6 (scrutinee relaxation) + R5 (remove the stale guard). Proven
   end-to-end on a user-defined self-tail clause combinator over a user enum that
   splices a branch in tail position and runs in constant stack. Lowering
   (`lower_clauses` splice, `body_tail_calls_self`) lands here for that combinator.
4. **Tail passes through the desugared shape** *(standard)* — R3. Teach
   `collect_tail_calls`/`body_tail_calls_self`/`has_self_tail_call`/
   `check_tail_call_cycles` the terminal-`if`-over-`~`-literals tail rule, tested
   directly on the desugared term shape (constructed, not yet emitted by the
   parser).
5. **Capability exit witnesses** *(standard)* — the user combinator sums, self-
   tails in constant stack, carries an aggregate without aliasing, nests, and
   rejects a linear over-capture; every phase-1–4 guard mutation-audited. This is
   the point past which the mechanism is proven independently of `if`.
6. **Parser desugar and `TermKind::If` deletion** *(hard)* — R7. Desugar in
   `parse_term`; delete the variant and re-express all 23 sites; fold `resolve`/
   `repl` If-handling into `Call`/`Quotation`. Parser golden for the desugared
   output; the tree still builds with the node gone.
7. **Inject `if`/`cond`; poly-body reroute** *(hard)* — R8 + R9's poly half + R10.
   Synthesize and inject the `if`/`cond` `WordDef`s; remove `poly_term`'s `If` arm
   and re-reach branching poly bodies through the combinator path; preserve the
   scrutinee diagnostics. `choose`/`mymax`/`mymax3` and the mono corpus check.
8. **Corpus, baseline, and mutation audit** *(standard)* — R9 behavioural
   equivalence (stdout/exit byte-identical; gcd/countdown/factorial; constant
   stack), R11 (`cond` witness), R12 (`~` non-materialisation, non-`~` rejection,
   library byte-unchanged), regenerated `qbe_baseline` `.ssa` diffed for
   dispatch-only change, and an audit enumerating every located error with its
   mutation evidence (R13).

## Exit criteria

10c exits when: `check_literal_against_declared_effect` and the row-grounding
sites carry a separate input/output row, a run-once combinator may declare a
transforming `~[ ..i -- ..o ]` and a run-many one still may not (R1); a run-once
combinator's quotation parameter splices inline, tail-preserving and D3-exempt,
and a self-tail call inside such a splice is a constant-stack back-edge (R2); the
syntactic tail passes recurse through a terminal `if`'s `~` branch literals (R3);
`is_combinator`/`collect_combinators`/`combinator_of`/`inline_combinator`/
`alpha_rename_locals` accept a clause-bodied combinator and splice the matching
arm with move/borrow/output state joined across arms (R4); the clause scrutinee is
the topmost enum-typed input (R6); the stale `clause_bodied_quotation_word_error`
is gone and a clause-bodied quotation-taking word compiles and runs (R5); the
parser desugars `if…else…end` into `~[ T ] ~[ E ] if` and `TermKind::If` and its
23 sites are deleted (R7); `if`/`cond` are injected clause combinators with no
bespoke checker or lowering arm, branching poly bodies reroute through the
combinator path, and the `if`-scrutinee diagnostics survive (R8/R10); the whole
corpus, `while`, and the library run byte-identically in stdout and exit code with
gcd/countdown/factorial intact and countdown constant-stack, the `qbe_baseline`
goldens regenerated and diffed to a dispatch-only change (R9/R12); `cond` ships
fixed-arity (R11); and every new guard has been shown capable of failing (R13).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "row_in row_out generalisation", "difficulty": "standard" },
    { "phase": 2, "focus": "run once attribute and d3 exempt splice", "difficulty": "hard" },
    { "phase": 3, "focus": "clause combinator recognition splice and arm join", "difficulty": "hard" },
    { "phase": 4, "focus": "tail passes through the desugared shape", "difficulty": "standard" },
    { "phase": 5, "focus": "capability exit witnesses", "difficulty": "standard" },
    { "phase": 6, "focus": "parser desugar and termkind if deletion", "difficulty": "hard" },
    { "phase": 7, "focus": "inject if cond and poly body reroute", "difficulty": "hard" },
    { "phase": 8, "focus": "corpus baseline and mutation audit", "difficulty": "standard" }
  ]
}
```
