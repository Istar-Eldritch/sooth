# Phase 7 Slice 3c: slicing a buffer into a view (brief)

A word that works over a borrowed buffer of *any* length cannot be written. The two
spellings available both fail, verified against the built compiler:

```text
: sum ( &[i64 'N] -- i64 )        \ generic length: cannot index it
error: cannot index a generic-length array in `sum` (line 1, col 47)
  the array's length is the type variable `'N`, so its element cannot be statically
  bounds-checked; index a concrete-length array (`['T 4]`), or use a fixed length in
  this word's signature

: mylen ( &['T 'N] -- usize ) len ;   \ ask the reference for its own length
error: `len` is not permitted on a reference in `mylen`
```

So a non-`inline` word must either **fix the length in its signature** (`&[i64 5]`, which
then only accepts length-5 arrays) or **thread the length as a second parameter**
(`( &[i64 5] usize -- i64 )`, which still fixes the element count in the type). The only
escape today is `inline`, which defers the index check to each concrete call site — that is
why every generic word in `lib/arrays.sth` is `inline`, and it is why a slice-shaped
signature is a prerequisite for the trait-bounds consumers (P7.S3e).

A `Slice['T]` is a borrowed view carrying its length at runtime: one parameter, no length
variable, indexable from a non-`inline` body.

## Recon (probe-verified against the built compiler)

Four parallel probes against the prebuilt binary; every claim below is compile- or
run-grounded, and the honest unknowns are carried into "Open questions".

**References cannot be returned, at all, and the rule is declaration-level.**
`stored_reference_output_error` (`src/check/builtins.rs:311`) fires from
`check_reference_free_signature` for any declared output whose type *is* or *contains* a
`Type::Ref`. It is a blanket type test with no provenance or lifetime reasoning: a
reference reborrowed from an input is rejected exactly like a freshly-minted one.

```text
: mk ( -- &[i64 5] )
error: a reference cannot be stored: `mk` declares the output `&[i64 5]`
  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the
  caller reads it; take the reference as an input instead
```

**Inputs are explicitly exempt**, which is what makes this slice possible at all:
`word_entry.rs:174` permits an input to *be* a reference, forbidding only one nested inside
an aggregate. So `( Slice['T] -- i64 )` is legal and `( -- Slice['T] )` is not.

**The ban applies to user declarations only.** Its call sites are `word_entry.rs:52`
(a `WordDef`), `declarations.rs:50`, and `audits.rs:277` (the poly twin). Compiler-known
words are dispatched by name through the checker's word families (`check_array_word`,
`word_families.rs:716`, which owns `fill` and the `&>`/`&!>` index ops) and never reach
that audit — `&>` already produces a `&T` output today. Construction therefore does not
need new grammar: a compiler-known word can produce a `Slice` on the stack.

**`str` is a precedent for the semantics and a misleading one for the representation.**
`IrType::Str` is *one* 8-byte word: the opaque address of a `{ptr, len}` descriptor built
**statically at compile time** by `emit_str_literal` (`backend/qbe.rs:733`), with the
length read at a backend-only `STR_LEN_OFFSET = 8` (`qbe.rs:727`). A slice's pointer and
length are computed at runtime, so that trick does not carry: a slice needs a genuinely
two-word value or a runtime-materialised descriptor. What *does* carry is the type-level
shape — an opaque pointer, the length as the only promise, and no participation in
`contains_reference`.

**Borrow tracking is coarse and per-place, with no notion of extent.** `Deriv`
(`src/check/engine.rs:55`) keys on a bare local-name `String`; `project()` clones the
parent chain and only flips `projected: bool`, leaving `place` unchanged. `PolyBorrow`
(`src/check/poly.rs:96`) is coarser still, with no `projected` flag. Consequences, each
probed:

- two shared borrows of one array: **accepted**
- two mutable borrows of one array: rejected, `at most one &! to a place`
- one mutable + one shared: rejected, `never a & alongside a &!`
- two mutable borrows of *different elements*: **rejected**, with the compiler's own
  `note: path disjointness is not modeled: a reference projected into one field borrows the
  whole place`
- two mutable borrows of *different struct fields*: rejected identically — field projection
  gets no finer treatment than array indexing

A borrow **dies when consumed**, which is what keeps sequential code working:
`examples/poly_borrow_setat.sth` writes elements 0..3 of one array in sequence because each
element borrow is retired by its `!`.

**A runtime bounds-check trap already exists.** `emit_oob_trap` (`backend/qbe.rs:790`,
`$oobfmt` at `:64`) does `dprintf(2, ...)` then `exit(1)`; its own comment calls it "Sooth's
first runtime failure path". An out-of-range index traps at runtime with a located message;
the *compile-time* index check fires only for a bare integer literal handed straight to `&>`
(writing `9 >usize` clears the tracked literal and defers to the runtime trap).

**An `Option` cannot carry a reference.** Probed in three forms — a struct field, a
concrete enum payload, and a generic enum instantiated at `&i64` — all rejected with
`a reference cannot be stored`. So a fallible accessor returning `Option[&'T]` is not
merely unimplemented, it is forbidden by the second-class-reference rule.

