# Getting Started

## Build the compiler

Sooth is a Rust project. You need `cargo` and `qbe` on your `PATH`:

```sh
git clone <repo-url> sooth
cd sooth
cargo build --release
```

The release binary is `target/release/sooth`. QBE must be installed
separately — it is the backend that generates native machine code. On
Debian/Ubuntu, `apt install qbe` may give you a version that is too old
(see the README build note); install from
[source](https://c9x.me/qbe/) if the packaged version rejects float
conversions.

## Compile a program

Sooth source files use the `.sth` extension. The `gcd.sth` example
computes the greatest common divisor of two numbers using Euclid's
algorithm:

```sooth
: gcd ( i64 i64 -- i64 )
  dup 0 = if
    drop
  else
    swap over mod gcd
  end ;

: main ( -- )
  10 15 gcd . ;    \ prints 5
```

Compile and run it:

```sh
cargo run --release -- build examples/gcd.sth
./gcd
```

Output:

```text
5
```

Don't try to read the whole program yet. By the end of Part I you will
understand every word of it.

## The REPL

The fastest way to learn is the REPL. Start it:

```sh
cargo run --release -- repl
```

You get a prompt. Type a number and print it:

```text
> 42 .
42
stack: (empty)
```

What just happened? You pushed `42` onto the stack. The word `.`
popped the top of the stack and printed it. After printing, the stack
is empty, and the REPL tells you so.

Push two numbers and add them:

```text
> 1 2
stack: 1 2
> +
stack: 3
> .
3
stack: (empty)
```

The REPL shows the stack after each line. `1 2` pushes both numbers.
`+` pops them and pushes their sum. `.` prints the result.

## The stack is the program

Every value in Sooth lives on a single stack. Words consume values from
the top and push results back. There are no variables (not yet), no
function arguments, no expression nesting. You describe a computation as
a sequence of operations on one shared channel.

This is the concatenative model: programs are *concatenations* of words,
and the meaning of a program is the composition of the meanings of its
words. `1 2 +` means: push 1, push 2, add. `42 .` means: push 42, print.
`1 2 + .` means all three at once.

Try a longer chain:

```text
> 3 4 + 5 + .
12
stack: (empty)
```

Read it left to right: push 3, push 4, add (stack: 7), push 5, add
(stack: 12), print.

## Stack manipulation

Three words you will use constantly:

- `dup` — copy the top of the stack
- `drop` — discard the top of the stack
- `swap` — swap the top two elements

```text
> 2 dup * .
4
stack: (empty)
```

`2 dup *` squares a number: push 2, copy it (stack: 2 2), multiply
(stack: 4), print.

```text
> 1 2 3 drop .
2
stack: 1
```

Read it carefully: push 1, push 2, push 3 (stack: 1 2 3). `drop` removes
3 (stack: 1 2). `.` pops 2 and prints it (stack: 1). The `1` is still
on the stack. Type `drop` to clear it, or `:quit` to leave the REPL.

## Quitting

Type `:quit` to exit. The REPL disposes any remaining values on the
stack (you will learn what "dispose" means in Part II).

## What's next

You can push numbers, add them, print them, and manipulate the stack.
The next chapter builds the full mental model: how the stack carries
types, how words declare what they consume and produce, and why the
compiler can catch mistakes before the program runs.
