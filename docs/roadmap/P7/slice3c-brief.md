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

So a non-`inline` word must either **fix the length in its signature** (`&[i64 5]`,
which then only accepts length-5 arrays) or **thread the length as a second parameter**
(`( &[i64 5] usize -- i64 )`, which still fixes the element count in the type). The only
escape today is `inline`, which defers the index check to each concrete call site — that
is why every generic word in `lib/arrays.sth` is `inline`, and it is why a slice-shaped
signature is a prerequisite for the trait-bounds consumers (P7.S3e).

A `Slice['T]` is a borrowed view carrying its length at runtime: one parameter, no
length variable, indexable from a non-`inline` body.

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
`word_entry.rs:174` permits an input to *be* a reference, forbidding only one nested
inside an aggregate. So `( Slice['T] -- i64 )` is legal and `( -- Slice['T] )` is not.

**The ban applies to user declarations only.** Its call sites are `word_entry.rs:52` (a
`WordDef`), `declarations.rs:50`, and `audits.rs:277` (the poly twin). Compiler-known
words are dispatched by name through the checker's word families (`check_array_word`,
`word_families.rs:716`, which owns `fill` and the `&>`/`&!>` index ops) and never reach
that audit — `&>` already produces a `&T` output today. Construction therefore does not
need new grammar: a compiler-known word can produce a `Slice` on the stack.

**`str` is a precedent for the semantics and a misleading one for the representation.**
`IrType::Str` is *one* 8-byte word: the opaque address of a `{ptr, len}` descriptor
built **statically at compile time** by `emit_str_literal` (`backend/qbe.rs:733`), with
the length read at a backend-only `STR_LEN_OFFSET = 8` (`qbe.rs:727`). A slice's pointer
and length are computed at runtime, so that trick does not carry: a slice needs a
genuinely two-word value or a runtime-materialised descriptor. What *does* carry is the
type-level shape — an opaque pointer, the length as the only promise, and no
participation in `contains_reference`.

**Borrow tracking is coarse and per-place, with no notion of extent.** `Deriv`
(`src/check/engine.rs:55`) keys on a bare local-name `String`; `project()` clones the
parent chain and only flips `projected: bool`, leaving `place` unchanged. `PolyBorrow`
(`src/check/poly.rs:96`) is coarser still, with no `projected` flag. Consequences, each
probed:

- two shared borrows of one array: **accepted**
- two mutable borrows of one array: rejected, `at most one &! to a place`
- one mutable + one shared: rejected, `never a & alongside a &!`
- two mutable borrows of *different elements*: **rejected**, with the compiler's own
`note: path disjointness is not modeled: a reference projected into one field borrows
the whole place`
- two mutable borrows of *different struct fields*: rejected identically — field projection
gets no finer treatment than array indexing

A borrow **dies when consumed**, which is what keeps sequential code working:
`examples/poly_borrow_setat.sth` writes elements 0..3 of one array in sequence because
each element borrow is retired by its `!`.

**A runtime bounds-check trap already exists.** `emit_oob_trap` (`backend/qbe.rs:790`,
`$oobfmt` at `:64`) does `dprintf(2, ...)` then `exit(1)`; its own comment calls it
"Sooth's first runtime failure path". An out-of-range index traps at runtime with a
located message; the *compile-time* index check fires only for a bare integer literal
handed straight to `&>` (writing `9 >usize` clears the tracked literal and defers to the
runtime trap).

**An `Option` cannot carry a reference.** Probed in three forms — a struct field, a
concrete enum payload, and a generic enum instantiated at `&i64` — all rejected with `a
reference cannot be stored`. So a fallible accessor returning `Option[&'T]` is not
merely unimplemented, it is forbidden by the second-class-reference rule.

## The consumer

The forcing consumer is a non-`inline` word over a buffer whose length is not in its
type. Today's closest working twin threads the length and fixes the element count, and
it compiles and runs (probe `p4_sum.sth`, prints `25`):

```sooth
: sum ( &[i64 5] usize -- i64 )
  | arr n |
  0 n >i64 ~[ | i | arr i >usize &> @ add ] times ;
```

Note the two warts this slice removes: `5` in the type (so it accepts only length-5
arrays) and `usize` as a second parameter. The delivered shape:

```sooth
: sum ( Slice[i64] -- i64 )
  | s |
  0 s len >i64 ~[ | i | s i >usize &> @ add ] times ;
```

Recursive divide-and-conquer over sub-ranges works under the input-only rule, because
every slice is created and consumed inside one body and only ordinary values cross the
return boundary:

```sooth
: rec ( Slice[i64] -- i64 )
  | s |
  s len 2 div | mid |
  s 0 mid subslice        rec
  s mid s len mid sub subslice  rec
  add ;
```

