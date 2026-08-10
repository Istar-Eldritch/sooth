# Phase 4 Slice 10a spec: the inline-only quotation type, and rows in quotation effects

Derives from [`docs/phase4-slice10-brief.md`](./phase4-slice10-brief.md). The brief's
recon is treated as ground truth **except where this spec says otherwise** — review found
two of its claims false against the live tree, and both are corrected here rather than
inherited (see "Corrections to the brief").

`times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` is a compiler intrinsic only because user
source cannot *spell its signature*, not because it needs magic to run. Everything under
it is already general: blanket self-tail-call TCO, quotation parameters threaded as
compile-time constants, shared `Builder` loop state (`save_loop_state` `src/ir.rs:3016`,
`alloca_home` `:2718`). This slice makes the signature writable.

**Scope is 10a only.** 10b (deleting the intrinsic, moving `times` into
`lib/combinators.sth`, and deciding how much of the rest of the library retypes) gets its
own brief and spec; it is deliberately not in this document's phase plan, so migration risk
and mechanism risk do not share a review.

**Phase numbering note.** `~` was inserted as phase 1 in a later revision, shifting every
subsequent phase by one. 10c (`if`/`cond` as ordinary words,
`docs/phase4-slice10c-brief.md`) consumes **phases 1–3** of this spec — `~`, rows, and
grounding — and nothing beyond. It does not gate on phase 4's back-edge rewrite, phase 5,
or 10b. Any document still saying 10c gates on "phases 1–2" predates this renumbering.

## The target signature

```sooth
: times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )
```

Exactly today's intrinsic signature plus a spelled row and the `~` sigil. **The index type
does not change in this slice, and should not change in 10b either.**

An earlier revision of this spec moved the count and index to `usize`, on the strength of
the corpus round-trip: `len` already returns `usize` (`src/check.rs:5261`), so all four
count-taking library combinators convert it *down* with `len >i64` purely to satisfy
`times` and convert each index back *up* per iteration (twice in one body in
`examples/array_totals_hand.sth:20` and `examples/inplace_fold.sth:33`). That observation
is real, but `usize` is the wrong repair, for two reasons.

First, it only moves the cost. A `usize` index cannot be added to a computed `i64`
(`error:`+` mixes `usize` with a computed `i64`: convert it explicitly with`>usize`
first`), so `usize` deletes a conversion at every indexing site and *adds* one in every
body that accumulates the index into an `i64` — which is precisely R14's summing witness
and R16's aggregate witness. It is a swap of which half of the corpus pays, not a fix. (The
"same QBE register" argument the earlier revision leaned on is also the wrong rationale:
`IrType::Usize` normalizes to `Int { signed: false }` (`src/backend/qbe.rs:631-633`), and
signedness drives `cslt`/`cult`, `sar`/`shr`, and `.` printing via `$fmt`/`$ufmt`.)

Second, the honest fix is a bounded type variable — `times ( ..s 'T: Int ~[ ..s 'T -- ..s ] -- ..s )`
— so an indexing caller instantiates at `usize`, an arithmetic caller at `i64`, and neither
converts. That is the recorded end state, and it is **not available today**: `Bound` is
`{ Copy, Ord }` (`src/ast.rs:594`) with no `Int`, and arithmetic on a bound type variable
is unsupported. Isolated to one changed token:

```sooth
: add2 ( 'T: Copy Ord 'T -- 'T ) over over > if drop else swap drop end ;   \ compiles
: add2 ( 'T: Copy Ord 'T -- 'T ) + ;                                        \ fails
```

`<` works on a bound variable; `+` does not, and fails with a misleading diagnostic
(`` `+` needs 2 values, but the stack holds 0 `` plus `note: declared ( -- )`, neither of
which is true of the signature). So an `Int` bound needs a new `Bound` variant, arithmetic
operator support on bound type variables, per-instantiation monomorphization of the loop,
and a diagnostic fix. That is its own slice, sequenced after 10b, and migrating the index
to `usize` first would only have to be re-migrated to `'T: Int` afterwards.

