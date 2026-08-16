# Phase 4 Slice 1: Type/row/length variables + monomorphization (native) — *shipped*

Base: `main` @ `9f8644c`. Brief D1–D7 locked. **Native only** (REPL monomorphization is Slice 2; no `src/repl.rs` behaviour change beyond the shared `lower_call`).

## What shipped

A user `:` word can declare a polymorphic stack effect with three variable forms: a **type variable** `'T`, the **row variable** `..s`, and a **length variable** in an array count `['T 'N]`, optionally bounded (`'T: Copy`). The checker unifies each polymorphic word against the concrete stack at every call site, checks bounds against the concrete instantiation Kitten-style, and records each instantiation; the backend emits one monomorphized `IrFunc` per distinct ground instantiation. The multi-output call panic is closed by a synthesized aggregate-return ABI, which is also the lowering path for a row variable once monomorphization resolves it to a concrete output count. `max` (integer tower) and `max-total` (floats, `total_cmp` order) ship as inline builtins. The core shuffles (`dup`/`swap`/`over`/`rot`/`drop`) are unchanged: already type-transparent by construction.

## Locked decisions (D1–D7 → spec decisions S1–S6)

- **S1.** A signature stops being purely concrete in exactly one bounded place: a new checker-side `PolyType`/`PolySig` holds variables in a word's declared effect and call-site unification only. `Type` gains **no** variant; the `Slot` virtual stack stays concrete; monomorphic words resolve to a concrete `Sig` exactly as before.
- **S2.** Multi-output ABI = **synthesized aggregate return**, reusing the already-shipping `out_arity == 1` struct-return path (`vm-pop`). No new `Instr`; count-agnostic, so a row-expanded count is free. (Out-parameters and carried runtime stack rejected.)
- **S3.** Core shuffles are checker-only and need no signature representation: `check_shuffle` (`src/check.rs:4717`) is type-transparent and intercepted before `env` lookup; `lower_call`'s shuffle arms (`src/ir.rs:2032`+) dispatch on runtime `value_type`, emit no `Call`. New machinery is for user-declared polymorphic words only.
- **S4.** `Copy` is an ordinary required-operation constraint, not privileged. A type variable carries a bound set (`Copy`, `Ord`), resolved at the concrete instantiation by existing predicates. `is_copy`/`is_linear` keep their concrete-only matches; a bare variable's copy/Ord-ness is answered from its bound set by the separate `check_poly_body` pass. Keeps the door open for a polymorphic `drop` in Slice 6.
- **S5.** Dogfood is `examples/stack.sth`, rewritten to return multiple values directly. Polymorphic `dup`/`swap` touch no existing example (reported, not papered over).
- **S6.** `max` is integer-only; over a float it errors naming `max-total`. `max-total` is `f32`/`f64` by `total_cmp` bit-pattern order. Both inline builtins, neither monomorphized.

## Requirements by stage

### Surface syntax / parsing (`src/lexer.rs`, `src/parser.rs`, `src/ast.rs`)
- **R1.** Three variable forms, each arriving as a single `Token::Word` (`'`/`..` are not delimiters), recognised in `parse_slot`/`parse_type_expr`: type variable `'T` (any type slot), length variable `'N` (array count slot, distinguished from `'T` purely by grammatical position), row variable `..s` (deepest/leftmost slot of a side, at most once per side). First leftmost occurrence binds, later ones use. Same name in a type slot vs count slot = two variables (X1). `..s` non-deepest or twice on a side (X2).
- **R2.** A word with no variables is unchanged: `parse_effect` still yields a concrete `StackEffect`; the polymorphic representation attaches only when a variable is present (regression-checked, R15).
- **R3.** Bound `'T: Copy` (or `'T: Copy Ord`) at the binding occurrence; capabilities `Copy`, `Ord`. Bound on a use occurrence or unknown capability (X3).

