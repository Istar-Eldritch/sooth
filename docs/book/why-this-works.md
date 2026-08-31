# Why This Works

The linear spine (every value used exactly once) and stack effects enforce memory safety at compile time. Unlike Rust, Sooth doesn't need lifetimes, borrow checkers, or RAII-aware flow analysis to prevent use-after-free or memory leaks. That's because the stack itself encodes data flow and consumption:

## Stack effects as consumption contracts

Every word declares what it takes (`--` part before) and produces (`--` part after). The compiler checks that the body matches this contract, and ref-checking guarantees the caller provides exactly what's promised.

When a word pushes a value as part of its output, that value is *consumed* from the caller's perspective: any value left on the stack after the call must be accounted for (used, returned, or dropped).

```sooth
import: intrinsics * ;
import: hosted::show | . | ;

: sum ( i64 i64 -- i64 ) add ;
: main ( -- ) 3 4 sum . ;
```

`3` and `4` go in, `sum` consumes both and pushes `7`. After the call, `7` sits on the stack. `.` prints it, removing it from the stack and finishing the word without leftover resources. There is no capacity to silently leak anything.

## Ownership by consumption

"Ownership" in Sooth is encoded by whether the compiler guarantees the value will be used. A `Copy` type (ints, floats, `bool`, `str`) is just bits that can be duplicated freely—mentioning it once copies it, mentioning it twice duplicates the copy (linearity doesn't constrain `Copy`). A non-Copy type (structs, strings, file handles) is an opaque handle to a resource, and the word taking it owns that resource; the value cannot survive if the caller forgets it.

Consequently, ownership is statement-level, not block-level. The effect tells you what the caller receives; any surplus `i64` is a compile error just like a surplus `File` handle. This means:

- There is no lifetime analysis: the only relationship tracked is "did this word leave the exact number/types of values its effect promised?"
- No reference temporary tracking: references (Part IV) are a normal type with their own struct layouts, not a meta-layer over lifetimes.
- No RAII or region inference: a non-Copy local does not "claim" the resource; ownership is whatever the effect provides and the user discards with explicit `drop`.

## Branching with quotations

Branching isolates each path in a quotation, so each path must leave a consistent stack shape. Because quotations are values, the compiler can treat branches as independent stack effects and verify them independently:

```sooth
: sign ( i64 -- i64 )
  | n |
  n 0 lt ~[ -1 ] ~[ 1 ] if ;
```

When the compiler sees the `if`, it checks that `~[ -1 ]` and `~[ 1 ]` produce one `i64`, and it verifies that the surrounding effect declares one `i64` output. The `bool` consumed by `if` comes from the condition (`n 0 lt`), not from the branch quotations themselves.

This makes borrowing and lifetime tracking unnecessary in the branch logic itself. Each branch can splice values freely as long as the quoted body matches the declared output shape.

## What you don't get

The simplicity comes from design choices that Sooth deliberately avoids:

- **No autodrop on scope exit:** Values are only destroyed by explicit `drop` calls. This is why the linear spine exists: the compiler catches forgotten values as outputs that don't match the effect.
- **No partial moves:** No word can move part of a struct and leave the rest. Either you have the full struct value or you don't.
- **No lifetime annotations:** References are types, not meta-annnotations. Their representation is decided at type-check time, not at borrow-check time.
- **No hidden mutable aliases:** All explicit data flow is visible in stack effects; there is no hidden aliasing tracked across words or branches.

What you do get:

- **Explicit, checkable lifetime:** An alias through a reference exists exactly at the area where the reference produces a value. The effect declares when a value is returned; that's the bound on any reference it encloses.
- **No memory leaks:** The linear spine catches any value that wasn't explicitly dropped before word exit.
- **Composable control flow:** Branches are ordinary quotations that can be passed around, composed, and nested without special runtime state.

## What's next

With memory management grounded in the linear spine and stack effects, we move to richer data structures—structs and enums—where the compiler still guarantees that all owned resources are properly handled.
