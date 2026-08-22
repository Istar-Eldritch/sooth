# Sooth — memory model

Design detail for the memory model, split from [DESIGN.md](../../DESIGN.md).

## Memory model

Ownership + linear types, deterministic explicit drop, **no tracing GC**. Reference counting
is opt-in only (`Rc`/`Arc`-equivalent), reached for knowingly when shared ownership
is genuinely needed, because dropping the last ref cascades frees synchronously.

References are **second-class**, in the Hylo (mutable value semantics) mould, not
Rust's full borrow checker: refs can be passed into a word but cannot be stored and
cannot escape their scope. Because they can't escape, no *lifetime* system is
needed: no lifetime variables, no region annotations, nothing that binds a
reference's validity to a named scope. Lifetimes attach to named bindings; stack
values are anonymous and shuffled by `swap`/`rot`, so a lifetime system is the worst
possible fit here and stays deliberately avoided. Phase 3 Slice 6 shipped a narrower
rule instead: per-place exclusivity (at most one live mutable reference to a place),
checked at the point each place is consumed rather than by a liveness pass. That is
not a lifetime system — it never asks how long a reference is allowed to live, only
whether two live ones alias — and it works with none of the lifetime apparatus
because a reference already can't escape its creating scope. Affine values plus
non-escaping, exclusivity-checked refs give most of the safety with none of the
lifetime apparatus.

A `&!T` is consequently a third disposal category, neither `Copy` (so it cannot be
`dup`ed) nor linear (so it carries no `drop` obligation): it owns nothing, so a
reference-typed local simply expires, while a reference left on the stack is still a
surplus value. Exclusivity is also keyed per place rather than per path, so two
references into disjoint fields of one local conflict while both are live; sequencing
them is the workaround.

Borrow *liveness* is asymmetric, and only by accident deliberately so: a reference on the
stack is live from the term that creates it until the term that consumes its slot, while a
reference bound to a local is live for the whole block (`live_derivs` chains the stack
slots with the scope's bindings). Chaining a borrow therefore compiles where naming it does
not, and the rejection lands on the natural shape — borrow a place, write through the
borrow, then consume the place. Ending a reference local's borrow at its *last use* would
make the two consistent. That is not a lifetime system by the definition above (no lifetime
variables, no regions, nothing binding a reference's validity to a named scope), only a
rule about when a borrow ends inside one block, and the anonymous case already works that
way. Deferred; see ROADMAP Phase 4 Slice 6f.

The rule that specifying references actually forced into the open is not about references
at all: **naming an aggregate does not copy it**, so two names can denote one region of
memory. That was invisible while nothing could mutate in place, and `!`/`+!` make it
observable. So taking a mutable reference to a place another live name denotes is an error,
and the remedy is `dup`. Two things follow, and both are deliberate. The error fires at the
*borrow*, never at the naming: two names for a value nothing mutates read identically, so
rejecting the naming would refuse correct programs. And no copy is ever inserted for the
programmer, even though that would be the friendlier fix, because `dup` is *the* explicit
copy in a language whose whole point is that copying and destruction are visible, and
because hard real-time here is carried by the programmer's own worst-case reasoning, which
requires instruction counts to be readable off the source. A compiler-inserted copy is the
same category of invisible behaviour as an auto-drop.

The rule is `Copy`-only by construction, which bounds how bad it ever was: every route to a
linear aggregate is closed independently, by move tracking, by `@`'s refusal to
dereference a reference into a linear place, and by the standing ban on linear array
elements. So the failure mode was a wrong *value*, never a double free or a
use-after-free, and the linear spine was never at risk. It is still exactly the class of
silent failure this language exists to turn into a compile error, which is why it is
closed rather than documented.

That same `Copy`-only construction has a payoff that only surfaced under Phase 4's
combinators. A **linear** aggregate threaded through a loop as an accumulator needs no copy
at all: passing it *moves* it, so the outer name is dead, the aliasing rule has nothing to
fire on, and a body that borrows it mutably, writes through the borrow, and hands the same
value back lowers to stores straight into the loop's carried slot. A `Copy` aggregate in
the same position must copy, because another live name could observe the old value — the
`dup` the aliasing rule demands, plus the loop's own back-edge staging. So linearity is not
the awkward case for in-place iteration, it is the *enabling* one, and the awkward case is
an aggregate that is `Copy` by inference. Two things follow. Linearity cannot be
**declared**: `is_copy` derives it structurally, and the only way to opt a type out is to
give it a `drop` overload, which spells "thread this in place" as "give this a destructor".
And the `Copy` case is awkward only because the aliasing rule is keyed on names that are
merely in lexical scope: measured, a `Copy` aggregate accumulator threads with the same zero
copies once a name that is never used again stops counting as a second denoting name. Paying
for a *performance* property with a *semantic* one (a destructor obligation, no free `dup`,
move-on-every-use) is the wrong trade, so that relaxation rides with the borrow one; see
ROADMAP Phase 4 Slice 6f.

Because a branch merge can denote either arm's place, a value carries a *set* of regions
rather than one. The merge unions both arms, so no aliasing rejection ever happens at a
join: selecting one of two owned records takes no borrow and compiles, and the error waits
for the borrow, where the diagnostic can name both ends.

Pointers (`^T`) are non-null by default: there is no compiler-known optional/nullable
pointer type. Nullability, when a program wants it, is `Option['T]` (`lib/option.sth`),
an ordinary generic enum built from Phase 5's `type:` declarations rather than a
compiler primitive — a compiler-synthesized `Option` would be exactly the throwaway
machinery generics exist to replace, so it is never built. The return stack is
hidden or balance-checked; raw return addresses are never exposed.
FFI is the explicit unsafe hole, wrapped in safe words that establish invariants
(same discipline as Rust std over libc), and only exists at the hosted layer.
