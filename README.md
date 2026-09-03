# Sooth

A small, statically-checked concatenative language, compiled to native
code with no external runtime. See [DESIGN.md](./DESIGN.md) for the why and
[ROADMAP.md](./docs/roadmap/ROADMAP.md) for the plan.

The one bet that makes it more than a tidy Forth: in a stack language the stack
discipline already is move semantics, so linear types fall out for free. `dup`
is the explicit copy, and `drop` is the explicit destructor point (use exactly
once; forgetting is a compile error, nothing auto-drops). A DMA transfer *is* an
ownership transfer; "don't touch the buffer while the controller owns it"
becomes a type error instead of a comment in a driver.

```factor
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

: gcd ( a: i64 b: i64 -- i64 )
  b 0 eq
  ~[ a ]
  ~[ b a b mod gcd ]
  if
;

: main ( -- )
  10 15 gcd . \ prints 5
;    
```

## What it is

A concatenative language with two ergonomics Forth lacks: statically-checked
stack effects and named locals. Every word declares its effect (`( int int --
int )`), the compiler verifies the body against it, and a stack underflow
becomes a compile error, the signature failure mode of Forth, caught at
compile time rather than as a wrong number at runtime. Input slots can carry
their names right in the effect — `( a: i64 b: i64 -- i64 )` binds `a` and
`b` as locals before the body runs; a body-level `| ... |` block remains for
mid-body binding and for the places inline names can't go (quotations, impl
members, polymorphic effects).

```factor
: oops ( a: i64 -- i64 )
  a a add add
;

\ error: stack effect mismatch in `oops` (line 2)
\   `add` needs 2 values, but the stack holds 1
\   note: declared ( i64 -- i64 )
```

The type system is deliberately small: concrete monomorphic types, ADTs,
minimal row polymorphism for honest `dup`/`swap`/`max` signatures, user-declarable
trait bounds, and a `Copy` marker that distinguishes copyable data from linear
resources. No full HM inference, no refinement types, no effect rows, no borrow
checker.

## Features

**Linear types.** Plain data is `Copy`, reuse is free, `dup` copies the bits.
A resource is linear: `dup` on something that owns a resource is a type error.
`drop` is the sole disposal primitive; forgetting to dispose is a compile error.
This gives resource safety, deterministic destruction, and data-race-free concurrency
without a borrow checker or `Send`/`Sync` apparatus.

```factor
: leak ( File -- File File )
  dup
;

\ error: cannot `dup` a value of type File
\   File is linear: it owns an OS handle and has no Copy instance
```

**Checked stack effects.** Every word body must net to its declared effect. A
forgotten `int` and a forgotten `File` surface through the same check.

**ADTs and structural dispatch.** Enums with exhaustiveness-checked
elimination through the generated eliminator word (`Shape?`), no inline `match`:

```factor
type: Shape
  | Circle r f64
  | Rect   w f64 h f64
;

: area ( Shape -- f64 )
  ~[ ( Circle ) Circle> dup mul PI mul ]
  ~[ ( Rect )   Rect> | w h | w h mul ]
  Shape? ;
```

**Second-class references.** `&a`/`&!a` borrows a local, `@`/`!`/`+!` reads and
mutates through a reference. Governed by per-place exclusivity, not by a
lifetime system, references can't escape their scope, so no lifetime variables
or region annotations are needed.

**Resources with user-definable destructors.** A linear type with a `drop`
overload runs its destructor exactly where the programmer writes disposal. The
FFI is the explicit unsafe hole, wrapped in safe words:

```factor
extern: open     ( cstr i64 -- i64 )              "open" ;
extern: close-fd ( i64 -- i64 )                   "close" ;

type: Fd n i64 ;
: drop ( h: Fd -- ) h Fd> close-fd drop ;

type: File fd Fd ;          \ derived glue disposes Fd through its own drop
: close ( File -- ) drop ;
```

**Bounded polymorphism.** `'T`/`'N`/`..s` type, length, and row variables,
monomorphized per instantiation, no vtables. `dup`/`swap`/`max` get honest
generic signatures. A type variable can carry trait bounds, declared in a bracket between the
word's name and its effect (`: w['T: Order Show] ( ... )`) and resolved at each
call site against the caller's own `impl:` blocks.

**Traits and trait dispatch.** A `trait:` declaration lists required member
signatures over a type variable; an `impl:` block gives a concrete type its
own member bodies, inheriting each signature from the trait. A bounded word
(`['T: Order]`) dispatches each required member as an ordinary word call resolved
at monomorphization — never a runtime vtable. `Copy` and `Ord` are built-in
predicates satisfied by a type's own shape; user-declared traits are nominal,
not structural.

```factor
\ Named Display, not Show: core::show already owns Show/show for the
\ printing vocabulary below, with a different (by-value, sink-taking) shape.
import: hosted::show | . | ;

trait: Display['T]
  : display ( &'T -- ) ;
;

impl: Display for Point
  : display | p | "(" . p &x @ . "," . p &y @ . ")" . ;
;

: print-larger['T: Order Display] ( &'T &'T -- )
  | a b | a b cmp
  ~[ drop b display ]
  ~[ drop a display ]
  ~[ drop a display ]
  Rank? ;
```

**Quotations and combinators.** `~[ ... ]` inline quotations splice at their
call site (no runtime closure), `[ ... ]` literals are first-class values
carried on the stack, and `owning [ ... ]` quotations can capture linear
resources with a per-site disposer. `call` invokes a quotation value, and an
internal loop primitive lowers self-tail recursion to constant stack. The
combinator library (`each`/`map`/`fold`/`filter`/`while`/`times`) is written in
Sooth itself, inlined at call sites by a term-splicing pass, no per-element
call overhead. No combinator is compiler-known, not even `if`, which is a
`core::bool` word over the machine primitives `branch` and `tag`.

```factor
import: intrinsics * ;
import: hosted::show | . | ;
import: core::combinators | times | ;

: main ( -- )
  0 1000000 ~[ 1 add add ] times . ;   \ prints 500000500000
```

**Modules and packages.** A file is a compilation unit, a directory tree under
a `sooth.pkg` manifest is a package. `import:`/`export:` with qualified access,
selective imports, and transitive dependency resolution.

**Recursion.** Self-tail-recursion is a guaranteed constant-stack transform
(tail self-call → jump). A recursive list destructor disposes in constant stack,
verified past a million nodes under a 1 MB stack:

```factor
import: hosted::show | . | ;

type: List 
  | Nil 
  | Cons 
    v i64 
    next ^List 
;

: push-front ( rest: List v: i64 -- List )
  v rest ^ Cons ;

: build ( n: i64 acc: List -- List )
  n 0 eq
  ~[ acc ]
  ~[ n 1 sub acc n push-front build ]
  if
;

: main ( -- )
  10 Nil build           \ build a 10-element list
  3 0 sum-first Summed>  \ sum the first 3
  . drop ;               \ drop disposes the rest (constant-stack)
```

## Design philosophy

Sooth is built for the pleasure of building it and writing programs in it. It
stays small enough to hold in one head, with a compiler legible enough to read.
Where a decision trades reach or peak performance for simplicity and legibility,
simplicity wins.

But it is not a toy. The target domain is embedded and real-time systems.
This is where the linear spine shows its value:
A DMA transfer is an ownership transfer, unsynchronised ISR/mainline sharing
is a set intersection the checker can compute, and "deterministic destruction
at a statically-known time" is the default.