**Consequence for this slice: 10a changes no index type anywhere.** The intrinsic keeps its
two hardcoded `Type::I64` sites (`src/check.rs:6853` count check, `:6855`
`eff.inputs.last()`) plus the Known-literal path's own count check (`:8237`), untouched.
R14's user-space witness is written in `i64` so it mirrors what the future `Int` slice will
migrate.

**One row, the same on both sides.** `times`'s body row is a fixed point, not a
transformation, and this is forced rather than chosen: the body's output *is* the next
iteration's input across the back-edge. N=0 leaves the region untouched, so the declared
output row must equal the input row; N≥2 feeds iteration 1's output into iteration 2, whose
declared input is the input row, so again they must be equal. Only N≤1 would admit a
differing output row, and N is a runtime count. Encoding the fixed point in the signature
makes violating it a signature mismatch at the declaration instead of a back-edge row
mismatch later. (This is exactly why 10c's `if` legitimately takes `~[ ..i -- ..o ]`: it
runs its branch once, so it is permanently in the N≤1 case. `while` and `fold` are the
symmetric case — `fold`'s accumulator rides *inside* `..s` at a fixed shape and type, which
is why accumulation needs no asymmetry.)

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
row-bearing quotation must never reach an erasure boundary" from a rule bolted onto four
call sites into a structural impossibility.

**Representation: the poly layer only. No new `Type` variant.** `~` is recorded on
`PolyType::Quotation` (`src/ast.rs:622`) and a `~` effect **never folds to
`Concrete(Type::Quotation)`** — the fold at `src/parser.rs:1362` is suppressed for it even
when the effect is fully concrete. `Type` therefore never grows a `~` case, and the three
`materialize_quotation_at_boundary` call sites plus the one `if`-join erasure site all key
on `Type::Quotation`, so none of them can fire on a `~` value. That is R2's "unreachable by
construction", obtained without touching a single one of the 62 `Type::Quotation` match
sites in `src/check.rs` or the 34 in `src/ir.rs`.

The alternative representations were both rejected: a flag *inside* `Type::Quotation` still
matches every `matches!(_, Type::Quotation(_))` guard, so the erasure sites would need the
bolted-on runtime check R2 forbids; a separate `Type` variant delivers the guarantee but
turns all 96 existing match sites into silent non-matches, where a guard stops firing with
no error and no failing test — the exact under-delivery shape this project has shipped
before.

The `~` parameter's stack slot follows the existing quotation-marker idiom (`Slot.quot`,
`src/check.rs:213-218`): the marker carries the identity of the literal body, and splicing
consumes it. This is how quotation parameters already work; `~` adds no new slot mechanism.

**Adjacency is required.** `~[` is a single lexer token. `~` is not currently a delimiter
(`is_delimiter`, `src/lexer.rs:25`), so without a token `~[` would arrive as `Word("~")`
followed by `LBracket`, and the lexer discards adjacency — making the spaced form
`~ [ ..s i64 -- ..s ]` indistinguishable from `~[ ..s i64 -- ..s ]`. A real token makes the
spaced form a parse error, which is the intended behaviour.