Sequential mutation works for the same reason — one live mutable borrow at a time, each
retired by the call that consumes it — so an in-place `quicksort!` that recurses left
then right is expressible. Two *simultaneously* live mutable sub-slices are not, and
stay rejected by the coarse borrow table; that is the parallel case and it is deferred
(see "Out of scope").

## Shape of the work

**Compile-forced `Type` arms (3).** Exhaustive matches with no wildcard, found by
cross-grepping for the last variant so no wildcard-less match is missed: `ast.rs:1764`
(`Type::name`), `ir/types.rs:266` (`ir_type_of`), `declarations.rs:1181` (the
value-containment graph).

**Compile-forced `IrType` arms (~10, probe-recounted).** A slice needs its own `IrType`,
since IR-level dispatch is the entire reason `IrType::Str` exists: `ir/layout.rs:307`
(field width, where the current `... | Str | Cstr | Code => 8` encodes the one-word
assumption), `layout.rs:338` (`carried_slot_bytes`), the ABI/load/store classification
in `backend/qbe.rs:314`, `:365`, `:399`, `:423`, and `ir/driver.rs:507` (confirmed
forced, not "may or may not" — it ends `Code | Quotation => unreachable!()` with no
wildcard). A subagent audit (`/tmp/probe-slice-audit/findings.md`) found three more
exhaustive sites this brief's first draft missed: the `Instr::Print` dispatch
(`qbe.rs:1125`) and both REPL rich-value renderers (`repl.rs`,
`rich_value_size`/`render_rich_value`). All are low-risk (`unreachable!`/display arms
the compiler will flag), so the true forced count is ~13, not ~9 — the number was an
undercount, not the risk classification.

**The dangerous sites are the wildcard ones, which compile clean and are wrong.** An
audit against the built compiler (`/tmp/probe-slice-audit/findings.md`) confirmed the
six named below and found three more this brief's first draft missed — all three
soundness-relevant, not merely functional gaps:

- `check/builtins.rs:233` `is_copy` — `_ => true`, so a slice would be wrongly `Copy`
- `check/builtins.rs:279` `contains_reference` — `_ => false`, so a slice would be wrongly
reference-free, and the output ban above would stop covering it
- `check.rs:432` `find_zero_unsafe_element` — here `Str`/`Cstr`/`Quotation` are named
*explicitly* as zero-unsafe, so an omitted slice arm falls to the wildcard and is
treated as zero-**safe**, silently admitting an all-zero slice out of the array
constructor
- `check/operators.rs:342` (the `.` printable set), `check/poly.rs:783` (poly `len`),
`repl.rs:604` (`format_stack`), `word_families.rs:693`/`:705` (`len`/`cstr` guards),
`declarations.rs:176`/`:187` (the `extern` boundary)

**Three more, found by the audit, not in the brief's first draft:** `classify_capture`
(`check/captures.rs:169`, `_ => CaptureClass::Scalar`) — a captured slice is analysed as
a plain scalar at a quotation materialization boundary, an escape-analysis hole of
exactly the same shape as the six above; `remap_type` (`repl.rs:219`, `other => other`)
— rebases `Struct`/`Enum`/`Array`/`OwnedCell`/`Ref` ids by module base on cross-module
import, and a `SliceId` would NOT be rebased, colliding in session space (the same bug
class as the already-fixed `project_span_lacked_module_id`); `qbe_abi_ty`
(`backend/qbe.rs:335`, `_ => width()`) — a slice at an ABI boundary (param/return/arg)
would be classified with a scalar register width instead of a 16-byte aggregate, the
`IrType`-side twin of the fat-`Ref` failure this brief already uses to justify a
separate variant. The poly twins (`poly_is_copy`, poly `len`) inherit the same wildcards
through `PolyType::Concrete` delegation. Note that `is_copy` and `contains_reference` do
not contain the token `Str` at all, so a grep-driven port misses exactly the two most
load-bearing sites — and would also miss all three of these.

**New behaviour**, beyond making the type exist: a `len` arm that answers a slice's
runtime length (today `len` refuses a reference outright); index ops (`&>`/`&!>`)
accepting a slice receiver and bounds-checking against the runtime length rather than a
static count; a construction word and a sub-range word; and the borrow rules ported so a
slice is exclusivity-tracked, non-escaping, and banned from outputs exactly as a `&T`
is.

## Locked decisions

**A slice is a borrowed view, second-class, input-only.** It is not owning, creating one
consumes nothing, and no word may return one. This follows from the declaration-level
output ban, which has no exception mechanism to relax.

