# Phase 0 — Codegen spine (condensed, as implemented)

Proves the core architecture end to end. **Status: complete** across 7 sub-phases (P0.1–P0.7).

```
source → lex → parse → arity check → backend-neutral IR → QBE IL → qbe → cc → native binary
```

Downstream of `phase0-brief.md`, `DESIGN.md` (QBE backend; `Ptr[T]` opaque; affine spine; `no_std` layering; no loop keywords; no in-process JIT), `ROADMAP.md`, `CLAUDE.md`.

## Exit criteria
1. `examples/gcd.sth` → binary prints `5`.
2. `examples/factorial.sth` → binary prints `120`.
3. Negative golden: a stack-effect mismatch yields the *right* diagnostic (tested as behaviour).

`examples/lerp.sth` (prints `30`) is extra coverage for the `| … |` locals path (gcd/factorial are point-free). Every stage has `#[cfg(test)] mod tests` (happy + error), named `thing_condition_expected`.

## Scope
**In:** word defs `: name ( effect ) | locals | body ;`; effect `( in -- out )` with bare-`int` slots (checker counts arity only; `name:int` allowed as doc); named locals `| a b |` (bind top N left-to-right in effect order, `int` is `Copy` so reusable); decimal `i64` literals (optional leading `-`); builtins `+ - * mod` / `= < >` (each `(int int -- int)`, comparisons yield `1`/`0`); `.` `(int --)` via libc `printf("%ld\n",…)`; shuffles `dup drop swap over rot` (monomorphic, lower to pure value-stack juggling, **no IR op**); truth-as-int (`if` pops one int, nonzero = true); `if…else…then` only; shallow self-recursion (no TCO); `\` line comments; entry word `main ( -- )`.

**Out:** heap, structs/enums, `bool`, types beyond `int`, affine/move semantics, deterministic drop, polymorphism, quotations, loops, TCO, REPL, multi-value returns, WASM, `no_std` packaging. `Ptr` stays defined-but-unexercised.

## Scaffold changes made
- **§5.1** Removed `Term::BeginUntil` (violated no-loop-keywords invariant).
- **§5.2** `WordDef.locals: Vec<String>` (names in effect order; empty if absent). Local refs stay `TermKind::Call(name)`; resolution (locals → words/builtins) is the checker's job.
- **§5.3** Spans: `Span { line, col }` (1-based); `Term { kind: TermKind, span }`; `TermKind = IntLit(i64) | Call(String) | If { then_branch, else_branch }`. `lex` returns `Vec<(Token, Span)>` (Token enum unchanged); parser copies token span onto each term. No gutter/caret renderer — no filename in any message; parser/lexer errors carry `line:col`, checker errors carry `line` only.
- **§5.4** Filled `IrModule`/`IrFunc`; `Ptr` kept opaque.

## Stages

**P0.1 Lexer** (`src/lexer.rs`) — source → `Vec<(Token, Span)>`. Tokens: `Colon ; ( ) Pipe Int(i64) Word`. Whitespace (unicode-aware) separates & is discarded; `\` standalone word ⇒ skip to EOL; `-`+digits ⇒ `Int`, bare `-` ⇒ `Word`; i64 overflow ⇒ error.

**P0.2 Parser** (`src/parser.rs`, `src/ast.rs`) — tokens → `Module { words }`. Grammar: `worddef := ':' Word '(' effect ')' locals? term* ';'`; effect slots stored as `TypedSlot { name, ty="int" }` (count only); `locals := '|' Word* '|'`; body `if/else/then` recognised in body position (else optional). Non-keyword body `Word` ⇒ `Call`. Errors (missing `;`, unterminated `(`/`|`, dangling `then`/`else`, EOF) carry spans + contextual messages.

**P0.3 Checker** (`src/check.rs`) — word table seeded with builtins:

| word | in→out |
|---|---|
| `+ - * mod`, `= < >` | 2→1 |
| `.` | 1→0 |
| `dup` | 1→2 |
| `over` | 2→3 |
| `swap` | 2→2, `rot` 3→3 |
| `drop` | 1→0 |

Simulate integer `depth` per body: entry `depth = inputs.len() - locals.len()` (error if locals > inputs); `IntLit`/local-ref +1; `Call` checks `depth ≥ in` (else underflow) then `depth += out-in`; unknown name ⇒ error; self-calls resolve via the table (registered before body). `If` pops 1 (underflow if 0), simulates both branches from post-pop depth, requires equal join depth. End: require `depth == outputs.len()`. Shuffles need no special-casing (generic `out-in`).

**P0.4 IR lowering** (`src/ir.rs`) — `Module` → `IrModule`, one `IrFunc` per word; assumes checked input. Re-runs the sim with each slot carrying an SSA `Value`. Params = one `Int` per input; ret `Some(Int)` if 1 output else `None`. Locals popped into name→Value map (leftmost = deepest). `IntLit` ⇒ `Const`; shuffles ⇒ reorder/reuse/discard value ids (no instr); `+ - * mod` ⇒ `Bin`; `= < >` ⇒ `Cmp`; `.` ⇒ `Print`; user word ⇒ `Call`. `If` ⇒ `Jnz(test, then, else)`, lower each branch over a cloned stack, `Jmp(join)`, emit `Phi` at join for differing positions. End ⇒ `Ret`.

**P0.5 QBE emit** (`src/backend/qbe.rs`) — `IrModule` → IL string. One function per `IrFunc`; `Int` = `l`. Once-per-module `data $fmt = { b "%ld\n", b 0 }`. Header `export function [l] $NAME(l %p0,…)`; `main` → `$sooth_main` (no ret). Blocks `@blkN`, entry `@start`. `Const`→`copy`; `Bin`→`add/sub/mul/rem`; `Cmp`→`ceql/csltl/csgtl` into `l` (no `extuw`); `Call`→`call $f(...)`; `Print`→`call $printf(l $fmt, l %v, ...)` (**`...` marker goes last**); `Phi`→`phi @b1 %v1, @b2 %v2`; `Ret`/`Jnz`/`Jmp` direct. Verified against `qbe`.

**P0.6 Driver** (`src/driver.rs`, `src/main.rs`, `src/lib.rs`) — `build`: read → lex → parse → check → lower → emit → write `.ssa` → `qbe in.ssa -o out.s` (resolved via `PATH`, as with `cc`) → write C shim → `cc out.s shim.c -o binary` (named from source stem). `run`: build then exec inheriting stdio, propagate exit status (returns status, does not call `exit()`). Temp dirs unique per build. Non-zero from `qbe`/`cc` ⇒ `Err` with captured stderr. C shim:
```c
extern void sooth_main(void);
int main(void) { sooth_main(); return 0; }
```

**P0.7 Goldens** (`tests/phase0.rs`) — `gcd_compiles_and_runs` (`"5\n"`), `factorial_compiles_and_runs` (`"120\n"`), `lerp_compiles_and_runs` (`"30\n"`), `stack_effect_mismatch_reports_diagnostic` (asserts substrings: `oops`, `` `+` ``, `needs 2 values`, `holds 1`, `( int -- int )`). Added coverage for the driver surfacing checker errors. Requires `qbe` + `cc`.

## IR types (`src/ir.rs`)
`IrModule { funcs }`; `IrFunc { name, params: Vec<IrType>, ret: Option<IrType>, blocks }` (blocks[0] entry). `IrType = Int | Ptr` (Ptr unused). `Value(u32)`, `BlockId(u32)`. `Block { id, instrs, term }`. `Instr = Const | Bin(BinOp: Add Sub Mul Rem) | Cmp(CmpOp: Eq Lt Gt → 0/1) | Call(Option<Value>,String,Vec) | Print | Phi`. `Terminator = Ret(Option) | Jnz(v,t,e) | Jmp(b)`. Invariant: nothing assumes `Ptr` is a native `u64`.

## Diagnostic format
One-to-two lines. No filename in any message: parser/lexer errors are localised by
`line:col`, checker errors by `line` only. Underflow example:
```
error: stack effect mismatch in `oops` (line 2)
  `+` needs 2 values, but the stack holds 1
  note: declared ( int -- int )
```
Branch-depth: `` `if` branches leave different stack depths (then: X, else: Y) ``. Net-effect: `body leaves N values, but ( … ) declares M outputs`.

## Reference gcd IL (real emitter output)
Captured verbatim from `cargo run -- build examples/gcd.sth` (the driver's temp
`out.ssa`); names are `%vN`/`@blkN`/`@start`, and the `0` literal is an explicit
`copy` before the compare — not folded into it. Accepted by `qbe` unmodified.
```
data $fmt = { b "%ld\n", b 0 }

export function l $gcd(l %v0, l %v1) {
@start
	%v2 =l copy 0
	%v3 =l ceql %v1, %v2
	jnz %v3, @blk1, @blk2
@blk1
	jmp @blk3
@blk2
	%v4 =l rem %v0, %v1
	%v5 =l call $gcd(l %v1, l %v4)
	jmp @blk3
@blk3
	%v6 =l phi @blk1 %v0, @blk2 %v5
	ret %v6
}

export function $sooth_main() {
@start
	%v0 =l copy 10
	%v1 =l copy 15
	%v2 =l call $gcd(l %v0, l %v1)
	call $printf(l $fmt, l %v2, ...)
	ret
}
```

## Done when
All three exit criteria pass; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green; every stage has happy+error unit tests; `tests/phase0.rs` holds 3 positive + 1 negative golden (no `#[ignore]`); no out-of-scope feature added; `Ptr` opaque/unexercised; module layout unchanged except §5 edits.