**10a applies `~` to `times`'s signature shape alone**, via the user-space witness. Whether
`each`/`map`/`fold`/`filter`/`while` retype — making `~` the explicit combinator/closure
boundary, with ordinary `[ ... ]` reserved for genuinely first-class capturing quotations
(7b's territory) — is 10b's question. `lib/combinators.sth` is byte-unchanged by this slice.

## Corrections to the brief

**1. `check_abstract_quotation_times` is not the prototype the brief claims.** The brief
(recon 5, echoed in the ROADMAP) says it implements "pop the declared fixed inputs above an
opaque row, require the row restored." It does not. It pops and type-checks the count
(rejecting a quotation count, `src/check.rs:6849-6851`, and requiring `Type::I64`, `:6853`),
then requires the *declared effect* be self-similar and pointwise-matches the declared
outputs against the top slots (`:6855-6872`):

```rust
let row_preserving = eff.inputs.last() == Some(&Type::I64)
    && eff.inputs.len() == eff.outputs.len() + 1
    && eff.inputs[..eff.outputs.len()] == eff.outputs[..];
let base = stack.len() - row_len;          // row_len = eff.outputs.len()
for (i, want) in eff.outputs.iter().enumerate() { match_slot(stack[base + i], *want) ... }
```

There is no fixed-inputs-above-a-row decomposition and the row is never inspected —
everything below the matched slots is untouched by construction. So phase 3 **derives** the
grounding rather than generalising a prototype, and **R8** says how. The count handling is
called out because it, not the row logic, is what a future retype has to rewrite.

**2. "Nothing in the checker reads `PolySig.row_in`/`row_out`" is stale.** `poly_sig_shape_eq`
reads both (`src/check.rs:3188`/`:3191`) and `poly_sig_str`'s `render_row` prints them
(`:3231-3232`); the repl sites are `src/repl.rs:2407` and `:3010`/`:3015`. This matters for
**R6**: because `poly_sig_shape_eq` drives overload dedup, growing `PolyType::Quotation`
with row fields makes two candidates differing only by row distinguishable for free.

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
| Lexer delimiters | `is_delimiter` `src/lexer.rs:25` | `~` is not one; R1 adds the `~[` token here. |
| Poly quotation parse | `parse_poly_quotation` `src/parser.rs:1251` (opens with `expect(LBracket)` `:1252`); called from `:1253`/`:1255`; `RawTy::Quotation` declared `:641`, constructed `:1257` | Serves polymorphic signatures. |
| **Concrete** quotation parse | `parse_quotation_type_expr` `src/parser.rs:1451` | The **second** parse path: mono signatures, struct fields, ref/cell referents, externs. R1 covers both. |
| Nested slot parse | `parse_poly_slot` `src/parser.rs:1208` | Where the `~` sigil is dispatched, and where the row `..` branch is added (R4). |
| Top-level row parse | `parse_poly_slots` `src/parser.rs:1178`; `PolyBuilder` `:662` (`row_in` `:669`, `row_out` `:670`); `set_row` `:676`; `row_var_misplaced_error` `:747`, fired `:1197` | A later `..` in a slot list is already a located error. |
| Raw → poly fold | `raw_to_poly_type` `src/parser.rs:1346`, Quotation arm `:1362`; `RawTy` `:634` | A fully-concrete effect folds to `Concrete(Type::Quotation)`. **Suppressed for `~`** (R1). |
| Poly representation | `PolyType::Quotation` `src/ast.rs:622`; `PolySig` `:629` (`row_in` `:631`, `row_out` `:636`, `row_var_names` `:640`); `QuotEffect` `:889`; `Bound` `:594` | Where both `~` and the row live. `QuotEffect` needs no row field (recon 5). |
| Pointwise unify | `unify_poly_input` `src/check.rs:5845`, Quotation arm `:5912`, arity check `:5922` | Row must be excluded from the pairwise arity. |
| Grounding a declared effect | `apply_subst` `src/check.rs:5963`, Quotation arm `:5989`; `quotation_type` `src/ast.rs:895` | Returns an **interned** `&'static QuotEffect`. Not the place to splice a caller region (R8). |
| Splice-site checks | `check_poly_combinator_args` `src/check.rs:7226` (`n` `:7240`, `base` `:7244`, Pass 2 `:7262-7289`, `apply_subst` `:7267`, literal check `:7272`, abstract arm `matches!` `:7275`, comparison `:7280`); `check_literal_against_declared_effect` `:7301` (`fresh` sub-stack `:7331`, exit row `:7367`) | The caller holds `stack` and `base`; the callee does not. R8's plumbing point is the **callee**. |
| Mono declared-parameter path | `src/check.rs:7102-7118` (`:7112` the non-poly caller) | Handles quotation parameters without going through `check_poly_combinator_args`. R8 context 4. |
| Combinator dispatch | `check_term` `src/check.rs:8000`; `is_combinator` `:6921`; `has_self_tail_call` `:3943`; `check_combinator_cycles` `:6966`; `inline_combinator` `:7076` | Combinators mint no `IrFunc`; spliced per call site, so a row is concrete at every splice. |
| Self-tail back-edge | `SelfTailMarker` `src/check.rs:648`; set `:7178`; matched `:8452-8456`; arm `:8441-8484`; **`outs` construction `:8476-8480`** | Phase 4 rewrites `:8476-8480`. |
| Back-edge guards | `check_linear_across_back_edge` `src/check.rs:6762` (invoked `:8469`); `check_reference_across_back_edge` `:6741` (invoked `:8471`) | Run unchanged (**R12**). |
| Erasure boundaries (4) | `materialize_quotation_at_boundary` def `src/check.rs:7644`, called at `:4492` (word output), `:8357` (`!`/`+!` store through a ref), `:8554` (declared parameter); `if`-join erasure `:8797-8806` | The complete audit list for R2. All four key on `Type::Quotation`. |
| Surviving set | `Slot` `src/check.rs:194`, `quot` `:213-218`, `surviving` `:219-224`; `Slot::computed` `:224-234` (sets `quot: None, surviving: None`); `union_surviving` `:844` (called from the join `:8726`); `intern_surviving_set` `:826` | Phase 5's subject. Forwarding pattern to follow: `d1b3f0a`, `bee407c`. |
| Signature renderers | `poly_type_str` `src/check.rs:6284`, Quotation arm `:6295-6309`; `poly_quotation_concrete_hint` `:5946`, called `:5927`; `poly_sig_str`'s `render_row` `:3223`, calls `:3231-3232`; `poly_sig_shape_eq` `:3188`/`:3191` | **R9**: neither quotation renderer knows about rows, so a row would silently vanish from every new diagnostic. |
| Intrinsic (untouched in 10a) | `check_term` `"times"` arm `src/check.rs:8208`, count check `:8237`, abstract path `:8221`; `check_abstract_quotation_times` `:6840`; `ir.rs` `"times"` lowering `:3441` | 10b deletes these. `ir.rs:5804` is **not** a registration — it is `b.lower_call("times", …)` inside the test `times_saves_and_restores_loop_state` (`:5761`); `times` is absent from `BUILTIN_WORDS` (`check.rs:2371-2401`) and intercepted by literal name. |
| Constant-stack witness | `run_at_stack_limit` `tests/phase4_combinators.rs:1403`; e.g. `three_deep_times_nesting_runs_in_constant_stack` `:1127` | Reuse for R14. |

Landed prerequisites, for the record: 7b merged `3776579`, 8a merged `e20c52f`, slice 9
merged `c5db035`. (A pre-revision draft cited `5f645f0` — a 7b *doc* commit — as evidence 8a
had landed; that was wrong.)

## Requirements

"Located" means the diagnostic carries a span and names the offending row/argument and the
declared signature, per the project's diagnostics-are-behaviour convention.

### The `~` type

**R1 — `~[` is a single token, recognised on both parse paths, never folds to a concrete
`Type`, and forces its word poly.** A signature mentioning `~` sets
`WordDef.poly = Some(..)` and leaves `WordDef.effect` empty, exactly as a signature
mentioning a row variable already does (`src/ast.rs:565-576`) and for exactly the same
reason: neither a row nor a `~` can be forced into a concrete `Type` slot. This is what
makes the rest of R1 consistent — a `~` never needs a `Type` because a `~`-bearing word
never takes a concrete path.

*Detail:* Add `~[` to the lexer (`src/lexer.rs:25`) so adjacency is required and
`~ [` is a parse error. Recognise it in both `parse_poly_slot` (`src/parser.rs:1208`,
dispatching to `parse_poly_quotation`) **and** `parse_quotation_type_expr` (`:1451`, the
concrete path serving mono signatures, struct fields, ref/cell referents, externs). Record
`~` on `PolyType::Quotation` and **suppress the fold to `Concrete(Type::Quotation)`** at
`src/parser.rs:1362` for a `~` effect, including a fully concrete one. No new `Type`
variant; no change to any existing `Type::Quotation` match site.

**R2 — `~` bans materialization, not invocation.** `call` on a `~`-typed value is accepted
and is statically always a splice. The four erasure boundaries — `materialize_quotation_at_boundary`
at `src/check.rs:4492` (word output), `:8357` (`!`/`+!` store through a ref), `:8554`
(declared parameter), and the `if`-join erasure at `:8797-8806` — are unreachable for a `~`
value **because R1 keeps it out of `Type`**, and all four key on `Type::Quotation`. The
guarantee rests on an existing invariant, not a new one: a `~`-bearing word is poly, and a
poly word's concrete paths are already skipped (`src/ast.rs:565-576`). Phase 1
delivers one test per boundary demonstrating a `~` value cannot reach it. Every remaining
materializing use — declaring a `~` as a word output, a struct/array/cell field, or an
`extern` parameter — is a **located error** naming the type and the position. No runtime
check is added at any of the four boundaries; if the implementer finds one is genuinely
required, that is a spec defect to escalate, not to absorb.

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
error**. For 10a the row is the *same* row on both sides. A differing output row is a
located error whose text is exactly:

```
error: a loop body cannot change the shape of the carried region: `..a` in, `..b` out
note: 10c lifts this for a word without a back-edge
```

The wording is fixed here so the requirement is objectively judgeable and so the message
does not claim the shape is illegal in general.

**R6 — representation mirrors `PolySig`.** `PolyType::Quotation` (`src/ast.rs:622`) grows
optional row fields in the signature's existing row id space, mirroring
`PolySig.row_in`/`row_out` (`:629-640`). `QuotEffect` (`src/ast.rs:889`) needs **no** row
field: at every splice the row is concrete. Because `poly_sig_shape_eq`
(`src/check.rs:3188`/`:3191`) already reads the `PolySig` row fields, overload dedup
distinguishes candidates differing only by row for free once the representation lands.

**R7 — the pointwise unify walk excludes the row.** `unify_poly_input`'s Quotation arm
(`src/check.rs:5912`, arity check `:5922`) matches only the fixed, non-row slots pairwise.
The row contributes no pairwise slot, binds no type/len variable, and is excluded from the
equal-arity check.

**R8 — row grounding, in the callee, in four contexts.** A row-bearing declared quotation
parameter grounds to the concrete caller-stack region below its fixed inputs.

*Mechanism, settled here rather than deferred.* `apply_subst` (`src/check.rs:5963`) returns
an interned `&'static QuotEffect` via `quotation_type` (`src/ast.rs:895`); splicing a caller
region into it would mint an effect no literal and no forwarded parameter can equal,
breaking the abstract comparison at `:7280` and printing the caller's stack inside declared
types in every diagnostic. So **`apply_subst` is left alone.** Instead
`check_literal_against_declared_effect` (`:7301`) takes the row region as a new parameter,
prepends it to the `fresh` sub-stack it builds (`:7331`), and requires it back on the exit
row (`:7367`). The caller already holds it: `check_poly_combinator_args` computes
`base` at `:7244`, so the region is `stack[..base]`.

The four contexts, all covered explicitly:

1. **Known-literal splice** — the path above.
2. **Abstract pass-down** — the `matches!(found.ty, Type::Quotation(_))` arm at `:7275`: a
   quotation parameter forwarded by the combinator whose body is being checked. Grounding
   here is the declared-effect trust already used for type variables, extended to carry the
   row. The comparison at `:7280` must still work, which is why the interned effect is left
   untouched.
3. **Definition-site, no caller** — a combinator's body checked standalone at its own
   definition, where the row grounds to the **empty** region. This is why
   `: passthru ( ..s i64 -- ..s ) drop ;` compiles today and `shrinks-row` does not, and it
   is the context 10a's own exit repro fires in first.
4. **Mono declared parameter** — `src/check.rs:7102-7118`, which handles a quotation
   parameter for a non-poly word without going through `check_poly_combinator_args`. This
   context is **unreachable for a `~`**, and by an existing rule rather than a new one: a
   signature mentioning `~` sets `WordDef.poly = Some(..)` (R1), and for such a word
   `WordDef.effect` is left empty while "every concrete path (env registration, monomorphic
   body checking, bundle interning) skips such a word" (`src/ast.rs:565-576`). Phase 3 pins
   this with a test asserting a `~`-bearing mono-looking signature is routed poly and never
   reaches `:7112`, so the unreachability is checked rather than assumed.

