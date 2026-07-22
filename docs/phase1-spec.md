# Phase 1 spec — REPL and liveness

Implementation spec for the Phase 1 slice. Derived from [phase1-brief.md](./phase1-brief.md),
which resolves the design decisions; read that first. Grounded in the Phase 0 compiler on
`main` and constrained by the invariants in [../DESIGN.md](../DESIGN.md),
[../ROADMAP.md](../ROADMAP.md), and [../CLAUDE.md](../CLAUDE.md).

This spec adds nothing the brief doesn't ask for. Where the brief left a mechanism
under-specified (the exact wrapper ABI, where new code lives, the diagnostic text), this
picks the smallest choice that fits the existing code and records why. Everything in the
brief's **Out of scope** section stays out.

## Goal

`cargo run -- repl` starts an interactive session where you define words, evaluate bare
expressions, redefine words, and watch a data stack persist across lines, all running on
the real QBE backend via `dlopen` (no interpreter, no JIT). Phase 1 stays `int`-only.

Exit criteria are the brief's six, restated as tests in [Test plan](#test-plan).

## What already exists (codebase map)

The Phase 0 pipeline is a clean line of stage functions the REPL reuses unchanged for
word bodies; only the line surface, a marshalling wrapper, and a load-and-call driver
path are new.

- `src/lib.rs` — module list: `ast`, `backend`, `check`, `driver`, `ir`, `lexer`,
  `parser`. Phase 1 adds `repl`.
- `src/main.rs` — CLI dispatch; `Some("repl") => driver::repl()` is already wired
  (`src/main.rs:26`). Usage text already lists `repl` (`src/main.rs:12`).
- `src/driver.rs` — pipeline orchestration.
  - `build` (`src/driver.rs:18`): `read → lex → parse → check → ir::lower → qbe::emit`,
    then writes `out.ssa` + a C shim, runs `qbe` then `cc` to a **native binary**.
  - `run` (`src/driver.rs:49`): `build` + spawn the binary.
  - `repl` (`src/driver.rs:56`): stub returning `Err("repl: not implemented (Phase 1)")`.
  - `C_SHIM` (`src/driver.rs:9`) supplies `main` calling `sooth_main`; **not used** by the
    `.so` path (no `main` in a shared object).
  - `tempfile_dir` (`src/driver.rs:60`) and `run_command` (`src/driver.rs:70`) are private
    helpers the REPL needs; elevate to `pub(crate)`.
