# Phase 1: REPL and liveness (condensed)

`cargo run -- repl` starts an interactive `int`-only session: define words, evaluate bare expressions, redefine words, and persist a data stack across lines, all on the real QBE backend via `dlopen` (no interpreter, no JIT). Phase 0 stage functions are reused unchanged for word bodies; only the line surface, a marshalling wrapper, and a compile-load-call driver path are new.

## Execution model (Route C)

Each line goes through the normal pipeline to a **shared object** loaded into the session process:
- **Word def** → `.so` exporting the word, loaded `RTLD_NOW | RTLD_GLOBAL` so later lines resolve it by symbol. Touches no stack.
- **Bare expression** → synthesized wrapper `.so`, `dlsym`'d and called against the persistent stack buffer.

Objects are never `dlclose`d (old generations stay callable); handles retained in a `Vec`. Accepted cost: assemble+link+load per line; a runtime fault in loaded code takes the process down (low risk while `int`-only and statically checked).

## Persistent stack

Driver artifact bridging separately-compiled lines (a deliberate preview of Phase 4's uniform runtime stack, not a breach of the compile-time-only-stack invariant): a `Vec<i64>` (8-byte aligned; slot `i` ↔ index `i`) exposed to the ABI as `*mut u8`, plus a carried virtual stack that in `int`-only Phase 1 is just a slot count `D` (`top = D*8`).

**Whole-stack marshalling:** each expression wrapper loads the entire carried stack (`N = D`), runs the body in registers like a word, writes the stack back, returns the new top. Reuses the checker (simulate from `D`) and word lowering verbatim; no low-water-mark tracking.

## Line-wrapper ABI

```
export function l $sooth_line_{n}(l %stack, l %top)
```
`%stack` = buffer base (`IrType::Ptr`), `%top` = `D*8`. Prologue loads slots `0..D` (deepest first); body lowers terms as a word body; epilogue stores `M = D + net` slots; returns `%top + (M-D)*8`. Host calls it `extern "C" fn(*mut u8, usize) -> usize`, growing `buf` to `≥ M` first, then sets `top`/`D` from the return.

## Neutral IR additions

Three instrs keep `Ptr` opaque (WASM depends on it): `PtrOffset` (QBE `add`), `Load` (`loadl`), `Store` (`storel`). `IrType::Ptr` now used. `lower_line(terms, entry_depth, env) -> IrFunc` builds the `(Ptr,Int)->Int` wrapper (params `%v0`/`%v1`). QBE `emit` gained three arms; existing `l` param/return machinery covered the rest.

## Line surface + checking

- `ast::Line { Def(WordDef), Expr(Vec<Term>) }`.
- `parser::parse_line`: leading `Colon` → `parse_worddef`, else a term sequence to EOF. Trailing tokens after a def are rejected; unterminated def is a normal parse error (no multi-line accumulation).
- `check`: shared depth-simulation helpers reworked to take an arity `env` + light error context (name + optional declared effect) instead of `&WordDef`. `infer_line(terms, entry_depth, env)` returns final depth; carried-stack underflow (`depth < in_arity`) is a reported diagnostic (`error: stack underflow: needs N values, but the stack holds M`). Def path seeds the definee's own arity for self-recursion.

## Redefinition: generation-mangled symbols

Latest-symbol binding: each def of `name` exports `name__gen{K}`. Session keeps `name → { arity, generation, symbol }`. Calls resolve to the callee's **current** mangled symbol; self-reference binds the new generation (recursion). Already-loaded code keeps the callee it was compiled against. Builtins never appear as `Instr::Call`, so never mangled. Distinct generations = distinct symbols, sidestepping `RTLD_GLOBAL` clashes.

## Driver

- `tempfile_dir`/`run_command` elevated to `pub(crate)`.
- `compile_so(ssa, out)`: write `.ssa`, `qbe → .s`, `cc -shared -fPIC`. No C shim (`.so` has no `main`). macOS adds `-Wl,-undefined,dynamic_lookup` behind `cfg!`.

## Session loop (`src/repl.rs`)

`Session { env: HashMap<String, WordEntry>, buf: Vec<i64>, top, libs, seq }`, reads over `impl BufRead`, writes over `impl Write` (in-process testable; `driver::repl()` wires stdin/stdout). Per line:
1. Blank → skip.
2. Any lex/parse/check error → print diagnostic, **mutate nothing**, continue.
3. `Def`: check → bump generation → lower with resolver → emit → `compile_so` → `dlopen`; commit env entry only on success, print `defined <name>`.
4. `Expr`: `infer_line` → lower/emit/compile/load → `dlsym` → grow `buf` → **flush host stdout** (deterministic interleave with loaded-code `printf`) → call → update `top`/`D` → print residual stack.

Output (all on host stdout, diagnostics on stderr): expression prints any `.` output then `stack: <v0> … <vk>` (bottom→top) or `stack: (empty)`; def prints `defined <name>`. `format_stack` is a pure helper. No prompt; EOF exits `Ok`. Raw `dlopen`/`dlsym` FFI keeps zero dependencies.

## Where code lives

New `src/repl.rs` (Session, loop, FFI, mangling/resolution, `format_stack`). `compile_so` + `pub(crate)` helpers in `driver`. `parse_line`/`Line` in parser/ast. Env-driven checking + `infer_line` in check. `Load`/`Store`/`PtrOffset` + `lower_line` + env/resolver threading in ir. Emit arms in `backend/qbe.rs`. `DESIGN.md` records the buffer as a driver artifact / Phase 4 preview; `ROADMAP.md` + `README.md` mark Phase 1 complete.

## Delivered in 5 phases

1. **SO compile + dlopen** (`ef23fd7`), de-risk the mechanism: `compile_so`, `pub(crate)` helpers, raw FFI; compile a word to `.so` and call in-process.
2. **Line surface parse + check** (`11ab484`, `582965d`), `Line`, `parse_line`, env-driven simulation + `infer_line`.
3. **Wrapper IR + QBE** (`b0739e9`), `Load`/`Store`/`PtrOffset`, `lower_line`, resolver threading, emit arms.
4. **Session loop + redefinition** (`af7b808`, `b47e91d`, `72a5445`), Session state, generation mangling, def/expr paths, error recovery, deterministic output.
5. **Goldens + docs** (`60aa24f`, `a761a3e`), `tests/phase1.rs` scripted sessions (define/call, persist, redefine, survive bad line, calculator dogfood), DESIGN/ROADMAP/README.

## Test coverage

Unit tests beside each stage: `parser` (bare/colon/unterminated), `check` (net effect, carried depth, underflow text), `ir` (marshal all in/out, advanced top, resolved-generation call), `qbe` (wrapper signature, load/store present), `driver` (`compile_so_produces_loadable_object`), `repl` (`format_stack` ×2, resolve/redefine generation). Golden sessions in `tests/phase1.rs`, one per exit criterion, spawn the `repl` binary over piped stdin. Green = `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Out of scope / deferred

Out: new types, heap, affine/move, polymorphism, quotations, TCO, dependent recompilation, readline/history, owned backend, WASM, `no_std`, comptime words. Deferred (not exits): minimal-footprint (sub-stack) marshalling, recompiling dependents on redefine, multi-line accumulation, old-object reclamation.

## Risks tracked

macOS flat-namespace cross-`.so` resolution (`-Wl,-undefined,dynamic_lookup`; fallbacks noted); host/loaded-code stdout interleaving (flush-before-call discipline, guarded by test in `a761a3e`); goldens need `qbe`+`cc` present (same as Phase 0).
