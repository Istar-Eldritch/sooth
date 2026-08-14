# Phase 4 Slice 10c: `if` as an ordinary library word (spec)

Retire the last compiler-known control-flow construct. After this slice the
compiler knows three machine-level primitives (`branch` on a 32-bit flag, `tag`
reading an `is_scalar` enum's discriminant, and machine-word comparisons); `if`,
`unless` and `while` are ordinary words in `lib/`, and `TermKind::If` plus the
`if`/`else`/`end` grammar are gone.

This spec is written against the brief (`docs/phase4-slice10c-brief.md`), which is
the discovery document. Its **Decisions** section is settled and is not reopened
here. This spec resolves its **Open questions for the spec** and turns the three
settled phases into implementable requirements, exit criteria, and per-criterion
mutation tests.

The two probe worktrees (`probe/tailsplice`, `probe/rowgate`) are throwaway
evidence. Their line counts (recon 3: ~72 lines P1; recon 6: ~36 lines P2) are a
sanity check on effort, not a starting point; P1/P2 are implemented properly, not
by promoting the patches.

---

## Codebase map (verified against `main` at `c44d552`)

Every anchor below was re-verified in the tree at `c44d552`. None had moved
materially. Two line-precision clarifications are noted inline; report those, not
the brief's approximations, when editing.

**Control flow / the primitive to delete**

- `src/check/terms.rs:702` — the `TermKind::If` checker arm; `:716` is the
  `cond.ty != Type::BOOL` guard (the layering violation this slice removes).
- `src/ir/func_builder/control_flow.rs:99` — `lower_if`; becomes `branch`'s
  lowering essentially unchanged (jnz-on-a-machine-word + join).
- `src/parser.rs:2238` — `TermKind::If` construction (the `if`/`else`/`end`
  grammar); `:2222`ff parses it. To be deleted.
- `TermKind::If` has 23 references across `src/` (resolve.rs, ir/func_builder,
  repl.rs, ast.rs, check/terms.rs, check/drop_graph.rs, check/engine.rs,
  check/captures.rs, check/poly.rs, parser.rs). Deleting the variant deletes all
  of them; the P3 exit `grep -r "TermKind::If" src/` == empty enforces it.

**Enum / discriminant (for `tag`)**

- `src/ir/layout.rs:264`,`:480` — `EnumWord::Construct` is the *only* enum word
  (recon 1: no discriminant-read word exists today).
- `src/ir/layout.rs:641` — `is_scalar` (every variant payload-free); a scalar
  enum value *is* its discriminant, so `tag` is a no-op cast there.
- `src/ir/func_builder/control_flow.rs:225`ff (`scrutinee_is_value`) — confirms a
  non-reference scalar-enum scrutinee is already the bare discriminant.
- `src/ast.rs:302` (`BOOL_ENUM_ID`), `:310` (`bool_enum_decl`), `:1050`
  (`Type::BOOL = Type::Enum(BOOL_ENUM_ID, "bool")`) — `bool` is the injected
  library enum from slice 9 (`False`=0, `True`=1, both payload-free ⇒ scalar).

**Comparisons**

- `src/ir/func_builder/calls.rs:432` — the `=`/`<`/`>`/`<=`/`>=`/`<>` builtin arm;
  emits `Instr::Cmp` into an `IrType::Bool` value (already 32-bit, the same width
  the new flag uses) and the checker types the result `Type::BOOL`.

**Tail-splice recognition (decision 8: the shared predicate)**

- `src/check/drop_graph.rs:87` — `collect_tail_calls` (syntactic pass, `If`-only
  descent today); `:130` — `has_self_tail_call`.
- `src/ir/func_builder/mod.rs:99` — `body_tail_calls_self` (lowering twin,
  `If`-only descent). Its `:84-98` doc-comment (the brief says `:95-98`; the block
  actually spans `:84-98`) is the standing warning that check and lowering must
  agree on whether a splice is a loop.
- `has_self_tail_call` call sites (four): `src/ir/driver.rs:183` (per-word build
  gate), `src/ir/driver.rs:643` (REPL path), `src/ir/destructors.rs:372`
  (destructor path), `src/check/combinators.rs:343` (`splice_tail`, the checker
  twin of the lowering `tail` flag — the exact site at which check and lowering
  would disagree once a user combinator can self-tail through a splice).
- `body_tail_calls_self` call site: `src/ir/func_builder/calls.rs:520`
  (`self_tail` for the combinator splice). The `tail = false` pin the brief cites
  at `calls.rs:~519` resolves at `main` to two places: the call-of-literal splice
  `lower_terms(&body, false)` at `calls.rs:316` (load-bearing comment `:300-302`),
  and the combinator splice gated by `self_tail` at `calls.rs:520`/`:528`. Report
  these two, not a single `~519`.
- `src/ir/driver.rs:243` — at `main` this is a *comment* (`reusing
  has_self_tail_call ...`), not the spike's `spliced_self_tail` call. No spike code
  is on `main`.

**Row gate (P2)**

- `src/parser.rs:794` — `quotation_row_not_top_level_error` (10a R4).
- `src/parser.rs:813` — `quotation_row_shape_change_error` (10a R5), fired at
  `src/parser.rs:1374`. Its own doc-comment already reads "only 10c, for a word
  without a back-edge, lifts this."

**Clause dispatch (unchanged; recon 1 / decision 5)**

- `src/check/audits.rs:340` (`clause_bodied_quotation_word_error`),
  `src/parser.rs:1234` (`effect_has_variable` true on `TildeLBracket`),
  `src/check/poly.rs:158`/`:253` (clause body + polymorphic signature rejected) —
  the pincer that makes a user-written enum eliminator impossible. Not touched.

**Linear spine (P1)**

- `src/check.rs:2491` (`check_linear_value_across_self_tail_call_is_error`),
  `:2507` (`check_linear_value_forwarded_into_the_self_tail_call_is_ok`) — the
  guards to extend to the spliced back-edge.

**DESIGN.md**

- `:453-456` and the "irreducible core" list (~`:485-487`) name clause-bodied
  definition as the sole enum eliminator — decision 5 keeps this true.
- `:464-475` describes `if` as an *ordinary clause-bodied word dispatching on its
  Bool input*. That is the design recon 1 killed; it must be amended (see
  "DESIGN.md amendment").
- `:471-472` ("shrinks the core the honest way, by making `if` a word …") stays.
  Note `:469` is **not** that sentence: it is `written as an ordinary clause-bodied
  word dispatching on its`Bool`input`, part of the killed design that OQ7 edit 1
  must delete. Match on the quoted phrase, not the line number.

---

## Resolutions of the open questions

### OQ1 — the tail-spliced parameter analysis, properly

The spike's rule is one-level-deep and positional (caller's *last* term is a
combinator call; the quotations are a contiguous trailing run of literals). The
real rule is a per-combinator *tail-called-parameter set*:

