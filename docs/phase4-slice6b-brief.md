# Phase 4 Slice 6b — `filter`/`while`, and the gaps they actually need (brief)

Slice 6a made a combinator an ordinary Sooth library word: a quotation is nameable in a
signature, a word may take one, and every call inlines by term-splicing at the concrete
call site. It shipped `each`/`map`/`fold` and recorded two "polymorphic-path gaps" as the
work left for 6b: a polymorphic body rejects `if` (`src/check.rs:3664`), and a polymorphic
self-tail word does not get the loop transform (`src/ir.rs:1218`, `self_tail` hardcoded
`false`). ROADMAP.md (lines 380–400, 1258–1271) frames this slice as closing exactly those
two gaps, "because this is where their first real consumers appear": `filter` needing the
`if` fix, `while` needing both.

**Probing against the built compiler falsifies that framing on both counts.** `filter`
needs neither gap and compiles and runs today, unchanged. `while`'s blocker is not a
polymorphic-path gap at all: it is 6a's *own* D5 combinator-cycle rejection, which fires
identically for a *monomorphic* self-recursive combinator. The genuinely new work in this
slice is a third thing nobody scoped: relaxing that rejection for a self-*tail* combinator
edge and lowering it to a loop back-edge. Everything below was run, not read.

## Recon: measured against the built compiler, not read off ROADMAP

All probes live in `/tmp/slice6b-probes/`; the binary is `target/debug/sooth`.

**1. A combinator's body is checked by term-splicing at the *concrete* call site, never
through `poly_term` — so the polymorphic `if` gap does not gate a combinator.** This is the
single load-bearing fact of the slice. A polymorphic *non-combinator* word with an `if` is
rejected:

```
: pick ( 'T 'T bool -- 'T ) if drop else | a _b | a end ;
→ error: error: `if` in the polymorphic body of `pick` (line 2) is not yet supported
```

