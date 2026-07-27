# Phase 3 Slice 6 — Reference types, places, escape checking (brief)

Slice 1 made every linear value move on use, and Slice 2 put aggregates behind `^`. Together
those close off in-place mutation: a struct containing a `^` is linear, so the
name-it-repeatedly idiom that `examples/stack.sth` relies on (`s s Stack>items s Stack>top x
set Stack<items`) becomes a use-after-move, and the only remaining way to change one field is
to destructure the whole aggregate and rebuild it at every level. This slice adds non-owning
references so a word can read or mutate a value it does not own.

**At the base commit**, ROADMAP.md:438 named this slice "second-class references + parameter
conventions (`let`/`inout`/`sink`/`set`) + escape checking" (N-4, round 3 audit: stated in the
past tense now, since the spec's Amendment A already corrected the title in place, and this
slice has since renumbered from Slice 5 to Slice 6 with a further ROADMAP.md edit — see the
spec for the current text and line numbers). The conventions half does not survive contact with
the language (D2 below); the slice is reference types + places + escape checking.

Prerequisite state: Slice 4 merged (`a66c47a`), 700 tests green.

## Recon: what already works today (measured, not assumed)

**1. The reuse idiom is a hard error on a linear aggregate.**
`type: Buf data ^i64 len i64 ;` with `: bump ( Buf -- Buf ) | b | b b Buf>len 1 + Buf<len ;`,
the exact shape of `stack.sth`'s `push`:

```
error: use after move in `bump` (line 8)
  local `b` of type `Buf` was moved at line 8, col 3; `Buf` is linear, so it is used exactly once
```

So the functional-setter idiom is Copy-only in practice. It is not a style choice that linear
code avoids it; the language rejects it.

**2. The destructure-and-rebuild tower is the only alternative, and it costs a rebuild per
level.** Three probes incrementing one `i64` field at nesting depth 1, 2, 3 (each level a
struct holding the next plus a Copy field), counting the emitted QBE body of `bump`:

| depth | instrs | `alloc` | `blit` |
|---|---|---|---|
| 1 | 13 | 1 | 0 |
| 2 | 21 | 2 | 1 |
| 3 | 29 | 3 | 2 |

