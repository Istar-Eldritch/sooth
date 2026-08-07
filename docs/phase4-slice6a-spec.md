# Phase 4 Slice 6a: quotation types in signatures, the inliner, and `each`/`map`/`fold`

**Status: implemented.** Base `main` @ `d033b2d`. Makes a combinator an ordinary Sooth
library word: a quotation becomes nameable in a signature, a word may take one as a
parameter, and every call to such a word is inlined at the call site by term splicing.
Native only; the REPL is a located rejection at both chokepoints (6c lifts it). This slice
owns the compiler's **only** inliner (QBE is per-function, `cc` runs no `-O`), so the
inliner is the enabling mechanism, not an optimization.

## Fixed shape (verified against the built tree)

1. **The gap is front-end.** What the inliner produces is fully expressible today:
   hand-inlined `each`/`fold`/`map` over `[i64 4]` compiles and runs; element/length
   polymorphism (`len` at `[i64 4]` and `[f64 7]`) already works. No IR, lowering, or
   loop-primitive gap.
2. **A `Type`/`PolyType` change, not `IrType`.** `Type`/`PolyType` gain a quotation variant
   carrying a *declared effect*, with unification and `apply_subst` following. `IrType`, the
   calling convention, and the backend stay untouched: a quotation-taking word is never
   lowered standalone, so its type never reaches the backend.
3. **Inliner attaches at check/lower time by term splicing (forced).** A quotation-taking
   word's body contains `f call`/`f times` with no runtime representation, so no `IrFunc`
   for it can exist; there is nothing at IR level to inline. The attachment point
   generalizes the existing `call`/`times` fusion that clones a literal's AST body and
   re-runs `check_terms` against the live stack.

## Locked decisions

- **D1** Spelling stays `[ ... -- ... ]`, disambiguated on a top-depth `--` (an array type
  never contains `--`; a quotation effect always does, including nil `[ -- ]`). No new
  sigil; the malformed-array diagnostic stays sharp.
- **D2** Inliner splices at check/lower time: retain the callee's AST body, substitute the
  caller's literal for the quotation parameter, splice at the call site, let the existing
  `call`/`times` fusion fire.
- **D3** A quotation literal passed to a user word may capture only `Copy` locals, by value;
  no borrows of enclosing places. This makes a declared effect a *complete* summary of a
  literal.
- **D4** Standalone checking of a combinator body is compositional: `f call`/`f times` check
  against `f`'s declared effect exactly as an ordinary word call checks against a `Sig`.
- **D5** Inlining is total, transitive; anything un-inlinable is a located error. Recursion
  among quotation-taking words is the first case.
- **D6** The type never bakes in "a quotation is always statically known"; knownness stays a
  predicate on the value (`Slot.quot`), so slice 7 can admit a runtime closure without
  unpicking unification.
- **D7** Native only. The REPL is a located rejection at every chokepoint (the 5a lesson: an
  unpinned REPL gap was a silent miscompile, not a clean deferral). 6c lifts it.
- **D8** Library lives at `lib/combinators.sth`, imported by relative path.

## The `times` obligations under D3

`times` proves three obligations by walking a body it can see. With a quotation *parameter*
there is still a body to walk: `each`'s `times` takes a *literal* `[ ... f call ]` written
at `each`'s def site; the only opaque term is `f call`, which checks against `f`'s declared
effect `[ 'T -- ]` like an ordinary word call.

1. **Move-state identity** — discharged at the def site. `f`'s declared effect names no
   local, so walking `f call` records no move. Sound for every inline site because D3 limits
   the substituted literal to `Copy`-only captures (no linear local consumed; disposing a
   `Copy` N times is a no-op).
2. **Borrow-state identity** discharged in two places. Def-site check covers captured
   state (D3 forbids capturing an enclosing borrow). A literal that *creates* a borrow of a
   captured `Copy` local and leaves the reference on its output row is caught at the
   **splice site** by R12's borrow-left-on-row check (`quotation_borrows_place_error`),
   which `check_poly_combinator_args` runs on the poly argument path exactly as the
   monomorphic path does, before the body is ever spliced into the loop. The diagnostic
   lands on the caller's own literal, naming the word and the borrowed place, never on a call
   `each`'s author never wrote. Criterion 12c witnesses exactly this.
