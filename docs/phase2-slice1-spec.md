# Phase 2 Slice 1 — the typed-core spine (delivered)

Replaced the arity-only checker with one that carries a concrete `Type` per virtual-stack slot and unifies type (not just depth) through word bodies and at `if` join points. Introduced two frontend types, `i64` and `bool`. Built on the Phase 0/1 compiler (lexer → parser → checker → IR → QBE → driver/REPL). Scope held to this slice; nothing else from the Phase 2 epic.

## Locked decisions

- **D1.** `int` hard-renamed to `i64`; no alias. Slot types are concrete.
- **D2.** `true`/`false` are `bool` literals (`TermKind::BoolLit`, analogous to `IntLit`).
- **D3./D6.** `bool` is a distinct IR type (`IrType::Bool`, QBE `w` 0/1), never collapsed into `Int`. Frontend `Type { I64, Bool }` and backend `IrType { Int, Bool, Ptr }` stay separate; lowering maps `Type → IrType`.
- **D4.** `.` stays `( i64 -- )`; no `bool` printing.
- **D5.** Shuffles (`dup`/`drop`/`swap`/`over`/`rot`) are structural and type-transparent: they move whatever concrete slot types are present. Not table entries, not polymorphic.

Typed builtin signatures: `+ - * mod` → `( i64 i64 -- i64 )`; `= < >` → `( i64 i64 -- bool )`; `.` → `( i64 -- )`; shuffles structural.

## What was built

- **Frontend `Type` enum** in `ast.rs`; `TypedSlot.ty: String → Type`. Type-name words resolved to `Type` in `parse_slot` at the parse boundary; unknown name → reported error (with slot-name span tracked for accurate location). `int`→`i64` across examples and all test/inline sources.
- **`bool` literals + IR width.** `true`/`false` parse to `BoolLit`. `IrType::Bool` added; per-`Value` `IrType` map (IntLit/Bin/Load=Int, BoolLit/Cmp=Bool, Phi=unified, Call=word-output, params=declared). `lower_word` maps declared `Type → IrType`. Backend picks width from `IrType` (`Bool`→`w`, `Int`→`l`) and inserts a `w→l` extension at the only in-slice `bool` sinks: the line-wrapper `storel` epilogue and an `l`-typed `bool` return. `if` uses `jnz` (any width, no extension). 8-byte buffer marshalling unchanged.
- **Typed checker core.** `Arity`/`builtin_table` replaced by a typed-effect representation and table. `check_terms`/`check_term` simulate `Vec<Type>`: operand type checks, `if`-requires-`bool`, declared-output per-slot type check, structural type-transparent shuffles. `infer_line` takes and returns a typed carried stack. `repl.rs` rewired: `WordEntry` carries the typed effect, a typed checker env is built and an arity map derived for `ir` (envs not unified); `Session` records a `Type` per carried slot; `format_stack`/`buf` unchanged (a `bool` displays as its `0`/`1` slot value).
- **Branch-join type unification.** The `if` join in `check_term` compares arms slot-by-slot on `Type` as well as depth; a disagreement names the differing types. `lower_if` phi unchanged in shape (arm types now guaranteed equal by the checker).
- **Dogfood + goldens.** `examples/sign.sth` (`: sign ( i64 -- i64 ) 0 > if 1 else 0 then ;` + `main`) compiles to a native binary and runs. Five negative diagnostics asserted as goldens; a REPL type-error line proves the session survives; `gcd`/`factorial`/`lerp` still print `5`/`120`/`30` as `i64`.

## Diagnostics (assert salient substrings + type names)

1. `if` condition not bool → `` expected `bool` `` / `` found `i64` ``.
2. Operand type mismatch → `` expected `i64` `` / `` found `bool` ``.
3. Branch-join type mismatch → arms disagree, naming slot types.
4. Declared-output type mismatch → cites the declared effect.
5. Unknown type name → `` unknown type `foo` ``.

## Exit criteria (met)

- **E1.** `int`→`i64` throughout; goldens print `5`/`120`/`30` as `i64`.
- **E2.** `bool` is a real, distinct type: literals parse; comparisons yield `bool`; `if` requires `bool`.
- **E3.** Type mismatches are sharp compile errors (the five above).
- **E4.** `if` join points unify on type, not just depth.
- **E5.** Phase 1 REPL works; carried stack tracks types; a type-erroring line reports and the session survives.
- **E6.** `sign` dogfood runs as a native binary and in the REPL, plus negative goldens.

"Green": `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Delivery (as committed)

- **Phase 1 — i64 rename + `Type` enum:** `Type { I64, Bool }`, `TypedSlot.ty→Type`, `parse_slot` resolution + unknown-type error (slot-name span tracked), `int`→`i64` everywhere. (`8e596154`, `e81cc71d`)
- **Phase 2 — bool literals + IR:** `BoolLit`, `IrType::Bool`, per-`Value` type map, `Type→IrType` for params/ret, QBE width selection + `w→l` extension. (`86fe45dd`)
- **Phase 3 — typed checker core:** typed-effect table, `Vec<Type>` simulation, typed `infer_line`, `repl.rs` rewire (typed env + derived arity env, `Session` typed slots). (`e005eaa7`, doc `3f0c9ac0`)
- **Phase 4 — branch-join unification:** per-slot type agreement at the `if` join. (`ee400bac`)
- **Phase 5 — dogfood + goldens:** `examples/sign.sth`, five negative diagnostics, REPL survival test, regression checks. (`240e4ea4`, `c7480790`)

## Out of scope (later slices / Phase 4)

Numeric tower breadth, `*/` widening, literal-default adoption, structs, enums/`match`, arrays, optional/pointer, the `Copy` marker, polymorphic shuffle signatures. Backend remains QBE; IR stays backend-neutral.
