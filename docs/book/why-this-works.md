# Why This Works

The previous chapter said every value is used exactly once, and the
stack effect check enforces it. This chapter explains why that check
is enough — why a stack language doesn't need a borrow checker or
lifetime annotations to get move semantics.

## The stack is the data flow

In a language with named variables, data flow is implicit. A function
like `fn add(a: i64, b: i64) -> i64` names its inputs, but the
compiler must track where `a` and `b` live, whether they're moved,
whether they're borrowed, and when they die. That tracking is the
borrow checker.

In a stack language, data flow is explicit. Values enter at the bottom
of the stack effect, leave at the top, and the effect is a contract:

```sooth
: sub ( i64 i64 -- i64 ) - ;
```

Two `i64`s go in, one comes out. There is nothing to infer. The
programmer wrote the data flow down in the effect declaration, and the
compiler checks that the body matches.

When you call `sub`, the two values are consumed. They are gone from
the stack. There is no way to reference them again because the stack
is the only state — there is no named variable that holds a stale
reference to a consumed value.

## Consumption is move

"Move" means a value was transferred from one owner to another, and
the original owner can no longer use it. In a stack language, this is
what happens at every word call. The word takes the values from the
stack — they are moved into the word. They do not come back unless the
effect says so.

A word that takes a value and gives it back is *threading*:

```sooth
: print-passthrough ( i64 -- i64 ) | n | n . n ;
```

`n` goes in, is printed, and is put back. The effect `( i64 -- i64 )`
says one value enters and one leaves. The caller gets the value back.

A word that takes a value and does not give it back has consumed it:

```sooth
: consume-and-print ( i64 -- ) | n | n . ;
```

The effect `( i64 -- )` says one value enters and nothing leaves.
After the call, the value is gone. The caller cannot use it.

This is move semantics. The difference is that in Rust, the compiler
must *prove* a moved variable isn't used again — that proof is the
borrow checker. In Sooth, there is nothing to prove. The value was on
the stack, the word took it, it's gone. The stack discipline already
enforced the move.

## No lifetimes

In Rust, a function that returns a reference needs a lifetime
annotation:

```rust
fn first(s: &str) -> &str { /* ... */ }
```

The compiler must verify that the returned reference doesn't outlive
its source. Lifetimes are the annotation system that makes this
verification possible.

Sooth has no lifetimes. A word that takes a value and gives it back
just says so in its effect:

```sooth
: first-byte ( str -- str u8 ) \ …
```

The `str` goes in and comes back, along with a `u8`. The data flow is
in the effect. The compiler doesn't need to track a relationship
between the input and the output because there is no relationship to
track — the value flowed through, the effect said it would, and the
body matched.

References exist in Sooth (Part IV), but they are a type with their
own rules, not a lifetime annotation system. There is no `'a`, no
region analysis, no variance.

## Only one distinction: Copy vs non-Copy

The type system needs exactly one bit of information per type: can it
be copied?

**Copy** types — integers, floats, `bool`, `str` — are just bits.
Mentioning a local twice copies the bits. There is no ownership to
track because there is nothing to own:

```sooth
: square ( i64 -- i64 ) | x | x x * ;
```

`x` appears twice. The compiler copies it. No move state, no tracking.

**Non-Copy** types — which arrive in Part III — own resources. A
non-Copy local can be mentioned exactly once. Mentioning it moves it.
Mentioning it again is a compile error. The compiler tracks this with
a simple per-name check: is this name live or has it been moved? That
check is a lookup in a small table, not a borrow checker.

Why is the check so simple? Because the constraints that make borrow
checking hard don't exist here. There are no partial moves (you move
the whole value or nothing). There are no borrows with lifetimes
(references are a separate type). There is no auto-drop (you dispose
explicitly). Without those, "has this name been used?" is the only
question.

## The effect check is the liveness check

There is no separate pass that walks the program looking for dead
values. The stack effect check does the job. When a word body
finishes, the compiler compares the stack it leaves against the effect
it declared:

- Too many values: something was forgotten. A surplus `i64` is the
  same error as a surplus file handle.
- Too few values: something was over-consumed. The body used a value
  the effect promised to return.
- Wrong types: the body produced something the effect didn't promise.

This works for every type because the check is about stack shape, not
about individual values. The compiler doesn't care whether the surplus
value is an `i64` or a `File` — it cares that the body and the effect
disagree.

## What you don't get

The simplicity comes from things the language does not have:

- **No partial moves.** You cannot move a field out of a struct and
  leave the rest. You either have the whole struct or you don't.
- **No borrows with lifetimes.** References are a separate type (Part
  IV), not a lifetime system. There is no `'a` to annotate or infer.
- **No auto-drop.** A value that goes out of scope is not silently
  destroyed. If you forget it, the effect check catches it. If you
  want it gone, you write `drop`.

Each of these would add power. Each would also add a borrow checker.
Sooth trades that power for a type system you can hold in your head.

## What's next

The next chapter covers how branching interacts with the linear spine.
Both branches of an `if` must leave the same stack shape — and for
non-Copy types, that means a value can't be forgotten in one branch
and kept in the other.
