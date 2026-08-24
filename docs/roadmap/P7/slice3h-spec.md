# P7.S3h spec: an escaping closure may capture a linear value (closure env disposal)

Delivery plan. Read `docs/roadmap/P7/slice3h-brief.md` first for the root cause, but note its
"what the fix needs" section is superseded: it describes a disposal mechanism this spec
deleted. Every `path:line` anchor below was verified against live `main`; `captures.rs`,
`quotation.rs`, `func_builder/mod.rs`, `types.rs`, `ast.rs` and `layout.rs` drift as other
slices land, so treat anchors as pointers to re-confirm at implementation, not contracts.
Known drift already: `ast.rs` anchors run about +270 from the numbers quoted here, and the
QBE backend is `src/backend/qbe.rs`, not `src/qbe.rs`.

**Revision 5.** Revision 4 was reviewed by three fresh-context reviewers and BLOCKed on two
soundness holes, both now closed by narrowing. The design itself (a type-level marker, the
body as the sole disposer, `call` as the consuming use) survived review intact and is
unchanged.

## Problem

Capture into an escaping (materialized, non-spliced) closure is gated by
`check_capture_admission` (`src/check/captures.rs:202`), reached from
`materialize_quotation_at_boundary` (`captures.rs:287`) only when the literal actually
captures (`body_captures_enclosing`, `captures.rs:12`). `classify_capture`
(`captures.rs:144`) buckets every captured name by its `Binding.ty` alone:

- `Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)` → always
  `FrameRooted` (`captures.rs:162`), unconditionally, **not gated on linearity or `is_copy`**.
- `Type::Ref(..) | Type::Slice(..)` → `FrameRooted`/`OuterRooted` per `ref_root_is_in_frame`
  (`captures.rs:126`, `174`).
- Anything else → `Scalar` (`captures.rs:182`), snapshotted into the env, admitted everywhere.

A `FrameRooted` capture at an escaping boundary is rejected (`captures.rs:249`,
`past_owning_frame_error`, `:49`). This is not a linearity check, and the "restricted to
`Copy`" framing describes the surviving case rather than a branch that exists. DESIGN.md
states no such rule; the roadmap's `DESIGN.md:372` citation for it pointed at no such text
and has been corrected. A `bool` local, a payload-free and structurally-`Copy` enum since
S3i, hits the same rejection, live-probed:

```sooth
import: intrinsics * ;  import: core::prelude * ;
: mk ( Bool -- [ -- Bool ] ) | b | [ b ] ;
: main ( -- ) true mk call ~[ 1 . ] ~[ 0 . ] if ;
\ error: an escaping closure captures `b`, a local of this frame, whose storage does not
\   survive the return (line 3)
```

`is_copy` (`builtins.rs:219`, its `Type::Enum` arm folding structurally over variant fields)
calls `bool` `Copy`, so the blanket `Type::Enum(..) => FrameRooted` over-rejects a `Copy`
enum exactly like a linear one. Same root cause as the code comment's own named narrowing
(a by-value aggregate parameter or global treated like a locally-constructed one,
`captures.rs:145-160`), which stays out of scope.

A 2+-capture escaping closure is separately rejected regardless of what it captures
(`multi_capture_escaping_error`, `captures.rs:76`), and its transitive-escape twin blocks the
stack bundle (`build_env`'s `many` arm, `quotation.rs:97-113`, an `Alloc` into the current
frame via `push_alloc`) from reaching an escaping boundary. Deferred case, not a live
use-after-return bug.

### What the probes establish

1. **A quotation body consuming a captured linear value already works.** Two `if` arms each
   splicing `~[ s drop ]` over a linear `Spy` builds and runs. "The body disposes what it
   holds" is not a new capability; it is how a spliced body already behaves.
2. **The must-consume-on-every-path discipline already exists**, with the diagnostic this
   slice needs, and it covers **anonymous** stack-resident values too, not just named locals:

   ```text
   error: linear value left on the stack in `main` (line 6)
     body leaves a `Spy` beyond the 0 declared output(s): a linear value must be consumed
     exactly once, so `drop` it or return it
   ```

3. **Exactly one gate blocks the target shape.** `: mk ( Spy -- [ -- ] ) | s | [ s drop ] ;`
   fails with only `past_owning_frame_error`. Not a disposal error, not a linearity error: a
   **storage-lifetime** error.

