# P7.S3h spec — An escaping closure may capture a linear value (closure env disposal)

Delivery plan. Read `docs/roadmap/P7/slice3h-brief.md` first: it holds the confirmed
root cause, the live-probed diagnostics, and the design this spec commits to. Every
`path:line` anchor below was re-verified against live `main` (`71de84a`);
`captures.rs`/`quotation.rs`/`func_builder/mod.rs`/`types.rs`/`destructors.rs` drift as
other slices land, so line numbers are anchors to re-confirm at implementation, not
contracts.

The brief left exactly one design question open (the check-time struct-minting
mechanism: which pass mints a per-closure-literal env type for its surviving capture
set, and how it acquires a mutable struct/cell registry without changing
`check_capture_admission`'s signature or breaking its callers' borrow shape). It is
resolved below in "Resolved: where the env type is minted", grounded in the existing
generic-instantiation / output-bundle precedent, not restated as a requirement.

## Problem

Capture into an escaping (materialized, non-spliced) closure is gated by
`check_capture_admission` (`src/check/captures.rs:202`), reached from
`materialize_quotation_at_boundary` (`captures.rs:306`) only when the literal actually
captures (`body_captures_enclosing`, `captures.rs:12`). `classify_capture`
(`captures.rs:144`) buckets every captured name by its `Binding.ty` alone:

- `Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)` → always
  `FrameRooted` (`captures.rs:162`), unconditionally — **not gated on linearity or
  `is_copy`.**
- `Type::Ref(..) | Type::Slice(..)` → `FrameRooted`/`OuterRooted` per
  `ref_root_is_in_frame` (`captures.rs:126`, `174`).
- Anything else → `Scalar` (`captures.rs:185`), snapshotted into the env (D4), admitted
  everywhere.

A `FrameRooted` capture at an escaping boundary is rejected (`captures.rs:249`,
`past_owning_frame_error`, `:49`). This is not a linearity check. DESIGN.md:372's
"restricted to `Copy`" framing describes the surviving case, not a branch that exists:
a `bool` local (a payload-free, structurally-`Copy` enum since S3i) captured into an
escaping closure hits the exact same rejection —

```sooth
import: intrinsics * ;  import: core::prelude * ;
: mk ( Bool -- [ -- Bool ] ) | b | [ b ] ;
: main ( -- ) true mk call ~[ 1 . ] ~[ 0 . ] if ;
\ error: an escaping closure captures `b`, a local of this frame, whose storage does not
\   survive the return (line 3)
```

`is_copy` (`check/builtins.rs:219`, its `Type::Enum` arm folds structurally over variant
fields) calls `bool` `Copy` — no fields to be linear in — so the blanket
`Type::Enum(..) => FrameRooted` over-rejects a `Copy` enum exactly like a genuinely
linear one. Same root cause as the code comment's own named narrowing (a by-value
aggregate parameter/global treated like a locally-constructed one, `captures.rs:145-160`),
which stays out of scope here.

A 2+-capture escaping closure is rejected regardless of what it captures, deliberately
(`multi_capture_escaping_error`, `captures.rs:76`, "R18: ... a heap env is deferred"),
and its R22 transitive-escape twin (the review-fix clause in the same doc comment) blocks
the stack bundle (`build_env`'s `many` arm, `quotation.rs:105-113`, an `Alloc` into the
current frame via `push_alloc`, `func_builder/mod.rs:452`) from ever reaching an escaping
boundary. This is the deferred case, not a live use-after-return bug.

Disposal never touches a closure's env, confirmed by absence: `emit_drop`
(`quotation.rs:305`, the sole disposal dispatch) matches `IrType::OwnedCell`/`Struct`/
`Enum`/`Array` with a silent `_ => {}` covering `Quotation`/`Ptr`;
`synthesize_aggregate_destructors` (`destructors.rs:37`) walks only
`structs`/`enums`/`cells`. No env struct type is minted anywhere: `EnvCapture`
(`func_builder/mod.rs:138`: `name`, `ty`, `ref_mutable`, `referent`) and the checker's
`SurvivingCapture` (`check.rs:220`: `{name, frame_rooted}`) both record per-capture facts
as flat lists, never as an aggregate `IrType` a destructor can hang on.

