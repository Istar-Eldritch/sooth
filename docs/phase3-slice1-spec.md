# Phase 3 Slice 1 — Linear analysis + move + `dup`-gate + explicit `drop` (spec)

## Context

The first cut of the **linear spine**, isolated from heap. This slice makes the compiler
enforce linear (use-*exactly*-once) discipline: values move by default, a second use of a
moved value is a compile error, `dup`/`over` are gated on `Copy`, `drop` runs a destructor,
and **forgetting to dispose a linear value is a compile error** (no auto-drop, no join
reconciliation; Copy and linear are handled symmetrically for disposal). It is proven on a
**test-only builtin drop-spy** whose destructor prints, so drop count/order/timing are
golden-observable. Aggregates are in scope via **destructure-whole** (no partial moves). No
heap, no references, no RC, no fds. Design is locked in `docs/phase3-slice1-brief.md` (D1-D8)
and the target semantics are in `DESIGN.md` "## The linear spine".

This slice **changes the compiler** (`src/**`), unlike the Slice 7 dogfood.

## Requirements (traceable)

- **R1 (D1)** — Linear, exactly-once. No auto-drop anywhere. A linear value must be consumed
  (moved out as a declared output, moved into a consuming word, or `drop`ped) exactly once.
- **R2 (D1)** — Copy/linear symmetry for disposal: every value is accounted for by the
  signature or an explicit `drop`, and a surplus value at scope end is an error for both. The
  asymmetry is only that (i) `drop` on a Copy value has no runtime effect while on a linear
  value it runs the destructor, and (ii) a Copy value may be `dup`ed and `drop`ped freely
  with no ownership obligation, whereas a linear value must be consumed exactly once.
- **R3 (D2)** — Mentioning a linear local moves its value out; a second mention is a
  `use after move` error that names the move site. No mid-body rebinding (existing rule).
- **R4 (D3)** — `dup` and `over` on a non-`Copy` value are compile errors with a diagnostic
  of the DESIGN.md form ("cannot `dup` a value of type X; X is linear …"). `swap`/`rot`/`nip`
  (pure reorderings) are legal on linear values.
- **R5 (D4)** — `drop` is the universal disposal primitive: it runs the destructor on a
  linear value, discards a Copy value. (User-overridable `drop` is Phase 4; here destructors
  are compiler-known.)
- **R6 (D5)** — A **test-only builtin linear primitive** exists: the drop-spy, internal name
  **`__spy`** (not user-facing surface, not in the tutorial, convention-fenced not
  flag-gated), carrying an `i64` tag, with a compiler-known destructor that prints a
  deterministic line (`drop <tag>`). It lowers as an `i64` under the hood (reuse
  `IrType::Int{64,signed}`) and its constructor `( i64 -- __spy )` lowers inline as identity
  (no emitted `Call`). It is the sole linear primitive this slice introduces, and it
  dissolves into an ordinary type once `drop` is overridable (Phase 4).
- **R7 (D6)** — A struct/enum is **linear iff any field/variant-payload is linear**
  (transitive). Plain-data aggregates stay `Copy`, unchanged.
- **R8 (D6)** — `dup`/`over` on a linear aggregate is an error. `S` (construct) moves fields
  in. `S>` (destructure) consumes the aggregate and pushes all fields (each linear field then
  tracked per R3).
- **R9 (D6)** — `S>fi` (get) keeps its `( S -- field )` consuming effect for **any** field;
  on a linear receiver it additionally runs drop glue on the non-extracted fields. Existing
  Copy code is unchanged (drop-the-rest is a no-op when the rest is Copy).
- **R10 (D6)** — `S|>fi` is a **new** non-consuming `( S -- S field )` peek, **Copy fields
  only**; on a linear field it is a compile error (workaround: `S>`). It transfers no
  ownership, needs no reference machinery.
- **R11 (D6)** — `S<fi` (set) is allowed on linear aggregates and **drops the overwritten
  field if it is linear**; the other linear fields transfer via the existing blit-move (old
  shell consumed, never dropped).