No abstract row unification, no `Subst` extension (it stays `ty`+`len`), no mangling impact.

**R9 — new rejections render the row and the sigil.** Neither `poly_type_str`'s Quotation
arm (`src/check.rs:6295-6309`) nor `poly_quotation_concrete_hint` (`:5946`) knows about
rows, so once `PolyType::Quotation` carries them, `[ ..s i64 -- ..s ]` would print as
`[ i64 -- i64 ]` in every R4/R5/R8 diagnostic and a one-sided-row error would show two
identical-looking effects. Both renderers become row-aware and print `~`. Pinned by
asserting **exact** diagnostic text, never a substring that survives the row vanishing.

### The self-tail back-edge

**R10 — the back-edge arm produces the ground declared outputs, along an explicit index
map.** Rewrite `src/check.rs:8476-8480` so the arm's result is the combinator's ground
declared outputs (via `apply_subst`), not its non-quotation inputs. The current comment's
claim — "the non-quotation inputs, which for a self-tail combinator are exactly its declared
outputs" — holds only for the state-threading shape (`while`) and is false for a loop that
consumes its counters, which is why the recon-4 `my-times` fails today with a spurious
`` `if` branches leave different stack depths (then: 3, else: 1) ``.

Today the positional correspondence "i-th declared output ↔ i-th non-quotation input" is
*implicit* in building `outs` from `stack[base..]`. The rewrite destroys it, and R13 needs
it. So the rewrite **builds an explicit source-index map**: walk the declared outputs
against the ordered non-quotation declared inputs, carrying each output's source index (or
`None` where it has no input counterpart). R13 forwards along that map. Without it an
implementer must re-invent a convention, and a wrong guess forwards *another slot's* capture
set, which is worse than dropping it.

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

