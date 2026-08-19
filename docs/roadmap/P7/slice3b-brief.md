# Phase 7 Slice 3b: quotations in a polymorphic body (brief)

A quotation literal in a **non-inline** polymorphic word body is rejected outright
(`src/check/poly.rs:505-513`). It fires for any quotation: a bare `~[ ]`, an `if`, an
eliminator arm.

The forcing consumer is **enum elimination inside a polymorphic body**, because a Phase 6
eliminator's arms *are* quotation literals (`examples/eliminator.sth`):

```sooth
: area ( Shape -- i64 )
  ~[ ( Rect )   &w @ swap &h @ swap drop * ]
  ~[ ( Circle ) &r @ swap drop dup * 3 * ]
  Shape? ;
```

Move that into a polymorphic signature and it hits this wall and nothing else, verified
against the built compiler at HEAD:

```text
: area_and_keep ( Shape 'T -- 'T ) ~[ ( Rect ) ... ] ~[ ( Circle ) ... ] Shape? . ;
error: a quotation in the polymorphic body of `area_and_keep` (line 8) is not yet supported
```

So **no polymorphic word can eliminate an enum at all**, which rules out the whole
`unwrap_or`/`map_or`/`Result`-combinator family. This slice lifts that.

## Recon (worker-verified against the built compiler)

Three parallel recon workers, one of which built and ran a stubbed compiler in an
isolated copy. Findings are `file:line`-grounded or output-grounded; the honest
unknowns each worker declared are carried into "Open questions" rather than papered over.

**The rejection's own stated reason is false.** The comment says "`poly_term`'s stack is
`Vec<PolyType>`, not `Vec<Slot>`, so there is nowhere to hang the `quot` marker". There
is: `poly_term` already threads a parallel per-slot side vector
`lits: &mut Vec<Option<i64>>` (`poly.rs:385`), mirroring `Slot::int_val`, maintained in
lock-step at all 27 shuffle/produce/consume sites and guarded by
`debug_assert_eq!(stack.len(), lits.len())` (`poly.rs:413`). A `QuotId` is the same shape
of datum as an `int_val` and can ride the same way.

Two narrower statements *are* true, and they are the real constraints:

- **Identity must not live in the type.** `PolyType::Quotation(ins, outs, ..)`
  (`ast.rs:1179`) carries only the *effect*, so two distinct literal bodies with the same
  effect are the same `PolyType` and it cannot answer "which body to splice". A
  placeholder `PolyType` would additionally leak a fake type into output unification,
  `Subst`, and mangling. D1's refusal of a `PolyType` variant stands.