3. **Row-effect equality** — discharged at the def site by composing visible ops with `f`'s
   declared effect.

Obligations 1 and 3 are discharged entirely at the def site; obligation 2 is discharged at
the def site for captured state and at the splice-site R12 check for literal-created borrows.
No obligation is *deferred* to the inline site in the sense the brief warned about.

## Requirements by stage

Located diagnostics assert message text **and** named identifiers/positions.

### Parsing (`src/parser.rs`, `src/ast.rs`)

- **R1.** In type position only, a `[` is disambiguated by scanning to its matching `]` for a
  top-depth `--`: present routes to `parse_quotation_type_expr`, absent keeps
  `parse_array_type_expr`. No new token or sigil.
- **R2.** `parse_quotation_type_expr` parses `[ <in> -- <out> ]` into the new quotation type,
  each side a possibly-empty list via existing `parse_type_expr`. Nil `[ -- ]` legal. A row
  variable `..s` inside is out of scope (R28).
- **R3** *(located)*. `[i64]` still reaches `parse_array_count` and reports `array count must
  be a decimal literal`. Unterminated `[` and malformed type lists are located parse errors.
  Golden asserts `[i64]` is *still* the array diagnostic.

### Type representation (`src/ast.rs`, `src/ir.rs`)

- **R4.** `Type` gains a `Quotation` variant carrying a `QuotEffectId` into an interned
  `(inputs, outputs)` registry, mirroring `Type::Array`, so `Type` stays `Copy` and
  self-renders. No "statically known" bit (D6).
- **R5.** `PolyType` gains the parallel case so a declared effect may mention the sig's
  type/length variables.
- **R6.** Unification and `apply_subst`/`Subst` extend: unify input/output rows pointwise
  (equal arity, else located mismatch); `apply_subst` maps through both rows. `[ 'T -- ]`
  vs `[ i64 -- ]` binds `'T = i64`; arity mismatch is located.
- **R7.** `instantiation_symbol`/mangling and `IrType` gain no reachable quotation case; the
  arms are `unreachable!` pinning the reachable case to slice 7, guarded by R20u. Sound only
  because of R7a.
- **R7a** *(located)*. Type-position audit: exactly one position accepts a quotation type (a
  direct input in a word's declared effect); every other is a located rejection naming the
  position and slice 7 — struct field, enum payload, array element (including the
  `[ [ i64 -- ] 3 ]` parse path), cell payload, reference referent, word output, `extern:`
  either direction, `main`, and nesting inside another effect. One table-driven test in
  slice 4's audit shape.

### Checking, monomorphic (`src/check.rs`)

- **R8.** `call` accepts an abstract quotation beside the literal: `Known(id)` splices;
  `Type::Quotation(eff)` checks against the declared effect (pop `eff.inputs`, push
  `eff.outputs`), no splice; else the reworded `call_needs_quotation_error`.
- **R9.** `times` accepts an abstract quotation: the declared effect must be row-preserving
  with a trailing `i64` index; the three obligations reduce to declared-effect checks.
- **R10.** A quotation argument to a user word is accepted when the target parameter is a
  `Type::Quotation` (routed to the inliner); against a non-quotation parameter it is
  rejected with `reject_quotation_argument` reworded.
- **R11** *(located)*. Directional literal check: instantiate the parameter's declared input
  row, run the literal body via `check_terms` (bracketed, `tail = false`), require exit to
  match the declared output row. Mismatch names word, parameter position, and both rows. No
  standalone effect inferred.
- **R12** *(located)*. D3 capture restriction enforced at the literal: consuming a non-`Copy`
  enclosing local, or a `&`/`&!` borrow of an enclosing place, is a located rejection naming
  the local and word. A `Copy` local read by value is allowed.
- **R13** *(located)*. A quotation parameter is `Copy`, so `| f | ... f call ... f call` is
  accepted and splices once per use; a body failing to consume it is no leak; a unit pins
  that it registers no move obligation.

### Checking, polymorphic (`src/check.rs`)

- **R14.** `poly_term`'s `TermKind::Quotation` arm (slice 4's outright rejection) is lifted
  for the immediately-consumed case: the poly stack gains a `quot` marker keyed by stack
  position (since `poly_term` walks `Vec<PolyType>` with no `Slot`). A quotation reaching a
  runtime-value position in a poly body is still rejected, reworded.