- `src/lexer.rs` — `lex` returns `Vec<(Token, Span)>`; already handles ints, words, `: ; ( ) |`
  and `\` comments. Unchanged.
- `src/parser.rs` — `parse` (`src/parser.rs`, top) loops `parse_worddef` over the whole
  token stream, producing `Module { words }`. `parse_term` handles ints, calls, and
  `if/else/then`. Phase 1 adds a `parse_line` entry that also accepts a bare term sequence.
- `src/ast.rs` — `Module`, `WordDef { name, effect, locals, body }`, `StackEffect`,
  `TypedSlot`, `Term { kind, span }`, `TermKind::{IntLit, Call, If}`. Phase 1 adds a `Line`
  enum.
- `src/check.rs` — `check` (`src/check.rs:33`) builds a word→arity table from
  `builtin_table` (`src/check.rs:15`) plus `module.words`, then simulates the compile-time
  virtual stack via `check_terms`/`check_term` (`src/check.rs:100`, `:112`). `Arity =
  (usize, usize)`. `underflow_error` (`src/check.rs:93`) and `effect_str` (`src/check.rs:52`)
  format diagnostics. Phase 1 adds env-driven checking and net-effect inference for a bare
  line.
- `src/ir.rs` — `lower` (`src/ir.rs:83`) → `lower_word` (`:104`) → `FuncBuilder` (`:139`).
  Words become `IrFunc { name, params: Vec<IrType>, ret, blocks }`; the compile-time stack
  is a `Vec<Value>` and shuffles/`dup`/`drop` emit no instructions. `IrType::Ptr`
  (`src/ir.rs:29`) exists but is unused. `Instr` (`:49`) has no memory ops. Phase 1 adds
  `Load`/`Store`/`PtrOffset` and a `lower_line` wrapper builder, and threads an env +
  call-name resolver so a unit can be compiled against previously-loaded words.
- `src/backend/qbe.rs` — `emit` writes one `data $fmt` then a function per `IrFunc`;
  `qbe_name` maps `main → sooth_main`; params and returns are all QBE `l` (64-bit). Phase 1
  adds emit arms for the three new instrs; the function/param machinery already handles a
  two-param `l` wrapper returning `l`.
- `tests/phase0.rs` — golden pattern: `driver::build` then run the binary and assert
  stdout/exit. Phase 1 adds `tests/phase1.rs` driving the `repl` binary over piped stdin.
- `Cargo.toml` — zero dependencies today. Phase 1 keeps it that way (raw `dlopen` FFI, see
  [Driver](#driver-compile-to-shared-object-and-load)).

## Design

### Execution model: compile each line to a shared object, `dlopen` it (Route C)

No interpreter, no JIT (DESIGN: no in-process JIT, no comptime interpreter, no immediate
words). Each REPL line goes through the normal pipeline to a **shared object** loaded into
the session process:

- A **word definition** line compiles to a `.so` exporting the word, loaded with global
  visibility so later lines can call it by symbol. It touches no stack.
- A **bare expression** line compiles to a synthesized wrapper `.so` with a uniform
  signature, loaded, and called immediately against the persistent stack buffer.

Objects are loaded `RTLD_NOW | RTLD_GLOBAL` so a symbol defined by an earlier line resolves
when a later object references it. This is the design's stated cost: an assembler + linker +
load round-trip per line, acceptable for a craft REPL.

### Persistent data stack: a byte buffer plus the carried virtual stack

The stack persists across lines; **no runtime data stack lives inside compiled word
bodies** (the compile-time-virtual-stack invariant holds). Persistence is a driver artifact
bridging separately-compiled lines, held by the session:

- a growable **byte buffer** of raw values, and
- the **carried virtual stack**: the typed slots (type + offset) live at the top of the
  buffer, carried line to line. Phase 1 is `int`-only, so this is just a slot count `D`;
  the buffer's live region is `[0, D*8)`.

Backing storage is a `Vec<i64>` (guarantees 8-byte alignment; slot `i` ↔ index `i`),
exposed to the ABI as `*mut u8`. `top` is a byte length, always a multiple of 8.

**Marshalling model — whole carried stack.** Each expression line compiles to a wrapper
that loads the **entire** current carried stack as its inputs (`N = D`), runs the body in
registers exactly like a word, writes the resulting stack back, and returns the new top.
This is the simplest correct choice: it reuses the existing checker (start the depth
simulation at `D`; underflow past 0 is the carried-stack underflow error) and the existing
word lowering verbatim, with no low-water-mark tracking. `D` is tiny in a REPL, so copying
the whole stack per line is a non-issue. Loading only the touched sub-stack is a deferred
optimization (see [Deferred](#deferred-intended-not-phase-1-exits)).

### Line-wrapper ABI

Uniform signature, per the brief, so every expression line looks identical to the host
regardless of arity and results flow through the buffer, not the C return value:

```
export function l $sooth_line_{n}(l %stack, l %top)
```

- `%stack`: buffer base pointer (an `IrType::Ptr`). `%top`: live byte length on entry
  (`= D*8`).
- **Prologue:** for `i` in `0..D`, load slot `i` (byte offset `i*8`) and push onto the
  virtual stack (deepest first), reproducing the carried stack shape.
- **Body:** lower the line's terms exactly as a word body over that stack.
- **Epilogue:** the resulting stack holds `M` values (`M = D + net`, `net` from the
  checker); for `j` in `0..M`, store slot `j` (byte offset `j*8`).
- **Return:** `new_top = %top + (M - D)*8`, i.e. `ret` the advanced top.

The compiler emits the load/store marshalling from the layout it already knows, so this
generalizes to richer types (Phase 2) with no host-side reflection. Word bodies are
unchanged from Phase 0; only this wrapper touches the buffer.

The host calls it as `extern "C" fn(*mut u8, usize) -> usize`. Before the call the session
grows the `Vec<i64>` to at least `M` slots (outputs may exceed inputs) and passes
`base_ptr, D*8`; after, sets `top = new_top` and `D = new_top / 8`.

### Neutral IR additions

To express the wrapper while keeping `Ptr` opaque (DESIGN: `Ptr[T]` never assumed to be a
native `u64`, WASM depends on it), add three neutral instructions to `Instr`
(`src/ir.rs:49`):

- `PtrOffset(Value dst, Value base, i64 bytes)` — `dst: Ptr = base + bytes`. QBE: `add`;
  WASM later: `i32.add`.
- `Load(Value dst, Value ptr)` — `dst: Int` loaded from `*ptr`. QBE: `loadl`.
- `Store(Value ptr, Value val)` — store `Int` to `*ptr`. QBE: `storel`.

Phase 1 uses these only in the line wrapper, with 8-byte `int` slots. The `IrType::Ptr`
variant becomes used (update its "unused in Phase 0" comment). `emit` (`src/backend/qbe.rs`)
gains three arms; the existing function/param code already emits `l` params and an `l`
return, so a `(Ptr, Int) -> Int` wrapper needs no other backend change.

`lower_line(terms, entry_depth, env) -> IrFunc` builds the wrapper: params `[Ptr, Int]`
occupy value ids `%v0`/`%v1` (matching the "params are the first value ids" convention in
`lower_word`), then prologue, body via the existing `FuncBuilder` term lowering, epilogue,
`Ret(Some(new_top))`.

### Line surface

`parser::parse_line(tokens) -> Result<Line, String>` where:

```rust
// src/ast.rs
pub enum Line { Def(WordDef), Expr(Vec<Term>) }
```

If the first token is `Colon`, delegate to the existing `parse_worddef`; otherwise parse a
term sequence to EOF with the existing `parse_term` (reusing its `if/else/then` handling).
`parse` (whole-module) is untouched, so `build`/`run` are unaffected. Each stdin line is one
complete unit: a def must be closed with `;` on the line, an unterminated def is a normal
parse error (multi-line accumulation is deferred).

The checker gains net-effect **inference** for a bare line: a small extension of the
existing depth simulation.

- `check::infer_line(terms, entry_depth, env) -> Result<usize, String>` runs `check_term`
  from `entry_depth = D` (the carried depth) and returns the final depth `D'`. Underflow
  against the carried stack (`depth < in_arity`) is a normal, reported error.
