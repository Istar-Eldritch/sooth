# Phase 4 Slice 10a spec: the inline-only quotation type, and rows in quotation effects

Derives from [`docs/phase4-slice10-brief.md`](./phase4-slice10-brief.md). The brief's recon
is treated as ground truth **except where this spec says otherwise** — review falsified two
of its claims, and both are corrected here rather than inherited.

`times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` is a compiler intrinsic only because user
source cannot *spell its signature*, not because it needs magic to run. Everything under it
is already general: blanket self-tail-call TCO, quotation parameters threaded as
compile-time constants, shared `Builder` loop state (`save_loop_state` `src/ir.rs:3016`,
`alloca_home` `:2718`). This slice makes the signature writable.

**Scope is 10a only.** 10b (deleting the intrinsic, moving `times` into
`lib/combinators.sth`, and deciding how much of the rest of the library retypes) gets its
own brief and spec, so migration risk and mechanism risk do not share a review.

**Phase numbering.** Seven phases. 10c (`if`/`cond` as ordinary words,
`docs/phase4-slice10c-brief.md`) consumes **phases 1–4** — the `~` type, its surface syntax,
rows, and grounding — and nothing beyond. It does not gate on phase 5's back-edge rewrite,
phase 6, or 10b. Any document saying 10c gates on "phases 1–2" or "phases 1–3" predates a
renumbering.

## The target signature

```sooth
: times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )
```

Today's intrinsic signature plus a spelled row and the `~` sigil. **The index type does not
change in this slice, and should not change in 10b either.**

An earlier revision moved the count and index to `usize`, on the strength of the corpus
round-trip: `len` returns `usize` (`src/check.rs:5261`), so all four count-taking library
combinators convert it *down* with `len >i64` purely to satisfy `times` and convert each
index back *up* per iteration (twice in one body in `examples/array_totals_hand.sth:20` and
`examples/inplace_fold.sth:33`). The observation is real; `usize` is the wrong repair.

It only moves the cost. A `usize` index cannot be added to a computed `i64`, so `usize`
deletes a conversion at every indexing site and *adds* one in every body that accumulates
the index — precisely R15's summing witness and R17's aggregate witness. (The "same QBE
register" argument an earlier draft leaned on is also the wrong rationale: `IrType::Usize`
normalizes to `Int { signed: false }`, `src/backend/qbe.rs:631-633`, and signedness drives
`cslt`/`cult`, `sar`/`shr`, and `$fmt`/`$ufmt`.)

The honest fix is a bounded type variable — `times ( ..s 'T: Int ~[ ..s 'T -- ..s ] -- ..s )`
— so an indexing caller instantiates at `usize`, an arithmetic caller at `i64`, and neither
converts. That is the recorded end state and it is **not available today**: `Bound` is
`{ Copy, Ord }` (`src/ast.rs:594`) with no `Int`, and arithmetic on a bound type variable is
unsupported. Isolated to one changed token:

```sooth
: add2 ( 'T: Copy Ord 'T -- 'T ) over over > if drop else swap drop end ;   \ compiles
: add2 ( 'T: Copy Ord 'T -- 'T ) + ;                                        \ fails
```

`<` works on a bound variable; `+` does not, and fails with a misleading diagnostic
(`` `+` needs 2 values, but the stack holds 0 `` plus `note: declared ( -- )`, neither true
of the signature). An `Int` bound therefore needs a new `Bound` variant, operator support on
bound type variables, per-instantiation monomorphization, and a diagnostic fix: its own
slice, sequenced after 10b. Migrating to `usize` first would only be re-migrated.

**Consequence: 10a changes no index type anywhere.** The intrinsic keeps its two hardcoded
`Type::I64` sites (`src/check.rs:6853`, `:6855`) and the Known-literal path's own count check
(`:8237`), untouched. R15's user-space witness is written in `i64` so it mirrors what the
future `Int` slice will migrate.

**One row, the same on both sides.** `times`'s body row is a fixed point, not a
transformation, and this is forced: the body's output *is* the next iteration's input across
the back-edge. N=0 leaves the region untouched, so the declared output row must equal the
input row; N≥2 feeds iteration 1's output into iteration 2, whose declared input is the input
row. Only N≤1 would admit a differing output row, and N is a runtime count. Encoding the
fixed point in the signature makes violating it a signature mismatch at the declaration
instead of a back-edge row mismatch later. (This is why 10c's `if` legitimately takes
`~[ ..i -- ..o ]`: it runs its branch once, so it is permanently in the N≤1 case. `while` and
`fold` are the symmetric case — `fold`'s accumulator rides *inside* `..s` at a fixed shape
and type, which is why accumulation needs no asymmetry.)

## `~`: the inline-only quotation type

`~[ ... ]` is a quotation type that **cannot be materialized**: no runtime representation,
never stored in a struct/array/cell field, never returned, never captured by an enclosing
quotation, never widened or coerced to an ordinary `[ ... ]`, never reaching an erasure
boundary. `call` remains its invocation syntax — unchanged from how a combinator's body
invokes its own quotation parameter today — because a `call` on a `~` value is statically
always a splice, never a runtime dispatch. The ban is on materialization, not invocation.

**Why it belongs in this slice rather than 10c.** A row-bearing quotation parameter *must*
be spliced: `QuotEffect` (`src/ast.rs:875-882`) has no row field and a row's size is not
known at runtime, so there is nowhere for the row to live in a materialized value. Every
combinator today relies on that as an unstated guarantee. `~` states it.