- **R15.** `poly_call_term` intercepts `call`/`times` in a poly body (the poly twins of
  R8/R9) and accepts a quotation argument routing to the inliner (R10's poly twin).
- **R16** *(located)*. The standalone `times` obligations run against `f`'s declared effect
  at the def site. Negatives: a poly combinator whose times-body consumes an outer linear
  local (`times_body_consumes_local_error`), and one whose borrow crosses the back-edge
  (`times_body_borrow_across_loop_error`).
- **R17.** `each`/`map`/`fold` type-check standalone at `lib/combinators.sth`
  (`: each ( ['T 'N] [ 'T -- ] -- )`, `: map ( ['T 'N] [ 'T -- 'T ] -- ['T 'N] )`,
  `: fold ( ['T 'N] 'A [ 'A 'T -- 'A ] -- 'A )`); `map`/`fold` check `each` against its
  `PolySig`, never its body.

### The inliner (`src/check.rs`, `src/ir.rs`)

- **R18.** D2 term-splice at the call site: bind the caller's literal(s) to the callee's
  quotation parameter(s) as `Known` markers, clone the callee's AST body, splice against the
  live stack (bracketed, `tail = false`), let `call`/`times` fusion fire.
- **R19.** Same at lowering: `lower_call` takes a quotation-taking-word branch that inlines
  the callee body with params mapped to phantom quotation `Value`s, emitting no
  `Instr::Call`, transitively.
- **R20.** A quotation-taking word mints no symbol and no `IrFunc`; excluded from standalone
  monomorphization. `instantiation_symbol` never computed for it. Unit R20u asserts no
  `IrFunc`/`Instr::Call` for it, mono and poly.
- **R21.** Transitive inlining, outermost-first: a combinator that forwards its own
  quotation *parameter* to a nested combinator splices through both frames (witnessed by
  `abstract_quotation_forward_inlines_and_runs` and, at IR level,
  `abstract_forward_inlines_transitively_with_no_call`). Termination rests on R22 (the
  subgraph is a DAG). *(The original example, `map` splicing `each`, is a cost choice
  rather than a necessity: `map`/`fold` are leaf combinators, not built on `each`, though
  they could be; see §Accepted narrowings and gaps.)*
- **R22** *(located)*. D5 recursion rejection: a pre-lowering pass builds the call graph over
  quotation-taking words and rejects any cycle, reusing the 3-colour DFS of
  `check_tail_call_cycles`. "Un-inlinable" = participates in a cycle in this subgraph.

### REPL (`src/repl.rs`), per D7 and against what 5b shipped

The 5a/slice-2 hazard: an unpinned REPL gap was a *silent miscompile*, not a clean deferral,
so every REPL chokepoint here is a located rejection, specified and tested.

- **R23** *(located)*. A session line whose word signature names a `Type::Quotation`
  parameter is rejected at the definition chokepoint (a session discards word bodies; the
  inliner needs them — the 6c retention problem).
- **R24** *(located)*. An imported closure *exporting* a quotation-taking word is rejected at
  import time, naming file and word. **Pinned against what 5b actually shipped**: a
  quotation-taking word used purely *internally* to an imported closure inlines fine during
  that closure's own native compilation; only *exporting* it to the session (where a later
  line would call it, needing the discarded body) is the chokepoint. Verified by
  `repl_import_exporting_quotation_word_is_rejected` (both halves).

### Stale diagnostics and dogfood/docs

- **R26.** The nine stale "Phase 6" diagnostics corrected. Eight are pure wording (a
  quotation reaching a runtime-*value* position → slice 7, not "Phase 6"). The ninth,
  `reject_quotation_argument`, changes behaviour (R10) with wording retargeted to "this word
  does not take a quotation". Slice-4 goldens asserting old wording updated in lockstep.
