# Numbers

Sooth has a fixed-width integer tower, two float widths, and a `bool`
type. No implicit promotion, no big integers, no automatic widening.
Every conversion is explicit. This chapter covers the types, the
operators, and how literals interact with the type system.

## The integer tower

There are eight integer types, plus `usize` and `isize`:

| Type | Width | Signed |
|------|-------|--------|
| `i8` | 8-bit | yes |
| `i16` | 16-bit | yes |
| `i32` | 32-bit | yes |
| `i64` | 64-bit | yes |
| `u8` | 8-bit | no |
| `u16` | 16-bit | no |
| `u32` | 32-bit | no |
| `u64` | 64-bit | no |
| `usize` | pointer-width | no |
| `isize` | pointer-width | yes |

`usize` and `isize` are pointer-width integers — 64-bit on a 64-bit
target. You will meet `usize` again in Part III, where it indexes
arrays. `i64` is the default: a bare integer literal like `42` is an
`i64`.

## Floats

Two float widths: `f32` (32-bit) and `f64` (64-bit). A bare float
literal like `3.14` is an `f64`.

```text
> 3.14 .
3.14
stack: (empty)
> 3.0 .
3
stack: (empty)
> 0.5 .
0.5
stack: (empty)
```

## Arithmetic

The arithmetic operators are `+`, `-`, `*`, `/`, and `mod`:

- `+`, `-`, `*` work on any pair of the same numeric type (integers or
  floats).
- `/` is **float-only**. There is no integer division. Use `mod` for
  the remainder.
- `mod` is **integer-only**. It gives the remainder of integer
  division.

```text
> 1 2 + .
3
> 10 3 mod .
1
> 7.0 2.0 / .
3.5
> 10 3 /
error: type mismatch: `/` requires two operands of the same float
type (integer division is unsupported), found `i64` and `i64`
```

Both operands must be the same type. There is no implicit widening or
promotion — `i32` and `i64` do not mix, `i8` and `u8` do not mix,
`f64` and `i64` do not mix:

```text
> 1 >i32 2 >i64 + .
error: type mismatch: `+` requires two operands of the same numeric
type, found `i32` and `i64`
```

## Bitwise and shifts

`and`, `or`, `xor` are bitwise on integers and logical on `bool`. `not`
is bitwise complement on integers and logical negation on `bool`. The
shift operators `shl` and `shr` take any integer on the bottom and an
`i64` shift count on top:

```text
> 12 10 and .
8
> 12 3 or .
15
> 12 10 xor .
6
> 5 not .
-6
> 1 4 shl .
16
> 256 3 shr .
32
```

## Comparisons

Six comparison operators: `=`, `<>`, `<`, `>`, `<=`, `>=`. They work
on any pair of the same numeric type and produce a `bool`:

```text
> 1 2 < .
true
> 1 2 > .
false
> 1 1 = .
true
> 1 2 <> .
true
> 3 2 >= .
true
```

Comparisons do not work on `bool`. There is no bool ordering. For
bool logic, use `and`, `or`, `xor`, `not` instead.

## max

`max` takes two integers of the same type and leaves the larger. It
does not work on floats — IEEE 754 has NaN, and a naive `max` would
silently pick the wrong value. Use `max-total` for floats:

```text
> 3 7 max .
7
> 1.0 2.0 max .
error: type mismatch: `max` does not support float operands (found
`f64` and `f64`); use `max-total` for a total-ordered float maximum
> 1.0 2.0 max-total .
2
```

## Conversions

Every type change is explicit. Conversion words are named `>type`:

```text
> 42 >i8 .
42
> 42 >u8 .
42
> 42 >f64 .
42
> 3.7 >i64 .
3
```

The full set: `>i8`, `>i16`, `>i32`, `>i64`, `>u8`, `>u16`, `>u32`,
`>u64`, `>f32`, `>f64`, `>usize`, `>isize`. Each takes one numeric
value and produces the named type. Integer-to-integer conversions
truncate. Float-to-integer conversions truncate toward zero.
Integer-to-float conversions widen. Float-to-float conversions narrow
or widen.

Conversions can change the printed value. A negative `i64` converted
to `u8` wraps:

```text
> -1 >u8 .
255
> -1 >u64 .
18446744073709551615
```

## Literals and usize

A bare integer literal is an `i64`. When it appears in a position
that expects `usize` or `isize`, the compiler coerces it automatically
— but only a bare literal, not a computed value.

This means a literal argument to a word that expects `usize` works:

```sooth
: needs-usize ( usize -- ) drop ;
5 needs-usize       \ works: 5 is a bare literal, coerced to usize
```

But a computed `i64` value does not:

```text
> 1 1 + needs-usize
error: type mismatch: `needs-usize` mixes `usize` with a computed `i64`:
convert it explicitly with `>usize` first
```

`1 1 +` is a computed `i64`. You must convert it explicitly with
`>usize`. The compiler can confirm a bare literal fits in the target
width, but a computed value has no known value at compile time, so the
compiler refuses to guess.

This coercion is specific to `usize` and `isize`. Other integer types
(`i8`, `u16`, etc.) never auto-coerce — you always need an explicit
conversion:

```sooth
: needs-i8 ( i8 -- ) drop ;
42 >i8 needs-i8    \ works
42 needs-i8        \ error: expected i8, found i64
```

## Overflow

Arithmetic wraps silently on overflow. There is no overflow check at
runtime or compile time. `255 >u8 1 >u8 +` gives `0`:

```text
> 255 >u8 1 >u8 + .
0
```

This is two's-complement wraparound, the same behavior as C. If you
need bounds checking, you write it yourself with comparisons.

## bool

`bool` is its own type, not an integer. It has two literal values:
`true` and `false`. The word `if` consumes a `bool` (chapter 4). The
bitwise words `and`, `or`, `xor`, `not` work on `bool` as logical
operators:

```text
> true true and .
true
> true false or .
true
> true not .
false
> true false xor .
true
```

## Printing

The `.` word prints any scalar. It picks the format from the type:
signed integers print as signed decimal, unsigned integers as unsigned
decimal, floats with `%g`, and `bool` as `true` or `false`:

```text
> 42 .
42
> 3.14 .
3.14
> true .
true
> -1 >u32 .
4294967295
```

## What's next

You now know the numeric types, the operators, and the conversion
rules. The next part of the book covers the linear spine — the
property that makes Sooth's memory management work without a garbage
collector.
