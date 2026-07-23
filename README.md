# Sooth

A small, statically-checked concatenative language in the Forth/Factor/Kitten
lineage, compiled to native code with no external runtime. A craft language: built
for the pleasure of writing it and writing programs in it, not as a product. See
[DESIGN.md](./DESIGN.md) for the why and [ROADMAP.md](./ROADMAP.md) for the plan.

The one bet that makes it more than a tidy Forth: in a stack language the stack
discipline already is move semantics, so affine types drop in for free. `dup` is the
explicit copy, and drop is a statically-known destructor point.

```forth
: gcd ( i64 i64 -- i64 )
  dup 0 = if
    drop
  else
    swap over mod gcd
  then ;
```

## Status

**Phase 0 (codegen spine): complete** and **Phase 1 (REPL / liveness): complete**
(see ROADMAP.md). The pipeline is implemented end to end and the goldens (`gcd`,
`factorial`, `lerp`) compile to native binaries and run; `cargo run -- repl` gives an
interactive session where words are compiled to shared objects and `dlopen`'d in as
you define them. Next is Phase 2 (a typed core). The compiler is a Rust bootstrap; the
language will later self-host.

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
cargo test                            # unit tests + the Phase 0/1 goldens
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
```

## Layout

```
src/lexer.rs  parser.rs  ast.rs   front end
src/check.rs                      stack-effect checker
src/ir.rs                         backend-neutral IR (Ptr[T] kept abstract)
src/backend/qbe.rs                QBE IL emission
src/driver.rs                     pipeline orchestration
src/repl.rs                       REPL session: dlopen loop, generation mangling
examples/                         Phase 0 target programs
tests/phase0.rs                   golden tests (gcd / factorial / lerp + a diagnostic)
tests/phase1.rs                   golden REPL sessions (define/redefine/recover)
```