- **R27.** Dogfood: `examples/array_totals.sth` rewritten over `each`/`map`/`fold`, matching
  hand-threaded `examples/array_totals_hand.sth`. ROADMAP slice-6a marked implemented;
  DESIGN.md control-flow section records combinators are library words inlined by term
  splicing (D2), the type nameable at `Type`/`PolyType` only (D6), total inlining (D5), REPL
  a located rejection (D7).

### Out of scope

- **R28.** Slice 7's runtime-quotation representation stays out (no `IrType` variant, no
  calling convention, no `(code, env)` value, no quotation in array element/struct
  field/branch join, no non-inlined higher-order call, no row variable inside an effect).
  6b (polymorphic-`if`, polymorphic-self-tail, `filter`/`while`) stays out; 6c out except
  R23/R24. `while` intrinsic declined. Slice-8 native holes stay recorded.

## Invariants preserved

Backend stays QBE; `Ptr[T]` opaque; no LLVM, native backend, JIT, comptime. `IrType` gains
no quotation variant (D6). Linear spine untouched: a quotation parameter is `Copy` (D3).
`core` stays `no_std`. Constant stack preserved: an inlined combinator lowers to the same
`times` back-edge machinery. No new `Instr`/`Terminator`; no `qbe.rs` change. A program
using no quotation parameter lowers byte-for-byte as today.

## Delivery

Landed across four implementation phases plus a review-fix sequence; each commit green.
The original phase commit hashes below predate the review-fix sequence and are indicative,
not current. Where the as-shipped behaviour differs from the first landing, this section
states the as-shipped reality:

1. **(hard)** Quotation type + monomorphic inliner. Parse disambiguation (R1–R3);
   `Type`/`PolyType` variant with unification/`apply_subst` and guarded mangling/`IrType`
   arms (R4–R7); type-position audit (R7a); abstract `call`/`times` (R8/R9); argument sites
   with directional and D3 checks (R10–R13); interprocedural term-splice inliner (R18–R20);
   cycle rejection (R22); `reject_quotation_argument` change (part of R26).
2. **(hard)** Polymorphic path + library. Poly `quot` marker (R14); poly `call`/`times` and
   arguments (R15); standalone `times` obligations (R16); `each`/`map`/`fold` at
   `lib/combinators.sth` (R17); transitive inlining of an abstract quotation forward (R21).
   **The constant-stack witness is not a 1M+ run** (infeasible — see criterion 14 and
   §Accepted narrowings and gaps): it is `each_lowers_to_a_loop_not_a_per_element_call`
   (structural: loop header + back-edge, no per-element user `Call`) plus
   `combinator_and_hand_threaded_loops_agree_across_stack_limits`, a
   combinator-vs-hand-threaded equivalence-plus-correctness witness at N=10k (equal exit
   code *and* stdout over a 1-filled array whose correct fold is N).
3. **(standard)** REPL located rejections and eight wording corrections (R26). R23 shipped
   in this phase; **R24 shipped as a review fix** (the located rejection of an imported
   closure exporting a quotation-taking word, with the mangled-name leak fixed alongside).
4. **(standard)** Dogfood + docs (R27). `examples/array_totals*.sth`, DESIGN.md, ROADMAP.md,
   tests.