- **R12 (D6)** — The compiler synthesizes **recursive/tag-dispatched drop glue**: struct →
  drop its linear fields in declaration order; enum → dispatch on the tag, drop the active
  variant's linear payload.
- **R13 (D7)** — Disposal is explicit: **no compensating drop is inserted for a value the
  programmer forgot** (at scope end or a branch join). A linear value left on the stack
  beyond the declared outputs is a stack-effect error (reusing `check_outputs`); a linear
  local never consumed by scope end is an error (`linear value X is never consumed; drop it
  or return it`). The drop glue in R9/R11/R12 is *not* an exception: it is part of the
  defined semantics of `S>fi`/`S<fi`/`drop`, not compensation for a forgotten value.
- **R14 (D7)** — Branch joins: no *drop* reconciliation, but the checker reconciles move-
  **state**. Move-state is a three-value lattice per linear local: `Live`, `Moved(site)`,
  `MaybeMoved(site)`. At a join the per-local states combine: equal states are preserved;
  any disagreement (`Live` vs `Moved`, or anything vs `MaybeMoved`) yields `MaybeMoved`. A
  later use of a `Moved` or `MaybeMoved` local is a use-after-move error located at the use; a
  local still `Live` or `MaybeMoved` at scope end and never consumed is an unconsumed-linear
  error located at scope end. So consumed in both arms is `Moved` (ok); consumed in one arm
  only is `MaybeMoved` (unconsumed-linear at scope end, or use-after-move if referenced past
  the join). The compiler errors; it never inserts compensating drops. Stack linear values
  are anonymous this slice (no identity tracking), so only a local's move-state can diverge —
  this stops holding once refs land (slice 4). The move-state is threaded as `&mut` through
  the checker walker (`Ctx` is immutable today), which is the bulk of the Phase 1 diff.
- **R15 (D8)** — Deferred, and enforced as located errors where reachable: a linear value
  live across a Slice 6 back-edge is a `linear values across a loop are not supported yet`
  error (Copy loops unaffected). Out of scope entirely: partial/field-level moves (excluded
  by destructure-whole), user-overridable `drop` (Phase 4), heap/refs/RC/fds.
- **R16 (IR constraint)** — No new `Instr`/`Terminator` variant: `drop` lowers to a `Call` to
  the (builtin or synthesized) destructor; the spy's print reuses the existing print path;
  branch dispatch reuses the existing `Jnz`. `.` (print) applied to a linear value is rejected
  in the checker's printable-scalar path (the backend's `unreachable!` guards assume this).
  `close`/`free` are *not* in this slice (library words layered on `drop` later).

## Delivery phases (dependency-ordered)

### Phase 1 — Isolated linear core on bare drop-spy values `[hard]`

The load-bearing novel analysis, with **no aggregates** yet, so a bug is in the analysis,
not in struct/enum glue.

- **Copy-vs-linear predicate** over `Type`: every current type is `Copy`; the new drop-spy is
  linear. A single query `is_copy(&Type) -> bool` (or `Linearity`) threaded where needed.
- **The drop-spy builtin** (`__spy`, per R6): a new `Type` variant lowering as `i64`; its
  constructor `( i64 -- __spy )` is the first entry in `builtin_table()` (empty today) and
  lowers inline as identity; its compiler-known destructor emits a runtime print of the tag
  via the existing print path. The new `Type` variant forces match arms across the listed
  files (ir.rs/ast.rs); `.` on a `__spy` is rejected in `check_operator`.
- **Move-tracking on linear locals** (forward flow through straight-line and `if`/`else`;
  the clause-body branch of the pass exists but ships untested here, since a linear clause
  payload needs Phase 2/4): the `Live`/`Moved`/`MaybeMoved` lattice of R14, threaded as
  `&mut` through the checker walker. First mention moves; a later use of a moved/maybe-moved
  local is `use after move` naming the move site; a divergent-arm move yields `MaybeMoved`.
- **`dup`/`over` gate**: reject on non-`Copy`, DESIGN.md-form diagnostic (both words).
- **`drop` lowering**: on a linear value, emit the destructor call (spy prints its tag); on
  Copy, discard as today.
