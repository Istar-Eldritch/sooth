# The Stack

Chapter 1 showed you the stack as a place to put numbers. Now we look
at it properly: the stack carries *typed* values, words declare their
stack effect as a contract, and the compiler checks every word call
against its declared effect before the program runs.

## Every slot has a type

The stack is not a pile of untyped bits. Each position on the stack
carries a type — `i64`, `str`, `bool`, and others you will meet later.
When you push `42`, the stack holds an `i64`. When you push `"hello"`,
it holds a `str`. The compiler tracks these types as values flow
through your program.

This is why the compiler catches this:

```text
> 1 "hello" + .
error: type mismatch: `+` requires two operands of the same numeric
type, found `i64` and `str`
```

`+` requires two operands of the same numeric type. The compiler sees
`i64` and `str` on top of the stack and refuses before the program runs.
There is no implicit coercion, no string-to-number conversion, no
guessing. The types don't match, so it's a compile error.

## Stack effects

Every word has a **stack effect**: a declaration of what it consumes
from the stack and what it leaves behind. The effect is written in
parentheses between the word name and its body:

```sooth
: square ( i64 -- i64 ) | x | x x * ;
```

Read the effect as a contract: "this word takes one `i64` from the
stack and leaves one `i64` on the stack." The part before `--` is what
the word consumes (inputs, bottom to top); the part after is what it
produces (outputs, bottom to top).

Some common shapes:

```sooth
: answer ( -- i64 ) 42 ;              \ no inputs, one output
: print-it ( i64 -- ) . ;             \ one input, no outputs
: double ( i64 -- i64 ) | x | x 2 * ; \ one in, one out
: add3 ( i64 i64 i64 -- i64 )          \ three in, one out
  | a b c | a b + c + ;
```

A word with no inputs is a constant (or a word that generates a
value). A word with no outputs is a sink — it consumes everything it
takes. Most words transform: they take some inputs and leave some
outputs.

A word can also leave more than one value. The effect lists each output
in order, bottom to top, just like inputs:

```sooth
: dup2 ( i64 -- i64 i64 ) | x | x x ;
: remainder-range ( i64 i64 -- i64 i64 )
  | a b |
  a b mod          \ remainder (first output, bottom)
  a b - a b mod - ; \ difference of remainders (second output, top)
```

`dup2` takes one value and leaves two copies. `remainder-range` takes
two values and leaves two: the remainder on the bottom, the difference
of remainders on top. The caller receives both values on the stack in
that order. Here is a program that uses it:

```sooth
: main ( -- )
  17 5 remainder-range . . ;
```

```sh
sooth build remainders.sth && ./remainders
```

```text
10
2
```

`17 5 mod` is `2` (bottom of stack). The difference is `17 - 5 - 2 = 10`
(top of stack). `.` prints the top first, so `10` appears before `2`.
The stack effect declared the order values land on the stack, and `.`
reads top-first, so the output appears reversed from the declaration.
This is the stack discipline at work: the effect tells you what lands on
the stack, and the stack order tells you what comes off first.

## The effect is a contract the compiler checks

The compiler checks two things: that the body produces what the effect
declares, and that every call site provides what the effect requires.

**The body is checked against its own declaration.** If the body
leaves the wrong number of values, the compiler catches it:

```text
> : bad2 ( i64 i64 -- ) drop ;
error: stack effect mismatch in `bad2` (line 1)
  body leaves 1 values, but ( … ) declares 0 outputs
  note: declared ( i64 i64 -- )
```

`drop` removes one value, leaving one behind — but the effect says zero
outputs. The compiler counts what the body leaves and compares to the
declaration.

If the body leaves the wrong *type*, that's caught too:

```text
> : bad-type ( i64 -- str ) | x | x ;
error: type mismatch in `bad-type` (line 1)
  body leaves `i64` where the declaration requires `str`
  note: declared ( i64 -- str )
```

**Each call site is checked against the word's declared effect.**
If you call a word that needs two values but the stack only has one:

```text
> : needs-two ( i64 i64 -- i64 ) + ;
> 1 needs-two .
error: stack underflow: needs 2 values, but the stack holds 1
```

The compiler sees one `i64` on the stack and knows `needs-two` needs
two. This is a compile-time check, not a runtime one — the program
never runs.

## Composition

Words compose by stacking their effects. The output of one word
becomes the input of the next:

```text
> : double ( i64 -- i64 ) | x | x 2 * ;
> : quadruple ( i64 -- i64 ) double double ;
> 3 quadruple .
12
stack: (empty)
```

Read `quadruple`'s body: `double` takes one `i64` and leaves one, then
`double` takes that and leaves one. The net effect is `( i64 -- i64 )`,
which matches the declaration. The compiler verifies the composition
type-checks end to end.

You can build up complex words from simple ones, and each layer is
checked independently. `double` is checked on its own; `quadruple` is
checked against `double`'s declared effect. If you change `double`
later to take two inputs, `quadruple` breaks at compile time — you
don't have to trace the failure at runtime.

## Reading the stack

The stack is ordered. The REPL shows it bottom-to-top, left-to-right:

```text
> 1 2 3
stack: 1 2 3
```

`1` is at the bottom, `3` is at the top. Words consume from the top.
`.` prints the top, so:

```text
> .
3
stack: 1 2
> .
2
stack: 1
> .
1
stack: (empty)
```

When you write `| a b c |`, the names bind bottom-to-top: `a` gets the
bottom value, `c` gets the top. Remember this — it's the most common
source of off-by-one confusion when reading Sooth for the first time:

```text
> 1 2 3 | a b c | a . b . c .
1
2
3
stack: (empty)
```

`a` is `1` (bottom), `b` is `2`, `c` is `3` (top).

## What's next

You now understand the stack as a typed channel, stack effects as
checked contracts, and how words compose. The next chapter looks at
the integer tower: the fixed-width numeric types, the conversion words,
and why Sooth has no implicit promotion.
