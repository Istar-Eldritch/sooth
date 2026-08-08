# Phase 4 Slice 7a: quotations as runtime values (spec)

Give a non-capturing quotation a `(code, env)` runtime representation so it can be stored in a
struct field, put in an array element, returned from a word, and left by two differing branches
of an `if`; and make `call`/`times` on a quotation whose identity the compiler cannot resolve to
a literal emit a real **indirect** call. **7a materializes only quotations that capture nothing.**
Recon 9/10 of the brief measured that *splice* (a late read at the call site) and *materialize*
(a snapshot at the literal) are different semantics, and that `map` in `lib/combinators.sth`
depends on the late read; capturing closures are therefore deferred to 7b. Force-inlining of
quotation-taking words (6a's D2) survives untouched.

Base `main` @ `adf13dd`. The brief (`docs/phase4-slice7a-brief.md`) is the recon of record; this
spec carries its six locked decisions D1–D6 as binding constraints, resolves its five open
questions Q1–Q5 concretely, and states exit criteria in the CLAUDE.md golden-test style. All line
citations were re-verified against `main` before writing; corrections to the brief's anchors are
called out inline (the brief cited `check_abstract_quotation_call` at `:6096` and
`check_literal_against_declared_effect` at `:5599`; the real lines are `:5124` and `:5603`).

## Where the change lands (verified against current `main`)

- **The `unreachable!` is the type-lowering seam.** `ir_type_of`'s `Type::Quotation(_)` arm is
  `unreachable!("a quotation type has no IrType this slice ...")` (`src/ir.rs:189`); its own
  comment names this slice as the lift. This becomes the new `IrType::Quotation` arm — additive at
  a known point (recon 2).
- **The type-side variant already exists.** `Type::Quotation(&'static QuotEffect)` (`src/ast.rs:742`),
  `PolyType::Quotation(Vec<PolyType>, Vec<PolyType>)` (`src/ast.rs:524`), and `struct QuotEffect`
  (`src/ast.rs:752`) exist with unification and `apply_subst` following (6a). `is_copy`/`poly_is_copy`
  already treat a quotation effect as **Copy** (`src/check.rs:4046`, "a quotation effect is always
  Copy (D3)"). No `Type`/`PolyType` change is needed; only `IrType` gains a variant (Q2/Q4).
- **The env-struct precedent is `intern_bundle_struct`** (`src/ast.rs:433`): it synthesizes a
  positional struct, structurally dedups it, and interns it into `Module::structs` before the
  layout pass. A quotation's two-slot value aggregate is the same construction with a different
  dedup key (recon 3).
- **Provenance is already the right bit.** `Slot.quot: Option<QuotRef>` (`src/check.rs:121`) is
  `Some(Known(id))` exactly when the checker can name the literal; `QuotRef` is a one-variant enum
  (`src/check.rs:89`) left in the shape a second case would arrive in. A materialized value carries
  `quot: None` (erased) with a real `Type::Quotation(eff)` — the signal `call`/`times` read to
  choose splice vs indirect.
- **The capture check today only rejects, never computes.** `check_literal_against_declared_effect`
  (`src/check.rs:5603`) flags a literal that consumes a linear enclosing local or leaves a borrow
  of an enclosing place on its exit row; it computes no capture set (recon 7). This spec adds a
  **boolean predicate** beside it (Q1).
- **The eleven slice-7 rejection sites** (all re-verified verbatim): `check.rs:1239`/`:1259` (R7a,
  a quotation type in any position but a direct word parameter), `:1251` (a quotation-taking word
  with a clause body — **not lifted**, D4/out-of-scope), `:2481` (a quotation on a REPL residual),
  `:5108`/`:5112` (`call` on a non-quotation), `:5743`/`:5747` (`times` on a non-quotation),
  `:7013` (operator operand), `:7026` (stored), `:7041` (passed to a word other than `call`/`times`),
  `:7053`/`:7064` (the two branch-join cases), plus `ir.rs:189`.
- **The join merge is a two-`QuotRef` compare** (`src/check.rs`, the `(t_then.quot, t_else.quot)`
  match near `:6467`): equal `Known` ids forward the marker; differing ids raise
  `different_quotations_at_join_error` (`:7053`); quotation-vs-non-quotation raises
  `quotation_versus_value_at_join_error` (`:7064`). The type-only `t_then.ty != t_else.ty` check
  follows immediately after.
- **The splice path that must not move.** `call`/`times` splice a `Known` literal's body in
  `lower_call`'s `"call"`/`"times"` arms (`src/ir.rs:2798`/`:2815`), emitting no `Instr::Call`; the
  checker inlines a `Known` literal at every real call site (D2, 6a) so an abstract quotation never
  reaches lowering today (recon 5). The abstract checker path
  `check_abstract_quotation_call` (`src/check.rs:5124`) exists but only fires while checking a
  quotation-taking word's own definition.
- **The IR call today is symbol-keyed.** `Instr::Call(Option<Value>, String, Vec<Value>)`
  (`src/ir.rs:922`), emitted as `call $sym(...)` in `src/backend/qbe.rs:904`. There is **no**
  indirect call (Q3); this slice adds one.
- **The alpha-rename walk is the predicate's model.** `alpha_rename_locals` (`src/ast.rs:1002`),
  `rename_call` (`src/ast.rs:1023`, strips a leading `&!`/`&`), `rename_terms` (`src/ast.rs:1039`,
  recurses into nested `TermKind::Quotation` at `:1055`). The capture predicate mirrors this walk.

## Locked decisions carried from the brief (binding constraints)

- **D1 — Provenance decides, never a size heuristic or budget.** `call`/`times` on a `Known`
  quotation splices, exactly as today; on one whose identity has erased, it emits an indirect call.
  No inline budget, no `inline`/`noinline` (out of scope).
- **D2 — Quotation-taking words stay force-inlined; 6a's D2 survives.** `each`/`times`/`filter`/
  `while` mint no standalone `IrFunc`; every call site splices them. An erased quotation composes
  for free: `table @ each` splices `each`'s loop skeleton, its abstract parameter binds a runtime
  value, and the inner `call` sees erased provenance and goes indirect. No `IrFunc`-for-`each`.
- **D3 — 7a materializes only quotations that capture nothing.** A capturing literal keeps working
  exactly as today wherever it is **spliced**; a capturing literal reaching a **materialization
  boundary** is a located rejection naming 7b (D4). The 7a/7b line is *no captures / captures*.
- **D4 — The materialization boundary is a checked event with its own diagnostic.** Identity erases
  at: a store into a struct field, a store into an array element, a word output, and a branch join
  with differing ids. At each, a non-capturing literal mints its `IrFunc` once and becomes a
  `(code, env)` value; a capturing one is rejected naming 7b, reusing `:7026`'s wording shape.
  (Capture *into another quotation* is nesting, handled by the predicate seeing through nested
  quotations, and is only reachable as a capture — so it lands as the same 7b rejection, never a
  new value form in 7a.)
- **D5 — One uniform `(code, env)` representation, env unused in 7a.** Building the pair now rather
  than a bare code pointer keeps 7b additive (it fills the env in) instead of a representation
  change later. The env slot is **not elided** (Q2).
- **D6 — `times` with an erased quotation is allowed, not rejected.** One indirect call per
  iteration is still constant stack. `times` gets its own golden (T-times), not `call`'s.

## Resolved open questions

### Q1 — The capture-set analysis: build the cheap boolean predicate now; the set is 7b's

**Decision:** add a predicate `fn body_captures_enclosing(body: &[Term], enclosing: &HashSet<String>)
-> bool` in `src/check.rs`, beside `check_literal_against_declared_effect` (`:5603`). It is strictly
less work than 7b's capture *set* and is all a materialization boundary needs (yes/no). Reject
"build the set now": 7a needs no set, and a set carries capture *kinds* (linear vs borrow vs
aggregate) that only matter once the env is populated (7b, gated on 6f).

The predicate mirrors `rename_terms` (`src/ast.rs:1039`): walk the body with a running
`bound: HashSet<String>` of names the body itself introduces via `Bind`; for a `TermKind::Call(s)`,
strip a leading `&!`/`&` exactly as `rename_call` does (`src/ast.rs:1023`) and return `true` on the
first stripped name that is **in `enclosing` and not in `bound`**; recurse into nested
`TermKind::Quotation` and `TermKind::If` arms carrying `bound` by value (so a nested quotation that
reads an outer name still counts, per D4's capture-into-another-quotation case). `enclosing` is the
set of local names live at the literal, taken from the checker's current `Scope`. The predicate
never inspects `Slot`/`Deriv` state, so it is pure over the term tree and testable in isolation.

**Why this is not `check_literal_against_declared_effect`'s job:** that function answers "does this
literal *illegally* consume/borrow an enclosing place under the D3 rule" (recon 8, the
declared-parameter regime). The predicate answers the weaker "does it *read* any enclosing name at
all," including a by-value read of a `Copy` scalar that the D3 check ignores. They are different
questions; the predicate is additive.

### Q2 — `IrType` and layout: a distinct `IrType::Quotation`, a two-pointer-slot aggregate, env not elided

**Decision:** add `IrType::Quotation(QuotSigId)` (`src/ir.rs`), keyed by a small `Copy`
`QuotSigId` into a new `Module`-level `Vec<QuotSigLayout>` interned by structural effect equality
(the same construction and dedup discipline as `intern_bundle_struct`, `src/ast.rs:433`). Its
layout is a **fixed two-slot aggregate `{ code: Ptr, env: Ptr }`**: `code` at offset 0, `env` at
offset `WORD_WIDTH`, size `2 * WORD_WIDTH`, align `WORD_WIDTH` — every offset/size derived from the
word-width parameter, never hardcoded (load-bearing invariant: backend-neutral IR, `Ptr` opaque).
It is spelled `:Q{id}` in ABI positions and `l` in a register, identical to `Struct`/`Enum`/`Array`,
so aggregate ABI classification is reused wholesale.

**Env is not elided (D5).** In 7a the `env` slot is always the null pointer; 7b fills it with a
pointer to a synthesized env struct. Pricing the elision the brief asked for: eliding env now would
make 7a's quotation a bare `l` code pointer, then 7b would have to widen every quotation value from
one slot to two — a representation change touching layout, ABI spelling, and every store/load of a
quotation, precisely the churn D5 exists to avoid. The measured cost of *not* eliding is that a 7a
quotation is an aggregate (a pointer to two-slot storage, `:Q` by-value through the C-ABI) rather
than a register-resident pointer: one extra word of storage and one extra store (the null env) per
materialization. That cost is accepted per D5. A `Type::Quotation` maps to `IrType::Quotation`
through the new `ir_type_of` arm at `src/ir.rs:189`, interning the `QuotEffect` into the module
signature table.

### Q3 — The backend has no indirect call today; add one `Instr` variant and one address op

**Decision (read against `src/ir.rs:922` and `src/backend/qbe.rs:904`, not assumed):** `Instr::Call`
is symbol-keyed (`call $sym(...)`). Add two IR instructions:

```rust
// src/ir.rs, in `enum Instr`
/// The address of a (materialized) function symbol as an opaque `Ptr` value.
/// Emitted at a materialization boundary to fill a quotation's `code` slot.
FuncAddr(Value, String),
/// An indirect call through a code-pointer `Value` (the quotation's `code`
/// slot, already `Load`ed). Mirrors `Call` but the callee is a value, not a
/// symbol. `env` is not passed in 7a (a non-capturing callee has no env
/// parameter); 7b adds the env argument here.
CallIndirect(Option<Value>, Value, Vec<Value>),
```

**QBE emission (`src/backend/qbe.rs`):**

```rust
Instr::FuncAddr(dst, sym) => {
    // A global symbol used as a value is an `l` (pointer); `copy $sym`
    // materializes its address into a temporary.
    writeln!(out, "\t{} =l copy ${}", val(*dst), qbe_name(sym))
}
Instr::CallIndirect(ret, fp, args) => {
    // Argument spelling is identical to `Instr::Call` (same ABI
    // classification): `qbe_abi_ty` yields `:S`/`l` per arg. Only the callee
    // token changes from `$sym` to the `%tmp` holding the code pointer.
    let a: Vec<String> = args.iter()
        .map(|x| format!("{} {}", qbe_abi_ty(ty_of(value_types, *x), layouts), val(*x)))
        .collect();
    match ret {
        Some(r) => {
            let w = qbe_abi_ty(ty_of(value_types, *r), layouts);
            writeln!(out, "\t{} ={w} call {}({})", val(*r), val(*fp), a.join(", "))
        }
        None => writeln!(out, "\tcall {}({})", val(*fp), a.join(", ")),
    }
}
```

The code pointer is obtained at a `call`/`times` site by `Instr::Load(codeptr, quot_base)` (offset
0 of the aggregate), yielding a `Ptr`-typed `Value` fed to `CallIndirect`. **No aggregate-argument
ABI wrinkle:** QBE applies the same by-value classification to an indirect callee as a direct one,
so a `:Q`/`:S`/`:E`/`:A` argument through `call %fp(...)` is spelled exactly as through
`call $sym(...)` — confirmed by reusing `qbe_abi_ty` unchanged. `FuncAddr`'s dst and
`CallIndirect`'s `fp` both carry `IrType::Ptr`.

### Q4 — A materialized quotation is `Copy` in 7a; the linear split is 7b's

**Decision: `Copy`.** A non-capturing quotation's aggregate is two non-owning pointer slots (`code`
= a static function address, `env` = null), so it owns no heap and `is_copy` derives Copy
structurally — which is already the checker's assumption (`src/check.rs:4046`, "a quotation effect
is always Copy (D3)"), so 7a changes nothing here. Reject "make it linear from the start": that
would manufacture a `drop` on every dispatch-table entry that disposes nothing, contradicting D6's
"one indirect call, constant stack" and the general rule that ending a non-owning value emits
nothing.

**Forward-compatibility.** 7b's `^Env`-carrying capturing closure is linear (single owner). Because
7a rejects every capturing literal at a materialization boundary (D4), **no linear quotation value
exists in 7a** — there is nothing to split. In 7b linearity is derived structurally from the env
slot's type: a null/empty env stays Copy, an `^Env` env is linear, so a non-capturing quotation
remains Copy in 7b and only a capturing one becomes linear. The split is by capture (D3's line),
not a per-slice flip of a shared type.

### Q5 — The dispatch table: one decode clause, uniform match-free entries, indirect-called

**Decision (read against `examples/vm.sth` and the enum machinery, not assumed):** Sooth enum
variants have **no getter/setter**; elimination is clause-style only (`enum_generated_sigs`,
`src/check.rs:2309`: "a variant has no destructure/getter/setter ... elimination is clause-style"),
and a clause word must be **exhaustive** (`src/check.rs:3329`, `non-exhaustive clause-style`). So
the brief's proposed `[ Vm Op -- Vm ]` entry shape — "each entry extracts its own known variant's
payload" — **cannot be written without a full (exhaustive) clause match inside every entry**, which
is exactly the failure Q5 warns against: the dogfood would prove nothing.

**Resolved shape.** The one legitimate `Op` elimination is a *decode* step — a single clause word,
run once per fetched instruction — that inspects the tag (unavoidable: sum elimination is
clause-style) and (a) yields the opcode number that indexes the table and (b) pushes any immediate
operand onto the `Vm`'s operand stack. The table is `[ [ Vm -- Vm ] N ]`: `N` handler quotations of
one **uniform, match-free** effect `[ Vm -- Vm ]`, each reading its operands from the `Vm` operand
stack. Per instruction: `decode` → index the table by tag → `call` (an indirect call). The
remaining clause word is a **decoder**, not a **dispatcher**; the executable behavior lives in the
indirect-called table. A golden asserts the dogfood's per-instruction path emits `CallIndirect` and
that no handler quotation contains a clause match, and that its stdout matches the retained
enum-plus-clause `examples/vm.sth` byte-for-byte.

## Requirements (traceable)

### Representation and backend (Phase 1)

- **R1.** `IrType::Quotation(QuotSigId)` added (`src/ir.rs`), `Copy`; `QuotSigId` indexes a
  `Module` signature table interned by structural `QuotEffect` equality (dedup like
  `intern_bundle_struct`, `src/ast.rs:433`). (Q2)
- **R2.** Layout: a two-slot aggregate `{ code: Ptr@0, env: Ptr@WORD_WIDTH }`, size `2*WORD_WIDTH`,
  align `WORD_WIDTH`, every figure word-width-derived (backend-neutral invariant). Spelled `:Q{id}`
  in ABI positions, `l` in a register. (Q2/D5)
- **R3.** `ir_type_of`'s `Type::Quotation(eff)` arm (`src/ir.rs:189`) replaces the `unreachable!`
  with interning `eff` and returning `IrType::Quotation(id)`. (recon 2)
- **R4.** `Instr::FuncAddr(Value, String)` and `Instr::CallIndirect(Option<Value>, Value,
  Vec<Value>)` added (`src/ir.rs`) with the QBE emission of Q3 (`src/backend/qbe.rs`). Argument
  spelling reuses `qbe_abi_ty` unchanged; no aggregate-arg ABI wrinkle. (Q3)
- **R5.** `is_copy` returns Copy for `IrType::Quotation` in 7a (env non-owning), consistent with the
  existing `Type::Quotation` Copy treatment (`src/check.rs:4046`). No manufactured `drop`. (Q4/D6)

### The capture predicate (Phase 2)

- **R6.** `fn body_captures_enclosing(body, enclosing) -> bool` (`src/check.rs`) as in Q1: mirrors
  `rename_terms` (`src/ast.rs:1039`), strips `&!`/`&` per `rename_call` (`:1023`), recurses through
  nested quotations and `if` arms carrying body-bound names by value, returns true on the first read
  of an enclosing name not bound within the body. Pure over the term tree; unit-tested in isolation.

### Materialization boundaries (Phases 2–3)

- **R7.** A **materialization boundary** materializes a non-capturing `Known` literal and rejects a
  capturing one (D4). At the boundary the checker: (i) runs `body_captures_enclosing`; (ii) if it
  captures, raises the capturing-quotation error (R12); (iii) else confirms the literal against the
  boundary's expected `Type::Quotation(eff)` via the existing `check_literal_against_declared_effect`
  (`src/check.rs:5603`), sets the slot to `ty: Type::Quotation(eff), quot: None` (erased).
- **R8.** Boundaries lifted: **store into a struct field** and **store into an array element** and
  **word output** — the R7a positions at `check.rs:1239`/`:1259` gain a carve-out for a
  non-capturing literal against a declared quotation field/element/output type; and the storage
  rejection `:7026`. The **clause-body** rejection `:1251` is **not** lifted (out of scope).
- **R9.** Lowering a materialization (`src/ir.rs`): mint **one** `IrFunc` per distinct `QuotId`
  (dedup by id; multiple boundaries referencing one literal share it), signature = the quotation
  effect (inputs→outputs, no env param in 7a); its symbol is a stable mangle
  (`{enclosing}__quot{n}`). Build the `(code, env)` value: `FuncAddr(code, sym)`, `env = Const 0`,
  stored into a fresh two-slot aggregate. This is the one place a quotation literal mints an
  `IrFunc` — quotation-*taking* words still mint none (D2).
- **R10.** `call`/`times` on an **erased** quotation (`quot: None`, `ty: Type::Quotation(eff)`):
  the checker accepts it (reusing `check_abstract_quotation_call`, `src/check.rs:5124`, over `eff`);
  lowering (`src/ir.rs:2798`/`:2815`) branches on provenance — a `Known` marker splices as today
  (byte-identical), an erased value `Load`s the code slot and emits `CallIndirect` (D1). `times`
  drives the indirect call once per iteration inside the existing constant-stack loop skeleton (D6).
  The non-quotation rejections `:5108`/`:5112`/`:5743`/`:5747` stay (they fire on an `i64` etc.,
  never on a `Type::Quotation`).

### Branch-join materialization (Phase 3)

- **R11.** The join (`src/check.rs`, the `(t_then.quot, t_else.quot)` match near `:6467`): when both
  arms leave a quotation, materialize each against an **expected `Type::Quotation(eff)` threaded
  into the `if`** from the enclosing declared context (the word's declared output row, or the
  declared type of the field/array/binding the join flows into). Both arms non-capturing and their
  effects unifying → the join yields a runtime quotation value (`quot: None`); the equal-`Known`-ids
  fast path (`a == b`) still forwards a marker with no `Phi` (splice preserved). Either arm
  capturing → R12. No expected quotation type at the join → keep `different_quotations_at_join_error`
  (`:7053`), reworded to also say "give the quotation a declared type" (no standalone bare-literal
  effect inference is added — `src/check.rs:5598` states none is inferred, and every exit use has a
  declared context). `quotation_versus_value_at_join_error` (`:7064`) stays: differing arm types are
  an ordinary `branch_type_mismatch`.

### Diagnostics (Phases 2–3)

- **R12.** One capturing-quotation diagnostic, reusing `:7026`'s vocabulary (D4). A single function
  `capturing_quotation_error(ctx, span, boundary)` producing exactly:
  `error: a capturing quotation cannot {boundary} (capturing closures are slice 7b) (line {n})`
  with `boundary` ∈ {`be stored`, `be an array element`, `be returned`, `be left on a branch`}.
  Each boundary has a test asserting the exact message (T-cap-*).

### Regression protection (R13a in Phase 1, verification in Phase 3; first-class)

- **R13.** Every 6a–6f combinator golden asserting a spliced tight loop with **no per-element
  `Instr::Call`** stays green and bit-identical. The guarantee is carried in two distinct layers
  and the spec pins both, because they fail differently:
  - **Structural (the load-bearing layer), in `src/ir.rs` units.** Eleven assertions match
    `matches!(i, Instr::Call(..))` and count it. The two that directly guard combinator splicing
    are `each_lowers_to_a_loop_not_a_per_element_call` (`:7726`) and
    `while_lowers_to_a_back_edge_not_an_infinite_splice` (`:7783`); the others
    (`call_of_literal_emits_no_call_instr` `:4715`,
    `times_lowers_to_a_loop_header_not_a_per_iteration_call` `:4750`,
    `tail_self_call_lowers_to_back_edge_not_call` `:6507`, and six more) guard adjacent
    lowering shapes this slice must not disturb.
  - **Behavioural, in `tests/phase4_combinators.rs`.** These are *equivalence witnesses* across a
    `ulimit -s` sweep against a hand-threaded twin, not structural assertions — their own comments
    (`:1188`, `:1410`) explicitly delegate the structural guarantee to the two `ir.rs` units above.
    Do not "strengthen" them into shape assertions; they catch a different failure (stack growth
    and wrong values) and duplicating the structural check there buys nothing.

  **The pin does NOT hold automatically — it is a placebo until this slice widens it.** Verified
  against the source, not assumed: the structural units route through the test helper
  `call_symbols` (`src/ir.rs:4343`), which matches **only** `Instr::Call(_, sym, _)` and maps every
  other instruction to `None`. `each_lowers_to_a_loop_not_a_per_element_call` then asserts
  `user_calls.is_empty()`. A splice regressing into an `Instr::CallIndirect` therefore contributes
  *nothing* to `user_calls`, the assertion still passes, and the regression ships silently. The
  same hole applies to every `matches!(i, Instr::Call(..))` count assertion: the new variant is
  invisible to all eleven.

  **R13a (required, blocking, Phase 1).** Widen the structural helpers *before* `CallIndirect`
  exists in any lowering path, so the pin is real by the time it could be tripped: extend
  `call_symbols` (or add a companion `call_instr_count`) to see `Instr::CallIndirect` as a call,
  and update the two combinator-splice units
  (`each_lowers_to_a_loop_not_a_per_element_call` `:7726`,
  `while_lowers_to_a_back_edge_not_an_infinite_splice` `:7783`) to assert the absence of *both*
  variants. An indirect call has no symbol, so it cannot be reported as a name; report it as a
  distinct count in the failure message (`unexpected calls: [...] + N indirect`) rather than
  fabricating a symbol for it.

  **M4 (mutation, mandatory).** Force a spliced combinator to emit an `Instr::CallIndirect` and
  confirm the widened assertions **fail**. If they pass, the pin is still a placebo and Phase 3 is
  not done. Running M4 against the *unwidened* helper is the control: it must pass there (proving
  the hole was real) and fail after R13a (proving the fix closed it).

## Sanctioned files

- `src/ir.rs` — `IrType::Quotation`/`QuotSigId` + signature table (R1); layout (R2); `ir_type_of`
  arm (R3); `Instr::FuncAddr`/`CallIndirect` (R4); materialization lowering + one-`IrFunc`-per-id
  mint (R9); `call`/`times` provenance branch (R10); unit tests.
- `src/backend/qbe.rs` — `FuncAddr`/`CallIndirect` emission + `:Q` ABI spelling (R4); unit tests.
- `src/ast.rs` — `QuotSig` interning helper if it lands here rather than `ir.rs` (mirrors
  `intern_bundle_struct`); no other change.
- `src/check.rs` — `body_captures_enclosing` (R6); boundary carve-outs + erased-slot production
  (R7/R8); erased `call`/`times` acceptance (R10); join materialization + expected-type threading
  (R11); `capturing_quotation_error` (R12); `is_copy` arm (R5); unit tests.
- `tests/phase4_quotations.rs` — new golden file (quotations-as-values is a distinct concern from
  combinator inlining; split under pressure per CLAUDE.md).
- `examples/vm_table.sth` — the dogfood (new); `examples/vm.sth` retained unchanged as the parity
  oracle.
- `ROADMAP.md` — mark 7a implemented.

No other files. No staged changes outside these.

## Exit criteria (golden tests)

`thing_condition_expected` naming. Compile-time checks use the file's `check_error`/`check_src`
harness; run goldens use `run_src`; QBE-shape assertions inspect emitted IR/QBE like the existing
`src/ir.rs` combinator tests.

| ID | Test | Kind | Phase | Source in → expected out |
|----|------|------|-------|--------------------------|
| T-irtype | `ir_type_of_quotation_is_two_slot_aggregate` (`src/ir.rs`) | unit | 1 | `Type::Quotation(eff)` → `IrType::Quotation`, layout size `2*WORD_WIDTH`, `code`@0/`env`@word |
| T-qbe-addr | `qbe_emits_func_addr_as_copy_of_symbol` (`src/backend/qbe.rs`) | unit+QBE | 1 | `FuncAddr(v,"f")` → `%… =l copy $f` |
| T-qbe-ind | `qbe_emits_indirect_call_through_value` (`src/backend/qbe.rs`) | unit+QBE | 1 | `CallIndirect(Some(r),fp,[a:Q])` → `%r =… call %fp(:Q %a)` (aggregate arg spelled `:Q`) |
| T-field | `quotation_stored_in_struct_field_compiles_and_calls` (`tests/phase4_quotations.rs`) | run | 2 | `type: Holder q [ i64 -- i64 ] ;` build a `Holder`, `Holder>q call` on `4` → prints `5`; path emits `CallIndirect` |
| T-array | `quotation_in_array_element_indirect_calls` (`tests/phase4_quotations.rs`) | run | 2 | array `[ [i64 -- i64] 2 ]` of two literals, index one, `call` on `4` → prints `5`; `CallIndirect` present |
| T-return | `quotation_returned_from_word_indirect_calls` (`tests/phase4_quotations.rs`) | run | 2 | `: mk ( -- [ i64 -- i64 ] ) [ 1 + ] ;` `mk 4 swap call .` → `5`; `CallIndirect` present |
| T-cap-store | `capturing_literal_stored_is_error_naming_7b` (`tests/phase4_quotations.rs`) | reject | 2 | a literal reading an enclosing local, stored → `a capturing quotation cannot be stored (capturing closures are slice 7b)` |
| T-splice-cap | `capturing_literal_spliced_still_works` (`tests/phase4_quotations.rs`) | run | 2 | a capturing literal at a direct `call` (spliced) → runs exactly as today (D3, unaffected) |
| T-join | `two_differing_quotation_arms_materialize_and_call` (`tests/phase4_quotations.rs`) | run | 3 | `: pick ( bool -- [ i64 -- i64 ] ) if [ 1 + ] else [ 2 + ] end ;` `true pick 4 swap call .` → `5`; `false pick` → `6`; `CallIndirect` present |
| T-join-same | `same_quotation_both_arms_still_splices` (`tests/phase4_quotations.rs`) | run+IR | 3 | one literal bound to a local and used (unchanged) in both arms of an `if`, then called on `5` → `6`, no `CallIndirect` (equal ids forward the marker, splice preserved) |
| T-cap-join | `capturing_literal_at_join_is_error_naming_7b` (`tests/phase4_quotations.rs`) | reject | 3 | a capturing literal in one arm of a materializing join → `a capturing quotation cannot be left on a branch (capturing closures are slice 7b)` |
| T-times | `times_over_erased_quotation_runs_constant_stack` (`tests/phase4_quotations.rs`) | run+IR | 3 | an erased quotation driven by `times` → correct result, loop is header+back-edge with one `CallIndirect` in the body, constant stack (D6) |
| T-reg | `combinator_goldens_unchanged_no_per_element_call` (`src/ir.rs`, `tests/phase4_combinators.rs`) | IR | 3 | every existing 6a–6f `no per-element Instr::Call` / tight-loop assertion passes unchanged (R13) |
| T-dogfood | `vm_table_dispatch_matches_clause_version` (`tests/phase4_quotations.rs`) | run+IR | 4 | `examples/vm_table.sth` stdout == `examples/vm.sth` stdout; execution path emits `CallIndirect`; no handler quotation contains a clause match (Q5) |
| T-roadmap | ROADMAP 7a marked implemented (`ROADMAP.md`) | doc | 4 | prose exit line; no test |

## Load-bearing / mutation-test-required criteria

Placebo tests have shipped on this project repeatedly; a criterion not flagged here tends not to get
mutation-tested. The reviewer **must** prove each can fail by reverting the specific guard it
protects in a **throwaway copy** of the compiler (not the shared worktree).

- **M1 (guards R10's provenance branch — T-field/T-return + T-join-same).** Force the `call`/`times`
  lowering to always splice (ignore erased provenance): T-field/T-return must **fail** to link or
  compile (no symbol to splice for an erased value). Independently force it to always emit
  `CallIndirect`: T-join-same must go **red** (a `Known` marker wrongly indirect-called; the splice
  fast path lost). Proves provenance, not a size heuristic, is the switch (D1).
- **M2 (guards R6 — the capture predicate — T-cap-store).** Make `body_captures_enclosing` always
  return `false`: T-cap-store (and T-cap-join) must go **red** (a capturing literal wrongly
  materialized). Make it always return `true`: T-field/T-return must go **red** (a non-capturing
  literal wrongly rejected). Proves the predicate is wired into the boundary, both directions.
- **M3 (guards R6's nested/borrow reach — T-cap-store variant).** Drop the recursion into nested
  `TermKind::Quotation`/`if` arms in the predicate: a test whose enclosing-name read sits inside a
  nested quotation (`capturing_through_nested_quotation_is_error`) must go **red** (the read is
  missed and the literal wrongly materialized). Proves D4's capture-into-another-quotation reach.
- **M4 (guards R13 — the regression pins are real, not vacuous).** The T-reg assertions match on
  `Instr::Call` specifically; confirm at least one flips **red** when a combinator splice is
  mutated to emit an `Instr::Call` (revert one splice site to a name-keyed call). Proves the pins
  detect a splice→call regression rather than passing vacuously.
- **M5 (guards Q5 — the dogfood proves the feature — T-dogfood).** Delete the `CallIndirect`
  assertion's counterpart by inlining a handler back into a clause dispatch: T-dogfood must go
  **red** on the "no handler contains a clause match" check. Proves the table is genuinely
  indirect-called and the decode/execute split is real, not a match moved into every entry.

## Phased delivery plan

Four phases; each independently green under
`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.

**Phase 1 — representation and backend (hard).** `IrType::Quotation`/`QuotSigId` + signature table
(R1), layout (R2), the `ir_type_of` arm (R3), `Instr::FuncAddr`/`CallIndirect` + QBE emission (R4),
`is_copy` arm (R5). Verified by hand-built-IR unit tests (T-irtype/T-qbe-addr/T-qbe-ind) — no
frontend producer yet, but the tree stays green because the new variants are exercised by tests.

**Phase 2 — materialization at store/field/array/output + indirect call + capture predicate (hard).**
`body_captures_enclosing` (R6); boundary carve-outs and erased-slot production for field/array/output
and the storage rejection (R7/R8); one-`IrFunc`-per-id materialization lowering (R9); the `call`/
`times` provenance branch (R10); the `capturing_quotation_error` (R12). Goldens T-field/T-array/
T-return/T-cap-store/T-splice-cap. Drive M1/M2/M3.

**Phase 3 — branch-join materialization + `times`-erased + regression pins (hard).** Expected-type
threading into the `if` join (R11); T-join/T-join-same/T-cap-join; the `times`-erased constant-stack
golden (T-times); the R13 regression pins (T-reg). Drive M4.

**Phase 4 — dogfood + ROADMAP (standard).** `examples/vm_table.sth` (the decode-clause + uniform
match-free handler table, Q5); the parity golden T-dogfood (stdout == `examples/vm.sth`, path emits
`CallIndirect`, no handler clause match). Drive M5. Mark ROADMAP 7a implemented (T-roadmap).

## Explicitly out of scope (do not spec, do not build)

- **Capturing closures** of any kind, downward or upward, and **`^Env`** (7b, gated on 6f). Recon
  9/10 are the argument; D3/D4 are the rule.
- **The capture-set analysis** — 7a builds only the boolean predicate (Q1).
- **Inline budgets, `inline`/`noinline`, and sinking a `call` into branch arms** to avoid
  materializing at a join. Optimizations against a semantics not settled until 7b.
- **The clause-body rejection** (`check.rs:1251`) — splicing a callee, not representing a value.
- **Any change to what splicing means** — recon 10 makes it library-breaking; the splice path comes
  out of this slice bit-identical, pinned by R13/T-reg.
- **Standalone bare-literal effect inference** — none exists (`src/check.rs:5598`) and none is added;
  every materialization boundary takes its effect from a declared context (R7/R11).
- **Polymorphic quotation *values*** — 7a materializes concrete-effect quotations; a quotation whose
  effect still carries type variables at a boundary stays under the existing R7a rejection until a
  later slice (the dogfood and all exit uses are monomorphic).

```json
{
  "phases": [
    { "phase": 1, "focus": "representation and backend: IrType::Quotation + QuotSigId signature table, two-slot code/env layout, ir_type_of arm replacing the unreachable, Instr::FuncAddr and Instr::CallIndirect with QBE emission, is_copy arm, R13a widening call_symbols and the two combinator-splice units to see CallIndirect before any lowering can emit it, hand-built-IR unit tests", "difficulty": "hard" },
    { "phase": 2, "focus": "materialization at store/field/array/output plus indirect call and the capture predicate: body_captures_enclosing, boundary carve-outs and erased-slot production, one IrFunc per QuotId mint, call/times provenance branch splice-vs-indirect, capturing_quotation_error, goldens and M1/M2/M3 mutations", "difficulty": "hard" },
    { "phase": 3, "focus": "branch-join materialization with expected-type threading, same-quotation-both-arms splice preservation, capturing-at-join rejection, times over an erased quotation in constant stack, and the 6a-6f no-per-element-Call regression pins with M4", "difficulty": "hard" },
    { "phase": 4, "focus": "dogfood examples/vm_table.sth as a decode clause plus uniform match-free handler table indirect-called, parity golden against examples/vm.sth, M5, and ROADMAP 7a marked implemented", "difficulty": "standard" }
  ]
}
```