- `check` currently takes `&WordDef` for diagnostics; refactor the shared depth-simulation
  helpers to take the arity `env` and a light error context (a word name + optional declared
  effect) instead of a full `&WordDef`, so both the def path and the line path share the
  simulation. The def path also seeds the env with the definee's own declared arity before
  checking its body, so self-recursion type-checks (as `check` already does module-wide).
- Line underflow diagnostic has no declared effect to cite; format e.g.
  `error: stack underflow:`+`needs 2 values, but the stack holds 1`. Tested as behaviour.

`env` is the session's accumulated word→arity map (builtins from `builtin_table` plus every
successfully-defined word). Both `check` and `ir::lower`/`lower_line` accept it so a single
unit compiles against previously-loaded words.

### Redefinition: generation-mangled symbols

Latest-symbol binding: a redefinition takes effect for **subsequently compiled** lines;
already-loaded code keeps the callee it was compiled against.

- Each definition of `name` exports `name__gen{K}`, `K` a per-name generation counter.
- The session keeps `name → { arity, generation, symbol }`. When lowering any unit, calls to
  a user word `w` resolve to `w`'s **current** mangled symbol; the def being compiled exports
  its own new-generation symbol.
- **Self-reference binds the new generation** (recursion), consistent with Phase 0's direct
  self-call (`gcd` calls `gcd`). Concretely: build the resolver from current generations,
  then override the definee's own name to its new generation before lowering its body.