So the gap is not "closures cannot dispose things". It is "an escaping closure's env has no
storage that outlives the frame".

## Design

### The obligation is in the type; the storage is not

- **Does this closure owe a disposal?** Must be in the type. A consumer discharges the
  obligation and cannot see the captures: `Type::Quotation(&'static QuotEffect)` is keyed by
  declared effect only, which is what lets combinators exist. Every consumer in
  `lib/combinators.sth` (`times`, `each`, `map`, `fold`, `filter`, `while`) declares a
  quotation parameter without knowing what a caller closed over, as does
  `dispatch ( Vm [ [ Vm -- Vm ] 9 ] -- i64 )` (`examples/vm_table.sth:219`). Putting the
  capture set in the type would make every one of those signatures unwritable.
- **Where does the env live?** Must *not* be in the type, or a combinator could only accept
  closures whose storage it happens to name. Inline, static and heap are all legitimate.

**So: a marker in the type, naming the obligation and nothing else.** Add
`Type::OwningQuotation(&'static QuotEffect)` (same payload as `Type::Quotation`), with
`is_copy` false for it (so `is_linear` is true), while plain `Type::Quotation` stays `Copy`
exactly as today. The precedent is `Type::InlineQuotation` (`ast.rs:1989`, slice 10a): same
payload, split purely to carry a capability difference, its doc comment naming the mechanism
this reuses, that structural `PartialEq` gives `InlineQuotation(e) != Quotation(e)` for free.
Mirror its shape, including a distinct `name_static` spelling.

### The body is the disposer, so the closure must be called

An owning closure is a word with its arguments already supplied. A word taking a linear
argument needs no disposal metadata: it is always called, and its body consumes the argument.
A closure differs in exactly one way, that it might never be called. Close that gap by
requiring it:

- **The owning closure value is linear** (from the marker), so it must be consumed on every
  path. That machinery exists, probe-confirmed, including for anonymous stack values.
- **`call` is its consuming use.** `call` already pops its receiver and never re-pushes it
  (`terms.rs:311`, routed at `:322-327` to `check_abstract_quotation_call`, `:1123-1146`,
  which pops `eff.inputs`, pushes `eff.outputs`, and leaves no closure behind). The checker
  needs **no** change: the pop already discharges the obligation.
- **`drop` on an owning closure is a located rejection.** The compiler cannot run the body's
  disposal without running the body.
- **An owning closure may not be a field or an element** (see below). This is what keeps the
  "body is the sole disposer" claim true.
- **The body consumes its captures**, exactly as a word body consumes a linear parameter.
- **The body frees its own env**, being the only code that statically knows the env layout,
  and now guaranteed to run exactly once.

**What this deletes:** no `drop` function pointer in the value, no third slot, no widened
`QuotLayout`, no new backend aggregate, no `emit_drop` arm, no check-time per-literal env
struct minting, no carrier out of per-word `Provenance`, no module symbol map, and no
check/lower quotation-numbering invariant. The runtime cost of an owning closure is the env
storage it needs anyway; the marker itself costs nothing.

### Containment: an owning closure is never reachable through an aggregate

Review found the hole this rule closes. If a struct could hold an `owning` field, then
`is_copy`'s struct arm (`builtins.rs:222-225`) makes the struct non-`Copy`, so the struct is
linear and `drop`ping it is a legal consumption. But the container's disposal cannot dispose
the closure: `emit_drop`'s match has arms for `OwnedCell`, `Struct`, `Enum` and `Array` and a
`_ => {}` fall-through that silently swallows a quotation, and synthesized glue has no way to
run the closure's body. Worse, `field_is_linear` (`src/ir/layout.rs:66`) and
`layout_field_is_linear` (`:889`) also fall to `_ => false` for a quotation, so
`StructLayout::is_linear` (`:790`) computes false and
`synthesize_aggregate_destructors` (`destructors.rs:61`) synthesizes **nothing**. The
container's `drop` becomes a complete no-op: the capture's own `drop` never runs and the env
block is never freed. The consumption obligation is discharged without the body running.

**Rule: an owning quotation is rejected in every aggregate position** (struct field, array
element, slice element, owned-cell payload, `extern:` boundary, reference referent), with a
located diagnostic. Consequences worth stating plainly:

- **Array and slice elements need no new work.** `check_no_linear_array_elements`
  (`src/check/declarations.rs:968-984`) and `check_slice_element_gate` (`:1008-1025`) already
  reject any element that is not `is_copy`, so an `owning` element is *already* rejected the
  moment `is_copy` returns false for it. Revision 4's audit table claimed these positions
  "admit"; that was wrong in both directions and is corrected here.
- **Struct fields are the position that needs the new gate**, because struct fields
  deliberately have no such restriction (ordinary linear struct fields are supported and
  wanted).
- **`field_is_linear` and `layout_field_is_linear` are deliberately untouched.** With the
  gate in place an owning quotation can never be a field, so neither predicate can ever see
  one. This is why phase 3 does not need to widen them, and the reason is recorded here so
  an implementer does not "fix" them and quietly reopen the hole.

The cost is that an owning closure cannot be stored in a data structure. It can be created,
passed, returned and called. Lifting that restriction needs a disposer the glue can invoke,
which is **P7.S3v**, sequenced after trait objects (**P7.S3u**) supply the erased-owner
mechanism.

### Surface syntax: `owning [ … ]`, in type positions only

**`^[ … ]` is deliberately not used.** `^` means "heap cell" (`sooth_alloc`/`sooth_free`,
`ir/types.rs:31`/`:35`), which answers the *storage* question this type must stay silent on.
It is also mechanically unavailable: `^[u8 64]` is live owned-cell-of-array syntax (the QBE
prelude's own `Buf`, `src/backend/qbe.rs:2868`) and `src/lexer.rs:441`
(`lex_owning_cell_array_type_splits_at_bracket`) pins `^[` splitting into two tokens.

**`owning` needs no lexer change but does need parser work**, and revision 4 understated
this. `is_delimiter` (`src/lexer.rs:30`) is `; ( ) | [ ]`, so `owning` lexes as an ordinary
word, and it collides with nothing (`grep -rn owning lib/ examples/ src/` finds only a prose
comment). But type-position dispatch branches on the *first token* (`parse_type_expr`,
`src/parser.rs:3448`; `parse_slot`, `:3391`): `[` goes to array/quotation, a `&`-word to ref,
a `^`-word to owned-cell, else the word is resolved as a **type name**. It never looks ahead.
Probed: `: consume ( owning [ -- ] -- ) call ;` gives `error: unknown type 'owning' at line 3,
col 13`. So a new prefix branch is required at **every** type-position entry, including
`parse_slot` and `parse_poly_slot` and the `parse_type_expr` recursion used for struct fields,
array elements, referents and quotation-effect lists. Because `owning` then occupies the
type-name namespace, it also needs a **reserved-name rejection**, mirroring
`reject_reserved_name`/`is_reserved_caret_name` (`parser.rs:112-128`) for `^`-led names, so a
user cannot declare `type: owning … ;` and shadow the syntax.

**Owningness is inferred at the literal; declared in the type.** A materialization boundary
already knows the capture set and each capture's type and already checks the literal against
the boundary's declared effect. So there is **no term-level syntax**: a literal stays `[ … ]`
and its owningness is derived. A literal capturing a linear value does not satisfy a plain
`[ … ]` slot, giving a located error naming `owning` as the fix. Declared signatures must
state it, because a consumer is compiled once and cannot straddle the linear and `Copy`
checking regimes, and because a declared effect is a checked contract: a signature that
inferred owningness from a body would tell its caller the result is forgettable when it is
not.

### Env storage

**In slice: heap**, via `intern_owned_cell_type` (`ast.rs:1015`, already structurally
deduping). DESIGN.md places heap-env closures in the `alloc` layer. The body frees its own
block (`FREE_SYMBOL`, as `synthesize_cell_destructor` already does at `destructors.rs:476`),
so no per-value metadata is needed. **Inline and static envs are follow-on work**, named but
not designed here; they are what would let a `fixed`-layer program own a linear capture, and
the type is silent on storage so they can land without touching a signature.

## The per-site admit/reject audit

Adding a `Type` variant makes every non-wildcard `match` a hard build error until handled, so
the compiler enumerates the sites; what it cannot do is decide them. **Do not mirror slice
10a blindly:** `InlineQuotation` rejects at every materialization boundary, whereas
`OwningQuotation` must be admitted there and must reach the backend.

| Site | Answer |
| --- | --- |
| `is_quotation_type` (`ast.rs:2049`) | **Some**, or `call` breaks (`terms.rs:324`/`327` falls to `call_needs_quotation_error`) |
| materialization boundaries | **admit**, the point of the slice |
| word parameter / output declaration | **admit** at parse and check level; *lowering* lands in phase 3 (see below) |
| struct field | **reject**, new gate (the containment rule) |
| array element, slice element | **already reject**, via the existing non-`Copy` element gates; no new work |
| owned-cell payload, `extern:` boundary, reference referent | **reject**, as plain `Quotation` is |
| `ir_type_of` (`types.rs:301`) | **real arm** in phase 3; phase 2 guards check-side, see below |
| capture-admission case-4 guard (`captures.rs:236`) | **defer** |

Two rows need their reasoning recorded, because revision 4 got both wrong.

**The `ir_type_of` guard is not just about materialization.** `ir_type_of` returns `IrType`,
not `Result`, and its `InlineQuotation` arm is `unreachable!()`, so a "deferral" there is an
ICE. But a declared owning parameter reaches `ir_type_of` through **signature lowering**
(`lower_word_parts`, `src/ir/func_builder/mod.rs:758` for inputs and `:780` for outputs, plus
`driver.rs:146`/`:485` and `layout.rs` for fields and elements) without ever crossing a
materialization boundary. So `: consume ( owning [ -- ] -- ) call ;` type-checks clean and
would ICE at `mod.rs:758`. Phase 2's guard must therefore cover **declaration positions as
well as materialization**, and phase 3 lifts it when it supplies the real arm. Stating it as
"guarded check-side" without that scope, as revision 4 did, invites exactly the ICE.

**`is_quotation_type` pulls both ways.** It must return `Some` for the `call` path, and the
same accessor backs the case-4 guard, so an owning-typed *name* capture is deferred by
`captured_quotation_name_deferred_error`. Acceptable and out of scope, but a stated
consequence.

## What ships

### Phase 1: the classifier fix

`classify_capture`'s aggregate arm (`captures.rs:162`) must not admit on `is_copy` alone.
Review proved why: the env slot is one word (`quotation_layout`, `ir/types.rs:231-240`) and
`build_env`'s single-capture arm stores the capture's live value inline
(`[(_, value)] => *value`, `quotation.rs:104`), but an aggregate's value is a **pointer** into
frame storage (`is_aggregate`, `func_builder/mod.rs:40`, true for `Struct`/`Array`/`Slice`).
A reviewer probed it: an in-frame closure over a `[i64 4]` local, with the array mutated to
`99` after materialization and before `call`, printed `99`. The env aliases the frame; at an
escaping boundary that same pointer outlives its frame. Admitting `Copy` aggregates would be
a use-after-return, and phase 3 does not rescue it because phase 3's heap env is gated on
*linear* captures.

- **The predicate is `is_copy` AND scalar-represented** (not aggregate-backed). This admits
  exactly one shape, and it is worth naming precisely: of the four types the arm matches, a
  struct and an array are always pointer-backed, an owned cell is never `Copy`, and an enum is
  scalar exactly when it is payload-free. **So the narrowing admits payload-free enums and
  nothing else**, which is the motivating `bool` case, and rejects `[i64 4]` and every `Copy`
  struct.
- **Plumbing, stated honestly.** `is_copy(ty, structs, enums, arrays)` needs `arrays`.
  `Ctx` exposes `structs()`/`enums()` (`engine.rs:1171`/`1177`) but **no `arrays()`**, so
  `check_capture_admission`'s parameter list changes. It has **two** production callers, not
  one: `captures.rs:306` (`materialize_quotation_at_boundary`) and `terms.rs:1298` (inside
  `check_branch_join`, the `if`-join path), plus three unit-test callers at `captures.rs:574`,
  `:683` and `:705`. `check_branch_join` already carries `arrays` in its signature
  (`terms.rs:1170`), so the thread-through is available at both sites. The `if`-join is
  exactly a path a newly-admitted capture flows through, so missing it is not cosmetic.
- **No existing test flips.** Revision 4 planned to migrate
  `tests/phase4_quotations.rs:336` (`escaping_closure_over_frame_local_is_past_owning_frame`,
  which captures a `[i64 4]`) from reject to admit. Under the corrected predicate that test
  keeps rejecting and needs no change. A reviewer grepped all seven `past_owning_frame` sites
  (`phase4_quotations.rs:336`, `354`, `374`, `700`, `727`, `756`, `781`) and confirmed the rest
  capture refs, which this arm does not touch, so **the migration count is zero**.
- The case-2 parameter/global narrowing (`captures.rs:145-160`) is untouched.

No new runtime mechanism, no IL change, and now that is a true statement rather than a
disclaimer contradicted by the content.

### Phase 2: the `OwningQuotation` type, syntax, containment

- `Type::OwningQuotation(&'static QuotEffect)` plus `owning_quotation_type` mirroring
  `inline_quotation_type`, with its own `name_static` spelling.
- `owning` accepted in type positions: a new prefix branch at every type-position entry
  (`parse_type_expr`, `parse_slot`, `parse_poly_slot`, and the recursion), plus the
  reserved-name rejection so `owning` cannot be declared as a type name.
- The per-site audit above, including **the struct-field rejection** (the containment rule)
  and the confirmation that array and slice elements already reject.
- `is_copy` returns `false` for it (an arm above the `_ => true` wildcard); `is_linear` then
  true with no change of its own, and move tracking, the `dup` gate, the forgotten-value
  error and the consumed-on-every-path check are all inherited.
- Boundary behaviour by structural inequality: a plain literal does not satisfy an `owning`
  slot or vice versa; an `if`-join of an owning and a plain closure is an ordinary type
  mismatch; a literal capturing a linear value at a plain boundary names `owning` as the
  remedy.
- **No representation yet, guarded check-side at both entries.** A located diagnostic for a
  materialized owning literal *and* for an owning type in any lowerable declaration position,
  so no phase-2 program can reach `ir_type_of`. The sharp case is a **non-capturing** owning
  literal (`owning [ 42 ]`): `body_captures_enclosing` is false, so a capture-side guard alone
  would not block it.

### Phase 3: env storage, the call-once lifecycle, disposal

- `IrType::OwningQuotation` and `ir_type_of`'s real arm; lift phase 2's guard.
- Heap env via `intern_owned_cell_type`: the captured linear value is *moved* into the block
  (consumed from the frame, so `Scope::leave`'s unconsumed-local check, `engine.rs:544-551`, is
  satisfied) and the block outlives the return.
- The body consumes its captures and frees its own env block before returning.
- `call` as the consuming use, **no checker change**; the existing consumed-on-every-path
  check already forces a conditional to call it on both arms.
- `drop` on an owning closure: a located rejection naming the remedy.
- Lift `past_owning_frame_error` for a linear capture at an `owning` boundary, and
  `multi_capture_escaping_error` with its transitive twin for the same path, now that the env
  is not a stack `Alloc`. The in-frame path (`escaping: false`) is unchanged.
- `field_is_linear`/`layout_field_is_linear` stay untouched, by the containment rule.

## Out of scope

- The case-2 aggregate-parameter/global narrowing (`captures.rs:145-160`).
- Capturing an already-quotation-typed name by value (`captured_quotation_name_deferred_error`).
- **Storing an owning closure in an aggregate, and discarding one unexecuted.** Both need a
  disposer the synthesized glue can invoke. That is **P7.S3v**, after **P7.S3u** (trait
  objects) supplies the erased-owner mechanism. Strict widenings of this design, not
  redesigns.
- **Inline and static env storage** (the `fixed`-layer, allocation-free case).
- Polymorphism over plain versus owning quotation types. Consequence: no existing
  higher-order word declared over `[ … ]` can accept an `owning` closure, so a combinator
  wanting either must be written twice. Review confirmed this is the *safety* story: the type
  inequality is what stops an owning closure reaching a combinator that cannot dispose it.
- The REPL's inability to link a materialized quotation at all (`__quot0` non-PIC relocation,
  pre-existing). The checker-side lift is not REPL-bypassed, so a REPL owning closure will
  check-pass and then fail to link, enlarging an existing broken set rather than creating a
  new class.