This requirement is deliberately **not** satisfiable by prose, and the obvious readings of
it are all vacuous or masked:

- *Vacuous:* `times`'s declared outputs are the row alone, so it has **zero** fixed output
  positions, and any rule phrased as "forward where a positional source exists, document
  where it doesn't" auto-satisfies.
- *Masked:* the natural end-to-end witness (`while` carrying an aggregate that holds a
  closure) exits through a conditional join, and `union_surviving` (`:844`, called at
  `:8726`) reconstructs the dropped set from the sibling arm, so a no-op implementation
  passes.
- *Placebo:* the old filter at `:8476-8479` already excludes **bare** quotation slots, so a
  white-box test whose input slot is a bare `Type::Quotation` proves nothing.

Therefore, all four of the following, or the requirement is not met:

1. **Extract the `outs` construction into a named, callable function.** It is currently
   inline in `check_term`, so there is no addressable target to assert on before the join.
2. **The witness slot is an aggregate carrying an erased quotation** — `ty` a struct,
   `surviving: Some(..)`, `quot: None` — not a bare quotation.
3. **A white-box unit test asserts the forwarded `SurvivingCaptureSetId` on the produced
   `outs` slots directly, before any join runs**, bypassing `union_surviving` entirely.
4. **The test is mutation-tested**: reverting the forward (restoring `Slot::computed`) must
   make it fail, and the phase report records that evidence.