### Representation / checking (`src/check.rs`, `src/ast.rs`)
- **R4.** `PolyType = Concrete(Type) | Var(TyVarId) | Array(Box<PolyType>, Len)` with `Len = Concrete(u32) | Var(LenVarId)`; `PolySig { row_in, inputs, outputs, row_out, bounds: Vec<(TyVarId, Bound)> }`, `Bound ∈ {Copy, Ord}`. `WordDef` gains optional `PolySig` (`None` when monomorphic). `sig_of` unchanged; new `poly_sig_of` builds a `PolySig` when variables are present.
- **R5.** Call-site unification (deepest-first) of `PolySig.inputs` against concrete `Slot` types plus `row_in` against the deeper prefix, producing ground `θ: {TyVar→Type, LenVar→u32, RowVar→Vec<Type>}`. A repeated `'T`/`'N` must unify identically (X4). Underflow and non-array-where-`['T 'N]`-expected reuse existing errors. θ applied to outputs (`row_out` expanded to `θ(row_in)`); concrete `Type`s pushed; downstream checks unchanged.
- **R6.** Bound checking at the instantiation: `Copy` via `is_copy`, `Ord` via the numeric-tower/total-order predicate. Failure is a located call-site error naming variable, concrete type, capability (X5 Copy, with linear-spine reason; X6 Ord).
- **R7.** `check_poly_body`: a dedicated pass over a `PolyType` stack, separate from `infer_line`, with **no catch-all**. `is_copy`/`is_linear`/`is_aggregate` gain no arm and are never reached with a variable. `Concrete(t)` delegates to existing predicates on `t`. A bare `Var(v)` supports only: the five core shuffles, an operation its bound set permits (`dup`/`over` need `Copy` → X7; `>`/`max` need `Ord` → X8), local bind/read, being passed to a call slot that is the same variable, and being returned; every other type-directed op on a bare `Var` errors naming `v`. Length variables opaque except through length-agnostic `len`/`&>`/`fill`/`@`; row variable pass-through only. Forgetting a bare linear `Var` reuses must-consume. Branch joins lift `infer_line`'s rule to `PolyType` (in-slice bodies are straight-line). Polymorphic-calls-polymorphic is out of scope (R14).
- **R8.** Each distinct ground θ recorded in a per-module specialization set (structural dedup, like `intern_array_type`). Bundle structs interned at check time into `module.structs`, gated on **concrete output count ≥ 2** (not on polymorphism), deduped by output tuple; the specialization carries the bundle `StructId`. Lowering only reads `module.structs`.

