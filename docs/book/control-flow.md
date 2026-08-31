# Control Flow

Soth has one control-flow construct right now: `if … else … end`. It
branches on a `bool`. Looping is done with recursion. Both of these
will change when quotations arrive in Part V — `if` will become a
word that takes quotations, and loops will be words too. This chapter
covers what exists today.

## if / else / end

`if` consumes a `bool` from the stack and runs one of two branches:

```sooth
: sign ( i64 -- i64 )
  | n |
  n 0 lt if
    -1
  else
    1
  end ;
```

The `then` branch runs between `if` and `else`. The `else` branch runs
between `else` and `end`. The condition comes from the stack, not
from parentheses — `n 0 lt` pushes a `bool`, and `if` pops it.

```text
> -5 sign .
-1
stack: (empty)
> 5 sign .
1
stack: (empty)
```

You can also write a bare `bool`:

```text
> true if "yes" . else "no" . end
yes
stack: (empty)
```

## Branches must agree

Both branches must leave the same stack shape — same number of values,
same types. The caller doesn't know which branch ran, so the stack
effect must hold regardless of the condition. The compiler enforces
this at the point of `if`:

```sooth
: bad ( i64 -- i64 )
  | n |
  n 0 lt if
    -1
  else
    "oops"
  end ;
```

```text
error: type mismatch in `bad` (line 1)
  `if` branches leave different types (then: `i64`, else: `str`)
  note: declared ( i64 -- i64 )
```

The `then` branch leaves an `i64`, the `else` branch leaves a `str`.
The compiler rejects this. It also rejects branches that leave
different *depths*:

```sooth
: bad-depth ( i64 -- i64 )
  | n |
  0 n lt if
    n n
  else
    n
  end ;
```

```text
error: stack effect mismatch in `bad-depth` (line 1)
  `if` branches leave different stack depths (then: 2, else: 1)
  note: declared ( i64 -- i64 )
```

## if without else

An `if` without `else` is an `if` with an empty else branch. Both
branches still must agree:

```sooth
: print-if-positive ( i64 -- )
  | n |
  0 n lt if
    n .
  end ;
```

The `then` branch prints `n` and leaves nothing. The implicit `else`
branch is empty and leaves nothing. Both leave zero values, so the
effect checks out.

```text
> 3 print-if-positive
3
stack: (empty)
> -3 print-if-positive
stack: (empty)
```

## Nesting

`if` branches can contain other `if` terms. The compiler matches
each `else` to the nearest open `if`, and each `end` closes the
innermost:

```sooth
: classify ( i64 -- )
  | n |
  n 0 lt if
    "negative" .
  else
    n 0 eq if
      "zero" .
    else
      "positive" .
    end
  end ;
```

```text
> -5 classify
negative
stack: (empty)
> 0 classify
zero
stack: (empty)
> 5 classify
positive
stack: (empty)
```

## Looping with recursion

There is no `while` or `for`. You loop by calling yourself:

```sooth
: countdown ( i64 -- )
  | n |
  n 0 eq if
  else
    n .
    n 1 sub countdown
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

The base case (`n 0 eq`) stops the recursion with an empty `then`
branch. The `else` branch prints and recurses. This is the pattern
for every loop in Sooth today.

## What's next

This chapter covers the control flow that exists now. When
quotations arrive, `if` and loops become ordinary words that take
code blocks as arguments, and this chapter will be rewritten in those
terms. The next chapter covers the numeric types: the fixed-width
integer tower, floating point, and the conversions between them.