The phase does not exit on "no witness exists, so the risk stays documented." If that
genuinely turns out to be the case, it is escalated, not absorbed.

**R13a — phase 4 lands R13's test `#[ignore]`d.** Phase 4 lands green by construction: with
the masking above, a missing forward is undetectable end to end, so CI cannot notice phase 5
being skipped and only exit-criteria prose would guard it. Phase 4 therefore lands the
phase-5 white-box test itself, `#[ignore]`d with a reason naming phase 5. Phase 5's
deliverable becomes "un-ignore it and make it pass", and its absence is visible in the tree.

### Exit witnesses

**R14 — user-space `my-times` compiles, sums, and loops in constant stack.** With its row
restored, `my-times` compiles from user source **beside** the untouched intrinsic, a
concrete call sums correctly, and it runs 1M iterations at `ulimit -s 1024` to completion,
exit 0, via `run_at_stack_limit` (`tests/phase4_combinators.rs:1403`). The program, written
out here rather than deferred:

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
count in `times`'s own signature. The non-obvious part worth stating: at the definition site
the row grounds to the empty region (R8 context 3), so the back-edge checks 3 arguments
against 3 declared non-row inputs, and both `if` arms leave zero fixed slots.

**R15 — grounding semantics are pinned, including what they lose.** Grounding a row against
a concrete region is type-equality over that region, not a proof the region was "restored
unchanged": a body that replaces a carried value with a different value of the same type
satisfies it. Worse, `Slot::computed` (`src/check.rs:224-234`) drops `deriv`, `surviving`
and `quot`, so **provenance is not preserved across the row** — a borrow in the row region
can be dropped and an unrelated borrow of the same referent type substituted. State both
explicitly and pin the borrow-substitution case with a golden, not merely a value swap.