## Invariants to preserve

- Plain `Type::Quotation` stays `Copy`: no new obligation, no `dup` ban, no IL or golden churn
  for any program that exists today. `type :Q{n} = { l, l }` (`src/backend/qbe.rs:151`) stays
  byte-identical.
- `is_linear` remains the single source of every exactly-once obligation; the marker adds no
  parallel machinery.
- `OwningQuotation(e) != Quotation(e)` structurally, so every boundary and `if`-join separates
  them by type inequality before lowering.
- The type names the obligation only. **Storage never appears in the type.**
- **An owning quotation is never a field, element, payload or referent**, so no synthesized
  destructor and no `emit_drop` path can ever reach one. This is what makes "the body is the
  sole disposer" true rather than aspirational.
- A capture is snapshotted into the one-word env only when it is **scalar-represented**;
  aggregate-backed values are pointers and may never be snapshotted at an escaping boundary.
- `call` is a consuming use and needs no checker change; `drop` on an owning closure is
  rejected.
- Owningness is inferred at literals and declared in types. No term-level `owning` syntax.

## Tests

`tests/phase7_slice3h.rs` plus unit tests beside each touched function.

**Phase 1.** The `bool`-capture program above now admits (golden). A `[i64 4]` capture still
rejects, and `phase4_quotations.rs:336` is unchanged, which is itself the evidence that the
predicate is narrow. A `Copy`-struct capture still rejects. Unit tests on `classify_capture`
for the scalar-enum admit and the aggregate reject. An `if`-join golden exercising the
`terms.rs:1298` caller, so the second call site is covered rather than assumed.
Mutation-test the new predicate by deleting the scalar-representation half: the `[i64 4]`
rejection golden must fail. Note the direction explicitly, because it is easy to get
backwards: the mutation that matters collapses the arm toward *admitting* (dropping the
aggregate check), and the guard is the `[i64 4]` case; a mutation collapsing the arm toward
`FrameRooted` is instead caught by the `bool` admit golden.