**The representation is `Type::Slice(SliceId)`, its own variant, not a fat flavour of
`Type::Ref`.** Reuse would inherit the soundness rules for free (`contains_reference`'s
`Type::Ref => true`, the output ban, exclusivity) but breaks an explicit, documented
uniformity invariant: `ir/types.rs:263` collapses every reference to `IrType::Ptr`, and
`layout.rs:377` states it outright ("Every reference lowers to `IrType::Ptr`"). Making
one flavour 16 bytes would leave every existing `IrType::Ptr` site compiling and wrong,
with no compile error to find them, and that failure set is diffuse. The separate
variant's failure set is enumerable instead: the nine wildcard sites listed above. `str`
also already occupies this shape as its own variant, so reuse would give Sooth two
unrelated mechanisms for pointer-plus-length; and partial struct projection, the future
beneficiary reuse would serve, wants a field-set rather than a length and so would not
share the mechanism anyway.

**The price of that choice is that every rule `Type::Ref` gets automatically must be
ported deliberately**, and each port needs a test that fails when its arm is missing.
The `Copy` classification is the sharpest of them: `is_copy` answers `Type::Ref(_,
mutable, _) => !mutable` (`builtins.rs:250`), so a slice must mirror that split — a
shared slice is `Copy`, a mutable one is not — where the `_ => true` wildcard would make
a `&!` slice freely duplicable and break exclusivity outright. The others are
`contains_reference` reporting true so the output ban keeps covering slices, exclusivity
tracking, and the non-escaping rule. A slice that silently inherits `str`'s answers is
the failure mode this decision accepts in exchange for loud representation errors, so it
is the spec's main test obligation.

**Construction and sub-ranging are compiler-known words, not user-declarable ones.**
They produce a reference-shaped value, which no user signature may declare as an output.
`&>` is the precedent: a checker word family arm, dispatched by name, exempt from the
signature audit. This also means no new grammar is required.

**Element access keeps the existing runtime trap; there is no fallible accessor in this
slice.** `Option[&'T]` is forbidden outright (references cannot be stored), and a
by-value `Option['T]` cannot be written generically either — `Option['T]` over a word's
own type variable is rejected (`grounding a generic over its own type variable is not
yet implemented`). A real fallible accessor needs a user-declarable `Option`/`Result`,
which needs traits (P7.S3e). Until then slice indexing traps exactly as array indexing
does. **This contradicts the current P7.S3c exit criterion, which must be rewritten as
part of this slice** — the criterion as written ("indexing reports failure through an
`Option`/`Result` the caller must handle, with no runtime panic path") describes a
change that cannot be built yet, and a runtime trap path is the status quo, not a
regression.

**Element types stay concrete and `Copy`.** Arrays cannot hold linear elements today
(`linear array elements are not supported yet`, probed on both the `fill` and
`[Type;Count]` paths), so a slice over a linear element has no buffer to view.

**Simultaneous mutable sub-slices stay rejected.** The coarse per-place borrow table is
unchanged by this slice. Sequential mutation, including recursive divide-and-conquer
taking one half at a time, is what is delivered.

**Sliceable sources are exactly the borrowable places, probed directly rather than
guessed: `Struct`, `Enum`, `Array`, `OwnedCell` locals, plus a scalar `static:` (`&COUNT
: &i64`, `&LABEL : &str`).** A `str` *local* cannot be borrowed at all — the scalar-
local gate (`word_families.rs:171`) rejects it before any ref type forms, named
explicitly in the error as `str` — so "slice a str into a view" cannot lean on borrowing
a str local; the only `&str` obtainable today is a str *static* (`/tmp/probe-
sliceable/findings.md` §1-3). Struct-typed statics aren't declarable this slice either
(the `static:` grammar only allows scalars), so there is no aggregate-static source to
slice from yet. The array case is the one this slice targets.

**A slice is accepted where a quotation parameter's input row is declared, confirmed
end-to-end.** A hand-written `inline` word whose quotation parameter's row contains a
reference (concrete, generic, or array-shaped) is accepted at declaration and lowers and
runs (`/tmp/probe-quotparam/findings.md`); output-side quotation references stay
rejected at the literal's construction boundary exactly as the input-only rule above
requires, so this does not reopen the output ban. No shipped combinator currently
threads an element reference into its quotation, so this slice needs no combinator
change to be usable — a future parallel/scoped combinator over slices is a type-
machinery non-issue, whatever else it needs.

**Length is carried, never rediscovered.** As with `str`, the length is authoritative
for every Sooth-side use and is never recovered by scanning.

## Open questions

1. **Can a slice be sub-ranged again, given references cannot nest?** Probed and
   confirmed closed on all four attempted forms: reborrowing a reference-typed local
   (value path) and writing a nested `&` in a stack effect (type path) are both rejected
   (`/tmp/probe-sliceable/findings.md` §4). If `Slice` is itself reference-shaped, `s 0
   mid subslice` cannot be "a reference to `s`'s reference" — it must be `subslice`
   constructing a fresh `Slice` value from `s`'s pointer and length (consuming or
   reborrowing `s` as an ordinary input, the way `&>` consumes an array reference to
   produce an element reference today), never nesting one slice inside another's
   representation. The recursive consumer's `s 0 mid subslice rec` shape is only valid
   under this reading; the spec must state it, since "sub-range again" sounds like re-
   borrowing and the mechanism that actually works is closer to re-deriving.