But the identical `if` inside a *combinator* (a quotation-taking word) compiles, because
6a's inliner splices the body at the call site where `'T` is already `i64`/`f64` and the
fully-built monomorphic `if` machinery (`src/check.rs:6059`: condition-pop, per-arm
unconsumed-linear check, move-join) runs. `filter`, below, has an `if`/`else`/`end` in its
body and builds clean.

**2. `filter` already works, end to end, with no compiler change.** Written as a
combinator and run at three shapes:

```
: filter ( ['T: Copy 'N] [ 'T -- bool ] -- ['T 'N] usize )
  | p | len >i64 | n | | arr |
  0 n [ | i | &arr i >usize &> @ dup p call if
          | v | &!arr over >usize &!> v ! 1 +
        else drop end ] times
  | wf | arr wf >usize ;
```

`[i64 4]` keeping `>4` → `2` kept, `out[0]=8 out[1]=9`; `[i64 6]` → `2`; `[f64 3]` keeping
`>1.0` → `1`. It compacts *in place* (reads `&arr i &> @`, writes kept elements to the
front through `&!arr`, threads the write cursor on the stack), so it needs **no fresh
allocation and no resizable/dynamic collection**. At `[i64 10000]` it runs to completion
under `ulimit -s 512` (a 512 KB stack), so it inlines to a constant-stack `times` loop, not
a per-element call. (A 1M witness is unavailable: `sooth build` of a 1,000,000-length array
type times out in fixed-array codegen, a pre-existing cost unrelated to this slice; 6a
already reduced its own witness to 10k for the same reason.)

**3. `filter`'s settled return shape holds against the current compiler.** ROADMAP.md's
"same-length array plus a count" shape compiles and runs at two lengths:

```
: pass-through ( ['T 'N] -- ['T 'N] usize ) len ;   \ prints 4 then 7
```

Slice 1's synthesized multi-output return bundle is real; `filter` bundles the compacted
array with its kept-count and needs no generic struct and no `Vec`. **The precedent that
`filter` must not need a resizable array holds with no surprise.**

**4. The "self-tail-poly" gap is mischaracterized: a polymorphic body cannot call *any*
polymorphic word, self or other — it is `unknown word` at the checker, long before
lowering.**

```
: spin ( 'T -- 'T ) spin ;         → error: unknown word `spin` in `spin` (line 2)
: id2  ( 'T -- 'T ) id1 ;          → error: unknown word `id1` in `id2` (line 2)
```

`poly_call_term` (`src/check.rs:3720`+) resolves only builtins, comparisons, *monomorphic*
words via `env.get(name)`, and concrete operators; anything else is `unknown_word_error`.
There is no arm for a call to a polymorphic word. A *monomorphic* self-call
(`: spin-i64 ( i64 -- i64 ) spin-i64 ;`) compiles fine, and a monomorphic word calling a
poly word compiles fine — poly words are callable only *from* monomorphic contexts.
Consequence: the `src/ir.rs:1218` branch whose comment says a self-recursive poly word
"lowers correctly as an ordinary recursive call" is **currently unreachable** — no poly
word can self-call to reach it. The gap the roadmap names in `ir.rs` cannot be exercised
today, and neither `filter` nor `while` routes through it.

**5. `while` must be a combinator, and a self-recursive combinator is rejected by 6a's D5 —
for monomorphic and polymorphic alike.** `while` is unbounded, so `times` cannot express it
(confirmed: `times` takes a static count), and it needs its loop body as a quotation
parameter, which makes it a combinator. Both the polymorphic and the monomorphic forms hit
the same wall:

```
: while ( 'a [ 'a -- 'a bool ] -- 'a ) | p | p call if p while else end ;
→ error: error: a quotation-taking word cannot be recursive (the inliner would splice it forever): `while` -> `while` (line 2, col 3)

: while-i64 ( i64 [ i64 -- i64 bool ] -- i64 ) | p | p call if p while-i64 else end ;
→ error: error: a quotation-taking word cannot be recursive ... `while-i64` -> `while-i64`
```

That message is `combinator_cycle_error` (`src/check.rs:5104`), from `check_combinator_cycles`
(`src/check.rs:5036`), 6a's D5. It builds the edge set from `all_calls` (every position, not
just tail) and treats a self-edge as the error. **This is `while`'s only blocker, and it is
independent of polymorphism.** Neither of the roadmap's two named gaps is involved.

**6. The loop shape `while` needs already runs; the gap is routing a self-tail combinator
call to it.** A monomorphic *non-combinator* self-tail word with its condition inlined runs
in constant stack:

```
: countup ( i64 -- i64 ) dup 1000000 < if 1 + countup else end ;
→ prints 1000000, exit 0 under `ulimit -s 1024`
```

So `src/ir.rs`'s self-tail loop transform is present and correct. The missing piece is
purely: when the inliner reaches a self-*tail* call inside a combinator, emit a loop
back-edge instead of splicing forever. The quotation is statically the same literal on
every iteration (a quotation is `Copy`, D3), so **the loop needs no runtime quotation
value** — this is a loop, not slice-7 territory.

**7. A borrow-predicate for `filter` is impossible: a scalar cannot be borrowed.**

```
5 | x | &x ...   → error: cannot borrow the scalar local `x` of type `i64`
                    a scalar has no address; borrow a field or an aggregate instead
```

So option (a) from the charter — a `[ &'T -- bool ]` predicate that inspects the element
without consuming it — cannot be generic over a scalar `'T`. `filter` must handle the
element by value. Two by-value predicate shapes both compile and run: `[ 'T -- bool ]`
(dup the element, one copy to the predicate, one kept — recon 2), and `[ 'T -- 'T bool ]`
(the predicate returns the element with its verdict, no dup).

**8. `filter`'s `Copy` question is moot today: no array of a non-`Copy` element can be
built.** `fill` is the only array constructor and rejects a linear element outright:

```
type: Box v i64 ;  : drop ( Box -- ) | b | b Box>v . ;
0 Box 3 fill  → error: linear array elements are not supported yet ... `Box` is linear and
                 has no `Copy` instance      (src/check.rs:6620; the array-type twin is 2017)
```

Every array that can exist has a `Copy` element, so `filter`'s `dup` (or, for the
`[ 'T -- 'T bool ]` shape, its in-place stale-tail compaction) is always sound, and the
compaction double-free that would threaten a linear element is *unreachable*. Note this
also falsifies the roadmap's stated rationale (ROADMAP.md ~966: "Slice 6 restricts `@` to a
`Copy` referent, so `each`'s element variable needs the bound"): I moved a non-`Copy`
element out through `&> @` and it built — `@` does not restrict to `Copy` on the current
compiler. The point is moot only because the *array* cannot be constructed, not because
`@` forbids it. (`each`/`map`/`fold` ship with no `Copy` bound and are equally protected by
this same wall, not by their signatures.)

**9. A `times` write-cursor must thread on the stack, not be rebound.** The first `filter`
I wrote bumped the cursor with `w 1 + | w2 |` inside the loop body; it type-checks and runs
but the cursor never advances — a rebind is a fresh per-iteration local, and `times`
forbids carrying move-state across the back-edge. The working `filter` threads the cursor
*below* the index exactly as `fold` threads its accumulator (`0 n [ | i | … ] times`, the
body's net effect `( w i -- w' )`). The spec's `filter` must use this shape; it is the same
constraint `fold` already lives under, worth stating so the implementer does not re-derive
it the hard way.

**10. A pre-existing 6a inliner limitation, shared by `map`, will surface while writing the
dogfood.** Binding a linear array to a local and *then* passing it to an in-place-mutating
combinator is rejected:

```
0 4 fill | a | ... a [ 4 > ] filter
→ error: cannot borrow `arr__inl0` mutably in `main`: it is aliased by `a`
         use `dup` for an independent copy
```

The same failure reproduces with 6a's shipped `map` (`… | a | a [ 2 * ] c::map …`), and
disappears when the array is passed inline / from a producer word (`mk4 [ 4 > ] filter`).
It is not new to `filter` and not a blocker — but the dogfood must pass arrays inline, and
the spec should say so rather than let a reviewer trip on it. (Whether to fix the
underlying alias-after-move tracking is 6a's business, not this slice's.)

**11. Two "not yet supported" diagnostics sit next to these gaps and stay accurate.**
`src/check.rs:3672` (`if` in a polymorphic body) and `:3690` (a quotation in a polymorphic
body) both remain *true* after this slice, because this slice does **not** lift the
polymorphic-body `if` (recon 1: combinators route around it). They should be left as-is —
unlike 6a's recon 9, there is no stale milestone name to correct here. The REPL rejections
(`src/repl.rs:1619`, `:1675`) are 6c's to lift.

## Decided (locked, one at a time)

**D1. `filter` ships as a `Copy`-element combinator, and needs no compiler change.**
Forced by recon 1–3, not chosen: a combinator's `if` is checked at the concrete splice
site, so the polymorphic-`if` gap never gates it. `filter` goes in as a library word whose
only "support" is 6a's inliner, already shipped. The slice's compiler work is entirely
`while`'s.

**D2. `filter`'s predicate is `[ 'T -- bool ]` and its element bound is `'T: Copy`,
declared explicitly.** The `dup`-predicate is chosen over the `[ 'T -- 'T bool ]`
return-predicate because it reads as what a filter predicate *is* (inspect, don't reshape)
and keeps the element off the predicate's output row. The `'T: Copy` bound is currently
inert — recon 8 shows no non-`Copy`-element array can be built, so the bound can never be
violated at a splice site today — but it is declared anyway: it is the honest constraint
(`dup` requires it, and in-place compaction requires it the day linear-element arrays
land), and it costs nothing now. This matches ROADMAP.md ~966's prediction that "a
constraint appears here," while recording that the *reason* it gave (`@` restricts to
`Copy`) is falsified (recon 8).

**D3. The slice's real deliverable is `while`, via a self-*tail* combinator loop, not via
either roadmap gap.** Locked by recon 4–6. The two gaps the roadmap and 6a's own "Next
action" note named (`src/check.rs:3664` poly-`if`, `src/ir.rs:1218` poly self-tail) are
*not what `filter`/`while` need* and are left untouched (D6). What ships instead: relax D5
for a self-tail combinator edge, and lower it to a loop back-edge.

**D4. `while`'s signature is `( 'a [ 'a -- 'a bool ] -- 'a )`.** The body applies the
quotation to the threaded state, branches on the returned `bool`, and self-tail-recurses on
the `true` arm with the state and the (`Copy`) quotation, returning the state on the `false`
arm:

```
: while ( 'a [ 'a -- 'a bool ] -- 'a ) | p | p call if p while else end ;
```

Justified by recon 6: the monomorphic non-combinator twin of exactly this shape
(`countup`) runs in constant stack, so the shape is sound once the self-tail call is
allowed to become a loop.

**D5. D5-of-6a is relaxed *only* for a self-tail combinator edge; non-tail combinator
recursion stays a hard error.** A self-tail call re-enters with the identical quotation, so
it is a loop and needs no runtime value. A non-tail self-call (or a mutual cycle) would
need the quotation as a runtime argument to a real call, which is slice 7 — those must stay
`combinator_cycle_error`. `check_combinator_cycles` must therefore distinguish a tail
self-edge (allow, hand to the loop transform) from a non-tail one (reject), where today
`all_calls` erases the distinction.

**D6. The polymorphic-body `if` gap and the polymorphic self-call gap are out of scope and
left in place.** Recon 1 and 4: `filter`/`while` are combinators and route around both.
Lifting the monomorphic arm machinery to `PolyType`, or teaching `poly_call_term` to
resolve a poly self-call, is real work with no consumer in this slice, so it would be pure
scope creep. Leave `src/check.rs:3664/3672` and the (unreachable) `src/ir.rs:1218` branch
exactly as they are.

**D7. `filter` and `while` land in the existing `lib/combinators.sth`.** They are the same
kind of leaf combinator as `each`/`map`/`fold` and change together with them; no new file.

**D8. The self-tail loop is a splice-time back-edge, not a specialized `IrFunc`.** Resolved
here rather than left to the spec, because the two candidates touch different code and only
one of them keeps 6a's invariants. `times` is already a mid-body loop opened at an arbitrary
live stack (`src/ir.rs:2630`): it takes the whole stack as loop-carried, saves and restores
the enclosing loop state so loops compose, and builds its own exit block. Every 6a
combinator drives that path today. The obligations a splice-time back-edge needs (stack-row
identity between back-edge and header, move-state identity, borrow-state identity) are the
same three `times` already enforces, and a back-edge from inside an `if` arm is likewise
already lowered and tested (`both_if_arms_tail_produce_two_back_edges`, `src/ir.rs:6314`).
So the mechanism is reuse, not invention.
Specializing instead (mint one monomorphic `IrFunc` per (word, quotation literal) with the
quotation baked in, so the existing whole-function self-tail transform fires) was weighed
and rejected. It buys one real thing, compositional recursion checking against the declared
signature instead of a loop-obligation rule in the splicer, and costs three: it reopens
6a's "inlining is total" and "a quotation-taking word mints no `IrFunc` and no symbol",
both of which survived three review rounds and are cited in DESIGN.md and 6a's merge commit;
it needs a specialization key with dedup, which is the collision hazard 6a's brief already
flagged against slice 2's `instantiation_symbol`; and it would make `while` compose inside a
loop while its siblings `each`/`map`/`fold` still cannot (see D9), which is an inconsistent
library surface. Note that both candidates reach the same `begin_loop`/`finalize_loop` pair,
so "reuses the battle-tested transform" does not discriminate between them.

**D9. `while` inherits the R18 nested-loop limit, and that is accepted, not fixed here.**
A `while` call sited inside another loop is rejected, exactly as a `each`/`map`/`fold` call
there is rejected today: `2 [ | i | mk [ . ] c::each ] times` already fails with "a `times`
cannot be nested in a loop yet". So this is a pre-existing limit `while` joins, not a
regression it introduces, and under D8 the library stays uniform (no combinator composes
inside a loop). Lifting it is the hoist-target split, now recorded as slice 6d, which lifts
it for all five combinators at once. The spec must state this limit plainly and pin it with
a test, so the first person to write an interesting `while` meets a documented rejection
rather than a surprise.

## Open questions the spec must answer

- **The hardest one, and what a reviewer should attack first: the exact loop obligations at
  a splice-time back-edge (D8).** The mechanism is decided; what it must *prove* is not. The
  spec must take each of the three in turn and say how it is discharged and where the
  diagnostic lands: stack-row identity between the back-edge and the header, move-state
  identity (no outer linear local consumed, or it is disposed once per iteration), and
  borrow-state identity (no reference crossing the back-edge). `times` enforces all three
  today by walking a body the checker can see, and the back-edge here carries the caller's
  live stack exactly as `times` does, so the claim is that they transfer unchanged. That
  claim is the load-bearing one in this slice: if any obligation turns out materially harder
  because the carried row is the caller's stack rather than a synthesized index, the
  specialization option rejected in D8 becomes attractive again and the decision should be
  reopened rather than patched around.
- **Whether the carried row's aggregate staging is exercised at all here.** `begin_loop`'s
  `stage_aggregates` path (one entry-hoisted stable slot plus the staged back-edge blit) is
  what fixed the slice-3 aggregate-return aliasing bug. `while` threads a state `'a` that may
  be an aggregate, so the spec should say whether the self-tail back-edge reuses that path
  verbatim (expected yes, since it is the same `begin_loop` call) and pin a carried-aggregate
  `while` as a test, because this is the known-fragile invariant in this code.
- **Where the tail-vs-non-tail distinction for D5 is computed.** `has_self_tail_call`
  (`src/check.rs:2714`) is AST-level and *would* structurally recognize `while`'s recursion
  (it descends into `if` arms and matches the name), but it is never consulted for a
  combinator (combinators are excluded from the per-word lowering pass) and D5's cycle check
  fires first regardless. So this is **not** purely an IR-lowering change: the checker
  itself must gain a tail-position self-call recognizer for combinators. State whether that
  reuses `tail_position_calls` and how it interacts with `check_combinator_cycles`'
  `all_calls`-based edges (a self-edge that is *only* tail must be allowed; a self-edge with
  any non-tail occurrence must still be rejected).
- **`while`'s empty `false` arm.** The `else end` arm is empty and must fall through leaving
  the state `'a` on the stack; the monomorphic `if`-join must accept an empty arm whose only
  content is the incoming stack (the `countup` witness has exactly this and builds, so this
  is a confirmation to pin as a test, not an unknown).
- **`while`'s constant-stack witness.** 10k under a reduced `ulimit` (the `filter`/`countup`
  precedent), plus an equivalence check against a hand-threaded monomorphic loop twin. The
  1M witness is unavailable (recon 2's fixed-array-codegen timeout); say so rather than
  specify a run that cannot build.
- **The `filter` predicate `Copy` bound at the splice site.** Because a combinator body is
  never checked through `poly_term`, `filter`'s declared `'T: Copy` is enforced only when it
  is spliced at a concrete type — and every such type is `Copy` today (recon 8). The spec
  should state what diagnostic *would* fire if a non-`Copy`-element array ever reached
  `filter` (presumably the ordinary `cannot_copy` on `dup` at the splice), and confirm that
  is acceptable as the enforcement point, since there is no definition-site poly check to
  carry the bound.
- **The dogfood must pass arrays inline (recon 10).** Specify a `filter`/`while` example
  paired with a hand-threaded twin (the `examples/array_totals*.sth` pattern) as the
  golden, and route arrays through producer words so the 6a bind-then-pass alias limitation
  is not tripped. State plainly that this limitation is 6a's, not a 6b regression.
- **REPL chokepoint parity.** `while`/`filter` at the REPL is 6c, but confirm the existing
  quotation-taking-word rejection (`src/repl.rs:1675`) still fires for a self-tail
  combinator (it keys on the declared quotation parameter, so it should) rather than letting
  the new self-tail path leak an unpinned session case, the way slice 1 did.

## Out of scope

The polymorphic-body `if` (`src/check.rs:3664`) and polymorphic self-call resolution
(`poly_call_term`, recon 4) — neither `filter` nor `while` needs them, and the roadmap's
claim that this slice closes them is what recon falsifies; leave both rejections in place.
The `src/ir.rs:1218` poly-instantiation `self_tail` hardcode: unreachable today (recon 4),
and D8 routes `while` through a splice-time back-edge rather than a specialized `IrFunc`, so
nothing in this slice reaches it. Leave it exactly as it is. Non-tail combinator recursion and mutual combinator cycles (they need
runtime quotation values — slice 7). Runtime quotation values / closures, a calling
convention, an `IrType` quotation variant (slice 7). REPL support for `filter`/`while`
(slice 6c). Generic `type:` declarations (Phase 6). A resizable / dynamic array type —
`filter` provably does not need one (recon 2–3); the settled precedent holds. Arrays of
non-`Copy` elements (`fill` rejects them, recon 8) — a separate future capability, not this
slice. Fixing the 6a bind-then-pass alias limitation (recon 10) and the fixed-array-codegen
1M timeout (recon 2) — pre-existing, out of this slice's charter.
Lifting the R18 nested-loop rejection, so that a combinator can be called inside a loop:
that is the hoist-target split, now recorded as ROADMAP slice 6d, and it is deliberately not
a rider on this slice (D9). `while` ships with the same limit `each`/`map`/`fold` already
have, and 6d lifts it for all five at once.