## Design (from the brief; not re-derived)

Trait objects / `dyn Drop` are **not** built here. A materialized quotation's
`(code, env)` shape is structurally a one-method trait object and a per-closure
destructor pointer is a hand-rolled `dyn Drop`, but real trait objects were deferred
(S3e) with no consumer forcing them, and speccing them for one consumer is the premature
generalization the growth conventions warn against. Build the concrete three-word
mechanism below; note that a later trait-object spec has this as a precedent to fold in.

**`QuotLayout` grows to three words: `{ code: Ptr@0, drop: Ptr@word_width, env:
Ptr@2*word_width }`**, size `3*word_width`. `quotation_layout` (`src/ir/types.rs:232`,
whose own comment already anticipates "widening to a capturing closure stays additive")
gains a `drop_offset` field. `code` and every existing consumer are unchanged.

**Construction, decided once per literal at `materialize_quot_value`
(`quotation.rs:48`), never at the drop site:**

- No captures, or every capture is `Scalar`-classified (today's only reachable case,
  widened by phase 1 to include a `Copy` aggregate/enum snapshot): `env` built exactly
  as today; `drop = null`. Zero new runtime cost for every program that exists today.
- Any capture needing disposal (linear, or a 2+-capture bundle on the escaping path):
  the env is a heap cell, `drop` is the constant address of that cell's
  `cell_drop_symbol` (`layout.rs:142`), chosen the way `code`'s `FuncAddr`
  (`quotation.rs:82`) already is.

**At the drop site, `emit_drop` gains one `IrType::Quotation(_)` arm**, matching the
shape of every other arm: load `drop` at `drop_offset`; if non-null, indirect-call it
with `env`; a null `drop` is a no-op. No vtable, no runtime type inspection — the pointer
was chosen once, statically, from the enumerable set of already-synthesized cell
destructor symbols. Additive over every other arm; none change.

Exactly-once and "no hidden control flow" (DESIGN.md:135-154) are unaffected: both are
checker-side properties about *when* `drop` runs (linear-use tracking), and the
indirection only affects *which* already-statically-chosen body runs.

The R18/R22 deferral lifts for the escaping path because the heap-owned env makes it
sound: an escaping closure's env is no longer a stack-frame `Alloc`, so R22's
transitive-escape hazard no longer applies. The in-frame single-boundary case
(`escaping: false`) keeps today's stack bundle unchanged.

## Resolved: where the env type is minted

The brief establishes *that* the per-literal env type must be minted at check time (by
materialization, lowering's `structs`/`cells` registries are already finalized:
`FuncBuilder::structs: &'a Structs` is an immutable borrow, `func_builder/mod.rs:160`)
and *why*. This spec resolves the plumbing.

**Ruling: mint in a post-body-walk pass in `check::check`, not inside
`check_capture_admission` and not inside `materialize_quotation_at_boundary`.** This
mirrors an existing precedent exactly: multi-output return bundles are interned into
`module.structs` *after* the per-word walk (`intern_output_bundles` +
`for inst in insts.values_mut() { inst.bundle = Some(intern_bundle_struct(&mut
module.structs, &inst.output_types)) }`, `check.rs:970-985`), and generic instantiations
mint into a `RefCell<GenericTypes>` scratch buffer during the walk and
`flush_structs_into`/`flush_enums_into` the module registries once nothing is still
minting (`check.rs:940-942`). Both avoid the `&mut module.structs`-vs-`Ctx`'s immutable
`&[StructDecl]` (`check/engine.rs:1173`) aliasing conflict by minting when no `Ctx` borrow
is live.

Concretely:

1. **`check_capture_admission`'s signature is unchanged.** It gains nothing; it already
   reads each captured name's `Binding` at the site it classifies it
   (`captures.rs:242`, `scope.local(name)`). The one change inside it is that
   `SurvivingCapture` carries the capture's `Type` (read from that same `Binding.ty`)
   alongside `name`/`frame_rooted`, so the post-walk pass has the field types without
   re-deriving them. This touches the struct's construction sites in `captures.rs` only
   (`:252`, `:257`), not the function's parameter list, so no caller's borrow shape moves.
