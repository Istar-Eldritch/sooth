# P7.S3h spec — An escaping closure may capture a linear value (closure env disposal)

Delivery plan. Read `docs/roadmap/P7/slice3h-brief.md` first: it holds the confirmed root
cause and the live-probed diagnostics. Every `path:line` anchor below was re-verified
against live `main` (`271c983`); `captures.rs`/`quotation.rs`/`func_builder/mod.rs`/
`types.rs`/`ast.rs`/`check.rs` drift as other slices land, so line numbers are anchors to
re-confirm at implementation, not contracts.

**Revision 4.** Three earlier drafts were each BLOCKed by fresh-context reviewers. This
revision replaces their disposal mechanism outright, so most of their findings are
*superseded* rather than folded: the mechanism they were finding holes in no longer exists.
What those drafts got wrong at the root, and what this one deletes as a result, is in
"The body is the disposer" below. The findings that survived the redesign are folded and
marked where they land: the `arrays` threading correction and the `phase4_quotations.rs:336`
test flip (Phase 1), the per-site audit polarity (its own section), and the `ir_type_of`
ICE (Phase 2).

## Problem

Capture into an escaping (materialized, non-spliced) closure is gated by
`check_capture_admission` (`src/check/captures.rs:202`), reached from
`materialize_quotation_at_boundary` (`captures.rs:287`) only when the literal actually
captures (`body_captures_enclosing`, `captures.rs:12`). `classify_capture`
(`captures.rs:144`) buckets every captured name by its `Binding.ty` alone:

- `Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)` → always
  `FrameRooted` (`captures.rs:162`), unconditionally — **not gated on linearity or
  `is_copy`.**
- `Type::Ref(..) | Type::Slice(..)` → `FrameRooted`/`OuterRooted` per `ref_root_is_in_frame`
  (`captures.rs:126`, `174`).
- Anything else → `Scalar` (`captures.rs:182`), snapshotted into the env (D4), admitted
  everywhere.

A `FrameRooted` capture at an escaping boundary is rejected (`captures.rs:249`,
`past_owning_frame_error`, `:49`). This is not a linearity check, and DESIGN.md:372's
"restricted to `Copy`" framing describes the surviving case rather than a branch that
exists: a `bool` local (a payload-free, structurally-`Copy` enum since S3i) hits the same
rejection, live-probed —

```sooth
import: intrinsics * ;  import: core::prelude * ;
: mk ( Bool -- [ -- Bool ] ) | b | [ b ] ;
: main ( -- ) true mk call ~[ 1 . ] ~[ 0 . ] if ;
\ error: an escaping closure captures `b`, a local of this frame, whose storage does not
\   survive the return (line 3)
```

`is_copy` (`builtins.rs:219`, its `Type::Enum` arm folding structurally over variant
fields) calls `bool` `Copy` — no fields to be linear in — so the blanket
`Type::Enum(..) => FrameRooted` over-rejects a `Copy` enum exactly like a linear one. Same
root cause as the code comment's own named narrowing (a by-value aggregate parameter/global
treated like a locally-constructed one, `captures.rs:145-160`), which stays out of scope.

A 2+-capture escaping closure is separately rejected regardless of what it captures
(`multi_capture_escaping_error`, `captures.rs:76`, "R18: … a heap env is deferred"), and its
R22 transitive-escape twin blocks the stack bundle (`build_env`'s `many` arm,
`quotation.rs:97-113`, an `Alloc` into the current frame via `push_alloc`,
`func_builder/mod.rs:452`) from reaching an escaping boundary. Deferred case, not a live
use-after-return bug.

### What the probes establish (the shape of the actual gap)

Three probes against live `main`, and they narrow the work sharply:

1. **A quotation body consuming a captured linear value already works.** Two `if` arms each
   spliced `~[ s drop ]` over a linear `Spy` builds and runs. So "the body disposes what it
   holds" is not a new capability — it is how a *spliced* body already behaves.