- **The side state must be positional, not name-keyed.** The existing coarse borrow table
  (`PolyScope.borrows`, `poly.rs:66-71`) is keyed by a local's *name* and does **not**
  transfer: a quotation literal is an anonymous stack value with no stable key surviving a
  `swap`. `lits` is the correct precedent; `borrows` is not. The borrow table is also
  self-described as a lossy compromise (`poly.rs:81-89`, "one unrelated live reference
  keeps *every* recorded borrow alive"), and quotation identity cannot be
  over-approximated: a coarse merge mis-splices rather than merely over-rejecting.

**The comment also cites a sibling that no longer exists.** "Mirrors the
`if`-in-a-polymorphic-body rejection above" refers to deleted code: `TermKind` has no
`If` variant (`ast.rs:1793`), and slice 10c's turning `if` into a library word left only a
tombstone (`src/check/drop_graph.rs:233`, "the role the deleted `TermKind::If` descent
played"). The adjacent array-constructor rejection's back-reference (`poly.rs:517-527`) is
accurate; this one is stale.

**Lowering is free, and this is output-grounded rather than reasoned.** A worker stubbed
the check-side rejection open in an isolated copy and pushed programs through to running
QBE:

- `[ dup . ] call` in a non-inline poly body splices and grounds `'T`: emits
  `call $printf($fmt, %v0)` then `ret %v0`.
- One quotation body span, two instantiations, both correct: the same `.` lowered to an
  i64 print in `sooth_mono_foo__m0__t0_i64` and to a length-and-pointer str print in
  `..._t0_str`.
- The case most likely to break did not: a linear `'T` dropped inside a spliced quotation
  selected the correct per-type destructor (`call $sooth_struct_drop_0(:Res %v0)`),
  despite the `drop` having a single shared span across instantiations.
- `branch` (the primitive `if` compiles to) emitted a correct `jnz`/two-arm/join.

The mechanism: `lower_instantiation` (`ir/driver.rs:760`) grounds only the *signature*
via `concrete_effect` and hands the raw AST body to `lower_word_parts`, the ordinary
concrete path. Nothing threads `Subst` into the body walk; every op derives behaviour from
the runtime value's `IrType` (`emit_drop` at `ir/func_builder/quotation.rs:297` matches on
`self.value_type(v)`, consulting no span-keyed map). So a shared body span across
instantiations is harmless for these ops.

**But there is no abstract branch-join at all.** `poly_walk` (`poly.rs:387`) is a strictly
linear fold over one stack; `poly_term` (`poly.rs:424`) has arms only for
`IntLit`/`FloatLit`/`StrLit`/`Bind`/`Call`/`Quotation`(rejected)/`ArrayCtor`(rejected).
`PolyScope` holds a `Moves` (`poly.rs:56`) whose two-arm reconciler `Moves::join`
(`check/engine.rs:453`) exists and is **never called from `poly.rs`**. All branch-and-join
lives in the concrete `Slot` checker (`check_branch_join`, `terms.rs:1097`, ~290 lines),
reachable from a polymorphic word only by `inline` splicing — which is exactly why every
branch test in the poly suite declares `inline` (`poly.rs:3075-3082`).

**And the whole combinator family is absent from the poly path.** `poly_call_term`'s
`match name` has no `call`, no `branch`, no `if`, and no inline-combinator splice. The
`resolve_combinator_overload`/`inline_combinator` machinery (`poly.rs:1620-1760`) exists
but is wired only for combinator *parameters*, not for a quotation *literal spliced inside
a poly body*. This is the bulk of the slice.

**Mechanical surface for the representation change**: 28 stack-mutation call sites
(~18 logical ops) plus ~26 type-reads, across the 6 functions threading the stack
(`poly.rs:384/421/537/985/1147/1431`). Hand-counted, accurate to about ±1.

## The consumer (probe-verified, and the reason this brief was rewritten)

The first draft justified the slice as "everything interesting is forced `inline`" and
named no word that would stop being `inline`. A probe falsified that framing twice over,
and the real consumer turned out to be elsewhere.

**The "forced inline" premise is partly false.** A non-inline polymorphic word that
branches *via a callee* already compiles today:
`: less ( 'T: Copy Ord 'T -- bool ) > ;` builds clean. `>` is an inline library word whose
body is `u< [ true ] [ false ] branch`, and the poly walk never sees that quotation because
the splice happens later, at concrete lowering. The wall fires only on a quotation written
*directly* in the polymorphic body.

**No existing `inline` library word is freed by this slice.** Surveyed, with the reason
each is inline:

| word(s) | why inline | freed by S3b? |
|---|---|---|
| `= < > <= >= <>` (`core.sth`) | body needs `u<` on an abstract operand; `poly_delegate_op` only feeds an operator the maximal *concrete* stack suffix (`poly.rs:1437-1451`) | no, different wall |
| `if` `unless` (`core.sth`) | row-typed, take quotation *parameters* | no, inherently inline |
| `each` `map` `fold` `filter` `while` `times` | row-typed quotation-parameter combinators | no, inherently inline |
| `sort` `bin_search` (`arrays.sth`) | comparator parameter *and* index a generic-length `['T 'N]` array | no, two other walls (comparator call is P7.S3d, `'N` indexing is P7.S3c) |
| `En` `TxEn` `ClkDiv` `TxRdy` (`uart_mmio.sth`) | monomorphic constant pairs, inline as call-avoidance | no, not polymorphic |

**The consumer is enum elimination, and it is available to test today.** Eliminator arms
are quotation literals, so a polymorphic word eliminating even a *concrete* enum is blocked
(the `area_and_keep` probe above). P6.S3 has shipped, so this consumer needs nothing from
Phase 6's in-flight work.

**Phase 6 widens it and then makes it mandatory, but is not an implementation
dependency.** P6.S3b (in flight) makes *generic* enums eliminable with bare tags, which
extends this slice's reach to `Opt['T]`/`Result['T 'E]` eliminators. P6.S4 then deletes
`WordBody::Clauses` and `parse_clauses` entirely, after which the eliminator word is the
only elimination mechanism that exists, so this wall becomes the sole thing standing
between a polymorphic word and any enum at all. Neither changes the mechanism this slice
builds; they change how much it unlocks and how urgent it is.

A consequence for scope: the clause-style rejection at `poly.rs:196-201`/`:336-341`
("combines a clause-style body with a polymorphic signature") is **not** this slice's
problem and must not be lifted here. Its subject is being deleted by P6.S4.

## Shape of the work

Three check-side layers and no lowering work:

1. **Representation.** `PolySlot { pt: PolyType, quot: Option<QuotRef> }` replacing the
   bare `Vec<PolyType>`. Exactly one new field beyond the type: `alias`, `deriv`, and
   `surviving` are all excluded because the poly walk tracks none of them, and carrying
   them would be dead weight inviting "why is this always `None`". Folding `lits`'s
   `int_val` into the struct (deleting the parallel vector) is optional consolidation and
   should be decided, not drifted into.
2. **Quotation-consuming word dispatch.** Teach `poly_call_term` to consume a quotation
   slot. `poly_call_term`'s `match name` has no `call`, `branch`, `if`, `times`, or `tag`,
   and while `resolve_combinator_overload` is defined (`poly.rs:1701`) it is never called
   from `poly.rs`; `inline_combinator` (`check/combinators.rs:364`) is reached only from
   the concrete path (`check/terms.rs:644`). The generated eliminators (`Shape?`, `Opt?`)
   are members of this family and are the consumer, so they lead. The bulk of the slice.
3. **Abstract N-arm join.** Port the depth check, per-slot agreement, and `Moves::join`
   wiring from `check_branch_join`. Needed in its **N-arm** form, not just two-arm: an
   eliminator has one arm per variant, and the concrete path already generalizes
   `Moves::join` from two to N with a per-slot `merge_arm_output_slot` (`check.rs:2443`,
   `:2480`). `Moves::join` itself is reusable as-is.

## Locked decisions

**Type variables stay rigid across arms; the body gets no mid-body `Subst`.** Today
nothing in a polymorphic body binds a type variable — `Var(v)` is a skolem, and every
`Subst::default()` in `poly.rs` is at a call-site/instantiation boundary
(`:177/1611/1655/1811`), never in the term walk. Keeping that means the arm merge is a
decidable structural `PolyType` comparison: arm A leaving `Var(0)` and arm B leaving
`Var(1)` disagree, and arm A `Var(0)` against arm B `Concrete(i64)` is a new located
error. Admitting the latter by binding `'T := i64` would mean a genuinely new mid-body
unifier with ripples into `poly_output_mismatch_error` and `instantiation_symbol`
mangling. Not needed for `if`, and ruled out.