- **Linear disposal check**: `check_outputs` @check.rs:554 *already* errors on any surplus
  value (pure length comparison), so a surplus linear value is caught today; decide whether
  to give it a distinct linear-flavoured message (needs the linearity query + a branch before
  the generic arity error) or reuse the arity error. The genuinely **new** work is a separate
  **unconsumed-linear-local** pass (locals are not on `final_stack`, so `check_outputs`
  cannot see them). Copy surplus keeps the existing error; add a no-misfire golden.
- **Deferred guard (R15)**: a linear value live across the self-tail-call back-edge is a
  located "not supported yet" error. Tail-ness is computed by a separate syntactic pass
  (`tail_position_calls`/`has_self_tail_call` @check.rs:~600) the stack-simulating walker does
  not see, so thread tail-position into the walker (or a pre-pass mapping term positions to
  tail-ness) as part of this slice; it is a real checker hook, not an afterthought.
- **REPL `:quit` disposal**: `src/repl.rs` walks the residual typed stack at `:quit` and runs
  each linear value's destructor LIFO (the REPL-main scope end; see "REPL residual linear
  values" below). Word definitions typed at the REPL keep the strict rule.
- **Changes**: `src/check.rs`, `src/ir.rs`, `src/backend/qbe.rs`, `src/ast.rs` (drop-spy
  `Type` variant + constructor), `src/repl.rs` (`:quit` disposal), `tests/phase0.rs`,
  `tests/phase1.rs` (REPL `:quit` golden). (`src/lexer.rs`/`src/parser.rs` are *not* needed
  here: `__spy` resolves via `resolve_type_name`/`Type::from_name`, which the parser already
  calls; the `S|>fi` surface form is Phase 3.)
- **Exit**: criteria 1 (incl. `over`), 2, 3, 4a/4b/4c, 10a/10b/10c, 12, 14 (REPL `:quit`
  disposal), and the `swap`-legal positive pass as goldens; `cargo fmt --check && cargo clippy
  -- -D warnings && cargo test` green; all pre-existing examples/tests still pass.

### Phase 2 — Struct aggregates via destructure-whole

- **Linear propagation** for structs: linear iff any field linear (transitive).
- **`S>fi` drop-the-rest** on a linear receiver (R9); **`S<fi` drop-on-overwrite** (R11);
  **recursive struct drop glue** (R12, struct case, via the synthesized-destructor mechanism
  chosen in Phase 4). `S<fi` drops the overwritten field **before** the store (read old value,
  drop it, then store new) so the order is deterministic and golden-assertable.
- **Changes**: `src/check.rs`, `src/ir.rs` (`lower_struct_word` @ir.rs:1592),
  `src/backend/qbe.rs`, `tests/phase0.rs`.
- **Exit**: criteria 5, 5b (transitive), 6, 8, 13 (drop-whole-struct glue order) pass as
  native goldens; green; no regression.

### Phase 3 — The `S|>fi` non-consuming peek word

- New surface form `Struct|>field`. **Lexing rule** (needed because `|` is a hard delimiter
  emitting `Token::Pipe`, used by `| locals |` and enum clause heads `| Circle …`): in the
  word scanner glue `|` into the current word only when it is immediately followed by `>`
  *and* immediately preceded by a non-whitespace word char, so `Point|>x` scans as one word
  while `| a b |` and `| Circle` are untouched; re-verify `examples/shapes.sth` and
  `examples/vm.sth` do not regress. Then parser/ast, checker typing (`( S -- S field )`, Copy
  field only, error on a linear field), inline non-consuming projection lowering.
- **Changes**: `src/lexer.rs`, `src/parser.rs`, `src/ast.rs`, `src/check.rs`, `src/ir.rs`,
  `tests/phase0.rs`.
- **Exit**: criteria 7a (peek keeps the aggregate live, proven by full-stdout one-drop) and
  7b (peek on a linear field is a compile error) pass; green; no regression.

### Phase 4 — Enums: tag-dispatched drop glue

