# Phase 3 Slice 6 — Reference types, places, escape checking (spec)

Design input: [the brief](./phase3-slice6-brief.md). Base: `main` @ `0e2763f`, 731 tests green.

This is a flattened rewrite of a spec that went through five review-driven drafts. It states
only the decisions that stood at the end of that process; the round-by-round back-and-forth
that produced them is gone. Nothing here has been implemented yet.

**What changed since the last draft.** Phase 3 Slice 5 landed in between: `| names |` binding
now works at any point in a body, not just at word entry, and a REPL line has locals too. Two
consequences, both applied below rather than left as stale prose:

- The old "top-of-scope locals only" restriction this spec worked around is gone. The dogfood
  is rewritten to use it: `main` binds its two buffers directly instead of needing a `run`
  helper only to have somewhere to bind them, and `push-byte` names its two projected
  intermediates instead of re-deriving them by reborrowing three times.
- R20 below (superseding `get`/`set`) previously deferred the migration because a bare REPL
  line had no locals and so could never form a place. That is no longer true, but the migration
  still stays out of this slice's delivery phases, and it does not get a slice of its own
  either: it is small and mechanical enough to do as a standalone follow-up commit once this
  slice lands, with no brief or spec of its own.

## Context: what is already true on the base commit

Measured by building and running programs, not by reading code.

- The name-it-repeatedly idiom `examples/stack.sth` uses for functional setters
  (`s s Stack>items s Stack>top x set Stack<items`) is a compile error the moment the aggregate
  is linear (`use after move`). The destructure-and-rebuild tower is the only alternative today,
  and its cost (`Alloc`+`Blit` per rebuilt level) grows with nesting depth — qualitatively, not
  by any specific instruction count (three independent attempts at counting one produced three
  different tables; the criterion this slice needs asserts the qualitative absence directly
  against `push-byte`'s own emitted body, never against a table of counts).
- Aggregates already have addresses (a struct arrives as a pointer, `alloc8` slots back array
  and cell payloads). Scalars are SSA temporaries with no address, so borrowing a bare scalar
  would need giving it a memory home it does not otherwise need.
- `PtrOffset` (src/ir.rs:736), `ElemAddr` (src/ir.rs:742), `FieldLoad` (src/ir.rs:758),
  `FieldStore` (src/ir.rs:763), `Alloc` (src/ir.rs:750), and `Blit` (src/ir.rs:754) already
  exist; `ElemAddr`'s own doc comment already calls its result "an opaque element place." This
  slice needs no new `Instr` variant.
- DESIGN.md already distinguishes a lifetime system (ruled out) from a per-place aliasing rule
  (what this slice adds): DESIGN.md:139 reads "There is no lifetime-tracking borrow checker,"
  and DESIGN.md:213-224 states the distinction directly. No DESIGN.md work remains for this
  slice beyond recording the answer to ROADMAP.md's parked design question (R15).
- **Naming an aggregate local does not copy it — a real, currently-open hole, unaffected by
  Slice 5.** See "Open question: aggregate-local aliasing" below.

## Requirements

### Surface: reference types, places, projection, access

**R1 — Two reference types, `&T` and `&!T`.** Shared and mutable. Neither owns; neither is
linear. `&T` is `Copy`. `&!T` is neither `Copy` nor linear (a third category the exactly-once
machinery does not otherwise have — see R8). Both are constructed only by R2 (a fresh borrow of
a local) or by projecting through an existing reference (R3, R16); they are ordinary types in
type position, including on an effect signature's *input* side (R8 forbids the output side and
every other storage position).

**R2 — A borrow is taken from a local, prefix: `&a`/`&!a`.** A **place**, for this slice, is a
local name — nothing more. Applied to anything else (a computed value, a literal, a word
result, or a projection expression) it is a located compile error. Prefix, not postfix, because
naming a linear local moves it before any following word could run: a postfix `<local> &!` would
need the parser to fold backward into one place term, and nothing in the codebase does that.
Prefix sidesteps it entirely — `&a` and `&!a` each lex as a single token, so the checker resolves
a borrow in one step like any other word. The sigil binds tightly: `& a` (with a space) is two
tokens, not a borrow, and it must always be its own token glued to nothing (`a&!` typed as one
run is the single unknown-word token `a&!`, not `a` then `&!`). A reference-typed *parameter*
needs no sigil — if `b : &!Buf` arrived as an input, the body writes `b`, not `&!b`.

