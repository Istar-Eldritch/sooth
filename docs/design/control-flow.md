# Sooth — control flow and iteration

Design detail for control flow and iteration, split from [DESIGN.md](../../DESIGN.md).

## Control flow and iteration

Boolean branching is the library word `if` (`~[ then ] ~[ else ] if`, see below);
structural dispatch is the generated eliminator word. There are deliberately **no loop
keywords** (no `begin/until`, `do/loop`); dropping them keeps the surface small and
matches the Factor/Kitten lineage, where iteration is expressed with combinators
rather than syntax.

The iteration story, top to bottom:

- **Quotations are the only iteration primitive.** A quotation `[ ... ]` is a
  first-class code value; `call` invokes it. This is the one piece that must be built
  into the language (new syntax + a value kind); it cannot be a library.
- **An internal loop primitive gives constant-stack iteration.** The IR has a
  loop/back-edge construct (not surface syntax, not user-facing). It is what makes
  iterating a large collection not overflow, independently of whether the TCO pass
  exists.
- **Combinators are library words, not keywords.** `each`, `map`, `filter`, `fold`,
  `while`, `times` are ordinary higher-order words that take quotations, written in
  Sooth. None is compiler-known: each bottoms out on a self-tail call, which the
  compiler lowers to the loop primitive. Reserving them as keywords would bloat the
  core for no reason.
- **Combinators are inlined.** The compiler inlines the common combinators and their
  quotation arguments at the call site, so `~[ ... ] each` lowers to a tight loop with
  the body inlined, not a higher-order `call` per element. This is what makes "loops
  are a library" perform as well as loop syntax would have.
- **Raw recursion is legal but not the idiom.** A word may call itself; it is just a
  word. But threading the stack across a self-call by hand is fiddly, so combinators
  are the normal tool. Tail-call handling is therefore a convenience for hand-written
  recursion, not the lifeline for iteration (combinators over the loop primitive are
  that). Where it does apply it is a **guarantee, not a best-effort optimisation**: a
  self-call in tail position is compiled to a jump, so self-tail-recursion runs in
  constant stack (Scheme-style), and code may rely on it not overflowing.

Quotations and the combinator library land in Phase 4; but the internal loop/back-edge
primitive and the self-tail-call transform that sits on it are **brought forward to
Phase 2**, because the bytecode-VM dogfood (the Phase 2 exit) needs an overflow-free
dispatch loop and pulling quotations forward would be the larger change. Phases 0-1 and
the scalar/aggregate slices need only shallow recursion; the VM is the first golden that
needs real iteration.

**Phase 4 Slice 4 shipped the quotation literal and the `times` floor**, landing the first
bullet above narrower than the design implies and the second bullet exactly as designed. A
quotation stays a **compile-time-only marker** rather than a first-class runtime value
(D1): it carries the identity of its literal body, rides the checker's `Slot`/`Binding` and
lowering's phantom `Value` verbatim through shuffles and binds (D2), and is consumed only by
fusion — `call` splices a literal's body at the consumption site, type-checking identically
to writing the body inline, because there is no standalone effect to infer for a bare body
(D3). `call` therefore accepts only a statically-known literal; every position that would
need a real runtime value instead (a branch join, an array element, a user or polymorphic
word argument, an operator operand) is a located rejection, not a
panic (D4). This defers the `Type`/`PolyType`/`IrType`/unification/mangling change a real
runtime quotation type implies to the slice that gives escaping quotations a consumer for
it (Phase 4 Slice 6). `times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )` passes the iteration index
and requires the body return the row it received, so effect realization only ever checks an
inner row against itself (D6). It is library source, not a compiler-known word (slice 10b):
a thin wrapper over a self-tail-recursive `times-helper`, whose tail call is what
reaches the internal loop primitive (`begin_loop`/`finalize_loop`), and loops nest at any
depth. Both names are exported: an import path retains only exported words, so a
helper reached only transitively through a splice would be unresolvable. The
quotation-literal fusion this slice owns (splicing a literal's body at its
`call`) never crosses a `:` word boundary (D5); the interprocedural user-word inliner that
lowers the combinator library itself is Slice 5's.