**Splice-consumed quotations only.** A quotation in a polymorphic body must be consumed by
`call`/`branch`/a combinator argument in that same body. It may not be materialised: stored
in a field or array element, returned, or erased into a capture set. This is what keeps
`surviving` out of `PolySlot`, and it avoids two known pre-existing ICEs (a quotation inside
a row-typed combinator's row, and a materialized quotation returning a ref) plus the
`unreachable!("a quotation effect never reaches monomorphized lowering")` at
`ir/driver.rs:664`, which fires only when a quotation type reaches the grounded *signature*.
The rejection for a materialised one must be located and must name why, not fall through to
the generic message this slice is deleting.

**Shuffling a quotation stays legal.** Restricting a quotation to immediate consumption
(no `dup`/`swap` before use) would be cheaper but rejects legal programs, since combinator
code shuffles quotations routinely. Identity must survive reordering.

**The arm merge unions the borrow table.** Probe-established, and this one can produce a
**false accept** rather than merely a false reject, so it is not a detail. `PolyScope`'s
borrow table is keyed by a local's *name*, and `prune_dead_borrows` (`poly.rs:90-104`) only
ever decides whether to clear the table wholesale: a conflict is detected by
`live_borrow_of(place)` *finding* a record, so a **missing** record reads as "no conflict"
and accepts. If arm A borrows `&!x` and arm B borrows `&!y`, a merge that picks one arm or
intersects drops the other's record while its reference is still live on the merged stack,
and a later use of that place is silently accepted. Within a single linear path the
table's coarseness only over-rejects, which is why it has been safe until now; a branch
breaks that property. The concrete path already does the right thing for comparison:
`check_branch_join` threads one `&mut Provenance` through both arms (accumulating a union),
and `merge_arm_output_slot`'s doc states that "one arm leaving a live borrow the other
doesn't (or of a different place) is rejected here rather than silently erased to a
provenance-free slot" (`check.rs:2476-2480`). The poly join must union, and must reject the
disagreement rather than erase it.

## Open questions

1. **Can the poly path dispatch a generated eliminator word?** `Shape?` is an ordinary word
   resolved by name, unlike `if`, which is a row-typed inline combinator. If eliminator
   dispatch is materially simpler than the row-typed combinator path, the slice may be able
   to deliver its consumer without solving row-unification-against-an-abstract-stack at all,
   and `if`/`call`/`times` could follow separately. This is the single biggest remaining
   sizing question and the spec should open with it.
2. **Does an abstract scrutinee interact with P6.S3b's check-time tag resolution?** P6.S3b
   moves arm-tag typing to check time against the `EnumId` the scrutinee operand carries.
   In a polymorphic body that operand may be abstract. For a *concrete* enum in a poly body
   (this slice's first consumer) the `EnumId` is known, so this may be vacuous — but it must
   be confirmed, and it is the seam where this slice meets the other session's work.
3. **`PolySlot` struct vs. a third lock-step vector.** A parallel
   `quots: Vec<Option<QuotRef>>` is fewer edits (the ~26 `stack[i]` type-reads stay
   untouched) but adds a third vector to keep synchronised across all 28 mutation sites and
   widens the length invariant to three-way. The struct is more edits but removes that
   invariant class, mirrors the concrete `Slot`, and lets `lits` be deleted. Recon
   recommends the struct on the grounds that a desynced `quot` **mis-compiles** where a
   desynced `lits` only mis-diagnoses. The spec should confirm and record the choice.
4. **Does the quotation literal's own body get walked abstractly, and against what?** The
   concrete path checks a literal against a declared effect
   (`check_literal_against_declared_effect`); there is no poly analogue. For `if`'s arms
   this is where a `~[ ..a -- ..b ]` row effect meets an abstract stack.
5. **Capture admission has no poly twin.** The concrete `check_branch_join` carries
   capture-admission machinery. Under the splice-only rule a quotation cannot escape, so
   this may be vacuous — but "may be" needs to become "is", with a test.
6. **How much of the quotation-consuming family is in scope?** The consumer needs
   eliminators. `call`, `branch`, `if`, and `times` are separable, and `if` is the expensive
   one (row-typed). Each one left out needs a located rejection rather than an `unknown
   word` fallthrough, which is what they produce today with the wall stubbed open.

## Out of scope

- Trait bounds (P7.S3e).
- Slices (P7.S3c).
- Calling a rowless quotation parameter (a comparator, no `..a`/`..b`) from a poly body:
  P7.S3d, inserted after this slice and before the trait-bounds slice.
- Materialised, escaping, or erased quotations in a polymorphic body (see locked
  decisions); the two pre-existing ICEs in that neighbourhood are not this slice's to fix.
- Mid-body unification of type variables (see locked decisions).
- The array-constructor rejection (`poly.rs:517-527`), which is a separate gap with the
  same shape and should not be quietly bundled in.

## Ready to spec?

Yes. The consumer question that sank P7.S3e is answered here and answered by a compiling
witness, not an argument: a polymorphic word cannot eliminate an enum, and the failure is
this wall alone.

No dependency on Phase 6's in-flight work for the implementation — the first consumer is a
*concrete* enum eliminated in a polymorphic body, which P6.S3 already shipped. P6.S3b
widens the consumer to generic enums and P6.S4 makes this the only route to elimination at
all, so the two threads should stay in touch (open question 2 is the seam), but neither
blocks starting.

Sizing is **L**, comparable to P7.S3a: a mechanical representation change across ~54 sites,
a new quotation-consuming dispatch family, and an N-arm port of `check_branch_join`,
against zero lowering work. Open question 1 could reduce it: if eliminator dispatch alone
satisfies the consumer, the row-typed `if` path can be deferred to its own slice.

Two stale/false source comments at `poly.rs:505-513` should be corrected by this slice,
since it deletes the rejection they justify.
