# P7b.S6d probe log — slices as Iterator targets

Round S6d-1, run 260907. One `prober` worker, throwaway `.sth` files under
`examples/`/`/tmp` only; no `src/` file touched; repo clean after the run.

## S6d-1 — does `impl: Iterator for Slice[i64]` parse/check today?

`next`'s shape copied from `range.sth`'s pattern (destructure via `len`/`subslice`).
Exact output:

```text
error: trait member `next` of `Iterator` (line 5, col 5) applies the trait variable `'It`, but the impl target `Slice[i64]` is concrete
  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead
```

Confirmed: `member_app_concrete_target_error` (`src/ast.rs:2135`), raised via
`fence_member_app_against_concrete_target` (`src/ast.rs:2208`) — `next`'s
`'It['T]` is App-headed and `Slice[i64]` grounds as a bare `PolyType::Concrete`,
not a `Generic`/ctor pattern the fence exempts.

## S6d-2 — `impl: Iterator for !Slice[i64]`

Identical error, target string swapped:

```text
error: trait member `next` of `Iterator` (line 5, col 5) applies the trait variable `'It`, but the impl target `!Slice[i64]` is concrete
  an application-headed member has no monomorphic representation ...
```

Same code path; no divergence between shared and mutable targets.

## S6d-3 — `impl: Iterator for Slice['T]`

Fails earlier, before the member-app fence is ever reached:

```text
error: unknown type `'T` at line 4, col 26
```

`Slice['T]` is not a legal impl-target spelling at all — nothing in the
parser's target grammar takes a bare type variable as a slice's element, so
the generic-element spelling is unreachable as a workaround, not merely
unadmitted.

## S6d-4 — is `Slice[i64]` Copy? Is `!Slice[i64]` not?

`src/check/builtins.rs:520`: `Type::Slice(_, mutable, _) => !mutable`,
anchored above the `_ => true` wildcard specifically to block a mutable
slice from free duplication (comment at `builtins.rs:508-514`). Existing
unit test `is_copy_shared_slice_is_copy_mutable_slice_is_not`
(`builtins.rs:1056`) already pins this — pre-existing since P7.S3c, not new.

Probe (via `&buf slice` / `&!buf slice`, `examples/slices.sth`'s pattern):

- Shared `Slice[i64]` local, `dup drop drop`: compiles clean.
- Mutable `!Slice[i64]` local, `dup drop drop`:

```text
error: cannot `dup` a value of type `!Slice[i64]` in `main` (line 6)
  `!Slice[i64]` is exclusive: at most one may be live for a place, so copying it would make a second one; use it where it is, or borrow again once it is consumed
  note: declared ( -- )
```

## S6d-5 — slice words' real signatures

From `src/check/word_families.rs` (poly twins in `src/check/poly.rs`):

- `len ( Slice[T] -- usize )` — **consumes** the receiver
  (`word_families.rs:1020-1038`; comment at `:1024`: "a slice answers its
  runtime length... unlike the array fold... it consumes its receiver").
  Poly twin `poly.rs:3225`.
- `subslice ( Slice[T] usize usize -- Slice[T] )` — consumes the receiver,
  re-derives a fresh view offset by `start`, length `len`; output mutability
  matches the receiver's (`word_families.rs:842-861`). Poly twin
  `poly.rs:3275`.
- `&>`/`&!>` element read (`word_families.rs:32-63`) —
  `( Slice[T] usize -- &T )` / `( !Slice[T] usize -- &!T )`, bounds-checked
  against the view's *runtime* length (no compile-time count), matched
  ahead of `ref_parts` since a slice is not `Type::Ref`.

A `next` body sketch follows the shape already in `examples/slices.sth`'s
`sum`/`double`: `len` to test 0, else `&>`/`&!>` plus `subslice` to split
off one element and advance the view.

## S6d-6 — does the dispatch matcher already handle `Type::Slice`?

`match_impl_target_rec`, `src/check/poly.rs:9276-9278`:

```rust
PolyType::Concrete(t) => {
    if *t == ty { Some(()) } else { None }
}
```

`Slice[i64]` grounds as `PolyType::Concrete(Type::Slice(id, mutable, _))` —
not a `Generic`/`Array`/`Ref`/`OwnedCell` shape, so it falls to the plain
`Concrete` equality arm, which works today for `Type::Slice` values with no
dedicated arm needed. Dispatch matching is not the blocker; the blocker is
entirely the declaration/grounding-time member-signature fence (confirmed
by S6d-1/2: the `impl:` is rejected before dispatch matching is ever
exercised).

## Round-level finding

The blocker for both `Slice[i64]` and `!Slice[i64]` as `Iterator` targets is
squarely `member_app_concrete_target_error` — identical for shared and
mutable. `Slice['T]` is unreachable as a spelling, not merely unadmitted, so
there is no generic-element workaround to fall back to. `is_copy`, dispatch
matching, and the constituent words (`len`/`subslice`/`&>`/`&!>`) all
already work correctly and are not blockers. Landing the trait over slices
needs either (a) a dedicated grounding path that lets an App-headed member
row dissolve against a `Concrete(Type::Slice(..))` target directly (no
`GenericId`/ctor to route through, unlike the S8 Range lift — see the
roadmap's own "measured, not assumed" note: slices have no header for a
third `GenericId` case), or (b) narrowing what the member row is allowed to
express so it stays App-free against a concrete target. The S8 Range
lift's mechanism (`impl_target_pattern_poly_type`'s reverse lookup into
`GenericStructDecl`/`GenericEnumDecl`, re-wrapping a `Concrete(Type::Struct
| Type::Enum)` back into `PolyType::Generic` so `is_mono_ctor_app()` holds)
does not transfer: `Type::Slice` is never minted through that
instantiation registry, and `is_mono_ctor_app` only matches the `Generic`
pattern's `(is_enum, idx, module)` header, which a slice has none of.

## Round S6d-2 (DQ4 spike, run 260907)

One `prober` worker, working entirely in a `git archive HEAD` scratch copy at
`/tmp/sooth-spike`; this worktree is untouched (`git status`/`git diff` empty
after the run, confirmed).

### Candidate (a) — dedicated slice grounding path: WORKS, smaller than expected

A sentinel-substitution trick, not a new `PolyType` variant: build a fake
`PolyType::Generic` carrying the slice's element type in `args[0]` (the
parser already owns `self.slices[id.index()].element`), run the *existing,
unmodified* `build_member_var_union`/`ground_member_poly` against that
sentinel, then recursively rewrite every occurrence of the sentinel back to
`PolyType::Concrete(Type::Slice(id, mutable, name))` **before** it reaches
`ground_var_free`/`substitute_generic_field` — the one place that would
dereference the sentinel's bogus `idx`/`module` as real registry indices
and panic or corrupt.

Diff shape, verified by actually building it: one new ~25-line function
(`rewrite_slice_sentinel`, mirrors `member_ty_mentions_app`'s recursion
shape) plus one new ~40-line `else if` branch in `parse_impl_member_body`,
alongside the existing `is_mono_ctor_app`/`is_concrete` branches. Zero
changes to `ast.rs`'s shared functions, zero changes to `MutRegistries`
(27 construction sites checked — threading a `slices` field through
`ground_member_poly`/`build_member_var_union` properly would have touched
all 27; the sentinel trick avoids that entirely). `cargo build` succeeds;
`impl: Iterator for Slice[i64]` with a `next` body grounds cleanly and
mints `Step[i64 Slice[i64]]` as a real monomorph.

**Caveat, load-bearing:** the sentinel value momentarily violates
`PolyType::Generic`'s own documented invariant ("`idx` indexes
`GenericTypes::structs`/`enums`", `ast.rs:2711`) between its construction
and its rewrite. A real implementation needs either (i) this accepted as a
documented, narrowly-scoped exception with a comment at both the
construction site and the invariant doc pointing at each other, or (ii) a
dedicated `PolyType::SliceApp` case — a bigger diff, touching every
exhaustive match over `PolyType` (`member_ty_mentions_app`,
`poly_type_is_var_free`, `apply_subst`, `render_target_pt`,
`substitute_generic_field`, `ground_member_poly` itself, and likely several
`check/poly.rs` sites — not fully counted). (i) is smaller and was the one
built and verified; (ii) is more honest to the invariant and unverified.

### Candidate (b) — ground before fencing: DEAD END, confirmed by reading the call site

`fence_member_app_against_concrete_target`'s call site in
`parse_impl_member_body` runs immediately before `ground_member_type`, and
`ground_member_type`'s own `App` arm is `unreachable!()` by design (its doc
comment at `ast.rs:2168`) precisely because the fence is unconditional and
earlier. There is no ordering to invert: `ground_member_type` takes a
plain `Type`, not a `PolyType`, so it has nowhere to substitute a member
local into even though `'It` is already known (it *is* the target `Type`).
The fence isn't premature; the concrete-grounding function's signature
can't carry what an App-headed row needs, full stop. (b) is not a viable
direction.

### Third finding — a second, independent blocker neither candidate predicted

Even with candidate (a) fully working, the resulting monomorph
`Step[i64 Slice[i64]]` — exactly the shape S6d's own `next` sketch needs
(the remainder slice packed into the enum's `Rest` field) — is
**independently rejected** by `check_no_stored_references`
(`check/declarations.rs:1081`): `contains_reference`'s `Type::Slice(..) =>
true` arm treats a slice as reference-shaped by design (same reasoning as
`&T` — it borrows storage it doesn't own, doc comment at
`declarations.rs:1146`, pre-existing since P7.S3c). This has nothing to do
with App-headed member rows or DQ4's mechanism — it's the ordinary
struct/enum field audit, run once against the final monomorphic `Step`
type, and it fires regardless of how grounding is reached. No version of
candidate (a) routes around it, because it isn't a grounding-time check at
all.

### Round-level verdict

DQ4 alone is answerable: candidate (a)'s sentinel-substitution route is
mechanically sound and smaller than S8's own registry-reverse-lookup
mechanism, at the cost of a documented, narrow invariant exception (or a
bigger, cleaner `PolyType::SliceApp` alternative, unverified). Candidate
(b) is closed. But DQ4 was never the whole blocker: **`Step`'s `Rest` field
cannot hold a `Slice[i64]`/`!Slice[i64]` at all today**, independently of
how the impl grounds, because `Type::Slice` is reference-shaped and
`Step['T 'Rest]` is a plain (non-reference-holding) enum. This is a new,
separate open question the brief's DQ3 gestured toward ("linear discipline
for a view that owns nothing") but did not name: the Step-row protocol's
shape itself (a self-referential `Rest` field) is what trips
`check_no_stored_references`, not anything about drop/ownership semantics.
A slice `Iterator` impl needs either a `Step`-protocol exception for
reference-shaped `Rest` payloads (a `check_no_stored_references` carve-out—
scope and soundness unmeasured), or a different exhausted-case shape for
slice iteration specifically (diverging from the Step-row protocol S8
established), or the ruling that slices are simply not Iterator targets
under the current protocol (an exit-narrowing ruling, not a mechanism fix).

## Round S6d-3 (DQ5 spike, run 260907)

One `prober` worker, working in a plain `git archive HEAD` extract at
`/tmp/sooth-spike` (not a git repo); this worktree confirmed untouched
before and after.

### Why `check_no_stored_references` exists

Doc comment (`declarations.rs:1077`): the declaration-site half of the
no-stored-reference rule — a struct field, enum payload field, interned
array element, or interned cell payload whose type transitively contains a
reference is a located error. The diagnostic states the rationale directly
(`declarations.rs:1146`): "a reference cannot be stored: ... a `&T`/`&!T`
borrows a local and may not outlive it, so it cannot be put anywhere that
survives the borrow." This is **escape/lifetime safety**, not disposal
ordering or exclusivity tracking (a separate, already-working mechanism in
`poly.rs`). It is the *entire* enforcement mechanism: Sooth has no
lifetime-tracking pass at all (no `outlive`/`dangling`/`escape` machinery
anywhere in `check/` besides this rule's own comments). `Type::Slice`'s own
arm (`builtins.rs:564`) carries its own rationale: "a slice carries a
borrow, so it is reference-bearing... reported reference-free, a user word
could declare `( -- Slice[T] )` and hand back a view of its own dead
frame." Pinned since P7.S3c by `contains_reference_true_for_slice`
(`builtins.rs:1091`) and `contains_reference_sees_through_a_struct_field`
(`builtins.rs:1026`) — exactly the transitive struct/enum-field escape
shape DQ5 needs.

### Would a `Type::Slice` carve-out be sound?

No — the rationale applies identically to slices as to `&T`. A slice is a
real runtime `{ptr, len}` view (`ir/types.rs:266` `SliceLayout`, "always the
opaque handle, never the referent's", `ir/types.rs:315`), and there is no
scope/lifetime tracker anywhere to catch a `Step[i64 Slice[i64]]` outliving
the array it views after the fact. `contains_reference`'s unconditional ban
*is* the safety mechanism; removing it for slices removes the only thing
preventing exactly the escape the doc comment already names.

### Spike: the carve-out is not just unsound in theory — it doesn't build

Flipped `Type::Slice(..) => true` to `false` in `contains_reference`
(`builtins.rs:564`, 1-line diff); rebuilt clean; minted
`Step[i64 Slice[i64]]` directly via a plain non-Iterator word (bypassing
DQ4 entirely) to isolate this question.

- **Before**: rejected exactly as predicted — `error: a reference cannot be
  stored: payload field 1 of variant \`More[i64 Slice[i64]]\` ... has type
  \`Slice[i64]\``.
- **After**: the checker accepts it, but the program panics at IR-lowering
  time: `thread 'main' panicked at src/ir/layout.rs:271:29: internal error:
  entered unreachable code: a slice value resolves via \`slice_layout\`, not
  a scalar`. Deliberate, pinned by`scalar_size_align_refuses_a_slice`
  (`layout.rs:1216`): "a slice is two words, not one, and its align is a
  word rather than its size... Mistaking a slice for`Str`'s single opaque
  word is the specific failure the separate`IrType`variant exists to make
  impossible." Enum-variant payload layout (`layout.rs:854`,
  `scalar_size_align_ww`) has no branch for slice-shaped fields at all — no
  registry threading exists for it, unlike struct/array/enum fields.

The carve-out never reaches a runnable dangling-buffer test: it ICEs at the
very first construction, before any consuming loop could be exercised.
Slice-shaped enum payload layout (tag placement, field projection over a
two-word slot, codegen) is real new IR-layout work, not a checker-only
relaxation.

### Round-level verdict

**DQ5 option (i) (a `check_no_stored_references` carve-out) is closed**:
unsound by the same argument the rule already makes for itself (no lifetime
tracker exists to catch the escape any other way), and independently
unbuildable today (enum/struct field layout has a deliberate, tested
refusal for slice-shaped fields — new IR-layout work, not a relaxation).
**DQ5 option (ii) (a different exhausted-case shape that never packs a
slice into an enum field) is the direction this evidence points to** —
materially cheaper than (i), which needs both a soundness ruling reversal
(none available) and new backend work. Option (iii) (rule slices out of
Iterator under the current protocol) remains the fallback if (ii) doesn't
pan out.

## Round S6d-4 (DQ5 option-(ii) verification, run 260907)

One `prober` worker (model-pinned), scratch `git archive HEAD` extract;
worktree confirmed untouched. Question: does the S8-rejected Option row
(`next ( 'It['T] -- Option['T] 'It['T] )`, remainder as a bare stack value,
never packed in an enum field) dodge the layout wall for a slice target?

### Setup

The DQ4 sentinel fix was re-derived in the scratch copy and needed a
**second half not previously identified**: `check/poly.rs::
resolve_mono_member_call` re-derives a concrete-target member's effect via
a raw `ground_member_type` call at every *call site*, and
`imp.target.is_concrete()` is true for `Concrete(Type::Slice(..))`, so the
call site panics on the same `unreachable!` even after the parser-side fix.
The `lifted_mono` condition must also admit slice targets ("read the
already-grounded word, don't re-derive"). With both halves, a probe trait
`Cursor['It: * -> *]` with the Option row, an `impl: Cursor for Slice[i64]`
(real body: `len`-zero test, `&> @` element read, `subslice` remainder),
and a hand-written draining consumer all **parse and check**.

Two checker rules bit along the way, both pre-existing and neither in the
brief:

- `next` must be declared `inline`: `check_reference_free_signature`
  unconditionally bans a non-inline word from *declaring a slice output* at
  all (`: two-out ( Slice[i64] -- i64 Slice[i64] ) ...` → "error: a
  reference cannot be stored: two-out declares the output Slice[i64]") —
  a separate rule from `check_no_stored_references`.
- The remainder must flow as an unnamed stack value: naming it in a local
  and referencing it from a quotation builds a captured-closure env struct
  with a slice field (the same storage class the rules ban).
  [Superseded by Ruling D (260908): the env encoding is one word per capture,
  the capture is checker-admitted, and it ICEs at `qbe.rs:526` — the
  mechanism statement above is what the S6d-4 probe concluded from its
  run, and the prereq spec's fence replaces it.]

### Result: fails at lowering — the same wall, reached through a different struct

```text
thread 'main' panicked at src/ir/layout.rs:271:29:
internal error: entered unreachable code: a slice value resolves via `slice_layout`, not a scalar
```

Isolated with a minimal fixture (no trait, no Option, no closures):

```text
: two-out inline ( Slice[i64] -- i64 Slice[i64] )  |s| 0 s ;
```

panics identically. **Any word with ≥2 declared outputs, one a slice,
panics at lowering**: `check.rs`'s `intern_output_bundles` (R8/R10)
unconditionally interns a synthesized return-bundle struct for every
monomorphic word with `out_arity >= 2`, and that struct hits the exact
`layout.rs` slice-field refusal round S6d-3 found for `Step`'s `Rest`
field. The Option row's "nothing stored in a user-declared enum field"
premise is true at the `check_no_stored_references` level and irrelevant at
the layout level — the compiler synthesizes the aggregate the user avoided
writing.

### Generic consumer (bound-dispatch route)

A `['It: Cursor]` draining fold fails at check with a stack-effect
mismatch (`add` needs 2 values, but the stack holds 0): the poly-body
stack checker loses an App-headed dispatch call's outputs — the standing
S8-era limitation on poly bodies calling poly words over compound
receivers, not S6d-specific. Recorded, not chased.

### Negative control

Without the sentinel fix, the same Cursor+Slice[i64] file dies at
`member_app_concrete_target_error`, byte-identical shape to S6d-1 — the
Option row alone changes nothing.

### Round-level verdict

**DQ5 option (ii) is closed as stated (conditional-no).** The stack-passed
remainder dodges the user-visible storage rule but not the IR: `next`'s
own two-output shape forces a synthesized return-bundle struct subject to
the identical slice-field layout refusal. Ledger of what shipping (ii)
would actually take: the two-half sentinel fix (parser + `resolve_mono_
member_call`), the `inline` requirement on `next` (or lifting
`check_reference_free_signature`'s slice-output ban), the poly-body
stack-checker fix for App-headed dispatch outputs, **and the same new
IR-layout capability (slice-shaped fields in synthesized aggregates) that
option (i) needed** — the supposed cheapness over (i) does not survive
contact with the return-bundle mechanism. Under any protocol whose `next`
has ≥2 outputs (Step row and Option row alike), slices-as-Iterator-targets
is gated on that layout capability plus a checker-level story for
reference-bearing aggregates (a taint/storage-class design, since
`check_no_stored_references`'s escape-safety rationale would apply to the
aggregate exactly as to the bare slice). Option (iii) — defer slices out of
Iterator until such a capability slice exists — is the only route needing
no IR work.