2. **The must-consume-on-every-path discipline already exists**, with the diagnostic this
   slice needs. One arm consuming and one not gives:

   ```text
   error: linear value `s` is not consumed on every path in `main` (line 10)
     `s` has type `Spy`, which is linear: it is consumed on one `if` arm but not the
     other, so drop it (or return it) on every path
   ```
3. **Exactly one gate blocks the target shape.** `: mk ( Spy -- [ -- ] ) | s | [ s drop ] ;`
   fails with only `past_owning_frame_error` — "an escaping closure captures `s`, a local of
   this frame, whose storage does not survive the return". Not a disposal error, not a
   linearity error: a **storage-lifetime** error.

So the gap is not "closures cannot dispose things". It is "an escaping closure's env has no
storage that outlives the frame". That is the whole slice.

## Design

### The obligation is in the type; the storage is not

Two axes were conflated in earlier drafts:

- **Does this closure owe a disposal?** This *must* be in the type. A consumer discharges
  the obligation, and it cannot see the captures: `Type::Quotation(&'static QuotEffect)` is
  keyed by declared effect only, deliberately, which is what lets combinators exist at all.
  Every consumer in `lib/combinators.sth` — `times` (`:40`), `each` (`:43`), `map` (`:48`),
  `fold` (`:53`), `filter` (`:66`), `while` (`:79`) — declares a quotation parameter's type
  without knowing what a caller closed over, as does the materialized case
  `dispatch ( Vm [ [ Vm -- Vm ] 9 ] -- i64 )` (`examples/vm_table.sth:219`). Putting the
  capture set in the type would make every one of those signatures unwritable.
- **Where does the env live?** This must *not* be in the type, or a combinator could only
  accept closures whose storage it happens to name. Inline, static (P7.S2 landed statics and
  global sets), and heap are all legitimate; the type is silent on which.

**So: a marker in the type, naming the obligation and nothing else.** Add
`Type::OwningQuotation(&'static QuotEffect)` (same payload as `Type::Quotation`), with
`is_copy` returning `false` for it (so `is_linear` is `true`), while plain `Type::Quotation`
stays `Copy` exactly as today. The precedent is `Type::InlineQuotation` (`ast.rs:1989`,
slice 10a): same payload, split purely to carry a capability difference, its own doc comment
naming the mechanism this reuses — "structural `PartialEq` gives
`InlineQuotation(e) != Quotation(e)` for free, so every materialization boundary rejects a
`~` by type inequality before the boundary". Mirror its shape, including a distinct
`name_static` spelling so the two render distinctly (`inline_quotation_type`, `ast.rs:2020`).

### The body is the disposer, so the closure must be called

An owning closure is a word with its arguments already supplied. A word taking a linear
argument needs no disposal metadata: it is always called, and its body consumes the
argument. A closure differs in exactly one way — it might never be called. Close that gap
by requiring it:

- **The owning closure value is linear** (from the marker), so it must be consumed on every
  path. That machinery exists and is probe-confirmed (probe 2).
- **`call` is its consuming use.** `call` already pops its receiver and never re-pushes it
  (`terms.rs:311`, routed at `:322-327` to `check_abstract_quotation_call`, `:1123-1146`,
  which pops `eff.inputs`, pushes `eff.outputs`, and leaves no closure behind). So the
  checker needs *no* change here: the pop already discharges the obligation.
- **`drop` on an owning closure is a located rejection.** Dropping it would leak the
  capture, because the compiler cannot run the body's disposal without running the body.
  The diagnostic names the closure, says it owns captured values, and gives the remedy
  (call it, or pass it on).
- **The body consumes its captures**, exactly as a word body consumes a linear parameter.
  Probe 1 shows a spliced body already does this.
- **The body also frees its own env.** The body is compiled per literal, so it is the only
  code that statically knows the env's layout — and it is now guaranteed to run exactly
  once. It disposes the captures and releases the env storage as its last act.

**What this deletes, versus every earlier draft:** no `drop` function pointer in the value,
no third slot, no widened `QuotLayout`, no new backend aggregate, no `emit_drop` arm, no
indirect dispose, no check-time per-literal env `StructDecl` minting pass, no
`OwningEnvRequest` carrier out of the per-word `Provenance`, no `Module` symbol→cell map, no
map threading into lowering, and no check/lower quotation-numbering invariant (which existed
only to key that map). The runtime cost of an owning closure is the env storage it needs
anyway; the marker itself costs nothing.