- **Drop-glue home (decision)**: synthesize a **destructor function per linear aggregate
  type** (an `IrFunc` appended during lowering, taking the aggregate as a param) that `drop`
  calls, rather than block-splitting inside `lower_call("drop")`. The existing whole-function
  `lower_clauses` (@ir.rs:1747, builds N clause blocks + a `Phi` join) is *not* reusable for
  tag dispatch; the synthesized destructor does its own tag `Jnz` + per-variant field drops.
  This keeps every `drop` a plain `Call` (R16) and avoids `lower_call` creating
  blocks/terminators. The struct case (Phase 2) uses the same mechanism, so the two are
  uniform.
- **Enum case**: the synthesized destructor tag-dispatches and drops the active variant's
  linear payload; a matched clause consumes/drops its exposed payload per R3/R13.
- **Open hole inherited from Phase 2 (start here)**: `check::is_copy` returns `true` for
  `Type::Enum` unconditionally, so an enum with a linear payload is silently duplicable and
  droppable-to-nothing today: `type: Box | Full v __spy | Empty ;` with
  `1 __spy Full dup drop drop` compiles, runs, and prints nothing (an R1 exactly-once
  violation with no diagnostic). Phase 2 deliberately left this rather than shipping a
  reject-guard it would delete here, unlike the linear *array element* case, which is a
  located not-supported-yet error because arrays get no glue in this slice at all. Extending
  `is_copy` to enums (linear iff any variant has a linear field) belongs with the
  tag-dispatched glue and must land in the same phase as it. Note this makes linearity a
  *four*-site decision: `check::is_copy`, `ir::field_is_linear`, the `ensure_struct` fold,
  and the new enum-variant fold; `struct_linearity_agrees_across_the_checker_and_both_lowering_folds`
  (@ir.rs) pins the first three and should be extended to cover enums.
- **Changes**: `src/check.rs`, `src/ir.rs`, `src/backend/qbe.rs`, `tests/phase0.rs`.
- **Exit**: criteria 9 (runtime tag dispatch, stdout differs per tag) and 9b (matched clause
  disposes payload) pass; green; no regression.

### Phase 5 — Regression sweep, REPL parity, doc consistency

- Confirm every existing example and test stays green.
- **No new `examples/*.sth`**: a linear-disposal example would have to use `__spy`, which is
  convention-fenced *out* of user surface (R6/D5), so linear disposal is demonstrated by a
  golden in `tests/phase0.rs`, not an example file.
- REPL parity golden (`tests/phase1.rs`) for the disposal path (a `__spy` created and
  `drop`ped within one line prints once). **See the open REPL decision below** for residual
  linear values that persist across lines.
- Confirm `DESIGN.md`/`ROADMAP.md` linear framing matches what shipped.
- **Changes**: `tests/phase0.rs`, `tests/phase1.rs`.
- **Exit**: criterion 11 (no regression) holds; green.

## REPL residual linear values (resolved)

The REPL session is the "main" word of the interactive program: the residual stack carried
across lines (`self.types`, repl.rs:514) is that word's working stack, and `:quit` is the end
of its scope. Because the body is revealed incrementally and is never complete until `:quit`,
the compile-time "you forgot to dispose X" proof can never fire (the next line might consume
it), so the linear scope-end check degenerates gracefully at runtime: **at `:quit` the REPL
runs the destructor of every linear value left on the residual stack**, top-of-stack first
(LIFO unwind). Exactly-once is preserved (each residual value is disposed once, at `:quit`),
so this is consistent with the linear core invariant (R1); what is relaxed is only that
forgetting cannot be a *compile error* in a live, never-complete session.

**Scope of the relaxation**: it applies only to residual bare-expression stack values. A word
*definition* entered at the REPL (`: foo … ;`) is checked with the full strict linear rule,
exactly like a compiled word (forgetting a linear value in `foo`'s body is a compile error,
R13). A residual linear value persists across lines and may be consumed by a later line; only
the leftovers at `:quit` are auto-disposed.

**Implementation**: `src/repl.rs` walks the residual typed stack at `:quit` and invokes each
value's destructor via the same drop-glue mechanism (bare `__spy` in Phase 1; aggregates once
their glue exists in Phases 2/4). Golden (criterion 14): a REPL session creates a `__spy`,
quits, and asserts the destructor printed at `:quit`, and a paired session that `drop`s it
explicitly on an earlier line prints exactly once (not again at `:quit`).