2. **The post-walk pass** (new, called from `check.rs` immediately after the output-bundle
   loop, with `&mut module.structs` and `&mut module.owned_cells` and no `Ctx` alive)
   iterates the escaping, disposal-needing surviving sets recorded during the walk. For
   each it mints a fresh per-literal env `StructDecl` (one `(name, ty)` field per surviving
   member, in the body's sorted capture order), structurally deduped by field-type tuple,
   then wraps it via `intern_owned_cell_type(cells, Type::Struct(id))` (`ast.rs:1015`,
   already dedupes structurally, needs only a `Type::Struct` payload). The minted struct
   is **not** flagged `is_bundle` (unlike `intern_bundle_struct`, `ast.rs:1179`, whose
   flag suppresses destructor synthesis): the env's whole purpose is that the layout pass
   computes its `is_linear` and the cell's `synthesize_cell_destructor`
   (`destructors.rs:453`) disposes any linear field. The resulting `OwnedCellId` is
   recorded on a new `Module` map keyed by the materialized quotation's symbol
   (`{word}__quot{n}`, the stable cross-phase identity `materialize_quot_value` already
   dedups on, `quotation.rs:50`/`:54` — the per-word check `QuotId` and the per-function
   lowering `QuotId` do not coincide, so the symbol string is the correct key, not either
   id).
3. **Lowering reads that map read-only** in `materialize_quot_value`: on a hit, build the
   cell (`Alloc` + per-field `FieldStore`), set `env` to it and `drop` to
   `cell_drop_symbol(id, self.cells.drop_generations[id.index()])` (the same
   generation-qualified symbol `emit_drop`'s existing `OwnedCell` arm uses,
   `quotation.rs:311`); on a miss, today's inline/null `env` and `drop = null`.

This deviates from the brief's "mint at `materialize_quot_value`" phrasing only in *when*
within check time (post-walk, not mid-walk), for the borrow reason above; it is minted at
check time and interned through `intern_owned_cell_type` exactly as ruled.

## What ships

**Phase 1 — the classifier fix, no new runtime mechanism.**

- `classify_capture`'s aggregate arm (`captures.rs:162`) gains an `is_copy` check. It needs
  `structs`/`enums`/`arrays` threaded in to match `is_copy`'s signature
  (`check/builtins.rs:219`). These reach `classify_capture` from
  `check_capture_admission`'s `ctx` (`ctx.structs()`/`ctx.enums()`) plus the `arrays`
  registry, which `materialize_quotation_at_boundary` already holds (`captures.rs:295`)
  and threads down — so this is an internal helper-signature change, **not** a change to
  `check_capture_admission`'s public signature. `Copy` → treat as `Scalar` (admit
  everywhere, snapshot, no surviving-set member, no disposal obligation); not `Copy` →
  keep today's `FrameRooted`/`OuterRooted` split unchanged.
- The R15 case-2 by-value-parameter/global narrowing (`captures.rs:145-160`) is untouched,
  named only as the pre-existing independent gap the code comment already flags.

**Phase 2 — the heap-owned env, the destructor pointer, R18/R22 lift.**

- `SurvivingCapture` (`check.rs:220`) carries the capture `Type`.
- The post-walk minting pass and the new `Module` symbol→`OwnedCellId` map (see
  "Resolved" above).
- `QuotLayout` (`types.rs:225`) grows a `drop_offset` word; `quotation_layout`
  (`types.rs:232`) fills it; `size` becomes `3 * word_width`.
- `materialize_quot_value` (`quotation.rs:48`) sets `drop` from the map (or null) and
  builds the heap cell for a disposal-needing literal.
- `emit_drop` (`quotation.rs:305`) gains the `IrType::Quotation(_)` arm.
- `multi_capture_escaping_error` (`captures.rs:76`) and its R22 transitive twin lift for
  the escaping path; the in-frame stack-bundle path (`escaping: false`) is unchanged.

