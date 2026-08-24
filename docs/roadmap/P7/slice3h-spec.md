# P7.S3h spec — An escaping closure may capture a linear value (closure env disposal)

Delivery plan. Read `docs/roadmap/P7/slice3h-brief.md` first: it holds the confirmed
root cause and the live-probed diagnostics. Every `path:line` anchor below was
re-verified against live `main` (`3cd199f`, past the brief's `a5c084b`);
`captures.rs`/`quotation.rs`/`func_builder/mod.rs`/`types.rs`/`ast.rs`/`check.rs` drift
as other slices land, so line numbers are anchors to re-confirm at implementation, not
contracts.

**Revision 3. This spec supersedes two earlier drafts that fresh-context reviewers each
BLOCKed.** The revision-2 draft's remaining defects, all now folded in: it described a
closure lifecycle (`call` then later `drop`) the language cannot express (`call` already
consumes the value, §Calling an owning closure); it claimed `^[` reuses the `^` token with
"no new token" (false at the lexer, sub-decision 1); it told the audit to "mirror slice
10a", whose answer is the *opposite* of what an owning quotation needs at every
materialization site (§The per-site admit/reject audit); its phase-1 test plan both flipped
and claimed-unchanged one existing passing test; and its phase-2 "located deferral" in
`ir_type_of` is an `unreachable!()` panic, not a diagnostic (§Phase 2).

The original draft attached a heap-owned, disposal-requiring env to an ordinary
`Type::Quotation` value and asserted exactly-once was "unaffected". It was not: `is_copy`
(`src/check/builtins.rs:219`) has a bare `_ => true` fallthrough that swallows
`Type::Quotation`, so `is_linear` (`builtins.rs:254`) is *false* for every quotation.
Move tracking only tracks `is_linear` values, so a disposal-owning closure typed
`Type::Quotation` could be `dup`-ed (double free) or silently forgotten (leak) — a direct
violation of CLAUDE.md's linear-spine invariant. The root cause is structural: linearity
in Sooth is decided from the `Type` alone, and `Type::Quotation(&'static QuotEffect)` is
keyed by declared *effect* only, so `: mk_owning ( Spy -- [ -- i64 ] ) | s | [ s use ] ;`
and `: mk_plain ( -- [ -- i64 ] ) [ 42 ] ;` share one type with different obligations and
can meet at an `if`-join. Per-value ownership is not expressible on one shared type.

**Resolution, approved by the owner (do not re-litigate): split the type.** Add a distinct
`Type::OwningQuotation(&'static QuotEffect)` (same payload as `Type::Quotation`) for a
closure whose env owns something; `is_copy` returns `false` for it (so `is_linear` is
`true`), while plain `Type::Quotation` stays `Copy` exactly as today. The precedent is
`Type::InlineQuotation` (`src/ast.rs:1989`, slice 10a): same payload, split purely to
carry a capability difference, its own doc comment naming the mechanism this reuses —
"structural `PartialEq` gives `InlineQuotation(e) != Quotation(e)` for free, so every
materialization boundary rejects a `~` by type inequality before the boundary". Mirror its
shape exactly, including a distinct `name_static` spelling so the two render distinctly
(`inline_quotation_type`, `ast.rs:2020`).

What the split *resolves for free* (these are consequences to reflect, not separate work):

- **Existing programs are untouched.** Plain quotations stay `Copy`: no new drop
  obligation, no banned `dup`, no forgotten-value error, no suite-wide churn.
- **No new linearity machinery.** Move tracking, must-drop-exactly-once, `dup` rejection,
  and the forgotten-value error all already key off `is_linear`; the split flips one
  predicate and inherits all of it.
- **An `if`-join of an owning and a plain closure is an ordinary type error** (structural
  `PartialEq` inequality), not a silent linearity hole.
- **Two IR types, two QBE aggregates.** `src/backend/qbe.rs:151` emits `type :Q{idx} = {{
  l, l }}` as a literal, independent of `quotation_layout`. Under the split that literal
  stays correct for plain quotations, and a *new* three-word aggregate is emitted for the
  owning type. This is additive, not a widening: no layout change for existing programs,
  no IL/golden churn. Do **not** widen `QuotLayout`; give the owning type its own layout
  (`code`/`drop`/`env`, three words) alongside the existing two-word `QuotLayout`.
- **`emit_drop` (`src/ir/func_builder/quotation.rs:305`) gets one type-directed arm** like
  every other: owning type → indirect call through the `drop` word; plain `Quotation` →
  unchanged no-op. There is **no null-check branch** inside `emit_drop` — an owning
  quotation always owns and always carries a real drop symbol, a plain one is never an
  owning arm — so `emit_drop` never becomes a control-flow emitter (no `Jnz`/`fresh_block`/
  `seal_block`, no disturbance to `destructors.rs` callers that keep pushing after it).

Three sub-decisions, settled:

1. **Surface syntax `^[ -- i64 ]`**, echoing the `^` owned-cell sigil, spelled at the
   declaration. This needs a **new glued token** `Token::CaretLBracket`, exactly as `~[`
   needed `Token::TildeLBracket` (`lexer.rs:27`, emitted at `lexer.rs:199`, whose own
   comment notes `~` is not a delimiter and `[` is, so without the glue it would lex as two
   tokens). `^` is likewise not a delimiter and `[` is (`lexer.rs` `is_delimiter`), so
   without glue `^[` lexes as `Word("^")` + `LBracket`. Note `^` is *not* a standalone
   token today: `^T` lexes as `Token::Word("^T")`, a prefix tested with `starts_with('^')`
   (`parser.rs:129`/`2685`/`3190`/`3233`). The glued token is unambiguous against `^T`
   (that stays a `Word`). The revision-2 claim of "no new token / reuse the `^` token" was
   false and is dropped; the *syntax choice* it justified is fine and stands.
2. **A non-capturing literal does not implicitly coerce into an owning-typed slot.**
   Magicless: declare the type you mean. A plain `[ ... ]` literal does not satisfy a
   `^[ ... ]` slot and vice versa — that inequality is the whole safety story.
3. **One slice, more phases** (the owner chose folding everything in over two slices).
   Each phase is independently green under `cargo fmt --check && cargo clippy -- -D
   warnings && cargo test`.

## Problem

Capture into an escaping (materialized, non-spliced) closure is gated by
`check_capture_admission` (`src/check/captures.rs:202`), reached from
`materialize_quotation_at_boundary` (`captures.rs:287`) only when the literal actually
captures (`body_captures_enclosing`, `captures.rs:12`). `classify_capture`
(`captures.rs:144`) buckets every captured name by its `Binding.ty` alone:

- `Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)` → always
  `FrameRooted` (`captures.rs:162`), unconditionally — **not gated on linearity or
  `is_copy`.**
- `Type::Ref(..) | Type::Slice(..)` → `FrameRooted`/`OuterRooted` per
  `ref_root_is_in_frame` (`captures.rs:126`, `174`).
- Anything else → `Scalar` (`captures.rs:182`), snapshotted into the env (D4), admitted
  everywhere.

A `FrameRooted` capture at an escaping boundary is rejected (`captures.rs:249`,
`past_owning_frame_error`, `:49`). This is not a linearity check. A `bool` local (a
payload-free, structurally-`Copy` enum since S3i) captured into an escaping closure hits
the exact same rejection —

```sooth
import: intrinsics * ;  import: core::prelude * ;
: mk ( Bool -- [ -- Bool ] ) | b | [ b ] ;
: main ( -- ) true mk call ~[ 1 . ] ~[ 0 . ] if ;
\ error: an escaping closure captures `b`, a local of this frame, whose storage does not
\   survive the return (line 3)
```

`is_copy` (`check/builtins.rs:219`, its `Type::Enum` arm folds structurally over variant
fields) calls `bool` `Copy` — no fields to be linear in — so the blanket
`Type::Enum(..) => FrameRooted` over-rejects a `Copy` enum exactly like a genuinely linear
one. Same root cause as the code comment's own named narrowing (a by-value aggregate
parameter/global treated like a locally-constructed one, `captures.rs:145-160`), which
stays out of scope here.

