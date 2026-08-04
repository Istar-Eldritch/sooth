# Move by Default

Every value in Sooth is used exactly once. This is not a style guide —
it is a compile-time check. If you produce a value and don't account
for it, the compiler rejects your program. Nothing runs behind your
back to clean up.

You have already seen this check without knowing it. When a word's
body leaves more values than its stack effect declares, the compiler
says so. That is the linear spine at work.

## Words consume their inputs

When you call a word, the input values are gone. The word takes them
from the stack, and they do not come back unless the word's effect
says so:

```sooth
: add ( i64 i64 -- i64 ) + ;
: main ( -- ) 3 4 add . ;
```

`3` and `4` go in, `7` comes out. The `3` and `4` are consumed — they
are not on the stack after the call. This is what the stack effect
means: `( i64 i64 -- i64 )` says two values go in, one comes out.

If a word wants to give a value back along with a result, it says so
in its effect. A word that takes a number, adds one, and returns both
the original and the sum:

```sooth
: bump ( i64 -- i64 i64 ) | n | n n 1 + ;
: main ( -- ) 5 bump . . ;
```

```text
6
5
```

`n` appears twice in the body. For an `i64`, that is fine — integers
are Copy. We'll get to what Copy means in a moment.

## Nothing is auto-dropped

If you produce a value and forget about it, the compiler catches it.
This is the first rule of the linear spine:

```sooth
: surplus ( i64 -- ) | n | n n + ;
: main ( -- ) 5 surplus ;
```

```text
error: stack effect mismatch in `surplus` (line 1)
  body leaves 1 values, but ( … ) declares 0 outputs
  note: declared ( i64 -- )
```

`n n +` produces one value, but `surplus` declares zero outputs. The
compiler does not silently discard the extra value. It tells you to
account for it: use it, return it, or drop it.

This applies to every type, not just resources. A forgotten `i64` is
the same error as a forgotten file handle. The check is the same
because the discipline is the same: every value is accounted for by
the signature or explicitly dropped.

## drop: explicit disposal

`drop` pops a value from the stack and discards it. It is the one word
that makes a value go away on purpose:

```sooth
: discard ( i64 -- ) drop ;
: main ( -- ) 42 discard 100 . ;
```

```text
100
```

`42` is pushed, `drop` removes it, `100 .` prints `100`. Without
`drop`, the `42` would be left on the stack, and `discard` would fail
its effect check.

`drop` works on any type. For the types you have seen so far —
integers, floats, `bool`, `str` — it simply removes the value. Later,
when you meet types that own resources (Part III), `drop` will run a
destructor. But the syntax is the same: `drop` is the single disposal
word, and you write it yourself.

## Copy: reuse is free

The types you have seen so far are all **Copy**: integers, floats,
`bool`, `str`. A Copy value is just bits. Referencing a Copy local
twice is ordinary reuse — the compiler copies the bits:

```sooth
: square ( i64 -- i64 ) | x | x x * ;
: main ( -- ) 5 square . ;
```

```text
25
```

`x` appears twice in `x x *`. Because `i64` is Copy, the second `x`
is a copy of the first. There is no `dup` word needed — the local
name does it.

This is the "by default" in "move by default." For Copy types, the
default is cheap: mention a local as many times as you like. The
discipline that matters — use exactly once — applies to types that
own resources. Those arrive in Part III. The rules you learn here are
the same rules; they just become load-bearing.

## The stack effect is the enforcement

There is no separate liveness pass or borrow checker. The same stack
effect check you met in chapter 2 does the work. When a word finishes,
the compiler compares the stack it leaves against the effect it
declared. If they don't match, it's an error — regardless of whether
the surplus value is an `i64` or a file handle.

This means the linear spine is not a feature you turn on. It is the
consequence of two things you already know: words declare their stack
effects, and the compiler checks them. The only addition is `drop`,
the explicit way to say "I am done with this value."

## What's next

The next chapter explains why this works — why a stack language's
data flow already encodes move semantics, and why the type system
only needs to distinguish Copy from non-Copy rather than tracking
lifetimes.
