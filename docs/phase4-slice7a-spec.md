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
questions Q1–Q5 concretely, and states exit criteria in the CLAUDE.md golden-test style.
Line citations were checked against `main`, corrected once through a review round, and re-checked
after; corrections to the brief's anchors are called out inline (the brief cited
`check_abstract_quotation_call` at `:6096` and `check_literal_against_declared_effect` at `:5599`;
the real lines are `:5124` and `:5603`). Two classes of drift survived the first pass and are fixed
here: a citation landing a few lines off a function/quote (harmless once corrected), and a citation
naming a *guard's definition* where a *call site* was the thing that mattered — the latter hid a
real boundary gap (R8) and is why R7/R8 now cite call sites, not definitions, wherever the
distinction is load-bearing.

## Where the change lands (verified against current `main`)

- **The `unreachable!` is the type-lowering seam.** `ir_type_of`'s `Type::Quotation(_)` arm is
  `unreachable!("a quotation type has no IrType this slice ...")` (`src/ir.rs:189`); its own
  comment names this slice as the lift. This becomes the new `IrType::Quotation` arm — additive at
  a known point (recon 2).
- **The type-side variant already exists.** `Type::Quotation(&'static QuotEffect)` (`src/ast.rs:742`),
  `PolyType::Quotation(Vec<PolyType>, Vec<PolyType>)` (`src/ast.rs:524`), and `struct QuotEffect`
  (`src/ast.rs:752`) exist with unification and `apply_subst` following (6a). `is_copy`/`poly_is_copy`
  already treat a quotation effect as **Copy** (`src/check.rs:4049`, "a quotation effect is always
  Copy (D3)"). No `Type`/`PolyType` change is needed; only `IrType` gains variants (Q2/Q4).
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
- **The eleven slice-7 rejection sites.** `audit_quotation_type_registries` (`check.rs:1105`, a
  quotation type as a struct field, enum-variant field, array element, owned-cell payload, or
  reference referent — R7a's *declaration-time* audit; this is the site the spec previously
  mis-cited as `:1239`/`:1259`, which land on a duplicated error-format string inside
  `reject_quotation_type_position`, not on a call site) and the word-output/input audit
  inside `audit_word_quotation_positions` (`:1150`); `:1251` (a quotation-taking word with a clause
  body — **not lifted**, D4/out-of-scope); `:2481` (a quotation on a REPL residual); `:5108`/`:5112`
  (`call` on a non-quotation); `:5743`/`:5747` (`times` on a non-quotation);
  `reject_quotation_operand` (`:7011`, operator operand); `reject_quotation_stored` (`:7024`,
  `fill`/`!`/`+!`); `reject_quotation_argument` (`:7038`, a definition with four call sites —
  `:4130`, `:5441`, `:5558`, and `:6352`; `:6352` is the generic monomorphic-call argument loop and
  is the site a struct **constructor** call actually goes through, per its own comment: "covers
  generated struct constructors/setters and `extern` args"); `:7053`/`:7064` (the two branch-join
  cases); plus `ir.rs:189`.
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

### Q2 — `IrType` and layout: a distinct `IrType::Quotation`, a two-slot `{ Code, Ptr }` aggregate, env not elided

**Decision:** add `IrType::Quotation(QuotSigId)` (`src/ir.rs`), keyed by a small `Copy`
`QuotSigId` into a new `Module`-level `Vec<QuotSigLayout>` interned by structural effect equality
(the same construction and dedup discipline as `intern_bundle_struct`, `src/ast.rs:433`). Its
layout is a **fixed two-slot aggregate `{ code: Code, env: Ptr }`**: `code` at offset 0, `env` at
offset `WORD_WIDTH`, size `2 * WORD_WIDTH`, align `WORD_WIDTH` — every offset/size derived from the
word-width parameter, never hardcoded (load-bearing invariant: backend-neutral IR, `Ptr` opaque).

**`code` is a new, distinct `IrType::Code`, not `IrType::Ptr` (corrected on review).** The first
draft spelled `code` as `Ptr`. That erases, at the exact point the checker still knows it, the fact
that this word holds a *function identity* rather than *addressable data* — which is precisely the
information the backend-neutrality invariant exists to preserve (`Ptr[T]` is opaque so a future
WASM lowering never has to reconstruct "is this word a code pointer" by dataflow analysis; on WASM
a function reference is a table index, not an address, and nothing distinguishes a funcref from a
data pointer inside a generic `Ptr`-typed aggregate slot). `IrType::Code` fixes that at zero QBE
cost: on QBE it classifies identically to `Ptr` — `l` in a register, `l` in `qbe_abi_ty` — so no
emission changes beyond adding the match arm (R4). It is spelled `:Q{id}` in ABI positions and `l`
in a register, same as `Struct`/`Enum`/`Array`, so aggregate ABI classification is otherwise reused
unchanged; only the *identity* of the code slot's type changes, not its QBE realization.

**Contract, so `Code` stays a handle and does not grow into a second pointer type:** no arithmetic,
no dereference, no cast to/from `Ptr` or an integer. Produced only by `FuncAddr`; consumed only by
`CallIndirect` (as the callee) and by ordinary aggregate store/load (writing/reading the `code`
slot). A future table-based backend (WASM) is free to realize `Code` as a table index instead of an
address without touching any other `IrType`; a QBE-only realization detail is exactly what `Ptr`
already keeps opaque for data pointers, and `Code` extends the same discipline to code pointers.

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
/// The address of a (materialized) function symbol as an `IrType::Code` value
/// (Q2: a distinct opaque handle, not `Ptr`). Emitted at a materialization
/// boundary to fill a quotation's `code` slot.
FuncAddr(Value, String),
/// An indirect call through a code-handle `Value` (the quotation's `code`
/// slot, already `Load`ed, `IrType::Code`). Mirrors `Call` but the callee is
/// a value, not a symbol. `env` is not passed in 7a (a non-capturing callee
/// has no env parameter); 7b adds the env argument here.
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

The code handle is obtained at a `call`/`times` site by `Instr::Load(codeptr, quot_base)` (offset
0 of the aggregate), yielding a `Code`-typed `Value` fed to `CallIndirect`. **No aggregate-argument
ABI wrinkle:** QBE applies the same by-value classification to an indirect callee as a direct one,
so a `:Q`/`:S`/`:E`/`:A` argument through `call %fp(...)` is spelled exactly as through
`call $sym(...)` — confirmed by reusing `qbe_abi_ty` unchanged, with one added arm (`Code => "l"`,
same as `Ptr`). `FuncAddr`'s dst and `CallIndirect`'s `fp` both carry `IrType::Code`.

### Q4 — A materialized quotation is `Copy` in 7a; the linear split is 7b's

**Decision: `Copy`.** A non-capturing quotation's aggregate is two non-owning slots (`code`
= a static function address, `env` = null), so it owns no heap and `is_copy` derives Copy
structurally — which is already the checker's assumption (`src/check.rs:4049`, "a quotation effect
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
`src/check.rs:2317`, doc comment at `:2312`–`:2313`: "a variant has no destructure/getter/setter
... elimination is clause-style"), and a clause word must be **exhaustive** (`src/check.rs:3329`,
`non-exhaustive clause-style`). So
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

