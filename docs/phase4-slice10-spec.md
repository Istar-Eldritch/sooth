# Phase 4 Slice 10 spec: rows in quotation effects, `times` becomes library code

This spec derives from [`docs/phase4-slice10-brief.md`](./phase4-slice10-brief.md). The
brief's recon (7 findings, verified against the built compiler on 2026-08-09, citations
re-verified 2026-08-10 against `main`) and its 7 settled decisions are treated as ground
truth and are **not** re-derived here. The spec turns those decisions into numbered,
traceable requirements, anchors a codebase map on the brief's citations (spot-checked
against the live tree while writing, see "Codebase map"), and lays out a phased delivery
plan split into **10a (mechanism)** and **10b (migration)**, matching the brief's own
split.

The one-line thesis, from the brief: `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` is a
compiler intrinsic only because user source cannot *spell its signature*, not because it
needs magic to run. Everything under it is already general (blanket self-tail-call TCO,
quotation params threaded as compile-time constants, shared `Builder` loop state). 10a
makes that signature writable; 10b deletes the intrinsic.

## Scope

**10a (mechanism, phases 1–4):** row variables inside a declared quotation effect
(parsing, representation, checking), plus the back-edge result-model fix (brief decision 3)
and its surviving-set obligation (brief decision 5). Exits on a user-space `my-times`
golden. The intrinsic is **not touched**.

**10b (migration, phase 5):** `times` moves to `lib/combinators.sth`; the intrinsic arms
in `check.rs` and `ir.rs` are deleted; diagnostics goldens are re-pointed deliberately;
corpus output stays byte-identical; the diagnostics diff and binary-size delta are
recorded. 8c-shaped lightweight process, separated from 10a because migration risk and
mechanism risk should not share a review (brief decision 7).

