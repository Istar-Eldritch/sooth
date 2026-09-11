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

## Round S6d-7 (clean-HEAD re-verification post-PREREQ, no patch, run 260911)

One `prober` worker. Scratch discipline: a pristine `git archive HEAD`
extract at `/tmp/s6d-clean` supplied the clean-tree binary; all compiler
patching happened in a separate extract at `/tmp/s6d-scratch`; this worktree
kept `src/` and `lib/` untouched throughout. Probe fixtures live under
`probes/` (`s6d_*.sth`); their clean-tree behavior is captured verbatim in
[`probes/s6d_baseline.md`](../../probes/s6d_baseline.md), and compiled
binaries were removed from `probes/` after each accepted build. Probes run
with `--manifest tests/fixtures/sooth.pkg`.

### S6d-7.1 — the fence, byte-for-byte, both targets

`probes/s6d_a_fence_baseline.sth` (full hosted program: local
`Step`/`Iterator`, `impl: Iterator for Slice[i64]` with a real `next` body,
a mono `next` call in `main`):

```text
error: trait member `next` of `Iterator` (line 37, col 3) applies the trait variable `'It`, but the impl target `Slice[i64]` is concrete
  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead
```

exit=1. The mutable twin (one word different — `impl: Iterator for
!Slice[i64]`, run from a /tmp copy of the same file):

```text
error: trait member `next` of `Iterator` (line 37, col 3) applies the trait variable `'It`, but the impl target `!Slice[i64]` is concrete
  an application-headed member has no monomorphic representation ...
```

Identical shape to the pre-PREREQ S6d-1/S6d-2 captures — the PREREQ changed
nothing about the fence (`member_app_concrete_target_error`, `src/ast.rs:
2135`, raised from `parse_impl_member_body`, now `src/parser.rs:4440`).

### S6d-7.2 — PREREQ admissions, isolated from the fence (plain words, no trait)

- **(a) inline packing word** — `probes/s6d_b_step_shared_inline.sth`:
  `: mk inline ( i64 Slice[i64] -- Step[i64 Slice[i64]] ) More ;` builds and
  runs on the CLEAN tree; the mono consumer destructures via `Step?`/`More>`
  and prints `41` then `5` (the remainder's `len`). exit=0 both. REQ-5's
  admit-and-taint works end-to-end for the shared slice in a declared enum
  payload.
- **(b) non-inline twin** — `probes/s6d_b2_noninline_out.sth`: the output ban
  fires at the declared signature, exactly as S6d-4 observed for a bare
  slice output:

```text
error: a reference cannot be stored: `mk` declares the output `Step[i64 Slice[i64]]`
  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead
```

- **(c) `!Slice` payload twin** — `probes/s6d_b3_mut_payload.sth`:
  `Step[i64 !Slice[i64]]` is rejected by the enum-payload sweep of
  `check_no_stored_references` (`src/check/declarations.rs:1108`), naming the
  parse-time-minted monomorph — the same sweep/wording shape as the old S6d-2
  rejection, naming `!Slice`, as predicted:

```text
error: a reference cannot be stored: payload field 1 of variant `More[i64 !Slice[i64]]` of type `Step[i64 !Slice[i64]]` has type `!Slice[i64]` (line 10, col 3)
  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow
