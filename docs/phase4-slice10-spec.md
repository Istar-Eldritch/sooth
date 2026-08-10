# Phase 4 Slice 10a spec: the inline-only quotation type, and rows in quotation effects

Derives from [`docs/phase4-slice10-brief.md`](./phase4-slice10-brief.md). The brief's
recon is treated as ground truth **except where this spec says otherwise** — a first
review round found two of its claims false against the live tree, and both are corrected
here rather than inherited (see "Corrections to the brief").

`times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` is a compiler intrinsic only because user
source cannot *spell its signature*, not because it needs magic to run. Everything under
it is already general: blanket self-tail-call TCO, quotation parameters threaded as
compile-time constants, shared `Builder` loop state (`save_loop_state` `src/ir.rs:3016`,
`alloca_home` `:2718`). This slice makes the signature writable.

**Scope is 10a only.** 10b (deleting the intrinsic, moving `times` into
`lib/combinators.sth`, and deciding how much of the rest of the library retypes) gets its
own brief and spec; it is deliberately not in this document's phase plan, so migration risk
and mechanism risk do not share a review. 10c (`if`/`cond` as ordinary words,
`docs/phase4-slice10c-brief.md`) consumes phases 1–3 of this spec and nothing else.

## The target signature

```sooth
: times ( ..s usize ~[ ..s usize -- ..s ] -- ..s )
```

Three things differ from the brief's version, each settled deliberately:

