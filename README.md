# Sooth

A small, statically-checked concatenative language in the Forth/Factor/Kitten
lineage, compiled to native code with no external runtime. A craft language: built
for the pleasure of writing it and writing programs in it, not as a product. See
[DESIGN.md](./DESIGN.md) for the why and [ROADMAP.md](./ROADMAP.md) for the plan.

The one bet that makes it more than a tidy Forth: in a stack language the stack
discipline already is move semantics, so affine types drop in for free. `dup` is the
explicit copy, and drop is a statically-known destructor point.

```forth
: gcd ( int int -- int )
  dup 0 = if
    drop
  else
    swap over mod gcd
  then ;
```

## Status

**Phase 0 (codegen spine): complete** (see ROADMAP.md). The pipeline is implemented
end to end and the goldens (`gcd`, `factorial`, `lerp`) compile to native binaries and
run. Next is Phase 1 (a REPL / liveness loop). The compiler is a Rust bootstrap; the
language will later self-host.

Pipeline: `source → lex → parse → stack-effect check → backend-neutral IR → QBE IL
→ native binary`. Backend is [QBE](https://c9x.me/compile/), invoked from `PATH`
alongside the system `cc`. WASM is a planned sibling lowering off the neutral IR.

## Build and run

Requires `qbe` and a C compiler (`cc`) on your `PATH`.

```sh
cargo build
cargo test                            # unit tests + the Phase 0 goldens
cargo run -- run   examples/gcd.sth   # compile and run (prints 5)
cargo run -- build examples/gcd.sth   # just compile, to examples/gcd
```

## Layout

```
src/lexer.rs  parser.rs  ast.rs   front end
src/check.rs                      stack-effect checker
src/ir.rs                         backend-neutral IR (Ptr[T] kept abstract)
src/backend/qbe.rs                QBE IL emission
src/driver.rs                     pipeline orchestration
examples/                         Phase 0 target programs
tests/phase0.rs                   golden tests (gcd / factorial / lerp + a diagnostic)
```