```

- **(d) copy-ness** — the same s6d_b file's `: dup-check ( i64 Slice[i64] -- )
  More dup drop drop ;` compiles: the packed container `dup`s clean (REQ-5's
  "a shared-slice container stays Copy").

### S6d-7.3 — cheap sanity

`sooth build examples/slices.sth` (manifest discovery via
`examples/sooth.pkg`), clean HEAD: build exit=0, run prints `15`, `6`, `6`,
exit=0. Bare-slice input to non-inline words is still admissible
post-PREREQ; `sum`/`double` unchanged.

### Round-level verdict

The PREREQ unlocked exactly the payload layer (S6d-7.2a/d) and kept every ban
sharp (7.2b/c); the S2-6 member-app fence is byte-identical to the pre-PREREQ
tree and is the only remaining declaration-time blocker for both targets.

## Round S6d-8 (the two-half sentinel patch, scratch copy, run 260911)

All compiler edits in `/tmp/s6d-scratch` (a `git archive HEAD` extract);
this worktree untouched. Both halves were written from the S6d-2/S6d-4
ledger and verified independently by the negative controls below.

### S6d-8.1 — the exact diff shape on the current tree

Two files, three hunks, no deletions (except one rewritten import line):

1. **`src/parser.rs`**, line 25: `SliceId` added to the `crate::ast` import.
2. **`src/parser.rs:855-964`** (inserted after `build_member_var_union`):
   `const SLICE_SENTINEL_IDX: u32 = u32::MAX;`, `fn slice_sentinel(element,
   name)` (~10 lines), and `fn rewrite_slice_sentinel(ty, id, mutable, name)`
   (~75 lines, recursion mirroring `member_ty_mentions_app`'s shape over
   Generic/GenericVariant/Array/Ref/OwnedCell/Quotation). ~110 lines with
   comments.
3. **`src/parser.rs` inserted at 4629-4691** (inside
   `parse_impl_member_body`, after the member lookup and `dg` construction,
   BEFORE both the `is_mono_ctor_app` and `is_concrete` branches — a slice
   target is `Concrete`, so it must preempt the concrete path): the ~63-line
   slice branch. It matches
   `PolyType::Concrete(Type::Slice(..))`, builds the sentinel (a fake
   `PolyType::Generic { is_enum: false, idx: u32::MAX, module: 0, args:
   vec![Concrete(element)], len_args: [], name }` carrying the slice's
   element from `self.slices[id.index()]`), clones the target with the
   sentinel pattern, runs the **unmodified** `build_member_var_union` +
   `ground_member_poly`, rewrites every sentinel back to
   `Concrete(Type::Slice(..))`, and on `poly_type_is_var_free` mints the mono
   member word exactly as the mono-ctor-app branch does (`poly: None`,
   `declares_inline` inherited from the trait member). A non-var-free
   grounding falls through to the same two fences the mono-ctor-app branch
   raises.
4. **`src/check/poly/ground.rs:1319-1333`** (inside
   `resolve_mono_member_call`'s mono branch, which a slice target enters
   because `imp.target.is_concrete()` is true): one new guarded arm in the
   effect-derivation match — `Some(_) if matches!(&imp.target.pattern,
   PolyType::Concrete(Type::Slice(..)))` reads the already-grounded member
   word's effect (`trait_resolve.words[*widx].effect`) instead of re-deriving
   via `ground_member_type`. 15 lines.

Total ~190 lines added, zero changed behavior for non-slice targets (the
branch falls through on any non-slice pattern; the ground.rs guard only
fires for slice targets).

**Invariant-exception comments (the spike's caveat (i), mutual pointers):**
`SLICE_SENTINEL_IDX`'s doc states the exception and points at the
`PolyType::Generic` invariant ("`idx` indexes `GenericTypes::structs`/
`enums`", `src/ast.rs:~2711`); the sentinel is recognizable by construction
(`u32::MAX` is unreachable as a real registry index) and is erased by
`rewrite_slice_sentinel` before `ground_var_free`/`substitute_generic_field`
— the two header-dereferencing paths — ever see it.

**Clippy/fmt note for the implementer:** `cargo clippy -- -D warnings` is
clean as written. The rewrite function binds the `&'static str` field under
a RENAME (`name: header`) in the `Generic` arm, which sidesteps clippy 1.96
`explicit_auto_deref`'s field-shorthand demand; a match arm that binds that
field AS `name` and reconstructs with shorthand may trip it. `cargo fmt`
wanted two cosmetic reformats (the ground.rs `match` header join; the
`PolyType::Array` arm layout) — applied.

### S6d-8.2 — the exact spelling that builds (and a NEW wall the brief did not know)

The prescribed body shape (`len` zero-test with `dup` before `len`; `More`
packing element deepest, remainder on top; Done arm dropping the exhausted
slice) does **not** check even with both halves in place:

