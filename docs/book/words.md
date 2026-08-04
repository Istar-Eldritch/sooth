# Words

A word is a named, reusable computation with a declared stack effect.
You have been writing them since chapter 1. This chapter covers the
full syntax: how to define them, how names resolve, how control flow
works, and what the compiler checks.

## Defining a word

A word definition has four parts: the name, the stack effect, the
body, and the terminating `;`:

```sooth
: inc ( i64 -- i64 ) | x | x 1 + ;
```

The colon starts the definition. The name follows — `inc`. The
parentheses hold the stack effect (chapter 2). The body is everything
between the effect and `;`. The semicolon ends the definition.

The body is a sequence of **terms**. A term is one of:

- A **literal**: `42`, `3.14`, `true`, `"hello"`.
- A **name**: a bare identifier like `inc`, `+`, `.`, or `x`. The
  compiler resolves it to a local if one is in scope, otherwise to a
  word. There is no syntactic difference between calling a word and
  referencing a local — you write the name, the compiler figures out
  which it is.
- A **binding**: `| a b c |`, which pops values from the stack and
  binds them to names for the rest of the surrounding block.
- A **conditional**: `if … else … end`.

There is no statement separator. Terms are whitespace-delimited, and
the parser reads them left to right until it hits `;`.

## Names

When the compiler sees a bare name in a word body, it checks the
locals in scope first. If the name matches a local, it pushes that
value onto the stack. If not, it looks for a word with that name. If
neither exists, it's a compile error.

This means locals shadow words. If you bind `| add |` and there is
also a word named `add`, the local wins for the rest of the block.
In practice you rarely need to think about this — locals and words
tend to have different names.

## Calling words

A word's body can call other words. The calls compose by stack effect:
the output of one becomes the input of the next. Chapter 2 covered how
this composition is checked. What matters here is **when** the
compiler resolves call targets.

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

A word can call itself. The compiler resolves the self-reference
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

You now know how to define words, resolve names, branch with `if`, and
recurse. The next chapter covers the numeric types: the fixed-width
integer tower, floating point, and the conversions between them.