Exactly-once and "no hidden control flow" (DESIGN.md:135-154) hold without special pleading:
the type carries the obligation, the existing linear machinery enforces that the closure is
consumed on every path, and disposal is ordinary compiled body code at the point the
programmer wrote the call.

### Surface syntax: `owning [ … ]`, in type positions only

`owning` is an ordinary word in type position: no new token, no lexer change, no collision.

**`^[ … ]` is deliberately not used.** `^` means "heap cell" (`sooth_alloc`/`sooth_free`,
`ir/types.rs:31`/`:35`), i.e. it answers the *storage* question, which this type must stay
silent on — a `fixed`-layer program holding a linear value with an inline or static env
needs the same type as a heap one. It is also unavailable mechanically: `^[u8 64]` is live
owned-cell-of-array syntax (the QBE prelude's own `Buf`, `qbe.rs:2868`) and `lexer.rs:440`
(`lex_owning_cell_array_type_splits_at_bracket`) pins `^[` splitting into two tokens, so a
glued `CaretLBracket` would break both. (For the record, `^[ i64 -- ]` is *not* a competing
live meaning: it is a located rejection today, "a quotation type cannot appear as an
owned-cell payload", from the audit table at `phase4_combinators.rs:158`, whose message
still cites a pre-7a slice. Free, but not what we want.)

**Owningness is inferred at the literal; declared in the type.** A materialization boundary
already knows the capture set and each capture's type, and already checks the literal
against the boundary's declared effect (`check_literal_against_declared_effect`, step (iii)
of `materialize_quotation_at_boundary`). So there is **no term-level syntax**: a literal is
written `[ … ]` as today and its owningness is derived. A literal capturing a linear value
simply does not satisfy a plain `[ … ]` slot — a located type error naming `owning` as the
fix. Declared signatures must state it, for two reasons: a consumer is compiled once and
cannot straddle the linear and `Copy` checking regimes, and a declared effect is a checked
contract, so a signature that inferred owningness from a body would tell its caller the
result is forgettable when it is not.

### Env storage

The env must outlive the frame; that is the one thing probe 3 says is missing. **In slice:
heap**, via `intern_owned_cell_type` (`ast.rs:1015`, already structurally deduping) — the
general case, and DESIGN.md already places escaping closures in the `alloc` layer
(`:298-317`: "a non-escaping quotation is core but an escaping one is `alloc`"). The body
frees its own block (`FREE_SYMBOL`, as `synthesize_cell_destructor` already does at
`destructors.rs:476`), so no per-value metadata is needed.

**Inline and static envs are named as follow-on work, not designed here.** They are what
would let a `fixed`-layer (allocation-free) program own a linear capture, and the type is
deliberately silent on storage so they can land later without touching a single signature.
That also means DESIGN.md's flat "an escaping one is `alloc`" becomes too coarse once they
exist — a one-line amendment for its owner, not this slice.

## The per-site admit/reject audit

Adding a `Type` variant makes every non-wildcard `match` a hard build error until handled,
so the compiler enumerates the sites; what it cannot do is decide them. **Do not "mirror
slice 10a" blindly:** `InlineQuotation`'s answer at every materialization boundary is
*reject* (`ast.rs:1989`: "cannot be materialized … `ir_type_of` never sees one"), whereas
`OwningQuotation` must be *admitted* there and must reach the backend. Copying 10a's
polarity would be wrong at exactly the sites that matter. Rule on at least:

| Site | Answer |
| --- | --- |
| `is_quotation_type` (`ast.rs:2049`) | **Some** — or `call` breaks (`terms.rs:324`/`327` falls to `call_needs_quotation_error`) |
| materialization boundaries | **admit** (the point of the slice) |
| declaration positions (word param/output, struct field, array element) | **admit** where plain `Quotation` is admitted |
| `ir_type_of` (`types.rs:301`) | **real arm** in the representation phase; guarded check-side before it |
| capture-admission case-4 guard (`captures.rs:236`) | **defer**, see below |
| `extern:` boundary, reference referent, owned-cell payload | **reject**, as plain `Quotation` is |

