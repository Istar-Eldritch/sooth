# Preface

Sooth is a small, statically-checked concatenative language. It compiles
to native code via [QBE](https://c9x.me/qbe/), and its defining idea is
that linear types — the property that every value is used exactly once —
fall out of the stack discipline for free. In a stack language, `dup` is
the only way to copy and `drop` is the only way to discard. If the
compiler tracks that, you get move semantics without a borrow checker,
without lifetime annotations, and without a runtime garbage collector.

This book teaches Sooth by building from the stack up. You will start
with pushing numbers and work toward writing a bytecode interpreter in
the language itself. Every example in this book compiles and runs. If
you have the repository checked out, you can follow along in the REPL.

## Who this is for

You should know how to program. Familiarity with a stack language
(Forth, Factor, PostScript) helps but is not required — the stack
mental model is introduced from scratch. Familiarity with Rust's
ownership system helps you appreciate what Sooth achieves, but is not
required either; the linear spine is explained on its own terms.

## What this book is not

This is not a reference manual. The language reference is the compiler:
its error messages, its accepted syntax, and its test suite. This book
is a guided path through the concepts, ordered so that each chapter
builds on the last.

This is also not a design document. For the reasoning behind the
language's decisions, see `DESIGN.md` in the repository. This book
teaches the *what* and the *how*, not the *why we chose this over that*.

## Conventions

Code blocks show Sooth source on their own, or REPL sessions with the
`stack:` prompt line that the REPL prints after each evaluation:

```text
> 1 2 + .
3
stack: (empty)
```

The `>` prefix marks what you type. Everything else is output. In the
REPL, `.` prints the top of the stack and removes it; after a print the
stack is often empty, which the REPL reports as `stack: (empty)`.