### Lowering: monomorphization + multi-output ABI (`src/ir.rs`, `src/backend/qbe.rs`)
- **R14.** Checker emits `instantiations: HashMap<Span, CallInst>` keyed by call-site `Span` (gains `Hash`; full `term.span` threaded into `lower_call`), passed into `ir::lower` alongside `&Module` (like `find_drop_overloads`). Each `CallInst` carries ground θ, mangled callee symbol, concrete `out_arity`, ordered output `IrType`s. A polymorphic call reads `instantiations[term.span]` and emits `Call` to `CallInst.symbol`; a monomorphic call has no entry and takes the existing `env`/`Resolver` path. In-slice each call-site `Span` resolves to one ground `CallInst`; nested polymorphic calls deferred to Slice 5.
- **R9.** One `IrFunc` per specialization `(word, θ)` under a **mangled** name (reusing `struct_drop_symbol`-style mangling, `src/ir.rs:288`). The mangling is a pure deterministic function of `(word, θ)` and is the single shared source of truth: Phase 2 computes `CallInst.symbol` and Phase 3 emits `IrFunc.name` by calling it on the same `(word, θ)`, so they cannot disagree. Ground θ → concrete array `N`, so length polymorphism is fully discharged; `lower_array_word`/`&>`/`@`/`len` need no length handling. Builtins exempt.
- **R10 (callee side).** Concrete output count ≥ 2 → synthesized bundle struct `__ret$<tuple>` interned at check time into `module.structs` (R8), a `StructDecl` with a new `is_bundle` flag. `build_registries` lays it out and copies `is_bundle` onto a new `StructLayout.bundle` flag (`src/ir.rs:717`, mirroring `has_drop_overload`). `synthesize_aggregate_destructors` (`src/ir.rs:1106`) filters `is_linear && !bundle`, so a bundle acquires **no** drop glue even with a linear field — enforced by construction (interned before the layout pass). `lower_word`'s two touch-points move: the ret projection (`src/ir.rs:1705`) yields the bundle `Struct` type for arity ≥ 2; finalization (`src/ir.rs:1761`) allocs the bundle, stores the top `out_arity` values deepest-first, `Ret`s it. `IrFunc.ret`/`env` `ret_ty` follow. No new IR variant.
- **R11 (caller side).** `lower_call`'s fallthrough (`src/ir.rs:2233`) stops discarding results when `out_arity ≥ 2` **and** a bundle `ret_ty` (`env`) or an R14 entry exists (discriminator is bundle presence, not raw count — the REPL's `ir_arity_env` never interns a bundle, so a REPL multi-output word is out of scope, D2). Monomorphic multi-output caller sources `out_arity`/bundle type from the name-keyed `env`; a polymorphic per-θ instantiation sources them from the R14 `CallInst`. Either way the caller receives one bundle value and **unpacks** it into `out_arity` field loads pushed deepest-first (reverse of R10's pack, via the `S>` destructure path). A linear field is moved out; the bundle shell owns no bytes, runs no destructor (enforced via the `bundle` flag), is never bound to a local or surplus-checked. Same path a row variable reaches. **One mechanism, D4 satisfied.**

### `max` / `max-total` (`src/check.rs`, `src/ir.rs`, `src/backend/qbe.rs`)
- **R12. `max` (integers).** New builtin arm in checker dispatch and `lower_call`: signature `( 'T 'T -- 'T )` with internal `Ord` bound, accepted for the integer tower (`i8..i64`, `u8..u64`, `usize`, `isize`), rejected for floats (X9 naming `max-total`) and type-mismatch/non-`Ord`. Lowers inline to `Cmp(Gt)` + select on the concrete width. No `Call`, no monomorphization.
- **R13. `max-total` (floats).** New builtin arm accepting two `f32` or two `f64`, lowering to a total-ordered maximum by the `total_cmp` bit-pattern rule (monotone key: flip all bits if sign set, else flip only the sign bit, then integer-compare), inline. Non-floats error (X10 directing to `max`). Emits no float `>`.

### Regression
- **R15.** Addition-only: no existing golden or unit test changes expected output; `Slot` stays concrete; `is_copy`/`is_linear` untouched. Verified by the unmodified suite plus the `stack.sth`-diff goldens.

## Success criteria (all → runnable goldens; Xn → behavioural negative tests)

1. Polymorphic `dup`/`swap` on `i64`, `bool`, a struct in one run (pins S3/R2).
2. `: pair ( i64 -- i64 i64 ) dup ;`, `5 pair . .` prints `5` `5` (recon-3 panic closed) — R10, R11.
3. A `'T: Copy` word called at two concrete types, prints both — R1, R4–R7, R9, R14.
4. A length-polymorphic word over `[i64 4]` and `[i64 8]` runs — R1, R5, R9.
5. A row-variable word (`( ..s 'a 'b -- ..s 'a 'b 'a 'b )`, `'a`/`'b: Copy`) runs (≥2-output row expansion) — R1, R5, R9, R10, R11, R14.
6. `max` over `i64`, `u8`, `usize` — R12.
7. `max-total` over `f64` and `f32` — R13.
8. Dogfood `stack.sth` rewritten, output unchanged `3 3 2 1 16` — R10, R11, S5.
9. A two-output word's emitted body ends in one struct `Ret` with both outputs stored (structural/emitted-IR) — R10.
10. A two-output word with a linear output field (`( -- ^i64 i64 )` via a Phase 3 owned cell) frees the cell exactly once, interned bundle carries no destructor (golden + structural) — R10, R11.
- **X1** one `'`-name in both type and count slot. **X2** `..s` non-deepest/twice. **X3** bound on a use occurrence / unknown capability. **X4** `'T`/`'N` forced to two concretes (names both). **X5** `'T: Copy` with a linear type (names variable, type, linear reason). **X6** `'T: Ord` with a non-`Ord` type. **X7** `dup` of unbounded `'T` (names missing `Copy`). **X8** `>` on unbounded `'T` (needs `Ord`). **X9** `max` on floats (names `max-total`). **X10** `max-total` on integers (names `max`).

Goldens in `tests/phase4_generics.rs`; unit tests beside each stage.

## Dogfood

`examples/stack.sth` rewritten: `pop`/`peek` return `( Stack -- Stack i64 )` directly; `type: Popped` and every `Popped>` destructure deleted; `main` calls `pop .`/`peek .`. Output unchanged (`3 3 2 1 16`). `Stack` is all-`Copy`; the trailing `drop` discards the items array from `Stack>items`. **D7 finding (reported):** polymorphic `dup`/`swap` simplify no existing example (already accepted every type); `max`/`max-total` touch none (no checked-in program computes a maximum). The real example-level win is the multi-output ABI removing hand-bundled result structs; `list.sth`/`vm.sth` are the same pattern, left untouched to keep the dogfood to one example.

## Non-functional / invariants

- Green unchanged: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- No new `Instr`/`Terminator`; ABI reuses `alloc_struct`/`Blit`/field-load and `Call`'s existing `Option<Value>`.
- `Type` gains no variant; `Slot` stack stays concrete.
- Backend stays QBE; `Ptr` opaque; bundle rides QBE's existing aggregate C-ABI. `core` stays `no_std`; no JIT/comptime/inliner. Monomorphization is compile-time only; no runtime dispatch.

## Out of scope

Quotations/`call` and the combinator library (Slices 4–5); the inliner (Slice 5); `+` overloading, multimethods, `if`-combinator, `Bool`-as-enum (Slices 6–7); REPL monomorphization and REPL-defined polymorphic words (Slice 2); generic `type:` declarations (Slice 3); polymorphic `drop` (Slice 6); migrating `list.sth`/`vm.sth`; HM inference (unification is one-directional, declared sig vs concrete stack); user-writable `Ord` beyond the numeric tower.

## Key risks (all addressed)

- **Bundle disposal (R11).** Enforced, not asserted: the `bundle` flag makes `synthesize_aggregate_destructors` skip it; fields moved out via `S>`; live case pinned by criterion 10 (`( -- ^i64 i64 )` frees the cell exactly once).
- **Opaque variable placeholders (R7).** A bare `Var` satisfies no concrete-type predicate and answers copy/Ord only from its bound set; every other op rejected (X7, X8).
- **Branchy polymorphic bodies (R7, deferred).** `if` in a polymorphic body is explicitly rejected at check time (the `TermKind::If` arm of `poly_term` returns a located diagnostic before touching the stack or scope). Implementing it properly for polymorphic bodies means mirroring the monomorphic `if` arm: pop the condition off the `PolyType` stack, run a per-arm unconsumed-linear check, and join the two arms' move-state; none of that is lifted to `PolyType` yet, and a partial version is worse than none. Before this rejection was added, the half-built arm could both spuriously reject valid programs (e.g. `: choose ( 'T 'T bool -- 'T ) | a b flag | flag if a b drop else b a drop end ;`, whose monomorphic sibling builds) and panic the compiler on others (a `^i64` allocated on one arm reaches `ir.rs`'s `drop: non-empty stack`). A future slice implementing all three pieces together is the way to enable it.
- **Instantiation explosion.** K shapes → K `IrFunc`s; acceptable at this scale, structural dedup only; flagged for Slice 5.
- **Row-variable scope creep.** `..s` passed through opaquely; only consumer is criterion 5; lowering fully subsumed by R9+R10+R11.

## Current-state anchors (`9f8644c`)

`Sig`/`sig_of` `src/check.rs:22-31`; `check_shuffle` `:4717`, `is_copy` `:177`, `is_linear` `:204`; `lower_call` shuffle arms `src/ir.rs:2032`+; multi-output desync `:2233-2246`; `lower_word` finalization `:1761`, ret projection `:1705`; `Arity` `:961`, `Instr::Call` `:861`; `vm-pop` struct return `examples/vm.sth:54`; `struct_drop_symbol` `src/ir.rs:288`; parse entry points `src/parser.rs:644`/`:663`/`:709`/`:804`; `Term`/`Span` `src/ast.rs:598`/`:5`, `lower_term` passes `term.span.line` `src/ir.rs:2015`; `StructLayout` `src/ir.rs:188`/`:195`, `synthesize_aggregate_destructors` `:1106`/`:1122`, `has_drop_overload` copy `:717`.

## Delivery (implemented)

- **Phase 1 — multi-output aggregate-return ABI (S2), monomorphic path only.** Bundle interning + `is_bundle`/`bundle` flags, `lower_word` pack + `lower_call` unpack, closes the recon-3 panic. Exit: criteria 2, 9, 10. (`0465e5be`, review `13fda235`; touched `src/ast.rs`, `src/check.rs`, `src/ir.rs`, `src/parser.rs`, `src/repl.rs`, `tests/phase4_generics.rs`.)
- **Phase 2 — checker-side variable machinery (S1/S4).** `PolyType`/`PolySig`, lex/parse `'T`/`'N`/`..s` + bounds, unification/substitution, bound checking, `check_poly_body`, instantiation recording, R14 table. Checker-only. Exit: X1–X8. (`17c0699e`; `src/ast.rs`, `src/check.rs`, `src/parser.rs`.)
- **Phase 3 — monomorphization lowering (R9).** One mangled `IrFunc` per instantiation, call sites resolved via the R14 table, length variables discharged by concrete `N`. Exit: criteria 3, 4, 5. (`01475134`, plus `1aae8a92` independent-symbol computation and `88cba2ca` deterministic ordering; `src/ir.rs`, `tests/phase4_generics.rs`.)
- **Phase 4 — `max`/`max-total` builtins (S6/D6).** Inline integer `max` (X9) and `total_cmp` float `max-total` (X10). Exit: criteria 6, 7. (`f7e53cb2`, review `4bf14158`, test widening `f9c1fb78`; `src/check.rs`, `src/ir.rs`, `tests/phase4_generics.rs`.)
- **Phase 5 — dogfood + docs (S5/D7).** `examples/stack.sth` rewritten (`Popped` deleted, output unchanged), D7 finding recorded, addition-only regression check. Exit: criteria 1, 8. (`e9972368`; `ROADMAP.md`, `examples/stack.sth`, `tests/phase4_generics.rs`.)
