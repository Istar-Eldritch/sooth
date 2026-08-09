# Phase 4 Slice 7a: quotations as runtime values

Give a non-capturing quotation a `(code, env)` runtime representation so it can be stored in a struct field, put in an array element, returned from a word, and left by two differing `if` arms; and make `call`/`times` on a quotation whose identity the compiler cannot resolve to a literal emit a real **indirect** call. **7a materializes only quotations that capture nothing** (splice ≠ materialize; `map` in `lib/combinators.sth` depends on the late read, so capturing closures are 7b). Force-inlining of quotation-taking words (6a's D2) is untouched.

Base `main` @ `adf13dd`.

## Where the change lands

- **`ir_type_of`'s `Type::Quotation(_)` arm** was `unreachable!` (`src/ir.rs:189`); becomes the new `IrType::Quotation` arm.
- **Type-side variants already exist**: `Type::Quotation(&QuotEffect)` (`src/ast.rs:742`), `PolyType::Quotation` (`src/ast.rs:524`), `struct QuotEffect` (`src/ast.rs:752`), with unification/`apply_subst`. `is_copy`/`poly_is_copy` already treat a quotation effect as Copy (`src/check.rs:4049`). No `Type`/`PolyType` change; only `IrType` gains variants.
- **Env-struct precedent** is `intern_bundle_struct` (`src/ast.rs:433`): synthesize a positional struct, structurally dedup, intern into `Module::structs`. The two-slot value aggregate uses the same construction with a different dedup key.
- **Provenance bit**: `Slot.quot: Option<QuotRef>` (`src/check.rs:121`) is `Some(Known(id))` exactly when the checker can name the literal (`QuotRef`, `src/check.rs:89`). A materialized value carries `quot: None` (erased) with a real `Type::Quotation(eff)` — the signal `call`/`times` read to choose splice vs indirect.
- **The capture check today only rejects, never computes**: `check_literal_against_declared_effect` (`src/check.rs:5603`) flags a literal that consumes a linear enclosing local or leaves a borrow on its exit row; it computes no capture set. 7a adds a boolean predicate beside it.
- **Splice path**: `call`/`times` splice a `Known` literal's body in `lower_call`'s arms (`src/ir.rs:2798`/`:2815`), emitting no `Instr::Call`; the checker inlines every `Known` literal at each call site (D2), so an abstract quotation never reaches lowering today. `check_abstract_quotation_call` (`src/check.rs:5124`) exists but only fires while checking a quotation-taking word's own definition.
- **IR call is symbol-keyed**: `Instr::Call(Option<Value>, String, Vec<Value>)` (`src/ir.rs:922`) → `call $sym(...)` (`src/backend/qbe.rs:904`). No indirect call today; this slice adds one.
- **The alpha-rename walk** (`alpha_rename_locals`/`rename_call`/`rename_terms`, `src/ast.rs:1002`/`:1023`/`:1039`, recursing into nested `TermKind::Quotation` at `:1055`) is the capture predicate's model.

## Locked decisions (binding)

- **D1** — Provenance decides, never a size heuristic or budget. `Known` splices; erased emits an indirect call. No `inline`/`noinline`.
- **D2** — Quotation-taking words stay force-inlined; each/times/filter/while mint no standalone `IrFunc`. An erased quotation composes for free: `table @ each` splices `each`'s skeleton, its abstract parameter binds a runtime value, the inner `call` sees erased provenance and goes indirect.
- **D3** — 7a materializes only quotations that capture nothing. A capturing literal keeps working wherever it is **spliced**; a capturing literal at a **materialization boundary** is a located rejection naming 7b. The line is no-captures / captures.
- **D4** — The materialization boundary is a checked event with its own diagnostic. Identity erases at: store into a struct field, store into an array element, word output, branch join with differing ids. Non-capturing literals mint their `IrFunc` once and become `(code, env)`; capturing ones are rejected naming 7b, reusing `:7026`'s wording. Capture into another quotation is nesting (predicate sees through), reachable only as a capture → same 7b rejection.
- **D5** — One uniform `(code, env)` representation, env unused in 7a (always null). Env slot is **not elided**, keeping 7b additive.
- **D6** — `times` with an erased quotation is allowed (constant stack, one indirect call per iteration). Gets its own golden (T-times).

## Resolved questions (as implemented)

- **Q1 — capture predicate, not the set.** `fn body_captures_enclosing(body, enclosing) -> bool` in `src/check.rs`, beside `:5603`. Walks the body mirroring `rename_terms`, tracking `bound: HashSet<String>` of names introduced via `Bind`; for `TermKind::Call(s)` strips leading `&!`/`&` per `rename_call` and returns `true` on the first stripped name in `enclosing` and not in `bound`; recurses into nested quotations and `if` arms carrying `bound` by value. Pure over the term tree, unit-tested in isolation. Distinct from `check_literal_against_declared_effect` (which answers "illegally consume/borrow", not "reads any enclosing name").
- **Q2 — `IrType::Code` + `IrType::Quotation(QuotSigId)`.** `QuotSigId` indexes a `Module` `Vec<QuotSigLayout>` interned by structural effect equality (dedup like `intern_bundle_struct`). Layout: fixed two-slot aggregate `{ code: Code@0, env: Ptr@WORD_WIDTH }`, size `2*WORD_WIDTH`, align `WORD_WIDTH`, every figure word-width-derived. `code` is a **distinct `IrType::Code`, not `Ptr`** (holds a function identity, preserving backend-neutrality for a future WASM funcref/table-index lowering). On QBE `Code` classifies identically to `Ptr` (`l` in a register, `l` in `qbe_abi_ty`), zero emission change beyond the added arm; spelled `:Q{id}` in ABI positions. Contract: `Code` is a handle — no arithmetic, deref, or cast to/from `Ptr`/int; produced only by `FuncAddr`, consumed only by `CallIndirect` (callee) and aggregate store/load. Env not elided per D5.
- **Q3 — one `Instr` variant + one address op.** Added:
  ```rust
  FuncAddr(Value, String),                        // code-handle from a symbol
  CallIndirect(Option<Value>, Value, Vec<Value>), // indirect call through a Code value
  ```
  QBE emission:
  ```
  FuncAddr(dst,sym)      => "\t{dst} =l copy ${sym}"
  CallIndirect(ret,fp,a) => "\t{ret} ={w} call {fp}({args})"  // args via qbe_abi_ty, only callee token changes
  ```
  Code handle obtained at a `call`/`times` site by `Load(codeptr, quot_base)` (offset 0). No aggregate-arg ABI wrinkle — same by-value classification as a direct callee; one added arm `Code => "l"`.
- **Q4 — a materialized quotation is `Copy` in 7a.** Two non-owning slots (static code address, null env), so `is_copy` derives Copy structurally, matching the existing `Type::Quotation` treatment. No manufactured `drop`. Because 7a rejects every capturing literal at a boundary, no linear quotation value exists in 7a; 7b derives linearity from the env slot's type (`^Env` → linear), so the split is by capture, not a per-slice flip.
- **Q5 — dispatch table: one decode clause, uniform match-free entries, indirect-called.** Sooth enum variants have no getter/setter; elimination is clause-style and must be exhaustive (`enum_generated_sigs` `:2317`, `:3329`). So a per-entry "extract my variant's payload" shape would need a full clause match inside every entry (a placebo). Resolved: a single `decode` clause word runs once per fetched instruction — inspects the tag, yields the opcode index, pushes any immediate onto the `Vm` operand stack. Table is `[ [ Vm -- Vm ] N ]` of uniform match-free handlers reading operands from the `Vm` stack. Per instruction: `decode` → index → `call` (indirect). Decoder, not dispatcher; executable behavior lives in the indirect-called table.

## Requirements

**Representation and backend (Phase 1)**
- **R1.** `IrType::Code` and `IrType::Quotation(QuotSigId)` added (`src/ir.rs`), both Copy; `Code` distinct from `Ptr`; `QuotSigId` indexes a `Module` signature table interned by structural `QuotEffect` equality.
- **R2.** Layout: two-slot `{ code: Code@0, env: Ptr@WORD_WIDTH }`, size `2*WORD_WIDTH`, align `WORD_WIDTH`, word-width-derived; `:Q{id}` in ABI positions, `l` in a register (`Code` classifies as `Ptr`, one added arm, no emission change).
- **R3.** `ir_type_of`'s `Type::Quotation(eff)` arm (`src/ir.rs:189`) replaces the `unreachable!` with interning `eff` and returning `IrType::Quotation(id)`.
- **R4.** `Instr::FuncAddr`/`CallIndirect` added with the Q3 QBE emission; argument spelling reuses `qbe_abi_ty` plus the new `Code => "l"` arm.
- **R5.** `is_copy` returns Copy for `IrType::Quotation` and `IrType::Code`. No manufactured `drop`.

**The capture predicate (Phase 2)**
- **R6.** `body_captures_enclosing` as in Q1. Pure over the term tree; unit-tested in isolation.

**Materialization boundaries (Phases 2–3)**
- **R7.** A boundary materializes a non-capturing `Known` literal and rejects a capturing one: (i) run `body_captures_enclosing`; (ii) if it captures, raise R12; (iii) else confirm against the boundary's expected `Type::Quotation(eff)` via `check_literal_against_declared_effect` (`:5603`), set slot `ty: Type::Quotation(eff), quot: None`.
- **R8.** Boundaries lifted at three guards:
  1. **Declaration-time legality** — `audit_quotation_type_registries` (`check.rs:1105`) and the word-output/input audit in `audit_word_quotation_positions` (`:1150`) gain a carve-out so a `type:`/`array:` declaration or a word's declared output may name a `Type::Quotation` field/element/output. Owned-cell payloads and reference referents stay rejected (not D4 boundaries).
  2. **Construction legality** — `reject_quotation_argument`'s call site in the generic monomorphic-call argument loop (`check.rs:6352`) gains a carve-out: when the declared parameter `want` is `Type::Quotation(eff)`, skip the unconditional `found.quot.is_some()` rejection and run R7. This is the site a struct constructor call reaches; gated on `want`'s type (also covers generated setters and ordinary user words). An `extern` argument cannot reach it structurally — `audit_quotation_type_positions`'s `module.externs` loop (`:1084`–`:1090`) rejects a `Type::Quotation` at any extern position unconditionally, so `want` can never be `Type::Quotation` for an extern callee. The other three call sites (`:4130`/`:5441`/`:5558`) apply the same `want`-typed gate uniformly; a type variable can never resolve to `Type::Quotation` in `want` on the poly path.
  3. **Mutation legality** — `reject_quotation_stored` (`check.rs:7024`, `fill`/`!`/`+!`) gains the same carve-out for array-element and struct-field-via-reference paths.

  The clause-body rejection `:1251` is **not** lifted (out of scope). A **generic container's constructor** (poly struct field of type variable monomorphized to a quotation) is a boundary this carve-out does not reach (poly path disjoint from `:6352`); deferred alongside polymorphic quotation values — every exit use here is monomorphic.
- **R9.** Lowering a materialization (`src/ir.rs`): mint **one** `IrFunc` per distinct `QuotId` (dedup by id; boundaries sharing a literal share it), signature = the quotation effect (no env param in 7a), symbol a stable mangle `{enclosing}__quot{n}`. Build `(code, env)`: `FuncAddr(code, sym)`, `env = Const 0`, stored into a fresh two-slot aggregate. Only quotation *literals* mint an `IrFunc`; quotation-taking words still mint none (D2).
- **R10.** `call`/`times` on an **erased** quotation: checker accepts (reusing `check_abstract_quotation_call` `:5124` over `eff`); lowering (`:2798`/`:2815`) branches on provenance — a `Known` marker splices byte-identically, an erased value `Load`s the code slot and emits `CallIndirect` (D1); `times` drives one indirect call per iteration inside the existing constant-stack loop (D6). The non-quotation rejections `:5108`/`:5112`/`:5743`/`:5747` stay.

**Branch-join materialization (Phase 3)**
- **R11.** The join (`src/check.rs`, `(t_then.quot, t_else.quot)` match near `:6467`): when both arms leave a quotation, materialize each against an expected `Type::Quotation(eff)` **threaded into the `if`** from the enclosing declared context. Both non-capturing and effects unifying → join yields a runtime value (`quot: None`); equal-`Known`-ids fast path (`a == b`) still forwards a marker with no `Phi` (splice preserved). Either arm capturing → R12, **checked before the id/expected-type resolution** (ordering pin: a capturing arm always raises R12, never falls through to `different_quotations_at_join_error`). No expected quotation type → keep `different_quotations_at_join_error` (`:7053`), reworded to also say "give the quotation a declared type" (no bare-literal effect inference added; `:5598`). `quotation_versus_value_at_join_error` (`:7064`) stays.

**Diagnostics (Phases 2–3)**
- **R12.** One `capturing_quotation_error(ctx, span, boundary)` reusing `:7026`'s vocabulary, producing exactly `error: a capturing quotation cannot {boundary} (capturing closures are slice 7b) (line {n})` with `boundary` ∈ {`be stored`, `be an array element`, `be returned`, `be left on a branch`}. Each has an exact-message test.

**Regression protection (R13a Phase 1, verified Phase 3)**
- **R13.** Every 6a–6f combinator golden asserting a spliced tight loop with no per-element `Instr::Call` stays green and bit-identical. The guarantee lives in two disjoint extractors in `src/ir.rs`:
  1. `call_symbols` (`:4343`), used by `each_lowers_to_a_loop_not_a_per_element_call` (`:7726`) and `while_lowers_to_a_back_edge_not_an_infinite_splice` (`:7783`); matches only `Instr::Call(_, sym, _)`.
  2. Eleven independent `count(f, |i| matches!(i, Instr::Call(..)))` closures. The two load-bearing for this slice are `call_of_literal_emits_no_call_instr` (`:4715`) and `times_lowers_to_a_loop_header_not_a_per_iteration_call` (`:4750`) — their programs are the only ones that can contain a `call`/`times` on a quotation. The other nine (`:4876`/`:4896`/`:5496`/`:6507`/`:6558`/`:6575`/`:6591`/`:6612`/`:6924`) contain no quotation.
  - Behavioural witnesses in `tests/phase4_combinators.rs` (`:1188`/`:1410`) are equivalence sweeps against a hand-threaded twin, not shape assertions; they delegate the structural guarantee to the two `call_symbols` units. Not to be "strengthened".

  **The pin is a placebo until both extractors are widened** — neither can see `Instr::CallIndirect` (distinct variant, mapped to `None`/`false`), so a splice regressing into `CallIndirect` ships silently, including at the two tests closest to R10's new branch.

  **R13a (blocking, Phase 1).** Widen both before `CallIndirect` exists in any lowering path:
  1. Extend `call_symbols` (or a companion) to see `CallIndirect`; update `:7726`/`:7783` to assert absence of both variants, reporting indirect calls as a distinct count (`unexpected calls: [...] + N indirect`), not a fabricated symbol.
  2. Introduce `fn is_call_instr(i: &Instr) -> bool { matches!(i, Instr::Call(..) | Instr::CallIndirect(..)) }` and replace all eleven inline closures with it — mechanical, uniform, all eleven (a reader cannot tell a widened site from an unwidened one).

## Sanctioned files

- `src/ir.rs` — `IrType::Quotation`/`QuotSigId` + signature table (R1); layout (R2); `ir_type_of` arm (R3); `Instr::FuncAddr`/`CallIndirect` (R4); materialization lowering + one-`IrFunc`-per-id (R9); `call`/`times` provenance branch (R10); unit tests.
- `src/backend/qbe.rs` — `FuncAddr`/`CallIndirect` emission + `:Q` ABI spelling (R4); unit tests.
- `src/ast.rs` — `QuotSig` interning helper if it lands here (mirrors `intern_bundle_struct`); no other change.
- `src/check.rs` — `body_captures_enclosing` (R6); boundary carve-outs + erased-slot production (R7/R8); erased `call`/`times` acceptance (R10); join materialization + expected-type threading (R11); `capturing_quotation_error` (R12); `is_copy` arm (R5); unit tests.
- `src/repl.rs` — touched for the new IR variants (Phase 1/2).
- `tests/phase4_quotations.rs` — new golden file with its own `check_error`/`check_src`/`run_src` harness copies (per-file convention).
- `tests/phase4_combinators.rs` — R13a widening.
- `examples/vm_table.sth` — the dogfood (new); `examples/vm.sth` retained unchanged as the parity oracle.
- `ROADMAP.md` — mark 7a implemented.

No other files.

## Exit criteria (golden tests)

| ID | Test | Kind | Phase | Source in → expected out |
|----|------|------|-------|--------------------------|
| T-irtype | `ir_type_of_quotation_is_two_slot_aggregate` (`src/ir.rs`) | unit | 1 | `Type::Quotation(eff)` → `IrType::Quotation`, size `2*WORD_WIDTH`, `code`@0/`env`@word |
| T-qbe-addr | `qbe_emits_func_addr_as_copy_of_symbol` (`src/backend/qbe.rs`) | unit+QBE | 1 | `FuncAddr(v,"f")` → `%… =l copy $f` |
| T-qbe-ind | `qbe_emits_indirect_call_through_value` (`src/backend/qbe.rs`) | unit+QBE | 1 | `CallIndirect(Some(r),fp,[a:Q])` → `%r =… call %fp(:Q %a)` |
| T-field | `quotation_stored_in_struct_field_compiles_and_calls` | run | 2 | `type: Holder q [ i64 -- i64 ] ;` build, `Holder>q call` on `4` → `5`; path emits `CallIndirect` |
| T-array | `quotation_in_array_element_indirect_calls` | run | 2 | array `[ [i64 -- i64] 2 ]`, index one, `call` on `4` → `5`; `CallIndirect` present |
| T-return | `quotation_returned_from_word_indirect_calls` | run | 2 | `: mk ( -- [ i64 -- i64 ] ) [ 1 + ] ;` `mk 4 swap call .` → `5`; `CallIndirect` present |
| T-cap-store | `capturing_literal_stored_is_error_naming_7b` | reject | 2 | stored capturing literal → exact `a capturing quotation cannot be stored (capturing closures are slice 7b)` via `assert_eq!` (distinct from `:7024`'s "escaping quotations are slice 7") |
| T-splice-cap | `capturing_literal_spliced_still_works` | run | 2 | capturing literal at a direct `call` → runs as today (D3) |
| T-join | `two_differing_quotation_arms_materialize_and_call` | run | 3 | `: pick ( bool -- [ i64 -- i64 ] ) if [ 1 + ] else [ 2 + ] end ;` `true pick`→`5`, `false pick`→`6`; `CallIndirect` present |
| T-join-same | `same_quotation_both_arms_still_splices` | run+IR | 3 | one literal used in both arms, called on `5` → `6`, no `CallIndirect` (equal ids forward the marker) |
| T-cap-join | `capturing_literal_at_join_is_error_naming_7b` | reject | 3 | exact `a capturing quotation cannot be left on a branch (capturing closures are slice 7b)`, `assert_eq!` (ordering pin: fires before `different_quotations_at_join_error`) |
| T-times | `times_over_erased_quotation_runs_constant_stack` | run+IR | 3 | erased quotation via `times` → correct result, header+back-edge with one `CallIndirect` in the body, constant stack (D6) |
| T-reg | ~13 existing `src/ir.rs` functions widened per R13a (`:7726`, `:7783`, and the eleven `count(.., is_call_instr)` sites) | IR | 1+3 | widened Phase 1, all pass unchanged through Phase 3 |
| T-dogfood | `vm_table_dispatch_matches_clause_version` | run+IR | 4 | `examples/vm_table.sth` stdout == `examples/vm.sth`; path emits `CallIndirect`; no handler quotation contains a clause match (Q5) |
| T-roadmap | ROADMAP 7a marked implemented | doc | 4 | prose exit line; no test |

## Load-bearing / mutation-test-required criteria

Prove each can fail by reverting its guard in a **throwaway copy** of the compiler (not the shared worktree).

- **M1 (R10 provenance — T-field/T-return + T-join-same).** Force always-splice: T-field/T-return must fail (no symbol to splice for erased). Force always-`CallIndirect`: T-join-same must go red (a `Known` marker wrongly indirect-called). Proves provenance, not size, is the switch (D1).
- **M2 (R6 predicate — T-cap-store).** Force `body_captures_enclosing` `false`: T-cap-store/T-cap-join go red. Force `true`: T-field/T-return go red. Both directions wired.
- **M3 (R6 nested reach — T-cap-store variant).** Drop the recursion into nested quotation/`if` arms: `capturing_through_nested_quotation_is_error` goes red. Proves D4's capture-into-another-quotation reach.
- **M4 (R13a — pins see `CallIndirect`, not just `Call`).** Mutate a `call`/`times`-on-a-literal lowering site to emit `Instr::CallIndirect` in place of the splice (not `Instr::Call` — already caught pre-widening, proves nothing). `call_of_literal_emits_no_call_instr` (`:4715`) and `times_lowers_...` (`:4750`) must flip red only after R13a lands, and pass (placebo) against the unwidened predicate.
- **M5 (Q5 dogfood — T-dogfood).** Inline a handler back into a clause dispatch: T-dogfood must go red on "no handler contains a clause match". Proves the table is genuinely indirect-called and the decode/execute split is real.

## Out of scope

- Capturing closures (downward/upward) and `^Env` (7b, gated on 6f).
- The capture-set analysis — 7a builds only the boolean predicate.
- Inline budgets, `inline`/`noinline`, sinking a `call` into branch arms.
- The clause-body rejection (`check.rs:1251`).
- Any change to what splicing means — splice comes out bit-identical, pinned by R13/T-reg.
- Standalone bare-literal effect inference — none exists (`src/check.rs:5598`), none added; every boundary takes its effect from a declared context.
- Polymorphic quotation *values* — 7a materializes concrete-effect quotations only; a still-poly effect at a boundary stays under the existing R7a rejection.

## Phased delivery (each green under `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

- **Phase 1 — representation and backend.** R1/R2/R3/R4/R5 + R13a widening before any lowering can emit `CallIndirect`. Hand-built-IR unit tests (T-irtype/T-qbe-addr/T-qbe-ind).
- **Phase 2 — materialization at store/field/array/output + indirect call + capture predicate.** R6; R7/R8 carve-outs + erased-slot production; R9; R10 provenance branch; R12. Goldens T-field/T-array/T-return/T-cap-store/T-splice-cap. Drive M1/M2/M3.
- **Phase 3 — branch-join + `times`-erased + regression pins.** R11 expected-type threading; T-join/T-join-same/T-cap-join/T-times; R13 pins (T-reg). Drive M4.
- **Phase 4 — dogfood + ROADMAP.** `examples/vm_table.sth` (decode clause + uniform match-free handler table); parity golden T-dogfood. Drive M5. Mark ROADMAP 7a implemented (T-roadmap).

## Implementation status

All four phases implemented and committed:
- **Phase 1** (`e917ff36`): `src/backend/qbe.rs`, `src/ir.rs`, `src/repl.rs`.
- **Phase 2** (`6d278189`, review `3cc0f822`): `src/backend/qbe.rs`, `src/check.rs`, `src/ir.rs`, `src/repl.rs`, `tests/phase4_combinators.rs`, `tests/phase4_quotations.rs`.
- **Phase 3** (`6f9132f0`, review `9dd72988`): `src/check.rs`, `src/ir.rs`, `tests/phase4_quotations.rs`.
- **Phase 4** (`d8d8e08c`): `ROADMAP.md`, `examples/vm_table.sth`, `tests/phase4_quotations.rs`.