## Criterion → test map

All goldens are runnable native binaries (`tests/phase0.rs`) or REPL sessions
(`tests/phase1.rs`), never IL-string assertions. **Every negative golden asserts the
diagnostic substring *and* the backticked type name** (per the `tests/phase0.rs` house
pattern); criterion 2 additionally asserts the move-site location, and criterion 12 asserts
the error is located. **Every drop-observing golden compares the *complete* stdout string
(`assert_eq!`), not `contains`**, so "exactly once" and drop *order* are actually proven.

| # | Criterion | Test (phase) |
|---|---|---|
| 1 | `dup` on a bare spy is a compile error with the linear diagnostic | `dup_of_linear_value_is_error` (P1) |
| 1b | `over` on a bare spy is a compile error | `over_of_linear_value_is_error` (P1) |
| 2 | Use-after-move: a moved spy local used twice errors, naming the move site | `use_after_move_of_linear_local_is_error` (P1) |
| 3 | A destructor runs **exactly once** at explicit `drop` (full-stdout equality) | `explicit_drop_runs_destructor_once` (P1) |
| 4a | Surplus linear value left on the stack is a compile error | `surplus_linear_on_stack_is_error` (P1) |
| 4b | A linear local never consumed is a compile error | `unconsumed_linear_local_is_error` (P1) |
| 4c | No misfire: a surplus **Copy** value keeps the existing arity error | `surplus_copy_value_keeps_existing_error` (P1) |
| swap | `swap`/`rot` on linear values is allowed (guards against an over-broad gate) | `swap_of_linear_values_is_allowed` (P1) |
| 10a | A spy consumed in **both** arms compiles and drops exactly once (full stdout) | `both_arms_consume_linear_ok` (P1) |
| 10b | Consumed in one arm only, referenced past the join → use-after-move | `divergent_arm_use_is_error` (P1) |
| 10c | Consumed in one arm only, not referenced → unconsumed-linear at scope end | `divergent_arm_unconsumed_is_error` (P1) |
| 12 | A linear value across a Slice 6 back-edge is a **located** not-supported-yet error; a Copy `countdown`-shaped loop is unaffected | `linear_across_loop_back_edge_is_located_error` + `copy_loop_still_compiles` (P1) |
| 5 | Destructure-whole: `S>` a struct of ≥2 distinctly-tagged spies, drop them; order asserted (full stdout) | `destructure_whole_drops_each_field` (P2) |
| 5b | Linear-ness is **transitive**: struct-of-struct-of-spy is linear and drops correctly | `nested_struct_is_linear_transitively` (P2) |
| 6 | `S>fi` on a linear struct extracts one field and drops the rest (full stdout) | `get_field_drops_the_rest_on_linear_struct` (P2) |
| 8 | `S<fi` drops the overwritten spy **before** the store, keeps the rest (order pinned) | `set_field_drops_overwritten_linear_field` (P2) |
| 13 | `drop` of a whole linear struct runs the synthesized field glue in **declaration order** (≥2 tagged spies, full stdout) | `drop_of_linear_struct_runs_field_glue_in_declaration_order` (P2) |
| 7a | `S|>fi` peeks a Copy field, leaving the struct live: peek twice then dispose → exactly one drop line | `peek_copy_field_keeps_struct` (P3) |
| 7b | `S|>fi` on a linear field is a compile error | `peek_linear_field_is_error` (P3) |
| 9 | Tag dispatch: a linear enum built at runtime behind an `if` with ≥2 differently-shaped variants, dropped unmatched, drops the **active** variant's payload (stdout differs per tag) | `drop_of_linear_enum_dispatches_on_tag` (P4) |
| 9b | A matched clause consumes/drops its exposed linear payload | `clause_body_disposes_linear_payload` (P4) |
| 14 | REPL `:quit` runs destructors on residual linear values (LIFO); a value `drop`ped on an earlier line prints exactly once, not again at `:quit` | `repl_quit_disposes_residual_linear` + `repl_explicit_drop_not_redisposed_at_quit` (P1, REPL session) |
| 11 | No regression: full existing example+test suite (gcd, factorial, vm, stack, shapes, vectors, rgb, countdown, lerp, sign, leap, mean, …) stays green + REPL parity | existing goldens + REPL parity (P5) |