## The consumer

The forcing consumer is a non-`inline` word over a buffer whose length is not in its type.
Today's closest working twin threads the length and fixes the element count, and it
compiles and runs (probe `p4_sum.sth`, prints `25`):

```sooth
: sum ( &[i64 5] usize -- i64 )
  | arr n |
  0 n >i64 ~[ | i | arr i >usize &> @ add ] times ;
```

Note the two warts this slice removes: `5` in the type (so it accepts only length-5 arrays)
and `usize` as a second parameter. The delivered shape:

```sooth
: sum ( Slice[i64] -- i64 )
  | s |
  0 s len >i64 ~[ | i | s i >usize &> @ add ] times ;
```

Recursive divide-and-conquer over sub-ranges works under the input-only rule, because every
slice is created and consumed inside one body and only ordinary values cross the return
boundary:

```sooth
: rec ( Slice[i64] -- i64 )
  | s |
  s len 2 div | mid |
  s 0 mid subslice        rec
  s mid s len mid sub subslice  rec
  add ;
```

Sequential mutation works for the same reason — one live mutable borrow at a time, each
retired by the call that consumes it — so an in-place `quicksort!` that recurses left then
right is expressible. Two *simultaneously* live mutable sub-slices are not, and stay
rejected by the coarse borrow table; that is the parallel case and it is deferred (see
"Out of scope").

## Shape of the work

**Compile-forced `Type` arms (3).** Exhaustive matches with no wildcard, found by
cross-grepping for the last variant so no wildcard-less match is missed:
`ast.rs:1764` (`Type::name`), `ir/types.rs:266` (`ir_type_of`), `declarations.rs:1181`
(the value-containment graph).

**Compile-forced `IrType` arms (~5-6).** A slice needs its own `IrType`, since IR-level
dispatch is the entire reason `IrType::Str` exists: `ir/layout.rs:307` (field width, where
the current `... | Str | Cstr | Code => 8` encodes the one-word assumption),
`layout.rs:338` (`carried_slot_bytes`), and the ABI/load/store classification in
`backend/qbe.rs:314`, `:365`, `:399`, `:423`. `ir/driver.rs:507` may or may not force one.

**The dangerous sites are the wildcard ones (~6), which compile clean and are wrong.**
These are where a new type variant silently inherits `str`'s answers:

- `check/builtins.rs:233` `is_copy` — `_ => true`, so a slice would be wrongly `Copy`
- `check/builtins.rs:279` `contains_reference` — `_ => false`, so a slice would be wrongly
  reference-free, and the output ban above would stop covering it
- `check.rs:432` `find_zero_unsafe_element` — here `Str`/`Cstr`/`Quotation` are named
  *explicitly* as zero-unsafe, so an omitted slice arm falls to the wildcard and is treated
  as zero-**safe**, silently admitting an all-zero slice out of the array constructor
- `check/operators.rs:342` (the `.` printable set), `check/poly.rs:783` (poly `len`),
  `repl.rs:604` (`format_stack`), `word_families.rs:693`/`:705` (`len`/`cstr` guards),
  `declarations.rs:176`/`:187` (the `extern` boundary)

Note that `is_copy` and `contains_reference` do not contain the token `Str` at all, so a
grep-driven port misses exactly the two most load-bearing sites.

**New behaviour**, beyond making the type exist: a `len` arm that answers a slice's runtime
length (today `len` refuses a reference outright); index ops (`&>`/`&!>`) accepting a slice
receiver and bounds-checking against the runtime length rather than a static count; a
construction word and a sub-range word; and the borrow rules ported so a slice is
exclusivity-tracked, non-escaping, and banned from outputs exactly as a `&T` is.

## Locked decisions

**A slice is a borrowed view, second-class, input-only.** It is not owning, creating one
consumes nothing, and no word may return one. This follows from the declaration-level
output ban, which has no exception mechanism to relax.

**Construction and sub-ranging are compiler-known words, not user-declarable ones.** They
produce a reference-shaped value, which no user signature may declare as an output. `&>`
is the precedent: a checker word family arm, dispatched by name, exempt from the signature
audit. This also means no new grammar is required.