## Exit criteria (goldens in `tests/phase4_combinators.rs`)

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | `[ i64 -- i64 ]` / `[ -- ]` parse to a quotation type | `quotation_type_in_signature_parses` | golden | 1 |
| 1b | `[i64]` is *still* the array-count diagnostic | `array_type_without_arrow_stays_array_diagnostic` | golden | 1 |
| 1c | malformed quotation effect is a located parse error | `malformed_quotation_type_is_located_parse_error` | golden | 1 |
| 2 | `[ 'T -- ]` unifies against `[ i64 -- ]`; arity mismatch located | `quotation_effect_unifies_and_binds_variable` | unit | 1 |
| 2b | quotation type in every audited non-parameter position is located, names slice 7 | `quotation_type_is_rejected_at_every_audited_position` | golden (table) | 1 |
| 2c | `[ [ i64 -- ] 3 ]` located as array-element rejection, no panic | `array_of_quotation_type_is_located_rejection` | golden | 1 |
| 2d | no audited position reaches mangling / `IrType` | `quotation_type_never_reaches_mangling_or_irtype` | unit | 1 |
| 3 | `3 [ 1 + ] apply .` prints `4` | `monomorphic_quotation_taking_word_inlines_and_runs` | golden | 1 |
| 3b | lowered `apply` caller has no `Instr::Call`, no `IrFunc apply` | `quotation_taking_word_emits_no_call_and_no_irfunc` | unit | 1 |
| 4 | literal/parameter effect mismatch names word, parameter, both effects | `literal_effect_mismatch_against_parameter_is_error` | golden | 1 |
| 5 | linear capture / enclosing borrow is a located D3 rejection | `quotation_literal_capturing_linear_local_is_error` | golden | 1 |
| 5b | `Copy` local read by value accepted and runs | `quotation_literal_capturing_copy_local_runs` | golden | 1 |
| 6 | `\| f \| ... f call ... f call` splices twice | `quotation_parameter_used_twice_splices_twice` | golden | 1 |
| 6b | quotation parameter registers no move obligation | `quotation_parameter_is_copy_no_move_obligation` | unit | 1 |
| 7 | quotation against non-quotation parameter rejected, reworded | `quotation_against_non_quotation_parameter_is_error` | golden | 1 |
| 8 | self-recursive quotation-taking word is a located cycle | `recursive_quotation_taking_word_is_located_error` | golden | 1 |
| 8b | two-word cycle names both members | `quotation_taking_word_cycle_names_members` | golden | 1 |
| U20 | no quotation-taking word mints a symbol | `quotation_taking_word_mints_no_symbol` | unit | 1 |
| 9 | `each` checks standalone | `each_checks_standalone` | golden | 2 |
| 9b | `map`/`fold` check standalone, each body checking `f call` against `f`'s *declared effect* (they are leaf combinators, not built on `each` — see Accepted narrowings) | `map_and_fold_check_compositionally` | golden | 2 |
| 10 | `arr [ . ] each` over `[i64 4]` prints each element | `each_over_array_inlines_and_runs` | golden | 2 |
| 10b *(respecified)* | a combinator forwarding its own quotation *parameter* to a nested combinator splices through both frames, no surviving `Instr::Call` | `abstract_forward_inlines_transitively_with_no_call` (`src/ir.rs`), `abstract_quotation_forward_inlines_and_runs` | unit + golden | 2 |
| 11 | `fold` sums `[i64 4]` to `28` | `fold_computes_sum` | golden | 2 |
| 12 | poly times-body consuming an outer linear local located (def site) | `poly_combinator_consuming_local_is_error` | golden | 2 |
| 12b | poly times-body borrow across back-edge located (def site) | `poly_combinator_borrow_across_loop_is_error` | golden | 2 |
| 12c | a caller literal creating a borrow of a captured `Copy` local and leaving it live on its output row is a located R12 borrow-left-on-row rejection at the splice site | `literal_created_borrow_across_loop_is_error_at_splice_site` | golden | 2 |
| 13 | quotation at runtime-value position in poly body rejected, reworded | `quotation_at_runtime_position_in_poly_body_is_error` | golden | 2 |
| 14 *(respecified)* | the combinator loop and its hand-threaded `times` twin agree in exit code *and* stdout across a sweep of stack limits, over a 1-filled array whose correct fold is N (N=10k) | `combinator_and_hand_threaded_loops_agree_across_stack_limits` | golden | 2 |
| 14b | inlined `each` lowers to loop header/back-edge, no per-element `Call` | `each_lowers_to_a_loop_not_a_per_element_call` | unit | 2 |
| 15 | session line defining a quotation-taking word is a located REPL rejection | `repl_quotation_taking_definition_is_rejected` | golden | 3 |
| 16 | importing a closure exporting a quotation-taking word located; internal imports fine | `repl_import_exporting_quotation_word_is_rejected` | golden | 3 |
| 17 | the eight reworded diagnostics name slice 7, not "Phase 6" | `stale_phase6_diagnostics_are_reworded` | golden | 3 |
| 18 | dogfood over `each`/`map`/`fold` matches hand-threaded result | `combinators_dogfood_matches_hand_threaded` | golden | 4 |

