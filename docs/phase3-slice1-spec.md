# Phase 3 Slice 1 — Linear analysis + move + `dup`-gate + explicit `drop` (shipped)

## What it is

First cut of the **linear spine**, isolated from heap: values move by default, a second use of
a moved value is a compile error, `dup`/`over` are gated on `Copy`, `drop` runs a destructor,
and forgetting to dispose a linear value is a compile error (no auto-drop, no join
reconciliation). Proven on the test-only builtin drop-spy `__spy`, whose destructor prints, so
drop count/order/timing are golden-observable. Aggregates covered by destructure-whole (no
partial moves). No heap, refs, RC, or fds. Design locked in `docs/phase3-slice1-brief.md`
(D1-D8); target semantics in `DESIGN.md` "## The linear spine".

## Requirements as shipped

- **R1/R2 (D1)** — Exactly-once, no auto-drop. Every value is accounted for by the signature
  or an explicit `drop`; surplus at scope end errors for Copy and linear alike. Asymmetry is
  only that `drop` runs a destructor on linear values (no-op on Copy) and Copy values carry no
  ownership obligation under `dup`/`drop`.
- **R3 (D2)** — Mentioning a linear local moves it; a second mention is `use after move`
  naming the move site. No mid-body rebinding.
- **R4 (D3)** — `dup`/`over` on non-`Copy` is an error in the DESIGN.md form ("cannot `dup` a
  value of type X; X is linear …"). `swap`/`rot` stay legal on linear values.
- **R5/R6 (D4/D5)** — `drop` is universal disposal. `__spy` is a `Type` variant lowering as its
  own `IrType::Spy` variant (distinct from a plain `i64`, so drop emission can tell a spy
  apart from an ordinary integer); constructor `( i64 -- __spy )` lowers inline as identity; its
  compiler-known destructor prints `drop <tag>` via the existing print path. Convention-fenced
  out of user surface; dissolves once `drop` is overridable (Phase 4 of the roadmap).
- **R7/R8 (D6)** — A struct/enum is linear iff any field/variant payload is linear
  (transitive). `dup`/`over` on a linear aggregate errors; `S` moves fields in, `S>` consumes
  and pushes all fields.
- **R9-R11 (D6)** — `S>fi` keeps its consuming `( S -- field )` effect and drops the
  non-extracted fields on a linear receiver. `S|>fi` is a new non-consuming `( S -- S field )`
  peek, Copy fields only (linear field → compile error, workaround `S>`). `S<fi` drops the
  overwritten linear field **before** the store; other fields transfer via the existing
  blit-move.
- **R12 (D6)** — Drop glue is a **synthesized destructor `IrFunc` per linear aggregate type**,
  taking the aggregate as a param: structs drop linear fields in declaration order; enums
  tag-dispatch (own `Jnz`, via the extracted `dispatch_on_tag`) and drop the active variant's
  payload. Every `drop` stays a plain `Call`.
- **R13/R14 (D7)** — No compensating drops, ever. Surplus linear on the stack reuses
  `check_outputs`; a **new** unconsumed-linear-local pass covers locals (not on
  `final_stack`). Move-state is a `Live`/`Moved(site)`/`MaybeMoved(site)` lattice threaded
  `&mut` through the checker walker; joins keep equal states and yield `MaybeMoved` on
  disagreement. Later use of `Moved`/`MaybeMoved` → use-after-move at the use; `Live`/
  `MaybeMoved` at scope end → unconsumed-linear at scope end.
- **R15 (D8)** — A linear value live across the self-tail-call back-edge is a located
  `linear values across a loop are not supported yet` error (tail-position threaded into the
  walker). Out of scope: partial/field moves, user-overridable `drop`, heap/refs/RC/fds.
- **R16 (IR)** — No new `Instr`/`Terminator`. `.` on a linear value is rejected in the
  checker's printable-scalar path. `close`/`free` deferred.

## Delivered phases

1. **Isolated linear core on bare `__spy` `[hard]`** — `is_copy(Type, structs, enums)`
   predicate; the `__spy` type/constructor/destructor; the move-state lattice in the walker;
   `dup`/`over` gate; `drop` lowering; surplus + unconsumed-local disposal checks; located
   back-edge guard; REPL `:quit` LIFO disposal of residual linear values.
   *Files*: `src/{check,ir,ast,repl}.rs`, `src/backend/qbe.rs`, `tests/phase{0,1}.rs`.
2. **Struct aggregates via destructure-whole** — transitive linearity by field recursion;
   `S>fi` drop-the-rest; `S<fi` drop-on-overwrite before the store; synthesized struct
   destructors, also registered in REPL modules; **linear array elements rejected**.
   *Files*: `src/{check,ir,repl}.rs`, `tests/phase{0,1}.rs`, spec doc.
3. **`S|>fi` non-consuming peek** — lexer glues `|` into a word only when immediately followed
   by `>` and preceded by a word char, leaving `| locals |` and `| Circle` clause heads alone
   (`shapes.sth`/`vm.sth` re-verified); parser/ast/checker typing (Copy fields only); inline
   projection lowering. *Files*: `src/{lexer,check,ir}.rs`, `tests/phase0.rs`.
4. **Enums: tag-dispatched drop glue** — variant-payload linearity; synthesized destructor
   tag-dispatches via `dispatch_on_tag`; matched clauses consume their exposed payload; REPL
   `:quit` disposes residual linear enums too. *Files*: `src/{check,ir,repl}.rs`,
   `tests/phase{0,1}.rs`.
5. **Regression sweep + docs** — full existing example/test suite green; **no new
   `examples/*.sth`** (an example would leak `__spy` into user surface, so disposal is a
   `tests/phase0.rs` golden); REPL parity goldens; `ROADMAP.md` linear framing updated.

## REPL residual linear values (resolved)

The REPL session is the "main" word: the residual typed stack (`self.types`) is its working
stack and `:quit` is its scope end. Since the body is never complete, the compile-time "you
forgot to dispose X" proof can never fire, so at `:quit` the REPL runs the destructor of every
residual linear value, top-of-stack first. Exactly-once holds (R1); only the *compile error* is
relaxed. Word definitions typed at the REPL keep the strict rule.

## Criterion → test map

All goldens are runnable native binaries (`tests/phase0.rs`) or REPL sessions
(`tests/phase1.rs`), never IL-string assertions. Negative goldens assert the diagnostic
substring **and** the backticked type name; criterion 2 also asserts the move site, 12 that
the error is located. Drop-observing goldens use full-stdout `assert_eq!`, so count and order
are proven.

| # | Criterion | Test (phase) |
|---|---|---|
| 1 / 1b | `dup` / `over` on a bare spy is an error | `dup_of_linear_value_is_error`, `over_of_linear_value_is_error` (P1) |
| 2 | Use-after-move names the move site | `use_after_move_of_linear_local_is_error` (P1) |
| 3 | Destructor runs exactly once at explicit `drop` (full-stdout equality) | `explicit_drop_runs_destructor_once` (P1) |
| 4a/4b/4c | Surplus linear on stack errors; unconsumed local errors; surplus **Copy** keeps the arity error | `surplus_linear_on_stack_is_error`, `unconsumed_linear_local_is_error`, `surplus_copy_value_keeps_existing_error` (P1) |
| swap | `swap`/`rot` on linear values allowed (over-broad-gate guard) | `swap_of_linear_values_is_allowed` (P1) |
| 10a/10b/10c | Both arms consume → one drop; one arm + later use → use-after-move; one arm, unused → unconsumed at scope end | `both_arms_consume_linear_ok`, `divergent_arm_use_is_error`, `divergent_arm_unconsumed_is_error` (P1) |
| 12 | Linear across a back-edge is a located not-yet error; Copy loop unaffected | `linear_across_loop_back_edge_is_located_error` + `copy_loop_still_compiles` (P1) |
| 5 / 5b | `S>` drops each field in order (>=2 distinctly-tagged spies); linearity is transitive through nesting | `destructure_whole_drops_each_field`, `nested_struct_is_linear_transitively` (P2) |
| 6 / 8 / 13 | `S>fi` drops the rest; `S<fi` drops the overwritten field before the store; whole-struct `drop` runs glue in declaration order (>=2 distinctly-tagged spies) | `get_field_drops_the_rest_on_linear_struct`, `set_field_drops_overwritten_linear_field`, `drop_of_linear_struct_runs_field_glue_in_declaration_order` (P2) |
| 7a / 7b | Peek keeps the struct live (peek twice then dispose -> exactly one drop line); peek on a linear field errors | `peek_copy_field_keeps_struct`, `peek_linear_field_is_error` (P3) |
| 9 / 9b | Runtime tag dispatch drops the active payload (built at runtime behind an `if` with >=2 differently-shaped variants; stdout differs per tag); matched clause disposes its payload | `drop_of_linear_enum_dispatches_on_tag`, `clause_body_disposes_linear_payload` (P4) |
| 14 | `:quit` disposes residual linear values LIFO (bare spy, struct, enum); an explicit earlier `drop` prints once | `repl_quit_disposes_residual_linear{,_struct,_enum}` + `repl_explicit_drop_not_redisposed_at_quit` (P1/P4) |
| 11 | No regression across the existing example+test suite + REPL parity | existing goldens (P5) |