**R16 — aggregate aliasing witness.** An aggregate carried across the row with
per-iteration data dependence prints arithmetically correct fields, so the slice-3 aliasing
class (a stale value from iteration *k* visible at *k+1*) surfaces as a wrong number rather
than a crash.

**R17 — nesting parity.** `my-times` nested inside itself produces correct output.

**R18 — no regression, intrinsic untouched, library untouched.** `while` and the full
existing corpus are unchanged; `tests/qbe_baseline*` goldens hold **byte-identically** —
10a changes no index type and adds no `~` to any shipped signature, so any baseline movement
is a defect to investigate, never to accept. The intrinsic's arms in `check.rs`/`ir.rs`
still exist and still serve `times`. `lib/combinators.sth` is **byte-unchanged and contains
no `~`** (this is approved constraint A2 made checkable rather than left as intent).

**R19 — mutation-test every new guard.** For each located error introduced by this slice —
R2's materializing uses, R3's mismatch, R4's fresh/absent row, R5's one-sided and differing
rows, R8's grounding mismatch (phase 3), R11's back-edge argument mismatch — prove the test
can fail by deleting the guard it protects and confirming the golden flips. R9's exact-text
assertions are included: a substring assertion that survives the row vanishing is precisely
the placebo class this convention exists for. Phase 6's audit enumerates them individually
rather than referring to "phases 1–4". This project has shipped placebo tests before and
reading does not catch them.

## Traceability

| Req | Traces to | Phase |
| --- | --- | --- |
| R1, R2, R3 | `~` decision; D1 (representation), D2 (adjacency) | 1 |
| R4, R5, R6, R7 | brief decisions 1–2; open q4 | 2 |
| R8 | brief recon 5 **corrected**; open q1; review B3 | 3 |
| R9 | review finding (renderer gap) | 2 (row), 3 (grounding messages) |
| R10, R11 | brief decision 3; recon 4; review B2 | 4 |
| R12 | brief decision 4 | 4 |
| R13, R13a | brief decision 5; recon 7; review (vacuity, masking, placebo, skip-risk) | 5 (R13a lands in 4) |
| R14–R18 | brief exit criteria | 6 |
| R19 | project convention | each phase, audited in 6 |

## Phased delivery plan

**Phase 1 — the inline-only quotation type.** *(R1, R2, R3)* Add the `~[` lexer token;
recognise it on both parse paths; record `~` on `PolyType::Quotation`; suppress the concrete
fold; reject every materializing declaration with a located error; land one test per erasure
boundary showing a `~` value cannot reach it. First, because the target signature cannot be
spelled until `~` parses. Unit tests beside the parser/type changes: the token round-trips,
the spaced form `~ [` is a parse error, `call` on a `~` value is accepted, each materializing
use is rejected, and no implicit conversion exists in either direction. Standard difficulty:
the representation decision that would have made this wide is settled (poly layer only), so
no existing `Type::Quotation` match site is touched.