One accessor pulls both ways: `is_quotation_type` must return `Some` for the `call` path,
and the same accessor backs the case-4 guard, so an owning-typed *name* capture is deferred
by `captured_quotation_name_deferred_error` (`captures.rs:87`). Acceptable and already out
of scope — but a stated consequence, not a discovery.

## What ships

### Phase 1 — the classifier fix, no new runtime mechanism

- `classify_capture`'s aggregate arm (`captures.rs:162`) gains an `is_copy` check. `Copy` →
  treat as `Scalar` (admit everywhere, snapshot, no surviving-set member, no disposal
  obligation); not `Copy` → keep today's `FrameRooted`/`OuterRooted` split.
- **Plumbing, stated honestly.** `is_copy(ty, structs, enums, arrays)` (`builtins.rs:219`)
  needs `arrays` for its `Type::Array` arm. `classify_capture` takes `(b, prov, scope)`
  (`captures.rs:144`) and is called only from `check_capture_admission`, which holds `ctx`;
  `Ctx` exposes `structs()`/`enums()` (`engine.rs:1171`/`1177`) but **no `arrays()`**, and
  `arrays` is held by `materialize_quotation_at_boundary` (`captures.rs:295`). So threading
  it **does** change `check_capture_admission`'s parameter list (`captures.rs:202`) — cheap
  (one caller, an immutable reborrow), and earlier drafts' claim that it stays internal was
  false.
- **Expected breakage, in scope.** `is_copy` makes `[i64 4]` **`Copy`**
  (`builtins.rs:243`), and `tests/phase4_quotations.rs:336`
  (`escaping_closure_over_frame_local_is_past_owning_frame`) captures exactly that shape by
  value (its sibling comment at `:353-355` calls it "case 2, the aggregate"). It flips from
  reject to admit and its `assert_eq` **will fail** — migrate it here. **Mandatory
  replacement witness:** a genuinely linear by-value aggregate (a `drop`-overloaded struct,
  hitting `builtins.rs:221`, or a `Type::OwnedCell`, hitting `:233`) still classifies
  `FrameRooted` and is still rejected, so the narrowed arm keeps coverage.
- The R15 case-2 parameter/global narrowing (`captures.rs:145-160`) is untouched.

No new runtime mechanism, no IL change.

### Phase 2 — the `OwningQuotation` type, syntax, classification

- `Type::OwningQuotation(&'static QuotEffect)` + `owning_quotation_type` mirroring
  `inline_quotation_type` (`ast.rs:2020`), with its own `name_static` spelling.
- `owning` accepted in type positions (word params/outputs, struct fields, array elements —
  wherever plain `Quotation` is legal). No term-level syntax; no lexer change.
- The per-site audit above.
- `is_copy` returns `false` for it (arm above the `_ => true` wildcard, `builtins.rs:245`);
  `is_linear` then `true` with no change of its own, and move tracking, the `dup` gate, the
  forgotten-value error, and the consumed-on-every-path check all inherit it.
- Boundary behaviour by structural inequality: a plain `[ … ]` literal does not satisfy an
  `owning` slot and vice versa; an `if`-join of an owning and a plain closure is an ordinary
  type mismatch. A literal that captures a linear value at a plain boundary is a located
  error naming `owning` as the remedy.
- **No representation yet, guarded check-side.** `ir_type_of` returns `IrType`, not
  `Result` (`types.rs:301`), and its `InlineQuotation` arm is `unreachable!()` (`:355`), so a
  "deferral" there would be an ICE. Phase 2 adds an explicit **check-side materialization
  guard** with a located diagnostic, distinct from the `ir_type_of` arm. The sharp case is a
  **non-capturing** owning literal: `body_captures_enclosing` is false, so it never reaches
  `check_capture_admission` and a capture-side guard would not block it.

### Phase 3 — env storage, the call-once lifecycle, disposal