Out of scope (brief's list, restated so the implementer does not drift into it): a numeric
`Bound::Int`; fixing non-combinator poly self-recursion (`spin`); rewriting
`while`/`each`/`map`/`fold`/`filter` onto rows (10b moves `times` only); anything
runtime-quotation beyond 7b's existing rules; mutual recursion or non-tail self-calls
(already rejected, stay rejected); widening the loop counter off `i64` (that is 8a overload
territory, brief decision 6).

## Sequencing

10a edits `check_term`'s dispatch spine and the literal checks, as did 7b and 8a; those
have landed on `main` (`5f645f0`, 7b's residual-bug note, merged 2026-08-10), so the
three-way merge the brief warned about is no longer a risk. 10a starts now. 10b follows 10a
alone.

## Codebase map (anchored on the brief's citations; spot-checked against the live tree 2026-08-10)

Spot-checks performed while writing this spec (all confirmed present at the cited lines):
`SelfTailMarker` at `src/check.rs:648`; the back-edge `outs` block at
`src/check.rs:8472-8479`; `check_abstract_quotation_times` at `src/check.rs:6840`;
`PolyType::Quotation(Vec<PolyType>, Vec<PolyType>)` at `src/ast.rs:622`; `PolySig`
row fields (`row_in`/`row_out`/`row_var_names`) at `src/ast.rs:629-640`; the `surviving`
field doc (7b/R19) at `src/check.rs:213-218`; `parse_poly_slot` (no `..` branch) at
`src/parser.rs:1208`. The tree moves; re-anchor before editing.

| Concern | Location | Notes |
| --- | --- | --- |
| Top-level row parse | `parse_poly_slots` `src/parser.rs:1178`; `PolyBuilder.set_row` `:676`; `row_var_misplaced_error` `:747` | Row recognized only at slot-list head; a later `..` in the list is already `row_var_misplaced_error`. |
| Nested (quotation) slot parse | `parse_poly_slot` `src/parser.rs:1208`; `parse_poly_quotation` `:1251`; `parse_poly_quot_list` (called from `:1253/1255`) | No `..` branch today: falls through to `parse_type_expr`, which rejects `..s` as an unknown type. This is the boundary 6a drew (its R2/R28). |
| Raw → poly fold | `RawTy` `src/parser.rs:634`; `RawTy::Quotation` `:1257`; `raw_to_poly_type` `:1346` (Quotation arm `:1362`) | Fully-concrete effect folds to `Concrete(Type::Quotation)`; only a variable-bearing effect stays `PolyType::Quotation`. |
| Poly representation | `PolyType::Quotation(Vec<PolyType>, Vec<PolyType>)` `src/ast.rs:622`; `PolySig.row_in/row_out/row_var_names` `src/ast.rs:629-640` | `PolyType::Quotation` carries **no** row fields today. `PolySig` is the shape to mirror. |
| Pointwise row walk (6a R6) | `unify_poly_input` `src/check.rs:5845`; Quotation arm `:5912` (arity check `:5922`) | Matches ins/outs pairwise, requires equal arity, binds vars. Row must be excluded from this arity. |
| Ground declared output | `apply_subst` `src/check.rs`, Quotation arm (just after `:5989`) | Substitutes both rows of a declared quotation effect. |
| Intrinsic row reasoning | `check_abstract_quotation_times` `src/check.rs:6840` | The prototype for "pop fixed inputs above an opaque row, require the row restored." |
| Splice-site declared-effect checks | `check_literal_against_declared_effect` `src/check.rs:7301` (Known literal); `check_poly_combinator_args` `:7226` (abstract pass-down) | Ground a declared effect with **no** row concept today. These are the two paths recon 5 / open-question 1 names. |
| Combinator dispatch / splice | `check_term` `src/check.rs:8000`; `is_combinator` `:6921`; `has_self_tail_call` `:3943`; `check_combinator_cycles` `:6966`; `inline_combinator` `:7076` | Combinators mint no `IrFunc`; term-spliced per call site, so the row is concrete at every check. |
| Self-tail marker & back-edge | `SelfTailMarker` `src/check.rs:648`; set at `:7178`; matched at `:8443-8446`; the `outs` block at `:8472-8479` | Decision 3 rewrites `:8472-8479`. |
| Back-edge guards (R8/R9) | `check_linear_across_back_edge` `src/check.rs:6762`; `check_reference_across_back_edge` `:6741` (invoked at `:8469/8471`) | Run unchanged (decision 4). |
| Surviving-set field | `Slot.surviving` `src/check.rs:213-218`; `Slot::computed` (used at `:8479`) | 7b/R19. The getter/array/cell forwarding pattern to follow is `d1b3f0a`/`bee407c`. |
| Intrinsic `times` interception | `check_term` "times" arm `src/check.rs:8208`; abstract path invoked `:8221` | Deleted in 10b, not 10a. |
| Intrinsic lowering | `src/ir.rs:3441` (`"times"` arm); registration `:5804` | Deleted in 10b. |
| Library combinators | `lib/combinators.sth` (`each`/`map`/`fold`/`filter`/`while`) | `times` is added here in 10b. |
| Constant-stack witness helper | `run_at_stack_limit` `tests/phase4_combinators.rs:1403`; e.g. `three_deep_times_nesting_runs_in_constant_stack` `:1127` | Reuse for R12. |

## Requirements

Each requirement cites the brief decision / recon / open question it traces to. "Located"
means the diagnostic carries a span and names the offending row/argument and the declared
signature, per the project's diagnostics-are-behaviour convention (CLAUDE.md).

### 10a — mechanism

**R1 — row inside a quotation effect must be the signature's own top-level row.** *(brief
decision 1; open question 1)* A `..`-prefixed name appearing inside a declared quotation
effect must denote the signature's top-level row (`..s` bound at the deepest input slot).
A fresh name (`..t` where the signature's row is `..s`), or any row when the signature
declared none at top level, is a **located error** naming the row and the declared
signature. There is nothing else the name could denote: the literal reaches into exactly
the caller region below the combinator's fixed inputs. Same discipline as bounds (declared
at the binding occurrence). Parser sites: `parse_poly_slot`/`parse_poly_quot_list`
(`src/parser.rs:1208`/`:1251`), threading `PolyBuilder`'s recorded top-level row
(`row_in`/`row_out`, `:665`).

**R2 — both sides or neither.** *(brief decision 2)* A row in a quotation effect appears in
both the effect's inputs and its outputs, or in neither (`[ ..s i64 -- ..s ]`). A one-sided
row (`[ ..s i64 -- ]` or `[ i64 -- ..s ]`) is a **located error**: a quotation that
consumes an unknown region and does not restore it cannot be meaningfully spliced. Mirrors
the intrinsic's shape.

**R3 — representation mirrors `PolySig`.** *(open question 4)* `PolyType::Quotation`
(`src/ast.rs:622`) grows optional row fields — `row_in`/`row_out` as row ids in the
signature's existing row id space (the same id when the same `..s` passes both sides),
mirroring `PolySig.row_in/row_out` (`src/ast.rs:629-640`). A fully-concrete effect still
folds to `Concrete(Type::Quotation)` in `raw_to_poly_type` (`src/parser.rs:1362`); only a
row- or variable-bearing effect stays `PolyType::Quotation`. Prefer growing the existing
variant over a new wrapper unless the pointwise walk (R4) forces otherwise; justify the
choice in the phase 1 plan. Note the runtime-side `QuotEffect` (`src/ast.rs:889`) needs
**no** row field: at every splice the row is concrete (recon 5), so a spliced literal's
`Type::Quotation` carries a ground effect, not a row.

**R4 — the pointwise unify walk excludes the row.** *(open question 4; 6a R6)*
`unify_poly_input`'s Quotation arm (`src/check.rs:5912`, arity check `:5922`) must match
only the *fixed* (non-row) slots pairwise. The row contributes no pairwise slot, binds no
type/len variable, and is excluded from the equal-arity check. Confirm this is the only
place 6a's pointwise row walk needs to skip the row; if `apply_subst`'s Quotation arm (just
after `src/check.rs:5989`) also needs to carry the row through when grounding a declared
output, that is in scope of this requirement.

**R5 — row grounding at the declared-effect check sites (general form of the intrinsic's
depth arithmetic).** *(brief recon 5; open question 1)* The depth arithmetic
`check_abstract_quotation_times` (`src/check.rs:6840`) implements for the one blessed
signature — pop the declared fixed inputs above the opaque row, require the row region
restored unchanged — is generalized to the two general declared-effect check paths:
`check_literal_against_declared_effect` (`src/check.rs:7301`, the Known-literal splice) and
the abstract pass-down `check_poly_combinator_args` (`:7226`) / its abstract-quotation
sub-path. A row in a declared quotation parameter is grounded to the concrete caller-stack
region below the fixed inputs at each site. This stays per-splice depth arithmetic: **no**
abstract row unification, **no** `Subst` extension (it stays `ty`+`len`), **no** mangling
impact (combinators produce no symbols). The spec's phase 2 plan must state, in prose,
*which* check runs for the Known-literal operand and which for the abstract-parameter
pass-down (the intrinsic's two paths are the prototype), resolving open question 1 before
code.

**R6 — the back-edge arm produces the ground declared outputs.** *(brief decision 3; recon
4)* Rewrite the back-edge `outs` construction (`src/check.rs:8472-8479`) so the arm's
result stack is the combinator's **ground declared outputs** (via `apply_subst`), not its
non-quotation inputs. The current fiction ("non-quotation inputs, which for a self-tail
combinator are exactly its declared outputs") is true only for the state-threading shape
(`while`: `'a` in, `'a` out) and false for a loop that *consumes* its counters — which is
why the recon-4 `my-times` fails today with a spurious "if branches leave different stack
depths (then: 3, else: 1)".

**R7 — the self-call's arguments are checked against the ground declared inputs.** *(brief
decision 3)* Replacing the fiction (R6) removes the transitive type-check the `if`-join
previously got from it, so the back-edge gains an **explicit** unification of `stack[base..]`
against the combinator's ground declared inputs, emitting a **located** "back-edge argument
mismatch" diagnostic naming the argument and the declared signature on failure. Sound
because the marker matches only in tail position (recon 3), so the join this feeds is the
body-final join. `while` must check identically before and after (its non-quotation inputs
equal its outputs — the very equality that hid the bug); a regression test must pin that
`while` is unchanged.

**R8 — R6's rewrite must not re-source 7b's surviving-set drop.** *(brief decision 5; recon
7; open question 3)* This is a correctness obligation **on R6's own rewrite**, not a
separable nice-to-have. The old block builds `Slot::computed(s.ty)`, dropping `s.surviving`
and `s.quot` (7b/R19, `src/check.rs:213-218`); its filter excludes a bare erased-quotation
slot but **not** an aggregate carrying one — the exact gap 7b's review fixed in the
getter/array/cell paths (`d1b3f0a`/`bee407c`) and explicitly left here as documented
residual risk. R6 sources `outs` from manufactured declared-output types, which can re-drop
the identical set. Therefore:

- For any declared-output position that **positionally corresponds** to a `stack[base..]`
  slot (the state-threading shape R6/R7 center on — `while`, and any row-adjacent fixed
  position of `times`), the rewritten `outs` must **forward that slot's
  `surviving`/`quot`** fields, following the `d1b3f0a`/`bee407c` pattern — not manufacture a
  fresh `Slot::computed(declared_ty)`.
- For any declared-output position with **no** positional source slot on `stack[base..]`
  (if any signature shape produces one), the spec must state **explicitly** what populates
  its `surviving` field and why that is sound, rather than defaulting silently to `None`.
  The phase-3 plan must first determine whether `my-times`'s own declared-output shape has
  any such position (open question 3); recon 4's probes do not exercise one.
- If the gap is not closed for a given position, 10a must **re-verify against the rewritten
  code** (not by inheriting 7b's assumption) that 7b's masking condition still holds — no
  self-tail combinator lacking a conditional exit exists in the corpus — and record that on
  the exit checklist. This does not reopen 7b's decision to leave its own instance
  unfixed; it forbids R6 from *silently* inheriting that exemption for a freshly-sourced
  instance.

**R9 — R8/R9 back-edge guards run unchanged.** *(brief decision 4)*
`check_linear_across_back_edge` (`src/check.rs:6762`) and
`check_reference_across_back_edge` (`:6741`) run unchanged at the back-edge; rows add no
exemption. A linear or reference-into-frame-local value carried across the edge is still an
error.

**R10 — three new located diagnostics, numbered and golden-tested.** *(open question 2;
decisions 1/2/3)* Fresh row name in a quotation effect (R1), one-sided row (R2), and
back-edge argument mismatch (R7). Each located, each naming the row/argument and the
declared signature. Wording and numbering settled in the owning phase; each is a
golden (source in → expected diagnostic out), per CLAUDE.md.

**R11 — user-space `my-times` compiles and sums.** *(brief exit)* With the row restored to
its signature, `: my-times ( ..s i64 i64 [ ..s i64 -- ..s ] -- ..s ) ... ;` compiles from
user source **beside** the intrinsic, and `0 0 5 [ bump ] my-times .` prints `10`.

**R12 — constant stack at 1M iterations.** *(brief exit; recon 6)* `my-times` runs 1M
iterations to completion at `ulimit -s 1024`, constant stack, exit 0, via
`run_at_stack_limit` (`tests/phase4_combinators.rs:1403`).

**R13 — aggregate aliasing witness.** *(brief exit; recon 6)* A struct/aggregate carried
across the row with per-iteration data dependence prints arithmetically correct fields, so
the slice-3 aliasing class (a stale value from iteration *k* visible at *k+1*) surfaces as a
wrong number, not a crash.

**R14 — nesting parity.** *(brief exit; 6d)* `my-times` nested inside itself produces
correct output.

**R15 — no regression; intrinsic untouched.** *(brief exit; decision 7)* `while` and the
full existing corpus are unchanged, and 10a does not touch the intrinsic (its arms in
`check.rs`/`ir.rs` still exist and still serve `times`).

**R16 — mutation-test the new guards.** *(project convention: mutation-test the guards)*
For each R10 rejection golden, prove the test can fail by deleting the guard it protects
(the R1/R2/R7 check) and confirming the golden flips; Sooth has shipped placebo tests
before, and reading does not catch them.

### 10b — migration

**R17 — `times` becomes library source; intrinsic arms deleted.** *(brief decision 7)*
`times` is written in `lib/combinators.sth`; `check_abstract_quotation_times`
(`src/check.rs:6840`), the `check_term` `"times"` interception arm (`src/check.rs:8208`),
and the `src/ir.rs` `"times"` arm (`:3441`, plus its registration at `:5804`) are deleted.

**R18 — diagnostics goldens re-pointed deliberately.** *(brief decision 7)* Each intrinsic
bespoke `times` message that becomes the general combinator message is re-pointed under
review — none silently. The full diagnostics diff is recorded in this spec's 10b section at
implementation time.

**R19 — byte-identical corpus, measured size delta.** *(brief decision 7)* Corpus
*outputs* stay byte-identical; the splice depth of `each`/`map`/`fold`/`filter` call sites
grows by one; the binary-size delta across `examples/` is **measured and recorded** here,
not waved through.

## Requirement → decision traceability

| Req | Traces to | Owning phase |
| --- | --- | --- |
| R1 | decision 1, open q1 | 1 |
| R2 | decision 2 | 1 |
| R3 | open q4 | 1 |
| R4 | open q4, 6a R6 | 1 |
| R5 | recon 5, open q1 | 2 |
| R6 | decision 3, recon 4 | 3 |
| R7 | decision 3 | 3 |
| R8 | decision 5, recon 7, open q3 | 3 |
| R9 | decision 4 | 3 |
| R10 | open q2, decisions 1/2/3 | 1 (R1/R2), 3 (R7) |
| R11–R15 | brief exit criteria | 4 |
| R16 | project convention | 4 |
| R17–R19 | decision 7 | 5 |

## Phased delivery plan

### 10a — mechanism (phases 1–4)

**Phase 1 — row syntax and representation.** *(R1, R2, R3, R4; R10 for R1/R2)* Grow
`PolyType::Quotation` with row fields mirroring `PolySig` (R3). Add the `..` branch to the
nested-quotation parse path, tied to the signature's recorded top-level row, rejecting a
fresh name (R1) and a one-sided row (R2) with located diagnostics (R10). Teach the pointwise
unify walk (and `apply_subst` if needed) to exclude the row from arity (R4). No checking of
row *depth* against a live stack yet — that is phase 2. Unit tests beside the parser/AST
changes: the two rejections plus a happy-path parse of `[ ..s i64 -- ..s ]` that reaches the
checker. Standard difficulty: mechanical AST/parser surgery with diagnostic care, no
type-system reasoning.

**Phase 2 — row grounding at the check sites.** *(R5)* Resolve open question 1 in prose
first (which check runs for the Known-literal operand, which for the abstract pass-down),
then generalize the intrinsic's depth arithmetic into
`check_literal_against_declared_effect` and the abstract path, so a row-bearing declared
quotation parameter grounds to the concrete caller-stack region below its fixed inputs. No
`Subst`/mangling changes. Unit tests: a row-bearing literal checked against a matching
concrete stack (accepted, row restored) and a mismatching one (rejected), plus the
abstract pass-down shape. **Hard**: this is the type-checking core and the open-question the
brief flags as needing a map before requirements.

**Phase 3 — back-edge rewrite and surviving-set.** *(R6, R7, R8, R9; R10 for R7)* Rewrite
the back-edge `outs` block to the ground declared outputs (R6) and add the explicit
unify of `stack[base..]` against the ground declared inputs with a located diagnostic (R7,
R10). Determine whether any `my-times` declared-output position lacks a positional source
slot (open q3), then forward `surviving`/`quot` where a source exists or document the
no-source case, or, if left open, re-verify 7b's masking condition against the rewritten
code and record it (R8). Confirm R8/R9 guards unchanged (R9) and pin `while` byte-identical.
Unit tests beside the change: `while` unchanged; a `times`-shape self-tail combinator that
type-checks where it did not before; the back-edge argument-mismatch rejection; the R8
surviving-set assertion (an aggregate-carrying-closure slot forwarded across the edge, not
dropped). **Hard**: the riskiest half, rewriting the exact block 7b's review flagged, with
a correctness obligation (R8) that can be silently violated.

**Phase 4 — exit witnesses and mutation guards.** *(R11–R16)* The 10a golden suite proving
the exit criteria: the user-space `my-times` sum (R11), 1M constant-stack (R12), aggregate
aliasing witness (R13), self-nesting (R14), corpus/`while`/intrinsic unchanged (R15), and
mutation-testing every R10 rejection (R16). Land the on-record resolution of R8 (forwarded,
or documented-residual with the masking re-verified) on the exit checklist. Standard: no new
mechanism, but substantial integration witnesses; kept a separate phase so the correctness
witnesses are not folded into the phase that could make them pass trivially.

### 10b — migration (phase 5)

**Phase 5 — migrate `times` to the library.** *(R17, R18, R19)* Write `times` in
`lib/combinators.sth`; delete `check_abstract_quotation_times`, the `check_term` `"times"`
arm, and the `ir.rs` `"times"` arm and its registration (R17). Let the suite find the
fallout (8c-shaped). Re-point each intrinsic diagnostics golden under review and record the
full diff in this spec's 10b section (R18). Verify byte-identical corpus output and measure
and record the `examples/` binary-size delta (R19). Standard: deletion plus deliberate
golden re-pointing; no new type-system reasoning.

## Exit criteria

**10a** (from the brief, mapped to requirements): the recon-4 `my-times` with its row
restored compiles from user source beside the intrinsic and — sums correctly through a
literal (R11); runs 1M iterations at `ulimit -s 1024` in constant stack (R12); carries a
struct across the row with per-iteration dependence, printing correct fields (R13); nests
inside itself correctly (R14); leaves `while` and the full corpus unchanged (R15);
decisions 1–3's rejections are located-error goldens (R10), mutation-tested (R16); and
decision 5's surviving-set gap is resolved on the record — either R6's `outs` forwards
`surviving`/`quot` where a positional source exists, or the spec documents, against the
rewritten code (not by assumption), that 7b's masking condition still holds and the risk
stays documented residual, never silently reintroduced (R8).

**10b:** `times` is Sooth source; `check_abstract_quotation_times` and the `"times"` arms in
`check.rs`/`ir.rs` do not exist (R17); the corpus prints byte-identical output, and the
diagnostics diff and binary-size delta are recorded in this spec's 10b section (R18, R19).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "row-syntax-and-representation", "difficulty": "standard" },
    { "phase": 2, "focus": "row-grounding-at-check-sites", "difficulty": "hard" },
    { "phase": 3, "focus": "back-edge-rewrite-and-surviving-set", "difficulty": "hard" },
    { "phase": 4, "focus": "exit-witnesses-and-mutation-guards", "difficulty": "standard" },
    { "phase": 5, "focus": "migrate-times-to-library", "difficulty": "standard" }
  ]
}
```
