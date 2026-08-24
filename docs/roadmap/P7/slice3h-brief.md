# P7.S3h brief — An escaping closure may capture a linear value (closure env disposal)

## Problem, confirmed live against current `main` (`a5c084b`)

Capture into an escaping (materialized, non-spliced) closure is gated by
`check_capture_admission` (`src/check/captures.rs:196-247`), reached only from
`materialize_quotation_at_boundary` (`captures.rs:287`) when the literal actually captures
(`body_captures_enclosing`, `captures.rs:12`). `classify_capture` (`captures.rs:144-171`)
buckets every captured name by its `Binding.ty` alone:

- `Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)` → always
  `FrameRooted` (`captures.rs:158-160`), unconditionally — **not gated on linearity or
  `is_copy` at all**.
- `Type::Ref(..) | Type::Slice(..)` → `FrameRooted` or `OuterRooted` per
  `ref_root_is_in_frame` (`captures.rs:126-137`, `164-168`).
- Anything else → `Scalar` (`captures.rs:170`), admitted everywhere (snapshotted into the
  env, D4 amendment — it can never dangle).

`FrameRooted` at an escaping boundary is rejected (`captures.rs:227-229`,
`past_owning_frame_error`, `:49-51`):
> `error: an escaping closure captures {name}, a local of this frame, whose storage
> does not survive the return (line {})`

**This is not a linearity check, and the roadmap's "restricted to `Copy` values" framing is
the description of the surviving case, not an explicit branch anywhere in the code** (and
not a rule stated in DESIGN.md either — the roadmap's `DESIGN.md:372` citation for it points
at no such text). Live-probed: a `bool` local (an ordinary, payload-free, structurally-`Copy`
enum since S3i) captured into an escaping closure hits this exact same rejection —

```
import: intrinsics * ;  import: core::prelude * ;
: mk ( Bool -- [ -- Bool ] ) | b | [ b ] ;
: main ( -- ) true mk call ~[ 1 . ] ~[ 0 . ] if ;
\ error: an escaping closure captures `b`, a local of this frame, whose storage does not
\   survive the return (line 3)
```

— confirmed against `main`, and this is exactly the case `is_copy` (`check/builtins.rs:219`,
its `Type::Enum` arm folds structurally over variant fields) would call `Copy`: `bool` has no
fields to be linear in. So **the classifier's own doc comment already names the narrower,
long-standing gap** (a by-value aggregate *parameter*/global is conservatively treated the
same as a locally-constructed one, `captures.rs:145-157`) but the `bool`-via-S3i case is a
second, previously unnamed instance of the same root cause: a blanket `Type::Enum(..) =>
FrameRooted` that never consults `is_copy`, so a payload-free enum is over-rejected exactly
like a genuinely linear one. Both are the same fix: `classify_capture`'s aggregate arm needs
an `is_copy` check, admitting a `Copy` aggregate/enum as `Scalar`-equivalent (snapshot, no
disposal obligation) rather than `FrameRooted`.

**A 2+-capture escaping closure is rejected today regardless of what it captures — even two
plain scalars — and this is deliberate, not incidental,** per `multi_capture_escaping_error`
(`captures.rs:76-83`, "R18: ... a heap env is deferred") and live-probed:

```
: mk ( i64 i64 -- [ -- i64 ] ) | a b | [ a b add ] ;
\ error: an escaping closure may capture at most one reference (a heap env is deferred)
```