**Phase 2.** `owning [ … ]` parses in every admitted type position and is rejected in every
rejected one (unit plus goldens). `type: owning … ;` is a located reserved-name rejection.
`is_copy(OwningQuotation) == false`, `is_linear(OwningQuotation) == true`,
`is_copy(Quotation) == true` unchanged (unit); mutation-test the new `is_copy` arm by deleting
it and watching the linearity unit fail. `OwningQuotation(e) != Quotation(e)` (unit).
**The struct-field rejection is a golden, and mutation-testing it means deleting the gate and
watching a leak-shaped program become accepted**, which is the guard for the whole containment
rule. Goldens: a plain literal in an `owning` slot; an owning/plain `if`-join; a linear capture
at a plain boundary naming `owning`; a `dup` of an `owning` binding giving the ordinary linear
`dup` rejection and forgetting one giving the ordinary forgotten-linear error (these prove the
marker inherited move tracking with zero new code); and a materialized owning literal,
**including the non-capturing `owning [ 42 ]` and a declared `( owning [ -- ] -- )` parameter**,
hitting the check-side guard with a real diagnostic and **not** a panic.

**Phase 3.** A linear local moved into an escaping `owning` closure, returned, and `call`ed
disposes it **exactly once**: a forced-linear `Spy`-style fixture with an observable `drop`
side effect, asserting one observation, not zero and not two. `drop`ping an owning closure
instead is the located rejection. A conditional calling it on one arm only is the pre-existing
`not consumed on every path` error; calling it on both arms builds and runs. A 2+-linear-capture
owning closure builds, runs and disposes both exactly once. IL assertions that a plain
quotation keeps the two-word layout with an unchanged `code` offset and that no
plain-quotation program gained an allocation.