Load-bearing units (mutation-test the guards): 2, 2b, 2c, 2d, 3b, 6b, U20, 10b, 14b.
2b/2c/2d make R7's `unreachable!` arms sound. 3b/10b/14b guard against a silent fallback.
5/5b are the D3 pair. **14b, not 14, carries the constant-stack guarantee**: it is the
structural claim (loop header + back-edge, no per-element user `Call`). 14 is an
equivalence-plus-correctness witness at N=10k (equal exit code and stdout against the
hand-threaded twin, over a 1-filled array whose correct fold is N); it does not by itself
witness stack cost, since equal exit codes are compatible with both programs sitting on the
same side of every sampled limit, and nothing in the suite measures stack frame size. 12/12b
pin obligation 1 and obligation 2's captured-state half at the combinator's own definition
site; 12c pins obligation 2's literal-created-borrow half, a located R12 rejection at the
splice site (below).

## Accepted narrowings and gaps

Two genuine narrowings where the as-shipped slice is narrower than this spec first claimed,
and two claims this spec first made that a reviewer falsified by writing the programs the
spec said could not exist; both false claims were then fixed. Each is recorded rather than
silently absorbed.

**1. `map`/`fold` are leaf combinators, and that is a cost choice, not an impossibility.**
R21's original example was `map` splicing `each`. This spec first claimed it could not be
written in this slice, on the grounds that `fold`'s accumulator would need a row variable
(R28) and `map`'s write-back a captured mutable borrow (D3). That claim is false, and a
reviewer falsified it by writing the programs. `fold` over `each` compiles and runs: the
accumulator rides a captured one-element array reached by **balanced** `&`/`&!` borrows
inside the literal, which D3 as shipped accepts (this is narrowing #3, four paragraphs
down, so the original document contradicted itself). `map` is likewise expressible with a
second captured counter cell for the index. The `each`-based versions are expressible; no
row variable is required. The real reason to keep `each`/`map`/`fold` as leaves is cost.
Inlining in this slice is *total*, not heuristic: a quotation-taking word mints no `IrFunc`
and no symbol, because a quotation has no runtime representation, so there is nothing to
call and splicing is the only lowering that exists. The decision is one predicate
(`is_combinator`: does the word declare a quotation parameter). Termination is structural
rather than budgeted: R22 rejects cycles among quotation-taking words, so the subgraph is a
DAG and transitive splicing bottoms out on its own; there is no depth limit or size budget
anywhere in the splice path. The consequence is that **library composition depth is code
size at every call site**. Leaf combinators keep the library flat at depth 1; building
`map` on `each` would make every `map` call site depth 2, and cost an extra array copy and a
counter cell. R21 is nonetheless real and delivered by a different route: a combinator may
forward its own quotation *parameter* to a nested combinator, the transitive case criterion
10b names. "When to inline" becomes a real question only at slice 7, when a runtime
representation first makes a genuine choice possible; until then "always" is the only
implementable answer, and a budget would be actively harmful, since exceeding it could only
be a compile error.

**2. Criterion 12c is reachable, and now implemented and rejected.** This spec first claimed
12c was unreachable because a combinator whose quotation parameter has a *reference* output
row is rejected at its own definition site, so no literal could reach the position 12c
describes. Both halves were false. Such a combinator (`refout`, a `[ 'T -- &i64 ]`
parameter) compiles clean standalone, and before the fix its caller's borrow-creating
literal ran with no diagnostic, printing `7 7 7 7`. The cause: a polymorphic combinator
keeps its signature in `word.poly`, not `word.effect` (which is empty), so
`inline_combinator`'s monomorphic argument loop ran **zero** checks on the polymorphic path,
skipping both the directional R11 effect check and the D3 capture check (R12). Commit
`c0b0bb2` routes polymorphic calls through `check_poly_combinator_args`, and 12c is now a
located R12 borrow-left-on-row rejection **at the splice site**, covered by
`literal_created_borrow_across_loop_is_error_at_splice_site`. The same fix closes an
undeclared mono/poly divergence a reviewer found: `: applyr ( 'T [ 'T -- &i64 ] -- )`
accepted a caller literal its monomorphic twin `( i64 [ i64 -- &i64 ] -- )` correctly
rejected. This vindicates obligation 2's original two-places shape: definition site for
captured state, splice site for literal-created borrows.