Exactly +8 instructions, +1 alloc, +1 blit per level, because each level reconstructs its
whole aggregate into a fresh slot and blits the child in. The target for the same operation
through a reference is one `add` per level plus a load, an add, and a store, with no alloc and
no blit. Source cost tracks it: depth 3's body is `L3> swap L2> swap L1> 1 + L1 swap L2 swap
L3 ;`, where five of the eleven words are plumbing.

**3. Aggregates have addresses; scalars do not.** In the depth-2 emission, `%v0` arrives as
`:Outer %v0` (a pointer) and rebuilt aggregates are `alloc8` slots, while the arithmetic is
`%v8 =l copy 1` / `%v9 =l add %v7, %v8`, pure SSA temporaries. QBE has no address-of for a
temporary, so borrowing a scalar means creating a home for it that does not exist today.

**4. Locals are top-of-scope only.** `a 1 + | b |` mid-body is `parse error: unexpected token
Pipe`. This matters because a projection is the natural thing to want to name mid-body.

## Decided (locked, one at a time)

**D1. A borrow is taken from a place, not from a stack value.** A place is a local, or a
projection path from one. Postfix `&` (shared) and `&!` (mutable), matching `^`'s existing
postfix form. The alternative considered and rejected was `& ( T -- T &T )`, a plain word
leaving the value below its own reference. It is genuinely tighter for a single borrow
(`new & 72 push-byte` against `new | a | a &! 72 push-byte`) and it is trackable, since the
virtual stack is a compile-time `Vec<Value>` and shuffles are permutations that preserve the
`Value` id. It loses on two counts: it cannot use locals at all (naming a linear local moves
it, so the value must stay stack-threaded), and the two-borrow call in the dogfood degenerates
to `& rot & rot swap` where nothing says which reference belongs to which buffer. It is purely
additive later. Revisit if `examples/` after Slices 5 and 7 is dominated by
build-then-configure pipelines over a single value.

**D2. No parameter-convention keywords. The reference type is the convention.** Hylo needs
`let`/`inout`/`sink`/`set` because it has named parameters; Sooth's interface is a stack
effect, which is a list of types, and `&Buf`/`&!Buf` already say what `let`/`inout` would.
`sink` is the unannotated default, so **no existing signature changes meaning**. `set` is cut
twice over: stack returns are a better out-parameter than a mutable hole, and `set` is already
a user-callable array word in `stack.sth`.

**D3. Exclusivity is the whole aliasing rule.** At most one live `&!` per place, and no `&` to
a place while an `&!` to it is live. Everything else follows rather than being stated
separately: `&` is `Copy` because shared references carry no exclusivity constraint, `&!` is
not `Copy` because two live ones violate the rule, and `dup` on a `&!` is rejected by the rule
itself. Naming a `&!` local is a **reborrow**, not a move, or a mutable helper would kill its
own parameter on first use.

**D4. Escape safety is structural, not analytic.** A reference is forbidden in exactly two
positions: stored in a struct or enum field, and on the output side of an effect. Combined
with place-only creation (D1), a reference cannot outlive its referent, so no lifetime
apparatus is needed, which is what DESIGN.md:210 was always claiming.

**D5. No new IR instruction.** `PtrOffset`, `ElemAddr`, `FieldLoad`, `FieldStore` already exist
and `ElemAddr`'s own doc already calls its result "an opaque element place". Mutation through a
reference is an offset and a store, with the `Blit` from recon 2 disappearing. Two lowering
obligations follow: a borrowed local needs a memory home (scalars spill, per recon 3), and a
borrowed local cannot be a loop-header phi.

**D6. A reference parameter may cross a Slice 6 back-edge; a reference to a current-scope local
may not.** The parameter's referent lives in an ancestor frame and outlives every iteration; a
current-scope local rebinds at the header, so a reference to it would alias a reused slot. This
is what keeps `: walk ( &!List -- ) ... walk ;` legal, which is the case that makes the feature
worth having.

**D7. Raw pointers stay out.** `^T` is the owning pointer and `&T`/`&!T` is the borrowing one;
the only client for a third is FFI, which is Phase 6 at the hosted layer. `*` is the
multiplication word, so it is not the spelling. If a foreign pointer ever lands it must be an
opaque handle with no arithmetic: `p 8 +` would force `Ptr` to be an integer and break the
backend-neutral invariant a WASM lowering depends on.

## Open questions the spec must answer

- **When the borrow check fires.** Recommendation, so the spec has something to argue against:
  no liveness pass. Check at consumption points instead. When a place is moved or dropped, scan
  the virtual stack and the locals map for a slot holding a reference to it, and reject if one
  exists. Both are exact compile-time structures (`stack: Vec<Value>`, `locals: HashMap<String,
  Value>`), so this is a scan rather than an analysis, and it accepts the dogfood.
- **Path disjointness.** `b Buf>data ^& b Buf>len @ get x !` holds two `&!` into disjoint
  fields of one struct simultaneously. Either the checker reasons about disjoint paths, or the
  code sequences the borrows at a cost of one `swap`. Recommendation: defer disjointness,
  revisit if real code hits it repeatedly.
- **Projecting through a `^` on a reference.** `^>` frees, so it is wrong here. Needs a
  spelling (`^&` used as a placeholder in the dogfood) and a decision on whether it is a word
  or an automatic step in a projection path.
- **Whether borrowing a Copy scalar local is in the first cut at all**, given it forces the
  spill from recon 3. Aggregates-only is a defensible smaller slice.
- **Mid-body locals.** Recon 4 says a projection cannot be named where you want it. Either
  stack-threading is enough (the dogfood suggests it is, at the cost of a `swap`), or this
  slice has to relax top-of-scope binding. Decide explicitly rather than discovering it during
  implementation.
- **Branch joins.** Borrow state has to agree at a join the way types already unify. Needs a
  lattice, presumably mirroring Slice 1's `Live`/`Moved`/`MaybeMoved`.
- **The question ROADMAP.md parks for this brief** (N-5, round 3 audit: at the base commit this
  was line 443; the spec tracks its current location, which has moved twice since — once for
  Amendment A's title correction, once for this revision's general-locals insertion), recorded
  formally: projections subsume a reified take/fill pair (`S/fi` yielding `∂S/∂fi`) for every
  statically known path, since a projection is the same residual made implicit and lexically
  bounded. Reified residuals remain worth having only where the focus must escape, which is
  Slice 3's zipper, and that wants this slice's own RC follow-on rather than a reference.
- **ROADMAP.md:438's title was wrong** (N-4b, round 3 audit: this is no longer a to-do — D2's
  correction landed via the spec's Amendment A, in the same revision that first raised it).

## Dogfood

An owned heap buffer mutated in place, with a two-borrow call, since one borrow is the easy
case and two is where the design earns or loses:

```
type: Buf  data ^[u8 64]  len usize ;

: push-byte ( &!Buf u8 -- )
  | b x |
  b Buf>len @
  b Buf>data ^& swap
  get x !
  b Buf>len 1 +! ;

: byte-at ( &Buf usize -- u8 )
  | b i |
  b Buf>data ^& i get @ ;

: copy-byte ( &!Buf &Buf usize -- )
  | dst src i |
  dst src i byte-at push-byte ;

: main ( -- )
  new new | a b |
  a &! 72 push-byte
  b &! 90 push-byte
  a &! b & 0 copy-byte
  a & 2 byte-at .
  a dispose
  b dispose ;
```

New vocabulary it exercises: `@` fetch `( &T -- T )`, `!` store `( &!T T -- )`, `+!` add in
place, accessor words projecting through a reference (`Buf>data` on `&!Buf` yields
`&!^[u8 64]`, `get` on `&![T N]` yields `&!T`), and whatever `^&` becomes. Exit criteria
should include the emitted body of `push-byte` containing no `alloc` and no `blit`, which is
the measurable form of the recon-2 table.
