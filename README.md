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

**Phase 0** (see ROADMAP.md): codegen spine. Pipeline is scaffolded, not yet
implemented. The compiler is a Rust bootstrap; the language will later self-host.

Pipeline: `source → lex → parse → stack-effect check → backend-neutral IR → QBE IL
→ native binary`. Backend is [QBE](https://c9x.me/compile/) (not yet installed here;
build from source). WASM is a planned sibling lowering off the neutral IR.

## Build and run

```sh
cargo build
cargo run -- build examples/gcd.sth   # not implemented yet: prints Phase 0 status
```

## Layout

```
src/lexer.rs  parser.rs  ast.rs   front end
src/check.rs                      stack-effect checker
src/ir.rs                         backend-neutral IR (Ptr[T] kept abstract)
src/backend/qbe.rs                QBE IL emission
src/driver.rs                     pipeline orchestration
examples/                         Phase 0 target programs
tests/phase0.rs                   golden tests (ignored until the pipeline lands)
```
