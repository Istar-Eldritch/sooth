# Phase 4 Slice 4: quotations + the `times` loop primitive (implemented)

Base: `main` @ `0f88ccb`. Adds a quotation literal `[ ... ]`, `call` to invoke it, and the
internal constant-stack loop primitive exposed as the single intrinsic `times`. Slices 1–3
supplied `Sig` type/row/length variables, native monomorphization, the REPL path, and the
loop-carried aggregate copy this builds on.

## Central constraint

No type at any layer can name a code value: `Type` (`src/ast.rs:566`), `PolyType`
(`src/ast.rs:406`), `Subst`, and `IrType` (`src/ir.rs:76`) have no code-value variant, and
adding one is a slice-1-sized change only escaping/non-inlined quotations need (Phase 6). So a
quotation is a **compile-time-only marker** carrying its body, consumed by `call` or `times`
via splicing, never a runtime value. The `Type`/`PolyType`/`IrType`/unification/mangling change
is deferred to slice 6, where a consumer for it exists.

## Locked decisions

- **D1: compile-time marker, no runtime type.** Consumed by `call`/`times` through splicing,
  never lowered to a runtime value. Marker kept minimal, not shaped to pre-empt the slice-6 type.
- **D2: the marker rides existing stacks as a phantom entry, forwarded asymmetrically.**
  Checker: a `Slot` (`src/check.rs:64`) with a new side-channel `quot: Option<QuotRef>`, `ty`
  a placeholder no user op accepts. Shuffles forward it free (`Slot` is `Copy`); a **bind is a
  second, explicit site** because a local read reconstructs a fresh `Slot` from a `Binding`
  (`src/check.rs:613`, read at `:4372`), so `Binding` also gains `quot`. Lowering: a phantom
  `Value` with no defining instruction, recorded in `quot_bodies: HashMap<Value, QuotId>`;
  `self.locals` forwards it verbatim through both binds and shuffles, so lowering has no
  asymmetry. `QuotRef` is a single `Known(QuotId)` variant (no `Merged`: a differing-quotation
  join is rejected outright, R7).
- **D3: carries a body, not a pre-computed effect.** `[ + ] call` checks identically to `+` at
  that point; fusion at the consumption site is both the checking and lowering rule.
- **D4: `call` accepts only a statically-known literal** (direct or forwarded through
  binds/shuffles). Identity lost at a merge, or a value that would have to be runtime (array
  element, non-inlined word parameter), is a **located rejection**, not a panic (R7–R10).
- **D5: the only inlining this slice owns is quotation-literal fusion.** A term-level local
  fusion in lowering; never crosses a `:` word boundary (the interprocedural inliner is slice 5).
- **D6: the floor is one intrinsic, `times`, passing the index; IR back-edge machinery reused.**
  `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` drives `begin_loop`/`finalize_loop`
  (`src/ir.rs:2301`/`:2348`) plus slice 3's carried-slot staging. `while` declined (its
  condition quotation is strictly harder). The floor is permanent, not a bootstrap.
- **D7: `if` unchanged; polymorphic-path gaps are slice 5's.** `if` stays a keyword, stays
  rejected in a polymorphic body; polymorphic self-tail words still get no loop transform.
  The `times` witness is monomorphic, so neither gap blocks this slice.

## Requirements by stage

Diagnostics `Rn` marked *(located)* are behavioural negatives asserting message **and** named
identifiers/positions.

### Surface syntax / parsing (`src/lexer.rs`, `src/parser.rs`, `src/ast.rs`)

- **R1.** `TermKind` (`src/ast.rs:787`) gains `Quotation(Vec<Term>)`, parsed from between a
  term-level `[` and `]`; nesting is by construction (element list is `parse_terms`). No new token.
- **R2.** `parse_term` (`src/parser.rs:1463`) gains an `LBracket` arm calling `parse_terms`
  and expecting `]`. The term-level `[` is **unambiguous** against type-level `[`: every type
  `[` reader is reached only from signature parsing, never `parse_term`, so no disambiguation is
  added; R2 replaces the old `other =>` hard-error reach for `[` only.