- Builtins (`+`, `dup`, `.`, …) never appear as `Instr::Call` (they lower inline / to
  `printf`), so they are never mangled.
- Mangling is a linkage concern applied at lowering time via the resolver the session
  supplies (`Instr::Call` carries the resolved symbol). `qbe_name`'s `main → sooth_main`
  special case is irrelevant here (the REPL never defines `main`).

Generation mangling also sidesteps symbol clashes under `RTLD_GLOBAL`: distinct generations
are distinct symbols, so "loaded callers keep the old callee" falls out for free.

### Driver: compile to shared object and load

- `driver`: elevate `tempfile_dir`/`run_command` to `pub(crate)`; add
  `compile_so(ssa: &str, out: &Path) -> Result<(), String>`: write `out.ssa`,
  `qbe out.ssa -o out.s`, then `cc -shared -fPIC out.s -o <out>`. No C shim (a `.so` has no
  `main`). On macOS also pass `-Wl,-undefined,dynamic_lookup` (via `cfg!(target_os =
  "macos")`) so undefined symbols (earlier generations, `printf`) resolve at load under the
  two-level namespace; on Linux `RTLD_GLOBAL` covers it.
- `repl.rs`: raw `dlopen`/`dlsym` FFI in a small `extern "C"` block (keeps the zero-dependency
  posture the craft ethos prizes; `libloading` is the ergonomic fallback if the unsafe
  surface bites). Open with `RTLD_NOW | RTLD_GLOBAL`; **never `dlclose`** (the session keeps
  every object resident, so old generations stay callable); retain each handle in a `Vec` so
  it outlives the session's use. Only the line wrapper is `dlsym`'d and called; word symbols
  are resolved by the dynamic linker at load via the global scope.

### Session loop, output, recovery

`repl.rs` owns a `Session`:

```
env:   HashMap<String, WordEntry>   // WordEntry { arity, generation, symbol }
buf:   Vec<i64>                     // 8-aligned stack storage
top:   usize                        // live byte length (D = top/8)
libs:  Vec<*mut c_void>             // loaded objects kept alive
seq:   u64                          // line-wrapper name counter
```

The loop reads lines over `impl BufRead`, writes over `impl Write` (so the core is testable
in-process; `driver::repl()` wires `stdin`/`stdout`). Per line:

1. Blank/whitespace-only → skip silently.
2. `lex` → `parse_line`. Any stage error → print the diagnostic, **mutate nothing**,
   continue.