2. **Naming.** `subslice` is a placeholder. The longer-term intent recorded for
   projections is a distinct prefix-sigil form consistent with `&`, rather than
   word-shaped spellings; this slice should pick names that do not fight that later
   change.
3. **Reborrow chain depth: probed clean today, but the load-bearing case doesn't exist
   yet.** A battery of 10 depth-2/3 probes against today's `&!buffer -> &!field ->
   &!^cell -> &!element`-shaped chains (struct/array/owned-cell nesting, both
   owned-local and `&!`-parameter roots, branch-join arms) found no live bug:
   `drop`ping an inner reborrow never frees or pins an outer place's tracking any
   differently than an ordinary consumer would (`/tmp/probe-reborrow/findings.md`). The
   mechanism is structural, not coincidental — `Deriv`/`PolyBorrow` records are
   value-copied into an append-only arena per `borrow`/`reborrow`/`project` call, so
   there is no parent-child link for a drop to sever. **But** the probe's own caveat is
   the one that matters here: today's deepest chains route through owned-cell deref and
   array indexing, never through a *reference stored inside an aggregate* that outlives
   the expression — which is exactly what `Type::Slice` is, if it is a struct-shaped
   `{ptr, len}` carrying a reference-typed field. That shape is untested by this probe
   and must be re-run once the real type exists; the spec must require a test that
   mutates through the innermost hop of a `&!buffer -> &!Slice -> &!sub-slice ->
   &!element` chain while the outer hops are still live, since that is the one new
   shape this decision's own evidence didn't cover.
4. **Does the poly path need its own arms?** `PolyBorrow` is coarser than `Deriv` (no
   `projected` flag) and poly `len` is a separate match (`poly.rs:783`). A slice used
   inside a polymorphic body is the point of the slice, so the poly twins are not
   optional, but their extent is unmeasured.

## Out of scope

- Any fallible/`Option`-returning accessor, and the choice between a plain `Option` and a
`Fallible` bound: both wait on traits (P7.S3e), which is also where P7.S3e's own
standing note about fixing the index-failure carrier is discharged.
- Generic element types (`Slice['T]` instantiated over a word's own type variable), blocked
on generic-instantiation-over-own-type-variable.
- Range-aware borrow tracking and simultaneously-live disjoint mutable sub-slices. The
parallel map-reduce case is served instead by a scoped combinator that owns the split
and joins before returning, keeping disjointness inside the primitive; that combinator,
and the scoped-spawn intrinsic it needs, belong to P10.
- Owning, runtime-length buffers (a `Box<[T]>` analogue). `core` has no allocator, so an
owned variable-length buffer is P9 layer work; this slice views storage it does not own.
- Length arithmetic in signatures (`['T 'N+'M]`), already ruled out phase-wide.
- Slicing as a route to linear elements: arrays cannot hold them at all yet.

## Ready to spec?

Yes. The representation is ruled (`Type::Slice(SliceId)`), which fixes what the spec's
tests must guard: the ported soundness rules, since this choice trades silent
unsoundness for loud representation errors.

The consumer is probe-established rather than argued: a non-`inline` word over a buffer
of unfixed length cannot be written today, by either available spelling, and the two
errors are quoted above. The cheapest dogfood is a concrete-element `sum`, whose
length-threading twin already compiles and runs, so the delivered shape can be diffed
against a working baseline rather than against a hypothesis.

No dependency on P7.S3d: every capability this slice needs is exercised by a concrete
consumer, and the comparator quotation that needs the rowless splice belongs to `sort`,
not here.

Sizing is **M**: a new type variant with ~13 compile-forced arms (probe-recounted from
an initial ~9), nine wildcard sites that must be ported deliberately rather than found
by grep (three of them — `classify_capture`, `remap_type`, `qbe_abi_ty` — found only by
a dedicated audit, not by the brief's own first pass), two new compiler-known words, a
`len` arm, index ops taught a runtime bound, and the borrow rules ported — against a
representation that is genuinely new at the IR and ABI level, since `str`'s static
descriptor does not generalise.
