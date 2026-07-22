# Phase 0 spec — codegen spine

Technical specification and phased delivery plan for **Phase 0** of the Sooth
compiler. This document is downstream of, and must not contradict:

- [phase0-brief.md](./phase0-brief.md) — the discovery document; resolves every
  Phase 0 decision. Where this spec and the brief agree, the brief is the source of
  truth; where this spec adds detail (data types, algorithms, `path:line` anchors),
  it is filling in implementation, not overriding a decision.
- [../DESIGN.md](../DESIGN.md) — load-bearing invariants (QBE backend; backend-neutral
  IR with `Ptr[T]` abstract; affine spine; `no_std` layering; no loop keywords;
  `if/else/then`; no in-process JIT).
- [../ROADMAP.md](../ROADMAP.md) — Phase 0 is "Codegen spine". Everything past the
  Phase 0 boundary is out of scope here.
- [../CLAUDE.md](../CLAUDE.md) — test-coverage discipline and start-small growth
  structure.

## 1. Goal

Prove the core architectural bet end to end:

```
source → lex → parse → stack-effect (arity) check → backend-neutral IR
       → QBE IL → qbe → cc → native binary
```

This is a go/no-go on the codegen architecture (ROADMAP: highest risk). It is a
**craft** slice: build exactly what the two goldens plus the one negative golden
need, and no abstraction beyond that.

## 2. Exit criteria (encode all three)

1. `examples/gcd.sth` compiles to a standalone native binary that prints `5`.
2. `examples/factorial.sth` compiles to a standalone native binary that prints `120`.
3. One negative golden: a stack-effect mismatch produces the expected diagnostic
   (the *right* error, tested as behaviour, not merely a non-zero exit).

`examples/lerp.sth` is an additional positive golden (prints `30`) that keeps the
`| … |` locals path covered now that `gcd`/`factorial` are point-free; it is coverage,
not a fourth architectural criterion.

Every stage (lex / parse / check / lower / emit) gets `#[cfg(test)] mod tests` beside
it: happy path plus at least one error/edge case. Naming: `thing_condition_expected`.

## 3. Scope guardrails

**In (Phase 0 surface, exactly as the brief fixes it):**

- Word defs `: name ( effect ) | locals | body ;`.
- Stack effect `( in... -- out... )`, slots written as bare types (`int`); the only
  type is `int`; the checker verifies **arity** (slot count), not types. A slot may
  carry a name (`a:int`) as caller-facing documentation, but a slot bound by `| … |`
  stays a bare type: types live in the effect comment, names live in `| … |`, never
  both for the same slot.
- Named locals `| a b |`: bind the top N stack items **left-to-right in effect
  order** and consume them; `int` is `Copy`, so a local may be referenced any number
  of times. This is the only place a bound slot is named.
- Decimal `i64` literals, optional leading `-`.
- Builtins: `+ - * mod` and `= < >`, each `( int int -- int )` (comparisons yield
  `1`/`0`); `.` `( int -- )` printing via libc `printf("%ld\n", …)`; and the core
  stack shuffles `dup ( int -- int int )`, `drop ( int -- )`,
  `swap ( int int -- int int )`, `over ( int int -- int int int )`,
  `rot ( int int int -- int int int )`. The shuffles are monomorphic here (all `int`);
  they lower to pure value-stack juggling with **no IR op of their own** (reorder,
  reuse, or discard SSA value ids), so they add surface without adding a codegen path.
- Locals are opt-in: with the shuffles above, most one- or two-value words stay
  point-free; `| … |` is for when shuffling reads worse than names (roughly three-plus
  reused values, as in `examples/lerp.sth`).
