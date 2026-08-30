# Getting Started

## Install the compiler

Sooth is a Rust project. You need `cargo` and `qbe` on your `PATH`.
Install the Sooth CLI with:

```sh
git clone <repo-url> sooth
cd sooth
cargo install --path .
```

This puts the `sooth` command on your `PATH`. QBE must be installed
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
import: hosted::show | . | ;

: gcd ( i64 i64 -- i64 )
  | a b |
  b 0 eq if
    a
  else
    b a b mod gcd
  end ;

: main ( -- )
  10 15 gcd . ;    \ prints 5
```

Compile and run it:

```sh
sooth build examples/gcd.sth
./gcd
```

Output:

```text
5
```

Don't try to read the whole program yet. By the end of Part I you will
understand every word of it. For now, notice the `| a b |` line: it
binds the two inputs to names, and the rest of the word uses those
names instead of shuffling them around on the stack.

## The REPL

The fastest way to learn is the REPL. Start it:

```sh
sooth repl
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
> add
stack: 3
> .
3
stack: (empty)
```

The REPL shows the stack after each line. `1 2` pushes both numbers.
`add` pops them and pushes their sum. `.` prints the result.

## The stack is the program

Every value in Sooth lives on a single stack. Words consume values from
the top and push results back. There are no function arguments, no
expression nesting. You describe a computation as a sequence of
operations on one shared channel.

This is the concatenative model: programs are *concatenations* of words,
and the meaning of a program is the composition of the meanings of its
words. `1 2 add` means: push 1, push 2, add. `42 .` means: push 42, print.
`1 2 add .` means all three at once.

Try a longer chain:

```text
> 3 4 add 5 add .
12
stack: (empty)
```

Read it left to right: push 3, push 4, add (stack: 7), push 5, add
(stack: 12), print.

## Naming values

Sooth has locals. You bind the top of the stack to names with
`| names |`, and those names are available for the rest of the
surrounding block. This is how you write readable Sooth: name what you
need, then use the names.

Square a number:

```text
> 5 | x | x x mul .
25
stack: (empty)
```

`5` pushes the value. `| x |` binds it to the name `x`. Then `x x mul`
pushes x twice and multiplies, and `.` prints the result. The name `x`
is available for the rest of the line.

Add three numbers:

```text
> 1 2 3 | a b c | a b add c add .
6
stack: (empty)
```

`| a b c |` binds the top three values: `c` gets 3 (top), `b` gets 2,
`a` gets 1. Then `a b add c add` adds them left to right.

This is the style you will see throughout the book. Instead of
shuffling values around with stack-manipulation words, you name what
you need and use the names. The stack is still there underneath — it's
how values flow through the program — but you interact with it through
named locals, not by tracking positions.

## drop

The one stack word that stays essential is `drop`. It discards the top
of the stack. In Part II you will learn that `drop` is not just
"discard" — it calls a value's destructor, which is how Sooth cleans up
resources without a garbage collector. For now, think of it as the way
to remove a value you no longer need:

```text
> 1 2 drop .
1
stack: (empty)
```

Push 1, push 2 (stack: 1 2). `drop` removes 2 (stack: 1). `.` prints 1.

## Quitting

Type `:quit` to exit. The REPL disposes any remaining values on the
stack (you will learn what "dispose" means in Part II).

## What's next

You can push numbers, name them with locals, add them, print them, and
drop them. The next chapter builds the full mental model: how the stack
carries types, how words declare what they consume and produce, and why
the compiler can catch mistakes before the program runs.
