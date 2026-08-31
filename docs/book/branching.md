# Branching

Branching in Sooth is explicit and data-driven: you provide two quotations—one for the `then` branch, one for the `else` branch—and the `if` word chooses which one to splice. Both branches must leave the same stack shape for the effect check to pass.

## The `if` word

`if` is an ordinary word that takes **two quotations** in sequence, then controls which one runs:

```sooth
: sign ( i64 -- i64 )
  | n |
  n 0 lt ~[ -1 ] ~[ 1 ] if ;
```

Read the definition left-to-right:

- `n 0 lt` pushes a `bool` indicating whether `n` is negative.
- `~[ -1 ]` is an inline quotation (TildeLBracket) that, when spliced, pushes `-1`.
- `~[ 1 ]` is another inline quotation that pushes `1`.
- `if` consumes the `bool` and the two quotations, then splices either `-1` or `1` depending on the condition.

```sh
sooth build examples/sign.sth && ./sign
```

```text
-5
5
```

The `if` word outputs exactly one `i64`, matching its declared stack effect `( i64 -- i64 )`.

## Stack effect matching

Both branches must leave a stack shape consistent with what comes after the `if`. This is the main compile-time safety guarantee of branching:

```sooth
: bad ( i64 -- i64 )
  | n |
  n 0 lt ~[ -1 ] ~[ "oops" ] if ;  \\ error
```

```text
error: type mismatch in `bad` (line 3)
  `if` branches leave different types (then: `i64`, else: `str`)
  note: declared ( i64 -- i64 )
```

The `then` branch leaves an `i64`, the `else` branch leaves a `str`. The caller expects one `i64`, so the compiler rejects the definition.

The same check applies to stack depth:

```sooth
: bad-depth ( i64 -- i64 )
  | n |
  0 n lt ~[ n n ] ~[ n ] if ;  \\ error
```

```text
error: stack effect mismatch in `bad-depth` (line 3)
  `if` branches leave different stack depths (then: 2, else: 1)
  note: declared ( i64 -- i64 )
```

One branch leaves two values (the `then` stack), the other leaves one (the `else` stack). The effect doesn't know which will be spliced, so it must agree on a single shape.

## Empty `else` branch

An `if` can omit the `else` quotation. This is equivalent to providing an empty quotation on the `else` side. Both branches still must agree on stack shape:

```sooth
import: intrinsics * ;
import: core::prelude * ;
import: hosted::show | . | ;

: print-if-positive ( i64 -- )
  | n |
  0 n lt ~[ n . ] ~[ ] if ;
```

The `then` branch leaves nothing (pushes `n` then prints it and drops it). The implicit `else` branch also leaves nothing. Both leave zero values, so the effect `( i64 -- )` is valid.

```sh
sooth build examples/print-if-positive.sth && ./print-if-positive
```

```text
3
stack: (empty)
```

For a negative input, the `else` branch does nothing (splice of an empty quotation).

## Nesting

Because each branch is a quotation, branches can contain other `if` words. This is how you build multi-way decision trees:

```sooth
import: intrinsics * ;
import: core::prelude * ;
import: hosted::show | . | ;

: classify ( i64 -- )
  | n |
  n 0 lt 
  ~[ "negative" . ]
  ~[ 
    n 0 eq 
    ~[ "zero" . ]
    ~[ 
      n 1 sub 10 lt
      ~[ "one-digit positive" . ]
      ~[ "other positive" . ]
      if
    ]
    if
  ]
  if ;
```

Parsing rules: `else` pushes the next term (which can be another `if`) as the start of the `else` branch, and `end` is only accepted at the topmost level in parsed bodies—inline quotations terminate at their closing quotation brackets.

```sh
sooth build examples/classify.sth && npm run -s classify
```

```text
-5
negative
```

```sh
sooth build examples/classify.sth && npm run -s classify << 'EOI'
0
```

```text
zero
```

```sh
sooth build examples/classify.sth && npm run -s classify << 'EOI'
5
```

```text
one-digit positive
```

```sh
sooth build examples/classify.sth && npm run -s classify << 'EOI'
42
```

```text
other positive
```

## What's next

Branching isolates each branch path with quotations, making control flow safe, composable, and explicit. The next chapter explains why this design works without a borrow checker or lifetime annotations to model branching state.