A place is exactly a local name, never a projection path (`a Buf>len &!`, "borrow just this
field," is not supported): nothing in the codebase inspects a preceding term the way that would
require. The same result is reached through R3 instead — borrow the whole local first, then
project through the reference with an ordinary word (`&!a Buf&!>len`).

The rejected alternative, a stack-value borrow (`& ( T -- T &T )`, leaving the value below its
own reference), is recorded because it is close and purely additive later: it cannot use locals
at all, since naming a linear local moves it, and a two-borrow call degenerates to
`& rot & rot swap`.

**R3 — Accessor words project through a reference, one spelling per shape *and* per
mutability.** `fill` and `len` are unchanged. `get` and `set` keep their existing signatures
unchanged through this slice (R19's additive claim depends on it); R20 marks them superseded, a
separate, later, mechanical migration. A parallel family of reference-mode accessor words is
added: each takes an already-reference-typed value and yields a narrower reference, never a
plain value.

| shape | shared word | shared effect | mutable word | mutable effect |
|---|---|---|---|---|
| struct field | `T&>fi` (one per struct/field) | `( &S -- &Ti )` | `T&!>fi` | `( &!S -- &!Ti )` |
| array element | `&>` (generic, like `get`) | `( &[T N] usize -- &T )` | `&!>` | `( &![T N] usize -- &!T )`, same runtime bounds trap as `get` |
| cell payload | `&^` (generic, like `^|>`) | `( &^T -- &T )` | `&!^` | `( &!^T -- &!T )` |

Mutability is explicit in the token itself (no "inherited from the receiver" rule): a reader
gets reference-ness, mutability, and arity from the word alone. Every field's projected type may
itself be linear (`Buf&!>data : &!^[u8 64]`, a mutable reference to a linear field) — this is
not a second Copy gate layered on top of R3; R4's Copy restriction is what governs fetch/store,
not projection itself.

**Projection through a `&!` consumes the parent reference, and separately suspends the place it
was reborrowed from.** Each of `T&>fi`/`T&!>fi`, `&>`/`&!>`, `&^`/`&!^` takes its reference
argument off the stack the way every word takes its arguments off the stack. Consuming the
parent *value* is not enough by itself to make a chain of mutable projections linear, because
naming a `&!` local is a **reborrow** — it manufactures a fresh parent independent of whatever
was derived from the previous one:

```forth
: two-live ( &!Buf -- )
  | b |
  b Buf&!>len        \ reborrow #1 consumed by the projection, derived ref live
  b Buf&!>len        \ reborrow #2, while #1's derived ref is still live
  1 +! 1 +! ;
```

Both reborrows are individually consumed by their own projection, yet the program still
manufactures two simultaneously-live `&!usize` into the same field. The rule is stated over the
**place**, not the reborrow value: **taking a mutable reference derived from a place suspends
that place for as long as any reference derived from it (through any number of projection steps)
is live; naming the place again during that window is a located error**, symmetric with R5's
rule that consuming a place while a borrow of it is live is an error. `two-live` is rejected the
moment the second `b` is named. `push-byte`'s own hand-trace (below) is the accept-case: each of
its reborrows of `b` is fully consumed before the next is taken, so the place is never
suspended at the point of a subsequent reborrow.

The suspend rule is **mutable-only**. `&T` is `Copy`, so a live `&T` parent alongside a
reference derived from it composes freely (`&a dup Buf&>len` is fine) — shared references carry
no exclusivity, so there is nothing for a suspend to protect.

R11's borrow-root list needs no reference-typed case: R2 is never applied to a value that is
already a reference, only to a plain aggregate local.

**Name reservation.** `&`/`&!`-led names are rejected at every declaration site (`type:` name,
`:` word name, local binding, the REPL's own `type:` path), the identical shape the owning-cell
syntax already applies to `^`-led names (`is_reserved_caret_name`/`reserved_caret_name_error`,
src/parser.rs:89-103). Separately, `@`, `!`, and `+!` are exact-name builtins with no existing
shadowing protection (`: drop ( i64 -- ) . ;` compiles clean today) — this slice adds one: a `:`
word declaration using exactly `@`, `!`, or `+!` as its name is a located shadowing error, added
because those three names would otherwise silently change meaning to every future caller the
moment this slice introduces them as builtins.

**Type-position splitting is three cases.** `&`, `!`, `^` are not lexer delimiters but `[` is, so
`&T`/`&!T` in type position needs three parsing cases: a bare `&!Buf` arrives as one `Word`
token and splits within itself; a composed `&!^List` also arrives as one token, splits within
itself down to the remainder `^List`, and hands that remainder to the *existing* caret splitter
(`parse_owning_cell_type_expr`, src/parser.rs:580-611) rather than to `resolve_type` directly;
`&![u8 64]` splits *across* tokens (`Word("&!")` then `LBracket`) because `[` already is a
delimiter, handled by recursing into the ongoing token stream exactly as
`parse_owning_cell_type_expr`'s existing empty-remainder case does. This is not hypothetical:
`&!^List` is exactly the type `walk`'s `Cons` clause binds `next` at (R16).

**R4 — Access through a reference: `@`, `!`, `+!`.**

- `@` fetches, typed for **both** `&T -> T` and `&!T -> T`, consuming the reference either way.
- `!` stores, `( &!T T -- )` only — storing through a shared reference is meaningless.
- `+!` adds in place, `( &!T T -- )`, `T` an integer type, sugar for fetch-add-store. `T` is
  inferred from the receiver, so the same bare-literal coercion carve-out that applies anywhere
  else a type is inferred rather than declared (`usize`/`isize` only, no bare `i64`-to-narrower
  coercion) applies here — `b Buf&!>len 1 +!` accepts the bare literal because `T = usize`.
- **Restricted to `Copy` `T`, and a Copy *aggregate* is a real case, not a rejection.**
  `lower_call`'s `"dup"` arm (src/ir.rs:1721-1740) already allocs a fresh slot and blits the
  bytes for a Copy `Struct`/`Enum`/`Array`, exercised on every `dup` of a Copy aggregate today
  (`1 2 V dup V> . . V> . .` prints `2 1 2 1`, i.e. independent copies). `@` on a Copy aggregate
  `T` lowers to `Alloc`+`Blit` the same way; `!` on a Copy aggregate lowers to `Blit` alone. Both
  instructions already exist and are already used for this shape of copy, so this is a new
  lowering arm over existing instructions, not new IR. The restriction that matters is
  Copy-vs-linear, never scalar-vs-aggregate: `@`/`!`/`+!` stay rejected on a linear `T` either
  way, since fetching one would produce a second owner and storing one would silently leak the
  overwritten value (nothing auto-drops).
- **Criterion: the fetched copy survives the source's later mutation.** A golden that only
  fetches and stores cannot tell an `Alloc`+`Blit` fetch apart from one that returns the field
  address directly — both read and write correctly until the source is mutated after the fetch.
  Fetch a Copy aggregate through `&`, mutate the original through `&!`, print the *fetched copy*,
  and assert it still reads its pre-mutation value.
- **Caution for whoever writes the golden, not a rule:** `@`'s `Alloc` is entry-block-hoisted
  (a pre-existing property of `Alloc`, not new here), so a Copy aggregate fetched inside a
  self-tail-call loop and carried across the back-edge is clobbered by the next iteration's
  fetch. Do not write criterion 4's golden inside a loop.

**R5 — Exclusivity is the entire aliasing rule for values reached by borrowing a place, and it
is symmetric.** At most one live `&!` to a place; no `&` to a place while a `&!` to it is live;
no `&!` to a place while a `&` to it is live. (This last direction is easy to omit by accident —
"no `&` while a `&!` is live" alone has no converse, so `&a &!a` would slip through as two live
references to the same place if only the first half were stated.) Consequences, not further
separate rules: `&T` is `Copy`, `&!T` is not; `dup` on a `&!` is rejected by this same rule
(two live mutable references to one place); naming a `&!` local is a **reborrow**, not a move
(without this a mutable helper would kill its own parameter on first use), itself subject to the
suspend rule above; two live `&!` rooted at *different* places never conflict (per-place, never
a single global counter — `copy-byte`'s two-borrow call, one `&!Buf` and one `&Buf` into
different locals, exercises exactly this). `over` shares `dup`'s pre-existing Copy-gate
rejection and its message, which is worded for linearity/ownership and is misleading for a
reference — a pre-existing wording gap, not new, and no criterion needs it fixed here.

R5 governs borrows taken from places. It does **not** cover two aggregate *values* that alias
one address with no borrow ever taken — see "Open question: aggregate-local aliasing" below,
still unresolved and still gating implementation.

**R6 — The borrow check fires at consumption points, keyed on the place and its outstanding
derivations, not a liveness pass.** When a place is moved, dropped, or (re)borrowed in a way R5
forbids, the check asks whether anything on the virtual stack or in the locals map traces its
provenance back to this place, through any number of projection steps — not whether some slot's
`Value` is literally the reborrow's own value. A projection's result is a *new* `Value`, so a
naive scan for "the reborrow itself" misses a derived reference two steps removed that is
nonetheless still live against the place (the `two-live` example above). Provenance is cheap to
track — every projection already knows its own operand. The predicate is over reference-typed
values only, and `@` terminates provenance (its result is a plain `T`, not traced further); the
exclusivity counter counts outstanding *derivations*, never the reference a local's own content
happens to hold. Reject with a located error naming both the place and the conflicting borrow.
A reference is live from the instruction that creates it until the term that consumes its slot;
a reference-typed *local* is live for the whole word body (R8).

Rejected alternative: last-use (NLL-style) liveness. Materially more machinery, and no
criterion in this slice needs the precision.

**R7 — Path disjointness is not modeled.** Two references derived from the same local conflict
under R5 even when they project into disjoint fields, if both are simultaneously live. The
measured cost is one `swap` in `byte-at`'s two-projection chain, sequencing so the first is
fully consumed before the second is taken. A stated limitation with its own criterion, additive
later. R16's reference-mode clause payload bindings are a narrow, named exemption from this
rule, not a second case of it: a clause binds every field of one variant simultaneously, with no
root local to reborrow from at all, and the fields are statically known to be disjoint (the
checker knows the full field layout of the variant at the point it binds them) — sound by
construction on grounds R7's general case does not have available.

**R8 — Escape is prevented structurally, by six positional rejections over transitive
containment (five over compiled code, one over the REPL's cross-line storage).** A type that
**transitively contains** a reference is a located error in: a struct field declaration, an enum
variant payload declaration, an array element (`fill`'s payload — and, transitively, `set`'s:
`fill`'s own rejection already means no reference-holding array can exist for `set` to write
into), a cell payload (`^`'s payload), the **output** side of a declared effect signature, and a
value surviving to the end of a REPL line that would be carried into the session's inter-line
stack. A reference on the effect **input** side is accepted, narrowly: the carve-out is a type
that **is itself** `&T`/`&!T` at the top level, not one that merely contains one nested inside
an array or cell — so the carve-out stays closed if a future aggregate constructor arrives that
today's two construction-site rejections (`fill`, `^`) don't already cover.

The rejections must be enforced **at the construction site** (`^`'s and `fill`'s own arms
reject a payload/element that transitively contains a reference), not only at declaration
sites — a naive three-position version (struct field, enum payload, effect output) misses that
`^`'s arm interns a cell over any payload type with no filter and `fill`'s arm accepts any
`Copy` element (and `&T` is `Copy`), so both would otherwise let a reference in through
construction with no declaration in sight.

**The REPL's carried stack is a real, live rejection now, not a hardening note for an
unreachable case.** `Session` persists the inter-line stack as raw 8-byte cells plus a per-slot
`Type` (src/repl.rs:227ff.); R12 maps a reference to `IrType::Ptr`, an 8-byte cell, so nothing
about the storage format rejects a reference by itself. Before Slice 5 this was unreachable
because a bare REPL line had no locals at all, so R2's local-only place could never fire inside
one. **That is no longer true**: Slice 5 gave the REPL line the same `Scope`-threaded locals a
word body has, so a line can now form a place and (once this slice's `&`/`&!` exist) take a
borrow — meaning this sixth rejection is something a real REPL session can actually reach, and
needs a real golden proving it, not a note explaining why one couldn't be written.

A reference's own **`drop`** frees nothing — it never owned anything, whether the reference sits
on the stack or in a local. A leftover reference on the **stack** without a `drop` is still a
surplus-value error, the same check that catches a forgotten `int`; a reference-typed **local**
is never surplus-checked, it simply expires silently at the end of the word body (matching that
a parameter is never itself "left over" — `push-byte`'s `b` is never explicitly dropped, and
this is correct).

Combined with place-only creation (R2) and R11 (only an aggregate/cell local can be a borrow
root), a reference cannot outlive its referent, so no lifetime apparatus is needed.

**R9 — Loops: the referent must outlive the iteration, from both sides.** A reference
**parameter** may cross a self-tail-call back-edge, since its referent lives in an ancestor
frame and outlives every iteration — this is what keeps `walk ( &!List -- ) ... walk ;` legal,
and is the reason the feature is worth having (R16). A reference derived by projection from a
parameter inherits the same permission (its provenance traces to the ancestor-frame referent,
not to anything created in the current frame). Two located errors guard the other side: a
reference derived from a **current-scope local** may not cross a back-edge, and a **currently
borrowed local** (borrowed at the point of the back-edge, per R6's live-until-consumed
definition — not "ever borrowed during this iteration") may not itself be loop-carried, because
locals rebind at the header (`header_phis`, src/ir.rs:1491) and either would alias a reused slot.

**R10 — Branch joins: borrow state must agree.** The existing type-unification join already
rejects two arms whose stacks disagree in *shape* (a live reference on one arm, none on the
other). What it does not check is *which place* a live reference's suspension is attributed to:
two arms can each leave a stack of identical shape while each arm's value suspends a *different*
place (one derives from local `x`, the other from `y`), and type unification alone has nothing
to say about that. R10 is the rule that the suspended-place bookkeeping must also agree across
arms — real content the type-only join does not supply, tested on both the disagreement and the
agreement side so an over-broad "any borrow crossing an `if` is an error" cannot pass by
accident.

Rejected alternative: a `MaybeBorrowed` lattice mirroring Slice 1's `MaybeMoved`. The
conservative rule is smaller and no criterion needs the imprecision.

**R11 — Only aggregate or cell locals may be borrowed.** The root of a place must be a local of
struct, enum, array, or cell type; a local of scalar type is a located error ("borrow a field or
an aggregate"). Scalars are SSA temporaries with no address, and giving them one is real work no
criterion needs. A projection whose *result* is scalar (`b Buf&!>len` yielding `&!usize`) is
unaffected — the referent is a field inside an aggregate that already has a slot.

**R12 — No new IR instruction; a reference is always `IrType::Ptr`.** Struct-field projection is
`PtrOffset`, array-element projection is `ElemAddr`, cell projection (`&^`/`&!^`) is a `Load` of
the stored pointer, `@` is `FieldLoad` (Copy scalar) or `Alloc`+`Blit` (Copy aggregate), `!` is
`FieldStore` or `Blit`, `+!` is `FieldLoad`+`Bin(Add)`+`FieldStore`. `Ptr` stays opaque; no
pointer arithmetic is exposed to the surface language.

`&!Buf` must **not** map to `IrType::Struct(id)`: QBE's C-ABI classification passes a
`:Buf`-spelled parameter **by value**, so a callee storing into it would silently mutate a
caller-side temporary — measured directly, this is not a hypothetical. `&T`/`&!T` map to
`IrType::Ptr` instead, always, including in ABI positions; `IrType::Ptr` already exists
(src/ir.rs:131) for exactly this shape, and both `width` and `qbe_abi_ty` already spell it `l`
with no change needed to either function — only `ir_type_of` (src/ir.rs:154) gains the two new
`Type` arms.

**This is a soundness answer, not only a mechanical one.** `Type` is matched exhaustively at
many sites — `is_copy`, every `is_linear`-shaped predicate, the surplus-value check — and each
of those gains an arm whose *answer* correctness depends on: `is_copy` must return `true` for
`&T`, `false` for `&!T`; every `is_linear`-shaped predicate must return `false` for both.
Getting either wrong silently misclassifies a reference as duplicable-and-droppable, or as
needing linear drop-tracking machinery it must never receive. `Moves::new` and the back-edge
check both need an explicit reference-local exclusion for this reason: `push-byte`'s second and
third reborrows, and R9's accept-case, are otherwise rejected as written.

A reference also needs an interned `RefId` registry, an `(inner: Type, mutable: bool)` pair,
mirroring the existing `Type::Array(ArrayId, &'static str)` and `Type::OwnedCell(OwnedCellId,
&'static str)` registries (`ArrayDecl`/`OwnedCellDecl`, src/ast.rs:152-171, interned via
`intern_owned_cell_type`/the array equivalent) — needed the moment a parameterized reference
type has to render its own name in an error message or a generated accessor name.

**R13 — Mutation through a reference emits no rebuild.** The measurable form: `push-byte`'s
emitted body contains no `alloc` and no `blit`. Its array-element projection (`&!>`) has a
*computed* index, so `bounds_check` (src/ir.rs:2348-2358) emits a `Cmp`, a `Jnz`, a trap block,
and a `Call sooth_oob_trap` on top of the address-arithmetic-plus-store shape; a criterion's
instruction-count ceiling must be set from `push-byte`'s own measured shape including that
guard, not from an idealized reference-only body.

**R14 — No parameter-convention keywords.** `let`, `inout`, `sink`, and `set` are not added. The
reference type is the convention: `&Buf` is what `let Buf` would have meant, `&!Buf` is what
`inout Buf` would have meant, and the unannotated default is `sink` — no existing signature
changes meaning, no existing code migrates. `set` is not added as a keyword for a second,
independent reason: it is already a user-callable array word in `examples/stack.sth`. (Its other
cited alternative — multiple stack outputs as an out-parameter substitute — is not currently
reliable: a word with two declared outputs is a reachable panic today (`: w ( -- i64 i64 ) 1 2
;` called then printed panics at `print: value`, src/ir.rs:1832, because a general word call
only ever pushes one result value regardless of its declared effect); `get`'s two outputs work
only because `get` lowers as a checker/IR special case, not an ordinary multi-output `Call`.
Pre-existing, out of scope here, and does not weaken the argument for cutting the keyword,
which stands on its other leg.)

**R15 — ROADMAP.md's parked design question, answered.** (ROADMAP.md:476-488, verified against
the current file.) `inout` projections **do** subsume a reified take/fill pair (`S/fi` yielding
a residual `∂S/∂fi`, refilled exactly once) for every statically known path — a projection is
the same residual made implicit and lexically bounded, and covers whole-value borrows too. No
residual form is added. Reified residuals remain worth having only where the focus must escape,
which is a later slice's zipper; R8 forbids storing a reference, so the zipper waits for that
slice's RC rather than for a residual type.

**R16 — Reference-mode enum elimination.** When a word's declared top input is a reference to
an enum (`&Enum` or `&!Enum`), the existing clause-style whole-word form applies in **reference
mode**, same syntax, with four differences from value mode:

- The scrutinee is **borrowed and consumed by the dispatch**, not owned and not freed: reading
  the discriminant through the reference is a tag `FieldLoad` (no new IR instruction); the enum
  value itself is never freed or moved, only the reference *value* is consumed — the same way
  any reference argument to any word is consumed by that word — so every clause body starts from
  the same stack shape a value-mode clause would (the scrutinee reference is removed before
  payload binding, exactly as value mode removes the owned scrutinee, `stack_below =
  params[..params.len() - 1]`, src/ir.rs:2686ff.). Leaving it as a surplus value instead would
  trip R8 at every clause exit, which is not what this rule specifies.
- Each clause's payload bindings are **references inheriting the scrutinee's mutability**: a
  `Cons v i64 next ^List` clause under a `&!List` scrutinee binds `v : &!i64` and
  `next : &!^List`, exactly as a struct-field projection under `&!` would (R3).
- **No clause may consume a payload binding.** Moving one out, rather than projecting through it
  or feeding it to `@`/`!`/`+!`, is a located error, the same rule R4 applies to a
  fetched/stored `T`.
- **A single clause's payload bindings are exempt from R7** (see R7 above for why: statically
  disjoint fields of one variant, bound simultaneously with no root local to reborrow from).

`lower_clauses` must thread the scrutinee's `EnumId` from the already-checked frontend `Type`
rather than re-deriving it from the lowered scrutinee's `IrType` — under R12 a `&!List`
scrutinee lowers to `IrType::Ptr`, not `IrType::Enum(id)`, so the current
`_ => unreachable!("checked: a clause word's top input is an enum")` (src/ir.rs:2686ff.) becomes
a reachable panic the first time reference-mode dispatch reaches it, unless the checked type is
threaded down instead. Also: `dispatch_on_tag` short-circuits to a bare `Jmp` with no tag read
when the enum has exactly one variant (src/ir.rs:2437), so a golden asserting the tag `FieldLoad`
needs an enum with at least two (`List`'s `Nil`/`Cons` qualifies).

This resolves the feature's motivating case:

```forth
: walk ( &!List -- )
  | Nil
  | Cons | v next |
      v 1 +!
      next &!^ walk
  ;
```

`v`'s reborrow is fully consumed by `+!` before `next` is named, and `next &!^` derives a
`&!List` whose provenance traces to `walk`'s own parameter, so the recursive call is a legal
back-edge exactly as it would be for a struct.

The mode follows the declared scrutinee type, explicit in the signature (`&!List` vs `List`), so
choosing reference mode is never implicit or type-directed in a way that would need inferring
from context.

### Test discipline

**R17 — Every criterion is a runnable golden**, source in to expected stdout or source in to
expected diagnostic, with one reasoned exception: R13's no-rebuild criterion asserts on the
emitted module, since a runtime golden cannot distinguish "mutated in place" from "rebuilt
correctly," and eliminating the rebuild is the point of the slice. R12's `IrType::Ptr` mapping is
**not** a second exception — it is directly asserted: a mutation-visible-to-caller golden is
exactly what would fail if the mapping regressed to a by-value aggregate. Both structural
criteria (the no-rebuild body shape, the mapping) are unit tests over `backend::qbe::emit`'s
output, asserted against a single named function body (`func_body`, mirroring
`emitted_alloc_shim_has_null_trap`, src/backend/qbe.rs:2094-2103), never a whole-module IL
string match, and pinned to the *mangled* symbol (`qbe_name` rewrites `-` to `_`,
src/backend/qbe.rs:186) — `func_body(&il, "export function $push_byte(")`, never the literal
`"push-byte"`, which cannot match. New lexer/parser/check/ir code carries its own unit tests
beside it in addition to the goldens below, per CLAUDE.md's convention; this includes a parser
unit test for the type-position splitter's `^`-led-remainder case, since nothing in the dogfood
exercises it via a hand-written signature (only R16's own inference reaches it).

Diagnostic checks that assert several distinct rejections against one compiled source only ever
exercise the first, because checking fails fast — the criterion table below gives each rejection
its own test and its own small program rather than bundling.

**R18 — Every diagnostic criterion asserts the specific error**, not merely that compilation
failed. Turning silent failure into a sharp error is the point, so the error text and its
location are part of the spec.

**R19 — The reference feature itself is purely additive**, and changes no existing signature's
meaning. Demonstrated, not asserted: `git diff --name-status 0e2763f -- examples/ tests/phase0.rs
tests/phase1.rs` must show only additions (`A`), never a modification (`M`), of a pre-existing
file. This slice's own new file (the dogfood, its own test file) is fine; an edited one is the
regression this exists to catch.

### Superseded vocabulary

**R20 — `get` and `set` are superseded by `&!> @` (or `&> @` for a read-only borrow) and
`&!> !`; migration and removal are a standalone follow-up, not part of this slice or any
numbered slice.** Not renamed, not changed here (R3).
`get ( [T N] usize -- [T N] T )` is non-consuming and two-output because Slice 1 gave it no
other way to leave the array live; every read-only call site pays for it with an immediate
`swap drop` (`examples/vm.sth:62,99`). `&> @` reads the same value with no re-pushed array to
discard. `set ( [T N] usize T -- [T N] )` writes by rebuilding the whole array — the same
rebuild cost R13 exists to eliminate for structs, just for arrays; `&!> !` mutates one element in
place. `fill` and `len` have no reference-mode replacement (neither reads nor writes a single
element) and are not part of this supersession.

The vocabulary only genuinely shrinks because R4 lifted `@`/`!`'s restriction to cover a Copy
*aggregate*, not just a Copy scalar: `examples/vm.sth`'s `prog`/`build` arrays hold `Op`, an
all-scalar-payload enum and therefore a Copy aggregate, and without that lift `&>`/`&!>` composed
with `@`/`!` would be strictly less capable than `get`/`set` for any array this codebase
actually has. `get`/`&!> @` differ in one way worth stating outright rather than leaving a future
reader to spot: `get` on an aggregate element aliases the array's storage (the pushed pointer IS
the element), while `&!> @` copies it out. Both are correct for a Copy value; they are not the
same operation.

**The migration's old blocker is gone, but it stays out of this slice, and it does not get a
slice of its own either.** The previous reason to defer was that a bare REPL line had no locals
(`Ctx::Line` carried none, so R2's place could never form there), which made the entire
replacement vocabulary unreachable from a REPL line and would have deleted
`tests/phase1.rs`'s `stack_dogfood_runs_in_repl`/`vm_dogfood_runs_in_repl` goldens' REPL-scope
coverage with no replacement. Slice 5 removed that: a REPL line has locals now, the same as a
word body. Doing the migration inside this slice would still be scope creep for the reasons
above (a fourth delivery phase, two pre-existing example files and two pre-existing REPL
goldens edited, in direct tension with R19's additive-only regression check), and it is not
new language design either — rewrite four existing files to the new spelling, then delete
`get`/`set` from the checker and IR. Once Slice 6 lands and `&>`/`&!>`/`@`/`!` exist, do it as an
ordinary follow-up commit, no brief, no spec, no phased pipeline: small enough to do in one
pass, reviewed like any other change, not run through the slice machinery this document uses
for actual design work.

## Load-bearing invariants (must survive)

- Backend stays QBE; no LLVM. `Ptr[T]` stays opaque, never assumed to be a `u64`. R12 adds no
  instruction and maps every reference to the existing `IrType::Ptr`; R2 exposes no pointer
  arithmetic; a WASM lowering stays possible.
- The linear spine holds: exactly-once, no auto-drop, forgetting is a compile error. References
  do not weaken it, because they never own — R4's Copy restriction on `@`/`!`/`+!` is what stops
  a borrow from manufacturing a second owner or leaking an overwritten one, and R8 stops a
  reference outliving its referent. `&!T`'s own disposal (neither `Copy` nor linear) is stated
  explicitly in R8 rather than left to fall through the existing two categories silently.
- `core` stays `no_std`. No in-process JIT, no comptime interpreter.
- **This is the third and fourth ad-hoc payload-interned constructor after arrays and cells**
  (docs/phase3-slice2-spec.md:9's tripwire: "a third is the signal to switch to Phase 4 generics
  instead"). `&T`/`&!T` will need the identical `(inner, mutable)`-registry treatment `Array`
  and `OwnedCell` already have (R12). The sequencing argument, stated plainly: references are
  needed now, in Phase 3; generics are Phase 4. Phase 4's planned ad-hoc dispatch (ROADMAP.md:
  523-530 — static overloading over statically-known input types, plus open multimethods) is
  expected to eventually subsume both the reference type constructors and R3's explicit
  accessor spellings, once a word can be overloaded on whether its receiver is `T`, `&T`, or
  `&!T` rather than needing a distinct name per case. **Revisit trigger**: when Phase 4's
  dispatch work lands, re-examine whether `&`/`&!` and the accessor family should collapse into
  overloads of `S>fi`/`get`/`^|>`.

## Delivery phases

1. **Reference types, places, projection, access, and every escape/root rejection needed for
   this phase's lowering to be total.** `&T`/`&!T` in the type system including the `RefId`
   registry and the `is_copy`/linearity answers; `&`/`&!` as prefix borrow operators with the
   `^`-style name reservation and the `@`/`!`/`+!` shadowing rejection; the three-case
   type-position splitter; the accessor family, one spelling per shape per mutability,
   projecting through a reference and suspending its root place for the mutable forms; `@`/`!`/
   `+!` with the Copy restriction covering a Copy aggregate as well as a Copy scalar; R11's
   scalar-local rejection; R8's six transitive-containment rejections paired with the
   narrowed input-side accept-case; R8's reference-`drop`-is-a-no-op rule; R12's lowering.
   Checking here is types plus these specific soundness rules, not yet the borrow-conflict
   machinery (R5-R7, R9-R10) — state explicitly in the phase-1 commit message that at this
   commit a program using two conflicting borrows, or a borrow crossing a back-edge unsafely, is
   *accepted*. Deliberate and temporary.
   Exit: criteria 1 through 6, 13, and 15.
2. **The borrow rules and their diagnostics.** R5 exclusivity in both symmetric directions, R6's
   consumption-point scan keyed on outstanding derivations rather than literal `Value` identity,
   R7's disjointness rejection and its sequenced-workaround accept-case. Every rejection lands
   with its located error and its own diagnostic golden.
   Exit: criteria 7 through 9.
3. **Loops, joins, reference-mode enum elimination, the full dogfood, and the documentation
   correction.** R9's back-edge rules from both sides; R10's join rule with both the disagreement
   and agreement accept-case; R16's reference-mode clause elimination end to end, including
   threading the scrutinee's `EnumId` from the checked frontend type; the full dogfood including
   `walk`; recording R15's answer into ROADMAP.md's parked design question (the only remaining
   ROADMAP.md action — the title/DESIGN.md amendment are already in place); R19's additive-work
   regression check.
   Exit: criteria 10, 11, 12, 14, and 16.

There is no phase 4. See R20 for what that leaves out and why.

## Criterion → test map

Goldens live in a new file, `tests/phase3_refs.rs`, not `tests/phase0.rs`: criterion 16 asserts
`tests/phase0.rs` is never modified from base, so a new golden belongs somewhere the
addition-only check does not have to reason about. The two structural criteria (6, 13) belong in
unit tests beside `backend/qbe.rs`, mirroring `emitted_alloc_shim_has_null_trap`.

| # | criterion | test | phase |
|---|---|---|---|
| 1 | `&`/`&!` on a local yield reference-typed values; applied to a literal, an arithmetic result, or a word result they are located errors (each its own test, since checking fails fast); `&`/`&!`-led names are reserved; `@`/`!`/`+!` cannot be shadowed by a `:` word | `borrow_of_place_is_accepted`, `borrow_of_literal_is_error`, `borrow_of_arithmetic_result_is_error`, `borrow_of_word_result_is_error`, `borrow_led_name_is_reserved`, `shadowing_builtin_access_word_is_error` | 1 |
| 2 | borrowing a scalar local is a located error; borrowing a scalar *field* through a projection is accepted; `dup` on a `&T` is accepted, `dup` on a `&!T` is not (R5) | `borrow_of_scalar_local_is_error`, `projection_to_scalar_field_is_accepted`, `dup_of_shared_reference_is_accepted`, `dup_of_mutable_reference_is_error` | 1 |
| 3 | projection reads correctly through all three shapes with the correct spelling per mutability: struct field, array element (incl. the bounds trap), cell payload; storing through the *shared*-spelled projection is a located error | `projection_through_field_element_and_cell_reads_correctly`, `element_projection_out_of_bounds_still_traps`, `store_through_shared_reference_is_error` | 1 |
| 4 | `@`, `!`, `+!` read/write/increment through a reference; `@` on a linear `T` and `!` on a linear `T` are two located errors (each its own test); `@`/`!` on a Copy **aggregate** `T` read/write correctly via `Alloc`+`Blit`/`Blit`, not a panic and not an error; the fetched copy is independent of its referent, proven by mutating the source after the fetch | `access_through_reference_reads_and_writes`, `increment_through_mutable_reference_adds_in_place`, `fetch_of_linear_referent_is_error`, `store_of_linear_referent_is_error`, `fetch_or_store_of_copy_aggregate_reads_and_writes`, `fetch_of_copy_aggregate_survives_source_mutation` | 1 |
| 5 | escape: a reference in a struct field, an enum variant payload, an array element (`fill`), a cell payload (`^`), on an effect's output side, and surviving to the end of a REPL line, are six located errors; a reference on an effect's *input* side is accepted; `drop` of a reference frees nothing, pinned to the alloc trace (no free observed) | `reference_in_struct_field_is_error`, `reference_in_enum_payload_is_error`, `reference_as_array_element_is_error`, `reference_in_cell_payload_is_error`, `reference_returned_from_word_is_error`, `reference_surviving_repl_line_is_error`, `reference_in_effect_input_is_accepted`, `drop_of_reference_frees_nothing` | 1 |
| 6 | **structural**: the emitted body of `push-byte` contains no `alloc` and no `blit` and does contain the address-arithmetic-plus-store shape, under an instruction-count ceiling that budgets for the bounds guard; a rebuild-style control word in the same module still contains `alloc`/`blit` | `mutation_through_reference_emits_no_rebuild`, `rebuild_style_equivalent_still_emits_alloc_and_blit` | 1 |
| 7 | exclusivity, both directions: two live `&!` to *one* place, a `&` taken while a `&!` is live, a `&!` taken while a `&` is live, and a reborrow taken while a reference derived by projection from the previous reborrow is still live, are four located errors; two live `&!` to *different* places is accepted; `&` names twice cleanly (Copy); naming a `&!` local twice, once the prior derivation is fully consumed, is an accepted reborrow | `two_live_mutable_borrows_is_error`, `shared_borrow_while_mutable_live_is_error`, `mutable_borrow_while_shared_live_is_error`, `reborrow_while_projected_reference_still_live_is_error`, `two_live_mutable_borrows_to_different_places_is_accepted`, `shared_reference_is_copy`, `naming_mutable_reference_local_reborrows` | 2 |
| 8 | consuming a place while a borrow of it is live is a located error naming both the place and the borrow, whether the conflicting borrow sits on the virtual stack or in the locals map; disposing a borrowed place is likewise an error; the same place consumed, or disposed, *after* its borrow is gone is accepted | `move_of_place_borrowed_on_stack_is_error`, `move_of_place_borrowed_in_locals_is_error`, `dispose_of_borrowed_place_is_error`, `move_after_borrow_ends_is_accepted` | 2 |
| 9 | two references into disjoint fields of one place, held simultaneously, are rejected (stated limitation); sequencing them (fully consuming the first before taking the second) is accepted | `disjoint_field_borrows_are_conservatively_rejected`, `sequenced_borrows_of_two_fields_are_accepted` | 2 |
| 10 | a reference parameter crosses a self-tail-call back-edge and mutates in constant stack over 1,000,000 nodes, an intermediate node's value read back afterward to confirm the mutation actually landed; a reference to a current-scope local crossing a back-edge, and a currently-borrowed local being loop-carried, are two located errors | `reference_parameter_crosses_back_edge_in_constant_stack`, `reference_to_local_across_back_edge_is_error`, `borrowed_local_carried_across_back_edge_is_error` | 3 |
| 11 | a borrow live on one arm of an `if` and not the other is a located error at the join; a borrow live on both arms, or on neither, joins cleanly | `borrow_on_one_arm_only_is_error`, `borrow_live_on_both_arms_is_accepted` | 3 |
| 12 | reference-mode clause elimination: a word whose declared top input is `&Enum`/`&!Enum` may dispatch clause-style; a clause's payload bindings are references inheriting the scrutinee's mutability and may be simultaneously live (the statically-disjoint exemption from R7); a clause body that consumes a payload binding is a located error | `reference_mode_clause_binds_payload_as_reference`, `reference_mode_clause_consuming_payload_is_error` | 3 (typing groundwork in 1) |
| 13 | **structural**: a mutation a callee makes through a `&!` parameter is visible in the caller (proves `&!T` lowers to `IrType::Ptr`, not a by-value aggregate) | `mutation_through_reference_parameter_is_visible_to_caller` | 1 |
| 14 | the dogfood runs end to end: the buffer program prints the bytes it actually wrote and its length, the two-borrow `copy-byte` call runs, and `walk` over `&!List` mutates every node and the mutation is observed after the call returns | `reference_dogfood_prints_expected_bytes` | 3 |
| 15 | a leftover reference on the *stack* without a `drop` is a surplus-value error; a reference *local* that is never explicitly dropped is accepted (expires silently at scope end) | `unused_reference_is_surplus_value_error`, `reference_local_expires_without_drop` | 1 |
| 16 | no regression: the full existing suite passes, and `git diff --name-status 0e2763f -- examples/ tests/phase0.rs tests/phase1.rs` shows only additions | existing suite, unmodified; `regression_diff_shows_only_additions` | 3 |

Plus one parser unit test beside `src/parser.rs`, not a golden: the type-position splitter's
`^`-led-remainder case (`&!^List`), reachable via R16's inference but not via any hand-written
signature in the dogfood.

## Dogfood

```forth
type: Buf  data ^[u8 64]  len usize ;

: new ( -- Buf )
  0 >u8 64 fill ^ 0 >usize Buf ;

: push-byte ( &!Buf u8 -- )
  | b x |
  b Buf&!>len @ | i |
  b Buf&!>data &!^ | arr |
  arr i &!> x !
  b Buf&!>len 1 +! ;

: byte-at ( &Buf usize -- u8 )
  | b i |
  b Buf&>data &^ i &> @ ;

: copy-byte ( &!Buf &Buf usize -- )
  | dst src i |
  dst src i byte-at push-byte ;

: main ( -- )
  new new
  | a b |
  &!a 72 >u8 push-byte
  &!b 90 >u8 push-byte
  &!a &b 0 copy-byte
  &a 0 byte-at .
  &a 1 byte-at .
  &a Buf&>len @ .
  a drop
  b drop

  10 Nil build
  | l |
  &!l walk
  l pop Popped>
  . drop ;

type: List | Nil | Cons v i64 next ^List ;
type: Popped rest List val i64 ;

: push-front ( List i64 -- List )
  | rest v |
  v rest ^ Cons ;

: build ( i64 List -- List )
  | n acc |
  n 0 = if
    acc
  else
    n 1 - acc n push-front build
  end ;

: pop ( List -- Popped )
  | Nil   Nil 0 Popped
  | Cons  | v next | next ^> v Popped
  ;

: walk ( &!List -- )
  | Nil
  | Cons | v next |
      v 1 +!
      next &!^ walk
  ;
```

`main` prints `72`, `90`, `2`, `2`: the byte `push-byte` wrote into `a` at index 0, the byte
`copy-byte` copied from `b` into `a` at index 1, `a`'s resulting length, and the head value of a
10-node list (`build`'s front value is `1`) after `walk` increments every node's value by one
in place.

### Hand-trace of `push-byte`, `byte-at`, `copy-byte`

`push-byte ( &!Buf u8 -- )`, `b : &!Buf`, `x : u8`:

| term | stack / locals after |
|---|---|
| `b` (reborrow 1) | `[&!Buf]` |
| `Buf&!>len` (consumes the reborrow) | `[&!usize]` |
| `@` | `[usize]` |
| `\| i \|` | `[]`, `i : usize` |
| `b` (reborrow 2 — the first's derived `&!usize` was fully consumed by `@`, so `b`'s place is not suspended) | `[&!Buf]` |
| `Buf&!>data` | `[&!^[u8 64]]` |
| `&!^` | `[&![u8 64]]` |
| `\| arr \|` | `[]`, `arr : &![u8 64]` |
| `arr i &!>` | `[&!u8]` |
| `x` (Copy local, no move) | `[&!u8, u8]` |
| `!` | `[]` |
| `b` (reborrow 3 — the second's derived chain was fully consumed by `&!>` then `!`) | `[&!Buf]` |
| `Buf&!>len` | `[&!usize]` |
| `1` | `[&!usize, i64(1)]` |
| `+!` | `[]` |

Ends `[]`, matching `( &!Buf u8 -- )`. `b` is reborrowed three times; at each subsequent naming,
nothing derived from the previous reborrow is still live, so `b`'s place is never suspended at
the point of a new reborrow, and R7's disjointness rule never has to reason about two live
derivations from `b` at once. No `swap` is needed: naming `i` and `arr` as they are produced
does the sequencing that a `swap` would otherwise have to do.

`byte-at ( &Buf usize -- u8 )`, `b : &Buf`, `i : usize`: `b` → `Buf&>data` (`&^[u8 64]`) → `&^`
(`&[u8 64]`) → `i` → `&>` (`&u8`) → `@` (`u8`). Every projection here is shared, so the suspend
rule never engages (mutable-only).

`copy-byte`: naming `dst src i` pushes `[&!Buf, &Buf, usize]`; `byte-at` consumes the top two,
pushing `[&!Buf, u8]`; `push-byte` consumes both. Ends `[]`.

`walk`'s reference-mode dispatch over `&!List`: the scrutinee reference is consumed by the
dispatch itself. `Nil` has an empty body. `Cons` binds `v : &!i64`, `next : &!^List`
simultaneously (R16's statically-disjoint exemption from R7). `v 1 +!` mutates the node's value
in place; `next &!^` yields a `&!List` whose provenance traces to `walk`'s own parameter, so the
tail call is a legal R9 back-edge. Every node's value is incremented exactly once as the walk
recurses, in constant stack, and `walk` never frees or moves the list — ownership stays with the
caller.

## Open question: aggregate-local aliasing (not resolved this revision)

**Naming an aggregate local does not copy it.** `lower_call` pushes the *same* `Value` — a
pointer to one frame slot — when a local is named ("i64 is Copy; reuse the value id",
src/ir.rs:1717), including for a struct/array/enum local, while `dup` deep-copies via
`Alloc`+`Blit` (src/ir.rs:1721-1740, R4's own justification above). Independently, a
non-consuming aggregate projection (`S|>fi`'s `Peek`, src/ir.rs:2557ff.; `get` on an array
element, src/ir.rs:2033ff.) pushes the interior address with no copy, on the stated
justification that "the owning aggregate is consumed by the getter/destructure/clause" — false
for a non-consuming peek. Either way, **two distinct locals can denote one region of memory**
today. This is pre-existing and currently invisible, because nothing mutates in place; this
slice's `!`/`+!` make it observable for the first time:

```forth
type: V x i64 y i64 ;  type: S a V b i64 ;
: f ( V V -- ) | p q | p V> . . q V> . . ;
: main ( -- )
  1 2 V 3 S
  S|>a swap S|>a swap drop
  f ;
```

verified to print `2 1 2 1`: `p` and `q` are two aliases of one `V`, and mutating through one
after this slice's `!` lands would be observed through the other, with no rule in R5/R6
noticing, since neither `p` nor `q` was ever borrowed from a *place* — they are two plain values
that happen to share one address. R5's claim to be the entire aliasing rule holds only for
values reached by borrowing a place; this hole is about two aggregate *values* sharing an
address with no borrow ever taken.

Three candidate resolutions, none chosen:

1. **Naming an aggregate local materialises a copy.** Closes the hole at the point of naming, at
   the cost of a real, performance-visible `Alloc`+`Blit` every time an aggregate local is
   named — a cost this slice otherwise works to avoid (R13).
2. **R5 extends to track outstanding aggregate copies of a place**, not just borrows of one, so
   `p`/`q` above would be rejected as two live aliases the moment both are named. More machinery
   than R5 as stated, and its interaction with `dup` (which *does* copy) needs working out.
3. **Borrow roots are restricted** so an aliasable local (one that arrived by a non-consuming
   peek of another place, rather than being bound at word entry from the stack) cannot be a
   borrow root at all — narrower than either of the above, and it only closes the hole where a
   reference is later taken, not the aliasing itself.

**This question gates implementation.** Phase 1 cannot ship R4's `!`/`+!` without an answer,
since they are exactly what makes the aliasing observable.

## Explicitly out of scope

`& ( T -- T &T )`, the stack-value borrow form (R2; revisit if `examples/` after this slice and
the RC slice is dominated by build-then-configure pipelines over a single value). Path-disjoint
borrows (R7). Borrowing a scalar local, and therefore the scalar spill it would need (R11). `!`
over a linear value with drop-on-overwrite (`S<fi`'s precedent, docs/phase3-slice1-spec.md:60,
src/ir.rs:2472's `emit_drop` doc comment). Raw or foreign pointers: `^T` is the owning pointer
and `&T`/`&!T` the borrowing one, the only client for a third is FFI at the hosted layer, and any
future foreign pointer must be an opaque handle with no arithmetic. Collapsing `&`/`&!` and the
accessor family into overloads of the value-form words (the revisit trigger above; waits for
Phase 4's ad-hoc dispatch). Reference counting and storable references, including a zipper.
User-definable destructor bodies. Worklist-based branching disposal. The `get`/`set` migration
itself (R20). The aggregate-local aliasing question above (gating, not deferred by choice).

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "reference-types-places-projection-access",
      "difficulty": "hard",
      "summary": "Add &T/&!T mapped to IrType::Ptr with a RefId registry and correct is_copy/is_linear answers, name reservation for &-led names and shadowing rejection for @/!/+!, the three-case &T/&!T type-position splitter, prefix &/&! on locals with the place-suspend rule for mutable projections, the T&>fi/T&!>fi/&>/&!>/&^/&!^ accessor family split by mutability, @/!/+! typed for both &T and &!T and covering a Copy aggregate via Alloc+Blit as well as a Copy scalar, R11's scalar-local rejection, R8's six escape rejections (five over compiled code plus the REPL carried-stack case, now genuinely reachable since Slice 5 gave REPL lines locals) plus drop-as-no-op, and the surplus-value rule for a leftover reference on the stack.",
      "changes": [
        "src/lexer.rs, src/parser.rs: `&` and `&!` as prefix borrow operators on a local; `&T`/`&!T` in type position with its three splitting cases (bare, `^`-composed, `[`-delimited); `T&>fi`, `T&!>fi`, `&>`, `&!>`, `&^`, `&!^`, `@`, `!`, `+!` as words; `&`-led name reservation mirroring `is_reserved_caret_name`/`reserved_caret_name_error`; a shadowing rejection for the exact names `@`, `!`, `+!`; a unit test for the type-position splitter's `^`-led-remainder case",
        "src/check.rs: reference types in the type lattice with an interned RefId (inner, mutable) registry; is_copy true for &T/false for &!T and every is_linear-shaped predicate false for both, including Moves::new's and the back-edge check's explicit reference-local exclusion; R2's local-only place; the accessor family split by mutability, each consuming its reference argument and suspending its root place for the mutable forms; R4's Copy restriction on @/!/+!, @ typed for both &T and &!T, now covering a Copy aggregate as well as a Copy scalar; R11's scalar-local rejection; R8's six transitive-containment rejections (struct field, enum payload, fill's array element, ^'s cell payload, effect output, REPL carried-stack survival) plus the effect-input accept-case narrowed to a top-level reference type; the surplus-value rule treating a leftover &!T on the stack like any non-Copy value while a reference local expires silently",
        "src/ir.rs: ir_type_of gains &T/&!T -> IrType::Ptr; lower T&>fi/T&!>fi to PtrOffset, &>/&!> to ElemAddr, &^/&!^ to a Load of the stored pointer, @ to FieldLoad (Copy scalar) or Alloc+Blit (Copy aggregate), ! to FieldStore (Copy scalar) or Blit (Copy aggregate), +! to FieldLoad+Bin(Add)+FieldStore; drop of a reference emits no destructor call",
        "no new Instr variant; Alloc and Blit already exist and are already used this way by dup's own Copy-aggregate arm"
      ],
      "tests": [
        "borrow_of_place_is_accepted",
        "borrow_of_literal_is_error",
        "borrow_of_arithmetic_result_is_error",
        "borrow_of_word_result_is_error",
        "borrow_led_name_is_reserved",
        "shadowing_builtin_access_word_is_error",
        "borrow_of_scalar_local_is_error",
        "projection_to_scalar_field_is_accepted",
        "dup_of_shared_reference_is_accepted",
        "dup_of_mutable_reference_is_error",
        "projection_through_field_element_and_cell_reads_correctly",
        "element_projection_out_of_bounds_still_traps",
        "store_through_shared_reference_is_error",
        "access_through_reference_reads_and_writes",
        "increment_through_mutable_reference_adds_in_place",
        "fetch_of_linear_referent_is_error",
        "store_of_linear_referent_is_error",
        "fetch_or_store_of_copy_aggregate_reads_and_writes",
        "fetch_of_copy_aggregate_survives_source_mutation",
        "reference_in_struct_field_is_error",
        "reference_in_enum_payload_is_error",
        "reference_as_array_element_is_error",
        "reference_in_cell_payload_is_error",
        "reference_returned_from_word_is_error",
        "reference_surviving_repl_line_is_error",
        "reference_in_effect_input_is_accepted",
        "drop_of_reference_frees_nothing",
        "mutation_through_reference_emits_no_rebuild",
        "rebuild_style_equivalent_still_emits_alloc_and_blit",
        "mutation_through_reference_parameter_is_visible_to_caller",
        "unused_reference_is_surplus_value_error",
        "reference_local_expires_without_drop"
      ],
      "exit": "Criteria 1 to 6, 13, and 15. The dogfood's push-byte and byte-at compile and run correctly, push-byte's emitted body contains no alloc and no blit while a rebuild-style control word in the same module still does, and a callee's mutation through a &! parameter is visible to the caller. Commit message records that R5/R6/R7/R9/R10 do not exist yet at this commit."
    },
    {
      "phase": 2,
      "focus": "borrow-rules-and-diagnostics",
      "difficulty": "hard",
      "summary": "Exclusivity as the single, symmetric per-place aliasing rule, the consumption-point scan keyed on a place's outstanding derivations rather than literal Value identity, and the disjointness rejection with its sequenced-workaround accept-case.",
      "changes": [
        "src/check.rs: R5 exclusivity, both directions (at most one live &! per place; no & alongside a live &!; no &! alongside a live &; per-place not global), with &-is-Copy and &!-is-not derived from it rather than stated separately",
        "src/check.rs: R6 consumption-point scan over the virtual stack and the locals map, keyed on a place's outstanding derivations (provenance traced through projection, not literal Value equality; reference-typed values only; @ terminates provenance), firing on move, dispose, and conflicting-borrow (including a reborrow taken while a projection derived from the previous reborrow is still live), no liveness pass",
        "src/check.rs: R7 disjointness rejection as a stated limitation with its own diagnostic"
      ],
      "tests": [
        "two_live_mutable_borrows_is_error",
        "shared_borrow_while_mutable_live_is_error",
        "mutable_borrow_while_shared_live_is_error",
        "reborrow_while_projected_reference_still_live_is_error",
        "two_live_mutable_borrows_to_different_places_is_accepted",
        "shared_reference_is_copy",
        "naming_mutable_reference_local_reborrows",
        "move_of_place_borrowed_on_stack_is_error",
        "move_of_place_borrowed_in_locals_is_error",
        "dispose_of_borrowed_place_is_error",
        "move_after_borrow_ends_is_accepted",
        "disjoint_field_borrows_are_conservatively_rejected",
        "sequenced_borrows_of_two_fields_are_accepted"
      ],
      "exit": "Criteria 7 to 9. Every borrow rule produces its specific located error in both R5 directions plus the reborrow-suspend case, and the accept-cases (different-places, reborrow after full consumption, Copy shared reference, move/dispose after borrow ends, sequenced disjoint fields) are all accepted."
    },
    {
      "phase": 3,
      "focus": "loops-joins-reference-mode-enums-dogfood-and-docs",
      "difficulty": "standard",
      "summary": "Back-edge rules from both sides, the branch-join rule with its accept-case, reference-mode clause elimination over an enum, the full dogfood including walk, and the additive-work regression check.",
      "changes": [
        "src/check.rs: R9 back-edge rules (a reference parameter, or a reference derived from one by projection, may cross; a reference to a current-scope local may not; a currently-borrowed local may not be loop-carried)",
        "src/check.rs: R10 borrow state must agree at a branch join, both the disagreement rejection and the agreement accept-case",
        "src/check.rs: R16 reference-mode clause elimination when a word's top input is &Enum/&!Enum: consume the scrutinee reference at dispatch, bind clause payloads as references inheriting mutability and exempt from R7's disjointness rule, reject a clause body that consumes a payload binding",
        "src/ir.rs: lower_clauses threads the scrutinee's EnumId from the checked frontend Type rather than re-deriving it from the lowered scrutinee's IrType, closing the reachable unreachable! a &!Enum scrutinee would otherwise hit",
        "examples/refs.sth (new file): the dogfood (push-byte/byte-at/copy-byte/main, plus a self-contained List/walk); tests/phase3_refs.rs (new file): the golden that runs it and every other criterion test for this slice, kept out of tests/phase0.rs so criterion 16's addition-only check has nothing pre-existing to modify",
        "tests/: a regression check asserting `git diff --name-status 0e2763f -- examples/ tests/phase0.rs tests/phase1.rs` contains only additions",
        "ROADMAP.md: record the slice as done and write R15's answer into the parked design question at 476-488"
      ],
      "tests": [
        "reference_parameter_crosses_back_edge_in_constant_stack",
        "reference_to_local_across_back_edge_is_error",
        "borrowed_local_carried_across_back_edge_is_error",
        "borrow_on_one_arm_only_is_error",
        "borrow_live_on_both_arms_is_accepted",
        "reference_mode_clause_binds_payload_as_reference",
        "reference_mode_clause_consuming_payload_is_error",
        "reference_dogfood_prints_expected_bytes",
        "regression_diff_shows_only_additions"
      ],
      "exit": "Criteria 10 to 12, 14, and 16. The dogfood runs end to end, a reference parameter walks a long list in constant stack while mutating through the reference, and the full existing suite passes with the regression diff check confirming no modification to any pre-existing example or test file. This is the slice's exit: there is no phase 4."
    }
  ]
}
```
