# Phase 4 Slice 10a: the inline-only quotation type, and rows in quotation effects

Makes `times`'s signature writable in user source. `times` is a compiler intrinsic only
because its signature was previously unspellable, not because it needs runtime magic:
self-tail-call TCO, quotation params threaded as compile-time constants, and shared `Builder`
loop state are already general. This slice adds the missing surface type.

**Scope: 10a only.** 10b (deleting the intrinsic, moving `times` into `lib/combinators.sth`,
retyping the library) and 10c (`if`/`cond` as ordinary words) are separate specs. 10c consumes
phases 1–4 of this slice; 10b consumes all of 10a.

## Target signature

```sooth
: times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )
```

Today's intrinsic signature plus a spelled row and the `~` sigil.

**Index type is unchanged.** `usize` was rejected: it only moves conversions from indexing
sites to index-accumulating bodies, and `IrType::Usize` normalizes to `Int { signed: false }`,
where signedness still drives `cslt`/`cult` etc. The honest end state is a bounded type variable
(`'T: Int`), but `Bound` is `{ Copy, Ord }` with no `Int` and no arithmetic on bound variables,
so it is its own slice sequenced after 10b. 10a therefore changes **no index type anywhere**;
the intrinsic keeps its hardcoded `Type::I64` sites. R15's user-space witness is written in
`i64` to mirror the future `Int` migration.

**One row, same on both sides.** `times`'s body row is a fixed point, not a transformation:
N=0 leaves the region untouched and N≥2 feeds iteration i's output into iteration i+1's input,
so the declared output row must equal the input row. Encoding this in the signature makes a
violation a declaration-site signature mismatch rather than a later back-edge mismatch. (This
is why 10c's `if` legitimately takes an asymmetric `~[ ..i -- ..o ]`: it runs once, permanently
in the N≤1 case.)

## `~`: the inline-only quotation type

`~[ ... ]` is a quotation type that **cannot be materialized**: no runtime representation,
never stored in a struct/array/cell field, never returned, never captured, never widened or
coerced to an ordinary `[ ... ]`. `call` remains its invocation syntax, because `call` on a `~`
is statically always a splice. **The ban is on materialization, not invocation.**

