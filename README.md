# Sooth

A small, statically-checked concatenative language in the Forth/Factor/Kitten
lineage, compiled to native code with no external runtime. A craft language: built
for the pleasure of writing it and writing programs in it, not as a product. See
[DESIGN.md](./DESIGN.md) for the why and [ROADMAP.md](./ROADMAP.md) for the plan.

The one bet that makes it more than a tidy Forth: in a stack language the stack
discipline already is move semantics, so linear types fall out for free. `dup` is the
explicit copy, and `drop` is the explicit destructor point (use exactly once, forgetting
is a compile error).

```forth
: gcd ( i64 i64 -- i64 )
  dup 0 = if
    drop
  else
    swap over mod gcd
  end ;
```

## Status

**Phases 0-2 complete; Phase 3 (the memory model) in progress** (see ROADMAP.md). The
pipeline is implemented end to end: the examples compile to native binaries and run, and
`cargo run -- repl` gives an interactive session where words are compiled to shared objects
and `dlopen`'d in as you define them.

Phase 2 delivered the **typed core**, all of it heap-free: a `Type` per stack slot, unified
at branch joins; the fixed-width integer tower (`i8`..`i64`, `u8`..`u64`) with explicit
target-only conversions (`>i8`..`>u64`); floating point (`f32`/`f64`); bitwise operators;
boolean logic with the full comparison set (`= < > <= >= <>`); **structs** (the `type:`
form, inline-aggregate layout, generated constructor/accessor words); **enums/ADTs**
(`|`-separated variants, tagged inline aggregates, exhaustiveness-checked clause-style
elimination, no inline `match`); **fixed-size arrays** with `usize`; self-tail-call
lowered to a jump; and a bytecode VM as the exit dogfood.

Phase 3 is making the linear spine real rather than aspirational. Landed so far:
move-by-default with `dup` gated on `Copy` and `drop` as an explicit destructor call;
`^T`, a compiler-known single heap cell that is always linear and propagates linearity
transitively through structs and enums, behind a `malloc`/`free` shim with an OOM trap;
recursive heap data (lists, trees, mutually recursive shapes) whose synthesized
destructors dispose in **constant stack**, verified past a million nodes under a 1 MB
stack; and **general locals**, where `| names |` binds at any point in a body and REPL
lines can bind too. Next is second-class references and escape checking.

The compiler is a Rust bootstrap; the language will later self-host.

Pipeline: `source → lex → parse → stack-effect check → backend-neutral IR → QBE IL
→ native binary`. Backend is [QBE](https://c9x.me/compile/), invoked from `PATH`
alongside the system `cc`. WASM is a planned sibling lowering off the neutral IR.

## Build and run

Requires `qbe` and a C compiler (`cc`) on your `PATH`. Needs a reasonably modern QBE:
Debian's packaged `qbe` (1.2) predates the unsigned int/float conversion ops
(`uwtof`/`ultof`/`stoui`/`dtoui`) and fails with `unknown keyword` on those; build
from [c9x.me/git/qbe.git](https://c9x.me/git/qbe.git) if so.

```sh
cargo build
cargo test                            # unit tests + the goldens
cargo run -- run   examples/gcd.sth   # compile and run (prints 5)
cargo run -- build examples/gcd.sth   # just compile, to examples/gcd
cargo run -- repl                     # interactive session
```

A REPL session compiles each line to a shared object and `dlopen`s it into the
process, with a stack that persists across lines (no prompt is printed in Phase 1;
lines below are input, other lines are the session's output):

```forth
: sq ( i64 -- i64 ) | n | n n * ;
defined sq
5 sq
stack: 25
1 2 3
stack: 25 1 2 3
| a b | a b + .
5
stack: 25 1
```

The last line binds two values the *previous* line left on the session stack. A line's
names are scoped to that line; the stack is what persists.

## Layout

```
src/lexer.rs  parser.rs  ast.rs   front end
src/check.rs                      stack-effect checker
src/ir.rs                         backend-neutral IR (Ptr[T] kept abstract)
src/backend/qbe.rs                QBE IL emission
src/driver.rs                     pipeline orchestration
src/repl.rs                       REPL session: dlopen loop, generation mangling
examples/                         target programs (gcd, lerp, shapes, stack, list, vm, ...)
tests/phase0.rs                   golden tests: build+run + diagnostics across the typed core
tests/phase1.rs                   golden REPL sessions (define/redefine/recover)
tests/phase3_locals.rs            goldens for general locals (mid-body and REPL-line)
```
