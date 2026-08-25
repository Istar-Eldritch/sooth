# Phase 7 Slice 3v: dropping and storing a linear-capturing quotation

**Status:** specified, not implemented.
**Discovery:** `docs/roadmap/P7/slice3v-brief.md` (written and re-verified against `1dabcbc`;
this spec re-verified every load-bearing citation against `3b85119`).
**Roadmap:** `docs/roadmap/P7-language-prereqs.md`, the **P7.S3v** entry, bounded by the
**P7.S3u** (parked, not a prerequisite) and **P7.S5** (linear array elements, not this slice's)
entries.
**Predecessor:** `docs/roadmap/P7/slice3h-spec.md` — ships the owning-closure type and its two
restrictions, both lifted here for three positions only.

## Problem

**P7.S3h** shipped `owning [ … ]`: a closure whose type marks a disposal obligation, discharged
by `call`. Two restrictions exist because nothing can invoke a per-value disposer:

**(i) `drop` on an owning closure is a located rejection**, twinned on the concrete
(`src/check.rs:3367-3368`) and generic (`src/check/poly.rs:1254-1255`) paths, both calling
`cannot_drop_owning_quotation_error` (`src/check.rs:3009`). Probed against `3b85119`:

```
import: intrinsics * ;
type: Spy tag i64 ;
: drop ( Spy -- ) | s | "drop " . s Spy> . ;
: mk ( Spy -- owning [ -- ] ) | s | [ s drop ] ;
: main ( -- ) 7 Spy mk drop ;
\ error: cannot `drop` a value of type `owning [ -- ]` in `main` (line 5): an owning closure
\   disposes its captures by running, so `call` it -- no destructor can run a closure body
```

**(ii) An owning closure may not sit in an aggregate position.** One audit,
`reject_quotation_type_position` (`src/check/audits.rs:510`), dispatches on `is_quotation_type`
across every declared position; `audit_quotation_type_registries`
(`src/check/audits.rs:158-211`) walks struct fields, enum variant fields, array elements, cell
payloads and reference referents. Re-probed per position, one fixture each, against `3b85119`:

| Position | Result today | Whose gate |
|---|---|---|
| struct field | rejected: "cannot appear as the field `q` of struct" | **this slice** |
| enum variant field | rejected: same audit, "field `q` of enum variant" | **this slice** |
| owned-cell payload, spaced (`^ owning [ -- ]`) | rejected: "cannot appear as an owned-cell payload" | **this slice** |
| owned-cell payload, glued (`^owning [ -- ]`) | rejected: `unknown type \`owning\`` at the `^` remainder — the parser never reaches the audit | **this slice**, parser fix |
| array element | rejected: `linear array elements are not supported yet` | **P7.S5** |
| slice element | rejected: a view does not own what it points at | **P7.S5** |

The array and slice rows are not this slice's gate. Probed: `type: Arr xs [Spy 2] ;` fails with
the identical "linear array elements are not supported yet" message for an ordinary linear
struct, with no `owning` anywhere in the source — `audit_quotation_type_registries`'s own array
loop (`:195-203`) already carves out a *plain* `Type::Quotation` element (a pre-existing D4
materialization boundary) but not an owning one, so an owning array element does still reach
`reject_quotation_type_position` today. Whether that audit or the general non-`Copy`-element gate
in `src/check/declarations.rs` is the one that actually fires first is immaterial to this slice:
**this slice does not touch the array loop, the general element gate, or any slice-element
gate.** Widening the array carve-out to admit `OwningQuotation` is explicitly out of scope (R1) —
doing so would let an owning closure past the audit into whatever gate fires second, and that
gate's own soundness for a per-construction-site disposer is P7.S5's question to answer, not
this slice's.

**Why lowering cannot currently be reached even if the checker admitted these positions.**
`emit_drop` (`src/ir/func_builder/quotation.rs:368-390`) has no arm for a quotation type — its
`_ => {}` swallows one silently. `field_is_linear` (`src/ir/layout.rs:66-80`) and
`layout_field_is_linear` (`:895-911`) likewise fall through to `false` for a quotation type, so
no container's `is_linear` fold sees an owning field, and `synthesize_aggregate_destructors`
(`src/ir/destructors.rs:37`) never emits a destructor for it. If the checker gates were deleted
without any of this, an owning field would fall through the whole pipeline and leak both the
capture and the heap env block, silently, on every container `drop`.

## Design

### R1 — The disposer is a third word in the shared quotation value, keyed on the construction site

`Type::OwningQuotation` carries only the declared effect (`src/ast.rs:2295`), so two closures
with identical effects and different capture sets are one type — nothing type-directed (a
`Drop` trait, a specialized `impl:`, a trait-object vtable) can discriminate them. The value
itself grows a third word, populated at the one place the capture's concrete types are known:
`materialize_quot_value` (`src/ir/func_builder/quotation.rs:54`), where a compiler-synthesized
disposer symbol is minted per literal, exactly as `code`'s `FuncAddr` already is.

`quotation_layout` (`src/ir/types.rs:243-250`) widens from `2 * word_width` to `3 * word_width`,
gaining a `disposer_offset` field on `QuotLayout` (`:236-241`) at `2 * word_width`. Every field
offset used by lowering is read from this struct (`materialize_quot_value`,
`lower_indirect_call` at `src/ir/func_builder/quotation.rs:203`), so no call site needs an
offset literal edited.

**Every quotation value carries the third word, owning or not** — the alternative (a
per-variant width) is rejected in the brief: `:Q{n}` is keyed on effect alone
(`quot_index`, `src/backend/qbe.rs:57-62`), collapsing a plain and an owning quotation of the
same effect to one symbol today, and the doc comment at `src/ir/types.rs:185-197` already
asserts they are byte-for-byte identical. Diverging the width would re-key that symbol on
effect *and* owning-ness across every site that maps both variants together
(`src/backend/qbe.rs:405-413,449-463`), for an 8-byte saving on the minority case (a
materialized rather than inlined closure). **This is the canary named in the brief: if a phase
finds itself editing `types.rs:185-197` to say the two variants' widths differ, the ruling has
been reversed by accident — stop and report it, do not proceed.** A non-owning quotation's third
word is always the null pointer, mirroring the existing null-env convention for a non-capturing
literal.

Backend sites touched, all mechanical width/text changes with no shape decision:

- `src/backend/qbe.rs:151` — the hardcoded `type :Q{idx} = { l, l }` → `{ l, l, l }`, the only
  literal-string spelling of the width.
- `src/ir/types.rs:700-704` (`ir_type_of_quotation_is_two_slot_aggregate`) — the
  `assert_eq!(layout.size, 2 * WORD_WIDTH)` becomes `3 * WORD_WIDTH`, and the test gains a
  `disposer_offset` assertion at `2 * WORD_WIDTH`.
- Every QBE IL golden carrying the literal `type :Q{n} = { l, l }` — a mechanical sweep, not a
  design problem, called out here so a phase does not treat a wave of golden diffs as a
  regression.

`width`/`qbe_abi_ty`/`member_ty`/`field_store_op` (`src/backend/qbe.rs:395-463,520-540`) already
pair `IrType::Quotation` and `IrType::OwningQuotation` in every arm (S3h's doing); none of them
spell the width literally, so none change here.

### R2 — The disposer's body composes existing per-type disposal; it invents no dispatch

At each owning-closure construction site, `build_owning_env` (`quotation.rs:101`) already lays
out one heap slot per capture via `owning_env_slots` (`src/ir/func_builder/mod.rs:752`) with a
known `(offset, IrType)` per capture. The new disposer is a synthesized `IrFunc`, one per
literal (named `{symbol}__dispose`, alongside the existing `{enclosing}__quot{id}` body
symbol), taking the env pointer as its sole parameter:

1. For each capture, at its known offset: `FieldLoad` the value, then call the **existing**
   `FuncBuilder::emit_drop` (`quotation.rs:368-390`) on it — the exact primitive a struct's own
   field-glue destructor already calls per field. A scalar or borrow capture takes `emit_drop`'s
   `_ => {}` no-op arm, matching what the closure body's own prologue does with one (uses it,
   never frees it); a linear struct/enum/cell capture takes its existing arm and calls that
   type's own `struct_drop_symbol`/`enum_drop_symbol`/cell destructor.
2. Free the block (`FREE_SYMBOL`), guarded exactly as `build_owning_env`'s existing zero-capture
   case: a capture-free literal stores a null disposer, mirroring its null env, and needs no
   synthesized function at all.

This reuses `emit_drop` rather than re-deriving per-type disposal, so the disposer is not new
recursive machinery — it is the same fold `synthesize_struct_destructor`'s field loop performs,
applied to an anonymous capture list instead of a declared field list.

### R3 — `emit_drop` gains the one arm that makes both `drop` and containment work

`emit_drop`'s match (`quotation.rs:368-390`) gains:

```rust
IrType::OwningQuotation(_) => {
    // load the layout's disposer slot; if non-null, indirect-call it with
    // the loaded env slot as its sole argument (mirrors lower_indirect_call's
    // code-slot load, R1's third word instead of the first)
}
```

guarded by a null check on the disposer slot, symmetric with the null-env convention. This one
arm is what both consumers below reduce to:

- **`drop` on a bare owning closure value** discharges through the ordinary `"drop"` shuffle
  arm in `src/check.rs` and its poly twin — once the checker stops rejecting the type (R4), the
  value reaches `FuncBuilder::emit_drop` exactly like a struct value does, and this arm fires.
  This is a **new consuming use, distinct from `call`**: `call` runs the closure's own code,
  which may do arbitrary work before disposing its captures via the body's own logic; `drop`
  runs *only* the disposer, discarding the closure unexecuted. Both are legal, and they run
  different code — this is the "discarding one unexecuted" capability S3h's own out-of-scope
  section named for this slice.
- **A container's `drop`** (a struct/enum holding an owning field) reaches this arm through the
  *existing, unmodified* `synthesize_struct_destructor`/`synthesize_enum_destructor` field-glue
  loop, once `field_is_linear` (R5) tells that loop the field is linear at all. No change to
  destructor synthesis itself is needed — R5 is the whole of what makes the existing machinery
  see the field.

### R4 — Delete the twinned `drop` rejection

`src/check.rs:3367-3368` and `src/check/poly.rs:1254-1255` (both matching `OwningQuotation` and
calling `cannot_drop_owning_quotation_error`) are deleted. `cannot_drop_owning_quotation_error`
itself (`src/check.rs:3009`) is deleted too if nothing else calls it (verify with a
workspace-wide reference check before removing; if some other located error reuses its message
text, keep the function and just drop the two call sites).

This is a **twinned guard** (this project's own repeat failure mode): the monomorphic path
(`check.rs`'s `"drop"` shuffle arm) and the generic path (`poly.rs`'s `"drop"` arm, reached only
from an actual generic body) must each be **mutation-tested independently** — restore each
deleted arm in isolation and confirm the corresponding migrated test (R7) fails. The generic-path
test must go through a real generic body (mirroring S3h's own
`dropping_an_owning_closure_in_a_generic_body_is_a_located_rejection`, which calls a poly word
returning an owning closure built by an ordinary one), not a monomorphic program that happens to
type-check under the poly checker.

### R5 — Widen the two linearity folds, and only them

`field_is_linear` (`src/ir/layout.rs:66-80`) and `layout_field_is_linear`
(`:895-911`) each gain:

```rust
IrType::OwningQuotation(_) => true,
```

matching `IrType::OwnedCell(_) => true` immediately above. `IrType::Quotation` (plain) stays on
the `_ => false` wildcard — no change, no new `Copy` obligation, no IL churn for any program that
does not use `owning`. This is the one edit `check/audits.rs`'s own doc comment
(`:922`, in `an_owning_field_is_rejected`'s neighborhood) names as the hole this slice closes on
purpose: with it, `StructLayout::is_linear`/`EnumLayout::is_linear` see an owning field, so
`synthesize_aggregate_destructors` emits a destructor for the container, and that destructor's
field-glue loop calls `emit_drop` on the field, which is R3's new arm.

No change to `crate::check::is_copy`: its `Type::OwningQuotation` arm has answered `false` since
S3h, so the checker already treats a struct with an owning field as non-`Copy`/linear. This gap
was purely on the IR/backend side.

### R6 — Lift the containment audit for three positions only

`audit_quotation_type_registries` (`src/check/audits.rs:158-211`) gains three carve-outs, not
two mirrored ones. The struct-field loop already carves out plain `Type::Quotation` (`:169-171`,
R8/D4), which this slice widens:

```rust
if matches!(fty, Type::Quotation(_)) { continue; }
```

to

```rust
if matches!(fty, Type::Quotation(_) | Type::OwningQuotation(_)) { continue; }
```

**The enum-variant loop (`:180-191`) has no such carve-out today, for either quotation flavour**
— probed: `type: E | A q [ -- ] ;` (a *plain* quotation variant field) is rejected on `3b85119`
with "cannot appear as the field `q` of enum variant", the identical message the struct-field
case gets *before* R8/D4's own carve-out. This is not a sibling to mirror; it is its own new
carve-out, admitting only the owning flavour (a plain quotation variant field stays out of scope
and rejected, matching S3h's own scoping of D4 to struct fields only):

```rust
if matches!(fty, Type::OwningQuotation(_)) { continue; }
```

and a **third, separately new** carve-out on the owned-cell loop (`:204-206`), which today has
*no* plain-quotation exception at all (a plain `Type::Quotation` cell payload is out of scope,
unchanged, and stays rejected):

```rust
if matches!(c.payload, Type::OwningQuotation(_)) { continue; }
```

**Nothing else in this function changes.** The array loop (`:195-203`) keeps its existing
carve-out unchanged (plain `Quotation` only); the reference-referent loop (`:207-209`) gains no
carve-out. An owning closure behind a reference, as an array element, or at an `extern:`
boundary stays exactly as rejected as it is on `3b85119`.

### R7 — The owned-cell parser gap: the glued `^owning` form

`^i64` lexes as one glued `Word` token; `split_owning_cell_word`
(`src/parser.rs:3592-3617`) peels the leading `^`-run and recurses on the remainder. For
`^owning`, the remainder is the bare string `"owning"`, which matches none of the function's
existing arms (empty / starts with `'` / else) and falls to `resolve_type_or_apply("owning",
…)`, reporting `unknown type \`owning\``. Reproduced on`3b85119`:

```
: mk ( -- ^owning [ -- ] ) 0 . [ 0 . ] ;
\ error: unknown type `owning` at line 2, col 12
```

The spaced form (`^ owning [ -- ]`, `^` and `owning` as two tokens) already works today —
`parse_owning_cell_type_expr`'s empty-remainder branch (`:3595-3603`) recurses into
`parse_type_expr`, which already dispatches on `owning_quotation_ahead()` (`:3542-3543`).
Reproduced: it reaches the audit and is rejected with "cannot appear as an owned-cell payload",
confirming the parser is not the blocker for that spelling.

**Fix, scoped to the glued form only:** `split_owning_cell_word`'s remainder match gains an arm
for `remainder == OWNING_QUOTATION_KEYWORD`, calling the same quotation-effect reader
`parse_owning_quotation_type_expr` uses, then folding the result through `intern_owned_cell_type`
exactly as every other arm does. No lexer change (`OWNING_QUOTATION_KEYWORD` stays an ordinary
word, matching S3h's own no-lexer-change ruling); no change to `parse_type_expr`'s own dispatch,
which already handles the spaced form correctly.

### R8 — The REPL override-epoch obligation

`src/ir/destructors.rs:8-35`: once a session holds any user `drop` override, every linear
struct's/enum's/cell's destructor is epoch-suffixed session-wide, because any of them may
transitively call the overridden one. The per-construction-site disposer (R2) calls into those
destructors through `emit_drop` exactly as a struct's own field glue does, so it inherits the
same obligation — the disposer is minted at `materialize_quot_value`, in the same
`self.materialized`/lowering path both `repl.rs` entry points already thread through
`synthesize_aggregate_destructors` with `self.apply_drop_generations` (`src/repl.rs:3198`,
`:3391`, the sites already citing "R12: this module/line must carry its own struct/enum
destructors"). **No new plumbing is required for the disposer to see the live epoch** — it calls
`emit_drop`, and `emit_drop`'s existing arms already resolve `struct_drop_symbol`/
`enum_drop_symbol` against `self.structs.layouts[..].drop_generation`, which
`apply_drop_generations` has already set before lowering runs. The risk is not a missing wire; it
is an **untested** one, whose failure mode is a silent `dlopen` link failure
(`src/repl.rs`'s `Library::open`), not a diagnostic — exactly why the brief requires a dedicated
golden rather than trusting the wiring by inspection.

## Codebase map

| Anchor | Role in this slice |
| --- | --- |
| `src/ir/types.rs:236-250` | `QuotLayout`/`quotation_layout` — R1's width and new `disposer_offset` |
| `src/ir/types.rs:185-197` | the byte-identical-variants doc comment — update to name the third slot; the canary (R1) |
| `src/ir/types.rs:700-704` | the two-slot-aggregate unit test — R1's size/offset assertions |
| `src/backend/qbe.rs:151` | the hardcoded `:Q{idx}` type string — R1's only literal-width site |
| `src/ir/func_builder/quotation.rs:54-90` | `materialize_quot_value` — where R1's disposer symbol is minted |
| `src/ir/func_builder/quotation.rs:101-124` | `build_owning_env` — R2's capture offsets, reused not rebuilt |
| `src/ir/func_builder/mod.rs:752` | `owning_env_slots` — the offset/type table R2's disposer body reads |
| `src/ir/func_builder/quotation.rs:368-390` | `emit_drop` — R3's new `OwningQuotation` arm |
| `src/check.rs:3009,3367-3368` | `cannot_drop_owning_quotation_error` + the mono `drop` guard — R4 |
| `src/check/poly.rs:1254-1255` | the poly-path twin of the same guard — R4 |
| `src/ir/layout.rs:66-80,895-911` | `field_is_linear`/`layout_field_is_linear` — R5 |
| `src/check/audits.rs:158-211` | `audit_quotation_type_registries` — R6's three carve-outs |
| `src/check/audits.rs:510` | `reject_quotation_type_position` — unchanged, reached only for the positions still rejected |
| `src/parser.rs:3592-3617` | `split_owning_cell_word` — R7's glued-token fix |
| `src/parser.rs:3528-3548` | `parse_type_expr`'s `owning_quotation_ahead()` dispatch — unchanged, already handles the spaced form |
| `src/ir/destructors.rs:8-35` | the override-epoch rule — R8, unchanged, just newly exercised |
| `src/repl.rs:3198,3391` | the two `synthesize_aggregate_destructors` call sites the epoch already flows through |
| `tests/phase7_slice3h.rs:169,187,335,354` | the four tests R4/R6 migrate, not delete |

## Tests

End-to-end, `tests/phase7_slice3v.rs` (through the real binary; every negative pins the exact
diagnostic string):

- **Migrated from `tests/phase7_slice3h.rs`** (delete from that file, add here with the new
  assertion):
  - `an_owning_quotation_field_is_rejected` (:169) → **admitted**, and its `Spy` capture is
    disposed exactly once when the containing struct is `drop`ped without the closure ever being
    `call`ed.
  - `an_owning_quotation_variant_field_is_rejected` (:187) → same, for an enum variant field.
  - `dropping_an_owning_closure_is_a_located_rejection` (:335) → **admitted**: `drop` on a bare
    owning closure disposes its capture exactly once, and the closure's own body (which would
    print something `call` does not) never runs — assert the captured side effect fires once and
    the body-only side effect does not fire at all.
  - `dropping_an_owning_closure_in_a_generic_body_is_a_located_rejection` (:354) → same,
    through a generic body (`g ( 'T: Copy -- 'T ) | x | mk drop x ;` shape), pinning the
    generic-path mutation guard (R4).
- `an_owning_quotation_cell_payload_is_admitted_spaced` and
  `an_owning_quotation_cell_payload_is_admitted_glued` (R6/R7) — `^ owning [ -- ]` and
  `^owning [ -- ]` both build; `^>` extracts and `call`s or `drop`s the closure, disposing its
  capture once either way.
- `an_owning_cell_of_owning_quotation_is_disposed_on_cell_drop` — `drop` on the *cell itself*
  (never unwrapped) disposes the capture exactly once, exercising the cell's own destructor
  calling into R3's `emit_drop` arm rather than a user unwrapping it first.
- `call_and_drop_run_different_code` (R3's headline) — a closure whose body prints one message
  and whose capture's own `drop` prints another; `call`ing it prints both (body, then capture);
  `drop`ping it prints only the capture's — pinned as two separate goldens on the same source,
  branching on the last line.
- `an_owning_field_disposes_alongside_its_siblings_exactly_once` — a struct with an owning field
  *and* a plain linear field (e.g. two `Spy`s), `drop`ped once: both captures' disposal messages
  appear exactly once each, in field order — guards against the disposer double-running or the
  container's own field glue double-visiting the quotation field.
- `an_array_element_owning_closure_is_still_rejected` and
  `a_reference_referent_owning_closure_is_still_rejected` — the two positions R6 explicitly
  leaves alone stay byte-identical to `3b85119`'s messages; these are the blast-radius guard on
  R6's carve-outs, not new coverage.
- `an_owning_cell_payload_of_a_plain_quotation_is_still_rejected` — the un-widened half of R6's
  new cell carve-out: a *plain* (non-owning) `^[ -- ]` stays rejected, unchanged.
- `explicit_repl_override_epoch_disposal` (R8, required) — at the REPL: define a struct with a
  user `drop` override, then on a later line build an owning closure capturing a *different*
  linear struct (not the overridden one) inside a struct field, and `drop` the container. Without
  R8's epoch flowing to the disposer's `emit_drop` call this dies at `dlopen` with an undefined
  `sooth_struct_drop_N`; with it, the session prints the capture's disposal line. This must run
  through the actual `repl` test harness (not `build_and_run`), since the failure mode is a link
  error a native build's single-module destructor set cannot reproduce.

Unit, beside each touched function:

- `src/ir/types.rs`: `quotation_layout`'s three offsets/size/align, mirroring the existing
  two-slot test.
- `src/ir/func_builder/quotation.rs`: `emit_drop` called directly on a constructed
  `IrType::OwningQuotation` value — asserts the emitted instructions contain a null check, a
  `FieldLoad` of the disposer slot, and a `CallIndirect`; and on a null-disposer value, asserts
  no call is emitted. Constructed directly rather than through a full build, since phase 2 lands
  before the checker admits any program that reaches this arm for real (R4/R6 land in phase 3).
- `src/ir/layout.rs`: `field_is_linear`/`layout_field_is_linear` on a constructed
  `IrType::OwningQuotation(_)`, asserting `true`, alongside the existing `OwnedCell`/`Struct`
  cases.
- `src/parser.rs`: `split_owning_cell_word` on `^owning [ -- ]`, asserting the interned
  `Type::OwnedCell` payload is `Type::OwningQuotation`; a control asserting `^Spy` is unaffected.
- `src/check/audits.rs`: `audit_quotation_type_registries` directly, admitting an owning struct
  field/variant field/cell payload and still rejecting an owning array element and reference
  referent — the R6 carve-out's own mutation guard, mirroring the existing
  `poly_quotation_behind_a_reference_inside_an_array_element_is_rejected`-style direct-call
  tests already in this file.

**Mutation-test before each phase exit**, deleting what each guards and proving the named test
fails:

- R3's null check on the disposer slot (force an unconditional call) — a capture-free owning
  closure's `drop` (or its container's) must fault or misbehave observably in the unit test that
  constructs a null-disposer value.
- R4's two guards, independently (restore each deleted arm in isolation) — the corresponding
  migrated `drop`-is-now-legal test must fail with each restored.
- R5's two widened folds, independently (`field_is_linear` and `layout_field_is_linear`,
  restoring each to its `_ => false` wildcard alone) — `an_owning_quotation_field_is_rejected`'s
  migrated form must fail (container drop leaks/no-ops) with either one reverted.
- R6's three carve-outs, independently — each of the three admitted-position goldens must fail
  when its own carve-out alone is reverted, and the two still-rejected goldens must keep passing
  throughout (proving the carve-outs are additive, not a widened wildcard).
- R7's glued-form fix — reverting `split_owning_cell_word`'s new arm must fail
  `an_owning_quotation_cell_payload_is_admitted_glued` while leaving the spaced-form golden
  passing (proving the two forms are genuinely two different code paths, not one).

## Phase 1 — widen the quotation value layout (hard)

**Scope.** `src/ir/types.rs` (`QuotLayout`, `quotation_layout`, the doc comment, the unit test),
`src/backend/qbe.rs:151`, and the QBE IL golden sweep the width change forces.
`materialize_quot_value` (`quotation.rs:54-90`) is touched only to write a null pointer into the
new third word unconditionally (both flavours), so every existing owning-closure golden (S3h's)
stays byte-for-byte correct on the `code`/`env` slots and gains one more null word nobody reads
yet — a real, immediate consumer of the new offset (not pre-staged plumbing: the write happens
in this phase, at the site that already writes the other two slots).

**Out of bounds.** `emit_drop`, `field_is_linear`/`layout_field_is_linear`, the checker guards,
the audit, the parser, the REPL, any disposer synthesis.

**Entry.** `3b85119`, green.

**Exit.** `quotation_layout(WORD_WIDTH).size == 3 * WORD_WIDTH`; `disposer_offset ==
2 * WORD_WIDTH`; every quotation-bearing golden (plain and owning) still builds and runs
byte-identically on program *output*, with its IL golden's `:Q{n}` declaration updated to `{ l,
l, l }`; the doc comment at `types.rs:185-197` still asserts variant identity, now naming three
slots; `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
--no-fail-fast` green.

## Phase 2 — disposer synthesis and the `emit_drop` arm (hard)

**Scope.** `src/ir/func_builder/quotation.rs` (a new disposer-synthesis method beside
`build_owning_env`, wired into `materialize_quot_value` to replace phase 1's unconditional null
write with a real symbol whenever `owning_env_slots` reports at least one capture; R3's new
`emit_drop` arm), the `emit_drop` unit tests (constructed values, per the Tests section above).

**Out of bounds.** `src/check.rs`, `src/check/poly.rs`, `src/check/audits.rs`, `src/parser.rs`,
`src/repl.rs`, `src/ir/layout.rs`. Nothing in this phase is reachable by any program the checker
currently admits — every consumer is the direct unit-level `emit_drop` call named in Tests, which
is this phase's own, real, in-phase consumer, not a later one.

**Entry.** Phase 1 landed and green.

**Exit.** `emit_drop` on a constructed `IrType::OwningQuotation` value with a non-null disposer
emits a null check, a `FieldLoad` of the disposer slot, a `FieldLoad` of the env slot, and a
`CallIndirect`; with a null disposer it emits nothing. `materialize_quot_value` mints a real
`{symbol}__dispose` `IrFunc` for a capturing owning literal and keeps writing null for a
capture-free one. Every existing S3h golden (`call`-only usage) still passes unchanged, since
nothing yet calls the new symbol at runtime. Full green.

## Phase 3 — checker: delete the twinned guard, lift the audit, fix the parser gap (hard)

**Scope.** `src/check.rs:3009,3367-3368` and `src/check/poly.rs:1254-1255` (R4, both halves,
each independently mutation-tested), `src/ir/layout.rs:66-80,895-911` (R5, both functions,
independently mutation-tested), `src/check/audits.rs:158-211` (R6, three carve-outs,
independently mutation-tested), `src/parser.rs:3592-3617` (R7), the four migrated tests, and
every new end-to-end golden in the Tests section except the REPL one.

**Out of bounds.** `src/repl.rs` (phase 4), `src/ir/func_builder/`, `src/ir/types.rs`,
`src/backend/qbe.rs`.

**Entry.** Phase 2 landed and green.

**Exit.** All four migrated tests pass in their new (admitting) form and are removed from
`tests/phase7_slice3h.rs`; `an_owning_quotation_field_is_rejected` and its variant-field sibling
no longer exist under those names in that file. Every mutation check named in Tests for R4/R5/R6/
R7 fails as specified when its guard is individually reverted. The array-element and
reference-referent still-rejected goldens pass unchanged. `call_and_drop_run_different_code`
passes both branches. Full green.

## Phase 4 — the REPL override-epoch golden (standard)

**Scope.** `tests/`-level REPL harness test only (`explicit_repl_override_epoch_disposal`);
`docs/roadmap/P7-language-prereqs.md`'s S3v entry, closed out; `docs/roadmap/P7/slice3h-spec.md`'s
"Out of scope" line naming storage/discard as S3v's follow-on, updated to point at this document
instead of describing it prospectively.

**Out of bounds.** Any source file. If this phase needs a source edit, R8's claim that no new
plumbing is required was wrong, and that is the finding to report rather than a quiet fix folded
into a "docs" phase.

**Entry.** Phase 3 landed and green.

**Exit.** `explicit_repl_override_epoch_disposal` passes, run through the REPL harness, and
fails (an undefined-symbol `dlopen` error, not a checker diagnostic) if `apply_drop_generations`
is stubbed out for the test — confirmed once, then left as a comment rather than a permanent
stub, since disabling session-wide epoching is not a state this slice's own tests should leave
toggleable. Full green.

## Out of scope

- **Array and slice element positions**, for an owning closure or any other linear type — P7.S5.
  This slice's audit carve-outs (R6) are additive over exactly three positions and must not be
  widened to a fourth even if doing so would "just work" once R5 lands; that is exactly the kind
  of accidental scope creep the blast-radius goldens in Tests exist to catch.
- **P7.S3u** (trait objects / erased owners) — parked, not a prerequisite, not touched.
- Polymorphism over plain-versus-owning quotation types, and an owning parameter on a spliced or
  generic word — unchanged since S3h (`reject_owning_quotation_declarations`,
  `src/check/audits.rs:476`, untouched by this slice).
- Inline and static env storage for an owning closure — unchanged since S3h.
- A user-declared `drop` overload's interaction with an owning-quotation *field* specifically
  (i.e. can a struct holding one also declare its own `drop` body instead of the synthesized
  glue) — the existing override machinery (`DropOverrides`) is generic over any linear field and
  needs no new case for this one; not separately tested here beyond the ordinary override path
  already covered by `tests/`'s slice 8b coverage.
- The REPL's pre-existing inability to link a materialized quotation via `RTLD_GLOBAL`'s
  non-PIC relocation limits — a *bare* (`call`-only) owning closure joins that existing failure
  class exactly as it did in S3h; this slice's own REPL golden is scoped to the epoch obligation
  specifically, using the field/`drop` shape that avoids that unrelated limit.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "widen the shared quotation value layout to three words and sweep the QBE IL goldens", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "synthesize the per-construction-site disposer and add the emit_drop arm that calls it", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "delete the twinned drop guard, widen field_is_linear, lift the audit for struct/variant/cell positions, fix the glued ^owning parser gap", "effort": "L", "difficulty": "hard" },
    { "phase": 4, "focus": "the REPL override-epoch golden closing out P7.S3v", "effort": "S", "difficulty": "standard" }
  ]
}
```