- **R3.** Unterminated `[` is a located parse error (reusing `parse_terms`' EOF path); a stray
  `]` is a located parse error parallel to the stray-`end`/`else` arm *(located, both)*.

### Representation / checking (`src/check.rs`, `src/ast.rs`)

- **R4.** `Slot` and `Binding` each gain `quot: Option<QuotRef>`, defaulted `None`
  (addition-only, R16); `Binding` forwarded at the local-read push (`:4372`). `QuotRef` is a
  single `Known(QuotId)` indexing a per-check `Vec<QuotBody>` (body terms + literal `Span`); no
  `Merged` variant (R7 rejects at the join). Placeholder `ty` pinned to **`Type::Cstr`**, for two
  reasons: *registry-free, so it never panics* (aggregate-shaped sentinels panic by registry
  index in `is_copy`/`is_linear`/`contains_reference` at bind-time before any guard); and *fewest
  type-directed acceptors*, all on R11's audit list, so the audit is smallest. `Bool` deliberately
  rejected (a missed guard on an `if` condition would silently miscompile a `Jnz` over a phantom).
  Consequence: with an inhabited scalar a **missed R11 guard is a silent accept**, which is why
  R11's audit list is load-bearing and converted to a table-driven test (R11t).
- **R5.** A `TermKind::Quotation` arm in `check_term` (`fn` at `:4248`) interns the body and
  pushes a quotation `Slot`; it does **not** check the body (D3). The mirror `poly_term` arm
  (`:2990`) **rejects eagerly at the literal**: `poly_term`'s stack is `Vec<PolyType>` with no
  `Slot` to hang `quot`, and D1 forbids a `PolyType` variant. Wording: `` a quotation in the
  polymorphic body of `{word}` (line N) is not yet supported ``.
- **R6: `call`.** New compiler-known word, intercepted in Call dispatch **before every builtin
  family and user-word lookup** (a local named `call` still wins). Requires a `Known(id)`
  quotation on top; pops it and **splices** the body against the live stack via `check_terms`,
  seeing current locals (capture free). **Bracketed with `scope.depth()`/`leave_block`** like the
  `if` arms, or a body that binds leaks the local and a linear bound value escapes the
  unconsumed-linear check. `tail` pinned **`false`** (both `call` and `times` splices): inheriting
  `tail` would check a spliced self-word as a tail self-call and run back-edge checks lowering
  never builds.
- **R7: different quotations at a branch join** *(located)*. Fires **at the join, not at
  consumption** (`If` arm merge, `:4573`): when either merged slot has `quot.is_some()` and the
  two are not the identical `Known(id)`, error. **Two phrasings** (both-quotations vs
  quotation-vs-value), selected on whether the other slot also has `quot.is_some()`, because the
  `Cstr` placeholder means a real `Cstr` opposite a quotation has equal `ty` and the ordinary
  mismatch never fires. Same `Known` id in both arms is safe (`lower_if`'s `t == e` fast path
  emits no `Phi`). This is what makes R12's containment true.
- **R8: array element** *(located)*, both store paths, one golden each:
  1. `fill` (guard *strictly above* its `contains_reference` registry index at `:5543`).
  2. store through `&!`/`!`/`+!` (guard *strictly above* `match_slot` at `:5447`, which returns
     `Exact` on a `Cstr`-into-`&!Cstr` store, a silent accept). R8r proves guard placement.
- **R9: non-inlined word parameter** *(located)*, two sites, both **before** unification:
  1. the `env` argument loop (`:4427`), covering generated constructors/setters and `extern`
     (wording says "word");
  2. `check_poly_call`'s input loop (`:3289`), before `unify_poly_input`, or a quotation binds
     `'T` to the placeholder, monomorphizes a real `Call` passing a phantom, and the diagnostic
     is unreachable.
- **R10: quotation on a word's exit** *(located)*. Not the ordinary arity/type mismatch (which
  leaks the placeholder spelling on a matching count): add an explicit quotation-at-exit branch
  in `check_outputs` (`:2049`), pinned string.