**Representation: a distinct `Type` variant, `Type::InlineQuotation(&'static QuotEffect)`,
mirroring `Type::Quotation` (`src/ast.rs:879`).** `PolyType::Quotation` carries a `~` flag
and grounds to `InlineQuotation`.

*Two earlier designs are dead; recording why, because the reasoning is the requirement.*

The first was **poly-layer-only** — record `~` on `PolyType`, suppress the fold to
`Concrete`, so `Type` never grows a case. It is **false**.
`check_poly_combinator_standalone` (`src/check.rs:4856-4917`) checks a poly combinator's body
by calling `apply_subst` on every declared input, quotation ones included (`:4877-4881`), and
hands the result to the monomorphic `check_word` through a stand-in `WordDef` with
`poly: None` (`:4901`). `apply_subst`'s Quotation arm returns `crate::ast::quotation_type(...)`
— a real `Type::Quotation` (`:5989-5998`). So a `~` parameter *does* get a concrete `Type`,
on exactly the path this slice lives on.

The second was a **value-level `inline_only` flag on `Slot`**. Rejected for failing in the
wrong direction: the flag is lost by any `Slot` construction that does not forward it, and
`Slot::computed` (`:224-234`) hard-codes `quot: None, surviving: None`, so a dropped flag
silently turns a `~` into an ordinary quotation with no error and no failing test — the exact
defect this codebase hit twice with `surviving` (`d1b3f0a`, `bee407c`). It also leaves R3
with no structural enforcement, since type equality cannot see a slot flag.

The distinct variant was itself rejected once, on an estimate of "96 silent non-match sites".
That estimate was wrong by a factor of thirty, measured by adding the variant and compiling:
**three** errors, all in exhaustive matches, all arms worth writing anyway — `Type::name()`
(`src/ast.rs:1066`, the `~[ ... ]` spelling), `type_node()` (`src/check.rs:3316`, `None`: a
`~` is never a field), and `ir_type_of()` (`src/ir.rs:219`, `unreachable!()`: a `~` never
reaches the backend). Independently reproduced twice, each time followed by filling the arms
and re-running `cargo check --all-targets` and `cargo test --no-run` clean, so no test crate
hides a fourth.

What the variant buys and neither alternative could: **R3 free and exact**, because `Type`
derives structural `PartialEq` (`src/ast.rs:820`), so `InlineQuotation(e) != Quotation(e)` at
every equality site including `match_slot`'s `Exact` path (`src/check.rs:275-277`); and the
materialize boundaries rejecting a `~` by type inequality *before* the boundary rather than
at it. `Type` also stays `Copy` (the payload is `&'static`) and `is_copy` falls through
`_ => true` (`:491-512`), so a `~` local stays freely re-readable — which R15's witness needs,
since it reads `f` twice.

**The cost is an audit, and the audit is the deliverable.** The compiler flags three sites;
every other `Type::Quotation` site in `src/check.rs` becomes a silent non-match. Phase 1
dispositions each one in writing (R2). They split four ways:

- **Routing predicates and let-else gates** — the dangerous group, because they *panic* or
  silently reroute rather than merely not matching: the let-else at `:7268-7270`
  (`unreachable!()` on the statement immediately after `apply_subst` grounds a `~`);
  `poly_input_is_quotation` `:6945-6950` and `word_declares_quotation_parameter` `:6931`
  (a concrete `~` failing these makes the word not a combinator, so it is never spliced, is
  lowered as an ordinary call, and reaches `ir_type_of`'s `unreachable!()`); `unify_poly_input`'s
  let-else `:5913`; `ref_parts` gate `:8354`; mono gate `:7110`.
- **Enabling** — extend, and they break loudly if missed: `call` `:8165`, `times` `:8220`,
  the abstract-forward arms `:7115`/`:7275`.
- **Fail-open restrictions** — extend, each with a golden, because a `~` slips past silently:
  the aggregate-field rejections `:1873`, `:1899`, `:1925`, `:1942`, `:1947`, `:2032`
  (`reject_quotation_type_position` `:2031-2038` is `if let Type::Quotation(eff) = ty { Err }`,
  so a `~` returns `Ok`); the capture-admission guard `:7591`; the back-edge filter `:8478`
  (fails open until phase 5 deletes the line).
- **Leave alone** — the `if`-join erasure `:8776-8798` (unreachable by a different mechanism:
  its `expected` is sourced only from a declared word output or a `&!` referent, both banned
  at declaration by R2) and the join's own producer `:8797`.

**Adjacency is required.** `~[` is a single token. `~` is not a delimiter (`is_delimiter`,
`src/lexer.rs:24-26`) and `[` is, so `~[` and `~ [` both lex today as `Word("~")` +
`LBracket`, discarding adjacency. The token cannot live in `is_delimiter`, which is
`fn(char) -> bool`; it belongs in the word-scan loop (`src/lexer.rs:168-184`), mirroring the
existing `|>` glue at `:172-181`: when the scanned text is `~` and the next char is `[`,
consume it and emit `Token::TildeLBracket`. Adding that `Token` variant breaks **zero**
exhaustive matches (verified by compiling).

**10a applies `~` to `times`'s signature shape alone**, via the user-space witness. Whether
`each`/`map`/`fold`/`filter`/`while` retype is 10b's question. `lib/combinators.sth` is
byte-unchanged by this slice and contains no `~`.

## Corrections to the brief

**1. `check_abstract_quotation_times` is not the prototype the brief claims.** The brief
(recon 5, echoed in the ROADMAP) says it implements "pop the declared fixed inputs above an
opaque row, require the row restored." It does not. It pops and type-checks the count
(rejecting a quotation count, `src/check.rs:6849-6851`, requiring `Type::I64`, `:6853`), then
requires the *declared effect* be self-similar and pointwise-matches the declared outputs
against the top slots (`:6855-6872`):

```rust
let row_preserving = eff.inputs.last() == Some(&Type::I64)
    && eff.inputs.len() == eff.outputs.len() + 1
    && eff.inputs[..eff.outputs.len()] == eff.outputs[..];
let base = stack.len() - row_len;          // row_len = eff.outputs.len()
for (i, want) in eff.outputs.iter().enumerate() { match_slot(stack[base + i], *want) ... }
```

No fixed-inputs-above-a-row decomposition, and the row is never inspected — everything below
the matched slots is untouched by construction. So phase 4 **derives** the grounding rather
than generalising a prototype, and **R9** says how.

**2. "Nothing in the checker reads `PolySig.row_in`/`row_out`" is stale.** `poly_sig_shape_eq`
reads both (`src/check.rs:3188`/`:3191`) and `poly_sig_str`'s `render_row` (`:3223`) prints
them (called `:3231-3232`); the repl sites are `src/repl.rs:2407` and `:3010`/`:3015`. This
matters for **R7**: `poly_sig_shape_eq` drives overload dedup, so candidates differing only by
row become distinguishable for free.

**3. A top-level row is weaker than the brief implies.** The checker models it as size-zero
during the word's own body check and rejects any body that touches it:
`: shrinks-row ( ..a i64 -- ..b ) drop drop ;` fails with
`` `drop` needs 1 values, but the stack holds 0 ``. Differing `row_in`/`row_out` names parse,
but nothing verifies they differ semantically. Only "opaque and provably untouched" is
supported. 10a does not change that (it is exactly what `times` needs); 10c is where a
genuinely transforming row becomes new integration work.

## Codebase map

Verified against the tree at `92e7f16`. The tree moves — re-anchor before editing.

| Concern | Location | Notes |
| --- | --- | --- |
| `Type` and its three exhaustive matches | `Type` `src/ast.rs:821` (derives `:820`), `Type::Quotation` `:879`; breaks `Type::name()` `:1066`, `type_node()` `src/check.rs:3316`, `ir_type_of()` `src/ir.rs:219` | Measured by compiling. R1. |
| Lexer | `is_delimiter` `src/lexer.rs:24-26` (`fn(char) -> bool`, **cannot** host `~[`); word-scan loop `:168-184`; existing `\|>` glue `:172-181` | R1 adds `Token::TildeLBracket` in the glue, not the delimiter set. |
| Poly/mono routing | `effect_has_variable` `src/parser.rs:1138-1149` | Scans for a `Token::Word` starting `'` or `..`. A `~[` token matches **neither**. One line, load-bearing (R1). |
| Poly quotation parse | `parse_poly_quotation` `src/parser.rs:1251` (opens `expect(LBracket)` `:1252`); **sole caller** `:1211`; internal `parse_poly_quot_list` calls `:1253`/`:1255`; `RawTy::Quotation` declared `:641`, constructed `:1257` | The `~[` token has eaten the bracket, so R1 needs an inner entry point; redirect `:1211`. |
| **Concrete** quotation parse | `parse_quotation_type_expr` defined `src/parser.rs:1590`; **three** call sites `:1404`, `:1451`, `:1683`, each behind its own `LBracket` peek at `:1402`, `:1449`, `:1681` | Unnamed array slot, `parse_type_expr` (ref/cell referents, externs), struct fields. R2's three located errors each need their own gate edit. |
| Nested slot parse | `parse_poly_slot` `src/parser.rs:1208` | Where `~` dispatches and where the row `..` branch is added (R5). |
| Top-level row parse | `parse_poly_slots` `src/parser.rs:1178`; `PolyBuilder` `:662` (`row_in` `:669`, `row_out` `:670`); `set_row` `:676`; `row_var_misplaced_error` `:747`, fired `:1197` | A later `..` in a slot list is already located. |
| Raw → poly fold | `raw_to_poly_type` `src/parser.rs:1346`, Quotation arm `:1362-1381`, **predicate `:1378-1381`** | Folds iff every *slot* is concrete. The row is a field, not a slot, so it is **dropped** by the fold — see R1/R7. |
| Poly representation | `PolyType::Quotation` `src/ast.rs:622`; `PolySig` `:629` (`row_in` `:631`, `row_out` `:636`, `row_var_names` `:640`); `QuotEffect` `:875-882`; `Bound` `:594` | Where `~` and the row live. |
| Routing predicates | `poly_input_is_quotation` `src/check.rs:6945-6950` (used `:5645`, `:6938`, `:7251`, `:7263`); `word_declares_quotation_parameter` `:6931`; `is_combinator` `:6921`; `collect_combinators` `:6881` | **Poly words ARE combinators** (`is_combinator` doc `:6913-6920`). The comment at `:2173-2178` claiming otherwise is stale. |
| Pointwise unify | `unify_poly_input` `src/check.rs:5845`, Quotation arm `:5912`, **let-else `:5913`**, arity check `:5922` | The let-else reports an expected type of `quotation_type(vec![], vec![])`, i.e. `[ -- ]` — a type nobody wrote (R10). |
| Grounding a declared effect | `apply_subst` `src/check.rs:5963`, Quotation arm `:5989-5998`; `quotation_type` `src/ast.rs:895` | Returns an **interned** `&'static QuotEffect`. Not the place to splice a caller region (R9). |
| Splice-site checks | `check_poly_combinator_args` `src/check.rs:7226` (`n` `:7240`, `base` `:7244`, `Subst` `:7249` **discarded**, Pass 2 `:7262-7289`, `apply_subst` `:7267`, **let-else `:7268-7270`**, literal check `:7272`, abstract arm `:7275`, comparison `:7280`); `check_literal_against_declared_effect` `:7301` (`fresh` `:7318`, move-state diff `:7325-7336`, borrow guard `:7337-7346`, exit row `:7357`, mismatch render `:7364-7368`) | The caller holds `stack`/`base`; the callee does not. R9 plumbs the **callee**. |
| Combinator dispatch | `inline_combinator` `src/check.rs:7076`, **poly branch `:7097`**, mono path `:7102-7118` (`:7110` gate, `:7112` caller); `check_term` `:8000`; `has_self_tail_call` `:3943`; `check_combinator_cycles` `:6966` | `:7097` is the real guard for R9 context 4. |
| Capture admission | `captured_quotation_name_deferred_error` guard `src/check.rs:7591` | The **fifth** boundary, and it fails open. R2. |
| Self-tail back-edge | `SelfTailMarker` `src/check.rs:647-651` (only `name`, `input_count`); **sole set site** `inline_combinator` `:7177-7178`; matched `:8452-8456`; arm `:8441-8484`; `outs` `:8476-8480` | Phase 5 rewrites `:8476-8480` and extends the marker (R11). |
| Back-edge guards | `check_linear_across_back_edge` `src/check.rs:6762` (invoked `:8469`); `check_reference_across_back_edge` `:6741` (invoked `:8471`) | Unchanged (**R13**). |
| Erasure boundaries | `materialize_quotation_at_boundary` def `src/check.rs:7644`, called `:4492` (word output), `:8357` (`!`/`+!` store, gated by `ref_parts` at `:8354`), `:8570` (declared parameter); `if`-join erasure `:8776-8798` | With capture admission (`:7591`), **five**. |
| Surviving set | `Slot` `src/check.rs:194`, `quot` `:208-212`, `surviving` `:213-218`; `Slot::computed` `:224-234` (sets `quot: None, surviving: None`, `deriv: None`); `union_surviving` `:844` (join `:8726`); `intern_surviving_set` `:826` | Phase 6's subject. Pattern: `d1b3f0a`, `bee407c`. |
| Signature renderers | `Type::name()` `src/ast.rs:1066`; `poly_type_str` `src/check.rs:6284`, Quotation arm `:6295-6309`; `poly_quotation_concrete_hint` `:5946`, called `:5927`; `poly_sig_shape_eq` `:3188`/`:3191` | **R10**. `Type::name()` is phase 1; the poly renderers are phases 3–4. |
| Intrinsic (untouched in 10a) | `check_term` `"times"` arm `src/check.rs:8208`, count check `:8237`, abstract path `:8221`; `check_abstract_quotation_times` `:6840`; `ir.rs` `"times"` lowering `:3441` | 10b deletes these. `ir.rs:5804` is a **test** call inside `times_saves_and_restores_loop_state` (`:5761`); `times` is absent from `BUILTIN_WORDS` (`check.rs:2371-2401`). |
| Constant-stack witness | `run_at_stack_limit` `tests/phase4_combinators.rs:1403`; `three_deep_times_nesting_runs_in_constant_stack` `:1127` | Reuse for R15. |

Landed prerequisites: 7b `3776579`, 8a `e20c52f`, slice 9 `c5db035`.

## Requirements

"Located" means the diagnostic carries a span and names the offending row/argument and the
declared signature.

### The `~` type

**R1 — `~` is `Type::InlineQuotation`, reached by a `~[` token on every parse path.**

- Add `Type::InlineQuotation(&'static QuotEffect)` beside `Type::Quotation` and fill the
  three exhaustive matches the compiler flags.
- Add `Token::TildeLBracket` in the lexer's word-scan glue (`src/lexer.rs:172-181`), so
  adjacency is required and `~ [` is a parse error.
- Extend `effect_has_variable` (`src/parser.rs:1138-1149`) to recognise the token, or a
  `~`-only signature routes to the mono parser.
- Recognise the token on **four** entry points: `parse_poly_slot` (`:1208`) and the three
  `parse_quotation_type_expr` gates (`:1402`, `:1449`, `:1681`). `parse_poly_quotation`
  (`:1251`) opens with `expect(LBracket)`, which the token has consumed — split off an inner
  entry point and redirect its sole caller (`:1211`).
- Record `~` on `PolyType::Quotation`; ground it to `Type::InlineQuotation` in `apply_subst`
  (`src/check.rs:5989-5998`) and in the concrete fold (`src/parser.rs:1362-1381`).
- **Add `is_quotation_type(Type) -> Option<&'static QuotEffect>` to `ast.rs`** and route every
  enabling and routing site through it, rather than adding a second arm at each. Two ICE-class
  defects were found in one review round by sites that pattern-match `Type::Quotation`
  directly; an accessor is the version that cannot be missed a third time.

*Poly-forcing is a choice, not a necessity.* A `~`-bearing signature routes to the poly
parser, so `WordDef.poly = Some(..)` and `WordDef.effect` stays empty. Unlike a row, this is
now a deliberate choice — a fully concrete `~` effect *is* representable as a `Type` — and it
is made because R9 context 4's unreachability depends on it.

**R2 — `~` bans materialization, not invocation; and phase 1 dispositions every site.**
`call` on a `~` is accepted and is statically always a splice (`src/check.rs:8165`).

There are **five** materialization boundaries, not four. The fifth is capture admission
(`:7591`): the guard that stops an inner quotation literal capturing a quotation-typed local.
A `~` local is exactly that — `| f |` in R15's witness binds one — and with `InlineQuotation`
unlisted the guard passes, the `~` is recorded in a surviving capture set, and lowering
materializes it into an env bundle: the single thing `~` exists to forbid. It gets a located
error ("a `~` quotation cannot be captured") and its own golden.

The other four are `materialize_quotation_at_boundary` at `:4492` (word output), `:8357`
(store through a ref, gated by `ref_parts` at `:8354`), `:8570` (declared parameter), and the
`if`-join erasure `:8776-8798`. Each fires on a `Type::Quotation` target a `~` cannot satisfy,
so each rejects by type inequality before the boundary. No runtime check is added at any.

Every remaining materializing declaration — a `~` as a word output, a struct/array/cell field,
or an `extern` parameter — is a **located error**. The field cases are fail-open today
(`reject_quotation_type_position` `:2031-2038` returns `Ok` for a `~`) and each needs a golden.

**Phase 1's defining deliverable is the written disposition of every silent site**, per the
four-way split in the representation section. It is pinned mechanically, not by prose count:
the phase report pastes the output of

```sh
grep -n 'Type::Quotation' src/check.rs src/ir.rs | grep -vE '://|/// '
```

with one disposition per line. "Every site dispositioned" is otherwise not checkable at
review, and an omission from the list is exactly the failure this phase exists to prevent.

Phase 2 delivers **six** behavioural tests: one per boundary showing a `~` is rejected before
reaching it, plus one showing `call` on a `~` is still **accepted** — without the last, an
over-eager check silently breaks invocation and nothing notices.

**R3 — an ordinary `[ ... ]` does not become `~`, and vice versa.** No implicit widening or
narrowing; a mismatch is a located error naming both types. This falls out of the
representation: `Type` derives structural `PartialEq` (`src/ast.rs:820`), so
`InlineQuotation(e) != Quotation(e)` at every equality site including `match_slot`'s `Exact`
path (`src/check.rs:275-277`). The requirement is to **pin** it with goldens in **both**
directions — a `~` literal where an ordinary quotation is declared, *and* an ordinary literal
where a `~` is declared — and to confirm no site coerces between them.

### Rows in a quotation effect

**R4 — a row inside a quotation effect must be the signature's own top-level row.** A
`..`-prefixed name inside a declared quotation effect denotes the signature's top-level row.
A fresh name, or any row when the signature declared none at top level, is a **located error**.

**R5 — both sides or neither, and the same row.** A row appears in both the effect's inputs
and its outputs, or in neither. A one-sided row is a **located error**. For 10a the row is the
*same* row on both sides; a differing output row is a located error whose text is exactly:

```
error: a loop body cannot change the shape of the carried region: `..a` in, `..b` out
note: 10c lifts this for a word without a back-edge
```

Fixed here so the requirement is objectively judgeable and the message does not claim the
shape is illegal in general.

**R6 — the concrete fold must not eat the row.** `raw_to_poly_type`'s predicate
(`src/parser.rs:1378-1381`) folds to `PolyType::Concrete` iff every *slot* in both lists is
concrete. Per R7 the row is a **field**, not a slot, so `~[ ..s i64 -- ..s ]` yields
`ins=[Concrete(i64)]`, `outs=[]`, both "concrete", and collapses to
`Concrete(quotation_type([i64], []))` — **destroying the row at parse time**, before any
splice, on the exact signature this slice exists to make writable. `QuotEffect` has nowhere to
put it.

The fold is therefore suppressed whenever `row_in` or `row_out` is set, **independently of
`~`**. R7's justification ("at every splice the row is concrete") does not cover this: the
fold is not a splice. A phase 3 unit test asserts `~[ ..s i64 -- ..s ]` stays
`PolyType::Quotation` with both row fields populated.

**R7 — representation mirrors `PolySig`.** `PolyType::Quotation` (`src/ast.rs:622`) grows
optional row fields in the signature's existing row id space, mirroring
`PolySig.row_in`/`row_out` (`:629-640`). `QuotEffect` (`:875-882`) needs **no** row field: at
every splice the row is concrete (subject to R6). Because `poly_sig_shape_eq`
(`src/check.rs:3188`/`:3191`) already reads the `PolySig` row fields, overload dedup
distinguishes candidates differing only by row for free.

**R8 — the pointwise unify walk excludes the row.** `unify_poly_input`'s Quotation arm
(`src/check.rs:5912`, arity check `:5922`) matches only the fixed, non-row slots pairwise. The
row contributes no pairwise slot, binds no variable, and is excluded from the equal-arity
check.

**R9 — row grounding, in the callee, in four contexts.** A row-bearing declared quotation
parameter grounds to the concrete caller-stack region below its fixed inputs.

*Mechanism.* `apply_subst` (`src/check.rs:5963`) returns an interned `&'static QuotEffect` via
`quotation_type` (`src/ast.rs:895`); splicing a caller region into it would mint an effect no
literal and no forwarded parameter can equal, breaking the abstract comparison at `:7280` and
printing the caller's stack inside declared types. So **`apply_subst` is left alone.** Instead
`check_literal_against_declared_effect` (`:7301`) takes the row region as a new parameter,
prepends it to the `fresh` sub-stack (`:7318`), and requires it back on the exit row (`:7357`).
The caller holds it: `base` is computed at `:7244`, so the region is `stack[..base]`.

*The prepended region is type-only.* Prepend `Slot::computed(ty)` copies, not the caller's real
slots. The borrow guard over the exit row (`:7337-7346`) errors on any result slot whose
`deriv` traces to an enclosing local, so prepending real slots would report a caller borrow
riding untouched in the row as `quotation borrows place` — a false positive on correct code.
`Slot::computed` sets `deriv: None`, which is also exactly what R16 declares the grounding
semantics to be. (The move-state diff at `:7325-7336` reads `scope.moves`, not the prepended
slots, and is unaffected.) Pinned by a test whose caller row holds a live borrow.

*The region is stripped before rendering.* The mismatch diagnostic at `:7364-7368` builds
`actual` from `result`, which now contains the prepended region — so without stripping, every
grounding mismatch prints the caller's stack inside the effect, the precise defect this
mechanism was chosen to avoid. Pinned with an exact-text golden (R10).

*Five callers.* The new parameter touches `:7112` (mono declared parameter), `:7272` (the poly
path), `:7666` (inside `materialize_quotation_at_boundary`), and `:8780`/`:8784` (the
`if`-join). All but `:7272` pass an **empty** region — their effects are `QuotEffect`s, which
carry no row — but they are enumerated so phase 4 does not discover the signature change
mid-flight.

The four contexts:

1. **Known-literal splice** — the path above.
2. **Abstract pass-down** — the forward arm at `:7275`, reached through the let-else at
   `:7268-7270`, which R1's accessor must accept. Grounding is the declared-effect trust
   already used for type variables, extended to carry the row; the comparison at `:7280` must
   still work, which is why the interned effect is left untouched.
3. **Definition-site, no caller** — a combinator's body checked standalone, where the row
   grounds to the **empty** region. This is why `: passthru ( ..s i64 -- ..s ) drop ;` compiles
   and `shrinks-row` does not, and it is the context the exit repro fires in first.
4. **Mono declared parameter** — `:7102-7118`. Unreachable for a `~`, because
   `inline_combinator` branches on `comb.word.poly.is_some()` at **`:7097`** and routes a poly
   combinator to `check_poly_combinator_args`, so `:7102-7118` is reached only by a word with
   `poly: None`. R1's routing extension is what guarantees a `~` word is never such a word.
   The unreachability therefore depends on R1's poly-routing holding, and phase 4's test must
   assert the **routing**, not merely the absence of an error.

   *(Two earlier drafts justified this with `collect_combinators` registering only monomorphic
   words. That is false: `:2173-2178` is a stale comment, `is_combinator`'s own doc
   (`:6913-6920`) says a polymorphic quotation-taking word is a combinator too, and a probe
   confirms a poly combinator reaches the back-edge. Fix that comment while here.)*

No abstract row unification, no `Subst` extension (it stays `ty`+`len`), no mangling impact.

**R10 — new rejections render the row and the sigil.** `Type::name()` (`src/ast.rs:1066`)
gains the `~[ ... ]` spelling in **phase 1**, since R3's phase-2 goldens name both types.
Neither `poly_type_str`'s Quotation arm (`src/check.rs:6295-6309`) nor
`poly_quotation_concrete_hint` (`:5946`) knows about rows, so once `PolyType::Quotation`
carries them, `[ ..s i64 -- ..s ]` would print as `[ i64 -- i64 ]` in every R4/R5/R9
diagnostic. Both become row-aware. `unify_poly_input`'s let-else (`:5913`) currently renders
an expected type of `[ -- ]`, which is nobody's declaration, and is fixed here since it is the
R3-direction mismatch path. All pinned by **exact** text, never a substring that survives the
row vanishing.

### The self-tail back-edge

**R11 — the back-edge arm produces the ground declared outputs, along an explicit index map.**
Rewrite `src/check.rs:8476-8480` so the arm's result is the ground declared outputs, not the
non-quotation inputs. The current comment's claim holds only for `while`'s state-threading
shape and is false for a loop that consumes its counters, which is why the recon-4 `my-times`
fails today with a spurious
`` `if` branches leave different stack depths (then: 3, else: 1) ``.

*Reaching them is the work.* `SelfTailMarker` (`:647-651`) carries only `name` and
`input_count`; the arm has no `sig`, no `Subst`, no `arrays`. The marker therefore grows the
ground outputs and the index map, computed at its **sole** set site, `inline_combinator`
`:7177-7178`. There is exactly one source: `check_poly_combinator_args` computes a `Subst` at
`:7249` and **discards it**; it must be returned. Its call site (`:7097`) is eighty lines above
the marker set site inside the same function, so the plumbing is local.
(`check_poly_combinator_standalone` never sets the marker — a standalone body's self-call
resolves the real poly `WordDef` and takes the `:7097` branch — so an earlier draft's second
source does not exist. This *shrinks* the phase.)

*The index map rule, pinned:* declared output *i* maps to non-quotation declared input *i*,
**counting from the deepest slot**; `None` when *i* is at or beyond the input count, or when
the types differ. Bottom-aligned, not top-aligned: they agree for
`while ( 'a [ 'a -- 'a bool ] -- 'a )` (1↔1, today's implicit rule) and are vacuous for
`times`-shape, but disagree for asymmetric shapes such as `( ..s i64 i64 ~[ .. ] -- ..s i64 )`,
where bottom-aligned gives output 0 ← `from`. A unit test covers a differing-count shape.
Note the rule degenerates on the standalone path, where every type variable binds to
`Type::I64` and every length variable to `4` (`:4871-4875`), so structurally distinct variables
are type-identical and the map pairs positions a real splice would leave unpaired: harmless for
`times`-shape, over-permissive for R14 there.

**R12 — the self-call's arguments are checked against the ground declared inputs.** Replacing
the fiction removes the transitive check the `if`-join got from it, so the back-edge gains an
explicit unification of `stack[base..]` against the ground declared inputs, with a **located**
diagnostic. Sound because the marker matches only in tail position. `while` must check
identically before and after; a regression test pins that.

**R13 — the back-edge guards run unchanged.** `check_linear_across_back_edge`
(`src/check.rs:6762`) and `check_reference_across_back_edge` (`:6741`) are untouched; rows add
no exemption. (Named R8/R9 in slice 6's own numbering — cite them by function name, not number.)

**R14 — R11's rewrite forwards the surviving capture set. Its own phase, its own gate.** The
old block builds `Slot::computed(s.ty)` (`:8479`), which sets `quot: None, surviving: None`,
and its filter excludes a bare erased quotation but **not an aggregate carrying one** — the gap
7b's review fixed elsewhere (`d1b3f0a`, `bee407c`) and explicitly left here.

The obvious readings are all vacuous or masked: `times` has **zero** fixed output positions, so
"forward where a source exists" auto-satisfies; the natural end-to-end witness exits through a
conditional join where `union_surviving` (`:844`, called `:8726`) reconstructs the dropped set
from the sibling arm; and the old filter already excludes **bare** quotation slots, so a
white-box test over a bare `Type::Quotation` proves nothing. Therefore all five:

1. **The `outs` construction is a named, callable function** — extracted in **phase 5** (see
   R14a), since `#[ignore]` skips execution but not compilation.
2. **The witness slot is an aggregate carrying an erased quotation** — `ty` a struct,
   `surviving: Some(..)`, `quot: None`.
3. **The witness shape produces at least one `Some(j)` index-map entry**, and the test asserts
   `outs[i].surviving == Some(set)`. Without this, a shape whose every entry is `None` makes
   clause 4 assert `None == None` and clause 5's mutation invisible.
4. **A white-box unit test asserts the forwarded `SurvivingCaptureSetId` on the produced `outs`
   slots directly, before any join runs**, bypassing `union_surviving`.
5. **The test is mutation-tested**: reverting the forward must make it fail, and the phase
   report records that evidence.

The phase does not exit on "no witness exists, so the risk stays documented." That is escalated,
not absorbed.

**R14a — phase 5 extracts the function and lands R14's test `#[ignore]`d.** Phase 5 lands green
by construction: with the masking above, a missing forward is undetectable end to end, so CI
cannot notice phase 6 being skipped. Phase 5 therefore extracts the function and lands phase 6's
white-box test against it, `#[ignore]`d with a reason naming phase 6, whose deliverable becomes
"un-ignore it and make it pass". Absence is then visible in the tree.

### Exit witnesses

**R15 — user-space `my-times` compiles, sums, and loops in constant stack.** It compiles from
user source **beside** the untouched intrinsic, a concrete call sums correctly, and it runs 1M
iterations at `ulimit -s 1024` to completion, exit 0, via `run_at_stack_limit`
(`tests/phase4_combinators.rs:1403`).

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

The two `i64` inputs are **from** and **to**, per brief recon 4 — not a typo for the single
count in `times`'s own signature. At the definition site the row grounds to the empty region
(R9 context 3), so `sig.inputs.len() == 3` (the row lives in `PolySig.row_in`), the back-edge
checks 3 arguments against 3 declared non-row inputs, and both `if` arms leave zero fixed slots.
Reading the bound `f` twice needs no `dup`.

**R16 — grounding semantics are pinned, including what they lose.** Grounding a row against a
concrete region is type-equality over that region, not a proof it was "restored unchanged": a
body that replaces a carried value with a different value of the same type satisfies it. And
`Slot::computed` (`:224-234`) drops `deriv`, `surviving` and `quot`, so **provenance is not
preserved across the row** — a borrow can be dropped and an unrelated borrow of the same
referent type substituted. Pin the borrow-substitution case with a golden, not merely a value
swap.

**R17 — aggregate aliasing witness.** An aggregate carried across the row with per-iteration
data dependence prints arithmetically correct fields, so the slice-3 aliasing class surfaces as
a wrong number rather than a crash.

**R18 — nesting parity.** `my-times` nested inside itself produces correct output.

**R19 — no regression, intrinsic untouched, library untouched.** `while` and the full existing
corpus are unchanged. `tests/qbe_baseline*` goldens hold **byte-identically against the base
commit this slice branches from**, named in the phase-1 report — not against whatever is in the
tree when phase 7 runs, since sibling sessions land baseline-rewriting commits. 10a changes no
index type and adds no `~` to any shipped signature, so any movement is a defect to investigate.
The intrinsic's arms still exist and still serve `times`. `lib/combinators.sth` is
**byte-unchanged and contains no `~`**.

**R20 — mutation-test every new guard.** For every located error this slice introduces — R1's
spaced-form `~ [` parse error, R2's materializing uses and capture admission, R3's mismatch in
both directions, R4's fresh/absent row, R5's one-sided and differing rows, R9's grounding
mismatch, R10's exact-text renderings, R12's back-edge argument mismatch — prove the test can
fail by deleting the guard it protects and confirming the golden flips. Phase 7's audit
enumerates them individually rather than referring to "phases 1–5". This project has shipped
placebo tests before and reading does not catch them.

## Traceability

| Req | Traces to | Phase |
| --- | --- | --- |
| R1 | `~` decision; D1 (representation), D2 (adjacency); review (routing ICEs, lexer shape) | 1 (variant, accessor, audit), 2 (token, parse, routing) |
| R2 | `~` decision; review (fifth boundary, fail-open fields) | 1 (dispositions), 2 (behavioural tests) |
| R3 | `~` decision | 2 |
| R4, R5 | brief decisions 1–2 | 3 |
| R6 | review (fold destroys the row) | 3 |
| R7, R8 | brief decisions 1–2; open q4 | 3 |
| R9 | brief recon 5 **corrected**; review (plumbing, callers, rendering) | 4 |
| R10 | review (renderer gap) | 1 (`Type::name`), 3 (row), 4 (grounding messages) |
| R11, R12 | brief decision 3; recon 4; review (marker unreachable, single source) | 5 |
| R13 | brief decision 4 | 5 |
| R14a | review (skip risk, `#[ignore]` compiles) | 5 |
| R14 | brief decision 5; recon 7; review (vacuity, masking, placebo, all-`None` map) | 6 |
| R15–R19 | brief exit criteria | 7 |
| R20 | project convention | each phase, audited in 7 |

## Phased delivery plan

**Phase 1 — the type variant and the audit.** *(R1 partial, R2 partial, R10 partial)* Add
`Type::InlineQuotation`, fill the three exhaustive arms, add `Type::name()`'s `~[ ... ]`
spelling, and add the `is_quotation_type` accessor. Then the **audit**: paste the grep output
and disposition every silent `Type::Quotation` site, extending the routing predicates, the
enabling sites, and the fail-open restrictions. Nothing here is user-visible, and everything is
testable by constructing `Type::InlineQuotation` directly in unit tests without a parser. This
is where a missed site fails **silently**, which is why it is separated from phase 2. **Hard.**

**Phase 2 — surface syntax and behaviour.** *(R1 rest, R2 rest, R3)* Add
`Token::TildeLBracket` in the lexer glue; extend `effect_has_variable`; recognise the token on
all four entry points with the inner-entry-point split; ground `~` in `apply_subst` and the
fold. Land the six behavioural tests (five boundaries rejecting, `call` accepted), the
materializing-declaration rejections, and R3's goldens in both directions. Everything here
fails **loudly** at the first test. Standard.

**Phase 3 — rows inside a quotation effect.** *(R4, R5, R6, R7, R8; R10's row half)* Grow
`PolyType::Quotation` with row fields; add the `..` branch to the nested slot parse; **suppress
the concrete fold whenever a row is set**, with a unit test asserting `~[ ..s i64 -- ..s ]`
survives as `PolyType::Quotation` with both row fields populated; reject a fresh name, a
one-sided row, and a differing output row at R5's exact text; exclude the row from the pairwise
arity; make the poly renderers row-aware. No grounding against a live stack yet. Standard.

**Phase 4 — row grounding at the check sites.** *(R9; R10's message half)* Leave `apply_subst`
alone; give `check_literal_against_declared_effect` the row region as type-only
`Slot::computed` copies; prepend to `fresh`, require back on the exit row, strip before
rendering. Cover all four contexts, including the definition-site empty region and context 4's
**routing** assertion. Do not generalise `check_abstract_quotation_times`. **Hard.**

**Phase 5 — back-edge ground declared outputs.** *(R11, R12, R13, R14a)* Return
`check_poly_combinator_args`'s `Subst`; extend `SelfTailMarker` with the ground outputs and the
bottom-aligned index map at its sole set site; rewrite `:8476-8480`; add the explicit unify of
`stack[base..]` with a located diagnostic; confirm the guards untouched; pin `while` unchanged.
Extract the `outs` construction into a named function and land phase 6's white-box test against
it, `#[ignore]`d. **Hard.**

**Phase 6 — the surviving-set gate.** *(R14)* Forward `surviving`/`quot` along the index map,
following `d1b3f0a`/`bee407c`; un-ignore the test and make it pass with an aggregate-carrying
witness whose shape yields at least one `Some(j)` entry; record the mutation evidence. Its own
commit and review, so it cannot be folded into phase 5 and shortchanged. **Hard.**

**Phase 7 — exit witnesses and mutation audit.** *(R15–R20)* The golden suite: the user-space
`my-times` sum and its 1M-iteration constant-stack run, the pinned grounding semantics including
borrow substitution, the aggregate aliasing witness, self-nesting, corpus/`while`/intrinsic/
library unchanged with byte-identical baselines against the named base commit, and an audit
enumerating every located error from phases 1–5 with its mutation evidence. Standard.

## Exit criteria

10a exits when: `Type::InlineQuotation` exists with every silent `Type::Quotation` site
dispositioned in writing against the pasted grep output (R1, R2); `~[` is a single token whose
spaced form is a parse error, recognised on all four entry points and by the routing scan, with
all five materialization boundaries rejecting a `~`, `call` still accepted, every materializing
declaration rejected, and R3 pinned in both directions (R1–R3); a row parses, represents,
renders, and **survives the concrete fold**, with all three rejections golden-tested at exact
text (R4–R8, R10); grounding works in all four contexts via the callee-side mechanism, type-only
and stripped before rendering (R9); the back-edge produces ground declared outputs along the
bottom-aligned index map with the self-call arguments explicitly checked and `while` unchanged
(R11–R13); the surviving-set forward is implemented **and** proven by a mutation-tested
white-box assertion over an aggregate-carrying slot with a non-`None` map entry (R14, R14a); the
user-space `my-times` compiles beside the untouched intrinsic, sums, runs 1M iterations in
constant stack, carries an aggregate without aliasing, and nests (R15–R19); `lib/combinators.sth`
and the QBE baselines are byte-unchanged against the named base commit (R19); and every new
guard has been shown capable of failing (R20).

10b and 10c are separate specs. 10c consumes phases 1–4; 10b consumes all of 10a.

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