3. `Line::Def`: `check` against env (seeded with the definee's own arity); on success bump
   the generation, lower with the resolver, `emit`, `compile_so`, `dlopen`; only then commit
   the env entry (new arity/generation/symbol) and print `defined <name>`. On any failure,
   the env is untouched (no half-applied redefinition).
4. `Line::Expr`: `infer_line` against `D`; on success `lower_line`, `emit`, `compile_so`,
   `dlopen`, `dlsym $sooth_line_{seq}`; grow `buf` to `≥ M` slots; **flush the host stdout**
   (so any `.`/`printf` output from the loaded code interleaves deterministically), call the
   wrapper, set `top`/`D`, then print the residual stack.

Output contract (all on the host's stdout, distinct from the compiler's `stderr`
diagnostics, so a piped session captures everything on one stream):

- Expression success: the loaded code's `.` output (if any) first, then one line
  `stack: <v0> <v1> … <vk>` (space-separated, bottom→top) or `stack: (empty)`.
- Definition success: `defined <name>`.
- Any error: the diagnostic string; stack and env unchanged.

`format_stack(&[i64]) -> String` is a pure, unit-tested helper. No prompt is printed in
Phase 1 (bare line reading; readline/prompt/history is deferred polish per the brief). EOF
(pipe end / Ctrl-D) exits cleanly (`Ok`).

**Runtime-crash caveat (accepted):** a fault in loaded code takes the process down
(unavoidable in-process). Low risk while the surface is `int`-only and statically checked.

### Platform

Linux + macOS. The only OS-specific code is the macOS `cc` linker flag and the `dlopen`
constants; keep both behind `cfg!`/`extern "C"` and out of the stages.

### DESIGN.md note (required by the brief)

Add a short note to DESIGN.md that the REPL's runtime stack **buffer** is a driver artifact
bridging separately-compiled lines, and a deliberate **preview of the "uniform runtime
stack"** reserved for escaping quotations in Phase 4, not a breach of the
compile-time-only-stack invariant (word bodies still compute in SSA/registers).

## Where the new code lives

Start small; split only under real pressure (CLAUDE.md). The REPL is a new responsibility
(session state + `dlopen` FFI + wrapper synthesis) with imports the `build`/`run` path
doesn't share, which is genuine import divergence, so it earns its own module:

- **New `src/repl.rs`** (add `pub mod repl;` to `src/lib.rs`): `Session`, the read-eval-print
  loop, the `dlopen`/`dlsym` FFI, generation mangling/resolution, `format_stack`.
  `driver::repl()` delegates to `repl::run(stdin.lock(), stdout.lock())`.
- **`src/driver.rs`**: `compile_so`; `pub(crate)` on `tempfile_dir`/`run_command`.
- **`src/parser.rs`** + **`src/ast.rs`**: `parse_line`, `Line`.
- **`src/check.rs`**: env-driven checking + `infer_line`; shared simulation helpers reworked
  to take env + light context.
- **`src/ir.rs`**: `Load`/`Store`/`PtrOffset`; `lower_line`; env + resolver threaded through
  lowering.
- **`src/backend/qbe.rs`**: emit arms for the three new instrs.

Everything else (`compile_so` living in `driver` rather than `repl`) stays at the lowest
common ancestor only if a second caller appears; for now it can sit in `driver` beside the
existing `qbe`/`cc` plumbing it mirrors.

## Test plan

Per CLAUDE.md: a `#[cfg(test)] mod tests` beside each new/extended stage (happy path + at
least one error/edge case), named `thing_condition_expected`; every exit criterion a golden;
diagnostics tested as behaviour (right error, not just failure).

**Unit tests (in-process, no toolchain needed unless noted):**

- `parser`: `parse_line_bare_expression_is_expr`; `parse_line_colon_is_def`;
  `parse_line_unterminated_def_is_error`.
- `check`: `infer_line_net_effect_expected` (e.g. `2 3 +` from `D=0` → `1`);
  `infer_line_carries_entry_depth` (from `D=1`, `+` needs the carried slot);
  `line_underflow_against_carried_stack_is_error` (right text).
- `ir`: `lower_line_marshals_all_inputs_and_outputs` (D loads, M stores);
  `lower_line_returns_advanced_top`; `lower_call_uses_resolved_generation_symbol`.
- `qbe`: `emit_wrapper_signature_takes_stack_and_top`;
  `emit_line_wrapper_has_load_and_store`.
- `driver` (needs `qbe`+`cc`, like Phase 0 goldens):
  `compile_so_produces_loadable_object`.
- `repl`: `format_stack_bottom_to_top`; `format_stack_empty_is_marker`;
  `resolve_binds_current_generation`; `redefinition_bumps_generation`.

**Golden session tests — `tests/phase1.rs`** (spawn the `repl` binary via
`env!("CARGO_BIN_EXE_sooth")`, pipe a scripted stdin, assert stdout; captures both host
output and the loaded code's `printf` on one stream). One test per exit criterion:

- `define_then_call_across_lines` (criterion 2): `: sq ( int -- int ) dup * ;` then `5 sq`
  → `stack: 25`.
- `stack_persists_across_lines` (criterion 3): `5` then `sq` on separate lines still yields
  `25`; a following `1 +` yields `26`.
- `redefinition_takes_effect_for_later_lines` (criterion 4): define `sq` as `dup *`, call it,
  redefine as `dup dup * *`, call again, observe the new result.
- `bad_line_reports_and_session_survives` (criterion 5): a line with an unknown word or
  underflow prints the diagnostic; the next good line runs with the stack unchanged.
- `calculator_session_dogfood` (criterion 6): a small interactive calculator session driven
  end to end (define `sq`/`neg`/etc., run a sequence of expressions, check the residual
  stack and `.` output). This is the Phase 1 dogfood golden.

Criterion 1 (`cargo run -- repl` starts and reads stdin) is exercised by every golden.

**Green** stays `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Out of scope for Phase 1 (unchanged from the brief)

New language types (`bool`, numeric tower, structs/enums, pointers), heap, affine/move
semantics, polymorphism, quotations/combinators, TCO, recompiling dependents on
redefinition, readline/history, sub-millisecond compile / an owned backend, WASM, `no_std`
packaging, comptime/immediate words. All later phases or explicitly declined.

## Deferred (intended, not Phase 1 exits)

- **Minimal-footprint marshalling**: load only the sub-stack a line touches (low-water-mark
  of the depth simulation) instead of the whole carried stack. Optimization, not correctness.
- **Recompiling dependents** so a redefinition propagates to existing words (needs a
  dependency graph + cascading recompiles).
- **Multi-line input accumulation**, prompt, readline, history.
- **Old-object reclamation**: the session never `dlclose`s, so memory grows with line count.
  Fine for a session; revisit if it bites.

## Open questions / risks

- **macOS flat-namespace linking**: `-Wl,-undefined,dynamic_lookup` is the intended flag;
  verify cross-`.so` symbol resolution (an earlier generation called from a later object) on
  macOS as well as Linux. If it misbehaves, fall back to `-Wl,-flat_namespace` or resolving
  callees to absolute addresses the session already holds.
- **stdout interleaving**: host (Rust) and loaded-code (`printf`, C stdio) use separate
  buffers; the flush-before-call discipline must hold for goldens to be deterministic.
- **Toolchain at test time**: golden sessions need `qbe` + `cc` present, same assumption as
  Phase 0's goldens.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "shared object compile and dlopen",
      "difficulty": "hard",
      "id": "p1-so-load",
      "title": "Shared-object compile + in-process dlopen (de-risk the mechanism)",
      "summary": "Prove the novel systems bit first: compile through the existing pipeline to a .so and load-and-call it in-process, before any language changes.",
      "changes": [
        "driver: elevate tempfile_dir/run_command to pub(crate)",
        "driver: add compile_so(ssa, out) -> emit .ssa, qbe -> .s, cc -shared -fPIC (+ macOS -Wl,-undefined,dynamic_lookup), no C shim",
        "src/repl.rs: raw extern \"C\" dlopen/dlsym FFI (RTLD_NOW|RTLD_GLOBAL, never dlclose), handles retained; add pub mod repl to src/lib.rs"
      ],
      "tests": [
        "driver::compile_so_produces_loadable_object",
        "repl: a compiled word symbol is dlsym'able and callable in-process (5 -> 25)"
      ],
      "exit": "A Rust test compiles a word to a .so, dlopens it, and calls the symbol in-process on Linux and macOS."
    },
    {
      "phase": 2,
      "focus": "line surface parse and check",
      "difficulty": "standard",
      "id": "p2-line-surface",
      "title": "Line surface: parse + check bare expressions",
      "summary": "Accept a top-level term sequence and infer its net effect against a carried depth and an external word env; underflow against the carried stack is a reported error.",
      "changes": [
        "src/ast.rs: add Line { Def(WordDef), Expr(Vec<Term>) }",
        "src/parser.rs: add parse_line (Colon -> parse_worddef, else term sequence to EOF)",
        "src/check.rs: rework shared depth-simulation helpers to take an arity env + light error context; add infer_line(terms, entry_depth, env); def path seeds the definee's own arity for self-recursion"
      ],
      "tests": [
        "parser::parse_line_bare_expression_is_expr",
        "parser::parse_line_colon_is_def",
        "parser::parse_line_unterminated_def_is_error",
        "check::infer_line_net_effect_expected",
        "check::infer_line_carries_entry_depth",
        "check::line_underflow_against_carried_stack_is_error"
      ],
      "exit": "Given an env and a carried depth, a line is classified Def/Expr and an expression's net effect is inferred, with carried-stack underflow reported as the right diagnostic."
    },
    {
      "phase": 3,
      "focus": "line wrapper IR and QBE",
      "difficulty": "hard",
      "id": "p3-wrapper-marshalling",
      "title": "Line-wrapper marshalling (IR + QBE)",
      "summary": "Lower a bare expression to a uniform-signature wrapper that loads the whole carried stack, runs the body in registers, stores results back, and returns the advanced top; keep Ptr opaque via neutral IR ops.",
      "changes": [
        "src/ir.rs: add Instr::Load/Store/PtrOffset; make IrType::Ptr used (update comment)",
        "src/ir.rs: add lower_line(terms, entry_depth, env) building the (Ptr,Int)->Int wrapper (prologue loads D, body, epilogue stores M, ret advanced top)",
        "src/ir.rs: thread env arities + a call-name resolver so units compile against previously-loaded words with generation-mangled symbols",
        "src/backend/qbe.rs: emit arms for PtrOffset (add), Load (loadl), Store (storel)"
      ],
      "tests": [
        "ir::lower_line_marshals_all_inputs_and_outputs",
        "ir::lower_line_returns_advanced_top",
        "ir::lower_call_uses_resolved_generation_symbol",
        "qbe::emit_wrapper_signature_takes_stack_and_top",
        "qbe::emit_line_wrapper_has_load_and_store"
      ],
      "exit": "A bare expression compiles to a wrapper .ssa with correct load/store/return-top and generation-resolved call targets."
    },
    {
      "phase": 4,
      "focus": "REPL session loop redefinition",
      "difficulty": "standard",
      "id": "p4-session",
      "title": "REPL session: state, redefinition, buffer, loop, display",
      "summary": "Wire the read-eval-print loop over BufRead/Write: persistent Vec<i64> stack buffer + carried depth, generation-mangled redefinition, def and expr paths, error recovery with no state mutation on failure, deterministic stdout ordering.",
      "changes": [
        "src/repl.rs: Session { env, buf: Vec<i64>, top, libs, seq }; read-eval-print loop; blank-line skip; EOF exits Ok",
        "src/repl.rs: generation mangling + current-generation resolution (self-name binds new generation); commit env only after successful load",
        "src/repl.rs: expr path grows buf to M slots, flushes host stdout, calls wrapper, updates top/D, prints residual stack; def path prints defined <name>",
        "src/repl.rs: format_stack; error recovery prints diagnostic and leaves stack/env unchanged",
        "src/driver.rs: repl() delegates to repl::run(stdin, stdout)"
      ],
      "tests": [
        "repl::format_stack_bottom_to_top",
        "repl::format_stack_empty_is_marker",
        "repl::resolve_binds_current_generation",
        "repl::redefinition_bumps_generation"
      ],
      "exit": "cargo run -- repl runs an interactive session meeting exit criteria 1-5: define/call, persist stack, redefine, and survive a bad line."
    },
    {
      "phase": 5,
      "focus": "golden sessions and docs",
      "difficulty": "standard",
      "id": "p5-goldens-dogfood-docs",
      "title": "Golden sessions, dogfood calculator, and doc updates",
      "summary": "Nail the exit criteria as scripted golden sessions, write the dogfood calculator session, and record the runtime stack buffer in DESIGN plus mark Phase 1 done.",
      "changes": [
        "tests/phase1.rs: scripted stdin -> stdout goldens via CARGO_BIN_EXE_sooth",
        "DESIGN.md: note the REPL runtime stack buffer is a driver artifact and a Phase 4 uniform-runtime-stack preview, not a compile-time-stack breach",
        "ROADMAP.md + README.md: mark Phase 1 complete"
      ],
      "tests": [
        "phase1::define_then_call_across_lines",
        "phase1::stack_persists_across_lines",
        "phase1::redefinition_takes_effect_for_later_lines",
        "phase1::bad_line_reports_and_session_survives",
        "phase1::calculator_session_dogfood"
      ],
      "exit": "All Phase 1 goldens pass; the dogfood calculator session runs end to end; cargo fmt --check && cargo clippy -- -D warnings && cargo test is green."
    }
  ]
}
```