R18's own comment also names a review fix firing "at R22 when a 2+-capture closure's
stack-allocated env bundle (R16) escapes transitively through a returned carrier" — so the
lowering-side stack bundle (`build_env`'s `many` arm, `src/ir/func_builder/quotation.rs:97-113`,
which `Alloc`s a bundle sized to the capture count via `push_alloc`, `mod.rs:452-462`, always
into the *current* frame's own alloca home) is **checker-gated off from ever reaching an
escaping boundary today.** This is not a live use-after-return bug waiting to be triggered —
R18/R22 already block every path into it. It is exactly the deferred case this slice must
build a real mechanism for, not a defect to patch around.

**Disposal never touches a closure's env, confirmed by absence, not merely by prior note.**
`emit_drop` (`quotation.rs:305-329`) — the sole call-site-selected disposal dispatch, matching
`IrType::OwnedCell`/`Struct`/`Enum`/`Array`, with a silent `_ => {}` fallback covering
`IrType::Quotation`/`Ptr` — and `synthesize_aggregate_destructors` (`src/ir/destructors.rs:37-95`)
— which only ever walks `Registries::structs`/`enums`/`cells` — both have zero code paths that
could reach a closure's env; grep-confirmed no `closure`/`env`(-as-capture) hits in either
file. There is no env struct type minted anywhere, checker or lowering: `EnvCapture`
(`func_builder/mod.rs:138-...`, one per capture: `name`, `ty`, `ref_mutable`, `referent`) and
the checker's `SurvivingCapture` (`check.rs:220-224`, `{name, frame_rooted}`) both record
per-capture facts as flat lists, never as a single aggregate type. `build_env`'s ≥2-capture
bundle is an untyped, positionally-`FieldStore`d stack blob (`quotation.rs:97-113`) — the
per-capture type information exists (`EnvCapture.ty`) but is never reified into an `IrType`
anything downstream can hang a destructor on.

## Existing precedent (what's already there to build on)

**`QuotLayout` was deliberately designed to grow.** `quotation_layout` (`src/ir/types.rs:216-238`,
comment: "the `env` slot is always the null pointer in 7a (7b fills it); it is not elided,
so widening to a capturing closure stays additive") already anticipates exactly the widening
this slice needs. A third word costs nothing new in kind — it is the same kind of addition
`env` itself already was.

**The disposal mechanism this needs already exists, twice over, and needs no new dispatch
logic — only a value to point it at.** `synthesize_cell_destructor` (`destructors.rs:453-486`)
already does precisely "free a heap block, disposing its payload first if linear" for *any*
payload type: `load_owned_payload` copies the payload out if `field_is_linear`, `FREE_SYMBOL`
frees the block, then `b.emit_drop(payload)` disposes it — generic over whatever `payload_ty`
is. `emit_drop` itself is already a plain, statically-resolved `Instr::Call` to a symbol name
derived from the value's compile-time-known `IrType` (`quotation.rs:305-329`) — never a
runtime type-tag read, never a vtable load. Nothing in the destructor-synthesis path is
runtime-dynamic today; the only new capability needed is a function pointer *value*, decided
once at construction and carried in the aggregate, the same way `code` (a `FuncAddr`,
`quotation.rs:79`) already is.

**Minting a per-literal struct type must happen at check time, not lowering.**
`FuncBuilder::structs: &'a Structs` (`func_builder/mod.rs:160`) is an immutable borrow —
lowering runs after every `StructDecl` is finalized, so a closure literal's env struct
cannot be minted inside `materialize_quot_value`/`build_env` as written today. The natural
site is check time, alongside `check_capture_admission`'s own per-literal capture-set
computation (`captures.rs:196-247`), mirroring how a *generic instantiation*'s concrete
`StructDecl` is minted by the checker (`instantiate_struct`, `ast.rs:817`), never by
lowering — lowering only ever reads an already-finalized registry.

## Design ruled here — SUPERSEDED by `slice3h-spec.md`

> **Everything from this heading to the end of this file is superseded.** The delivery plan in
> `slice3h-spec.md` replaced the mechanism described below. It does **not** build a per-closure
> destructor pointer, a three-word closure value, a widened `QuotLayout`, a new `emit_drop`
> arm, or a check-time per-literal env `StructDecl` minting pass. Instead the closure value
> stays two words, a type-level marker (`Type::OwningQuotation`) makes it linear and
> must-call, `call` is its consuming use, and the compiled body disposes its own captures and
> frees its own env. An owning closure is rejected in every aggregate position, which is what
> removes the need for any glue-reachable disposer.
>
> The problem statement above (the classifier, the one blocking gate, the live diagnostics) is
> still accurate and is what the spec builds on. The trait-object framing below is no longer
> "deferred indefinitely": it is scheduled as **P7.S3u**, with **P7.S3v** as the consumer that
> lifts this slice's aggregate-position and unexecuted-discard restrictions.

**A `Drop`-as-trait-object framing was considered and explicitly deferred, not adopted.** A
materialized quotation's `(code, env)` shape is already structurally a one-method trait
object (the roadmap doc's own phrasing); a per-closure destructor pointer is a hand-rolled
instance of `dyn Drop`. Real trait objects don't exist (S3e deferred them, no consumer forced
the question) and speccing them now to serve one consumer would be exactly the premature
generalization the project's growth conventions warn against. **Ruling: build the concrete
three-word mechanism below; note in the spec that it is what `dyn Drop` would give for free
later, so a future trait-object spec has a known precedent to fold in, not reconcile
against — do not design trait objects as part of this slice.**

**`QuotLayout` grows to three words: `{ code: Ptr, drop: Ptr, env: Ptr }`**, fixed offsets
`0`, `word_width`, `2*word_width`, size `3*word_width`. `code` and every existing consumer of
it are unchanged.

**Construction, decided once per literal at `materialize_quot_value`, never at the drop
site:**

- No captures, or every capture is `Scalar`-classified (today's only reachable case, widened
  by the `is_copy` fix above to include a `Copy` aggregate/enum snapshot): `env` built exactly
  as today; `drop = null`. Zero new runtime cost for every program that exists today.
- Any capture needing disposal (linear, or — once R18 is lifted — a `Copy` aggregate/enum
  that still prefers heap storage for a 2+-capture bundle): check time mints a fresh
  per-literal `StructDecl` (one field per capture, named/typed from that literal's
  `SurvivingCapture` set) and interns it through `intern_owned_cell_type`
  (`ast.rs:1015-1024`, which already dedupes structurally and needs only a `Type::Struct`
  payload — no new interning mechanism). Lowering allocates that cell (this *is* the fix for
  R18/R22's deferred heap-env case, and it retires the stack-bundle shape entirely for the
  escaping path — the stack bundle stays as-is for the still-legal **in-frame** 2+-capture
  case, since it never escapes and needs no disposal decision at that boundary). `env` points
  at the cell; `drop` is set to the address of that cell's already-synthesized
  `cell_drop_symbol` (`layout.rs:142`) — a compile-time constant, chosen exactly the way
  `code`'s `FuncAddr` already is, not discovered at the drop site.

**At the drop site, `emit_drop` gains one new arm, matching the existing shape of every
other arm exactly:**

```rust
IrType::Quotation(_) => {
    // load `drop` word at offset word_width; if non-null, indirect-call it
    // with `env`; a null `drop` (the universal case today) is a no-op.
}
```

No vtable, no runtime type inspection: the pointer being called was chosen once, statically,
at construction, from a small enumerable set of already-existing destructor symbols — the
same shape `code` already has. This is additive over every other `emit_drop` arm; none of
them change.

**Exactly-once and "no hidden control flow" (DESIGN.md:135-154) are unaffected**, since both
are checker-side properties (about *when* `drop` runs, enforced by linear-use tracking) —
the indirection only affects *which* already-statically-chosen body runs, not whether or when
it runs, or whether the call is visible at the `drop` call site the programmer wrote.

**The R18/R22 deferral lifts for the escaping path specifically because the heap-owned env
now exists to make it sound** — a 2+-capture (or any-capture-needing-disposal) escaping
closure's env is no longer a stack-frame `Alloc`, so the transitive-escape hazard R22 exists
to catch no longer applies to it. The in-frame single-boundary case (`escaping: false`) keeps
today's stack bundle unchanged; only the escaping/heap-env path changes.

## Sizing

Two phases: the checker-side over-rejection fix is independently valuable and independently
testable without the disposal mechanism; the disposal mechanism depends on nothing from the
first phase except a wider set of literals actually reaching it.

**Phase 1 — the classifier fix, no new runtime mechanism.**

- `classify_capture`'s aggregate arm gains an `is_copy` check (needs `structs`/`enums`/`arrays`
  registries threaded in, mirroring `is_copy`'s own signature, `check/builtins.rs:219`):
  `Copy` → treat as `Scalar` (admit everywhere, snapshot, no surviving-set entry, no
  disposal obligation); not `Copy` → keep today's `FrameRooted`/`OuterRooted` split unchanged.
- Regression tests: the `bool`-capture case above now admits; the existing linear/aggregate
  rejection (`tests/phase4_quotations.rs`'s `escaping_closure_over_frame_local_is_past_owning_frame`)
  stays rejected unchanged (still not `Copy` in that fixture); a new `Copy` struct/array
  capture (e.g. `[i64 4]`, matching the existing test's own shape but `Copy`) now admits.
- The R15 case-2 by-value-parameter/global narrowing (`captures.rs:145-157`) stays exactly
  as conservative as it is today — out of scope here, named only as a pre-existing,
  independent gap the code comment already flags; do not fold it in.

**Phase 2 — the heap-owned env, the destructor pointer, and lifting R18/R22 for the escaping
path.**

- Check-time: mint a per-literal `StructDecl` from the surviving capture set at a
  materialization boundary that needs one (not every one — only escaping closures whose
  surviving set is non-empty after phase 1's widening); wire it through `intern_owned_cell_type`.
- Lowering: extend `QuotLayout` to three words; `materialize_quot_value`/`build_env` build the
  cell and set `drop` to the cell's destructor symbol address, or `null` for the no-disposal
  case; `emit_drop` gains the `Quotation` arm.
- Lift `multi_capture_escaping_error`/its R22 transitive-escape twin for the now-sound
  heap-env path; the in-frame stack-bundle path (`escaping: false`) is unchanged.
- Regression tests: a linear local moved into an escaping closure, returned, called, and
  dropped disposes the captured value exactly once (assert via a `Spy`-style forced-linear
  fixture with an observable `drop` side effect, mirroring the pattern the live-probe pass
  used); a leaked or double-disposed capture is a located checker error, not a silent
  miscompile (the existing linear-use-tracking machinery should already produce this once the
  capture is admitted — confirm by test, do not assume); a 2+-linear-capture escaping closure
  builds, runs, and disposes both exactly once; the no-capture and scalar-only paths are
  unchanged (regression, not new behavior) and still cost nothing.
- Out of scope, unchanged: capturing an already-quotation-typed name by value
  (`captured_quotation_name_deferred_error`, `captures.rs:87-94`, its own, separately named
  deferred case, needing a nested two-word env slot this slice's design does not add); the
  REPL's inability to link a materialized quotation at all (`__quot0` non-PIC relocation,
  pre-existing, unrelated to disposal, confirmed still live by direct probe;
  `lower_instantiation`'s REPL path stays out of scope for this mechanism, matching the
  existing bypass convention for operator/trait-call overload dispatch); trait objects /
  `dyn Drop` (named above as the thing this mechanism resembles, not a target).

## Ready to spec: yes, with two instructions for spec-writer

1. Verify every citation above against live `main` before writing — `captures.rs`,
   `quotation.rs`, and `destructors.rs` line numbers move as other in-flight slices land.
2. The check-time struct-minting mechanism (exactly which pass, exactly how it threads
   `&mut Vec<StructDecl>` alongside `check_capture_admission` without disturbing its existing
   signature or the borrow shape of its callers) is an open implementation question the spec
   must resolve with a concrete mechanism, not a restated requirement — this brief establishes
   *that* it must happen at check time and *why* (lowering's registries are immutable by then),
   not the exact call-site plumbing.