```text
error: borrow state disagrees at the branch join in `next` (member of trait `Iterator` for `Slice[i64]`) (line 26)
  the first arm leaves no live borrow, the second arm leaves a borrow with no local root: both arms must agree on which place, if any, stays borrowed past the join
  note: declared ( Slice[i64] -- Step[i64 Slice[i64]] )
```

This is `borrow_join_disagreement_error` (`src/check/terms.rs:3797`, raised
at `:3613`): the `More` arm's `Step` slot carries the remainder view's deriv
(forwarded by REQ-4d site 1's construction push, `src/check/terms.rs:1352`
— every reference-bearing ctor output forwards its first slice-bearing
operand's deriv), while the `Done` arm's `Done` (nullary, no operands)
carries none. The join refuses the `(None, Some(_))` asymmetry instead of
unioning. The Step-row protocol's Done/More asymmetry over a **borrow-flavored**
payload is exactly this shape: **a fifth wall, beyond the brief's four and
beyond the PREREQ's storage scope — the branch-join borrow rule.**

The spelling that builds routes the Done arm through a poly helper whose
call-site output push (REQ-4d site 4, the fixed mono/poly dispatch pushes)
forwards the dropped view's deriv onto the Step slot, giving both arms' Step
slots the same suspension. `probes/s6d_a_fence_baseline.sth`, verbatim:

```forth
: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;
```

with the trait member spelled `: next inline ( 'It['T] -- Step['T 'It['T]] ) ;`
— the member MUST be inline (S6d-7.2b's ban; see 8.4(iii)). `as-done` is poly
on purpose: a mono non-inline twin is banned at the word entry, and an inline
twin splices to `drop Done`, whose push carries no deriv (the join
disagreement again). The poly output slot is a declared `Var`, which the
signature audit treats as reference-free — the deferred per-instantiation
audit is the gap site 4's forward exists to close. This workaround is
load-bearing and slightly over-conservative (the Done-path Step claims a
borrow it does not have); the checker-generalization alternative (union
derivs at the join like aliases already are) is a design decision for the
implementer, not a probe.

The sentinel grounding itself works exactly as the S6d-2 spike predicted:
the grounded signature renders in the join error's own note — `note:
declared ( Slice[i64] -- Step[i64 Slice[i64]] )` — and `main`'s single
`next` step over a 5-element view prints `3` then `4` (element, remainder
length).

### S6d-8.3 — the monomorphic consumer

`probes/s6d_c_impl_consumer.sth`: `main` fills a 5-array and a 1-array,
takes views, and a NON-tail recursive `drain` walks them with bare `next`
(a mono member call — the `resolve_mono_member_call` slice arm exercised for
real), `Step?` dispatch arms and `More>` destructure, printing each element:

```text
3
3
3
3
3
9
```

build exit=0, run exit=0 — the second drain proves the `Done` path runs at
runtime (a 1-element view hits `More` once, then `Done`).

### S6d-8.4 — negative controls (each reverted independently)

- **(i) parser half reverted** (ground.rs half in place): the fence returns
  byte-identical to S6d-7.1 (`member_app_concrete_target_error`, line 37,
  col 3). exit=1.
- **(ii) ground.rs half reverted** (parser half in place): the impl parses
  and mints, and the consumer's `next` call site panics at exactly the
  predicted backstop:

```text
thread 'main' (2987092) panicked at src/ast.rs:2271:33:
internal error: entered unreachable code: an App-headed member signature reaches ground_member_type only through the concrete desugar, which fences it first (S2-6: no mono representation for member locals)
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

  exit=101. (`imp.target.is_concrete()` is true for `Concrete(Type::Slice(..))`,
  so the mono branch re-derives and hits `ground_member_type`'s App arm.)

- **(iii) member re-spelled non-inline** (both halves in place;
  `probes/s6d_next_noninline.sth`): the member word's own output ban —
  the inherited `declares_inline: false` sends the synthesized member word
  through `check_reference_free_signature`:

```text
error: a reference cannot be stored: `next` (member of trait `Iterator` for `Slice[i64]`) declares the output `Step[i64 Slice[i64]]`
  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead
```

### S6d-8.5 — regression, and a second regression the patch EXPOSED

With the two-half patch and `lib/` at HEAD: `cargo test --no-fail-fast` is
**3480 passed, 0 failed** across 92 test binaries; `cargo clippy -- -D
warnings` clean; `cargo fmt --check` clean (after the two cosmetic
reformats).

But the slice member must be `inline`, and `declares_inline` is inherited
from the **trait** member (`src/parser.rs:4452`) — there is no per-impl
inline spelling (impl members take no effect and no keyword slot). Spelling
`: next inline ( ... ) ;` on the shared `Iterator` trait (the only way to
make the slice member inline) breaks **6 of 39** `phase7b_slice8` goldens —
the List/Range impls' own bodies:

```text
emitting the fixture should succeed: error: the quotations passed to `List?` leave different stack shapes: an earlier one leaves `Step[i64 Range[i64]]`, this one leaves `Step[i64 List[i64]]` in `next` (member of trait `Iterator` for `List['T0]`) (line 51)
build should succeed; stderr: error: unknown word `Done` in `next` (member of trait `Iterator` for `List['T0]`) (line 50)
```

failures: `consuming_loop_over_range_is_one_frame_with_a_back_edge_and_next_
is_a_real_frame`, `core_iterator_module_drains_a_list_through_its_cross_
module_impl`, `fold_sums_a_list_through_the_iterator_bound`,
`for_each_and_fold_drain_a_range_through_the_iterator_bound`,
`for_each_drains_a_list_through_the_iterator_bound`,
`range_next_dispatches_at_a_mono_call_site`. Toggling ONLY the lib edit
(patches in place, no slice impl anywhere) reproduces it; reverting the lib
edit restores 39/39 — the trigger is the member's inline-ness, not the
sentinel patch and not any mint. Mechanism (observed, not chased to root):
an inline poly member is checked/spliced through the poly-combinator path,
where the nullary variant ctor `Done` either resolves against a WRONG
existing `Step` monomorph (`Step[i64 Range[i64]]` inside the List impl) or
fails to resolve at all (`unknown word Done`) — the S8b wrong-monomorph
disease ("variant words clobber the bare-name map") reached through a second
door. **Consequence for the spec: S6d cannot ship `next inline` at the trait
level. It needs a per-impl inline spelling (desugar change) or a fix to the
inline-poly-member check path, and either way the 6 goldens are the
regression pin to keep green.**

### S6d-8.6 — bound-generic consumers (for_each/fold) over the slice impl

Three gates fire in sequence, none of them the predicted ones:

1. **Placement gate (tree-independent).** A slice impl over the IMPORTED
   `core::iterator` trait in an entry file
   (`probes/s6d_f0_libiter_gate.sth`):

```text
error: `impl: Iterator for Slice[i64]` at line 8, col 1 must live in the module declaring `Iterator` (`Slice[i64]` declares no module of its own)
```

   The gate's co-declaration arm (an impl may live where its TARGET is
   declared) is unavailable — `Slice` declares no module — so the only
   admitted home is `core/iterator.sth` itself.
2. **Member-output ban.** Inside the lib, with the trait member non-inline
   (HEAD spelling), the impl parses (sentinel) and the member word dies at
   the S6d-8.4(iii) ban. With the trait member inline, S6d-8.5's 6-golden
   regression fires instead. The lib route is therefore blocked at both
   spellings on this tree.
3. **Bound-slot unification (the actual 8.6 answer).** With a probe-local
   Iterator (inline member) + the lib's `for_each`/`fold` bodies verbatim,
   both consumers die at the CALL SITE, before any App fence or back-edge
   check (`probes/s6d_f_for_each_slice.sth`, `probes/s6d_f2_fold_slice.sth`):

```text
error: type mismatch in `main` (line 41)
  `for_each` expected `'It['T]`, found `Slice[i64]`
  note: declared ( -- )
```

```text
error: type mismatch in `main` (line 49)
  `fold` expected `'It['T]`, found `Slice[i64]`
  note: declared ( -- )
```

   The check is `unify_poly_input` (`src/check/poly/unify.rs:127`, raising
   `poly_rendered_type_mismatch_error`): the bound slot `'It['T]` is
   App-headed and decomposes only against a ctor application — `'It` binds
   to the application's HEAD. A slice is a bare `Concrete` view with no ctor
   head, so `'It` has nothing to bind to. **SOO-60 (cross-call App fence)
   and SOO-42 (back-edge gate) are both unreached for slices: the
   bound-generic consumer channel is closed earlier, at the same structural
   point that made the S8 Range lift need `is_mono_ctor_app` — dispatch
   identity. A slice has no header to dispatch on.**

### Round-level verdict

Both halves of the sentinel patch work exactly as the S6d-2/S6d-4 ledger
predicted (negative controls (i)/(ii) confirm each half is load-bearing).
The Step-row protocol then checks and runs end-to-end for `Slice[i64]` — but
only via the `as-done` join workaround (a NEW fifth wall: the branch-join
borrow rule rejects the Done/More deriv asymmetry), and only with the member
inline, which today cannot be spelled per-impl and breaks 6 List/Range
goldens at the trait level. Bound-generic consumers are closed earlier than
predicted: the `'It['T]` slot cannot unify a bare slice. The mutable target
is separately closed by Ruling A (Round S6d-9).

## Round S6d-9 (the mutable target, scratch copy, patch in place, run 260911)

`probes/s6d_g_mut_impl.sth`: `impl: Iterator for !Slice[i64]` with the
analogous body — linear binder mentions instead of `dup` (`!Slice` is not
`Copy`, S6d-4), `&!>` element read, `&!buf slice` receiver shape in the
consumer. **Prediction confirmed**: the sentinel grounds and the impl parses
(the parse-time mint of `Step[i64 !Slice[i64]]` succeeds), and the
first-firing check is the Ruling A enum-payload sweep — nothing fires
earlier (the member is inline, so no word-entry ban; the sweep runs at the
`Step` type's own variant span):

```text
error: a reference cannot be stored: payload field 1 of variant `More[i64 !Slice[i64]]` of type `Step[i64 !Slice[i64]]` has type `!Slice[i64]` (line 7, col 3)
  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow
```

exit=1. (line 7 col 3 = the `| More 'T 'Rest` row of the file's own `Step`
declaration — the sweep names the monomorph's variant.) No lowering or
consumer probe was needed: the prediction held, so per the round plan
nothing further applies.

### Round-level verdict

`!Slice[i64]` is closed under the Step-row protocol exactly as Ruling A
says: the declared-aggregate payload ban fires before any impl-specific
check. The spec's deferred note should record that the FIRST firing is the
enum-payload sweep at the `Step` declaration, not anything at the impl.

## Round S6d-10 (consumer shapes and placement, scratch copy, patch in place, run 260911)

### S6d-10.1 — self-tail recursive drain: prediction FALSIFIED (then re-pinned where it does hold)

`probes/s6d_d_selftail.sth` — `rest drain` in tail position — **builds and
runs**, printing `3 3 3 3 3` (exit=0 both). `check_reference_across_back_edge`
(`src/check.rs:1670`) does NOT fire: the guard rejects only a crossing slot
whose deriv has a non-static `owned_root` — a local of the current frame —
and a non-inline word's parameter-derived remainder has none (the function's
own doc names this the accept-case: "A reference *parameter*, or one derived
from it by projection, has no owned root (`owned_root` is `None`, the
accept-case) and may cross freely"). The PREREQ's deferred note ("a tainted
Window can never cross a self-tail back edge") holds only where the root is
VISIBLE at check time — its own golden uses an INLINE consumer spliced into
main. The sharper twin proves the guard still bites there:
`probes/s6d_d2_selftail_inline.sth` (the same drain declared `inline`) is
rejected:

```text
error: a reference to a local cannot cross a loop in `main` (line 37)
  a reference derived from `buf`, a local of this frame, crosses the self-tail-call back-edge to `drain`: that local's storage does not survive to the next iteration
  note: declared ( -- )
```

exit=1. **SOO-42's gate is pinned for slices exactly at the root-visibility
boundary: non-inline recursive drains pass, spliced/inline ones cannot.**

### S6d-10.2 — non-tail recursive drain: admissible, as predicted

`probes/s6d_e_nontail_drain.sth` — the self-call NOT in tail position
(`r drain v .`, the element prints after the recursion returns) — builds and
runs, printing `4 4 4 4` (exit=0 both). A bare `Slice[i64]` input to a
non-inline word is admissible (the `examples/slices.sth` `sum` precedent);
a real call frame cannot outlive its caller. O(n) stack, one frame per
element — fine at probe scale, the same cost `double` already pays.

### S6d-10.3 — while-threaded drain: REJECTED, and the rejection is a PREREQ-mask finding

Three checks had to be navigated just to spell the shape (each captured on
the way): (a) a bare `Done` reconstructed inside the while quotation is
refused by strict use-determined grounding — the S11 amendment —

```text
error: `Done` in `drain` (line 39) cannot be grounded here: `Step['T 'Rest]`'s type parameter `'T` (parameter 1 of 2) is determined by neither this call site's operands nor its consumer
  note: pass the value to a consumer whose declared parameter names a concrete `Step[...]`, or name that instantiation in a signature so this call has one to ground at
```

(b) keeping the exhausted shell as the next state trips the variant-escape
rule —

```text
error: an arm of `Step?` leaves `Step[i64 Slice[i64]].Done` on the stack in `drain` (line 39)
  a variant-typed value is reachable only inside the arm that bound it; consume it there, or leave its fields instead
```

— and (c) with every spelling that survives (a) and (b) (state = the `Step`
value, Done arm deriving an empty view from the parameter and routing it
through `as-done`, concrete quotation annotation), the build dies at
**while's OWN internal join** (`lib/core/combinators.sth:80`,
`~[ p while ] ~[ ] if`; the error's line 80 is the spliced combinator's
body):

```text
error: borrow state disagrees at the branch join in `drain` (line 80)
  the first arm leaves no live borrow, the second arm leaves a borrow with no local root: both arms must agree on which place, if any, stays borrowed past the join
  note: declared ( Slice[i64] -- )
```

The mechanism is the PREREQ's own documented latent mask, now REACHABLE:
`back_edge_outs` (`src/check/terms.rs:3773`) "forwards `surviving` alone and
drops `deriv`" across while's self-tail back edge, so the recursion arm's
state arrives deriv-free while the base arm `~[ ]` keeps the state's deriv —
a manufactured join disagreement. The PREREQ's guard test pinned this as
"unreachable only because the guard rejects first"
(`back_edge_rejects_a_deriv_carrying_aggregate_argument`); a slice-threaded
state with a parameter-rooted (rootless) deriv passes that guard, so the
mask fires. **A while-threaded slice drain is impossible on this tree, and
the fix belongs to the checker (`back_edge_outs` must forward deriv), not to
any user-level spelling.** No workaround was found or expected: the state
inherently carries a view deriv, and every construction forwards it.

### S6d-10.4 — times-bounded drain: times refuses; unrolled bounded stepping works

The natural `times` spelling (state threaded through the row,
`probes/s6d_i_times_drain.sth`) is rejected at the times quotation's
standalone check — the row is abstract there, so `next`'s member dispatch
has no concrete operand:

```text
error: `next` in `drain3` (line 43, col 7) is a trait member of Iterator, but no `impl:` in this program dispatches on these operands
  the operand types here are ``; declare an impl of one of those traits for the operand's type, or import a word that claims this name
```

(`mono_member_no_dispatch_error`, with an EMPTY operand-type list — the row
renders as nothing.) The escape hatch of annotating the quotation concretely
is refused too — a times quotation's declared row renders `~[ i64 -- ]`, the
`..s` row is not annotation-comparable:

```text
error: the quotation passed to `times` is annotated `~[ Slice[i64] i64 -- Slice[i64] ]` but `times` declares it `~[ i64 -- ]` in `drain3` (line 38)
```

**Prediction ("works") falsified for `times` as such.** The unrolled
equivalent — N literal `next`/`Step?` steps in sequence, Done arm deriving
the row-satisfying empty view from the binder — builds and runs (fixed
3 steps over a 5-element view: prints `3 3 3` then the remainder's length
`2`), so bounded stepping itself is fine; only the combinator-hosted loop
shape is closed (by 10.3's mask for `while`, by the abstract-row standalone
check for `times`).

### S6d-10.5 — placement: the gate forces co-location, and co-location wakes a PRE-EXISTING bug

- The placement gate (captured in S6d-8.6) leaves `core/iterator.sth` as the
  only home for the slice impl: `Slice` declares no module, so the gate's
  co-declaration arm cannot apply. **The `range.sth` "beside-the-protocol"
  convention is structurally unavailable for slices** — it was a choice for
  `Range` (whose target IS co-declared with its impl), and it is not one for
  builtin-view targets.
- With the slice impl placed inside `lib/core/iterator.sth` in the scratch
  copy (lib member non-inline, HEAD spelling) and a List-drain consumer
  built against it, the build dies at the CONSUMER, not the impl:

```text
error: type mismatch in `drain` (line 9)
  `More>` expected `Step[i64 Slice[i64]].More`, found `Step[i64 List[i64]].More`
  note: declared ( List[i64] -- )
```

  **Reproduced on the CLEAN tree (no patch, no impl, no slice code) by a
  mere signature mention that mints the instantiation at parse:**

```forth
: touch inline ( Step[i64 Slice[i64]] -- ) drop ;
```

  in a file that also drains a `List[i64]` produces the byte-identical
  `More>` error. This is a **pre-existing wrong-monomorph resolution bug**
  (the S8b disease class: a newly minted `Step` instantiation re-types the
  bare generated variant words — `More>`/`Done`/`More` — of every OTHER
  `Step` monomorph in the program). S6d did not create it; S6d's lib
  placement would hit it immediately, which is exactly what the
  `range.sth` header comment feared ("which a consumer of the protocol's own
  List impl must not have to see"). **That concern is not stale — it is
  acute, and today it is not even a choice for slices.** Reported, not
  fixed (Notes).

- The lib placement with the member left non-inline also shows the impl's
  own ban on a trivial importer (`import: core::iterator ... ;` + `: main
  ( -- ) 1 drop ;`): `error: a reference cannot be stored:`next` (member of
  trait `Iterator` for `Slice[i64]`) declares the output ...` — the lib
  cannot ship the impl without the inline problem of S6d-8.5 being solved
  first.
- Consumer import spells (established, from `tests/phase7b_slice8.rs`'s
  goldens): `import: core::list | List Nil Cons | ;` +
  `import: core::iterator | Step Done More Iterator | ;` for a hand-written
  dispatch drain; `import: core::iterator | for_each | ;` (or `| fold |`,
  or `| for_each fold |`) + `import: core::range | Range | ;` for the
  bound-generic consumers. `core::iterator` deliberately does not export
  bare `next` (the mangled member name is the only spelling; bare `next`
  resolves through trait dispatch at the call site).

### Round-level verdict

Admissible consumer shapes on the patched tree: **non-tail recursive drain
(runs, O(n) stack), self-tail recursive drain (runs — the back-edge guard
accepts parameter-rooted remainders; it fires only for spliced/inline
consumers), single-step and unrolled bounded stepping (runs).** Closed
shapes: **while-threaded (the PREREQ mask, `back_edge_outs` deriv-drop,
becomes reachable), times-hosted (abstract-row standalone check), and every
bound-generic consumer (`'It['T]` unification — SOO-60/SOO-42 unreached).**
Placement: co-location in `core/iterator.sth` is forced by the gate, and the
parse-time `Step[i64 Slice[i64]]` mint wakes a pre-existing
wrong-monomorph bug in List/Range consumers (clean-tree reproducible without
any S6d code).

## Round-level summary (S6d-7..S6d-10, run 260911)

- **What the PREREQ already unlocked (S6d-7):** shared-`Slice[T]` payloads in
  declared enums (REQ-5, inline pack + mono destructure + `dup` all clean on
  unpatched HEAD), with every ban sharp and byte-stable (the non-inline
  output ban; the `!Slice` payload sweep). The fence itself is unchanged.
- **The exact current-tree diff shape (S6d-8):** parser half =
  `src/parser.rs` three hunks (import at :25; `SLICE_SENTINEL_IDX` +
  `slice_sentinel` + `rewrite_slice_sentinel` at :855-964; the slice branch
  at :4629-4691 inside `parse_impl_member_body`, preempting `is_concrete`);
  dispatch half = `src/check/poly/ground.rs:1319-1333`, the slice guard arm
  reading the already-grounded member word in `resolve_mono_member_call`'s
  effect match. ~190 lines, no registry threading, clippy/fmt clean, 3480/0
  tests. Negative controls confirm each half is load-bearing ((i) fence
  returns; (ii) `ast.rs:2271` unreachable panic at the call site; (iii) the
  member output ban).
- **The mutable-target verdict (S6d-9):** closed by Ruling A's enum-payload
  sweep, first-firing, at the `Step` declaration's span; the impl-side
  grounding itself works.
- **Admissible consumer shapes (S6d-10):** non-tail and self-tail
  (parameter-rooted) recursion; single-step and unrolled bounded stepping.
  Closed: while-threaded (the `back_edge_outs` deriv-drop mask, reachable
  for the first time), times-hosted (abstract-row standalone check), and all
  bound-generic consumers (`'It['T]` slot unification — a slice has no ctor
  head to bind, so SOO-60/SOO-42 are unreached). **for_each/fold over
  slices: not reachable on this tree, for the dispatch-identity reason, not
  the predicted fences.**
- **Two pre-existing bugs the round exposed (not created by it):** (1) the
  trait-level `next inline` breaks 6 List/Range goldens (the inline
  poly-member check path mis-resolves the nullary variant ctor);
  (2) any new `Step` instantiation minted anywhere re-types other Step
  monomorphs' bare variant words for List/Range consumers (clean-tree
  reproducible with a signature mention alone). Both are S8b-class
  wrong-monomorph resolution defects and are S6d blockers of the same rank
  as the fence.
- **The new S6d-specific wall (S6d-8.2):** the branch-join borrow rule
  (`borrow_join_disagreement_error`) rejects the Step-row Done/More deriv
  asymmetry for a borrow-flavored payload; the `as-done` poly-helper is the
  minimal working spelling, at the cost of a conservative over-approximated
  deriv on the Done path.

## Notes

- Unchased mechanism: WHY the inline poly member mis-resolves `Done` (two
  failure shapes: wrong-monomorph `Step[i64 Range[i64]]` at the arm join;
  bare `unknown word Done`). The S8b ledger's "wrong θ seeding + variant-map
  clobber" note is the closest prior art; a future slice should read
  `check_poly_combinator_standalone`'s concrete stand-in path first.
- The `back_edge_outs` deriv-drop mask (PREREQ deferred note) is now
  REACHABLE from user programs via a slice-threaded `while` state whose deriv
  has no owned root at the guard. The guard test pins the aggregate case;
  the rootless case slips past it. Worth a pin of its own.
- `mono_member_no_dispatch_error`'s rendering of an abstract-row operand
  list as EMPTY (`the operand types here are ```) reads as a diagnostic bug
  (an empty list where the row should render as`..s` or the abstraction).
  Cosmetic; noted for whoever touches that message.
- The `s6d_a` family's `as-done` workaround exploits the deferred
  per-instantiation signature audit exactly the way the PREREQ's own
  deferred note predicted a blanket audit must NOT strand ("it must leave an
  exempt path for synthesized multi-output bundles" — and, as now
  demonstrated, for poly ctor-shaped outputs too).