A 2+-capture escaping closure is rejected regardless of what it captures
(`multi_capture_escaping_error`, `captures.rs:76`, "R18: … a heap env is deferred"), and
its R22 transitive-escape twin blocks the stack bundle (`build_env`'s `many` arm,
`quotation.rs:105-113`, an `Alloc` into the current frame via `push_alloc`,
`func_builder/mod.rs:452`) from ever reaching an escaping boundary. This is the deferred
case, not a live use-after-return bug.

Disposal never touches a closure's env: `emit_drop` (`quotation.rs:305`, the sole disposal
dispatch) matches `IrType::OwnedCell`/`Struct`/`Enum`/`Array` with a silent `_ => {}`
covering `Quotation`/`Ptr`; `synthesize_aggregate_destructors` (`destructors.rs:37`) walks
only `structs`/`enums`/`cells`. No env struct type is minted anywhere: `EnvCapture`
(`func_builder/mod.rs:138`) and the checker's `SurvivingCapture` (`check.rs:220`) both
record per-capture facts as flat lists, never an aggregate `IrType` a destructor can hang
on.

## Design

Trait objects / `dyn Drop` are **not** built here. A materialized quotation's `(code,
env)` shape is structurally a one-method trait object and the per-closure drop pointer a
hand-rolled `dyn Drop`, but real trait objects were deferred (S3e) with no consumer
forcing them; a future trait-object spec has this owning-closure drop word as a precedent
to fold in, not a target to reconcile against.

### The type split (the spine of this slice)

`Type::OwningQuotation(&'static QuotEffect)` (new, `ast.rs`, beside
`Type::Quotation`/`Type::InlineQuotation`). Same payload; built by a new
`owning_quotation_type(inputs, outputs)` mirroring `inline_quotation_type` (`ast.rs:2020`)
with a distinct `name_static` spelling (`^[ ... -- ... ]`) so structural `PartialEq`
gives `OwningQuotation(e) != Quotation(e)` for free and the two render distinctly. Every
accessor that today folds `Quotation` and `InlineQuotation` (`is_quotation_type`,
`ast.rs:2033` region) must decide, per site, whether it also admits `OwningQuotation` —
this is the same whole-crate exhaustive-match audit slice 10a paid, and the audit is the
bulk of phase 2's risk, not the variant itself.

Classification: `is_copy` gains one arm returning `false` for `OwningQuotation`, placed
*above* the `_ => true` wildcard that currently swallows it (`builtins.rs:219`); plain
`Quotation` keeps falling through to `true`. `is_linear` (`builtins.rs:254`) then reports
`true` for `OwningQuotation` with no change of its own, and every move-tracking,
`dup`-gate, and forgotten-value site inherits the obligation.

### Runtime representation

Plain `Type::Quotation` → `IrType::Quotation(QuotSigId)` → the unchanged two-word `{ code,
env }` (`quotation_layout`, `types.rs:232`; `type :Q{n} = { l, l }`, `qbe.rs:151`).