**3. R12's borrow half is narrower than its prose.** It reads as "a `&`/`&!` borrow of an
enclosing place is a located rejection"; the implementation rejects a borrow **left live on
the literal's exit row**, and accepts a balanced borrow taken and released inside the
literal. A reviewer failed to turn the narrowing into unsoundness: the ordinary borrow
checker catches the dangerous shapes at the splice site (aliasing the array `each` itself
borrows, and a `&!` inside the literal while the caller holds a `&` of the same place). This
balanced-borrow acceptance is exactly what makes narrowing #1's `each`-based `fold` and
`map` expressible. The narrowing is the right call and the code, not this sentence, is the
contract.

**4. Criterion 14 no longer claims a 1M-element run.** A Sooth array is a stack value, so
1M `i64` is 8 MB of stack before any loop frame exists, and the "constant stack under a
reduced `ulimit`" framing cannot pass however good the inliner is. `fill`'s compile cost is
also superlinear (10k ~ 0.36s, 100k ~ 25s, 1M > 300s) and **pre-existing**: a hand-threaded
`times` twin is equally slow, so it is not an inliner regression. Recorded on ROADMAP as a
future-slice item.

One pre-existing defect noted here because this slice's diagnostics sit on top of it, and
deliberately **not** fixed: every native `build` diagnostic prints a doubled prefix
(`error: error: stack effect mismatch in `main``), because the error constructors embed
`error: ` and `src/main.rs:34` prepends another. It is repo-wide (165 such constructors on
`main`) and predates this slice; fixing it here would sprawl the diff and churn assertions
across the suite.

## Review fixes

After the initial landing an independent reviewer falsified two impossibility claims by
writing the programs (narrowings #1 and #2). These commits followed, each green:

- `c0b0bb2`: run R11 and R12 on the *polymorphic* combinator argument path
  (`check_poly_combinator_args`), which the monomorphic argument loop skipped entirely
  because a poly combinator's signature lives in `word.poly`, not `word.effect`. Makes 12c
  reachable and rejected, and closes the `applyr` mono/poly divergence (narrowing #2).
- `2ece6bb`: criterion 14 now compares stdout as well as exit codes, over a 1-filled array
  whose correct fold is N (the old 0-filled fixture made the expected value `0`, the least
  discriminating possible, so a fold computing the wrong answer still passed).
- `a8ab1f9`: the R7a audit now descends into a *polymorphic* array element. `[ [ 'T -- ] 3 ]`
  was accepted while the monomorphic `[ [ i64 -- ] 3 ]` twin was rejected, an unaudited
  position that defeated the default-deny contract R7's `unreachable!` arms rest on. Driven
  from one recursive enumeration over both the monomorphic and polymorphic type paths.
- `f84866b`: a `type:` line naming a quotation in an interned array/cell/ref position used to
  leave a poisoned registry entry that re-fired the R7a audit on every later line,
  permanently bricking the REPL session. The `type:` path now restores the interned
  registries on failure, mirroring the non-`type:` guard.
- `4eb7f57`, `c568687`, `38f44e0`: diagnostics no longer leak the internal `__m0`
  monomorphization mangling when a module has an import (a repo-wide defect predating this
  slice, arriving with module support), and the now-unreachable `demangle_local` was deleted
  as dead code.

## Sanctioned edits

Slice-4 negatives asserting old "Phase 6" wording updated with R26: the R9
`quotation_passed_to_user_word_…` golden (callee parameter now decides accept vs reject),
the R11 audited-site wording, the R19 residual-line wording. No behaviour a non-quotation
program relies on changes; each edit called out in its phase's commit.