**Phase 4 Slice 6a made a quotation nameable and shipped the inliner.** `Type`/`PolyType`
gain a `Quotation` variant carrying an interned declared effect (`[ 'T -- ]`), with
unification and `apply_subst` following, so a word may declare a quotation parameter and be
checked standalone against it — `IrType` gains no variant and there is still no "statically
known" bit on the type (D6): knownness stays a predicate on the value (`Slot.quot`), which is
what lets slice 7 later admit a genuine runtime closure without unpicking unification or the
monomorphization walk. `call`/`times` accept an *abstract* quotation typed only by a declared
parameter, beside the literal they already accepted; a literal passed to a declared
parameter is checked directionally against it, enforcing a `Copy`-only capture restriction on
what it may read from its defining scope. Combinators are now ordinary Sooth library words
(`lib/combinators.sth`'s `each`/`map`/`fold`), and every call to one is inlined by
term-splicing the callee's AST body against the caller's live stack — the compiler's only
inliner, generalizing slice 4's `call`/`times` fusion across a `:` boundary — transitively (a
combinator forwarding its own quotation parameter to a nested combinator splices through both
frames) and **totally**: with a quotation type but no runtime representation there is no
fallback, so anything un-inlinable, starting with recursion among quotation-taking words, is a
located error rather than a silent real call (D5). `each`/`map`/`fold` are leaf combinators
rather than `map` and `fold` being written over `each`, but that is a cost preference, not
an impossibility: `fold` and `map` over `each` are both expressible (the accumulator rides a
captured one-element array reached by balanced borrows, which D3 accepts). Because inlining
is total, library composition depth is code size at every call site, so building `map` on
`each` would make every `map` call site depth 2 plus an extra array copy and a counter cell,
where a leaf keeps the library flat at depth 1. "When to inline" becomes a real question
only at slice 7, when a runtime representation first makes a genuine choice possible; until
then "always" is the only implementable answer.

**Phase 4 Slice 6b relaxed the self-tail edge of 6a's D5 and lowered it to a loop
back-edge.** `filter` needed no compiler change at all: a combinator body is checked by
term-splicing at the concrete call site, never through `poly_term`, so the polymorphic-`if`
rejection never gates one. `while`'s blocker was 6a's own combinator-cycle rejection, which
fired identically for a *monomorphic* self-recursive combinator, so it was never a
polymorphism question. `check_combinator_cycles` now permits a self-edge iff every
occurrence of the self-name is in tail position (its `all_calls` count equal to its
`tail_position_calls` count); a non-tail self-call or any cycle of length ≥ 2 still returns
`combinator_cycle_error` unchanged, since those need slice 7's runtime quotation values.
`inline_combinator` gains a matching branch: while splicing a self-tail combinator's body, a
tail-position self-call is not re-spliced (which would recurse forever) but is treated as the
loop back-edge, discharging the same two obligations the whole-word self-tail transform
already runs at its self-call site — `check_linear_across_back_edge` and
`check_reference_across_back_edge` over the caller's residual and the self-call's input row
— before terminating that branch; the third obligation, stack-row identity between the
back-edge and the header, falls out of the ordinary `if`-join discipline, since both the
self-call arm and the base arm must present the same declared row. Lowering composes two
already-shipped ingredients rather than adding a third: `times`'s mid-body `begin_loop` open
and the whole-word transform's self-call-driven back-edge (`back_edges.push` + `Jmp`, not an
`Instr::Call` and not a re-splice), including the `stage_aggregates` stable-slot path for a
carried aggregate state, reused verbatim from the slice-3 aggregate-return aliasing fix. The
Copy quotation parameter itself carries no `IrType` and is excluded from the loop-carried
phis, since it is the same literal every iteration and is re-resolved statically at each
splice. `while` inherits the R18 nested-loop limit in both directions — opening a self-tail
combinator loop while a loop is already open, and a `times` reached while splicing one — by
raising and restoring the same counter `times` does (renamed `loop_depth`, since it now
counts two kinds of loop); the limit is not lifted here (6d lifts it for all five combinators
at once).