It belongs in this slice, not 10c: a row-bearing quotation parameter *must* be spliced
(`QuotEffect` has no row field, and a row's size is unknown at runtime), so `~` states a
guarantee every combinator already relied on.

**Representation: `Type::InlineQuotation(&'static QuotEffect)`, mirroring `Type::Quotation`.**
`PolyType::Quotation` carries a `~` flag and grounds to `InlineQuotation`. A distinct `Type`
variant (not a `PolyType`-only flag, not a `Slot` flag) buys R3 for free: `Type` derives
structural `PartialEq`, so `InlineQuotation(e) != Quotation(e)` at every equality site, and the
materialize boundaries reject a `~` by type inequality *before* the boundary. `Type` stays
`Copy` and `is_copy` falls through to `true`, so a `~` local is freely re-readable (R15 reads
`f` twice). Adding the variant breaks exactly three exhaustive matches (`Type::name()`,
`type_node()`, `ir_type_of()`), all arms worth writing.

**Adjacency is required.** `~[` is a single lexer token (`Token::TildeLBracket`), emitted in
the word-scan glue loop like `|>`; `~ [` is a parse error.

## Corrections to the brief (falsified during review)

1. **`check_abstract_quotation_times` is not a reusable prototype.** It pops/type-checks the
   count and pointwise-matches declared outputs against top slots; it never decomposes
   fixed-inputs-above-a-row and never inspects the row. Phase 4 therefore **derives** grounding
   rather than generalising it (R9).
2. **The checker does read `PolySig.row_in`/`row_out`** (`poly_sig_shape_eq`, `poly_sig_str`).
   Consequence: overload dedup distinguishes candidates differing only by row for free (R7).
3. **A top-level row is opaque-and-provably-untouched only.** The checker models it as
   size-zero during the word's own body check and rejects any body that touches it. 10a needs
   exactly this; a genuinely transforming row is 10c's work.

## Requirements

"Located" = diagnostic carries a span and names the offending row/argument and the declared
signature.

### The `~` type

**R1** — `Type::InlineQuotation`, reached by a `~[` token on every parse path. Fill the three
flagged exhaustive matches; add `Token::TildeLBracket` in the lexer glue; extend
`effect_has_variable` to recognise it; recognise it on four entry points (`parse_poly_slot` and
the three `parse_quotation_type_expr` gates), splitting an inner entry point off
`parse_poly_quotation` since the token has consumed the `LBracket`. Record `~` on
`PolyType::Quotation`; ground to `InlineQuotation` in `apply_subst` and the concrete fold. Add
`is_quotation_type(Type) -> Option<&'static QuotEffect>` to `ast.rs` and route every enabling
and routing site through it (two ICE-class defects were found from sites matching
`Type::Quotation` directly). A `~`-bearing signature routes to the poly parser by deliberate
choice, so R9 context 4's unreachability holds.

**R2** — `~` bans materialization, not invocation. Five materialization boundaries (not four):
word output, store-through-ref, declared parameter, `if`-join erasure, and **capture admission**
(the fifth, previously fail-open: without it a `~` local gets bundled into an env). Each rejects
a `~` by type inequality; no runtime check added. Every materializing declaration (`~` as word
output, struct/array/cell field, or `extern` param) is a located error; the field cases were
fail-open and each gets a golden. **Phase 1's defining deliverable is the written disposition of
every silent `Type::Quotation` site**, pinned by pasting the output of
`grep -n 'Type::Quotation' src/check.rs src/ir.rs | grep -vE '://|/// '` with one disposition
per line, split four ways: routing predicates/let-else gates (panic or reroute), enabling sites
(break loudly), fail-open restrictions (slip past silently, each gets a golden), and leave-alone.
Phase 2 delivers six behavioural tests: five boundaries rejecting plus `call` still accepted.

**R3** — no implicit widening/narrowing between `[ ... ]` and `~[ ... ]`; mismatch is a located
error naming both types. Falls out of structural `PartialEq`; pinned with goldens in **both**
directions.

### Rows in a quotation effect

**R4** — a `..`-prefixed name inside a declared quotation effect denotes the signature's own
top-level row. A fresh name, or any row when none was declared at top level, is located error.

**R5** — a row appears in both effect inputs and outputs or neither (one-sided = located error).
For 10a the row is the *same* on both sides; a differing output row errors with exact text:

```
error: a loop body cannot change the shape of the carried region: `..a` in, `..b` out
note: 10c lifts this for a word without a back-edge
```

**R6** — suppress the concrete fold whenever `row_in`/`row_out` is set, **independently of `~`**.
`raw_to_poly_type` folds to `Concrete` iff every *slot* is concrete; the row is a field, not a
slot, so `~[ ..s i64 -- ..s ]` would otherwise collapse to `Concrete(...)` and destroy the row
at parse time. Unit test asserts it stays `PolyType::Quotation` with both row fields populated.

**R7** — `PolyType::Quotation` grows optional row fields in the signature's existing row id
space, mirroring `PolySig`. `QuotEffect` needs **no** row field: at every splice the row is
concrete.

**R8** — `unify_poly_input`'s Quotation arm matches only fixed non-row slots pairwise; the row
binds no variable and is excluded from the equal-arity check.

**R9** — row grounding, in the callee, four contexts. `apply_subst` is **left alone** (it returns
an interned `&'static QuotEffect`; splicing a caller region would mint an effect no literal can
equal and print the caller's stack inside declared types). Instead
`check_literal_against_declared_effect` takes the row region as a new parameter, prepends it to
`fresh` as **type-only `Slot::computed` copies** (`deriv: None`, so the borrow guard doesn't
false-positive on a caller borrow riding untouched in the row), requires it back on the exit row,
and **strips it before rendering** (else every mismatch prints the caller's stack; pinned by
exact-text golden). Five callers gain the parameter (four pass an empty region). Four contexts:
known-literal splice; abstract pass-down (forward arm, comparison must still work); definition-site
with empty region (why `passthru` compiles and `shrinks-row` doesn't); mono declared parameter
(unreachable for a `~` because `inline_combinator` routes a poly combinator to
`check_poly_combinator_args` — phase 4's test asserts the **routing**, not merely absence of
error; also fix the stale "poly words aren't combinators" comment). No abstract row unification,
no `Subst` extension, no mangling impact.

**R10** — new rejections render the row and the sigil. `Type::name()` gains `~[ ... ]` in phase 1.
`poly_type_str`'s Quotation arm, `poly_quotation_concrete_hint`, and `unify_poly_input`'s let-else
(previously rendering `[ -- ]`) become row-aware. All pinned by **exact** text.

### The self-tail back-edge

**R11** — the back-edge arm produces the ground declared outputs along an explicit index map, not
the non-quotation inputs (the current code is correct only for `while`'s state-threading shape;
`my-times` fails today with a spurious depth-mismatch). `SelfTailMarker` grows the ground outputs
and the index map, computed at its sole set site; the source is the `Subst`
`check_poly_combinator_args` computes and currently **discards** — it must be returned.
Index-map rule: declared output *i* maps to non-quotation declared input *i*, **counting from
the deepest slot** (bottom-aligned), `None` when *i* ≥ input count or types differ. A unit test
covers a differing-count shape. (Note the rule degenerates on the standalone path where all
variables bind to `I64`/`4`.)

**R12** — the self-call's arguments are explicitly unified against the ground declared inputs with
a located diagnostic (replacing the transitive check the old fiction provided). `while` must check
identically; regression test pins it.

**R13** — `check_linear_across_back_edge` and `check_reference_across_back_edge` untouched; rows
add no exemption.

**R14 / R14a** — R11's rewrite must forward the surviving capture set along the index map (the
old block's `Slot::computed` drops `surviving`/`quot`, and its filter excludes a bare erased
quotation but **not an aggregate carrying one**). All obvious witnesses are vacuous or masked, so
the proof requires all five: extract the `outs` construction into a named function (phase 5);
witness slot is an aggregate carrying an erased quotation; the shape yields ≥1 `Some(j)` map
entry; a white-box unit test asserts the forwarded `SurvivingCaptureSetId` on produced `outs`
slots directly, before any join; and it is mutation-tested. Phase 5 lands the test `#[ignore]`d
(a missing forward is undetectable end-to-end, so absence must be made visible in the tree);
phase 6 un-ignores it and makes it pass, in its own commit.

### Exit witnesses

**R15** — user-space `my-times` compiles beside the untouched intrinsic, sums correctly, and runs
1M iterations at `ulimit -s 1024` to exit 0 via `run_at_stack_limit`:

```sooth
: my-times ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )
  | f | | to | | from |
  from to < if
    from f call
    from 1 + to f my-times
  else
  end ;

: main ( -- ) 0 0 5 [ + ] my-times . ;    \ prints 10
```

The two `i64` inputs are **from** and **to**. At the definition site the row grounds to the empty
region, so `sig.inputs.len() == 3` and the back-edge checks 3 args against 3 declared non-row
inputs. Reading `f` twice needs no `dup`.

**R16** — grounding semantics pinned, including what they lose: it is type-equality over the
region, not a proof of "restored unchanged", and `Slot::computed` drops provenance, so a borrow
can be dropped and an unrelated borrow of the same referent type substituted. Pin the
borrow-substitution case with a golden.

**R17** — aggregate carried across the row with per-iteration data dependence prints
arithmetically correct fields (slice-3 aliasing surfaces as a wrong number, not a crash).

**R18** — `my-times` nested inside itself produces correct output.

**R19** — no regression. `while` and the full corpus unchanged; `tests/qbe_baseline*` goldens
byte-identical **against the base commit named in the phase-1 report** (sibling sessions land
baseline-rewriting commits). No index type changed, no `~` added to any shipped signature; the
intrinsic's arms still serve `times`; `lib/combinators.sth` is byte-unchanged and contains no `~`.

**R20** — mutation-test every new guard: prove each located error's test can fail by deleting the
guard and confirming the golden flips. Phase 7's audit enumerates them individually.

## Codebase notes

`Type` derives `PartialEq` (`src/ast.rs:820`), `Type::Quotation` (`:879`), `Bound = { Copy, Ord }`
(`:594`). Poly words **are** combinators (`is_combinator` doc `src/check.rs:6913-6920`; the
`:2173-2178` comment saying otherwise is stale — fixed in R9). `Slot::computed` sets
`quot: None, surviving: None, deriv: None` — the drop-flag defect class of `d1b3f0a`/`bee407c`.
Landed prerequisites: 7b `3776579`, 8a `e20c52f`, slice 9 `c5db035`. Line anchors were verified
at `92e7f16`; re-anchor before editing.

## Phased delivery plan

1. **Type variant and audit** *(hard)* — add `Type::InlineQuotation`, fill three arms, add
   `Type::name()`'s spelling and the `is_quotation_type` accessor, then paste the grep output and
   disposition every silent `Type::Quotation` site. Testable by constructing the variant directly;
   this is where a miss fails silently.
2. **Surface syntax and behaviour** *(standard)* — `Token::TildeLBracket`; extend
   `effect_has_variable`; four entry points with the inner-entry split; ground `~` in `apply_subst`
   and the fold; six behavioural tests, materializing-declaration rejections, R3 both directions.
3. **Rows in quotation effects** *(standard)* — row fields on `PolyType::Quotation`; the `..`
   branch in nested slot parse; suppress the fold when a row is set (with the survival unit test);
   reject fresh/one-sided/differing rows at exact text; exclude the row from pairwise arity;
   row-aware renderers.
4. **Row grounding at check sites** *(hard)* — leave `apply_subst` alone; type-only `Slot::computed`
   region into `fresh`, require back on exit, strip before rendering; all four contexts including
   context 4's routing assertion. Do not generalise `check_abstract_quotation_times`.
5. **Back-edge ground declared outputs** *(hard)* — return the `Subst`; extend `SelfTailMarker`
   with ground outputs + bottom-aligned map; rewrite the arm; explicit unify with located
   diagnostic; guards untouched; `while` pinned; extract the `outs` function and land R14's test
   `#[ignore]`d.
6. **Surviving-set forwarding gate** *(hard)* — forward `surviving`/`quot` along the map; un-ignore
   the test with an aggregate-carrying witness (≥1 `Some(j)` entry); record mutation evidence. Own
   commit and review.
7. **Exit witnesses and mutation audit** *(standard)* — `my-times` sum + 1M-iteration run, pinned
   grounding semantics incl. borrow substitution, aggregate aliasing witness, self-nesting,
   corpus/`while`/intrinsic/library unchanged with byte-identical baselines against the named base,
   and an audit enumerating every located error with its mutation evidence.

## Exit criteria

10a exits when: `Type::InlineQuotation` exists with every silent site dispositioned against the
pasted grep (R1–R2); `~[` is a single token whose spaced form errors, recognised on all four entry
points and the routing scan, with all five materialization boundaries rejecting a `~`, `call` still
accepted, every materializing declaration rejected, R3 pinned both directions (R1–R3); a row parses,
represents, renders, and survives the concrete fold, all three rejections golden at exact text
(R4–R8, R10); grounding works in all four contexts, callee-side, type-only, stripped before
rendering (R9); the back-edge produces ground declared outputs along the bottom-aligned map with
self-call arguments explicitly checked and `while` unchanged (R11–R13); the surviving-set forward is
implemented and proven by a mutation-tested white-box assertion over an aggregate slot with a
non-`None` entry (R14, R14a); `my-times` compiles beside the untouched intrinsic, sums, runs 1M
iterations in constant stack, carries an aggregate without aliasing, and nests (R15–R19);
`lib/combinators.sth` and the QBE baselines are byte-unchanged against the named base (R19); and
every new guard has been shown capable of failing (R20).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "type variant and audit", "difficulty": "hard" },
    { "phase": 2, "focus": "surface syntax and behaviour", "difficulty": "standard" },
    { "phase": 3, "focus": "rows in quotation effects", "difficulty": "standard" },
    { "phase": 4, "focus": "row grounding at check sites", "difficulty": "hard" },
    { "phase": 5, "focus": "back edge ground declared outputs", "difficulty": "hard" },
    { "phase": 6, "focus": "surviving set forwarding gate", "difficulty": "hard" },
    { "phase": 7, "focus": "exit witnesses and mutation audit", "difficulty": "standard" }
  ]
}
```
