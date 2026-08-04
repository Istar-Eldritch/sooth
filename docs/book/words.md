# Words

A word is a named, reusable computation with a declared stack effect.
You have been writing them since chapter 1. This chapter covers the
full syntax: how to define them, how they call each other, how control
flow works, and what the compiler checks.

## Defining a word

A word definition has four parts: the name, the stack effect, the
body, and the terminating `;`:

```sooth
: inc ( i64 -- i64 ) | x | x 1 + ;
```

The colon starts the definition. The name follows — `inc`. The
parentheses hold the stack effect. The body is everything between the
effect and `;`. The semicolon ends the definition.

The body is a sequence of **terms**. A term is one of:

- A **literal**: `42`, `3.14`, `true`, `"hello"`.
- A **word call**: `inc`, `+`, `.`.
- A **local reference**: using a name bound by `| names |`.
- A **binding**: `| a b c |`, which pops values from the stack and
  names them.
- A **conditional**: `if … else … end`.

There is no statement separator. Terms are whitespace-delimited, and
the parser reads them left to right until it hits `;`.

## The stack effect, revisited

The effect is a typed contract. We covered it in chapter 2, but here
is the complete picture:

```sooth
: square ( i64 -- i64 ) | x | x x * ;
: answer ( -- i64 ) 42 ;
: print-it ( i64 -- ) . ;
: add3 ( i64 i64 i64 -- i64 ) | a b c | a b + c + ;
```

Inputs come before `--`, outputs after. Both are bottom-to-top: the
leftmost input is the deepest on the stack, the leftmost output lands
deepest. If there are no inputs, the `--` still appears. If there are
no outputs, nothing follows `--`.

The effect is not a comment. The compiler checks the body against it
and checks every call site against it. Get either wrong and the
program does not compile.

## Words calling words

A word's body can call other words. The calls compose by stack effect,
as you saw in chapter 2:

```text
> : inc ( i64 -- i64 ) | x | x 1 + ;
> : double ( i64 -- i64 ) | x | x 2 * ;
> : inc-and-double ( i64 -- i64 ) inc double ;
> 5 inc-and-double .
12
stack: (empty)
```

`inc` takes one `i64`, leaves one. `double` takes that, leaves one.
The net effect is `( i64 -- i64 )`, which matches the declaration.

**In a file**, a word can call any other word in the same file,
regardless of definition order. The compiler collects all definitions
first, then checks. This lets you put `main` at the top and helpers
below:

```sooth
: main ( -- ) 5 factorial . ;

: factorial ( i64 -- i64 )
  | n |
  n 1 = if
    1
  else
    n n 1 - factorial *
  end ;
```

**In the REPL**, each line is processed independently. A word must
exist before it can be called — there is no forward reference within a
session. Define helpers first, then words that use them.

## Recursion

A word can call itself. The factorial example above is recursive:
`factorial` calls `factorial`. The compiler resolves the self-reference
because the word's name is in scope from the moment its definition
begins.

```sooth
: countdown ( i64 -- )
  | n |
  n 0 = if
  else
    n .
    n 1 - countdown
  end ;
```

```sh
sooth run countdown.sth
```

```text
3
2
1
```

Recursion is the primary looping mechanism in Sooth. There is no
`while` or `for` — you use recursion, and later, quotations (Part V).

## Control flow: if / else / end

Sooth has one control-flow construct: `if`. It consumes a `bool` from
the stack and runs one of two branches:

```sooth
: sign ( i64 -- i64 )
  | n |
  n 0 < if
    -1
  else
    1
  end ;
```

The `if` term reads the condition from the stack. The `then` branch
runs between `if` and `else` (or `end` if there is no `else`). The
`else` branch runs between `else` and `end`.

Both branches must leave the same stack shape — same number of values,
same types. This is because the caller does not know which branch ran;
the stack effect must hold regardless. The compiler enforces this:

```sooth
: bad ( i64 -- i64 )
  | n |
  n 0 < if
    -1
  else
    "oops"
  end ;
```

The `then` branch leaves an `i64`, the `else` branch leaves a `str`.
The compiler rejects this: the branches disagree on what they leave.

An `if` without `else` is an `if` with an empty else branch. Both
branches still must agree:

```sooth
: print-if-positive ( i64 -- )
  | n |
  0 n < if
    n .
  end ;
```

The `then` branch prints `n` and leaves nothing. The (implicit) `else`
branch is empty and leaves nothing. Both leave zero values, so the
effect checks out.

## Comments

A backslash starts a line comment. Everything from `\` to the end of
the line is ignored:

```sooth
\ compute the greatest common divisor
: gcd ( i64 i64 -- i64 )
  | a b |
  b 0 = if          \ base case: b is zero
    a
  else
    b a b mod gcd   \ recursive case
  end ;
```

Comments can appear anywhere whitespace can. They are for the reader;
the compiler ignores them.

## The main word

A file compiled with `sooth build` must define a word named `main`
with effect `( -- )`. The C runtime calls `sooth_main` on startup, and
your `main` word is the entry point. If you forget it, the linker
fails with an undefined reference to `sooth_main`.

```sooth
: main ( -- )
  10 15 gcd . ;
```

The REPL does not use `main`. Each line you type is compiled and
executed immediately.

## What's next

You now know how to define words, compose them, branch with `if`, and
recurse. The next chapter covers the numeric types: the fixed-width
integer tower, floating point, and the conversions between them.