**Combinators are never frozen.** A combinator has no compile event of its own to freeze
against: it mints no `IrFunc` and no symbol (D2 above), and it is inlined by term-splice,
fresh, at every call site, re-checked and re-lowered against that site's own live env each
time. Slice 2's polymorphic-word precedent — a frozen defining-line resolver, read once
per instantiation at lowering — does not transfer, because a poly body is checked once and
never re-checked, while a combinator body is re-checked and re-lowered at every splice site.

**Conditionals and dispatch.** Boolean branching is the ordinary word `if`, taking a
`bool` and two quotations (`~[ then ] ~[ else ] if`). Structural dispatch on ADTs is
the **generated eliminator word** (`Shape?`), the sole enum eliminator: each variant
gets a variant-tagged quotation arm written immediately before the call
(`~[ ( Circle ) ... ] ~[ ( Rect ) ... ] Shape?`), exhaustiveness-checked, with no
inline `match`. It is an ordinary term, so it composes mid-body and needs no
definition of its own. The rejected Haskell-style machine — literal patterns, guards,
match sugar — never shipped; tag-routed arms keep none of it, needing only variant
names and ordinary quotation bodies. Haskell matches named positional arguments while
Sooth's inputs are anonymous stack values (the same named-vs-position tension that
rules out dependent types), which is why the guard and pattern apparatus stayed out
while the per-variant arm won. Multi-way branching is a **`cond` combinator** (a
library word taking `[ pred ] [ body ]` pairs), not syntax, so nested `if`s aren't the
only option.

**The machine layer and the library layer.** The compiler knows three machine-level
primitives and nothing else about conditionals: `branch`, a two-way jump on a 32-bit
flag taking two inline quotations (`( ..a u32 ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )`,
nonzero is true, and the single builtin exempt from the quotation-operand default-deny);
`tag`, which reads a payload-free enum's discriminant as that flag and is a register
relabel because such a value already *is* its discriminant; and the six comparison
primitives `ueq`/`ult`/`ugt`/`ulte`/`ugte`/`une`, one per comparison shape, each deriving
signed / unsigned / float behaviour from its operand type.

Everything typed is a library word over them, in the `core` package: `core::bool` and
`core::cmp`, which a program reaches by `import:` like any other module. The *type* `bool`
is one of `core::bool`'s declarations too (`type: bool | False | True ;`), reached the same
way -- with one wrinkle a program feels: a type name resolves against its declaring module,
so unlike a word it does not follow `core::prelude`'s re-export, and a file spelling `bool`
in an effect names `core::bool` directly. `if` and `unless` are term-body combinators
(`: if ( ..a bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )`, whose body reads the
condition's discriminant with `tag` and `branch`es on the flag); `eq`/`lt`/`gt`/`lte`/`gte`/`ne`
are `'T: Copy Ord`-polymorphic `inline` words that wrap a comparison primitive and build
their `bool` by branching and naming a variant. That last detail is the point rather than
an implementation accident: there is deliberately no operation turning a machine word
back into an enum, since not every machine word is a valid discriminant, so an invalid
`bool` is unconstructible rather than merely discouraged. `bool` has no special status
in any of it; `branch` never sees one. `cond` is a documented future word, not shipped:
a variadic `[ pred ] [ body ]` word is not fixed-arity.

This shrinks the core the honest way, by making `if` a word rather than
by replacing it with a bigger feature.

**INV-INLINE-COMBINATOR.** A word declaring an `inline` `~[ ... ]` parameter is always
inlined (spliced) at each call site and mints no `IrFunc`; it has no opaque call form. Its
declared output row is discovered by forward checking of the spliced terms, never solved
for by row unification. Both splice sites rest on this — the checker's tail walk reads a
callee's body because there is only ever one, spliced, form of it, and lowering threads the
caller's tail position into the splice because the body really does run in place of the
call. A word declaring an ordinary `[ ... ]` parameter is the other form and none of the
above holds of it: it is a genuine call that mints an `IrFunc` and receives the quotation
as a `(code, env, disposer)` value.