**On mutation-testing the env free:** revision 4 said "stubbing it must fail a leak-detecting
fixture", which review correctly called unbuildable, since a leaked heap block has no
observable effect in a normal run and the harness has no allocator accounting. Assert instead
that the emitted body **contains a `FREE_SYMBOL` call**, and mutation-test by stubbing the free
and watching that assertion fail. It guards the thing that is actually checkable.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Checker classifier fix in classify_capture's aggregate arm (src/check/captures.rs:162): admit a capture as Scalar only when it is BOTH is_copy AND scalar-represented rather than aggregate-backed. This is a soundness-critical narrowing, not is_copy alone: the env slot is one word and build_env's single-capture arm stores the value inline, but an aggregate's value is a pointer into frame storage (is_aggregate, src/ir/func_builder/mod.rs:40), so admitting a Copy aggregate at an escaping boundary is a use-after-return (probe: a captured [i64 4] mutated after materialization printed the mutated value, proving the env aliases the frame). Of the four types the arm matches, structs and arrays are always pointer-backed, an owned cell is never Copy, and an enum is scalar exactly when payload-free, so the narrowing admits payload-free enums (the motivating bool case) and nothing else. Thread structs/enums/arrays down; Ctx has no arrays() accessor so check_capture_admission's parameter list changes, and it has TWO production callers, captures.rs:306 and terms.rs:1298 (the if-join inside check_branch_join, which already carries arrays at terms.rs:1170), plus three unit-test callers. NO existing test flips: phase4_quotations.rs:336 keeps rejecting under the corrected predicate. No new runtime mechanism, no IL change.", "difficulty": "standard" },
    { "phase": 2, "focus": "Add Type::OwningQuotation(&'static QuotEffect) mirroring Type::InlineQuotation with owning_quotation_type and a distinct name_static, and accept `owning` in type positions only. This needs no lexer change but DOES need a new parser prefix branch at every type-position entry (parse_type_expr src/parser.rs:3448, parse_slot :3391, parse_poly_slot, and the recursion for struct fields/elements/referents/effect lists), because dispatch is first-token only and `owning` currently resolves as a type name (probed: `unknown type owning`), plus a reserved-name rejection mirroring is_reserved_caret_name so `type: owning ;` cannot shadow the syntax. Whole-crate exhaustive-match audit driven by the spec's per-site table, NOT slice 10a's polarity. CONTAINMENT RULE, load-bearing for soundness: reject an owning quotation in every aggregate position. A struct field needs a NEW gate (struct fields deliberately allow linear fields), while array and slice elements already reject via the existing non-Copy element gates at src/check/declarations.rs:968 and :1008 (no new work). Without this, a struct holding an owning field is linear check-side, but emit_drop's `_ => {}` swallows a quotation and field_is_linear/layout_field_is_linear (src/ir/layout.rs:66/:889) return false so no destructor is even synthesized: the container's drop is a no-op and the capture leaks. Leave those two predicates untouched on purpose. is_copy returns false for the new variant so is_linear, move tracking, the dup gate, the forgotten-value error and consumed-on-every-path are inherited. No representation yet: a check-side guard covering BOTH materialization AND lowerable declaration positions, because a declared owning parameter reaches ir_type_of via signature lowering (lower_word_parts src/ir/func_builder/mod.rs:758/:780, driver.rs:146/:485) without crossing a materialization boundary and would ICE, and a non-capturing `owning [ 42 ]` bypasses capture admission entirely.", "difficulty": "hard" },
    { "phase": 3, "focus": "Representation and the call-once lifecycle: IrType::OwningQuotation, ir_type_of's real arm, and lifting phase 2's guard. The captured linear value is moved into a heap env block (intern_owned_cell_type) that outlives the frame, and the per-literal compiled body consumes its captures and frees its own block (FREE_SYMBOL) before returning, so there is no drop pointer, no third layout slot, no new backend aggregate, no emit_drop arm and no per-value disposal metadata. call is the consuming use with no checker change (terms.rs:311 already pops its receiver) and the existing consumed-on-every-path check already forces a conditional to call it on both arms; drop on an owning closure is a new located rejection. Lift past_owning_frame_error and multi_capture_escaping_error plus its transitive twin for the escaping owning path only, leaving the in-frame stack-bundle path unchanged. field_is_linear and layout_field_is_linear stay untouched because the containment rule means an owning quotation can never be a field. Exactly-once disposal goldens with an observable drop side effect (assert one observation, not zero or two), the drop rejection, both-arms and one-arm conditionals, 2+ linear captures, and IL assertions that plain quotations keep the two-word layout and gain no allocation. Mutation-test the env free by asserting the emitted body contains a FREE_SYMBOL call and stubbing it, NOT by a leak-detecting fixture (a leaked block has no observable effect and the harness has no allocator accounting).", "difficulty": "hard" }
  ]
}
```