**`usize`, not `i64` (supersedes the brief's decision 6 text).** The brief said "the index
type stays `i64`", but its stated *reason* was that widening to several index types is 8a
overload territory — an argument about admitting many types, not about which single type is
right. The corpus argues for `usize`: `len` already returns `usize`
(`src/check.rs:5261`), so all four library combinators convert it *down* with `len >i64`
purely to satisfy `times` and then convert each index back *up* per iteration, twice in one
body in two cases (`examples/array_totals_hand.sth:20`,
`examples/inplace_fold.sth:33`). A count also cannot be negative, so `usize` makes the
nonsense case unrepresentable rather than needing a diagnostic. It is cheap on both axes
that could have made it expensive: integer literals already coerce into a `usize` parameter
(verified: `: takes-usize ( usize -- ) drop ;` fed a bare `5` compiles, exit 0), so literal
counts do not grow a `>usize`; and `IrType::Usize` and 64-bit `IrType::Int` are the same
QBE register (`"l"`, `src/backend/qbe.rs:298`).

**`~` on the quotation parameter.** See the next section.

**One row, the same on both sides.** `times`'s body row is a fixed point, not a
transformation, and this is forced rather than chosen: the body's output *is* the next
iteration's input across the back-edge. Trace it — N=0 leaves the region untouched, so the
declared output row must equal the input row; N≥2 feeds iteration 1's output into iteration
2, whose declared input is the input row, so again they must be equal. Only N≤1 would admit
a differing output row, and N is a runtime `usize`. Encoding the fixed point in the
signature makes violating it a signature mismatch at the declaration instead of a
back-edge row mismatch later. (This is exactly why 10c's `if` legitimately takes
`~[ ..i -- ..o ]`: it runs its branch once, so it is permanently in the N≤1 case. `while`
and `fold` are the symmetric case here — `fold`'s accumulator rides *inside* `..s` at a
fixed shape and type, which is why accumulation needs no asymmetry.)

## `~`: the inline-only quotation type

`~[ ... ]` is a quotation type that **cannot be materialized**: no runtime representation,
never stored in a struct/array/cell field, never returned, never widened or coerced to an
ordinary `[ ... ]`, never reaching an erasure boundary. `call` remains its invocation
syntax — unchanged from how a combinator's body invokes its own quotation parameter today —
because a `call` on a `~` value is statically always a splice, never a runtime dispatch.
The ban is on materialization, not on invocation.

**Why it belongs in this slice rather than 10c.** A row-bearing quotation parameter *must*
be spliced: `QuotEffect` (`src/ast.rs:889`) has no row field and a row's size is not known
at runtime, so there is nowhere for the row to live in a materialized value. Every
combinator today relies on that as an unstated guarantee. `~` states it, which turns "a
row-bearing quotation must never reach `materialize_quotation_at_boundary` or either
`if`-join erasure site" from a rule that would have to be bolted onto three call sites into
a structural impossibility: there is no coercion path from a `~` type to a runtime
`Type::Quotation`. And if the guarantee holds for every combinator parameter, then
`~[ ..s usize -- ..s ]` is the honest declaration for `times`'s own parameter, so shipping
`times` with a type we would then have to change is the expensive order.

**10a applies `~` to `times` alone.** Whether `each`/`map`/`fold`/`filter`/`while` should
retype — making `~` the explicit combinator/closure boundary, with ordinary `[ ... ]`
reserved for genuinely first-class capturing quotations (7b's territory) — is 10b's
question, deliberately deferred. The *design* answer is expected to be yes; the migration
is not this slice's risk to carry.

## Corrections to the brief

**1. `check_abstract_quotation_times` is not the prototype the brief claims.** The brief
(recon 5, echoed in the ROADMAP) says it implements "pop the declared fixed inputs above an
opaque row, require the row restored." It does not. Its actual body
(`src/check.rs:6855-6872`) requires the *declared effect* be self-similar and then
pointwise-matches the declared outputs against the top slots:

```rust
let row_preserving = eff.inputs.last() == Some(&Type::I64)
    && eff.inputs.len() == eff.outputs.len() + 1
    && eff.inputs[..eff.outputs.len()] == eff.outputs[..];
let base = stack.len() - row_len;          // row_len = eff.outputs.len()
for (i, want) in eff.outputs.iter().enumerate() { match_slot(stack[base + i], *want) ... }
```

There is no fixed-inputs-above-a-row decomposition and the row is never inspected —
everything below the matched slots is untouched by construction. So phase 3 **derives** the
grounding rather than generalising a prototype, and R5 below says how. (Its hardcoded
`Type::I64` is also a site the `usize` change touches.)

**2. "Nothing in the checker reads `PolySig.row_in`/`row_out`" is stale.** `poly_sig_shape_eq`
reads both (`src/check.rs:3188`/`:3191`) and `poly_sig_str`'s `render_row` prints them
(`:3231-3232`). This matters for R4: because `poly_sig_shape_eq` drives overload dedup,
growing `PolyType::Quotation` with row fields makes two candidates differing only by row
distinguishable for free.

**3. A top-level row is weaker than the brief implies.** It is not "a row that may change
shape" — the checker models it as size-zero during the word's own body check and rejects any
body that touches it: `: shrinks-row ( ..a i64 -- ..b ) drop drop ;` fails with
`` `drop` needs 1 values, but the stack holds 0 ``. Writing differing `row_in`/`row_out`
names parses, but nothing verifies they differ semantically. Only "opaque and provably
untouched" is supported today. 10a does not change that (it is exactly what `times` needs);
10c is where a genuinely transforming row becomes new integration work.

## Codebase map

Every citation below was verified against the tree while writing this revision. The tree
moves — re-anchor before editing.

| Concern | Location | Notes |
| --- | --- | --- |
| Quotation type parse | `parse_poly_quotation` `src/parser.rs:1251`; quot-list called from `:1253`/`:1255`; `RawTy::Quotation` declared `:641`, constructed `:1257` | Where `~` is recognised (phase 1). |
| Nested slot parse | `parse_poly_slot` `src/parser.rs:1208` | No `..` branch today: falls to `parse_type_expr`, which rejects. The 6a boundary (R2/R28). |
| Top-level row parse | `parse_poly_slots` `src/parser.rs:1178`; `PolyBuilder` `:662` (`row_in` `:669`, `row_out` `:670`); `set_row` `:676`; `row_var_misplaced_error` `:747`, fired `:1196` | A later `..` in a slot list is already a located error. |
| Raw → poly fold | `raw_to_poly_type` `src/parser.rs:1346`, Quotation arm `:1362`; `RawTy` `:634` | A fully-concrete effect folds to `Concrete(Type::Quotation)`; only a variable- or row-bearing effect stays `PolyType::Quotation`. |
| Poly representation | `PolyType::Quotation` `src/ast.rs:622`; `PolySig` `:629` (`row_in` `:631`, `row_out` `:636`, `row_var_names` `:640`); `QuotEffect` `:889` | `PolyType::Quotation` carries no row fields today; `PolySig` is the shape to mirror. `QuotEffect` needs none (recon 5). |
| Pointwise unify | `unify_poly_input` `src/check.rs:5845`, Quotation arm `:5912`, arity check `:5922` | Row must be excluded from the pairwise arity. |
| Grounding a declared effect | `apply_subst` `src/check.rs:5963`, Quotation arm `:5989` | Substitutes a declared quotation effect. Has no `stack` parameter today — phase 3's plumbing point. |
| Splice-site checks | `check_poly_combinator_args` `src/check.rs:7226` (`base` computed `:7240`, Pass 2 `:7262-7280`, `apply_subst` call `:7267`, literal check `:7272`); `check_literal_against_declared_effect` `:7301`; non-poly caller `:7112` | The caller already holds `stack` and `base`; the callee does not. |
| Combinator dispatch | `check_term` `src/check.rs:8000`; `is_combinator` `:6921`; `has_self_tail_call` `:3943`; `check_combinator_cycles` `:6966`; `inline_combinator` `:7076` | Combinators mint no `IrFunc`; spliced per call site, so a row is concrete at every splice. |
| Self-tail back-edge | `SelfTailMarker` `src/check.rs:648`; set `:7178`; matched `:8452-8456`; arm `:8441-8484`; **`outs` construction `:8476-8480`** | Phase 4 rewrites `:8476-8480`. |
| Back-edge guards | `check_linear_across_back_edge` `src/check.rs:6762` (invoked `:8469`); `check_reference_across_back_edge` `:6741` (invoked `:8471`) | Run unchanged (R9). |
| Surviving set | `Slot` `src/check.rs:194`, `surviving` `:213-218`; `Slot::computed` `:224-234` (sets `quot: None, surviving: None`); `union_surviving` `:844`; `intern_surviving_set` `:826` | Phase 5's subject. Forwarding pattern to follow: `d1b3f0a`, `bee407c`. |
| Signature renderers | `poly_type_str` `src/check.rs:6284`, Quotation arm `:6295-6309`; `poly_quotation_concrete_hint` `:5946`, used at `:5924`; `poly_sig_str`'s `render_row` `:3231-3232`; `poly_sig_shape_eq` `:3188`/`:3191` | R10: neither quotation renderer knows about rows, so a row would silently vanish from every new diagnostic. |
| Intrinsic (untouched in 10a) | `check_term` `"times"` arm `src/check.rs:8208`, abstract path `:8221`; `check_abstract_quotation_times` `:6840`; `ir.rs` `"times"` lowering `:3441` | 10b deletes these. `ir.rs:5804` is **not** a registration — it is `b.lower_call("times", …)` inside the test `times_saves_and_restores_loop_state` (`:5761`); `times` is not in `BUILTIN_WORDS`, it is intercepted by literal name. |
| Constant-stack witness | `run_at_stack_limit` `tests/phase4_combinators.rs:1403`; e.g. `three_deep_times_nesting_runs_in_constant_stack` `:1127` | Reuse for R14. |

Landed prerequisites, for the record: 7b merged `3776579`, 8a merged `e20c52f`, slice 9
merged `c5db035`. (The pre-revision spec cited `5f645f0` — a 7b *doc* commit — as evidence
8a had landed; that was wrong.)

## Requirements

"Located" means the diagnostic carries a span and names the offending row/argument and the
declared signature, per the project's diagnostics-are-behaviour convention.

### The `~` type

**R1 — `~[ ... ]` parses as a distinct quotation type.** The sigil is recognised wherever a
quotation type may appear (`parse_poly_quotation` `src/parser.rs:1251`) and carried through
`RawTy::Quotation` and `raw_to_poly_type` into the poly/concrete type layers. A `~` type is
distinguishable from an ordinary `[ ... ]` at every point the checker inspects a quotation
type; how (a flag on the existing variant versus a separate variant) is the implementer's
call, justified in the phase-1 plan.

**R2 — `~` bans materialization, not invocation.** `call` on a `~`-typed value is accepted
and is statically always a splice. Every *materializing* use is a **located error**: storing
it in a struct/array/cell field, returning it as a word output, binding it where an ordinary
`[ ... ]` is expected, or reaching `materialize_quotation_at_boundary` or either `if`-join
erasure site. The last three must be unreachable by construction (no coercion path exists) —
if any is reachable only by a runtime check, say so explicitly and justify it.

**R3 — an ordinary `[ ... ]` does not become `~`, and vice versa.** No implicit widening or
narrowing in either direction; a mismatch is a located error naming both types. Ordinary
first-class quotations (7b's capturing closures) are entirely unaffected by this slice.

### Rows in a quotation effect

**R4 — a row inside a quotation effect must be the signature's own top-level row.** A
`..`-prefixed name inside a declared quotation effect denotes the signature's top-level row.
A fresh name, or any row when the signature declared none at top level, is a **located
error** naming the row and the declared signature.

**R5 — both sides or neither, and the same row.** A row in a quotation effect appears in
both the effect's inputs and its outputs, or in neither. A one-sided row is a **located
error**. For 10a the row is the *same* row on both sides — the back-edge fixed point argued
above. A differing output row is a located error whose message says the body of a loop
cannot change the carried region's shape; 10c is where that restriction lifts for a word
without a back-edge, and the diagnostic should not claim the shape is illegal in general.

**R6 — representation mirrors `PolySig`.** `PolyType::Quotation` (`src/ast.rs:622`) grows
optional row fields in the signature's existing row id space, mirroring
`PolySig.row_in`/`row_out` (`:629-640`). A fully-concrete effect still folds to
`Concrete(Type::Quotation)` (`src/parser.rs:1362`). `QuotEffect` (`src/ast.rs:889`) needs
**no** row field: at every splice the row is concrete. Note that `poly_sig_shape_eq`
(`src/check.rs:3188`/`:3191`) already reads the `PolySig` row fields, so overload dedup
distinguishes candidates differing only by row for free once the representation lands.

**R7 — the pointwise unify walk excludes the row.** `unify_poly_input`'s Quotation arm
(`src/check.rs:5912`, arity check `:5922`) matches only the fixed, non-row slots pairwise.
The row contributes no pairwise slot, binds no type/len variable, and is excluded from the
equal-arity check.

**R8 — row grounding at the declared-effect check sites, derived not inherited.** A
row-bearing declared quotation parameter grounds to the concrete caller-stack region below
its fixed inputs. Three contexts, all of which must be covered explicitly — the pre-revision
spec named two:

1. **Known-literal splice.** `check_poly_combinator_args` (`src/check.rs:7226`) already
   computes `base = stack.len() - n` (`:7240`) before calling `apply_subst` (`:7267`) and
   then `check_literal_against_declared_effect` (`:7272`). The caller therefore already
   holds the row region (`stack[..base]`); the callee has no `stack` parameter at all. The
   grounding is plumbed by giving the grounding step access to that region — extending
   `apply_subst`'s inputs, or substituting the concrete region into the effect it returns
   before the literal check runs. The phase-3 plan states which, in prose, before code.
2. **Abstract pass-down.** The `matches!(found.ty, Type::Quotation(_))` arm alongside
   (`:7276`ff, R21): a quotation parameter forwarded by the combinator whose body is being
   checked. Grounding here is the declared-effect trust already used for type variables,
   extended to carry the row.
3. **Definition-site, no caller.** A combinator's body is checked standalone at its own
   definition, where there is no caller and the row grounds to the **empty** region — which
   is why `: passthru ( ..s i64 -- ..s ) drop ;` compiles today and why `shrinks-row` does
   not. This is the context 10a's own exit repro fires in first, and the pre-revision spec
   missed it.

No abstract row unification, no `Subst` extension (it stays `ty`+`len`), no mangling impact.

**R9 — new rejections render the row.** Neither `poly_type_str`'s Quotation arm
(`src/check.rs:6295-6309`) nor `poly_quotation_concrete_hint` (`:5946`) knows about rows, so
once `PolyType::Quotation` carries them, `[ ..s usize -- ..s ]` would print as
`[ usize -- usize ]` in every R4/R5/R11 diagnostic and a one-sided-row error would show two
identical-looking effects. Both renderers become row-aware, and `~` renders too. Pinned by
asserting the *exact* diagnostic text, not a substring that survives the row vanishing.

### The self-tail back-edge

**R10 — the back-edge arm produces the ground declared outputs.** Rewrite
`src/check.rs:8476-8480` so the arm's result is the combinator's ground declared outputs
(via `apply_subst`), not its non-quotation inputs. The current comment's claim — "the
non-quotation inputs, which for a self-tail combinator are exactly its declared outputs" —
holds only for the state-threading shape (`while`) and is false for a loop that consumes its
counters, which is why the recon-4 `my-times` fails today with a spurious
`` `if` branches leave different stack depths (then: 3, else: 1) ``.

**R11 — the self-call's arguments are checked against the ground declared inputs.**
Replacing the fiction removes the transitive check the `if`-join got from it, so the
back-edge gains an explicit unification of `stack[base..]` against the ground declared
inputs, with a **located** back-edge-argument-mismatch diagnostic. Sound because the marker
matches only in tail position, so the join this feeds is the body-final join. `while` must
check identically before and after; a regression test pins that.

**R12 — the back-edge guards run unchanged.** `check_linear_across_back_edge`
(`src/check.rs:6762`) and `check_reference_across_back_edge` (`:6741`) are untouched; rows
add no exemption. (Named R8/R9 in slice 6's own numbering — this spec's R8/R9 are different
requirements. Cite them by function name, not number.)

**R13 — R10's rewrite forwards the surviving capture set. Its own phase, its own gate.**
The old block builds `Slot::computed(s.ty)` (`src/check.rs:8479`), which sets
`quot: None, surviving: None` (`:224-234`), and its filter excludes a bare erased quotation
but **not an aggregate carrying one** — the exact gap 7b's review fixed in the
getter/array/cell paths (`d1b3f0a`, `bee407c`) and explicitly left here as documented
residual risk. R10 re-sources `outs` from manufactured declared-output types, which can
re-drop the identical set under a new rationale.

This requirement is deliberately **not** satisfiable by prose, because the obvious reading
of it is vacuous: `times`'s declared outputs are the row alone, so it has *zero* fixed
output positions, and any rule phrased as "forward where a positional source exists,
document where it doesn't" auto-satisfies. Worse, the natural positive witness (`while`
carrying an aggregate that holds a closure) is **masked**: `while` exits through a
conditional join and `union_surviving` (`:844`) reconstructs the dropped set from the
sibling arm, so a no-op implementation passes. Therefore:

- **A white-box unit test asserts the forwarded `SurvivingCaptureSetId` on the `outs` slots
  directly, before any join runs**, bypassing `union_surviving` entirely. Not an end-to-end
  program, not a diagnostic — an assertion on the slot state the back-edge arm produces.
- **The test is mutation-tested**: reverting the forward (restoring `Slot::computed`) must
  make it fail, and the phase report records that evidence. A test that cannot be shown to
  fail does not discharge this requirement.
- The phase does not exit on "no witness exists, so the risk stays documented." If that
  genuinely turns out to be the case, it is escalated, not absorbed.

### Exit witnesses

**R14 — user-space `my-times` compiles, sums, and loops in constant stack.** With its row
restored, `: my-times ( ..s usize usize ~[ ..s usize -- ..s ] -- ..s ) ... ;` compiles from
user source **beside** the untouched intrinsic; a concrete call sums correctly; and it runs
1M iterations at `ulimit -s 1024` to completion, exit 0, via `run_at_stack_limit`
(`tests/phase4_combinators.rs:1403`). The exit program is written out in full in the phase
plan and actually compiled — the pre-revision spec's `0 0 5 [ bump ] my-times .` referenced
an undefined `bump`.

**R15 — grounding semantics are pinned, including the part that is weaker than it sounds.**
Writing R14's program out reveals that grounding a row against a concrete region is
type-equality over that region, not a proof the region was "restored unchanged" — a body
that replaces a carried value with a different value of the same type satisfies it. State
that explicitly and pin it with a golden, rather than letting the phrase "the row is
restored" imply more than the check does.

**R16 — aggregate aliasing witness.** An aggregate carried across the row with
per-iteration data dependence prints arithmetically correct fields, so the slice-3 aliasing
class (a stale value from iteration *k* visible at *k+1*) surfaces as a wrong number rather
than a crash.

**R17 — nesting parity.** `my-times` nested inside itself produces correct output.

**R18 — no regression, intrinsic untouched.** `while` and the full existing corpus are
unchanged; `tests/qbe_baseline*` goldens hold (the `i64`→`usize` index is the same QBE
register, so any baseline movement is investigated, not accepted); the intrinsic's arms in
`check.rs`/`ir.rs` still exist and still serve `times`.

**R19 — mutation-test every new guard.** For each located-error golden (R2, R3, R4, R5,
R11), prove the test can fail by deleting the guard it protects and confirming the golden
flips. This project has shipped placebo tests before and reading does not catch them.

## Traceability

| Req | Traces to | Phase |
| --- | --- | --- |
| R1, R2, R3 | `~` decision (this spec; ROADMAP slice 10 entry) | 1 |
| R4, R5, R6, R7 | brief decisions 1–2; open q4 | 2 |
| R8 | brief recon 5 **corrected**; open q1 | 3 |
| R9 | review finding (renderer gap) | 2 (row), 3 (grounding messages) |
| R10, R11 | brief decision 3; recon 4 | 4 |
| R12 | brief decision 4 | 4 |
| R13 | brief decision 5; recon 7; review finding (vacuity + masking) | 5 |
| R14–R18 | brief exit criteria | 6 |
| R19 | project convention | each phase, audited in 6 |

## Phased delivery plan

**Phase 1 — the inline-only quotation type.** *(R1, R2, R3)* Parse `~[ ... ]`, represent it
distinguishably, enforce the materialization ban with located errors, and confirm the three
erasure paths are structurally unreachable rather than runtime-checked. First because the
target signature cannot be spelled until `~` parses. Unit tests beside the parser/type
changes: the sigil round-trips, `call` on a `~` value is accepted, each materializing use is
rejected, and no implicit conversion exists in either direction. Standard difficulty:
additive type-layer surgery with diagnostic care.

**Phase 2 — rows inside a quotation effect.** *(R4, R5, R6, R7; R9's row half)* Grow
`PolyType::Quotation` with row fields mirroring `PolySig`; add the `..` branch to the nested
slot parse, tied to the signature's recorded top-level row; reject a fresh name, a one-sided
row, and a differing output row; exclude the row from the pointwise unify arity; make both
signature renderers row-aware so the new diagnostics actually show the row. No grounding
against a live stack yet. Unit tests: the three rejections with exact expected text, plus a
happy-path parse of `~[ ..s usize -- ..s ]` reaching the checker. Standard.

**Phase 3 — row grounding at the check sites.** *(R8; R9's message half)* Resolve, in prose
first, which check runs for each of the three contexts in R8 — Known-literal splice, abstract
pass-down, and definition-site-with-no-caller — then implement the grounding by plumbing the
row region the caller already holds (`stack[..base]`) into the grounding step. Do **not**
generalise `check_abstract_quotation_times`; it does not do what the brief said. Unit tests:
a row-bearing literal accepted against a matching concrete region and rejected against a
mismatching one, the abstract pass-down shape, and a definition-site check with an empty row
region. **Hard**: the type-checking core, and the requirement whose premise the review round
falsified.

**Phase 4 — back-edge ground declared outputs.** *(R10, R11, R12)* Rewrite
`src/check.rs:8476-8480` to the ground declared outputs; add the explicit unify of
`stack[base..]` against the ground declared inputs with a located diagnostic; confirm the two
back-edge guards are untouched; pin `while` unchanged. This phase deliberately lands the
*type* fix without the surviving-set forward — phase 5 completes it, and 10a does not exit
until it does. Unit tests: `while` byte-identical; a `times`-shaped self-tail combinator that
type-checks where it did not before; the argument-mismatch rejection. **Hard**: rewriting the
exact block 7b's review flagged.

**Phase 5 — the surviving-set gate.** *(R13)* Forward `surviving`/`quot` through phase 4's
rewritten `outs` construction, following `d1b3f0a`/`bee407c`. Land the white-box assertion on
the produced slot state, before any join, and the mutation evidence that it fails when the
forward is reverted. A separate phase, with its own commit and its own review, precisely so
it cannot be quietly folded into phase 4 and shortchanged. **Hard**: a correctness obligation
whose obvious reading is vacuous and whose natural witness is masked.

**Phase 6 — exit witnesses and mutation audit.** *(R14–R19)* The 10a golden suite: the
user-space `my-times` sum and its 1M-iteration constant-stack run, the pinned grounding
semantics, the aggregate aliasing witness, self-nesting, corpus/`while`/intrinsic unchanged,
and an audit that every located-error golden from phases 1–4 has been mutation-tested.
Standard: no new mechanism, but the witnesses that decide whether the slice is real.

## Exit criteria

10a exits when: `~` parses and its materialization ban holds with the erasure paths
structurally unreachable (R1–R3); a row parses, represents, renders, and unifies correctly
inside a quotation effect, with all three rejections golden-tested (R4–R7, R9); grounding
works in all three contexts including definition-site (R8); the back-edge produces ground
declared outputs with the self-call arguments explicitly checked and `while` unchanged
(R10–R12); the surviving-set forward is implemented **and** proven by a mutation-tested
white-box assertion (R13); the user-space `my-times` compiles beside the untouched
intrinsic, sums correctly, runs 1M iterations in constant stack, carries an aggregate
without aliasing, and nests (R14–R18); and every new guard has been shown capable of failing
(R19).

10b and 10c are separate specs and are not gated on anything in this document beyond phases
1–3 (10c) and all of 10a (10b).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "inline-only-quotation-type", "difficulty": "standard" },
    { "phase": 2, "focus": "rows-in-quotation-effects", "difficulty": "standard" },
    { "phase": 3, "focus": "row-grounding-at-check-sites", "difficulty": "hard" },
    { "phase": 4, "focus": "back-edge-ground-declared-outputs", "difficulty": "hard" },
    { "phase": 5, "focus": "surviving-set-forwarding-gate", "difficulty": "hard" },
    { "phase": 6, "focus": "exit-witnesses-and-mutation-audit", "difficulty": "standard" }
  ]
}
```
