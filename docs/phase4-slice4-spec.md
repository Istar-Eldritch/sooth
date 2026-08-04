# Phase 4 Slice 4: quotations + the internal loop primitive

Base: `main` @ `0f88ccb`. Scoped by `docs/phase4-slice4-brief.md`; its "Decisions the
spec has to make" are settled constraints here, not options. Slice 1 gave `Sig`
type/row/length variables and native monomorphization, slice 2 carried them to the REPL,
slice 3 fixed the loop-carried aggregate copy. This slice adds the one piece of the
iteration story that cannot be a library (DESIGN.md:277): a quotation literal `[ ... ]`,
`call` to invoke it, and the internal loop primitive a quotation compiles to for
constant-stack iteration, exposed through the single compiler-known intrinsic `times`.

**Central constraint, from which everything follows.** There is no type, at any layer,
that can name a quotation (recon 2): `Type` (`src/ast.rs:566`), `PolyType`
(`src/ast.rs:406`), `Subst` (`src/ast.rs:443`), and `IrType` (`src/ir.rs:76`) have no
code-value variant, and adding one is a slice-1-sized representation change that only
non-inlined and escaping quotations need, both out of scope (Phase 6). So this slice makes
a quotation a **compile-time-only marker** that carries its body, is consumed by `call` or
by `times` via fusion, and never becomes a runtime value. The `Type`/`PolyType`/`IrType`/
unification/mangling change is deferred to slice 6, where a consumer for it finally exists.

## Locked decisions

- **D1: compile-time marker, no runtime type.** A quotation is a compile-time stack entry
  carrying the identity of its literal body, consumed by `call` or `times` through splicing,
  never lowered to a runtime code value. This defers the entire
  `Type`/`PolyType`/`IrType`/unification/mangling change (recon 2) to slice 6 alongside the
  escaping-quotation fallback that actually needs it. Taking the runtime type by reflex
  reopens the slice-1 representation at its most invasive for capability this slice cannot
  exercise (recon 9); the marker is not shaped to pre-empt that type, it is kept minimal.

- **D2: the marker rides the existing stacks as a phantom entry, moved verbatim.** On the
  checker side a quotation is an ordinary `Slot` (`src/check.rs:64`) distinguished by a new
  side-channel `quot: Option<QuotRef>` field, parallel to the existing `alias`/`deriv`
  discriminators, with a placeholder `ty` no user op accepts. On the lowering side it is a
  phantom `Value` id pushed with **no defining instruction**, recorded in a
  `quot_bodies: HashMap<Value, QuotId>` side map on `FuncBuilder`. Because `Slot` is `Copy`
  and a shuffle/bind moves a `Slot` verbatim (`src/check.rs:64` doc), and because
  `lower_call`'s shuffle arms (`src/ir.rs:2505`+) and `lower_term`'s `Bind` arm (`:2434`) move `Value` ids verbatim,
  the marker is forwarded through `dup`/`swap`/`over`/`rot`/bind/local-read for free on both
  sides, which is exactly the brief's "forwarded through binds/shuffles without merging"
  with no new mechanism. `Slot` stays `Copy` (a `QuotRef` is a `Copy` index).

- **D3: a quotation carries a body, not a pre-computed effect; its effect is realized at
  the consumption site by splicing.** `[ + ] call` type-checks *identically* to writing `+`
  at that point: the checker runs the body's terms against the live stack at the `call`.
  This sharpens the brief's "carries its inferred effect": there is no standalone effect to
  infer (a bare `[ + ]` would underflow an empty stack), and fusion at the consumption site
  is both the checking rule and the lowering rule. `times` realizes the body against the row
  plus a synthesized index and requires the result to equal the row (D6).

- **D4: `call` accepts only a statically-known literal.** With D1/D2, `call` type-checks
  only when the quotation on top of the stack is traceable to a single literal (directly, or
  forwarded through binds/shuffles). A quotation whose identity is lost at a branch merge, or
  that would have to be a runtime value (an array element, a non-inlined word parameter), is
  a **located rejection**, not a panic. These are the exact positions slice 6 later enables,
  so each gets its own diagnostic (R7–R10); diagnostics are behaviour here.

- **D5: the only inlining this slice owns is quotation-literal fusion.** Splicing a
  literal's body at its `call`, or at `times`, is a term-level local fusion in lowering,
  parallel to how builtins and generated struct/enum words already lower as `lower_call`
  arms that never emit `Instr::Call`. It **never crosses a `:` word boundary**; the
  interprocedural user-word inliner is slice 5's (recon 8).

