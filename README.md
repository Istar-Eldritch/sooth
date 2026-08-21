# Sooth

A small, statically-checked concatenative language in the Forth/Factor/Kitten
lineage, compiled to native code with no external runtime. A craft language: built
for the pleasure of writing it and writing programs in it, not as a product. See
[DESIGN.md](./DESIGN.md) for the why and [ROADMAP.md](./docs/roadmap/ROADMAP.md) for the plan.

The one bet that makes it more than a tidy Forth: in a stack language the stack
discipline already is move semantics, so linear types fall out for free. `dup` is the
explicit copy, and `drop` is the explicit destructor point (use exactly once, forgetting
is a compile error).

```forth
: gcd ( i64 i64 -- i64 )
  dup 0 eq ~[
    drop
  ] ~[
    swap over mod gcd
  ] if ;
```

## Status

**Phases 0-3 complete; Phase 4 (minimal polymorphism + quotations) well underway** (see
ROADMAP.md for the slice-by-slice detail). The pipeline is implemented end to end: the
examples compile to native binaries and run, and `cargo run -- repl` gives an interactive
session where words are compiled to shared objects and `dlopen`'d in as you define them.

Phase 2 delivered the **typed core**, all of it heap-free: a `Type` per stack slot, unified
at branch joins; the fixed-width integer tower (`i8`..`i64`, `u8`..`u64`) with explicit
target-only conversions (`>i8`..`>u64`); floating point (`f32`/`f64`); bitwise operators;
boolean logic with the full comparison set (`eq lt gt lte gte ne`); **structs** (the `type:`
form, inline-aggregate layout, generated constructor/accessor words); **enums/ADTs**
(`|`-separated variants, tagged inline aggregates, exhaustiveness-checked elimination
through the generated eliminator word, no inline `match`); **fixed-size arrays** with `usize`; self-tail-call
lowered to a jump; and a bytecode VM as the exit dogfood.

Phase 3 made the linear spine real: move-by-default with `dup` gated on `Copy` and `drop`
as an explicit destructor call; `^T`, a compiler-known single heap cell that is always
linear and propagates linearity transitively through structs and enums, behind a
`malloc`/`free` shim with an OOM trap; recursive heap data (lists, trees, mutually
recursive shapes) whose synthesized destructors dispose in **constant stack**, verified
past a million nodes under a 1 MB stack; **general locals**, where `| names |` binds at
any point in a body and REPL lines can bind too; **second-class references**, where
`&a`/`&!a` borrows a local, projection reaches a field, element or cell payload, and
`@`/`!`/`+!` read and mutate through a reference, governed by per-place exclusivity and
structural escape prevention rather than by any lifetime system; typed foreign calls
(`extern:`) and string slices; and resources as linear values with user-definable
destructor bodies (`drop` overrides). Opt-in reference counting (`Rc`/`Arc`) is deferred
to Phase 6, alongside the rest of the stdlib layering.

Phase 4 adds bounded polymorphism (`'T`/`'N`/`..s` type, length, and row variables,
monomorphized per instantiation, no vtables) and quotations: `[ ... ]` literals, `call`,
and an internal loop primitive that lowers self-tail recursion to a constant-stack back-edge
rather than an unrolled splice. On top of that, a combinator library written in Sooth
itself (`lib/combinators.sth`: `each`/`map`/`fold`/`filter`/`while`/`times`), inlined by a
term-splicing compiler pass rather than minting a function per call site; multi-file
modules with word/type imports, natively and at the REPL; and quotations as real runtime
values (non-capturing closures, storable and passable to non-inlined higher-order code).
No combinator is compiler-known: `times` is ordinary Sooth source over a
self-tail-recursive helper, like the rest. Nor is `if`: it is a `core::bool` word
taking a `bool` and two quotations, over the machine primitives `branch` and `tag`,
imported by name like anything else.
In progress: capturing closures and static ad-hoc overloading (`docs/roadmap/P4/slice7b-brief.md`,
`docs/roadmap/P4/slice8a-brief.md`).

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
: sq ( i64 -- i64 ) | n | n n mul ;
defined sq
5 sq
stack: 25
1 2 3
stack: 25 1 2 3
| a b | a b add .
5
stack: 25 1
```

The last line binds two values the *previous* line left on the session stack. A line's
names are scoped to that line; the stack is what persists.