- **R11: audited default-deny** *(located, one helper)*. A single guard
  `reject_quotation_operand(ctx, span, op)` at **every** site that reads a popped/inspected
  `Slot.ty` for a type-directed decision, since `match_slot` returns `Exact` on `ty` equality
  reading no side channel. Audited sites from `check_term`'s Call dispatch order:
  `check_operator`/print/conversions, the **`if` condition** (guarded before the `cond.ty !=
  Type::Bool` return at `:4457`), `check_str_word`, `check_access_word`, `check_array_word`/
  `check_array_index`, `check_owned_cell_word`, `check_struct_peek`/`get_word`,
  `check_reference_word` (`&q` at `:5337`), the R9 sites, the R8 stores/`fill`, `check_outputs`,
  the self-tail back-edge row, and the REPL boundary (`:1899`). **Carve-outs, unguarded:**
  shuffles (`dup`/`swap`/`over`/`rot`, forward the marker verbatim) and `drop` (discards a
  compile-time marker; skip its `prov.dropped` push, `:5786`). The audit is a **test artifact**
  (R11t table-driven unit), not prose, because completeness is load-bearing.
- **R18: `times` typing**, the rule R14 presupposes. Compiler-known word intercepted alongside
  `call`; requires a `Known(id)` quotation on top and an `i64` count beneath; splices body
  against row + synthesized index, bracketed, `tail = false`. Four obligations:
  - **Identity on move/borrow state, not just the row.** Body runs N times but the splice
    checks once, so a body consuming a linear local checks clean and disposes N times. Rule:
    **clone `scope` before, require unchanged after**, spelled as `moves.states` equal and
    `live_derivs().collect::<HashSet>()` equal before/after. Do **not** reuse
    `check_linear_across_back_edge` (cruder, over/under-rejects, wrong message). Wording:
    `` a `times` body cannot consume `{name}` (line N): the body runs more than once… ``.
    Negative golden required (the `[ + ]` witness never exercises it).
  - **Reject a `times` nested in a loop, in the checker** (not lowering: `src/ir.rs` has no error
    channel): a `times` in a self-tail word via `has_self_tail_call` (carried as a new
    `Ctx::Word.self_tail: bool` computed in `word_ctx`, R16), and a `times` in a `times` body via
    a splice-depth counter on `Provenance` (R16), **restored** after each splice so sequential
    `times` don't false-positive. Wording names the line.
  - **Guard every slot of the row**, not just the top; a quotation anywhere in `..s` reaches
    `begin_loop` and phis a phantom. Same for the self-tail back-edge row.
  - Body net effect on the row must equal the row (D6); mismatch is the ordinary row-effect
    error, own negative golden (R18c).
- **R19: quotation on a REPL line's residual stack** *(located)*. `infer_line` (`:1906`) has no
  declared outputs, so R10 doesn't apply and the phantom would be marshalled into the carried
  stack. Reject parallel to the no-stored-reference position (`:1897`). This slice's work, not
  slice 6's.

### Lowering: fusion + `times` (`src/ir.rs`)

- **R12.** `lower_term`'s `Quotation` arm (`:2410`) interns into `quot_bodies`, mints a phantom
  `Value` with placeholder `IrType`, **emits no `Instr`**, records `Value -> QuotId`. Containment
  rests on **R7's join rejection**: the only `Phi`-over-phantom builder is a differing-quotation
  merge, now rejected at the join. Placeholder `IrType` pinned **`I64`** (any non-aggregate
  scalar; the IR side has no `if`-condition concern): `dup` blits and `drop` calls a destructor
  on aggregates via `value_type`, both left unguarded by R11.
- **R13: `call`-of-literal fusion.** `lower_call`'s `"call"` arm pops the phantom, resolves the
  `QuotId`, lowers the body via `lower_terms(body, false)`, emits **no `Instr::Call`**. `tail =
  false` is load-bearing (self-tail arm fires on `tail && header.is_some() && name ==
  cur_word_name`). Own unit (`call_of_literal_emits_no_call_instr`).
- **R14: `times` lowering** into the back-edge machinery:
  0. Nested `times` already rejected by the checker (R18); lowering may assume
     `self.header.is_none()`, keeps only a `debug_assert!`. (Nesting can't ride R15 alone:
     `begin_loop` unconditionally sets `entry_block`, so an inner loop hoists once-per-outer or
     reads a stale stable slot. Clean split deferred.)
  1. Pop phantom quotation and body; pop the `i64` count.
  2. Synthesize an index seeded `Const 0`; `begin_loop(&[row..., index_seed], true)`
     (`stage_aggregates = true`, R17).
  3. Header: `cmp = Cmp(Lt, index_phi, count)`, seal `Jnz(cmp, body, exit)`.
  4. Body: `self.stack = row_phis`, push `index_phi` (body reads index on top), splice
     `lower_terms(body, false)`.
  5. `index_next = Add(index_phi, 1)`; require `!self.terminated`; record back-edge
     (`back_edges.push((body_pred, [row'..., index_next]))`), seal `Jmp(header)`.
  6. `finalize_loop()` back-patches scalar phis and appends aggregate staging blits (slice 3,
     unchanged).
  7. Start `exit_block`, **reset `self.terminated = false`** (or every later term is dropped),
     `self.stack =` `begin_loop`'s returned `Vec<Value>` minus the trailing index. Not "header-phi
     outputs": an aggregate carried slot has no header phi, `begin_loop` returns the stable slot
     pointer.
