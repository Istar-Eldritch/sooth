# Words

A word is a named, reusable computation with a declared stack effect.
You have been writing them since chapter 1. This chapter covers the
definition syntax, how names resolve, and how words find each other at
compile time.

## Defining a word

A word definition has four parts: the name, the stack effect, the
body, and the terminating `;`:

```sooth
: inc ( i64 -- i64 ) | x | x 1 add ;
```

The colon starts the definition. The name follows — `inc`. The
parentheses hold the stack effect (chapter 2). The body is everything
between the effect and `;`. The semicolon ends the definition.

The body is a sequence of terms. You have already seen every kind of
term there is: literals (`42`, `"hello"`), names (`inc`, `add`, `x`),
and bindings (`| a b |`). There is no statement separator — terms are
whitespace-delimited, and the parser reads them left to right until it
hits `;`.

## Inline words

The `inline` keyword goes between the name and the stack effect:

```sooth
: inc inline ( i64 -- i64 ) | x | x 1 add ;
```

An inline word is **spliced** at every call site: the compiler copies
the word's body into the caller and lowers it there. No function is
emitted for the word, and no call instruction appears in the caller.
The effect is text expansion — `41 inc` lowers exactly as `41 1 add`
would.

This works for any word, not just words that take quotations. A small
helper marked `inline` costs nothing at runtime:

```sooth
import: intrinsics * ;
import: hosted::show | . | ;

: square inline ( i64 -- i64 ) | x | x x mul ;

: main ( -- )
  7 square . ;    \ prints 49
```

`square` mints no function of its own; `main` contains the `mul`
directly.

Inline is **required** for words that take inline quotation parameters
(`~[ ... ]`). An inline quotation has no runtime representation, it
can only be spliced, so any word that accepts one must itself be
spliced. The array combinators in `lib/core/combinators.sth` are all
inline for this reason:

```sooth
: each inline ( ['T 'N] ~[ 'T -- ] -- )
  | f | len >i64 | count | | arr |
  count ~[ | i | &arr i >usize &> @ f call ] times
  arr drop ;
```

Two restrictions: `main` cannot be `inline` (it is called by the
runtime entry point, not spliced), and a word whose name overloads a
builtin operator (`add`, `mul`, etc.) cannot be `inline`.

## Names

When the compiler sees a bare name in a word body, it checks the
locals in scope first. If the name matches a local, it pushes that
value onto the stack. If not, it looks for a word with that name. If
neither exists, it's a compile error.

There is no syntactic difference between calling a word and
referencing a local — you write the name, the compiler figures out
which it is. This means locals shadow words: if you bind `| foo |`
and there is also a word named `foo`, the local wins for the rest of
the block.

```text
> : foo ( -- i64 ) 1 ;
> : bar ( i64 -- i64 ) | foo | foo foo add ;
> 5 bar .
10
stack: (empty)
> foo .
1
stack: (empty)
```

Inside `bar`, `foo` is the local (5), so `foo foo add` gives 10. Outside
the block, `foo` is the word, which leaves 1.

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
import: intrinsics * ;
import: core::prelude * ;
import: hosted::show | . | ;

: main ( -- )
  5 factorial . ;

: factorial ( i64 -- i64 )
  dup 0 eq ~[ drop 1 ] ~[ dup 1 sub factorial mul ] if ;
```

**In the REPL**, each line is processed independently. A word must
exist before it can be called — there is no forward reference within a
session. Define helpers first, then words that use them.

## Recursion

A word can call itself. The compiler resolves the self-reference
because the word's name is in scope from the moment its definition
begins.

```sooth
import: intrinsics * ;
import: core::prelude * ;
import: hosted::show | . | ;

: countdown ( i64 -- )
  dup 0 eq ~[ drop ] ~[ dup . 1 sub countdown ] if ;
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

## Comments

A backslash starts a line comment. Everything from `\` to the end of
the line is ignored:

```sooth
\ compute the greatest common divisor
: gcd ( i64 i64 -- i64 )
  | a b |
  b 0 eq ~[ a ] ~[ b a b mod gcd ] if ;
```

Comments can appear anywhere whitespace can. They are for the reader;
the compiler ignores them.

## The main word

A file compiled with `sooth build` or `sooth run` must define a word
named `main` with effect `( -- )`. The binary entry point executes
your `main` word. If you forget it, the linker fails with an undefined
reference to `sooth_main`.

```sooth
import: intrinsics * ;
import: core::prelude * ;
import: hosted::show | . | ;

: main ( -- )
  10 15 gcd . ;
```

The REPL does not use `main`. Each line you type is compiled and
executed immediately.

## What's next

You now know how to define words, resolve names, and recurse. The
next chapter covers the numeric types: the fixed-width integer tower,
floating point, and the conversions between them.