`Type::OwningQuotation` → a **new** `IrType::OwningQuotation(QuotSigId)` → a **new**
three-word layout `{ code: Ptr@0, drop: Ptr@word_width, env: Ptr@2*word_width }`, size
`3*word_width`, emitted as its own backend aggregate (e.g. `:QO{n} = { l, l, l }`) in a
loop parallel to `qbe.rs:151`, collected the same way `collect_quot_sigs`
(`driver.rs:365`) already collects plain effects. `code` at offset 0 matches the plain
layout, so a value that begins life plain and is never re-typed keeps its `code` slot; the
owning type is not a widening of the plain one, it is a sibling. Give it its own
`OwningQuotLayout` (do **not** add a `drop_offset` to `QuotLayout`).

`emit_drop` (`quotation.rs:305`) gains one arm:

```rust
IrType::OwningQuotation(_) => {
    // load `drop` at word_width; indirect-call it with `env`. No null branch:
    // an owning quotation always owns, so `drop` is always a real symbol.
}
```

The `drop` symbol is chosen once at construction, statically, from the enumerable set of
already-synthesized `cell_drop_symbol`s (`layout.rs:142`) — the same shape `code`'s
`FuncAddr` (`quotation.rs:82`) has, no vtable, no runtime type read. Additive over every
other arm; none change. Exactly-once and "no hidden control flow" (DESIGN.md:135-154) are
now *actually* unaffected (not merely asserted): the type carries the obligation, move
tracking enforces when `drop` runs, and the indirection only selects which
already-static body runs.

### `call` consumes and disposes an owning closure (call-once)

**The gap this closes.** `call` pops its receiver and never re-pushes it: `terms.rs:311`
(`let Some(top) = stack.pop()`), routed at `terms.rs:322-327` through `is_quotation_type`
to `check_abstract_quotation_call` (`terms.rs:1123-1146`), which pops `eff.inputs`, pushes
`eff.outputs`, and leaves no closure behind. For a plain `Copy` quotation that is fine
(nothing to dispose). For a linear `OwningQuotation` it would be fatal left alone: `call`
*consumes* the value, discharging its exactly-once obligation, while nothing runs
`emit_drop` — so the captured payload would leak with no diagnostic, and after a call there
is no value left to `drop`. "Returned, called, and dropped" is not a lifecycle this
language can express: `call` and `drop` are alternative final uses of one linear value,
not a sequence.

**Ruling: `call` on an owning closure consumes *and* disposes it. An owning closure is
callable at most once.** Checker-side, nothing changes: `call` already pops the receiver,
so linear accounting is already correct with no new stack discipline. Lowering-side, the
`call` site for an `OwningQuotation` receiver emits the indirect call through `code` and
then the env disposal through `drop` — the same word, the same `cell_drop_symbol`, the same
type-directed decision as the `emit_drop` arm above, which is why this lands in the same
phase rather than a phase of its own.

The rejected alternative was an `Fn`-style `call` that re-pushes its receiver so a closure
could be called repeatedly and dropped afterwards. Rejected because `call` would then
leave the stack in different shapes for two quotation types — exactly the kind of implicit,
type-dependent divergence this project rejects elsewhere. A repeat-callable owning closure
needs a *borrowing* call variant (`&`-semantics on the receiver, leaving disposal to a
later explicit `drop`); that is a deliberately deferred, larger slice, named here so a
future spec has the boundary already drawn.

### Disposal is uniform, so a body may not consume its own captures

Call-disposal creates an immediate hazard: if a body could itself consume a captured linear
value (`^[ s use ]`, where `use` takes the `Spy`), the payload would be disposed twice —
once by the body, once by the env disposal after the call returns. Avoiding that by having
two disposal shapes ("free the block only" after a consuming call, versus "dispose the
payload then free the block" for a drop with no call) would need either two synthesized
symbols per env or a runtime flag in the value.

**Ruling: disposal is uniform — always dispose the payload, then free the block, which is
exactly what `synthesize_cell_destructor` already emits (`destructors.rs:453`) — and
correspondingly an owning closure's body may not move a captured linear value out.** The
body reads its captures without consuming them. One destructor symbol per env cell, no
flag, no second shape, and call-disposal and drop-disposal are byte-identical paths.

**Enforcement is a located rejection, not an assertion.** The capture set a body reads is
already computed (`prov.quotation_captures`, pushed at `terms.rs:946`); the body's own walk
already move-tracks. A captured name that the body's walk consumes (leaves in
`MoveState::Moved` at body exit rather than merely reading) is rejected at the literal's
span with its own diagnostic naming the captured name and the rule — not the generic
forgotten/moved wording, since the fix here is "read it, don't hand it on". This needs its
own golden, and a mutation test: deleting the guard must let a body that moves its capture
through, which the exactly-once fixture then catches as a double dispose.

**The cost, stated plainly.** A body that wants to *consume* its capture — hand the owned
value onward, `FnOnce`-with-move semantics — is not expressible in this slice, and is
deferred alongside the borrowing-call variant above. What remains expressible is the shape
that motivates the slice: capture a linear resource, call the closure to operate on it by
reference, and have disposal (whether triggered by that call or by a `drop` with no call)
close the resource exactly once.

### The escaping owning env

An owning literal's env holds the captured values that must outlive the closure's calls,
disposed by the env's destructor when the closure is dropped.

- **Single linear capture.** The env is a bare `^T` cell over the capture's own `Type`,
  interned inline through `intern_owned_cell_type` (`ast.rs:1015`, already structurally
  deduping) using the `cells: &mut Vec<OwnedCellDecl>` that
  `materialize_quotation_at_boundary` already holds (`captures.rs:295`). `synthesize_cell_
  destructor` (`destructors.rs:453`) already disposes a linear payload. The linear value
  is *moved* into the cell (consumed from the frame, move-tracked), so the frame local is
  gone and the cell outlives the return — which is exactly why the escaping rejection is
  sound to lift for this path.
