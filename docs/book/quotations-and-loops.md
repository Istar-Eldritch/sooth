# Quotations and Loops

A quotation is a block of code written as a value: `[ 1 add ]`. You
have already used quotations without naming them — every `if` you
wrote in earlier chapters took two of them. This chapter makes the
mechanism explicit: what a quotation is, the two flavors the compiler
distinguishes, and how they replace `while`/`for` as the loop
construct.

## Quotations as values

A quotation literal is a bracketed sequence of terms:

```sooth
[ 1 add ]
```

It is a value like any other — it can be bound, passed as an
argument, and returned. `call` runs it, consuming whatever inputs its
body expects off the stack:

```sooth
import: intrinsics * ;
import: hosted::show | . | ;

: main ( -- )
  5 [ 1 add ] call . ;    \ prints 6
```

A word's stack effect can declare a quotation input with its own
nested effect: `( i64 [ i64 -- i64 ] -- i64 )` takes an `i64` and a
quotation from `i64` to `i64`.

```sooth
import: intrinsics * ;
import: hosted::show | . | ;

: apply ( [ i64 -- i64 ] i64 -- i64 ) | n | | f | n f call ;

: main ( -- )
  [ 1 add ] 5 apply . ;    \ prints 6
```

`apply` is an ordinary word: it compiles to a real function, and the
quotation `[ 1 add ]` is passed to it as a runtime value — a
`(code, env)` pair the callee can `call`, forward to another word, or
store. You can build a whole call chain this way:

```sooth
import: intrinsics * ;
import: hosted::show | . | ;

: apply2 ( [ i64 -- i64 ] i64 -- i64 ) apply ;

: run ( [ -- i64 ] -- i64 ) call ;

: main ( -- )
  [ 1 add ] 5 apply .
  [ 2 mul ] 20 apply2 .
  [ 42 ] run . ;
```

```text
6
40
42
```

`apply2` doesn't call its quotation itself — it forwards it to
`apply`, which does. The value crosses two function boundaries before
it's used.

## The other flavor: `~[ ... ]`

There is a second quotation syntax, `~[ ... ]` (tilde-bracket), used
throughout the standard library and in every `if` you've written:

```sooth
: sign ( i64 -- i64 )
  | n |
  n 0 lt ~[ -1 ] ~[ 1 ] if ;
```

`~[ ... ]` looks like `[ ... ]` but means something stricter:
**inline-only**. A `~[ ]` quotation has no runtime representation at
all — no `(code, env)` pair, nothing you could store in a variable or
pass to an ordinary function. It exists purely at compile time as a
block of terms, and the only thing the compiler can do with it is
splice those terms directly into the caller.

That's why `if` is spelled the way it is: it's not special syntax.
`if` is an ordinary library word, taking a `bool` and two `~[ ]`
quotations:

```sooth
: if inline ( ..a Bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )
  | else-arm | | then-arm | | cond |
  cond tag then-arm else-arm branch ;
```

(`branch` is a compiler intrinsic that picks one quotation by tag and
splices it; `..a`/`..b` are row variables, covered in the polymorphism
chapter.) Every `if`/`unless` you write compiles down to this word,
which is itself compiled away entirely by the time it reaches native
code — see the next section.

## Why `~[ ]` requires `inline`

A word that takes a `~[ ]` parameter must itself declare `inline`
(chapter on [Words](./words.md)). The reason follows directly from
`~[ ]` having no runtime representation: if the word compiled to a
real function, the quotation would have to cross the function
boundary as a value at the call — but there is no value to pass. The
only way the callee's body can use the quotation is if that body is
lowered in the same place the quotation literal exists, which means
the whole word must be spliced into its caller.

Concretely:

```sooth
import: intrinsics * ;
import: hosted::show | . | ;

: twice inline ( i64 ~[ i64 -- i64 ] -- i64 )
  | f | f call f call ;

: main ( -- )
  5 ~[ 1 add ] twice . ;    \ prints 7
```

`twice` mints no function of its own. Its body — `f call f call` —
is spliced into `main` with `~[ 1 add ]`'s terms spliced again in
place of each `f call`. The lowered `main` is, in effect, `5 1 add 1
add .`: two additions, no calls, no quotation value anywhere at
runtime.

This makes `~[ ]` **unconditionally zero-cost** — a guarantee, not an
optimization. Compare this to a generic higher-order function in a
language like Rust: `fn apply<F: Fn(i64) -> i64>(x: i64, f: F) -> i64`
usually also compiles away to nothing after monomorphization and
inlining, but that's the optimizer choosing to do so, contingent on
function size and inlining heuristics — it can fail to happen, and
`dyn Fn` explicitly opts out of it (vtable dispatch, often a heap
allocation). Sooth's `~[ ]` has no such fallback path: the type
carries no ABI, so there is nothing an "un-inlined" version could
even mean. The compiler either splices it or the program doesn't
compile.

The trade-off is the one you'd expect: a `~[ ]` value can't be stored
in a variable, returned, or put in an array of callbacks — for that
you need the ordinary `[ ]` flavor, at the cost of a real call.

## Capturing enclosing state

A quotation body can reference a name it doesn't bind itself — a
local from the enclosing word. That's a capture:

```sooth
: adder ( i64 -- [ i64 -- i64 ] )
  | n | [ n add ] ;
```

What's allowed to be captured, and how, depends on whether the
captured value is `Copy` (chapter on [Move by Default](./move-by-default.md))
and on which of the two quotation flavors is doing the capturing.

**A `Copy` local captures freely**, in either `[ ]` or `~[ ]`, no
restrictions — `adder` above is the ordinary case: `n` is `i64`, so
the returned quotation just carries a copy of it.