- `IrType::OwningQuotation`, `ir_type_of`'s real arm, and the env's heap storage: the
  captured linear value is *moved* into the block (consumed from the frame, so
  `Scope::leave`'s unconsumed-local check, `engine.rs:544-551`, is satisfied), and the block
  outlives the return.
- The body consumes its captures and frees its own env block before returning.
- `call` as the consuming use — **no checker change** (`terms.rs:311` already pops), and the
  existing consumed-on-every-path check already forces a conditional to call it on both
  arms (probe 2).
- `drop` on an owning closure: a located rejection naming the remedy.
- Lift `past_owning_frame_error` (`captures.rs:49`) for a linear capture at an `owning`
  boundary — the one gate probe 3 identified — and `multi_capture_escaping_error`
  (`captures.rs:76`) with its R22 twin for the same path, now that the env is not a stack
  `Alloc`. The in-frame path (`escaping: false`) is unchanged.

## Out of scope

- The R15 case-2 aggregate-parameter/global narrowing (`captures.rs:145-160`).
- Capturing an already-quotation-typed name by value (`captured_quotation_name_deferred_error`,
  `captures.rs:87`).
- **Discarding an owning closure unexecuted.** `drop` on one is rejected; releasing its
  capture requires running the body. Permitting it would need a per-value disposal pointer
  (a hand-rolled one-method `dyn Drop`), which is a strict *widening* of this design if a
  consumer ever appears — not a redesign.
- **Inline and static env storage** (the `fixed`-layer, allocation-free case). The type is
  silent on storage precisely so these land without touching signatures.
- Polymorphism over plain vs owning quotation types. Consequence to record: no existing
  higher-order word declared over `[ … ]` can accept an `owning` closure, so a combinator
  wanting either must be written twice. The type inequality is the safety story; this is its
  cost.
- The REPL's inability to link a materialized quotation at all (`__quot0` non-PIC
  relocation, pre-existing). Note the checker-side lift is *not* REPL-bypassed, so a REPL
  owning closure will check-pass and then fail to link — enlarging an existing broken set
  rather than creating a new class.
- Trait objects / `dyn Drop`.

## Invariants to preserve

- Plain `Type::Quotation` stays `Copy`: no new obligation, no `dup` ban, no IL/golden churn
  for any program that exists today. `type :Q{n} = { l, l }` (`qbe.rs:151`) stays
  byte-identical.
- `is_linear` remains the single source of every exactly-once obligation; the marker adds no
  parallel machinery.
- `OwningQuotation(e) != Quotation(e)` structurally, so every boundary and `if`-join
  separates them by type inequality before lowering.
- The type names the obligation only. **Storage never appears in the type**, so inline and
  static envs can land later without touching a signature.
- The body is the sole disposer: it consumes its captures and frees its own env. No
  per-value disposal metadata exists, and nothing outside the body knows the env's layout.
- `call` is a consuming use and needs no checker change; `drop` on an owning closure is
  rejected.
- Owningness is inferred at literals and declared in types. No term-level `owning` syntax.

## Tests

`tests/phase7_slice3h.rs` (single file, matching every landed P7.S3 slice) plus unit tests
beside each touched function.

**Phase 1.** The `bool`-capture program above now admits; a `[i64 4]` capture now admits;
`phase4_quotations.rs:336` migrated to assert the admission; the mandatory linear-aggregate
witness (`drop`-overloaded struct or `OwnedCell`) still rejected. Unit test on
`classify_capture` for both branches. Mutation-test the `is_copy` call: deleting it must
fail the *linear-aggregate* case — note it would **not** fail the `[i64 4]` case, which is
why the replacement witness is the real guard.

**Phase 2 (type-level).** `owning [ … ]` parses in every admitted type position (unit).
`is_copy(OwningQuotation) == false`, `is_linear(OwningQuotation) == true`, and
`is_copy(Quotation) == true` unchanged (unit) — mutation-test the new `is_copy` arm by
deleting it and watching the linearity unit fail. `OwningQuotation(e) != Quotation(e)`
(unit). Goldens: a plain literal in an `owning` slot and an owning/plain `if`-join are
located type errors; a linear capture at a *plain* boundary names `owning` as the remedy; a
`dup` of an `owning`-typed binding is the ordinary linear `dup` rejection and forgetting one
is the ordinary forgotten-linear error (these prove the marker inherited move tracking with
zero new code); materializing an `owning` literal — **including the non-capturing
`owning [ 42 ]`** — hits the check-side guard with a real diagnostic and **not** a panic.