- **R1.** `IrType::Code` and `IrType::Quotation(QuotSigId)` added (`src/ir.rs`), both `Copy`.
  `Code` is a new opaque handle type distinct from `Ptr` (Q2, corrected on review — not `Ptr`);
  `QuotSigId` indexes a `Module` signature table interned by structural `QuotEffect` equality
  (dedup like `intern_bundle_struct`, `src/ast.rs:433`). (Q2)
- **R2.** Layout: a two-slot aggregate `{ code: Code@0, env: Ptr@WORD_WIDTH }`, size `2*WORD_WIDTH`,
  align `WORD_WIDTH`, every figure word-width-derived (backend-neutral invariant). Spelled `:Q{id}`
  in ABI positions, `l` in a register (`Code` classifies identically to `Ptr` in `qbe_abi_ty` — one
  added match arm, no QBE-emission change). (Q2/D5)
- **R3.** `ir_type_of`'s `Type::Quotation(eff)` arm (`src/ir.rs:189`) replaces the `unreachable!`
  with interning `eff` and returning `IrType::Quotation(id)`. (recon 2)
- **R4.** `Instr::FuncAddr(Value, String)` and `Instr::CallIndirect(Option<Value>, Value,
  Vec<Value>)` added (`src/ir.rs`), both operating on `IrType::Code`, with the QBE emission of Q3
  (`src/backend/qbe.rs`). Argument spelling reuses `qbe_abi_ty` unchanged, plus the one new
  `Code => "l"` arm; no aggregate-arg ABI wrinkle. (Q3)