**A linear local is stricter.** Say `Fd` is a resource type with an
overloaded `drop` (chapter on [Disposal](./disposal.md) — an
overloaded `drop` makes a struct linear, not `Copy`, so it must be
consumed exactly once). A `[ ]` literal that captures a linear local
and is called immediately, in the same body, is fine — the checker
can see the single use:

```sooth
import: intrinsics * ;

: main ( -- )
  7 Fd | f |
  [ f drop ] call ;    \ fine: called once, right here
```

But pass that same literal to another word instead of calling it
directly, and the checker rejects it:

```sooth
: run ( [ -- ] -- ) call ;

: main ( -- )
  7 Fd | f |
  [ f drop ] run ;
```

```text
error: the quotation passed to `run` consumes the enclosing local `f`,
which is linear; a quotation may only read a `Copy` enclosing local by
value (D3) in `main`
```

The hazard is real, not pedantic: once the quotation is an ordinary
value crossing into another word's frame, nothing stops that word
from calling it twice (double-freeing `f`) or not at all (leaking
it). `call`ing the literal in place is the one shape the checker can
prove is safe without that guarantee; anything else needs a stronger
boundary.

Returning a capturing closure hits a related but distinct error —
the closure would outlive the frame `f` lives in:

```sooth
: mk ( -- [ -- ] )
  7 Fd | f |
  [ f drop ] ;
```

```text
error: an escaping closure captures `f`, a local of this frame, whose
storage does not survive the return
  declare the boundary `owning [ ... ]` to hand the closure ownership
  of `f`, so calling it disposes `f`
```

**`owning [ ... ]`** is that boundary — a third quotation spelling,
for a closure that owns what it captures:

```sooth
import: intrinsics * ;

: mk ( -- owning [ -- ] )
  7 Fd | f |
  [ f drop ] ;

: main ( -- )
  mk call ;    \ prints 7
```

An `owning [ ... ]` value is itself linear — not `Copy` — so it
inherits the same exactly-once discipline as `Fd`: it can't be
duplicated, and it can't be silently forgotten. Consuming it disposes
whatever it captured. `call` runs the body (which is where `f drop`
happens above); `drop`ping an unused `owning [ ... ]` without calling
it still runs a generated disposer that drops each capture, so
nothing leaks either way — `mk drop` prints `7` exactly like `mk
call` does, just without running the rest of the body.

`owning [ ... ]` can't be the parameter of an `inline` word (an inline
word is spliced, and splicing needs the captures visible as plain
locals, not boxed behind a heap-allocated environment) and can't
appear in a generic word's signature. It's specifically the type for
a closure that has to be a real, storable, disposable value.

`~[ ]` sits outside this whole discussion. Because it's always
spliced and never becomes a runtime value, a name it reads from the
enclosing body isn't a *capture* in the closure sense at all — after
splicing, it's just an ordinary local reference in the same frame. A
`~[ ]` quotation can read or consume a linear local with no `owning`
boundary and no D3 restriction:

```sooth
import: intrinsics * ;
import: core::prelude * ;

: main ( -- )
  7 Fd | f | True ~[ f drop ] ~[ f drop ] if ;    \ prints 7
```

This is why `if`, `times`, and the array combinators from the next
section can thread linear state through their quotation arguments for
free: there's no closure environment to manage, because there's no
closure.

## Loops are combinators

Sooth has no `while` or `for` keyword. Loops are ordinary words that
take a `~[ ]` body and splice it into a loop shape. `times`, from the
standard library, runs its quotation a fixed number of times:

```sooth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::combinators c | times | ;

: main ( -- )
  0 5 ~[ | i | i add ] times . ;    \ prints 10
```

`times` hands the quotation the 0-based index (`i`) on each
iteration; the accumulator (`0`, then each partial sum) threads
through the state row below it. This isn't recursion under the hood
— `times` is written as a self-tail combinator, so calling it
compiles to a loop back-edge in the emitted code, running in constant
stack regardless of how many iterations it performs. (Ordinary
recursion, from the [Words](./words.md) chapter, has no such guarantee
unless it's in the same self-tail shape.)

`while` is the general form: it threads a state value through a
predicate quotation until the predicate says stop.

```sooth
: while inline ( 'a ~[ 'a -- 'a Bool ] -- 'a )
  | p | p call ~[ p while ] ~[ ] if ;
```

`while` is `inline`, and its recursive call to itself is in tail
position — the same self-tail shape `times` uses — so it also lowers
to a loop, not to unbounded splicing.

## Writing your own combinator

Any word taking `~[ ]` parameters and declared `inline` is a
combinator. The array library builds `each`, `map`, `fold`, and
`filter` this way, each a thin wrapper driving `times`:

```sooth
: each inline ( ['T 'N] ~[ 'T -- ] -- )
  | f | len >i64 | count | | arr |
  count ~[ | i | &arr i >usize &> @ f call ] times
  arr drop ;
```

`each` takes an array and a quotation, and calls the quotation once
per element. Because everything here is `inline`, a call like
`arr ~[ . ] each` splices three levels deep — `each`'s body, then
`times`, then `times-helper` — down to a single loop with no
per-element function call. The library keeps these as separate
"leaf" combinators (rather than building `map`/`fold` on top of
`each`) specifically to avoid stacking splice depth at every call
site; see `lib/core/combinators.sth` for the full reasoning.

## What's next

Quotations are how Sooth expresses control flow, loops, and
higher-order code without a distinguished syntax for any of them —
`if`, `times`, and `each` are all ordinary words. The next chapter
covers polymorphism: how a word like `times` or `each` can be generic
over the element type and still compile to concrete, monomorphic code
at every call site.