- Truth-as-int: no `bool`; `if` pops one `int` and treats nonzero as true.
- Control flow `if ... else ... then` only. No loop keywords.
- Shallow self-recursion (checked against the word's declared effect). No TCO.
- Comments `\` to end of line. `( … )` is reserved for the effect header.
- Entry point: a word `main ( -- )`; the driver emits a C `main` that calls it.

**Out (do not build, do not design for):** heap, structs/enums, `bool`, any type
beyond `int`, affine/move semantics, deterministic drop, polymorphism, quotations /
combinators, loops / the internal loop primitive, TCO, REPL, multi-value returns,
WASM, `no_std` packaging. `Ptr` exists in the IR type model as an opaque handle
(invariant) but is **not exercised**.

## 4. Codebase map (scaffold anchors)

The module layout stays as-is (group by compiler stage). Each stub is filled; do not
add modules unless a genuine growth signal appears (§5.4).

| Stage | File | Current stub | Target |
|---|---|---|---|
| entry / CLI | `src/main.rs:25` | dispatch to `driver` | unchanged |
| lexer | `src/lexer.rs:14` | `lex` → `todo!()` | tokenise + spans |
| parser | `src/parser.rs:6` | `parse` → `todo!()` | tokens → `Module` |
| AST | `src/ast.rs:29` | `Term` enum, `WordDef` | see §5 changes |
| checker | `src/check.rs:13` | `check` → `todo!()` | arity simulation + diagnostics |
| IR | `src/ir.rs:27` | `lower` → `todo!()`; `IrModule`/`IrFunc`/`Ptr` skeletal | fill IR types + lowering |
| QBE emit | `src/backend/qbe.rs:10` | `emit` → `todo!()` | IR → QBE IL text |
| backend mod | `src/backend.rs` | re-exports `qbe` | unchanged |
| driver | `src/driver.rs:20` | `build`/`run`/`repl` return `Err` | wire pipeline + shell out |
| goldens | `tests/phase0.rs` | two `#[ignore]` tests | enable + add negative golden |
| goldens (src) | `examples/gcd.sth`, `examples/factorial.sth` | complete programs | unchanged |

## 5. Deliberate scaffold changes

These are the only structural edits Phase 0 requires. Each is justified; do not
generalise beyond them.

### 5.1 Remove the `BeginUntil` term (`src/ast.rs`)

`Term::BeginUntil { body }` (`src/ast.rs:38`) and its `//! begin … until` comment
contradict a load-bearing invariant: **Sooth has no loop keywords**; Phase 0 iterates
only via shallow recursion. It is a scaffold leftover with no path to being reached.
Delete the variant. (This is removing dead, invariant-violating code, not
restructuring the module layout.)

### 5.2 Give `WordDef` a `locals` field (`src/ast.rs`)

`| a b |` has nowhere to live in the current `WordDef` (`src/ast.rs:9`). Add:

```rust
pub struct WordDef {
    pub name: String,
    pub effect: StackEffect,
    pub locals: Vec<String>,   // names bound by `| … |`, in effect order; empty if absent
    pub body: Vec<Term>,
}
```

A local **reference** in the body stays a `Term::Call(name)`; the checker and lowering
resolve `name` against `locals` first, then against defined/builtin words (the AST
comment at `src/ast.rs:31` already anticipates this dual role).

### 5.3 Attach source spans to terms (`src/ast.rs`, `src/lexer.rs`, `src/parser.rs`)

Localised diagnostics are a Phase 0 requirement, not a later nicety (CLAUDE:
"Diagnostics are behaviour … localised compile errors start at Phase 0"), and the
negative golden must name a location. This warrants a minimal span facility:

```rust
// src/ast.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span { pub line: u32, pub col: u32 }   // 1-based

// Convert the `Term` enum into kind + span:
pub struct Term { pub kind: TermKind, pub span: Span }
pub enum TermKind { IntLit(i64), Call(String), If { then_branch: Vec<Term>, else_branch: Vec<Term> } }
```

Lexer tokens carry a `Span`: `lex` returns `Vec<(Token, Span)>` (keep the `Token` enum
itself unchanged). The parser copies the originating token's span onto each `Term`.
This is confined to three files and does not touch the module layout. Do **not** build
a fancy gutter/caret renderer; a one-to-two-line `file:line:col` message is the Phase 0
target (§9.3). Rich rendering is on the cross-cutting tooling track, not Phase 0.

### 5.4 IR types (`src/ir.rs`)

`IrModule`/`IrFunc`/`Ptr` are skeletal; fill them per §7. Keep `Ptr` opaque
(`src/ir.rs:25`); it is defined but unused in Phase 0. No other IR module split.

## 6. Delivery sub-phases

Ordered; each is independently testable and anchored to scaffold files. A less
capable implementer should be able to execute each without re-exploring.

### P0.1 — Lexer (`src/lexer.rs`)

**Responsibility:** UTF-8 source → `Vec<(Token, Span)>`.

- Tokens (existing enum): `Colon` `:`, `Semicolon` `;`, `LParen` `(`, `RParen` `)`,
  `Pipe` `|`, `Int(i64)`, `Word(String)`.
- Whitespace (space, tab, newline, CR) separates tokens and is discarded; track
  line/col to build spans (1-based).
- Comment: `\` (a backslash token that is its own word, i.e. surrounded by
  whitespace) starts a comment to end of line. Match Forth convention: `\` begins a
  comment only when it is a standalone token; keep the rule simple (a `\` word token
  ⇒ skip to newline).
- Integer: optional leading `-` followed by ASCII digits, parsed to `i64`. A bare `-`
  (not followed by a digit) is the `Word("-")` builtin. `parse::<i64>` overflow ⇒
  lex error.
- Everything else non-whitespace, non-delimiter is a `Word` (so `+`, `mod`, `dup`,
  `swap`, `drop`, `over`, `rot`, `gcd`, `=`, `<`, `>`, `.`, `main` are all `Word`). `:` `;` `(` `)` `|` are single-char
  tokens.

**Unit tests (`thing_condition_expected`):**

- `lex_word_definition_tokenises` — `: sq ( int -- int ) | n | n n * ;` yields the
  expected token stream.
- `lex_negative_integer_is_int` — `-5` ⇒ `Int(-5)`; `-` alone ⇒ `Word("-")`.
- `lex_backslash_comment_skips_to_eol`.
- `lex_integer_overflow_is_error` — a > i64 literal returns `Err`.

### P0.2 — Parser + AST (`src/parser.rs`, `src/ast.rs`)

**Prereq:** apply §5.1–5.3.

**Responsibility:** tokens → `Module { words }`.

Grammar (Phase 0):

```
module   := worddef*
worddef  := ':' Word '(' effect ')' locals? term* ';'
effect   := slot* '--' slot*
slot     := Word (':' Word)?          # bare `int` (default), or `name:int` doc form; ty ignored beyond arity
locals   := '|' Word* '|'
term     := Int | Word | if
if       := 'if' term* ('else' term*)? 'then'
```

- The stack effect is parsed from inside `( … )`. Store `TypedSlot { name, ty }`
  (`src/ast.rs:23`); `ty` defaults to `"int"`. Phase 0 uses only slot **count**.
  Slot syntax: `name:int` → name=Some, ty="int"; a bare token is treated as a slot
  (its text in `name` is fine; ty="int"). Keep it permissive: the checker only counts.
- `if`/`else`/`then` are recognised as reserved words **in body position** (they are
  `Word` tokens from the lexer; the parser gives them meaning). `else` is optional;
  absent ⇒ empty `else_branch`.
- A body `Word` that is not `if`/`else`/`then` becomes `Term::Call(text)` (covers
  builtins, user words, self-calls, and local references alike — resolution is the
  checker's job).
- Errors: missing `;`, unterminated `(`/`|`, `then` without `if`, `else`/`then`
  without an open `if`, EOF mid-definition — each an `Err(String)` with a span.

**Unit tests:**

- `parse_gcd_shape_matches_ast` — parses `examples/gcd.sth` (point-free: shuffle words
  are plain `Term::Call`s, nested `if`, empty `locals`).
- `parse_locals_block_populates_locals` — `examples/lerp.sth` parses with
  `locals == ["a", "b", "t"]`.
- `parse_if_without_else_has_empty_else_branch`.
- `parse_missing_semicolon_is_error`.
- `parse_then_without_if_is_error`.

### P0.3 — Stack-effect (arity) checker (`src/check.rs`)

**Responsibility:** simulate the compile-time virtual stack (depth only, all slots
`int`) through each word body; verify the declared effect equals the computed net
effect; unify `if`/`else` join points. Return `Ok(())` or the first `Err(String)`
diagnostic.

Build a word table first: every user word name → declared `(in_arity, out_arity)`,
seeded with the builtins:

| word | in | out |
|---|---|---|
| `+` `-` `*` `mod` | 2 | 1 |
| `=` `<` `>` | 2 | 1 |
| `.` | 1 | 0 |
| `dup` | 1 | 2 |
| `over` | 2 | 3 |
| `swap` | 2 | 2 |
| `rot` | 3 | 3 |
| `drop` | 1 | 0 |

The shuffles need no special simulation: the generic `depth += out - in` handles them
(e.g. `dup` +1, `drop` -1, `swap`/`rot` 0).

Simulation per word body, tracking an integer `depth`:

- Entry: `depth = 0`. If `locals` present, the `| … |` binding **pops** `locals.len()`
  (these are the top N inputs); the declared inputs supply them, so start with
  `depth = inputs.len()` then subtract the locals pop (net `depth = inputs.len() -
  locals.len()`, which is `0` when every input is bound, as in the goldens). If
  `locals.len() > inputs.len()`, error (`check_locals_exceed_inputs_is_error`).
- `TermKind::IntLit` ⇒ `depth += 1`.
- `TermKind::Call(name)`:
  - if `name` ∈ `locals` ⇒ local reference ⇒ `depth += 1`;
  - else look up `(in, out)` in the word table. If `depth < in` ⇒ underflow error
    (see §9.3). Else `depth += out - in`.
  - self-calls resolve through the same table (the word is registered before its body
    is checked), so recursion is checked against the declared effect.
  - unknown name ⇒ `check_unknown_word_is_error`.
- `TermKind::If { then_branch, else_branch }`: the `if` pops one `int`
  (`depth -= 1`; underflow if `depth == 0`). Simulate each branch from the post-pop
  depth independently; let their resulting depths be `d_then`, `d_else`. If
  `d_then != d_else` ⇒ branch-depth mismatch error
  (`check_branch_depth_mismatch_is_error`). Otherwise `depth = d_then` (the unified
  join depth).
- End of body: computed net = final `depth`. Require `depth == outputs.len()`. Else
  net-effect mismatch error.

**Unit tests:**

- `check_gcd_is_ok`, `check_factorial_is_ok` (both now exercise the shuffle words),
  `check_lerp_is_ok` (exercises `| … |` locals).
- `check_stack_underflow_is_error` — the `oops` program (§9.3).
- `check_branch_depth_mismatch_is_error` — `if` arms leave unequal depth.
- `check_declared_output_mismatch_is_error` — body leaves 2, declares 1 out.
- `check_unknown_word_is_error`.

### P0.4 — IR lowering (`src/ir.rs`)

**Responsibility:** `Module` → `IrModule` (§7 defines the types). One `IrFunc` per
word. Re-run the virtual-stack simulation, but this time each slot carries an SSA
value id instead of just contributing to a depth count. `if`/`else` become blocks with
a conditional branch and a join block whose differing slots become **phi** nodes.

The checker has already proven arity, so lowering may assume well-formed input (no
defensive re-checking beyond what it needs to read slots).

Algorithm sketch (see §7 for the emitted shapes):

- Params: one `IrType::Int` per declared input; result: `Some(Int)` if
  `outputs.len() == 1`, `None` if `0`. (Phase 0 words are single-out or none; the
  checker guarantees this for the goldens.)
- Maintain `stack: Vec<Value>` and a fresh-value/fresh-block counter.
- Local binding `| … |`: pop the top N param values into a name→Value map, in effect
  order (leftmost local = deepest of the N).
- `IntLit(n)` ⇒ emit `Const(v, n)`; push `v`.
- `Call(name)`:
  - local ⇒ push its mapped `Value` again (int is `Copy`; reuse the same value id);
  - `dup` ⇒ pop 1, push it twice (reuse the value id; no IR op);
  - `drop` ⇒ pop 1, discard it (no IR op; the popped id just goes unused);
  - `swap` ⇒ pop 2, push them in swapped order (no IR op);
  - `over` ⇒ push a second copy of the value one below the top (reuse its id; no IR op);
  - `rot` ⇒ rotate the top three (`a b c` → `b c a`; no IR op);
  - `+ - * mod` ⇒ pop 2, emit `Bin`, push result;
  - `= < >` ⇒ pop 2, emit `Cmp`, push result (0/1 as an `Int` value);
  - `.` ⇒ pop 1, emit `Print`;
  - user word ⇒ pop `in` args, emit `Call(Some(ret)/None, name, args)`, push ret if any.
- `If`: pop the test value; create `then`, `else`, `join` blocks; terminate the
  current block with `Jnz(test, then, else)`. Lower each branch over a **cloned**
  stack; at the end of each branch, `Jmp(join)`. For each stack position where the
  two branches produced different `Value`s, emit a `Phi` in `join` selecting by
  predecessor block; positions that are identical across branches pass through
  unchanged. Continue in `join`.
- End: terminate with `Ret(Some(top))` or `Ret(None)`.

**Unit tests:**

- `lower_square_has_one_mul` — `: sq ( int -- int ) | n | n n * ;` lowers to a func
  with one `Bin(Mul)` and `Ret(Some(_))` (covers `| … |` local binding + reuse).
- `lower_dup_reuses_value_id` — `dup` pushes the same `Value` twice, emitting no instr.
- `lower_swap_reorders_without_instr` — `swap` reverses the top two, emitting no instr.
- `lower_drop_pops_without_instr` — `drop` shrinks the stack, emitting no instr.
- `lower_if_emits_phi_at_join` — a word with `if … else … then` that leaves a value
  produces a `Phi` in the join block.
- `lower_print_emits_print_instr`.

### P0.5 — QBE emit (`src/backend/qbe.rs`)

**Responsibility:** `IrModule` → QBE IL text (a `String`). See §8 for the concrete
IL, including the `printf` FFI and `%ld\n` data string. `main` is emitted as a QBE
function named `sooth_main` (the driver supplies the C `main`, §9.2).

**Unit tests:**

- `emit_square_contains_mul_and_ret` — emitted IL contains `mul` and `ret`.
- `emit_print_uses_printf_and_fmt` — output contains `data $fmt = { b "%ld\n", b 0 }`
  and a `call $printf(...)`.
- `emit_if_has_jnz_and_phi`.
- Round-trip check (may live in `tests/phase0.rs`): the emitted IL for `gcd` is
  accepted by `/usr/bin/qbe` without error.

### P0.6 — Driver + linking (`src/driver.rs`)

**Responsibility:** wire the stages and shell out. Fill `build` (`src/driver.rs:20`)
and `run` (`src/driver.rs:26`); `repl` (`src/driver.rs:30`) stays a Phase 1 stub.

`build(path)`:

1. `let src = std::fs::read_to_string(path)?;`
2. `let tokens = lexer::lex(&src)?;`
3. `let module = parser::parse(&tokens)?;`
4. `check::check(&module)?;`
5. `let ir = ir::lower(&module)?;`
6. `let ssa = backend::qbe::emit(&ir)?;`
7. Write `ssa` to a temp `*.ssa`; run `/usr/bin/qbe <in.ssa> -o <out.s>`; write the C
   shim (§9.2) to a temp `*.c`; run `cc <out.s> <shim.c> -o <binary>`.
8. Output binary path: alongside the source, named after the source stem (e.g.
   `examples/gcd.sth` → `examples/gcd`). Use a temp dir for `.ssa`/`.s`/`.c`.

`run(path)`: `build`, then execute the produced binary, inheriting stdio; propagate
its exit status.

Shelling out: use `std::process::Command`; check exit status; on non-zero from `qbe`
or `cc`, return `Err` including the captured stderr. `qbe` path is `/usr/bin/qbe`; `cc`
is found on `PATH`.

**Unit tests / notes:** driver logic is thin orchestration; its coverage is the
integration goldens (P0.7). A small unit test may cover the source-stem → binary-path
naming if that logic is factored into a helper.

### P0.7 — Golden / integration tests (`tests/phase0.rs`)

Remove the `#[ignore]` attributes once the pipeline lands. These require `qbe` and
`cc` on the machine (both present).

- `gcd_compiles_and_runs` — `driver::build(Path::new("examples/gcd.sth"))`, execute
  the binary, assert stdout == `"5\n"` and exit 0.
- `factorial_compiles_and_runs` — same for `examples/factorial.sth`, stdout ==
  `"120\n"`. (`gcd`/`factorial` are point-free; the shuffles carry the reuse.)
- `lerp_compiles_and_runs` — `examples/lerp.sth` (the locals golden), stdout == `"30\n"`.
- `stack_effect_mismatch_reports_diagnostic` (the negative golden) — parse+check the
  inline `oops` source (§9.3) and assert the returned `Err` message contains the word
  name `oops`, the operator `` `+` ``, `needs 2 values`, `holds 1`, and the declared
  effect `( int -- int )`. Asserting substrings (not an exact column) keeps the
  golden robust while still testing the *right* error.

## 7. IR specification (`src/ir.rs`)

Backend-neutral, SSA-shaped, minimal. `Value` and `BlockId` are indices.

```rust
pub struct IrModule { pub funcs: Vec<IrFunc> }

pub struct IrFunc {
    pub name: String,               // word name; `main` → emitted as `sooth_main`
    pub params: Vec<IrType>,        // one per declared input
    pub ret: Option<IrType>,        // Some(Int) or None
    pub blocks: Vec<Block>,         // blocks[0] is entry
}

#[derive(Clone, Copy)]
pub enum IrType { Int, Ptr }        // Ptr defined for the neutral-IR invariant; unused in Phase 0

#[derive(Clone, Copy)]
pub struct Value(pub u32);
#[derive(Clone, Copy)]
pub struct BlockId(pub u32);

pub struct Block { pub id: BlockId, pub instrs: Vec<Instr>, pub term: Terminator }

pub enum Instr {
    Const(Value, i64),
    Bin(Value, BinOp, Value, Value),      // Add Sub Mul Rem
    Cmp(Value, CmpOp, Value, Value),      // Eq Lt Gt  → 0/1
    Call(Option<Value>, String, Vec<Value>),
    Print(Value),                          // libc printf("%ld\n", v)
    Phi(Value, Vec<(BlockId, Value)>),     // join-point merge
}

pub enum BinOp { Add, Sub, Mul, Rem }
pub enum CmpOp { Eq, Lt, Gt }

pub enum Terminator {
    Ret(Option<Value>),
    Jnz(Value, BlockId, BlockId),          // nonzero → first, else → second
    Jmp(BlockId),
}
```

Keep `Ptr(pub u32)` (`src/ir.rs:25`) as the opaque handle; do not wire it into
Phase 0 lowering. The invariant is that nothing in this module or downstream assumes
a pointer is a native `u64`.

## 8. QBE emit specification (`src/backend/qbe.rs`)

One QBE `function` per `IrFunc`. Types: `l` (i64) for `Int`. Emit one shared data
definition for the print format.

- Preamble (once per module): `data $fmt = { b "%ld\n", b 0 }`.
- Function header: `export function l $NAME(l %p0, l %p1, …)` when `ret == Some(Int)`;
  `export function $NAME(l %p0, …)` (no return type) when `ret == None`.
- `main` is emitted with QBE name `$sooth_main` and no return type.
- Blocks: `@blkN`; entry block first. Every function starts with `@start`.
- `Const(v,n)` → `%vN =l copy N`.
- `Bin` → `add`/`sub`/`mul`/`rem` (signed): `%r =l add %a, %b`.
- `Cmp` → emit the comparison directly into an `l` result (verified against
  `/usr/bin/qbe`): `%r =l ceql %a, %b` (also `csltl` for `<`, `csgtl` for `>`).
  Result is `0`/`1` in an `l` slot; **no `extuw` step is needed**.
- `Call(Some(r), f, args)` → `%r =l call $f(l %a0, l %a1, …)`; `Call(None, …)` →
  `call $f(…)`.
- `Print(v)` → `call $printf(l $fmt, l %v, ...)`. **Verified:** on this `qbe` the `...`
  variadic marker goes **last** in the argument list, not mid-list (matches QBE's own
  `test/vararg2.ssa`); the mid-list form `(l $fmt, ..., l %v)` is rejected. Return
  value ignored.
- `Phi(r, [(b1,v1),(b2,v2)])` → `%r =l phi @b1 %v1, @b2 %v2`.
- `Ret(Some(v))` → `ret %v`; `Ret(None)` → `ret`.
- `Jnz(c,t,e)` → `jnz %c, @t, @e`; `Jmp(b)` → `jmp @b`.

Reference lowering of `gcd` (**verified**: this exact IL, linked with the §9.2 C shim,
compiles via `/usr/bin/qbe` + `cc` and prints `5`):

```
data $fmt = { b "%ld\n", b 0 }

export function l $gcd(l %a, l %b) {
@start
    %c =l ceql %b, 0
    jnz %c, @then, @else
@then
    jmp @join
@else
    %m =l rem %a, %b
    %r =l call $gcd(l %b, l %m)
    jmp @join
@join
    %v =l phi @then %a, @else %r
    ret %v
}

export function $sooth_main() {
@start
    %g =l call $gcd(l 10, l 15)
    call $printf(l $fmt, l %g, ...)
    ret
}
```

## 9. Driver / linking / diagnostics detail

### 9.1 Toolchain invocation

- QBE: `/usr/bin/qbe input.ssa -o output.s` (default target = host).
- Link: `cc output.s shim.c -o binary`. `cc` resolves `printf` from libc; no extra
  flags needed on this host.
- Use `std::env::temp_dir()` or `std::process::Command` with a scratch dir for the
  intermediate `.ssa`/`.s`/`.c`. Clean up on success is optional for a craft tool.

### 9.2 The C entry shim

The Sooth `main` word lowers to `sooth_main`; the driver emits and links this shim so
that C's runtime start-up runs before Sooth code (brief: "the driver emits a C `main`
that calls it"):

```c
extern void sooth_main(void);
int main(void) { sooth_main(); return 0; }
```

### 9.3 Diagnostic format (negative golden)

Phase 0 diagnostics are one-to-two lines, localised by `file:line:col`, and name the
discrepancy. Format for the stack-effect underflow (the `oops` program from DESIGN):

Source (`oops`):

```forth
: oops ( int -- int )
  | a | a a + + ;
```

Expected diagnostic (assert on the substrings, not the exact column):

```
error: stack effect mismatch in `oops` (line 2)
  `+` needs 2 values, but the stack holds 1
  note: declared ( int -- int )
```

Other checker errors follow the same shape, e.g. branch-depth mismatch:

```
error: stack effect mismatch in `NAME` (line L)
  `if` branches leave different stack depths (then: X, else: Y)
```

and net-effect mismatch:

```
error: stack effect mismatch in `NAME` (line L)
  body leaves N values, but ( … ) declares M outputs
```

## 10. Definition of done

- All three exit criteria (§2) pass.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green ("green"
  per CLAUDE.md).
- Each stage has its `#[cfg(test)] mod tests` (happy + error), named
  `thing_condition_expected`.
- `tests/phase0.rs` has the three positive goldens (`gcd`, `factorial`, `lerp`; no
  longer `#[ignore]`) and the negative golden.
- No out-of-scope feature was added; `Ptr` remains opaque and unexercised; module
  layout unchanged except the deliberate edits in §5.

## Phases (JSON)

The `phases` array is what `/implement` parses: each entry needs an integer `phase`
and a string `focus` (`difficulty` optional, `standard`/`hard`); all other keys are
ignored by the pipeline and kept here as the per-stage brief the implementer reads.
The seven Phase 0 stages (P0.1–P0.7) map to `phase` 1–7.

```json
{
  "roadmapPhase": "0",
  "title": "Codegen spine",
  "pipeline": "source -> lex -> parse -> stack-effect(arity) check -> backend-neutral IR -> QBE IL -> qbe -> cc -> native binary",
  "exitCriteria": [
    "examples/gcd.sth compiles to a native binary that prints 5",
    "examples/factorial.sth compiles to a native binary that prints 120",
    "a stack-effect mismatch produces the expected diagnostic (negative golden)"
  ],
  "scaffoldChanges": [
    { "file": "src/ast.rs", "change": "remove Term::BeginUntil (violates no-loop-keywords invariant)" },
    { "file": "src/ast.rs", "change": "add WordDef.locals: Vec<String>" },
    { "file": "src/ast.rs", "change": "split Term into { kind: TermKind, span: Span }; add Span" },
    { "file": "src/lexer.rs", "change": "lex returns Vec<(Token, Span)>" },
    { "file": "src/parser.rs", "change": "attach spans to terms" },
    { "file": "src/ir.rs", "change": "fill IrModule/IrFunc + Block/Instr/Terminator; keep Ptr opaque and unused" }
  ],
  "phases": [
    {
      "phase": 1,
      "focus": "Lexer",
      "effort": "S",
      "difficulty": "standard",
      "id": "P0.1",
      "files": ["src/lexer.rs"],
      "deliverables": [
        "lex source -> Vec<(Token, Span)> with 1-based line/col",
        "tokens: Colon Semicolon LParen RParen Pipe Int Word",
        "backslash comment to end of line",
        "signed decimal i64 literals; bare '-' is Word",
        "i64 overflow is a lex error"
      ],
      "tests": [
        "lex_word_definition_tokenises",
        "lex_negative_integer_is_int",
        "lex_backslash_comment_skips_to_eol",
        "lex_integer_overflow_is_error"
      ],
      "dependsOn": []
    },
    {
      "phase": 2,
      "focus": "Parser + AST",
      "effort": "M",
      "difficulty": "standard",
      "id": "P0.2",
      "files": ["src/parser.rs", "src/ast.rs"],
      "deliverables": [
        "apply ast changes 5.1-5.3",
        "parse ': name ( effect ) | locals | body ;'",
        "parse if/else/then (else optional)",
        "body Word (not if/else/then) -> Term::Call (shuffles included)",
        "structural parse errors with spans"
      ],
      "tests": [
        "parse_gcd_shape_matches_ast",
        "parse_locals_block_populates_locals",
        "parse_if_without_else_has_empty_else_branch",
        "parse_missing_semicolon_is_error",
        "parse_then_without_if_is_error"
      ],
      "dependsOn": [1]
    },
    {
      "phase": 3,
      "focus": "Stack-effect arity checker",
      "effort": "M",
      "difficulty": "standard",
      "id": "P0.3",
      "files": ["src/check.rs"],
      "deliverables": [
        "word table seeded with builtins (+ - * mod = < > . dup drop swap over rot)",
        "virtual-stack depth simulation per body",
        "local binding pops N; local reference pushes 1",
        "if pops 1; branches must unify to equal depth",
        "self-call checked against declared effect",
        "declared outputs must equal computed net depth",
        "localised diagnostics per 9.3"
      ],
      "tests": [
        "check_gcd_is_ok",
        "check_factorial_is_ok",
        "check_lerp_is_ok",
        "check_stack_underflow_is_error",
        "check_branch_depth_mismatch_is_error",
        "check_declared_output_mismatch_is_error",
        "check_unknown_word_is_error"
      ],
      "dependsOn": [2]
    },
    {
      "phase": 4,
      "focus": "IR lowering",
      "effort": "L",
      "difficulty": "hard",
      "id": "P0.4",
      "files": ["src/ir.rs"],
      "deliverables": [
        "fill IR types per section 7",
        "one IrFunc per word; params from inputs; ret Some(Int)/None",
        "virtual stack carrying SSA Values",
        "builtins -> Bin/Cmp/Print; user words -> Call",
        "shuffles (dup/drop/swap/over/rot) -> value-stack juggling, no IR op",
        "if/else -> Jnz + branch blocks + Phi at join",
        "Ptr stays opaque and unused"
      ],
      "tests": [
        "lower_square_has_one_mul",
        "lower_dup_reuses_value_id",
        "lower_swap_reorders_without_instr",
        "lower_drop_pops_without_instr",
        "lower_if_emits_phi_at_join",
        "lower_print_emits_print_instr"
      ],
      "dependsOn": [3]
    },
    {
      "phase": 5,
      "focus": "QBE emit",
      "effort": "M",
      "difficulty": "hard",
      "id": "P0.5",
      "files": ["src/backend/qbe.rs"],
      "deliverables": [
        "IrModule -> QBE IL text",
        "data $fmt = { b \"%ld\\n\", b 0 }",
        "l-typed functions; main emitted as $sooth_main",
        "Const/Bin/Cmp(=l ceql/csltl/csgtl)/Call/Print(printf, ... last)/Phi/Ret/Jnz/Jmp",
        "output accepted by /usr/bin/qbe"
      ],
      "tests": [
        "emit_square_contains_mul_and_ret",
        "emit_print_uses_printf_and_fmt",
        "emit_if_has_jnz_and_phi"
      ],
      "dependsOn": [4]
    },
    {
      "phase": 6,
      "focus": "Driver + linking",
      "effort": "S",
      "difficulty": "standard",
      "id": "P0.6",
      "files": ["src/driver.rs"],
      "deliverables": [
        "wire lex->parse->check->lower->emit",
        "write .ssa; run /usr/bin/qbe -> .s",
        "emit C shim 'int main(){ sooth_main(); return 0; }'",
        "run cc .s shim.c -o binary; binary named from source stem",
        "run: build then execute inheriting stdio",
        "propagate qbe/cc stderr on failure"
      ],
      "tests": [
        "driver_binary_path_from_source_stem"
      ],
      "dependsOn": [5]
    },
    {
      "phase": 7,
      "focus": "Golden integration tests",
      "effort": "S",
      "difficulty": "standard",
      "id": "P0.7",
      "files": ["tests/phase0.rs"],
      "deliverables": [
        "un-ignore the three goldens",
        "gcd prints 5; factorial prints 120; lerp prints 30",
        "negative golden asserts the stack-effect diagnostic substrings",
        "requires qbe and cc present"
      ],
      "tests": [
        "gcd_compiles_and_runs",
        "factorial_compiles_and_runs",
        "lerp_compiles_and_runs",
        "stack_effect_mismatch_reports_diagnostic"
      ],
      "dependsOn": [6]
    }
  ],
  "outOfScope": [
    "heap", "structs/enums", "bool", "types beyond int", "affine/move semantics",
    "deterministic drop", "polymorphism", "quotations/combinators", "loops",
    "internal loop primitive", "TCO", "REPL", "multi-value returns", "WASM",
    "no_std packaging"
  ]
}
```