- **R5.** `is_copy` returns Copy for `IrType::Quotation` in 7a (env non-owning) and for
  `IrType::Code` (a non-owning handle), consistent with the existing `Type::Quotation` Copy
  treatment (`src/check.rs:4049`). No manufactured `drop`. (Q4/D6)

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
- **R8.** Boundaries lifted, at three distinct guards (corrected on review — the first draft cited
  only two of them, and the correction is what surfaces a hole a probe could not reach: the missing
  carve-out made every struct-field program fail one guard earlier, at declaration time):
  1. **Declaration-time legality** — `audit_quotation_type_registries` (`check.rs:1105`) and the
     word-output/input audit in `audit_word_quotation_positions` (`:1150`) gain a carve-out so a
     `type:`/`array:` declaration, or a word's declared output, may name a `Type::Quotation` field
     / element / output type. Owned-cell payloads and reference referents (also audited there)
     are **not** carved out — a quotation still cannot be a cell's payload or a reference's
     referent in 7a (neither is a listed D4 boundary; both stay rejected).
  2. **Construction legality** — `reject_quotation_argument`'s call site inside the generic
     monomorphic-call argument loop (`check.rs:6352`) gains a carve-out: when the declared
     parameter type `want` is `Type::Quotation(eff)`, skip the unconditional
     `found.quot.is_some()` rejection and run R7 instead (materialize a non-capturing literal,
     reject a capturing one via R12). This is the site a struct **constructor** call for
     `[ 1 + ] Holder` actually reaches — it is *not* reached by the type-position guards above,
     which only gate whether `Holder` may be *declared* with a quotation field. The carve-out is
     gated strictly on `want`'s type, not on the callee being a constructor, so it also covers a
     generated setter and an ordinary user word declaring a quotation parameter the same way; an
     `extern` word's argument (also routed through this loop, per its own comment) is **not**
     carved out — a quotation still cannot cross an FFI boundary. `reject_quotation_argument`'s
     other three call sites (`:4130`, `:5441`, `:5558`) are unaffected unless they too check a
     declared quotation parameter, in which case the same `want`-typed gate applies uniformly.
  3. **Mutation legality** — the storage rejection `reject_quotation_stored` (`check.rs:7024`,
     `fill`'s element and `!`/`+!`'s value) gains the same non-capturing-literal carve-out for the
     array-element and struct-field-via-reference paths.

  The **clause-body** rejection `:1251` is **not** lifted (out of scope).
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
  capturing → R12, **checked before the id/expected-type resolution runs** (ordering pin, so a
  capturing arm always raises R12's diagnostic and never `different_quotations_at_join_error` by
  falling through the id-compare path first). No expected quotation type at the join → keep
  `different_quotations_at_join_error` (`:7053`), reworded to also say "give the quotation a
  declared type" (no standalone bare-literal effect inference is added — `src/check.rs:5598` states
  none is inferred, and every exit use has a declared context). `quotation_versus_value_at_join_error`
  (`:7064`) stays: differing arm types are an ordinary `branch_type_mismatch`.

### Diagnostics (Phases 2–3)

- **R12.** One capturing-quotation diagnostic, reusing `:7026`'s vocabulary (D4). A single function
  `capturing_quotation_error(ctx, span, boundary)` producing exactly:
  `error: a capturing quotation cannot {boundary} (capturing closures are slice 7b) (line {n})`
  with `boundary` ∈ {`be stored`, `be an array element`, `be returned`, `be left on a branch`}.
  Each boundary has a test asserting the exact message (T-cap-*).

### Regression protection (R13a in Phase 1, verification in Phase 3; first-class)