- **D6: the floor is one intrinsic, `times`, passing the index; the IR back-edge machinery
  is reused unchanged.** `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` drives
  `begin_loop`/`finalize_loop` (`src/ir.rs:2301`/`:2348`) plus slice 3's carried-slot
  staging from a quotation loop rather than a syntactic self-`Call` (recon 5). The body
  quotation takes the iteration index and returns the same row it received, so effect
  realization only ever checks an inner row against itself. `while` was weighed as a second
  floor member (DESIGN.md:285 allows "one or two") and **declined** here: its condition
  quotation returns a `bool` on a passthrough row, strictly harder than `times` needs. The
  floor is permanent, not a bootstrap (DESIGN.md:281-289): slice 5 builds its library on
  `times`, it does not retire it.

- **D7: `if` is unchanged, and the polymorphic-path gaps are not this slice's.** `if` stays
  a keyword and stays rejected in a polymorphic body (`src/check.rs:2997`); this slice does
  not lift it to `PolyType` (slice 9). A polymorphic self-tail word still does not get the
  loop transform (`src/ir.rs:1176`). Both gaps land in slice 5 against their first real
  consumers (`filter`, `while`); neither blocks this slice, whose `times` witness is
  monomorphic.

## Requirements by stage

Diagnostics `Rn` marked *(located)* are behavioural negative tests asserting the message
**and** the named identifiers/positions, per the diagnostics-are-behaviour convention.

### Surface syntax / parsing (`src/lexer.rs`, `src/parser.rs`, `src/ast.rs`)

- **R1.** `TermKind` (`src/ast.rs:787`) gains `Quotation(Vec<Term>)`: an ordered term list
  parsed from between a term-level `[` and its matching `]`. Nesting is by construction
  (`[ 1 [ 2 + ] ... ]`), since the element list is `parse_terms`. No new token: `[`/`]`
  already lex (`src/lexer.rs:14-15,87-88`).

- **R2.** `parse_term` (`src/parser.rs:1463`) gains an arm for `Token::LBracket` that
  consumes the bracket, calls `parse_terms("`]` (unterminated quotation)", |t| matches
  `RBracket`)`, expects `]`, and yields `TermKind::Quotation`. The term-level `[` is
  **unambiguous** against the type-level `[`: every existing `[` reader
  (`parse_type_expr` `:1064`, `parse_array_type_expr` `:1159`, `parse_poly_slot` `:883`,
  `parse_field_type_expr` `:1232`) is reached only from signature/type parsing, never from
  `parse_term`, so no disambiguation logic is added; the two grammars simply never overlap.
  Today a `[` in a word body is a hard parse error (`unexpected token LBracket`, the
  `other =>` arm at `:1536`); R2 replaces that arm's reach for `[` only.

- **R3.** An unterminated quotation (`[` with no matching `]` before end-of-word or EOF) is
  a located parse error naming the unterminated quotation *(located)*, reusing
  `parse_terms`' EOF path; a stray `]` with no opening `[` is a located parse error parallel
  to the existing stray-`end`/`else` arm (`:1528`) *(located)*.

### Representation / checking (`src/check.rs`, `src/ast.rs`)

- **R4.** `Slot` (`src/check.rs:64`) gains `quot: Option<QuotRef>`, defaulted `None` in
  `Slot::computed` and every existing constructor (addition-only, R16).
  `QuotRef = Known(QuotId) | Merged(Span)`: `Known` indexes a per-check side table
  `Vec<QuotBody>` where `QuotBody` holds the literal's body terms (or a `Span` handle into
  the AST) and the literal's `Span`; `Merged(span)` is a poisoned quotation whose identity
  was lost at the branch join at `span` (R7). A quotation `Slot` carries a placeholder `ty`
  that no type-directed op accepts (R11).

- **R5.** `check_term`'s `TermKind::Quotation` arm (`src/check.rs:4248`, and the mirror
  `poly_term` arm, `:2990`) interns the body into the side table and pushes a quotation
  `Slot` (`quot: Some(Known(id))`). It does **not** check the body here (D3): a bare body's
  input row is unknown until its consumption site. In a polymorphic body, a quotation is
  pushed and rejected only if it reaches a `call`/`times` there (out of scope this slice, so
  a located "quotation combinators in a polymorphic body are not yet supported" is
  acceptable), but the monomorphic `times` witness never enters `poly_term`, so the poly
  arm only needs to not panic and to keep the exhaustive match compiling.