- For each combinator `C`, compute the set of `C`'s declared quotation parameters
  that are `call`ed in **tail position** of `C`'s body. "Tail position" is the
  existing positional rule (`lower_terms`' `tail && i == last`, and each arm of a
  two-way branch inherits it). A parameter that is `call`ed and then followed by a
  further term (recon 4's `t call e drop`, whose tail term is `drop`) is **not** in
  the set.
- A word `W` is a *spliced self-tail* iff `W`'s body tail term resolves to a
  combinator `C`, and `W` passes into some slot in `C`'s tail-called-parameter set
  a quotation **literal** whose own tail term is a call to `W`.

Answers to the four cases the brief demands:

- **Nested combinators.** The relation is transitive along the tail-called-param
  edges: if `C`'s tail term is itself a call to combinator `D` and forwards one of
  `C`'s own quotation params into `D`'s tail-called-param set, then that param
  stays in `C`'s set. Follow only those edges.
  **The closure must carry a visited set and decline (no back-edge) on a cycle.**
  The inline-always invariant proves *lowering* terminates; it does not prove this
  *static* walk terminates, because the walk follows edges between distinct
  combinator nodes and two combinators mutually forwarding a tail-called parameter
  would loop `C → D → C`. Whether that shape is constructible is beside the point:
  the visited set is one line, is self-evidently terminating, and declining on a
  cycle is the same conservatism R-P1-4 already applies to an ambiguous name.
  Declining is always safe: it costs a loop transform, never correctness.
- **A combinator call that is the tail of an `if`/`branch` arm in the caller.**
  This *is* the `sum-to` case once P3 lands: `... [ base ] [ acc n + n 1 - sum-to ]
  if` has `if` as its tail term, and `if`'s tail-called-param set is both branch
  quotations (`if`'s body ends by `branch`ing to a `call`ed param — see P3). So the
  branch literal carrying the self-call inherits tail position through `if`. The
  same rule covers a hand-written combinator over the primitive `if` in P1/P2. The
  recon-4 negative (`t call e drop`) does not, because `drop` is the tail term.
- **A quotation argument that is forwarded rather than a literal.** Two cases that
  look alike ("a quotation reached through a local") and must be decided **opposite
  ways**. Getting this wrong is silent: decide it too tightly and nothing self-tails,
  which shows up only as a stack overflow.
  - *A local bound to the word's own declared `~` parameter is **followed**.* Resolve
    it back to that parameter slot and treat it as param-forwarding. This is not an
    edge case, it is the mechanism the whole slice rests on: `if`'s body is
    `| e | | t | | c | c tag t e branch`, where `t` and `e` are locals holding `if`'s
    own `~` parameters. If they were opaque, `if`'s tail-called-param set would be
    empty and `gcd`/`sum-to` would lose their loops (E-P3-5).
  - *A local bound to a caller-supplied or materialized value is **opaque**, and the
    analysis **declines** (no back-edge).* The body is not statically visible. This
    matches lowering, where a materialized value goes through `lower_indirect_call`,
    not the splice branch (recon 3, R-P1-7).
- **A name that resolves to more than one combinator.** Resolve the combinator
  identity through the checker's existing call-site resolution
  (`check_term`/`inline_combinator`), not a bare-name table lookup. If the name is
  ambiguous / resolves to more than one candidate, **decline** (the same
  conservatism `has_self_tail_call` already applies via `symbols[idx] == w.name` at
  `driver.rs:183`).

### OQ2 — linear values across a spliced back-edge

Extend the two guards (`check.rs:2491`, `:2507`) to the spliced case, with the
**same rule as the direct self-tail**:

- A non-`Copy` value **live across** the spliced back-edge is **rejected** (mirror
  `check_linear_value_across_self_tail_call_is_error`). No drop glue is inserted
  across the spliced back-edge; rejection keeps the spliced case identical to the
  direct case rather than introducing a larger back-edge-disposal feature.
- A linear value **forwarded into** the self-tail call is **accepted** (mirror
  `..._forwarded_..._is_ok`).

Because the rule lives in the one shared predicate (decision 8), this falls out for
free once the predicate recognizes the spliced self-tail: the existing linear-spine
check runs over the recognized back-edge. Phase 2 makes every value `Copy`, so
this path is otherwise untested; P1 must add a **non-`Copy`** carried-value test to
exercise it (see E-P1-5).

### OQ3 — where the inline-always invariant is written down

State it as a named, owned invariant, not something rediscovered:

> **INV-INLINE-COMBINATOR.** A quotation-taking word is always inlined (spliced)
> at each call site and mints no `IrFunc`; it has no opaque call form. Its declared
> output row is discovered by forward checking of the spliced terms, never solved
> for by row unification.

- Written in DESIGN.md (control/quotation section) as a current-state invariant.
- Restated as a doc-comment on the shared tail predicate (`drop_graph.rs`) and the
  combinator splice (`calls.rs`), the two sites whose soundness rests on it.
- Owner: the combinator-splice path. Named risk: slice 7b (first-class runtime
  quotations) is where it breaks; the invariant comment must say 7b revisits it.

### OQ4 — `tag`'s domain

`is_scalar` enums **only**, where `tag` is a no-op cast (the value already is its
discriminant). `tag` applied to a payload-carrying enum is a **located compile
error** (a real field read is a larger feature not needed by this slice). This is
the minimal domain and keeps the primitive honest.

### OQ5 — comparisons

**Resolution: the first option (comparisons survive as library words).** Add six
comparison primitives spelled `u=`, `u<`, `u>`, `u<=`, `u>=`, `u<>` (each returns a
32-bit unsigned flag, `0` or `1`), and make `=`, `<`, `>`, `<=`, `>=`, `<>` ordinary
`lib/` words that wrap them and yield a `bool`. The `u` prefix marks the raw-flag
primitive layer (the unsigned 32-bit result), **not** an unsigned-*operand* variant:
each primitive is the single comparison of its shape and derives signed / unsigned /
float behaviour from its operand type exactly as today's `CmpOp` does at lowering
(`src/backend/qbe.rs:917`, the `Instr::Cmp` arm: `<`/`>` pick `cslt`/`csgt` vs
`cult`/`cugt`, floats use the ordered forms, `=`/`<>` are sign-agnostic), so there is
deliberately no separate `s<`/`f<` sibling. `=`/`<>` are sign-agnostic; `<`/`>`/`<=`/
`>=` are sign- and float-sensitive; all of it falls out of the operand type,
mirroring the per-type builtin rows at `src/check/builtins.rs:146`.
Rationale:

- It is the layering decision 1 mandates: the machine layer is exactly `{branch,
  tag, comparisons}` and every typed construct (`bool`, `if`, `=`) is library.
  Keeping `=` as a bool-returning *builtin* would leave a fourth comparison-shaped
  thing in the compiler.
- It gives the comparison primitives a real consumer in the same phase they are
  added, so they are not pre-staged dead plumbing.
- The corpus reads unchanged (the brief's own "reads much better"), and it costs
  nothing at run time **provided the library comparison words are declared `inline`**
  (R-P3-3a). Measured: with `inline`, the canonical `a b = if` compiles to the same
  instruction sequence as today's builtin, because QBE folds the branch-and-construct
  diamond to a branchless conditional move; without it, every comparison becomes a
  real call with a frame.

**The emitted IR is NOT byte-identical, and this spec must not claim it is.** An
earlier draft argued that a comparison's flag result and a `bool` value 'share one
representation, so the step is a no-op at lowering (the same `Instr::Cmp` value)'.
That was wrong twice over: `branch` lowers to a jump-and-join with a phi and the
backend has no folding pass, so a library `=` is `Cmp` **plus a diamond** in IR; and
the premise was width-false as originally written, since `IrType::Bool` is 32-bit
while a target-width `usize` is 64-bit (`qbe.rs:279-300`). Both are resolved: the
condition type is `u32` (R-P3-1), which removes the width gap, and equivalence is
asserted at the machine-code level by E-P3-4 part 3 rather than at the IR level. The
only way to get byte-identical IR would be the `usize → enum` reinterpret this
section goes on to reject, so asserting it would be self-defeating.

**The `usize → bool` step needs no new operation, and must not get one.** This
spec's first draft called for an explicit retype, "the typed dual of `tag`", on the
grounds that `EnumWord` holds only `Construct(id, variant_idx)` (nullary variant
constructors) with no "reinterpret a machine word as this enum". The first half of
that is correct; the conclusion is not. A comparison wrapper constructs its result
by **branching and naming a variant**, which is exactly the "ordinary enum
construction" decision 4 refers to:

```sooth
: =  inline ( 'T: Copy Ord 'T -- bool )  u= [ true ] [ false ] branch ;
: if ( ..i bool ~[ ..i -- ..o ] ~[ ..i -- ..o ] -- ..o )  | e | | t | | c | c tag t e branch ;
```

`u=` leaves a 32-bit flag, `branch` consumes it and two quotations that each
construct a nullary variant. Measured on `probe/rowgate` with today's `if` standing
in for `branch`: `: eqb ( i64 i64 -- bool ) = [ true ] [ false ] myif ;` checks,
lowers, and yields the right values through a row-polymorphic combinator; adding
`inline` makes the emitted machine code byte-identical to the primitive version,
while omitting it emits a frame and a `call`.

**A flag→enum retype is therefore rejected for this slice**, and not merely as
unnecessary. It is a *partial* operation dressed as a total one: `tag` is total
(every enum value has a valid discriminant) while its apparent inverse is not (not
every machine word is a valid discriminant), so it can manufacture a `bool` whose
discriminant no clause-dispatch compare chain matches. Rust draws this line in the
same place: `E::A as usize` is safe for a field-less enum, and there is no safe
builtin in the other direction at all, only a hand-written fallible conversion.
Branch-and-construct makes the invalid value *unconstructible* rather than
discouraged, so the three-primitive envelope stands with no fourth operation.

**Note for a future slice, not built here.** The general integer→enum conversion
(parsing a byte into a command enum) should be **branch-shaped**, never
value-shaped, for the same reason and one more: it supplies the target enum type,
which a bare `( usize -- 'E ... )` cannot infer from an integer.

```sooth
untag ( ..i usize ~[ ..i 'E -- ..o ] ~[ ..i usize -- ..o ] -- ..o )
```

Its natural library wrapper (`[ Ok ] [ Err ] untag`) additionally needs **generic
enums**, which do not exist: `parse_enum_typedef` (`src/parser.rs:2018`) reads a
name and goes straight to variants with no type parameters, and the corpus already
works around it by hand (`examples/shapes.sth:6`, `type: MaybeInt | None | Some v
i64 ;`, a specialized `Option<i64>`). Out of scope; recorded so the shape is not
re-derived value-first later.

### OQ6 — which library words move in P3

- **`if`, `unless`: ship as new `lib/` words** (required). Neither exists today.
- **`while`: stays in `lib/combinators.sth` but its body is rewritten.** Its
  current body uses the `if`/`else`/`end` grammar, which P3 deletes, so it *must*
  change to postfix `if`. It is not a new word, just a migration.
- **Every existing `if`/`else`/`end` site in `lib/` and `examples/` is migrated**
  to postfix `[ T ] [ E ] if` (this is the "corpus migration" part of P3): `lib/`
  = `arrays.sth`, `combinators.sth` (`filter`, `times-helper`, `while`);
  `examples/` = `array_ctor.sth`, `bool_abi.sth`,
  `countdown.sth`, `factorial.sth`, `fill_relower.sth`, `filter_while_hand.sth`,
  `filter_while.sth`, `gcd.sth`, `leap.sth`, `list.sth`, `poly_if.sth`, `refs.sth`,
  `sign.sth`, `vm.sth`, `vm_table.sth`.
- **NOT migration targets, despite appearances:** `lib/binary_search.sth` and
  `lib/uart_mmio.sth`. Both are **untracked** working-tree sketches, absent from
  `c44d552` (`git ls-tree c44d552 lib/` is `arrays.sth` + `combinators.sth` only),
  and both already use **postfix** `if`, so neither contains `if`/`else`/`end`
  grammar to migrate. `binary_search.sth` is additionally a self-declared
  hypothetical sketch that this spec's own Out-of-scope section says stays
  unbuildable after this slice. An earlier draft listed both, having read the
  working directory rather than the reviewed tree.
- **`cond`: NOT shipped this slice.** It does not exist in `lib/`. A `cond` that
  takes a variable number of `[ pred ] [ body ]` pairs is not a fixed-arity word
  and cannot be an ordinary library word without variadics (out of scope). DESIGN's
  mention of `cond` is left as a documented future word; the amendment must not
  over-promise it as shipping here.

### OQ7 — DESIGN.md's amendment

Current state only, no history (per project convention). Three edits:

1. Replace the `:464-475` claim that `if` is a *clause-bodied* word dispatching on
   `Bool` (the killed design) with: `if`/`unless` are ordinary **term-body
   combinators** over the `branch` and `tag` primitives; `bool` is a library enum
   with no special status; `if` takes a `bool`, reads its discriminant with `tag`,
   and `branch`es on the resulting flag.
2. Add the machine/library layer split and the three primitives (`branch`, `tag`,
   comparisons over a 32-bit flag) to the conditionals section.
3. Add **INV-INLINE-COMBINATOR** (OQ3) to the control/quotation section.

Keep `:453-456` and the irreducible-core list (clause-bodied definition as the sole
enum eliminator) unchanged; keep the `:471-472` sentence unchanged. Do not change DESIGN's `cond`
sentence beyond removing any implication that `cond` ships in 10c.

---

## Phase P1 — shared tail-splice recognition

**Value on its own:** fixes recon 2's latent segfault (a self-recursive word whose
recursive call sits inside a quotation handed to a combinator is not recognized as
self-recursive today and blows the host stack). This is a correctness bug on `main`,
independent of the rest of the slice.

P1 uses the *existing* primitive `if`; it does not need `branch`/`tag`. Its test
combinators are hand-written over the primitive `if`, exactly as the tailsplice
probe's `Bool?`.

### Requirements

- **R-P1-1 (one rule).** Implement OQ1's tail-called-parameter-set rule so that
  `has_self_tail_call(W)` returns true for a spliced self-tail. Generalize
  `collect_tail_calls` (`drop_graph.rs:87`) and `body_tail_calls_self`
  (`func_builder/mod.rs:99`) to descend into a combinator-tail-called quotation
  literal, from their current `If`-only descent.
- **R-P1-2 (discrimination).** The rule follows only quotation-parameter slots
  `call`ed in tail position. Recon 4's `t call e drop` (tail term `drop`)
  contributes no tail position and stays ordinary recursion.
- **R-P1-3 (literals only).** A forwarded / non-literal quotation slot is opaque;
  the analysis declines. See OQ1.
- **R-P1-4 (resolution).** Resolve the combinator at the call site through the
  checker's existing resolution, not a bare-name lookup; decline on ambiguity.
- **R-P1-5 (single predicate, six sites).** All consumers share one predicate so
  check and lowering agree by construction: the two syntactic passes
  (`collect_tail_calls`→`has_self_tail_call`, `body_tail_calls_self`), the four
  `has_self_tail_call` sites (`driver.rs:183`, `driver.rs:643`,
  `destructors.rs:372`, `combinators.rs:343`), and the lowering splice gate
  (`calls.rs:520`). `combinators.rs:343` (`splice_tail`) is the site that currently
  diverges from lowering and must be driven by the same predicate.
- **R-P1-6 (lowering).** Thread the real `tail` flag through the call-of-literal
  splice (`calls.rs:316`) and the combinator splice (`calls.rs:520`/`:528`), gated
  on the shared predicate having sanctioned this splice. The whole-word back-edge
  (`name == cur_word_name` in `lower_call`) then fires with no new phi/CarriedSlot
  machinery (recon 3): the branch quotations are phantom literals spliced inline,
  so their locals resolve to the whole-word header phis.
- **R-P1-7 (linear spine).** Extend the guards per OQ2: reject a non-`Copy` value
  live across the spliced back-edge; accept a forwarded one.
- **R-P1-8 (invariant).** Write INV-INLINE-COMBINATOR (OQ3) as a doc-comment on the
  shared predicate and the combinator splice.

### Exit criteria and mutation tests

- **E-P1-1 — spliced self-tail lowers to a back-edge.** A hand-written combinator
  whose quotation argument self-tails lowers to a `jmp` back-edge with **no** self
  `Instr::Call`, asserted on lowered IR (not stdout).
  *Mutation:* revert R-P1-6 (`tail` → `false` at `calls.rs:520`); a self
  `Instr::Call` reappears and the back-edge is gone ⇒ test fails.
- **E-P1-2 — constant stack, correct result.** The same word at **1e6** iterations
  under `ulimit -s 512` exits 0 **and prints its computed value**, asserted against a
  golden the way `countdown` asserts `500000500000`. Asserting only `exits 0` is a
  placebo: a back-edge wired to the wrong block, or an off-by-one carried phi, still
  exits 0 and still passes E-P1-1's IR-shape assertion, so the computed value is the
  part that pins correctness.
  *Mutation:* same revert ⇒ real recursion ⇒ segfault (exit 139).
  **On the iteration count:** 1e6 is measured-sufficient (the tailsplice probe's
  pre-fix segfault was at 1e6, and a 512 KB stack overflows after a few thousand
  frames), while being ~100x cheaper than 1e8 in a debug build. Do not drop to a
  small N: real recursion would also exit 0, which is exactly the placebo this
  criterion exists to prevent. The same count applies to E-P2-4 and E-P3-5.
- **E-P1-3 — discard-after-call stays recursive (recon 4 negative golden).** A
  combinator whose body is `... t call e drop ...` lowers as a real self
  `Instr::Call`, **no** back-edge, asserted on IR.
  *Mutation:* make R-P1-2 blanket-accept a trailing combinator call as tail-splice;
  the word wrongly builds a loop and the "self `Instr::Call` present" assertion
  fails. (This is the recon-4 anti-placebo: a test that only checked "it builds"
  would pass either way.)
- **E-P1-4 — check and lowering cannot diverge.** A unit test that constructs the
  recon-2 shape and the recon-4 shape and asserts the checker's `splice_tail`
  (`combinators.rs:343`) and lowering's `body_tail_calls_self` decision
  (`calls.rs:520`) return the **same** answer for each, via the shared predicate.
  *Mutation:* give lowering a private rule (revert `combinators.rs:343` to the
  old `has_self_tail_call`-only, leaving lowering on the new predicate); the two
  disagree on the recon-2 shape ⇒ test fails. (Satisfies the brief's "a test that
  fails if they are allowed to diverge.")
- **E-P1-5 — linear value across the spliced back-edge.** A program carrying a
  **non-`Copy`** value live across a spliced back-edge is rejected, **asserting the
  exact diagnostic, not merely that it fails**: mirror the sibling assertion style at
  `check_linear_value_across_self_tail_call_is_error` (`src/check.rs:2491`), which
  asserts the message substring (``not supported yet``), the offending value's name
  (`` `Spy` ``), and that the error is **located** (`err.contains("line …")`). Without
  those, a parse failure or a generic linear error for the wrong reason would satisfy
  the criterion. A forwarded non-`Copy` value still builds.
  *Mutation:* drop the R-P1-7 extension; the linear-across program builds
  (unsound) ⇒ the negative test fails.

---

## Phase P2 — the quotation-effect row gate

**Value on its own:** makes shape-changing quotation-taking words writable at all
(`~[ ..i -- ..o ]` with `..i != ..o`), which no word can express today. P2 builds
on P1 (rung 6 of the rowgate ladder needs P1's back-edge) and still uses the
primitive `if`.

### Requirements

- **R-P2-1 (defer R4).** Defer `quotation_row_not_top_level_error`
  (`parser.rs:794`) until the whole signature is parsed, then admit a row inside a
  quotation effect iff it is one of the signature's **own top-level rows**
  (including an output row `..o` named later at top level). A genuinely fresh name
  stays a located error. This is narrower than the spike, which interned any fresh
  name.
- **R-P2-2 (lift R5 shape-change for inlined words).** Lift
  `quotation_row_shape_change_error` (`parser.rs:813`, fired `:1374`) so `~[ ..i --
  ..o ]` with `..i != ..o` parses for a quotation-taking (always-inlined) word. The
  shape change is splice-local and reconciled by the word's own control-flow join;
  it never becomes a distinct carried region and never rides a back-edge (recon 7).
  The 10a same-row rule still holds for a loop body's own carried region (the
  back-edge fixed point) — that is not this case, since a quotation-taking `if`/
  `myif` is not itself the loop; the caller (`sum-to`) is.
- **R-P2-3 (per-branch shape check — decision 7, not a bypass).** Compare the
  region a branch actually leaves against the region the declaration and its
  sibling expect. A branch whose actual output **contradicts** the declared `..o`
  is rejected **at the argument site**, not merely at the splice site. This is a
  comparison, not unification (recon 6); it restores the early diagnostic recon 8
  measured as lost. It must **not** be the spike's vacuous accept-anything.
- **R-P2-4 (no row unification).** `..o` is discovered by forward checking the
  spliced terms (recon 6), justified by INV-INLINE-COMBINATOR (R-P1-8).
- **R-P2-5 (mostly keep, and add the positive case).** Of the four existing parser
  tests, **three keep their current assertions unchanged**:
  `..._fresh_name_is_error` (`parser.rs:3517`), `..._no_top_level_row_is_error`
  (`:3526`), and `..._naming_an_output_only_row_from_the_input_side_is_error`
  (`:3560`) — all three remain located errors under R-P2-1.
  The fourth, `parse_row_in_quotation_effect_differing_output_row_is_error`
  (`:3544`), declares `( ..s i64 -- ..t ~[ ..s i64 -- ..t ] )`: the `~[ … ]` is on
  the **output** side, a word *returning* an inline quotation, which is NOT the
  input-side quotation-*parameter* case R-P2-2 lifts. **It therefore also stays an
  error**, and R-P2-2's lift must key on the quotation being a *parameter of an
  always-inlined word*, not merely on 'differing rows, both top-level'. Keying on
  the latter would wrongly accept an output-side inline quotation and turn this
  test into a lie.
  So nothing is 'retargeted'; the real work is the **new positive test**, whose
  shape is pinned here rather than left to the implementer: an **input-side**
  quotation parameter on a quotation-taking word, with differing rows that are both
  the signature's own top-level rows, e.g.
  `: myif ( ..i bool ~[ ..i -- ..o ] ~[ ..i -- ..o ] -- ..o )`. Do not delete a test
  to make anything pass.

### Exit criteria and mutation tests

- **E-P2-1 — the boundary parses and rejects correctly.** `~[ ..i -- ..o ]` with
  both rows the signature's own top-level rows parses; a fresh name and a row that
  is not a top-level row are still located errors.
  *Mutation A:* revert R-P2-1/R-P2-2 ⇒ the positive parse fails.
  *Mutation B:* widen R-P2-1 to the spike's "intern any fresh name" ⇒ the
  `fresh_name_is_error` negative test fails. (The two mutations pin the boundary
  from both sides — the anti-placebo for "too loose".)
- **E-P2-2 — a shape-changing quotation checks, runs, prints correctly.** With a
  non-empty carried region below it (rowgate rung 5): the carried value survives
  the call untouched and prints, and the shape-changing branches produce the right
  results.
  *Mutation:* revert R-P2-2 ⇒ the program no longer parses/checks.
- **E-P2-3 — contradicting branch rejected at the argument site (recon 8
  anti-placebo).** A branch whose actual output contradicts the declared `..o` is
  rejected with a diagnostic located at the **argument site**, asserting the exact
  message and location — not the splice-site "`+` needs 2 values, stack holds 1"
  message.
  *Mutation:* remove R-P2-3's per-branch shape check; the error either disappears
  or relocates to the splice site with a different message ⇒ the argument-site
  assertion fails. (A test that only asserted "it is rejected somewhere" is a
  placebo here — the splice-site forward check would satisfy it regardless.)
- **E-P2-4 — the back-edge interaction is sound (recon 7).** A fully
  row-polymorphic `myif` (hand-written over the primitive `if`), driving `sum-to`,
  runs at 1e6 under `ulimit -s 512` and emits correct carried-region phis
  (asserted on IR: a `jmp` back-edge, no self `Instr::Call`).
  *Mutation:* revert P1's `tail` threading ⇒ recursion ⇒ segfault. (P2 depends on
  P1; this criterion also guards that the row work did not regress P1.)

---

## Phase P3 — primitives, library `if`, keyword deletion, corpus migration

Atomic by necessity (decision 9): the word `if` and the keyword `if` cannot
coexist, and a transitional shim is banned. P3 **may** be sequenced internally
(P3.a add primitives + their consumers, then P3.b the atomic swap), but it is a
single phase and **must not** be split at the `if` swap. The internal sequencing is
descriptive, not two machine-readable phases.

### P3.a — the three primitives and their consumers

**Condition type: a 32-bit unsigned integer (`u32`), not `usize`.** The condition is
a *flag*, not a machine word, and every conversion vanishes at 32 bits: `bool` is
already a 32-bit 0/1 (`IrType::Bool`), `Instr::Cmp` already produces that width, and
the conditional jump consumes it directly. A target-width `usize` condition would
cost a widening after every comparison, a widening in `tag`, and a narrowing at every
branch, for nothing. It is also stable across 32- and 64-bit targets, which a flag
should be.

**Express every width in this phase as a bit width, never as a QBE register class.**
`src/ir/types.rs:69-77` is explicit that the IR stays backend-neutral and a future
WASM lowering reads `bits`/`signed`, never `w`/`l`; `src/backend/qbe.rs:279` is the
only place a register class is spelled. Requirement text says '32-bit unsigned'.

- **R-P3-1 (`branch`).** Add `branch` with checker signature
  `( ..a u32 ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )`. Its lowering is today's
  `lower_if` (`control_flow.rs:99`), rerouted to the `branch` builtin: conditional
  jump on the flag, run each branch quotation in its own block, join. `branch` knows
  32-bit integers only — no `Type::BOOL`. Nonzero is true (decision 3). Consumer:
  `if`/`unless` (R-P3-4).
- **R-P3-1a (`branch`'s checker arm).** `lower_if` is *lowering only*; the checker
  side of today's `if` is the `TermKind::If` arm at `src/check/terms.rs:702` (with the
  `Type::BOOL` guard at `:716`), which R-P3-5 deletes along with every arm that
  matched the variant, so `branch` needs its own checker arm and this requirement
  says what it is: (a) `branch` is the **single sanctioned builtin exempt** from the
  R11 quotation-operand default-deny (`reject_quotation_operand`, defined at
  `src/check.rs:1681`, applied at 15 call sites across the check stage) that rejects a
  quotation operand to every other builtin, so `branch` alone may take quotation
  operands; (b) `branch`'s arm reuses the `TermKind::If` arm's **branch-and-join**
  logic only (the two cloned scopes, the per-branch shape check, the `MaybeMoved`
  join, `releasable_into`), minus the `Type::BOOL` guard (`terms.rs:716`); and (c)
  **operand acquisition is extracted into a shared helper**, described next, because
  the `If` arm has none to reuse.

  **Why (c) is not optional.** The `TermKind::If` arm gets its two branches from the
  node's embedded `then_branch`/`else_branch: Vec<Term>` fields (destructured at
  `terms.rs:703-708`). `branch` has no such fields: its branches arrive as quotation
  **operands on the stack**, in two distinct forms that must both work. At a real
  splice they are `QuotRef::Known(id)` literals. But inside `if`'s own body
  (`| e | | t | | c | c tag t e branch`, R-P3-4), checked once at its definition site,
  `t` and `e` are **abstract quotation parameters** with no `QuotRef::Known`, which is
  the R21 forwarding case. The `If` arm handles neither form. The only code that
  resolves both is `inline_combinator` (`src/check/combinators.rs:236`). So P3 extracts
  that operand resolution (the `QuotRef::Known(id)` literal path **and** the R21
  abstract-forward path) into a helper called by **both** `inline_combinator` and
  `branch`'s arm. An earlier draft of this requirement forbade reusing
  `inline_combinator` and directed an implementer to the `If` arm alone; followed
  literally that destructures fields that do not exist and never checks `if`'s body at
  all. Do not reintroduce that prohibition: a second hand-rolled copy of the operand
  logic is what it was trying to prevent, and a shared helper prevents it properly.
  The condition is a 32-bit flag, not `Type::BOOL`.

  **Measured end to end** (worktree `sooth-branchcheck-probe`, branch
  `probe/branchcheck`): a shared `resolve_quotation_operand` function, called from
  both `inline_combinator`'s existing argument loop and a new `branch` checker arm,
  classifies a quotation operand into the `QuotRef::Known(id)` literal case or the
  R21 abstract-forward case. `branch` used directly with literals
  (`3 4 = [ 1 ] [ 2 ] branch`) builds and runs correctly with no symbol minted for
  `branch`. The load-bearing case, a library word's own body forwarding its `~`
  parameters into `branch` (`: myif ( bool [ -- i64 ] [ -- i64 ] -- i64 ) | e | | t |
  | c | c t e branch ;`), **type-checks at `myif`'s own definition site** and a
  real-literal caller through it builds and runs correctly. A mutation test (route
  `branch`'s arm through the `QuotRef::Known` case only, bypassing the shared
  resolver) breaks exactly the abstract-forward case with a located error and leaves
  the literal case passing, so the pass is attributable to the shared resolver and
  not some other path. Full suite stays green with `inline_combinator` refactored to
  route through it. Not exercised: the spec's actual row-polymorphic
  `~[ ..i -- ..o ]` signature, which hits the pre-existing, unrelated
  shape-changing-rows-in-quotation-effects parser limit (recon 3's territory,
  `parser.rs:813`); the probe used a monomorphic condition and branches instead,
  which exercises the identical R21 classification path.
- **R-P3-1b (keep the tail passes honest about `branch`).** The doc comment on
  `body_tail_calls_self` at `src/ir/func_builder/mod.rs:83-98` (`:99` is the fn) is a
  standing warning: check and lowering ``only agree today because a builtin-named
  combinator cannot exist: a combinator takes a quotation operand, and
  `check_operator`'s R11 guard rejects a quotation operand to any builtin name before
  the env combinator lookup runs. Nothing pins that, so if the R11 guard ever narrows,
  this needs the same refusal or check and lowering will disagree about whether a
  splice is a loop``. `branch` is exactly that narrowing (R-P3-1a lets one builtin
  take a quotation operand). P3 **updates that doc comment** to record `branch` as the
  sanctioned exception and **applies the refusal it demands**: give
  `body_tail_calls_self` the same builtin-name refusal `has_self_tail_call` has
  carried since slice 8a, so neither pass may treat a builtin name as a self-call
  while the other refuses. Combined with the R-P3-5a seed (how both passes descend
  into `branch`'s arms), `body_tail_calls_self` and `has_self_tail_call` cannot
  disagree about whether a splice through `branch` is a loop.
- **R-P3-2 (`tag`).** Add `tag` with signature `( E -- u32 )` for an `is_scalar`
  enum `E`; lowering is a genuine no-op (the scalar value is its discriminant and is
  already 32-bit, `layout.rs:641` / `control_flow.rs:225`), which is true at `u32`
  and would be false at `usize`. `tag` on a payload-carrying enum is a located
  compile error (OQ4). **The `is_scalar` predicate must be computed at check time
  from the enum declaration** (all variants payload-free, derivable from the AST),
  not by reaching into `ir::layout`, or the 'located' error lands at lowering
  instead. Consumer: `if`/`unless`.
- **R-P3-3 (machine-word comparisons + library comparisons).** Add six comparison
  primitives `u=`/`u<`/`u>`/`u<=`/`u>=`/`u<>`, each returning `u32` (`0`/`1`), backed
  by the existing `Instr::Cmp` (`calls.rs:432`) and polymorphic over the full 12-type
  numeric tower via one builtin-table row per type, exactly as the comparison
  builtins are today (`src/check/builtins.rs:146`, guarded by
  `builtin_table_comparisons_have_a_row_per_numeric_type` at `builtins.rs:460`). They
  mirror `CmpOp`'s operand-type-driven sign/float dispatch (OQ5): one primitive per
  shape, no `s<`/`f<` split. Move `=`/`<`/`>`/`<=`/`>=`/`<>` into `lib/` as ordinary
  words over them.
  **The move is a rename of the existing rows, not an addition beside them**, in both
  the checker table (`builtins.rs:146`) and the lowering match arm (`calls.rs:432`,
  which dispatches on the literal name string). The six old names must **leave**
  `BUILTIN_TABLE`; see R-P3-3c for why that is load-bearing rather than tidy.
  **Retarget, do not delete, the guard test.**
  `builtin_table_comparisons_have_a_row_per_numeric_type` (`builtins.rs:460`) asserts
  one row per numeric type for `=`/`<`/…; once those rows are renamed it must assert
  the same thing for `u=`/`u<`/…, which now carry the per-type rows. It is the only
  thing pinning tower coverage, so deleting it to go green would remove the guard at
  exactly the moment this slice puts it under strain. **All six library words are `'T: Copy Ord`-polymorphic** (so they
  keep covering the whole numeric tower, not just `i64`, which a monomorphic
  `( i64 i64 -- bool )` replacement would silently regress) **and declared `inline`**
  (possible only because of R-P3-3b), each yielding a `bool` by branch-and-construct
  per OQ5:
  `: = inline ( 'T: Copy Ord 'T -- bool ) u= [ true ] [ false ] branch ;`. Note
  `( 'T: Copy Ord 'T -- bool )` is **two** inputs of type `T`, not three: the bound
  declaration is itself the first input slot (`examples/poly_if.sth:6`,
  `: mymax ( 'T: Copy Ord 'T -- 'T )`, a two-argument max).
- **R-P3-3a (the library comparisons MUST be declared `inline`).** Unlike
  `if`/`unless`, a comparison word takes no quotation *parameter*, so it is not a
  combinator and would mint an `IrFunc` and be **called**. Measured: without
  `inline` a comparison call site emits a frame plus `call`; with `inline` (slice
  11) the emitted machine code is byte-identical to today's builtin, because QBE
  folds the branch-and-construct diamond to a branchless conditional move. Slice 11
  is therefore a **hard, load-bearing dependency of this slice**, not merely a
  shipped predecessor: without it this phase turns every comparison in the language
  into a function call.
- **R-P3-3b (lift the polymorphic-`inline` policy gate).** The six comparison words
  are both polymorphic (`'T: Copy Ord`) and `inline`, which `check_inline_declaration`
  (`src/check/word_entry.rs:66`) rejects today via the block at `word_entry.rs:88-96`
  (testing `ty_var_names` / `len_var_names` / `row_var_names`, error text
  `` `inline` requires a monomorphic effect ``). That block's own doc comment
  (`word_entry.rs:63`) states it is ``a policy rule, not a soundness one: the splice
  itself handles a variable-bearing body, so `inline` on a poly signature is not
  unsound, merely excluded``. P3.a **deletes the block** for **all three** variable
  kinds (type, length and row, matching the planned direction), and the full suite
  must stay green. Measured by the parent: with the block removed,
  `: cmpgt inline ( 'T: Copy Ord 'T -- bool ) > ;` builds, is spliced (no symbol in
  `nm`) and returns correct results for `i64`, `u32` and `i8` through one word; with
  it restored the identical program is rejected. So it is a policy gate, not a
  capability gate: lifting it needs **no lowering work** (the splice already handles a
  variable-bearing body) and is a deliberate reversal of a policy, not a soundness
  change. It is the enabler for R-P3-3a: without it a `'T: Copy Ord`-polymorphic
  comparison word cannot be `inline`.
  **Delete the whole `if let Some(sig) = &word.poly { … }` block, `word_entry.rs:87-97`,
  not the inner `if` alone.** Lines 88-96 are only the inner test; removing exactly
  those leaves an empty `if let` with an unused `sig` binding, which fails
  `clippy -D warnings`, this project's green bar.
- **R-P3-3c (the second inline gate is NOT lifted, and does not need to be).**
  `check_inline_declaration` has a *second* rejection after the polymorphic one:
  `` if BUILTIN_TABLE.contains_key(name) `` (`word_entry.rs:107`), error
  `` overloads a builtin operator name ``, guarded by
  `check_inline_builtin_operator_overload_is_error`. Unlike the polymorphic gate this
  one is a **soundness** gate: its comment explains that a builtin-operator name
  records `poly.builtin_overloads[span]` so lowering emits a real `Instr::Call`, and if
  such a name were then spliced, lowering would look the symbol up in an `env` a
  combinator is excluded from, giving ``a checker contradicting itself, and a panic
  downstream``. **Leave it in place.** The six comparison words escape it for free,
  because it keys on table *membership* and R-P3-3 removes them from the table. This
  makes the sequencing load-bearing: **rename the builtin rows before defining the
  library words**, or the definitions hit this gate. The gate stays in full force for
  every name that remains a builtin.

  **Measured end to end** (worktree `sooth-cmpprobe`, branch `probe/cmpinline`), with
  the six rows renamed to `u`-prefixed and the polymorphic gate lifted:

  ```sooth
  : =  inline ( 'T: Copy Ord 'T -- bool ) u= ;
  : <  inline ( 'T: Copy Ord 'T -- bool ) u< ;
  : <= inline ( 'T: Copy Ord 'T -- bool ) u<= if true else false end ;
  ```

  builds and prints `true true false true` across `i64` and `u32`, exit 0, with **no
  symbol minted** for any of the three (all spliced) and **no panic**. Two controls
  prove both changes are load-bearing rather than assumed: restoring the polymorphic
  gate alone rejects with `` `inline` on `=` … requires a monomorphic effect ``, and
  reverting the rename alone rejects with ``generic overload `: = ( 'T 'T -- bool )` …
  overlaps a concrete overload of `=` ``. The builtin-name gate was never observed
  firing for `=` after the rename, as expected.

### P3.b — the atomic swap

- **R-P3-4 (`if`, `unless`).** Define in `lib/`:
  - `: if ( ..a bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b ) | e | | t | | c | c tag t e branch ;`
  - `: unless ( ..a bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b ) | e | | t | | c | c tag e t branch ;`
    (branches swapped).
  `if`/`unless` are term-body combinators whose tail term is `branch`, whose
  tail-called-param set is both branch quotations — so a caller's `... [ base ] [
  rec ] if` self-tails through P1's rule.
- **R-P3-5 (delete the grammar and the node).** Delete `TermKind::If` and the
  `if`/`else`/`end` grammar (`parser.rs:2222`ff / `:2238`) and every arm that
  matched the variant. `grep -r "TermKind::If" src/` must be empty.
- **R-P3-5a (seed `branch` into the tail passes as the `If` arm is removed).**
  Two of the arms deleted by R-P3-5 are the `TermKind::If` descents in
  `drop_graph::collect_tail_calls` and `func_builder::body_tail_calls_self`, which
  P1 relies on to see into branch arms. `branch` is a **primitive with no walkable
  body**, so the OQ1 closure cannot compute its tail-called-parameter set by
  inspection: it must be **seeded** with 'both quotation operands are in tail
  position', taking over the role the `If` arm played. R-P3-4 asserts this set as a
  fact; this requirement is what makes it true. Omit the seed and `if`'s set
  computes empty, so `gcd`/`sum-to` silently lose their loops (E-P3-5 is the guard,
  but the requirement must be explicit rather than discovered by a segfault).
- **R-P3-6 (migrate the corpus).** Rewrite every `if`/`else`/`end` site in `lib/`
  and `examples/` (enumerated in OQ6) to postfix `[ T ] [ E ] if`, and rewrite
  `while`'s body accordingly. Program output must be byte-identical across the
  migration.
- **R-P3-7 (unit tests beside each new stage function, per CLAUDE.md).** CLAUDE.md
  makes "unit tests beside each stage function (happy path plus at least one
  error/edge case)" a phase-completion gate. P3 adds real stage code: `branch`
  lowering (`control_flow.rs`), `tag` lowering (`layout.rs` / `control_flow.rs`), the
  comparison primitive lowering (`calls.rs`) and the new `branch` checker arm
  (`check/terms.rs`, R-P3-1a). Each gets a `#[cfg(test)]` unit test beside it covering
  the happy path plus at least one error/edge case, **including a direct unit test for
  the `tag`-domain compile error** (`tag` on a payload-carrying enum, asserted at
  check time per R-P3-2). These are in addition to the golden exit criteria below.

### Exit criteria and mutation tests

- **E-P3-1 — `if` is a library word, not a primitive.** `if` resolves to a `WordDef`
  sourced from `lib/`: present in the word environment, absent from builtin
  dispatch. Assert the resolution, not the absence of a symbol.
  *Mutation:* leave the primitive in place ('nobody removed it') ⇒ the resolution
  assertion fails.
  **Do not** assert `nm`-silence or 'a call site emits a jump-and-join with no
  `Instr::Call`' as the load-bearing check: both pass **identically** whether `if`
  is a library word or still a primitive, since the primitive also emits a
  jump-and-join and mints no symbol. That formulation is a placebo of exactly the
  kind this project has shipped five times. The real discriminators are this
  criterion and E-P3-2.
- **E-P3-2 — the grammar and node are gone.** `grep -r "TermKind::If" src/` is
  empty; a source using `if`/`else`/`end` grammar now fails to parse with an
  unknown-construct diagnostic (asserted message).
  *Mutation:* leave one `TermKind::If` arm / the grammar in place ⇒ the grep test
  fails, or the old grammar still parses ⇒ the parse-error test fails.
- **E-P3-3 — `tag` domain.** `tag` on `bool` (scalar) is a genuine no-op, asserted
  concretely on the lowered IR for the `tag` operation: it contains **no
  width-conversion instruction** (operand and result are both 32-bit, so there is no
  widen or narrow) **and no memory access** (no load or store; the scalar value
  already is its discriminant). State this in backend-neutral terms, bit widths not
  QBE register classes, per `src/ir/types.rs`. `tag` on a payload-carrying enum is a
  located compile error, rejected **at check time** (asserted message and location,
  per R-P3-2).
  *Mutation:* accept `tag` on a payload-carrying enum ⇒ the negative test fails;
  make `tag` return a target-width integer ⇒ a width-conversion instruction appears
  in the lowered IR ⇒ the no-width-conversion assertion fails.
- **E-P3-4 — comparisons are library words, at no cost.** Four parts, because
  byte-identical *IR* is impossible by construction and asserting it would be
  unsatisfiable:
  1. `=`/`<`/`>` resolve to `lib/` definitions and no comparison builtin arm remains
     in `calls.rs`;
  2. the comparison **primitive** emits the same `Instr::Cmp` op, operand for
     operand, as today's builtin;
  3. the canonical `a b = if ... ...` pattern compiles to the **same emitted
     instruction sequence** as before the migration (measured achievable: QBE folds
     the diamond to a conditional move), and no comparison word mints a symbol;
  4. a golden exercises a comparison **library word** on a **non-`i64`** numeric type
     (`u32` and `i8`), proving the library replacement stayed `'T: Copy
     Ord`-polymorphic over the whole tower rather than silently narrowing to `i64`
     (the regression the sole `( i64 i64 -- bool )` worked example would have masked).
  *Mutation:* leave the builtin arm in place ⇒ part 1 fails; drop `inline` from a
  comparison word (R-P3-3a) ⇒ part 3 fails with a frame and a `call`; monomorphize a
  comparison word to `i64` ⇒ part 4 fails to type-check on `u32`/`i8`.
  **Note:** the emitted *IR* is deliberately NOT byte-identical. `branch` lowers to
  a jump-and-join with a phi and the backend has no folding pass, so a library `=`
  is `Cmp` plus a diamond in IR. Equivalence is recovered at the machine-code level,
  which is what part 3 asserts. An earlier draft asserted IR byte-identity; that was
  false in two independent ways (mechanism, and a 32-vs-64-bit width premise) and
  would have forced an implementer to add back the `usize → enum` reinterpret OQ5
  rejects.
- **E-P3-5 — constant stack preserved on IR.** `gcd`, `countdown`/`sum-to`, and
  `filter_while` still lower to a `jmp` back-edge with no self `Instr::Call`
  (asserted on IR, not inferred from output), through the new library `if`.
  *Mutation:* revert P1's `tail` threading or mis-shape `if`'s body so `branch` is
  not its tail term ⇒ recursion reappears ⇒ IR assertion / 1e6 stack test fails.
- **E-P3-6 — corpus output byte-identical.** Every migrated `examples/*.sth`
  produces byte-identical stdout before vs after the migration (a golden diff over
  the corpus).
  **Known hazard for this migration:** `>=` collides with the `>type` cast syntax.
  Measured: `5 >i8 >=` lexes as `>` followed by `=` and fails with ``unknown type `=` ``.
  The collision is pre-existing and not caused by this slice, but the slice turns `>=`
  into a library word and rewrites every comparison site, so the migration is where it
  will surface. If a corpus site trips it, report it rather than working around it
  silently: it is a lexer bug, not a migration detail.
  **Capture the 'before' stdout into committed golden fixtures as the FIRST step of
  P3, before deleting `TermKind::If`.** Once R-P3-5 lands, the pre-migration source
  no longer parses, so the comparison is impossible unless it was recorded first.
  The same applies to the whole-slice witness below.
  *Mutation:* any semantic drift in a migrated file (e.g. swapped branches) ⇒ the
  byte diff fails.
- **E-P3-7: a polymorphic `inline` word splices (retargets the poly-`inline`
  negative test).** The existing negative test
  `check_inline_polymorphic_signature_is_error` (`src/check/word_entry.rs`, whose
  rejection assertion is at `:528`) is **replaced with a positive test**. State in the
  test that the negative assertion is **retargeted because R-P3-3b deliberately
  reverses the rule**, not deleted to make a build go green; the same test's other
  half (a `~`-bearing but variable-free effect is still accepted) is unaffected and
  stays.
  **The witness must be a word whose name is one of the six comparison operators**
  (`: = inline ( 'T: Copy Ord 'T -- bool ) …`), spliced (mints no symbol) and correct
  across **at least two distinct numeric types**. A neutral name such as `cmpgt` is
  **not** an acceptable witness on its own: no builtin claims it, so it slips past the
  R-P3-3c builtin-name gate and would pass whether or not the real comparison words can
  ever be `inline`. That is precisely the placebo this criterion exists to exclude, and
  an earlier draft of this spec shipped it.
  *Mutations, both of which this witness catches and `cmpgt` does not:* restore the
  `word_entry.rs:87-97` block ⇒ rejected with `` requires a monomorphic effect ``;
  leave the six rows in `BUILTIN_TABLE` under their old names ⇒ rejected with ``generic
  overload … overlaps a concrete overload of `=` ``. Both were measured on
  `probe/cmpinline` (R-P3-3c).

---

## The whole-slice witness

A program using `if`, `unless` and `while` entirely from `lib/`, self-tailing
through a branch, produces the **same output** and the **same loop shape** (IR:
`jmp` back-edge, no self `Instr::Call`) as its pre-slice equivalent built with the
primitive `if`. Asserted on both stdout and lowered IR. This is the integration
golden that ties P1+P2+P3 together.

---

## Out of scope (from the brief, restated)

- Enum eliminators of any spelling, `match`, tagged branch literals (`Variant[
  ... ]`), and the `Ident[` lexing question — abandoned (decision 5), not deferred.
- Payload-carrying enum dispatch — stays with clause bodies.
- `cond` (OQ6): not shipped; a variadic `[ pred ] [ body ]` word is not fixed-arity.
- Dispatch-after-locals (`lib/binary_search.sth`'s `Ordering` sketch) — unrelated,
  still unbuildable after this slice.
- Early return — no `TermKind` for it; the two-way branch plus a join suffices.
- First-class runtime quotations / closures — slice 7b (where
  INV-INLINE-COMBINATOR must be revisited).
- Declared combinator recognition: slice 12, which marks whatever `if`/`unless`/
  `while` become. **This slice narrows slice 12.** ROADMAP item 12 currently claims
  it lifts the polymorphic-`inline` policy gate; 10c lifts that gate here (R-P3-3b),
  so slice 12 narrows to: retiring the `word_declares_quotation_parameter` leg of
  `is_combinator`, requiring the `~` literal at call sites, and migrating
  `lib/combinators.sth`. The lift belongs in 10c, not 12, because slice 12 **depends
  on** 10c (12's own text migrates "10c's injected if/cond"), so 12 cannot simply run
  first, and 10c ships the lift's first consumer (the six `'T: Copy Ord`-polymorphic
  `inline` comparison words) in the same phase, so it is not pre-staged plumbing.
  Item 12's *gate* description is stale in two further ways and should be corrected in
  the same edit: it says it migrates "10c's **injected** `if`/`cond`", but under this
  design `if` is an ordinary `lib/` word rather than injected, and `cond` is **not
  shipped at all** (OQ6). The migration target is 10c's library `if`/`unless`.
  **This needs a ROADMAP edit to item 12. ROADMAP.md is deliberately not touched by
  this spec (it carries unrelated uncommitted changes); the edit is reported to the
  parent instead.**

---

## Mutation-testing summary

Every new test names the mutation that must break it (above). The three
placebo-prone findings the brief flags are covered explicitly:

| Finding | Criterion | The mutation a placebo would survive |
| --- | --- | --- |
| recon 4 (discard-after-call stays recursive) | E-P1-3 | blanket-accept a trailing combinator as tail-splice; assert on IR (self `Instr::Call` present), not "it builds" |
| recon 7 (back-edge interaction sound) | E-P2-4 / E-P3-5 | assert `jmp` back-edge at 1e6 under `ulimit -s 512`; a small-N "exit 0" test would pass under real recursion |
| recon 8 (per-branch shape check, not a bypass) | E-P2-3 | remove the per-branch check; assert the **argument-site** diagnostic, not "rejected somewhere" (the splice-site catch would satisfy the loose form) |
| P3: `if` really is a library word | E-P3-1 | leave the primitive in place; assert `if` **resolves to a `lib/` `WordDef`**, not `nm`-silence or jump-and-join — a primitive `if` passes both of those identically |
| P3: comparisons cost nothing | E-P3-4 | drop `inline` from a comparison word; assert the canonical `a b = if` emits the same **instruction sequence**, not the same IR (IR byte-identity is impossible by construction) |
| P3: the poly-`inline` lift actually enables the real words | E-P3-7 | witness a **builtin-operator-named** word (`: = inline …`), not a neutral name like `cmpgt`: a neutral name slips past the R-P3-3c builtin-name gate and passes whether or not `=` can ever be `inline` |
| P3: `branch`'s checker arm actually handles the abstract-forward case | R-P3-1a | route the arm through `QuotRef::Known` only, bypassing the shared resolver: `myif`'s own definition (`c t e branch` over its own `~` parameters) fails to check, while a literal call site still passes |

Plus the divergence guard E-P1-4 (check vs lowering) and the boundary-from-both-
sides guard E-P2-1.

The E-P3-7 row was added after round 2, which caught the `cmpgt` witness as a placebo.
The measurement that had been used to justify the design used that same neutral name,
so it had never exercised the builtin-named case at all. Both gates were then measured
directly (R-P3-3c). The general lesson, now twice-learned in this slice: a witness must
carry every property of the real case, and a name is a property.

The two P3 rows above were added after round 1: both criteria originally asserted
something that a non-implementation would satisfy (E-P3-1) or that no correct
implementation could satisfy (E-P3-4). A criterion in either state is a placebo.

---

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Shared tail-splice recognition: one predicate (the per-combinator tail-called-parameter set) recognizes a self-call inside a combinator-tail-called quotation literal, consumed identically by the two syntactic passes, the four has_self_tail_call sites, and the lowering splice gate, with the linear-spine guards extended to the spliced back-edge",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Quotation-effect row gate: admit the signature's own top-level rows (including differing input/output rows) inside a quotation effect, with a real per-branch shape check that rejects a contradicting branch output at the argument site, keeping all four existing parser tests as located errors and ADDING a positive test for an input-side quotation parameter with differing top-level rows",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Add the branch, tag and comparison primitives over a 32-bit flag, giving branch a checker arm that shares its quotation-operand resolution with inline_combinator rather than duplicating it; RENAME the six comparison builtin rows to u-prefixed primitives (in both the checker table and the lowering match) and lift the polymorphic-inline policy gate BEFORE defining the library comparison words, since both gates are what otherwise reject a polymorphic inline word named after a builtin operator; define the six comparison words plus if and unless (and rewrite while) as library words, declared inline so no comparison becomes a real call; seed branch into the tail passes as TermKind::If and the if/else/end grammar are deleted in one atomic step; and migrate the lib and examples corpus to postfix if with byte-identical output against goldens captured before the swap",
      "effort": "L",
      "difficulty": "hard"
    }
  ]
}
```