- **R13.** Every 6a–6f combinator golden asserting a spliced tight loop with **no per-element
  `Instr::Call`** stays green and bit-identical. The guarantee is carried in **two disjoint
  extractors** in `src/ir.rs` (corrected on review — the first draft conflated them, which is what
  let the placebo through: it prescribed widening only one of the two):
  1. **The `call_symbols` helper** (`:4343`), used by the two combinator-splice units
     `each_lowers_to_a_loop_not_a_per_element_call` (`:7726`) and
     `while_lowers_to_a_back_edge_not_an_infinite_splice` (`:7783`), which assert
     `call_symbols(main).is_empty()` / `user_calls.is_empty()`. `call_symbols` matches only
     `Instr::Call(_, sym, _)` and maps everything else to `None`.
  2. **Eleven independent inline closures**, each written as `count(f, |i| matches!(i,
     Instr::Call(..)))` at its own call site — there is no shared predicate function to widen
     once; `count` (`:4652`) takes an arbitrary closure, and each of the eleven sites supplies its
     own. The two load-bearing for *this slice* — because their test programs are the only ones
     among the eleven that can ever contain a `call`/`times` on a quotation, the exact path R10
     touches — are `call_of_literal_emits_no_call_instr` (`:4715`) and
     `times_lowers_to_a_loop_header_not_a_per_iteration_call` (`:4750`). The other nine
     (`:4876`, `:4896`, `:5496`, `:6507`, `:6558`, `:6575`, `:6591`, `:6612`, `:6924`) exercise
     unrelated lowering shapes (max/compare, string length, self-tail-call, drop) whose test
     programs contain no quotation at all, so `Instr::CallIndirect` cannot appear in them; they are
     widened anyway, for uniformity, but are not safety-critical for 7a.
  - **Behavioural, in `tests/phase4_combinators.rs`.** These are *equivalence witnesses* across a
    `ulimit -s` sweep against a hand-threaded twin, not structural assertions — their own comments
    (`:1188`, `:1410`) explicitly delegate the structural guarantee to the two `call_symbols` units
    above. Do not "strengthen" them into shape assertions; they catch a different failure (stack
    growth and wrong values) and duplicating the structural check there buys nothing.

  **The pin does NOT hold automatically — it is a placebo until this slice widens both
  extractors.** Verified against the source, not assumed: neither `call_symbols` nor any of the
  eleven inline `matches!(i, Instr::Call(..))` closures can see `Instr::CallIndirect` (it is a
  distinct variant, mapped to `None`/`false` by both). A splice regressing into `CallIndirect`
  therefore contributes nothing to either extractor's count, every existing assertion still
  passes, and the regression ships silently — including at `call_of_literal_emits_no_call_instr`
  and `times_lowers_...`, the two tests closest to R10's new provenance branch.

  **R13a (required, blocking, Phase 1).** Widen **both** extractors *before* `CallIndirect` exists
  in any lowering path, so the pin is real by the time it could be tripped:
  1. Extend `call_symbols` (or add a companion) to see `Instr::CallIndirect` as a call; update the
     two units at `:7726`/`:7783` to assert the absence of both variants. An indirect call has no
     symbol, so report it as a distinct count in the failure message
     (`unexpected calls: [...] + N indirect`) rather than fabricating one.
  2. Introduce one shared predicate, e.g. `fn is_call_instr(i: &Instr) -> bool { matches!(i,
     Instr::Call(..) | Instr::CallIndirect(..)) }`, and replace all eleven inline
     `|i| matches!(i, Instr::Call(..))` closures with it. This is a mechanical, uniform edit across
     all eleven sites — do not selectively widen only the two load-bearing ones, since a future
     reader has no way to tell a widened site from an unwidened one without re-deriving which nine
     are "safe" to leave narrow.

  **M4 (mutation, mandatory).** Mutate a `call`/`times`-on-a-literal lowering site (not an
  arbitrary combinator) to emit `Instr::CallIndirect` in place of the splice, and confirm
  `call_of_literal_emits_no_call_instr` (`:4715`) and `times_lowers_...` (`:4750`) **both** flip
  red. Mutating to `Instr::Call` instead of `CallIndirect` does not exercise this hole (the
  unwidened predicate already catches a bare `Instr::Call`) and is not an acceptable substitute.
  Running M4 against the code *before* R13a's widening is the control: it must **pass** there
  (proving the hole was real) and **fail** after R13a (proving the fix closed it).

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
  combinator inlining; split under pressure per CLAUDE.md). Being new, it needs its own
  `check_error`/`check_src`/`run_src` harness helpers, copied from the per-file convention already
  used in `tests/phase4_combinators.rs`/`tests/phase4_generics.rs` (each test file owns its own
  copies; there is no shared `src`-level harness to import).
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
| T-cap-store | `capturing_literal_stored_is_error_naming_7b` (`tests/phase4_quotations.rs`) | reject | 2 | a literal reading an enclosing local, stored → exact message `a capturing quotation cannot be stored (capturing closures are slice 7b)`, asserted with `assert_eq!`, not `.contains` (distinct vocabulary from the pre-existing `:7024` "escaping quotations are slice 7" wording it must not be conflated with) |
| T-splice-cap | `capturing_literal_spliced_still_works` (`tests/phase4_quotations.rs`) | run | 2 | a capturing literal at a direct `call` (spliced) → runs exactly as today (D3, unaffected) |
| T-join | `two_differing_quotation_arms_materialize_and_call` (`tests/phase4_quotations.rs`) | run | 3 | `: pick ( bool -- [ i64 -- i64 ] ) if [ 1 + ] else [ 2 + ] end ;` `true pick 4 swap call .` → `5`; `false pick` → `6`; `CallIndirect` present |
| T-join-same | `same_quotation_both_arms_still_splices` (`tests/phase4_quotations.rs`) | run+IR | 3 | one literal bound to a local and used (unchanged) in both arms of an `if`, then called on `5` → `6`, no `CallIndirect` (equal ids forward the marker, splice preserved) |
| T-cap-join | `capturing_literal_at_join_is_error_naming_7b` (`tests/phase4_quotations.rs`) | reject | 3 | a capturing literal in one arm of a materializing join → exact message `a capturing quotation cannot be left on a branch (capturing closures are slice 7b)`, `assert_eq!` not `.contains` (R11's ordering pin: the capture check runs first, so this fires rather than `different_quotations_at_join_error`) |
| T-times | `times_over_erased_quotation_runs_constant_stack` (`tests/phase4_quotations.rs`) | run+IR | 3 | an erased quotation driven by `times` → correct result, loop is header+back-edge with one `CallIndirect` in the body, constant stack (D6) |
| T-reg | Not a new test (corrected on review — the first draft named one that would not exist): the real protection is ~13 existing functions in `src/ir.rs`, unchanged and widened per R13a — `each_lowers_to_a_loop_not_a_per_element_call` (`:7726`), `while_lowers_to_a_back_edge_not_an_infinite_splice` (`:7783`), and the eleven `count(.., is_call_instr)` sites (`:4715`, `:4750`, `:4876`, `:4896`, `:5496`, `:6507`, `:6558`, `:6575`, `:6591`, `:6612`, `:6924`) | IR | 1+3 | widened in Phase 1 (R13a), all pass unchanged through Phase 3 (R13) |
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
- **M4 (guards R13a — the regression pins actually see `CallIndirect`, not just `Call`).** Mutate a
  `call`/`times`-on-a-literal lowering site to emit `Instr::CallIndirect` in place of the splice
  (not `Instr::Call` — that variant was already caught pre-widening and proves nothing about the
  hole). Confirm `call_of_literal_emits_no_call_instr` (`:4715`) and `times_lowers_...` (`:4750`)
  both flip **red** only after R13a's widening lands, and would have passed (the placebo) against
  the unwidened predicate. Proves the pins detect a splice→indirect regression, not just a
  splice→direct-call one.
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
    { "phase": 1, "focus": "representation and backend: IrType::Code (distinct from Ptr) and IrType::Quotation + QuotSigId signature table, two-slot code/env layout, ir_type_of arm replacing the unreachable, Instr::FuncAddr and Instr::CallIndirect with QBE emission, is_copy arm, R13a widening call_symbols plus a shared is_call_instr predicate across all eleven inline count sites to see CallIndirect before any lowering can emit it, hand-built-IR unit tests", "difficulty": "hard" },
    { "phase": 2, "focus": "materialization at store/field/array/output plus indirect call and the capture predicate: body_captures_enclosing, carve-outs at all three guards (declaration-time type-position audit, constructor/setter argument check, storage rejection) plus erased-slot production, one IrFunc per QuotId mint, call/times provenance branch splice-vs-indirect, capturing_quotation_error, goldens and M1/M2/M3 mutations", "difficulty": "hard" },
    { "phase": 3, "focus": "branch-join materialization with expected-type threading, same-quotation-both-arms splice preservation, capturing-at-join rejection, times over an erased quotation in constant stack, and the 6a-6f no-per-element-Call regression pins with M4", "difficulty": "hard" },
    { "phase": 4, "focus": "dogfood examples/vm_table.sth as a decode clause plus uniform match-free handler table indirect-called, parity golden against examples/vm.sth, M5, and ROADMAP 7a marked implemented", "difficulty": "standard" }
  ]
}
```