**Element access keeps the existing runtime trap; there is no fallible accessor in this
slice.** `Option[&'T]` is forbidden outright (references cannot be stored), and a by-value
`Option['T]` cannot be written generically either — `Option['T]` over a word's own type
variable is rejected (`grounding a generic over its own type variable is not yet
implemented`). A real fallible accessor needs a user-declarable `Option`/`Result`, which
needs traits (P7.S3e). Until then slice indexing traps exactly as array indexing does.
**This contradicts the current P7.S3c exit criterion, which must be rewritten as part of
this slice** — the criterion as written ("indexing reports failure through an
`Option`/`Result` the caller must handle, with no runtime panic path") describes a change
that cannot be built yet, and a runtime trap path is the status quo, not a regression.

**Element types stay concrete and `Copy`.** Arrays cannot hold linear elements today
(`linear array elements are not supported yet`, probed on both the `fill` and
`[Type;Count]` paths), so a slice over a linear element has no buffer to view.

**Simultaneous mutable sub-slices stay rejected.** The coarse per-place borrow table is
unchanged by this slice. Sequential mutation, including recursive divide-and-conquer taking
one half at a time, is what is delivered.

**Length is carried, never rediscovered.** As with `str`, the length is authoritative for
every Sooth-side use and is never recovered by scanning.

## Open questions

1. **`Type::Slice(SliceId)` or a new fat flavour of `Type::Ref`? Unruled, and it is the
   spec's first decision.** Reuse inherits the soundness rules for free —
   `contains_reference`'s `Type::Ref => true`, the output ban, exclusivity — which is
   precisely where a separate variant is silently wrong. But reuse breaks an explicit,
   documented uniformity invariant: `ir/types.rs:263` collapses every reference to
   `IrType::Ptr`, and `layout.rs:377` states it outright ("Every reference lowers to
   `IrType::Ptr`"). Making one flavour 16 bytes leaves every existing `IrType::Ptr` site
   compiling and wrong, with no compile error to find them. The recommendation is the
   **separate variant**, on the grounds that its failure set is enumerable (the ~6
   wildcard sites above, already listed) while reuse's is diffuse (every "a reference is
   one pointer" assumption in IR and backend); that `str` already occupies this shape as
   its own variant, so reuse would give Sooth two unrelated mechanisms for
   pointer-plus-length; and that partial struct projection, the cited future beneficiary,
   wants a field-set rather than a length and so would not share the mechanism anyway. The
   counter-case is real and the spec must rule explicitly.
2. **What is sliceable?** A fixed array through `&`/`&!` is the obvious source. A `static:`
   place and a `str` are candidates; each needs a ruling rather than falling out of the
   implementation.
3. **Can a slice be sub-ranged again?** The recursive consumer above requires it, so the
   answer is presumably yes, but it makes the reborrow chain longer and the mechanism must
   be stated rather than assumed.
4. **Naming.** `subslice` is a placeholder. The longer-term intent recorded for projections
   is a distinct prefix-sigil form consistent with `&`, rather than word-shaped spellings;
   this slice should pick names that do not fight that later change.
5. **Reborrow chain depth is where a bug should be expected.** `&!buffer` -> `&!Slice` ->
   `&!sub-slice` -> `&!element` is three or four reborrow hops off a reference local.
   Depth-1 tests will all pass and prove nothing; a prior forwarding fix in this codebase
   passed every depth-1 test while a two-hop chain let `drop` strand a live reference. The
   spec must require a test that mutates through the innermost hop while the outer hops are
   live.
6. **Does the poly path need its own arms?** `PolyBorrow` is coarser than `Deriv` (no
   `projected` flag) and poly `len` is a separate match (`poly.rs:783`). A slice used inside
   a polymorphic body is the point of the slice, so the poly twins are not optional, but
   their extent is unmeasured.
7. **Is a slice accepted where a quotation parameter is expected?** Unverified, and
   load-bearing for the parallel combinators this slice is meant to leave room for, though
   not for anything inside this slice.

## Out of scope

- Any fallible/`Option`-returning accessor, and the choice between a plain `Option` and a
  `Fallible` bound: both wait on traits (P7.S3e), which is also where P7.S3e's own standing
  note about fixing the index-failure carrier is discharged.
- Generic element types (`Slice['T]` instantiated over a word's own type variable), blocked
  on generic-instantiation-over-own-type-variable.
- Range-aware borrow tracking and simultaneously-live disjoint mutable sub-slices. The
  parallel map-reduce case is served instead by a scoped combinator that owns the split and
  joins before returning, keeping disjointness inside the primitive; that combinator, and
  the scoped-spawn intrinsic it needs, belong to P10.
- Owning, runtime-length buffers (a `Box<[T]>` analogue). `core` has no allocator, so an
  owned variable-length buffer is P9 layer work; this slice views storage it does not own.
- Length arithmetic in signatures (`['T 'N+'M]`), already ruled out phase-wide.
- Slicing as a route to linear elements: arrays cannot hold them at all yet.

## Ready to spec?

Yes, with open question 1 ruled first, since the representation choice determines which
failure mode the implementation is exposed to and therefore what the spec's tests must
guard.

The consumer is probe-established rather than argued: a non-`inline` word over a buffer of
unfixed length cannot be written today, by either available spelling, and the two errors
are quoted above. The cheapest dogfood is a concrete-element `sum`, whose length-threading
twin already compiles and runs, so the delivered shape can be diffed against a working
baseline rather than against a hypothesis.

No dependency on P7.S3d: every capability this slice needs is exercised by a concrete
consumer, and the comparator quotation that needs the rowless splice belongs to `sort`,
not here.

Sizing is **M**: a new type variant with ~9 compile-forced arms, ~6 wildcard sites that
must be ported deliberately rather than found by grep, two new compiler-known words, a
`len` arm, index ops taught a runtime bound, and the borrow rules ported — against a
representation that is genuinely new at the IR and ABI level, since `str`'s static
descriptor does not generalise.