- **R6: `call` (`src/check.rs` builtin dispatch, `src/ir.rs` `lower_call`).** `call` is a
  new compiler-known word (grep confirms it is absent today), intercepted in `check_term`'s
  Call dispatch before user-word lookup. It requires a quotation `Slot` on top with
  `quot: Some(Known(id))`; it pops it and **splices** the interned body against the live
  stack via the ordinary term checker (`check_terms`, `:4215`), so `[ 1 + ] call` checks as
  `1 +`. The body sees the current locals/scope in lexical extent (capture is free by
  construction, recon 9). No standalone signature; net effect is whatever the splice yields.

- **R7: merged quotation is a located rejection** *(located)*. A monomorphic `if` (which
  is legal, unlike the polymorphic one) whose two arms each leave a quotation in the same
  stack position merges them at the join (`check_term`'s `If` arm). When the two slots are
  quotations with **different** `Known` ids, the join produces `quot: Some(Merged(join_span))`.
  `call` (R6) and `times` (D6) on a `Merged` quotation reject with a located error naming the
  join line: e.g. `` error: `call` needs a quotation whose body is known here, but this one
  was merged from two branches at line N (higher-order values are Phase 6) ``. Two arms
  carrying the **same** `Known` id (one literal bound before the `if`, read in both arms) do
  not poison.

- **R8: array element is a located rejection** *(located)*. Storing a quotation into an
  array (through `fill` or any array-construction path) rejects, because it would have to
  become a runtime value: `` error: a quotation cannot be stored in an array (escaping
  quotations are Phase 6) ``. Anchored at the array-store checking site.

- **R9: non-inlined word parameter is a located rejection** *(located)*. Passing a
  quotation as an argument to a user `:` word (or a polymorphic `'T`/row slot) rejects,
  since only `call`/`times` accept a quotation this slice: `` error: a quotation cannot be
  passed to `{word}`; only `call` and `times` accept one (higher-order user words are
  Phase 6) ``. This fires at the word-call argument-binding site before ordinary
  unification, so the message is specific, not a generic type mismatch.

- **R10: a quotation left on the stack at a word's exit is a stack-effect error.** A word
  cannot declare a quotation output (no quotation type exists), so an unconsumed quotation
  surfaces as the ordinary "declared vs actual outputs" mismatch at `check_outputs`; no new
  diagnostic, but the spec pins it (R10 golden) so a future reader sees it is intended, not
  a hole.

- **R11: every other consumer rejects a quotation operand** *(located, one helper)*. A
  single guard `reject_quotation_operand(ctx, span, op)` is invoked wherever a type-directed
  op would consume a slot with `quot.is_some()` and the op is not `call`/`times`: the
  operators and print (`check_operator`, `:4612`), conversions, the `if` **condition** pop
  (a quotation is not a `bool`), and the back-edge. One located wording naming the op. This
  is the choke that keeps the placeholder `ty` (R4) from ever being silently accepted.
  Shuffles (`dup`/`swap`/`over`/`rot`) and `drop` are **not** guarded: they forward the
  marker verbatim (D2), and `drop` of a compile-time-only marker discards it with nothing to
  dispose.

### Lowering: fusion + the `times` primitive (`src/ir.rs`)

- **R12: the quotation literal lowers to a phantom, no instruction.** `lower_term`'s new
  `TermKind::Quotation` arm (`src/ir.rs:2410`) interns the body into a `quot_bodies` table,
  mints a fresh `Value` with a placeholder `IrType` and **emits no `Instr`**, pushes it, and
  records `Value -> QuotId`. Because it defines no instruction and the checker guarantees it
  reaches only `call`/`times`/shuffle/bind, this phantom never enters an `Instr` operand, a
  `Phi`, or a `Terminator` (R7 rejects the only path, a branch merge, that would build a
  `Phi` over it).

- **R13: `call`-of-literal fusion.** `lower_call`'s new `"call"` arm pops the phantom
  quotation `Value`, resolves its `QuotId`, and lowers the body's terms in place via
  `lower_terms` (`src/ir.rs:2399`), emitting **no `Instr::Call`** and creating no runtime
  code value. `[ 1 + ] call` lowers exactly as `1 +`. This is the only inlining slice 4 owns
  (D5) and never crosses a `:` word boundary.

- **R14: `times` lowering into the back-edge machinery.** `lower_call`'s new `"times"` arm
  drives a constant-stack loop, reusing `begin_loop`/`finalize_loop` (D6):
  1. Pop the phantom quotation `Value` (top) and resolve its body; pop the `i64` **count**.
  2. Synthesize an induction `Value` seeded `Const 0`. Call
     `begin_loop(&[row..., index_seed], true)` where `row` is the remaining stack (the `..s`
     the body threads): each row slot gets its slice-3 carried-slot treatment (scalar phi, or
     an entry-hoisted **stable slot + staging** for an aggregate, `:2301`), and the index gets
     a scalar phi. `stage_aggregates = true` is load-bearing (R17).
  3. In the header (current after `begin_loop`), emit `cmp = Cmp(Lt, index_phi, count)` and
     seal it with `Terminator::Jnz(cmp, body_block, exit_block)` (`src/ir.rs:998`).
  4. In `body_block`: set `self.stack = row_phis`, push `index_phi` (the body reads the
     index as its top input), and `lower_terms(body)` (the splice), which transforms the
     row per the body's `..s i64 -- ..s` effect.
  5. Compute `index_next = Bin(Add, index_phi, Const 1)`. Record the back-edge exactly as a
     self-tail call does (`back_edges.push((body_pred, [row'..., index_next]))`,
     `src/ir.rs:2687`) and seal `body_block` with `Jmp(header)`.
  6. `finalize_loop()` back-patches the scalar phis (row scalars + index) and appends the
     aggregate read-before-write staging blits on the back-edge, unchanged from slice 3.
  7. Start `exit_block`; `self.stack = row_phis` (the carried row's header-phi outputs are
     the loop result, and the header dominates the exit).

- **R15: `times` saves and restores loop state so it composes.** `begin_loop` sets
  `self.header`/`self.entry_block`/`self.carried_slots`/`self.back_edges`
  (`src/ir.rs:2301`+), which the self-tail-call back-edge path also reads. R14 saves those
  four fields on entry and restores them after `finalize_loop`, so a `times` inside an
  otherwise-ordinary word (or a self-tail word) does not clobber an outer loop and vice
  versa. The headline witness `main` is not self-tail-recursive, so the common path saves
  four `None`/empty values, but the save/restore is required for correctness, not decoration.

- **R16: addition-only.** `Slot` gains a defaulted field; no existing golden or unit test
  changes expected output; no existing `Instr`/`Terminator` variant is added or changed
  (`Jnz`, `Cmp`, `Bin`, `Phi`, `Blit`, `Alloc` are all extant). `qbe.rs` is untouched.

### The constant-stack guarantee (`src/ir.rs`)

- **R17.** The loop runs in constant stack because (a) the carried row's aggregates ride
  slice 3's stable-slot staging (`begin_loop(_, true)`), so no carried aggregate re-allocates
  per iteration, and (b) any aggregate the body **constructs** each iteration is emitted
  while `entry_block.is_some()`, so `push_alloc` (`src/ir.rs:2252`) hoists its `Alloc` into
  the entry block (one slot reused every iteration) rather than bumping the stack per
  iteration (slice 3 recon 6: QBE `alloc` emits inline with no hoisting). R14 must keep
  `entry_block` set across the body splice (it is, until `finalize_loop`), so the
  body-constructed aggregate hazard is neutralized by the exact mechanism slice 3 built.

## Success criteria

Goldens in `tests/phase4_generics.rs` (the Phase 4 home); constant-stack goldens use the
existing signal-aware `run_stack_bounded_src` (`tests/phase4_generics.rs:239`, `ulimit -s
1024`), never an IL-string assertion. Naming is `thing_condition_expected`. `Rn` diagnostics
are behavioural negatives asserting message + named identifiers.

| # | criterion | golden | phase |
|---|---|---|---|
| 1 | `[ ... ]` parses into `TermKind::Quotation`; nested `[ [ ] ]` parses | `quotation_literal_parses_into_quotation_term` (parser unit) | 1 |
| 2 | `1 2 [ + ] call .` prints `3` (fusion runs) | `call_of_literal_quotation_fuses_and_runs` | 2 |
| 3 | a quotation forwarded through a bind then called runs (`[ + ] \| q \| 1 2 q call .` → `3`) | `quotation_forwarded_through_bind_still_calls` | 2 |
| 4 | headline: `0 1000000 [ + ] times .` prints `499999500000` in constant stack (1 MB, exit 0) | `times_loop_runs_in_constant_stack` | 3 |
| 5 | a `times` body constructing an aggregate each iteration runs in constant stack (R17) | `times_body_constructing_aggregate_runs_in_constant_stack` | 3 |
| 6 | IR-shape: a `times` call built a header block with a back-edge `Jmp` and **no** per-iteration `Instr::Call` (the only direct witness the primitive gets) | `times_lowers_to_a_loop_header_not_a_per_iteration_call` (lowering unit) | 3 |
| 7 | dogfood `examples/times.sth` (`0 1000000 [ + ] times .`) builds and prints `499999500000`, sitting beside `examples/countdown.sth`'s hand-threaded equivalent | `times_example_matches_hand_threaded_countdown` | 4 |
| R7 | `flag if [ 1 + ] else [ 1 - ] end call` rejects, naming the merge line | `call_of_branch_merged_quotation_is_error` | 2 |
| R8 | storing a quotation into an array rejects, naming the array-store position | `quotation_stored_in_array_is_error` | 2 |
| R9 | passing a quotation to a user `:` word rejects, naming the word | `quotation_passed_to_user_word_is_error` | 2 |
| R10 | a word leaving a quotation on the stack is a declared-vs-actual outputs mismatch, not a panic | `quotation_left_on_stack_is_output_mismatch` | 2 |
| R11 | `1 [ + ] +` (a quotation as an operand) rejects, naming the operator | `quotation_as_operator_operand_is_error` | 2 |

Criterion 6 is the only direct witness the internal loop primitive gets, since the
primitive is deliberately not user-facing (DESIGN.md:283): it asserts on IR structure (a
header `Block` reached by a back-edge `Terminator::Jmp`, and the absence of an `Instr::Call`
in the lowered `main`), mirroring slice 3's `header_phis`/`loop_header` structural tests,
not on emitted IL text.

## Sanctioned edits to existing tests

None expected. This slice is addition-only at the representation level (R16): `Slot` gains a
defaulted `quot` field, `parse_term` gains an arm that only fires on `[`, and `call`/`times`
are new dispatch arms. If a `parse_term` refactor forces a change to an existing
`unexpected token LBracket` negative test, that is the one place a sanctioned edit could
appear; call it out explicitly in the implementing commit the way slice 3 sanctioned its two
phi-count edits, so a reviewer can tell a sanctioned edit from a silently weakened one.

## Out of scope

A first-class runtime quotation type and the `Type`/`PolyType`/`IrType`/unification/mangling
changes it implies (recon 2, deferred to slice 6); escaping quotations and the
uniform-runtime-stack fallback (Phase 6); the interprocedural user-word inliner and the
`each`/`map`/`filter`/`fold`/`while` library (slice 5); a `while` intrinsic as a second
floor member (D6, declined); lifting `if` to polymorphic bodies and giving polymorphic
self-tail words the loop transform (recon 6/7, both slice 5); `if` as a combinator and
`Bool` as an enum (slice 9); nested/mutual quotation-loop optimization beyond R15's
save/restore; any new `Instr`/`Terminator`; any backend (`qbe.rs`) change; any REPL-facing
quotation work beyond what the shared checker/lowering already give a REPL line (a REPL
`times` rides the same `lower_call` arm, so it is in by construction, but no REPL-specific
retention like slice 2's is added).

## Delivery

Each phase leaves the tree green (`cargo fmt --check && cargo clippy -- -D warnings && cargo
test`) and coherent.

- **Phase 1, surface syntax + AST (parse only).** `TermKind::Quotation` (R1); `parse_term`
  bracket arm + unterminated/stray diagnostics (R2, R3). Exhaustive-match stubs in
  `check_term`/`poly_term`/`lower_term` so the tree compiles: the checker temporarily rejects
  a quotation as "not yet supported" (removed in Phase 2), lowering is unreachable behind it.
  Exit: criterion 1 (parser unit tests) + green build.

- **Phase 2, checker + `call`-of-literal fusion (the marker and its rejections).** `Slot`
  gains `quot` and the side table (R4); `check_term`/`poly_term` intern a literal (R5); `call`
  splices against the live stack (R6); the located rejections land (R7–R11); the fusion
  lowering for `call` (R12, R13) makes `[ + ] call` end-to-end. `times` is still rejected as
  not-yet. Exit: criteria 2, 3, R7–R11.

- **Phase 3, the `times` intrinsic and the constant-stack loop.** `times` typing (D6 splice
  against row + index, requiring the body return the row) and its lowering into
  `begin_loop`/`finalize_loop` with a synthesized index, header `Jnz`, and back-edge (R14),
  loop-state save/restore (R15), and the constant-stack guarantee (R17). Exit: criteria 4, 5,
  6 (the headline witness + the IR-shape test).

- **Phase 4, dogfood + docs.** Add `examples/times.sth` beside `examples/countdown.sth`;
  mark ROADMAP.md's slice-4 entry implemented; record D1–D6 and the marker/fusion/`times`
  design in DESIGN.md's iteration section. Exit: criterion 7.

## Non-functional / invariants

- Green unchanged: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- No new `Instr`/`Terminator`; `times` and fusion reuse `Jnz`/`Cmp`/`Bin`/`Phi`/`Blit`/`Alloc`
  and the existing `begin_loop`/`finalize_loop` staging.
- Backend stays **QBE**; `Ptr` opaque; no LLVM, no native backend, no JIT/comptime.
- `Type`/`PolyType`/`IrType` gain **no** variant (D1); the runtime quotation type is slice 6.
- `core` stays `no_std`; a non-escaping quotation is core (DESIGN.md:497,512).
- Constant stack preserved: every loop-body `Alloc` is entry-hoisted (R17), no per-iteration
  stack bump, witnessed under a 1 MB stack at 1e6 iterations (criteria 4, 5).

## Where the brief was underspecified, and what this spec did

- **"Carries its inferred effect."** There is no standalone effect to infer for a bare body
  (it would underflow); the spec resolves this as D3, the marker carries the **body**, and
  the effect is realized by splicing at the consumption site. This is strictly simpler and is
  what makes `call` "check identically to writing the body inline" true.
- **"How `Slot` grows, or quotations tracked off-stack."** The spec picks the on-`Slot`
  side-channel (D2/R4) over an off-stack table, because the `Copy`-moved-verbatim property of
  `Slot` gives bind/shuffle forwarding for free and mirrors the existing `alias`/`deriv`
  discriminators; an off-stack table would have to re-derive stack ordering by hand.
- **The lowering carrier.** The brief did not fix how a forwarded quotation reaches its
  `call` in lowering. The spec picks the phantom-`Value` with no defining instruction (D2/R12)
  over a checker→lowering Span map, because it needs no new cross-stage channel and reuses the
  fact that `lower_call` already moves `Value` ids verbatim through shuffles/binds; the
  checker's rejections guarantee the phantom never reaches a real `Instr`/`Phi`/`Terminator`.
- **`times` nested in a loop.** The brief did not say whether `times` must compose inside a
  self-tail word. The spec requires loop-state save/restore (R15) rather than rejecting the
  nesting, since it is a few lines and the alternative is a latent panic; left to spec review
  is whether to instead reject nesting explicitly if save/restore proves fiddly.

## Left for spec review to settle

- Whether R5's polymorphic-body arm should reject a quotation eagerly at the literal or only
  at a `call`/`times` inside a polymorphic body (both are out of scope this slice; the arm
  only needs to not panic and keep the match exhaustive).
- The exact placeholder `ty` a quotation `Slot`/phantom `Value` carries (R4/R12): any inert
  choice works since R11 guards every real consumer; review may prefer a dedicated
  `Type::Never`-style sentinel over reusing an existing scalar, weighed against D1's "add no
  `Type` variant".
- Whether `examples/times.sth` should also demonstrate a non-`+` body (e.g. an aggregate
  accumulator) to double as criterion 5's example, or stay minimal beside `countdown.sth`.

```json
{
  "phases": [
    { "phase": 1, "focus": "Surface syntax and AST: TermKind::Quotation, parse_term bracket arm distinct from type-position brackets, unterminated/stray diagnostics, exhaustive-match stubs keeping the tree green", "difficulty": "standard" },
    { "phase": 2, "focus": "Checker marker and call-of-literal fusion: Slot.quot side-channel and body side table, call splicing against the live stack, the merged/array/word-parameter/operand located rejections, and the call fusion lowering", "difficulty": "hard" },
    { "phase": 3, "focus": "The times intrinsic and the constant-stack loop: times typing, lowering into begin_loop/finalize_loop with a synthesized index, header Jnz and back-edge, loop-state save/restore, the constant-stack guarantee, the headline witness and the IR-shape test", "difficulty": "hard" },
    { "phase": 4, "focus": "Dogfood and docs: examples/times.sth beside countdown.sth, mark ROADMAP slice 4 implemented, record the marker/fusion/times design in DESIGN.md", "difficulty": "standard" }
  ]
}
```