## Phases JSON

```json
{"phases":[
  {"phase":1,"focus":"Isolated linear core on bare __spy values, no aggregates: Copy-vs-linear predicate; the test-only __spy builtin primitive (new Type variant lowering as i64) + inline-identity constructor with a print-on-drop destructor; move-tracking on linear locals as a Live/Moved/MaybeMoved lattice threaded &mut through the checker walker (use-after-move naming the move site; divergent-arm move -> MaybeMoved); dup AND over gated on Copy; drop lowers to a Call to the destructor; the disposal check (surplus linear value reuses/extends check_outputs, plus a NEW unconsumed-linear-local pass since locals are not on final_stack; Copy surplus unregressed); a linear value across the Slice 6 back-edge is a located not-supported-yet error (thread tail-position into the walker); and REPL :quit disposes residual linear values LIFO (the REPL-main scope end), while REPL word definitions keep the strict rule. No new Instr/Terminator.","difficulty":"hard","changes":["src/check.rs","src/ir.rs","src/backend/qbe.rs","src/ast.rs","src/repl.rs","tests/phase0.rs","tests/phase1.rs"],"tests":["tests/phase0.rs","tests/phase1.rs"],"exit":"criteria 1 (incl over), 2, 3, 4a/4b/4c, 10a/10b/10c, 12, 14 (REPL :quit disposal), and swap-legal pass as goldens; fmt/clippy/test green; existing examples and tests unregressed"},
  {"phase":2,"focus":"Struct aggregates via destructure-whole (no partial moves): linear-iff-any-field-linear propagation (transitive/nested); S>fi stays ( S -- field ) consuming and drops the non-extracted fields on a linear receiver; S<fi functional update drops the overwritten linear field BEFORE the store (order pinned); synthesized recursive struct drop glue (via the per-type destructor-function mechanism).","changes":["src/check.rs","src/ir.rs","src/backend/qbe.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criteria 5, 5b (transitive), 6, 8, 13 (drop-whole-struct glue order) pass as native goldens; green; no regression"},
  {"phase":3,"focus":"The new S|>fi non-consuming peek word: lexing rule that glues | into a word only when immediately followed by > and preceded by a word char (so | locals | and | Circle clause heads are untouched; re-verify shapes.sth/vm.sth); parser/ast; checker typing as ( S -- S field ) for Copy fields only with a compile error on a linear field; inline non-consuming projection lowering.","changes":["src/lexer.rs","src/parser.rs","src/ast.rs","src/check.rs","src/ir.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criteria 7a (peek keeps struct live, full-stdout one-drop) and 7b (peek on linear field errors) pass; green; no regression"},
  {"phase":4,"focus":"Enums via a synthesized destructor function per linear aggregate type (IrFunc appended during lowering, taking the aggregate as a param), called by drop; NOT block-splitting inside lower_call and NOT reusing lower_clauses. The destructor tag-dispatches (its own Jnz) and drops the active variant's linear payload; a matched clause consumes/drops its exposed payload. Struct case (Phase 2) uses the same mechanism.","changes":["src/check.rs","src/ir.rs","src/backend/qbe.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criteria 9 (runtime tag dispatch, stdout differs per tag) and 9b (matched clause disposes payload) pass; green; no regression"},
  {"phase":5,"focus":"Regression sweep (full existing example+test suite stays green); NO new examples/*.sth (a linear-disposal example would leak __spy into user surface, so it is a tests/phase0.rs golden instead); a REPL parity golden for the within-a-line disposal path; DESIGN.md/ROADMAP consistency. See the open REPL residual-linear decision (held for the user) which may add src/repl.rs and a Phase 1 golden.","changes":["tests/phase0.rs","tests/phase1.rs"],"tests":["tests/phase0.rs","tests/phase1.rs"],"exit":"criterion 11 (no regression) holds; green"}
]}
```