## Out of scope (unchanged)

- The R15 case-2 aggregate-parameter/global narrowing (`captures.rs:145-160`).
- Capturing an already-quotation-typed name by value
  (`captured_quotation_name_deferred_error`, `captures.rs:87`, its own deferred case,
  needing a nested two-word env slot this design does not add).
- The REPL's inability to link a materialized quotation at all (`__quot0` non-PIC
  relocation, pre-existing, unrelated to disposal; `lower_instantiation`'s REPL path
  stays out, matching the operator/trait-call overload-dispatch bypass convention).
- Trait objects / `dyn Drop`.

## Invariants to preserve

- `check_capture_admission`'s parameter list does not change; the minting pass runs after
  the body walk, when no `Ctx` immutable borrow of `module.structs` is live.
- The env type is minted at check time and interned through `intern_owned_cell_type`;
  lowering only reads finalized registries and the symbol→cell map.
- A no-capture or scalar-only literal keeps today's `env` and a null `drop`: byte-for-byte
  the current shape, zero new runtime cost.
- `code` at offset 0 and every existing consumer of it are unchanged by the layout
  widening; only `env` moves (to `2*word_width`) and `drop` is added.
- The env struct is minted with `is_bundle: false` so the cell destructor is synthesized;
  do not reuse `intern_bundle_struct`, whose flag suppresses it.
- The map key is the `{word}__quot{n}` symbol, not a `QuotId`: check and lowering number
  quotations differently.
- `emit_drop`'s new arm is additive; no other arm changes.

## Tests

`tests/phase7_slice3h.rs` (single file, matching every landed P7.S3 slice) plus unit
tests beside each touched function.

Phase 1: the `bool`-capture program above now admits (regression golden); a new `Copy`
struct/array capture (e.g. a `[i64 4]` matching the existing test's shape but `Copy`) now
admits; the existing linear/aggregate rejection
(`phase4_quotations.rs::escaping_closure_over_frame_local_is_past_owning_frame`) stays
rejected unchanged (still not `Copy`); a unit test on `classify_capture` pinning that a
`Copy` aggregate takes the new `Scalar` branch and a linear one keeps `FrameRooted`
(mutation-test the `is_copy` call: deleting it must fail the linear-capture case).

Phase 2: a linear local moved into an escaping closure, returned, called, and dropped
disposes the captured value exactly once (assert via a forced-linear `Spy`-style fixture
with an observable `drop` side effect, mirroring the live-probe pass); a 2+-linear-capture
escaping closure builds, runs, and disposes both exactly once; a leaked or double-disposed
capture is a located checker error, not a silent miscompile (confirm the existing
linear-use-tracking machinery produces this once the capture is admitted — test, do not
assume); the no-capture and scalar-only paths keep a null `drop` (assert on the IL: one
`Alloc` of `3*word_width`, `drop` slot stored null); the layout widening's `code` offset
is unchanged (a pre-existing `code`-slot golden must pass through the new layout).
Mutation-test the new `emit_drop` arm (a stubbed null-check must let a
double-free/miscompile fixture fail) and the minting pass's `is_bundle: false`
(flipping it to `true` must drop the destructor and fail the exactly-once fixture).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Checker: classify_capture's aggregate arm gains an is_copy check (structs/enums/arrays threaded through the helper chain, not through check_capture_admission's public signature), so a Copy aggregate/enum capture classifies as Scalar and admits at an escaping boundary; regression and unit coverage, no new runtime mechanism, no IL change", "difficulty": "standard" },
    { "phase": 2, "focus": "Heap-owned closure env and per-closure destructor pointer: SurvivingCapture carries the capture Type; a post-body-walk pass in check mints a per-literal env StructDecl (is_bundle false) and interns an owned cell via intern_owned_cell_type, recording OwnedCellId in a new Module symbol->cell map; QuotLayout grows a drop word; materialize_quot_value builds the cell and sets drop to cell_drop_symbol or null; emit_drop gains the Quotation arm; R18/R22 lift for the escaping path only; run/IL and exactly-once disposal goldens", "difficulty": "hard" }
  ]
}
```