- **Multi capture.** The env is a minted per-literal `StructDecl` (one `(name, ty)` field
  per surviving member, in the body's sorted capture order), wrapped via
  `intern_owned_cell_type(cells, Type::Struct(id))`. The struct fields come from the
  surviving set, which dies with the per-word `Provenance` — so this needs a carrier and a
  post-walk minting pass (see "Where the env type is minted"). Minted with
  **`is_bundle: false`** (unlike `intern_bundle_struct`, `ast.rs:1203`, whose flag
  *suppresses* destructor synthesis): the whole point is that the layout pass computes the
  env's `is_linear` and the cell's destructor disposes any linear field.

`drop` is set to `cell_drop_symbol(id, self.cells.drop_generations[id.index()])` — the same
generation-qualified symbol `emit_drop`'s existing `OwnedCell` arm uses (`quotation.rs:311`).

The R18/R22 deferral lifts **for the escaping owning path only**, because the heap-owned
env makes it sound: the env is no longer a stack-frame `Alloc`, so R22's transitive-escape
hazard does not apply. The in-frame single-boundary case (`escaping: false`) keeps today's
stack bundle unchanged. A capture needing disposal is admitted **only** at a `^[ ]`
(owning) boundary; at a plain `[ ]` boundary it stays rejected — the type-directed gate.

## Where the env type is minted (multi-capture only)

Lowering's registries are finalized (`FuncBuilder::structs: &'a Structs`,
`func_builder/mod.rs:160`, an immutable borrow), so a multi-capture env struct must be
minted at check time. **Ruling: mint in a post-body-walk pass in `check_module`, not inside
`check_capture_admission` and not inside `materialize_quotation_at_boundary`.** This mirrors
`intern_output_bundles` + the instantiation-bundle loop (`check.rs:970-985`), which interns
into `module.structs` *after* the per-word walk, and the generic-instantiation
`flush_structs_into`/`flush_enums_into` (`check.rs:941-942`) — both avoid the `&mut
module.structs`-vs-`Ctx`'s immutable `&[StructDecl]` (`check/engine.rs:1173`) aliasing
conflict by minting when no `Ctx` borrow is live. Reviewers independently confirmed nothing
mid-walk reads the minted env struct, so one post-walk pass is sound.

The unresolved carrier problem, resolved concretely. `Provenance` is created per word
(`word_entry.rs`) and **dies when `check_word` returns**: only `prov.dropped` is extracted
(`word_entry.rs:289`, `dropped.append(&mut prov.dropped)`); `prov.quotations`,
`prov.surviving_sets`, `prov.quotation_captures` all die with it. So the surviving-set
members, their capture `Type`s, and the owning literal's identity cannot be read after the
walk unless they are carried out. **Vessel: a new `&mut Vec<OwningEnvRequest>` accumulator
threaded into `check_word` exactly as `sites: &mut Vec<Vec<Type>>` already is** (the loop at
`check.rs:857-944` builds a fresh `sites` per word, passes `&mut sites` into `check_word`,
then `dropped.push(sites)`). The new accumulator rides the identical pattern: `check_word`
passes it down to `materialize_quotation_at_boundary` → `check_capture_admission`, which
pushes one `OwningEnvRequest { symbol, members: Vec<(String, Type)> }` per multi-capture
owning literal; the caller collects it into a module-scoped `Vec` alongside `dropped`.

Then the post-walk pass (new, called from `check_module` immediately after the
output-bundle loop, with `&mut module.structs`/`&mut module.owned_cells` and no `Ctx`
alive) drains those requests, mints one `StructDecl` per request (structurally deduped by
field-type tuple), wraps it via `intern_owned_cell_type`, and records the resulting
`OwnedCellId` on a **new `Module` map keyed by the `{word}__quot{n}` symbol**.

**Adding that field to `Module` breaks every exhaustive `Module { .. }` literal** — the
tests at `ast.rs:2731`, `ast.rs:2885`, `ast.rs:2953`, plus the parser's construction — so
the implementer must expect that compile break, not discover it.

### The check/lower numbering invariant (a correction, and a real test)

The map key is the `{word}__quot{n}` symbol (`materialize_quot_value`, `quotation.rs:49`),
**not** a `QuotId`. The earlier draft justified this by claiming "the per-word check
`QuotId` and the per-function lowering `QuotId` do not coincide". That rationale was
**false and self-contradictory**: both counters fire on the same `TermKind::Quotation`
match arm in the same per-word source-order walk — check-side `QuotId(prov.quotations.len())`
(`terms.rs:939`, fresh `Provenance` per word) and lowering-side `QuotId(self.quot_defs.len())`
(`calls.rs:56`, fresh `FuncBuilder` per word). They **coincide**. The symbol is a valid key
precisely *because* they coincide (the `n` in `{word}__quot{n}` is that shared count).

**INVARIANT (name it in the spec and pin it in a test): the check-side and lowering-side
per-word quotation numbering agree, in source order, so `{word}__quot{n}` denotes the same
literal on both sides.** Every earlier Phase-2 fixture had exactly one quotation per word,
where any counter is trivially `0` — a placebo w.r.t. this invariant. Require a fixture with
a capturing escaping owning closure in a word that **also** contains a preceding quotation
(including the branch-arm `materialize_join_quotations` shape), asserting the captured
linear value is disposed exactly once — a divergence in the two counters would mis-key the
env cell and either double-dispose or leak, which this fixture catches and a one-quotation
fixture cannot.

### Map threading into lowering

The map reaches lowering the same way `module.instantiations` does. On the compiled path,
`lower` passes `&module.instantiations` into `lower_word_parts` (`driver.rs:210`); the new
map is threaded as a sibling parameter and set on the builder post-construction, exactly as
`b.instantiations = instantiations` (`driver.rs:467`). The destructor/generic-monomorph
paths pass an `empty_*()` map (mirroring `empty_instantiations()`, `driver.rs:852`), and
the REPL path (`lower_line`, `driver.rs:436`) sets its own — where it stays empty, matching
the operator/trait-call overload-dispatch bypass convention, so no new plumbing crosses the
REPL boundary. `materialize_quot_value` reads the map read-only: on a hit, build the cell
and set `drop`; on a miss, today's inline/null `env` (plain path unchanged).

**REPL diagnostic regression, acknowledged.** `multi_capture_escaping_error`
(`captures.rs:76`) lives in the checker, which runs for REPL lines too; lifting R18/R22 for
the escaping owning path means a REPL 2-capture escaping owning closure that today gets a
clean "a heap env is deferred" diagnostic will instead check-pass and hit the pre-existing
non-PIC `__quot0` link failure. Single-capture escaping closures already fail to link in the
REPL today, so this enlarges an existing broken set rather than creating a new class. The
earlier draft waved this off by citing a lowering-path bypass convention, which does not
apply to a checker-side lift — say so plainly.

### The carrier's other two entry paths

`check_word` is not the only caller that builds a per-word `sites` accumulator, so the
`OwningEnvRequest` carrier must have a stated answer at all three, not one:

- **The compiled module walk** (`check.rs:858` / `:936` / `:944`) is the path described
  above: it owns the module-scoped accumulator and drains it in the post-walk pass.
- **`check_def_collecting_drop_sites`** (`check.rs:1066`, its own `let mut sites` at
  `:1087`, passed at `:1143`) is the REPL definition path. It takes the carrier and
  **discards it**: a REPL-defined owning closure gets no minted env cell, matching the
  map-threading bypass above and the existing operator/trait-call overload-dispatch
  convention. Because the map is then empty at lowering, a multi-capture owning literal
  defined in the REPL must be refused *at check time on that path* rather than silently
  lowering with a null `drop` — an explicitly located REPL rejection, with its own test.
- **The REPL line path** (`check.rs:1179` region) does the same, for the same reason.

This is the pattern the parent's review called out: two of three paths were previously
unmentioned, and "the map is empty here" is only safe when paired with a check-time refusal
on that path.

### Decision 2's fallout, stated

No implicit coercion means no existing higher-order library word declared over `[ ... ]`
can consume a `^[ ... ]` closure, and Sooth has no polymorphism over the two quotation
types (DESIGN.md keeps the type system small: concrete types plus minimal row
polymorphism). So a combinator intended to accept "either kind" must be written twice, or
not take owning closures at all. The decision stands — the type inequality *is* the safety
story, and the magicless tie-break applies — but the cost is real and is recorded here
rather than discovered later: **polymorphism over plain and owning quotation types is
explicitly deferred**, and no library word in this slice is duplicated to paper over it.

## What ships

### Phase 1 — the classifier fix, no new runtime mechanism

- `classify_capture`'s aggregate arm (`captures.rs:162`) gains an `is_copy` check. `Copy`
  → treat as `Scalar` (admit everywhere, snapshot, no surviving-set member, no disposal
  obligation); not `Copy` → keep today's `FrameRooted`/`OuterRooted` split unchanged.
- **Honest plumbing note (a correction to the earlier draft).** `is_copy`'s signature is
  `is_copy(ty, structs, enums, arrays)` (`builtins.rs:219`) — its `Type::Array` arm needs
  `arrays`. `classify_capture` today takes only `(b, prov, scope)` (`captures.rs:144`) and
  is called from `check_capture_admission`, which holds only `ctx`; `Ctx` exposes
  `structs()`/`enums()` (`engine.rs:1171`/`1177`) but **no `arrays()` accessor**, and
  `arrays` is held only by `materialize_quotation_at_boundary` (`captures.rs:295`).
  Threading `arrays` down to `classify_capture` therefore **does** change
  `check_capture_admission`'s parameter list (`captures.rs:202`). The earlier draft's claim
  that this stays internal, and its invariant "`check_capture_admission`'s parameter list
  does not change", were false and are dropped. The fix is cheap — a single caller
  (`materialize_quotation_at_boundary`), an immutable reborrow of the `arrays` it already
  holds — and does not disturb the phase-4 post-walk borrow reasoning that invariant was
  really about.
- The R15 case-2 by-value-parameter/global narrowing (`captures.rs:145-160`) is untouched.
- **Expected breakage, in scope for this phase: an existing passing test flips.**
  `is_copy` makes `[i64 4]` **`Copy`** (`builtins.rs:243`, the `Array` arm folds to
  `is_copy(i64) == true`), and
  `tests/phase4_quotations.rs:336`'s `escaping_closure_over_frame_local_is_past_owning_frame`
  captures exactly that shape by value (its sibling's comment at `phase4_quotations.rs:353-355`
  calls it "case 2, the aggregate"). Phase 1 reclassifies it `Scalar`, so it is **admitted**
  and that test's `assert_eq` on the rejection **will fail**. Migrating it is phase-1 work,
  not a surprise to discover: rewrite it to assert the new admission, and
  **add a correct witness that a genuinely linear by-value aggregate still classifies
  `FrameRooted`** — a `drop`-overloaded struct or a `Type::OwnedCell`, both of which
  `is_copy` makes linear. Without that replacement witness the phase ships with no coverage
  of the arm it just narrowed, which is precisely how this project has shipped placebo tests
  before, so treat it as mandatory.

No new runtime mechanism, no IL change.

### Phase 2 — the `OwningQuotation` type, syntax, classification, boundary inequality

- `Type::OwningQuotation(&'static QuotEffect)` + `owning_quotation_type`, plus the
  **new glued `Token::CaretLBracket`** (mirroring `Token::TildeLBracket`, `lexer.rs:27`,
  emitted `lexer.rs:199`) and its parse arms (`parse_poly_slot`, `parser.rs:2623`;
  `parse_slot`, `parser.rs:3169`), with its own lexer unit test. Per decision 1, this is a
  new token — `^[` cannot be lexed by reusing the `^` prefix.
- **The whole-crate exhaustive-match audit, with an explicit per-site admit/reject table —
  do *not* "mirror slice 10a" blindly.** `InlineQuotation`'s answer at every materialization
  boundary is *reject* (`ast.rs:1989`: "cannot be materialized … `ir_type_of` never sees
  one"); `OwningQuotation`'s answer at those same sites is *admit*, since it is designed to
  be a materializable declared output that reaches the backend. Copying 10a's polarity
  would be wrong at exactly the sites that matter. The table must rule on at least:
  `is_quotation_type` (`ast.rs:2049`), the materialization boundaries, the
  declaration-position checks, `ir_type_of`, and the capture-admission case-4 guard.
  **Note the one accessor pulling both ways:** `is_quotation_type` *must* return `Some` for
  `OwningQuotation` or `call` breaks (`terms.rs:324`/`327` would fall through to
  `call_needs_quotation_error`), and the same accessor backs the case-4 guard at
  `captures.rs:236`, so an owning-typed *name* capture is deferred by
  `captured_quotation_name_deferred_error`. That is acceptable and already out of scope, but
  it is a stated consequence, not a discovery for the implementer.
- `is_copy` returns `false` for it (arm above the `_ => true` wildcard); `is_linear` then
  `true` with no change.
- Boundary behaviour: structural `PartialEq` makes a plain `[ ]` literal not satisfy a
  `^[ ]` slot and vice versa; an `if`-join of an owning and a plain closure is an ordinary
  type mismatch. No implicit coercion (decision 2).
- **No disposal, no runtime representation — guarded check-side, because the `ir_type_of`
  arm cannot be a diagnostic.** `ir_type_of` returns `IrType`, not `Result`
  (`types.rs:301`), and its `InlineQuotation` arm is `unreachable!()` (`types.rs:355`), so a
  "located deferral" there would be an **ICE, not an error message**. The `InlineQuotation`
  precedent is only safe because a `~` can never be materialized; `OwningQuotation` is the
  opposite. The sharp case is a **non-capturing** owning literal: `body_captures_enclosing`
  is false, so it never reaches `check_capture_admission` and "owning-capture admission is
  not yet lifted" does **not** block it — it would reach the backend and ICE. Phase 2
  therefore adds an explicit **check-side materialization guard** for `OwningQuotation` (a
  located rejection with its own golden), distinct from the `ir_type_of` arm, which stays
  `unreachable!()` and is genuinely unreachable because the check-side guard fires first.
  Phase-2 tests are otherwise type-level (see Tests).

### Phase 3 — the owning representation and single-linear-capture env

- `IrType::OwningQuotation(QuotSigId)`; `OwningQuotLayout` (three words); `ir_type_of`
  maps to it; the backend emits its `{ l, l, l }` aggregate in a loop parallel to
  `qbe.rs:151`, collected alongside `collect_quot_sigs` (`driver.rs:365`).
- `emit_drop` gains the `IrType::OwningQuotation(_)` arm (indirect call through `drop`, no
  null branch).
- `materialize_quot_value` (`quotation.rs:48`), for a **single** linear capture at an
  owning boundary: intern a `^T` cell over the capture's `Type` (inline, via the `cells:
  &mut` already threaded through `materialize_quotation_at_boundary`), move the value into
  it, set `env` to the cell and `drop` to its `cell_drop_symbol`.
- Lift `past_owning_frame_error` (R24) for a single frame-rooted linear capture into an
  `^[ ]` boundary — sound because the value is moved into the heap cell.
- **`call`-disposal (see "`call` consumes and disposes an owning closure").** The `call`
  lowering site, for an `OwningQuotation` receiver, emits the indirect call through `code`
  and then the env disposal through `drop`. No checker change: `call` already pops its
  receiver (`terms.rs:311`), so the linear accounting is already correct. This lands here,
  with the `emit_drop` arm, because it is the same word, the same `cell_drop_symbol`, and
  the same type-directed decision.
- **The body-may-not-consume-its-captures guard (see "Disposal is uniform").** A located
  rejection when a body's walk moves a captured name out rather than reading it, with its
  own diagnostic naming the capture and the rule, its own golden, and a mutation test
  (deleting the guard must let a consuming body through, which the exactly-once fixture
  then catches as a double dispose).
- **Justified deviation from the task's "phase 3 = representation only".** Single capture
  needs no minted struct and no carrier (the `^T` cell interns inline from the capture's own
  type), so it is the smallest thing that makes the `emit_drop` arm reachable and green at
  source level. Multi-capture is what forces the struct env, the carrier, and the post-walk
  pass, so it is deferred to phase 4. This keeps each phase self-contained rather than
  splitting representation from its only in-phase exerciser.

### Phase 4 — the multi-capture struct env, carrier, R18/R22 lift, goldens

- `SurvivingCapture` (`check.rs:220`) carries the capture `Type`.
- The `&mut Vec<OwningEnvRequest>` carrier threaded through `check_word` (mirroring
  `sites`), the post-walk minting pass, and the new `Module` symbol→`OwnedCellId` map
  (see "Where the env type is minted"). The new `Module` field breaks the exhaustive
  `Module { .. }` literals at `ast.rs:2731`/`2885`/`2953` and in the parser.
- The map threaded into lowering as a `lower_word_parts` parameter set on the builder
  post-construction, `empty_*()` on the destructor/monomorph paths (see "Map threading").
- `materialize_quot_value` builds the multi-field cell for a multi-capture owning literal
  and sets `drop` from the map.
- `multi_capture_escaping_error` (`captures.rs:76`) and its R22 transitive twin lift for
  the escaping owning path; the in-frame stack-bundle path (`escaping: false`) is unchanged.
- The exactly-once disposal goldens, including the check/lower-numbering invariant fixture.

## Out of scope (unchanged)

- The R15 case-2 aggregate-parameter/global narrowing (`captures.rs:145-160`).
- Capturing an already-quotation-typed name by value
  (`captured_quotation_name_deferred_error`, `captures.rs:87`).
- The REPL's inability to link a materialized quotation at all (`__quot0` non-PIC
  relocation, pre-existing; enlarged, not created — acknowledged above).
- Trait objects / `dyn Drop` (the owning-closure drop word is a hand-rolled one-method
  instance; a future trait-object spec folds it in as a precedent).

## Invariants to preserve

- Plain `Type::Quotation` stays `Copy`: no new obligation, no `dup` ban, no IL/golden
  churn for any program that exists today.
- `is_linear` is the single source of every exactly-once obligation; the split adds no
  parallel machinery.
- `OwningQuotation(e) != Quotation(e)` structurally, so every materialization boundary and
  every `if`-join separates them by type inequality before any lowering.
- Two IR types → two QBE aggregates: `type :Q{n} = { l, l }` (`qbe.rs:151`) stays
  byte-identical; the owning three-word aggregate is additive.
- `emit_drop` has no null-check branch and emits no control flow; its new arm is additive.
- `call` on an owning receiver is a *final* use: it consumes and disposes, so an owning
  closure is callable at most once and `call`-then-`drop` is an ordinary use-after-move
  error, not a new diagnostic.
- Disposal is uniform — dispose the payload, then free the block, one `cell_drop_symbol`
  per env cell, no flag and no second shape — which is why a body may not move a captured
  linear value out.
- `:QO{n}` is a separate sig-numbering space from `:Q{n}`, so plain quotation indices never
  shift and existing IL goldens do not churn.
- The multi-capture env struct is minted `is_bundle: false` so its cell destructor is
  synthesized; do not reuse `intern_bundle_struct`.
- Check-side and lowering-side per-word quotation numbering agree in source order, so
  `{word}__quot{n}` denotes the same literal on both sides.
- The env type is minted at check time and interned through `intern_owned_cell_type`;
  lowering only reads finalized registries and the symbol→cell map.

## Tests

`tests/phase7_slice3h.rs` (single file, matching every landed P7.S3 slice) plus unit tests
beside each touched function.

**Phase 1.** The `bool`-capture program above now admits (regression golden); a `[i64 4]`
capture now admits. **`phase4_quotations.rs:336`'s
`escaping_closure_over_frame_local_is_past_owning_frame` captures that same `[i64 4]` shape
and therefore flips from reject to admit — migrate it in this phase** (assert the admission)
rather than expecting it to hold. **Replacement witness, mandatory:** a genuinely linear
by-value aggregate (a `drop`-overloaded struct, or a `Type::OwnedCell`) still classifies
`FrameRooted` and is still rejected at an escaping boundary — otherwise the narrowed arm
ships with no coverage. Unit test on `classify_capture` pinning that a `Copy` aggregate
takes the `Scalar` branch and a linear one keeps `FrameRooted`. Mutation-test the `is_copy`
call: deleting it must fail that linear-aggregate case (and note it would *not* fail the
`[i64 4]` case, which is why the replacement witness is the real guard).

**Phase 2 (type-level only).** `^[ ... ]` parses (parser unit test). `is_copy(OwningQuotation)
== false` and `is_linear(OwningQuotation) == true`, and `is_copy(Quotation) == true`
unchanged (unit) — mutation-test the new `is_copy` arm by deleting it and watching the
linearity unit fail. `OwningQuotation(e) != Quotation(e)` structurally (unit). A plain
`[ ]` literal in a `^[ ]` slot is a located type error, and an `if`-join of an owning and a
plain closure is a located type error (goldens asserting the message names the mismatch,
not `unknown type`). A `dup` of an `^[ ]`-typed binding is the ordinary linear `dup`
rejection, and forgetting one is the ordinary forgotten-linear error (goldens — these prove
the split inherited move tracking with zero new code). **Materializing an `^[ ]` hits the
check-side materialization guard** — a located rejection with a real message, asserted as
such (replaced in phase 3) — **including the non-capturing case** (`^[ 42 ]`, which never
reaches capture admission and would otherwise ICE in `ir_type_of`); assert the build fails
with the guard's diagnostic and *not* with a panic/ICE. Lexer unit test that `^[` lexes as
one `CaretLBracket` token and `^T` still lexes as a `Word`.

**Phase 3.** `call` and `drop` are *alternative* final uses of one owning closure, so the
exactly-once property needs **two** fixtures, not the single impossible
"returned-called-and-dropped" one an earlier draft described:

- **The called path.** A single linear local moved into an escaping `^[ ]` closure,
  returned, then `call`ed — the call disposes the captured value **exactly once** (a
  forced-linear `Spy`-style fixture with an observable `drop` side effect; assert the side
  effect fires once, not zero or twice).
- **The dropped-uncalled path.** The same closure returned and `drop`ped without ever
  being called — disposal fires exactly once, through the identical `cell_drop_symbol`.
- **Both are final uses.** `call`ing and then `drop`ing the same closure is a located
  use-after-move error from the ordinary linear machinery (golden, proving the call-once
  rule needs no bespoke diagnostic).

A leaked or double-disposed capture is a located checker error, not a silent miscompile
(confirm the linear-use machinery produces it once admitted — test, do not assume).
A body that moves a captured linear value out is the located guard rejection, with its own
golden, plus a mutation test: deleting the guard must let the consuming body through, and
the called-path fixture above then observes the double dispose.
Assert on the IL: an owning value is a `3*word_width` `Alloc` with a non-null `drop` slot;
a plain quotation's `code` offset and two-word layout are unchanged (a pre-existing
`code`-slot golden passes through untouched), and plain `:Q{n}` numbering is unshifted by
the new `:QO{n}` space (its own golden). Mutation-test the `emit_drop` owning arm and the
`call`-site disposal separately — stubbing either to a no-op must fail its corresponding
path's fixture above, which a single combined fixture could not distinguish.

**Phase 4.** A 2+-linear-capture escaping `^[ ]` closure builds, runs, and disposes both
exactly once. The **numbering-invariant fixture**: a capturing escaping owning closure in a
word that also contains a preceding quotation (including the branch-arm shape), asserting
the captured linear value is disposed exactly once — a placebo-proof against the one-
quotation-per-word fixtures. Mutation-test the minting pass's `is_bundle: false` (flipping
to `true` drops the destructor and fails the exactly-once fixture). The no-capture and
scalar-only paths keep the plain two-word layout and no drop (regression).

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Checker classifier fix: classify_capture's aggregate arm gains an is_copy check so a Copy aggregate/enum capture classifies as Scalar and admits at an escaping boundary. Thread structs/enums/arrays down, which DOES change check_capture_admission's parameter list (Ctx has no arrays() accessor; arrays is held only by materialize_quotation_at_boundary) -- a single-caller immutable reborrow. In scope: migrating tests/phase4_quotations.rs:336 escaping_closure_over_frame_local_is_past_owning_frame, which captures [i64 4] (Copy per builtins.rs:243) and therefore flips from reject to admit, plus a mandatory replacement witness that a genuinely linear by-value aggregate (drop-overloaded struct or OwnedCell) still classifies FrameRooted. No new runtime mechanism, no IL change.",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Add Type::OwningQuotation(&'static QuotEffect) mirroring Type::InlineQuotation, with owning_quotation_type and a distinct ^[ ... ] name_static spelling; a NEW glued Token::CaretLBracket (mirroring TildeLBracket at lexer.rs:27/199, since ^ is not a token and [ is a delimiter) plus its parse_poly_slot/parse_slot arms and lexer unit test; the whole-crate exhaustive-match audit driven by an explicit per-site admit/reject table (do NOT mirror slice 10a's polarity: InlineQuotation rejects at materialization boundaries, OwningQuotation must be admitted; note is_quotation_type must return Some or call breaks at terms.rs:324/327, which also defers owning-name captures via the captures.rs:236 case-4 guard); is_copy returns false for it so is_linear is true and move tracking/dup-gate/forgotten-value are inherited; structural PartialEq inequality at every boundary and if-join. No disposal and no runtime representation, enforced by an explicit CHECK-SIDE materialization guard with a located diagnostic (ir_type_of returns IrType not Result and its arm would be an ICE; a non-capturing ^[ 42 ] bypasses capture admission entirely). Type-level and guard tests only.",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Owning runtime representation and the call-once lifecycle: IrType::OwningQuotation(QuotSigId), its own three-word OwningQuotLayout (code/drop/env) without widening QuotLayout, a new { l, l, l } backend aggregate in a separately-numbered :QO{n} space parallel to qbe.rs:151, ir_type_of/collect_quot_sigs plumbing, and emit_drop's owning arm (indirect call through drop, no null branch). materialize_quot_value builds a single-linear-capture ^T env cell interned inline and sets drop to its cell_drop_symbol; lift past_owning_frame for a single frame-rooted linear capture at an owning boundary (value moved into the heap cell). call on an owning receiver consumes AND disposes it (call-once): lowering emits the indirect call through code then the env disposal through drop, with no checker change since call already pops its receiver. Plus the located guard rejecting a body that moves a captured linear value out rather than reading it, keeping disposal uniform (dispose payload then free block, one symbol, no flag). Exactly-once single-capture disposal goldens covering both the called and the dropped-uncalled path, IL assertions, and the guard's own golden plus mutation test; plain two-word layout and :Q{n} numbering unchanged.",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Multi-capture escaping owning env: SurvivingCapture carries the capture Type; a new &mut Vec<OwningEnvRequest> carrier threaded through check_word mirroring sites, drained by a post-body-walk minting pass in check_module that mints a per-literal env StructDecl (is_bundle false) interned via intern_owned_cell_type and records OwnedCellId in a new Module symbol->cell map (breaking the exhaustive Module {..} literals at ast.rs:2731/2885/2953 and the parser); the map threaded into lowering as a lower_word_parts parameter set post-construction like b.instantiations, empty on destructor/monomorph paths; materialize_quot_value builds the multi-field cell; multi_capture_escaping_error and its R22 twin lift for the escaping owning path only. Exactly-once multi-capture goldens plus the check/lower numbering-invariant fixture (a preceding quotation in the same word).",
      "difficulty": "hard"
    }
  ]
}
```