**Phase 2 — rows inside a quotation effect.** *(R4, R5, R6, R7; R9's row half)* Grow
`PolyType::Quotation` with row fields mirroring `PolySig`; add the `..` branch to the nested
slot parse, tied to the signature's recorded top-level row; reject a fresh name, a one-sided
row, and a differing output row with the exact text R5 fixes; exclude the row from the
pointwise unify arity; make both signature renderers row-and-`~`-aware. No grounding against
a live stack yet. Unit tests: the three rejections with exact expected text, plus a
happy-path parse of `~[ ..s i64 -- ..s ]` reaching the checker. Standard.

**Phase 3 — row grounding at the check sites.** *(R8; R9's message half)* Implement the
settled mechanism: leave `apply_subst` alone, give `check_literal_against_declared_effect`
the row region, prepend it to the `fresh` sub-stack, require it back on the exit row. Cover
all four contexts, including definition-site-with-no-caller and the mono declared-parameter
path. Do **not** generalise `check_abstract_quotation_times`; it does not do what the brief
said. Unit tests: a row-bearing literal accepted against a matching concrete region and
rejected against a mismatching one (mutation-tested per R19), the abstract pass-down shape
with the `:7280` comparison still intact, a definition-site check with an empty row region,
and the mono path. **Hard**: the type-checking core, and the requirement whose premise review
falsified.

**Phase 4 — back-edge ground declared outputs.** *(R10, R11, R12, R13a)* Rewrite
`src/check.rs:8476-8480` to the ground declared outputs, building the explicit source-index
map R13 needs; add the explicit unify of `stack[base..]` against the ground declared inputs
with a located diagnostic; confirm the two back-edge guards are untouched; pin `while`
unchanged; land phase 5's white-box test `#[ignore]`d. Unit tests: `while` byte-identical; a
`times`-shaped self-tail combinator that type-checks where it did not before; the
argument-mismatch rejection. **Hard**: rewriting the exact block 7b's review flagged.

**Phase 5 — the surviving-set gate.** *(R13)* Extract the `outs` construction into a
callable function; forward `surviving`/`quot` along phase 4's index map, following
`d1b3f0a`/`bee407c`; un-ignore phase 4's test and make it pass with an aggregate-carrying
witness slot; record the mutation evidence that it fails when the forward is reverted. A
separate phase, with its own commit and its own review, precisely so it cannot be quietly
folded into phase 4 and shortchanged. **Hard**: a correctness obligation whose obvious
reading is vacuous and whose natural witness is masked.

**Phase 6 — exit witnesses and mutation audit.** *(R14–R19)* The 10a golden suite: the
user-space `my-times` sum and its 1M-iteration constant-stack run, the pinned grounding
semantics including borrow substitution, the aggregate aliasing witness, self-nesting,
corpus/`while`/intrinsic/library unchanged with byte-identical baselines, and an audit
enumerating every located error from phases 1–4 with its mutation evidence. Standard: no new
mechanism, but the witnesses that decide whether the slice is real.

## Exit criteria

10a exits when: `~[` is a single token whose spaced form is a parse error, recognised on
both parse paths, never folding into `Type`, with all four erasure boundaries shown
unreachable and every materializing declaration rejected (R1–R3); a row parses, represents,
renders, and unifies correctly inside a quotation effect, with all three rejections
golden-tested at exact text (R4–R7, R9); grounding works in all four contexts via the
settled callee-side mechanism (R8); the back-edge produces ground declared outputs along an
explicit index map, with the self-call arguments explicitly checked and `while` unchanged
(R10–R12); the surviving-set forward is implemented **and** proven by a mutation-tested
white-box assertion over an aggregate-carrying slot (R13, R13a); the user-space `my-times`
compiles beside the untouched intrinsic, sums correctly, runs 1M iterations in constant
stack, carries an aggregate without aliasing, and nests (R14–R18); `lib/combinators.sth` and
the QBE baselines are byte-unchanged (R18); and every new guard has been shown capable of
failing (R19).

10b and 10c are separate specs. 10c consumes phases 1–3; 10b consumes all of 10a.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "inline only quotation type", "difficulty": "standard" },
    { "phase": 2, "focus": "rows in quotation effects", "difficulty": "standard" },
    { "phase": 3, "focus": "row grounding at check sites", "difficulty": "hard" },
    { "phase": 4, "focus": "back edge ground declared outputs", "difficulty": "hard" },
    { "phase": 5, "focus": "surviving set forwarding gate", "difficulty": "hard" },
    { "phase": 6, "focus": "exit witnesses and mutation audit", "difficulty": "standard" }
  ]
}
```