**Phase 3.** A linear local moved into an escaping `owning` closure, returned, and `call`ed
disposes it **exactly once** (a forced-linear `Spy`-style fixture with an observable `drop`
side effect; assert once, not zero or twice). `drop`ping an owning closure instead is the
located rejection. A conditional that calls it on one arm only is the pre-existing
`not consumed on every path` error (probe 2), and calling it on both arms builds and runs
(probe 1's shape, now materialized). A 2+-linear-capture owning closure builds, runs, and
disposes both exactly once. Assert on the IL that a plain quotation is still the two-word
layout with an unchanged `code` offset (a pre-existing `code`-slot golden passing through
untouched), and that no plain-quotation program gained an allocation. Mutation-test the
body's env free (stubbing it must fail a leak-detecting fixture) and the
`drop`-on-owning rejection (deleting it must let a leak through).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Checker classifier fix: classify_capture's aggregate arm gains an is_copy check so a Copy aggregate/enum capture classifies as Scalar and admits at an escaping boundary. Thread structs/enums/arrays down, which DOES change check_capture_admission's parameter list (Ctx has no arrays() accessor; arrays is held only by materialize_quotation_at_boundary) -- a single-caller immutable reborrow. In scope: migrating tests/phase4_quotations.rs:336, which captures [i64 4] (Copy per builtins.rs:243) and flips from reject to admit, plus a mandatory replacement witness that a genuinely linear by-value aggregate (drop-overloaded struct or OwnedCell) still classifies FrameRooted. No new runtime mechanism, no IL change.", "difficulty": "standard" },
    { "phase": 2, "focus": "Add Type::OwningQuotation(&'static QuotEffect) mirroring Type::InlineQuotation, with owning_quotation_type and a distinct name_static spelling, and accept the `owning` keyword in type positions only (an ordinary word: no lexer change, no new token, no term-level syntax -- owningness is inferred at the literal and checked against the declared effect). Whole-crate exhaustive-match audit driven by the spec's explicit per-site admit/reject table, NOT slice 10a's polarity (InlineQuotation rejects at materialization boundaries; OwningQuotation must be admitted; is_quotation_type must return Some or call breaks at terms.rs:324/327, which also defers owning-name captures via the captures.rs:236 case-4 guard). is_copy returns false for it so is_linear is true and move tracking, the dup gate, the forgotten-value error and the consumed-on-every-path check are all inherited. Structural PartialEq inequality at every boundary and if-join. No representation yet: an explicit check-side materialization guard with a located diagnostic, since ir_type_of returns IrType not Result and a non-capturing `owning [ 42 ]` bypasses capture admission entirely. Type-level and guard tests only.", "difficulty": "hard" },
    { "phase": 3, "focus": "Env storage and the call-once lifecycle: IrType::OwningQuotation and ir_type_of's real arm; the captured linear value is moved into a heap env block (intern_owned_cell_type) that outlives the frame, and the per-literal compiled body consumes its captures and frees its own block before returning -- so there is no drop pointer, no third layout slot, no new backend aggregate, no emit_drop arm and no per-value disposal metadata. call is the consuming use with no checker change (terms.rs:311 already pops its receiver) and the existing consumed-on-every-path check already forces a conditional to call it on both arms; drop on an owning closure is a new located rejection naming the remedy. Lift past_owning_frame_error and multi_capture_escaping_error plus its R22 twin for the escaping owning path only, the in-frame stack-bundle path unchanged. Exactly-once disposal goldens for the called path, the drop rejection, both-arms and one-arm conditionals, and 2+ linear captures; IL assertions that plain quotations keep the two-word layout and gain no allocation.", "difficulty": "hard" }
  ]
}
```