- **R15: save/restore loop state.** Save `header`/`entry_block`/`carried_slots`/`back_edges` on
  entry, restore after `finalize_loop`. Required independent of nesting: `finalize_loop`
  `mem::take`s only `carried_slots`/`back_edges`, never clears `header`/`entry_block`, so a
  `times` in an ordinary word would leave `entry_block` set and mis-hoist a later `Alloc`.
  Restoring `header = None` is what lets two sequential `times` run. Golden (extended to build an
  aggregate *after* the first `times`) + unit asserting all four fields restored.
- **R16: addition-only, forced edits named.** `Ctx::Word` gains `self_tail: bool` (in
  `word_ctx`), `Provenance` gains the splice-depth counter (both only to make R18 reachable);
  `Slot`/`Binding` each gain a defaulted `quot`. No existing golden/unit output changes; no
  `Instr`/`Terminator` variant added; `qbe.rs` untouched. Rust has no default fields, so the two
  **full** `Slot` literals (`:4267`, `:4573`) are edited; spread sites are free.

### Constant-stack guarantee (`src/ir.rs`)

- **R17.** Constant stack because (a) carried aggregates ride slice 3's stable-slot staging
  (`begin_loop(_, true)`), and (b) a body-constructed aggregate is emitted while
  `entry_block.is_some()`, so `push_alloc` (`:2252`) hoists its `Alloc` into the entry block (one
  reused slot). Witnessed **deterministically** by an IR-shape assertion (every `Instr::Alloc` in
  the loop entry block, none in the body block, on 5a's source), plus the 1e6 bounded run as a
  coarse backstop.

## Success criteria

Goldens in `tests/phase4_generics.rs`. Value/effect via `run_src`; constant-stack via
`run_stack_bounded_src` (`ulimit -s 1024`, returns `Option<i32>` exit code only, no stdout, so a
value claim can't ride it alone). Diagnostic goldens use a `check_error`/`parse_error` helper
**added** to the file (sanctioned). Every source pinned. `SPY_DEF` (`tests/phase3_locals.rs:75`)
shifts line numbers by 2.

| # | criterion | golden | phase |
|---|---|---|---|
| 1 | `[ ... ]` and nested `[ [ ] ]` parse | `quotation_literal_parses_into_quotation_term` | 1 |
| 1b/1c | unterminated `[` / stray `]` located parse errors | `unterminated_…` / `stray_closing_bracket_…` | 1 |
| 2 | `1 2 [ + ] call .` prints `3` | `call_of_literal_quotation_fuses_and_runs` | 2a |
| 3 | quotation forwarded through a bind then called → `3` | `quotation_forwarded_through_bind_still_calls` | 2a |
| 3b | body reads an enclosing local → `8` | `quotation_body_reads_enclosing_local` | 2a |
| 6b | `call` emits no `Instr::Call` | `call_of_literal_emits_no_call_instr` (unit) | 2a |
| R12u | `Quotation` arm emits no `Instr`, records `quot_bodies` | `quotation_literal_emits_no_instr_and_records_body` (unit) | 2a |
| Cu1 | quotation survives shuffles + bind | `quotation_survives_dup_swap_and_bind` (unit) | 2a |
| R7 | two quotations at a join reject **at `end`** | `different_quotations_at_a_join_are_error` | 2b |
| R7n | quotation vs real `Cstr` at a join (second phrasing) | `quotation_versus_value_at_a_join_is_error` | 2b |
| Cu2 | join rejects two different; same `Known` id passes | `merged_quotations_are_rejected_at_the_join` (unit) | 2b |
| R8f/R8r | store via `fill` / via `&!Cstr` reference reject (guard placement) | `quotation_stored_in_array_by_fill_…` / `…through_a_reference_…` | 2b |
| R9/R9p | quotation to user / polymorphic word rejects | `quotation_passed_to_user_word_…` / `…polymorphic_word_…` | 2b |
| R5p | quotation literal in a polymorphic body rejects | `quotation_in_polymorphic_body_is_error` | 2b |
| R10 | count-matching word exit gets the dedicated output diagnostic | `quotation_left_on_stack_is_output_error` | 2b |
| R11 | quotation as an operator operand rejects, naming `+` | `quotation_as_operator_operand_is_error` | 2b |
| R11if | quotation as `if` condition rejects (not a `Bool` mismatch) | `quotation_as_if_condition_is_error` | 2b |
| R11drop | `1 [ + ] drop .` → `1` (the one legal unguarded consumer) | `quotation_dropped_is_a_pure_pop` | 2b |
| R11t | table over `(source, op_name)`, one row per audited site | `quotation_as_operand_is_rejected_at_every_audited_site` (unit) | 2b |
| R6br1 | two calls of a binding-body quotation both run → `4`,`6` | `two_calls_of_a_binding_quotation_body_both_run` | 2b |
| R6br2 | linear bound inside a body left unconsumed at `call` rejects | `linear_bound_inside_a_quotation_body_is_error` | 2b |
| R19 | quotation on a REPL residual stack rejects | `quotation_left_on_repl_line_is_error` | 2b |
| 4a | `0 1000000 [ + ] times .` prints `499999500000` | `times_loop_computes_the_index_sum` | 3 |
| 4b | 4a runs under 1 MB, `Some(0)` (cheap tripwire) | `times_loop_runs_in_constant_stack` | 3 |
| 5a | body constructs a 16-byte `Vec2` each iter → `499999500000` | `times_body_constructing_aggregate_computes_expected` | 3 |
| 5b | 5a runs in constant stack, `Some(0)` (R17 backstop) | `times_body_constructing_aggregate_runs_in_constant_stack` | 3 |
| 5c | aggregate carried **through the row** → `3000000` | `times_carrying_an_aggregate_through_the_row_runs` | 3 |
| 5z | non-index seed, zero trips → `7` | `times_zero_trip_yields_seed_row` | 3 |
| R18a | body consuming a linear local rejects naming `s` | `times_body_consuming_a_linear_local_is_error` | 3 |
| R18b | quotation anywhere in the row rejects | `times_with_a_quotation_in_its_row_is_error` | 3 |
| R18c | body net effect ≠ row fires the row-effect error | `times_body_changing_the_row_is_error` | 3 |
| R18u | three typing obligations | `times_typing_obligations` (unit) | 3 |
| N | `times` nested in a loop rejects in the checker with a line | `times_nested_in_a_loop_is_rejected` (`check_error`) | 3 |
| 15 | two sequential `times` run + aggregate after the first prints | `two_sequential_times_in_one_word_both_run` | 3 |
| R15u | four loop-state fields restored after the `times` arm | `times_saves_and_restores_loop_state` (unit) | 3 |
| 6 | IR shape: index header `Phi`, header `Jnz`, back-edge `Jmp`, no per-iter `Call`, every `Alloc` in the entry block (5a) | `times_lowers_to_a_loop_header_not_a_per_iteration_call` (unit) | 3 |
| 7 | `examples/times.sth` builds, prints `500000500000`, matches `countdown.sth` | `times_example_matches_hand_threaded_countdown` | 4 |

Criterion 6 is the primary direct witness (the primitive is deliberately not user-facing). The
4/5 value/constant-stack split exists because the bounded harness never captures stdout. 4b is a
tripwire only (4a emits no `Alloc`); the real R17 witness is 5b + criterion 6's entry-block
assertion on 5a. Example 7 computes the same number as `countdown.sth` (parity, not off by 1e6).

Unit coverage beside every changed stage function (CLAUDE.md convention). Load-bearing units:
R12u, R11t, R18u, R15u, Cu1/Cu2. Also `check_outputs` (R10), `infer_line` (R19),
`check_poly_call` (R9p), `poly_term` (R5p) each get a targeted unit beside their goldens.

## Sanctioned edits to existing tests

One declared addition: a `check_error` (and if needed `parse_error`) helper in
`tests/phase4_generics.rs`, copied from `tests/phase3_locals.rs:59,65` /
`tests/phase3_refs.rs:45`. No existing test's expected output changes. If a `parse_term`
refactor forces a change to an `unexpected token LBracket` negative (no test asserts that string
today), call it out in the commit like slice 3's phi-count edits.

## Out of scope

The first-class runtime quotation type and its `Type`/`PolyType`/`IrType`/unification/mangling
changes (slice 6); escaping quotations and the uniform-runtime-stack fallback (Phase 6); the
interprocedural inliner and the `each`/`map`/`filter`/`fold`/`while` library (slice 5); `while`
as a second floor member (declined); lifting `if` to polymorphic bodies and the polymorphic
self-tail loop transform (slice 5); `if` as a combinator and `Bool` as an enum (slice 8); any new
`Instr`/`Terminator`; any `qbe.rs` change; REPL quotation work **beyond R19**.

## Invariants

- Green unchanged: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- No new `Instr`/`Terminator`; `times`/fusion reuse `Jnz`/`Cmp`/`Bin`/`Phi`/`Blit`/`Alloc` and
  the existing `begin_loop`/`finalize_loop` staging.
- Backend stays QBE; `Ptr` opaque; no LLVM, native backend, or JIT/comptime.
- `Type`/`PolyType`/`IrType` gain no variant (D1); the runtime quotation type is slice 6.
- `core` stays `no_std`; a non-escaping quotation is core.
- Constant stack preserved; every body `Alloc` entry-hoisted (R17), witnessed by criterion 6's
  entry-block assertion on 5a and backed by the 1 MB bounded run at 1e6 iterations (5b).

## Delivery (as implemented)

- **Phase 1 — surface syntax + AST** (`ad66d65`): `TermKind::Quotation` (R1); `parse_term`
  bracket arm + unterminated/stray diagnostics (R2, R3); exhaustive-match stubs where the checker
  rejects with a distinct `"TEMP-quotation consumer not yet wired"` stopgap string (deleted by
  string in 2b; a grep for `TEMP-quotation` returns nothing at slice end). Exit: criterion 1.
- **Phase 2a — marker + `call`-of-literal fusion** (`c5e07fb`, fixup `69d3496`): `Slot`/`Binding`
  `quot` + side table (R4, D2); `check_term` interns, `poly_term` rejects (R5); `call` splices
  bracketed with `tail = false` (R6); fusion lowering (R12, R13). Stopgap retained for every other
  consumer. Exit: 2, 3, 3b, 6b, R12u, Cu1.
- **Phase 2b — located rejections replace the stopgap** (`ed628f1`): R7 (both phrasings), R8
  (fill + reference), R9/R9p, R5p, R10, R11's audited default-deny (incl. `if` condition and
  `drop` carve-out), R6 bracketing, R19; stopgap deleted by string. Exit: R7, R7n, Cu2, R8f, R8r,
  R9, R9p, R5p, R10, R11, R11if, R11drop, R11t, R6br1, R6br2, R19.
- **Phase 3 — `times` intrinsic + constant-stack loop** (`d959907`): `times` typing (R18:
  row+index splice, clone-and-compare move/borrow identity, whole-row guard, nested-`times`
  checker rejection) and lowering (R14, `tail = false`, `debug_assert!(self.header.is_none())`),
  loop-state save/restore (R15), constant-stack guarantee (R17). Exit: 4a, 4b, 5a, 5b, 5c, 5z,
  R18a, R18b, R18c, R18u, N, 15, R15u, 6.
- **Phase 4 — dogfood + docs** (`9f1d463`): `examples/times.sth` beside `countdown.sth`
  (same sum); ROADMAP slice-4 marked implemented; D1–D6 and the marker/fusion/`times` design
  recorded in DESIGN.md. Exit: criterion 7.

## Resolved since the draft

- R5's polymorphic-body arm rejects eagerly at the literal (forced: no `Slot` in `poly_term`, no
  `PolyType` variant allowed).
- Placeholder `ty` pinned to `Type::Cstr` (registry-free, fewest acceptors, all on R11's list);
  safe only paired with R11's audited default-deny.
- `examples/times.sth` computes the same number as `countdown.sth`; criterion 5 gets its own
  pinned in-test source rather than doubling as the example.
