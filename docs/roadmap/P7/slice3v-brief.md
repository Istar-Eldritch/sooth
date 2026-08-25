# P7.S3v brief — Dropping and storing a linear-capturing quotation

## Problem, confirmed live against current `main` (`1dabcbc`)

**P7.S3h** shipped owning closures with two restrictions. Both exist because nothing can
invoke a per-value disposer, and both are located rejections today.

**(i) `drop` on an owning closure is rejected.** Probed:

```
import: intrinsics * ;
type: Spy tag i64 ;
: drop ( Spy -- ) | s | "drop " . s Spy> . ;
: mk ( Spy -- owning [ -- ] ) | s | [ s drop ] ;
: main ( -- ) 7 Spy mk drop ;
\ error: cannot `drop` a value of type `owning [ -- ]` in `main` (line 5): an owning closure
\   disposes its captures by running, so `call` it -- no destructor can run a closure body
```

Control: the same program ending `mk call` builds and prints `drop 7`.

This is a **twinned** guard, not one: `src/check.rs:3370` for the monomorphic path and
`src/check/poly.rs:1255` for the generic path, each matching `Type::OwningQuotation` and each
returning `cannot_drop_owning_quotation_error`.

**(ii) An owning closure may not sit in an aggregate position.** One shared guard,
`reject_quotation_type_position` (`src/check/audits.rs:510`), dispatches on
`is_quotation_type()` across struct field, enum variant field, array element, owned-cell
payload and reference referent.

**Only some of those positions are this slice's to lift, and the roadmap was wrong about
that until `1dabcbc`.** Probed, one fixture per position:

| Position | Result today | Whose gate |
|---|---|---|
| struct field | "a quotation type ... cannot appear as the field `q` of struct" | **S3v** |
| enum variant field | same audit rejection | **S3v** |
| owned-cell payload (`^`) | "... cannot appear as an owned-cell payload" for `^[ -- ]`; the `owning` form does not even resolve, giving "unknown type owning" | **S3v**, plus a parser gap |
| array element | `linear array elements are not supported yet` | **P7.S5**, not S3h |
| slice element | a view does not own what it points at | out of scope (**P7.S5**) |

The array row is the important one. That rejection is **not** about owning closures:
`type: Arr xs [Spy 2] ;` fails identically, while `type: Box s Spy ;` builds. A linear array
element is unsupported for every linear type, which is a P3-era restriction now named as
**P7.S5**. S3v cannot deliver it and must not claim it.

The owned-cell row is the newly-established one: `^Spy` builds, so cells do admit linear
payloads, and `^[ -- ]` hits the quotation audit. So the cell position *is* S3v's, but
`owning` is not parsed after `^` at all, which is a small parser fix this slice owns.

**Why lowering cannot currently be reached.** `emit_drop`
(`src/ir/func_builder/quotation.rs`, the `_ => {}` at `:389`) has no arm for a quotation type.
If the checker gates were simply deleted, an owning-closure field would fall through and leak
both the capture and the heap env block, silently. `field_is_linear` (`src/ir/layout.rs:66`)
and `layout_field_is_linear` (`:895`) likewise fall through to `false` for quotation types, so
no container would consider itself linear on account of one.

## Existing precedent (what's already there to build on)

**The value layout was designed for exactly this widening.** `src/ir/types.rs:230-235`:

> the fixed two-slot layout every quotation value shares … `code` at offset 0, `env` at
> offset `word_width`, size `2 * word_width` … The `env` slot is always the null pointer in
> 7a (7b fills it); **it is not elided, so widening to a capturing closure stays additive.**

So the third word is an anticipated move, not a reshape. Every field offset is read from the
`QuotLayout` struct (`quotation.rs:108`, `:116`, `:274`, `:280`), so those sites need no edit.

**The per-type disposer already exists.** `struct_drop_symbol` and the synthesized aggregate
destructors (`src/ir/destructors.rs`, `synthesize_aggregate_destructors`) already give every
linear type a callable disposer, and `src/ir/destructors.rs:29` records that substituting a
user `drop` body under that same symbol *is* the whole of dispatch, since no call site
resolves a `drop` overload by name. The new disposer composes those; it invents no dispatch.

**An indirect call through a code pointer already happens.** `lower_indirect_call`
(`quotation.rs:203`) is the precedent for calling through a slot in the value.

## Design ruled here

**The disposer is keyed on the construction site, not on the type.** `Type::OwningQuotation`
(`src/ast.rs:2295`) carries only the effect, so two closures with identical effects and
different capture sets are the same type. Nothing type-directed can discriminate them: not a
`Drop` trait, not a generic or specialized `impl:` (**P7.S4**), not a trait object's vtable
(**P7.S3u**, parked for this reason). The closure value grows a third word holding a
compiler-synthesized disposer symbol, minted where the capture's concrete type is known.

**The disposer is always constructible.** Probed: an owning closure cannot be returned from a
generic word, since `: mk ( 'T -- owning [ -- ] )` is rejected with "a quotation type ... cannot
appear as the output of `mk`", and a generic body that builds and calls one locally
(`: use ( 'T -- ) | s | [ s drop ] call ;`) is lowered per instantiation and runs correctly.
So there is no construction site at which the capture's type is unknown.

**Every quotation value carries the third word, owning or not.** No new type is introduced:
`Type::OwningQuotation` has existed since S3h, and `src/ir/types.rs:185-192` states that it is
not a representation difference at all, being "byte-for-byte the same two-slot `{ code, env }`
aggregate" sharing the same `:Q{n}` symbol, existing only so lowering can tell a heap env from
a frame env. The third word is added to that shared layout, and is the null pointer for a
non-owning quotation.

The alternative, giving only the owning variant a third word, is rejected. `:Q{n}` is keyed on
`QuotSigId`, the effect alone (`quot_index`, `src/backend/qbe.rs:57-62`), so two `IrType`s with
the same effect and different owning-ness collapse to one symbol today. Diverging them means
re-keying the symbol on effect *and* owning-ness, touching every site that maps both variants
together (`qbe.rs:408-413`, `:450-463`), contradicting the invariant documented at
`types.rs:185-192`, and introducing a width mismatch anywhere a value could flow between the
two types. The saving is 8 bytes, and only on a *materialized* quotation value, which is
already the minority case since combinators are inlined at their call sites rather than
materialized. Not worth the invariant.

**Rejected alternative, for the record: a runtime descriptor in the env block.** Store a
disposer pointer per capture slot plus a header, and one generic walker disposes every owning
closure. It works, and it needs no new symbol per literal. Rejected because it puts runtime
type info in the env (uniform slots need either every capture boxed to a word or a per-slot
size), and it puts N indirect calls on the disposal path instead of one. Disposal
determinism is where DESIGN.md is most load-bearing, so the cheaper runtime wins over the
smaller symbol table.

**The REPL obligation is not optional.** `src/ir/destructors.rs:8-35`: once a session holds any
user `drop` override, *every* linear aggregate's destructor is epoch-suffixed session-wide,
because any of them may transitively call the overridden one. A per-construction-site disposer
calls into those destructors, so it inherits the same obligation and must carry the same epoch.
Missing it does not produce a diagnostic; it dies at `dlopen` with an undefined
`sooth_struct_drop_N` (`src/repl.rs:3201`, `:3392`). This needs its own golden.

## Sizing

Widening the value:

- `quotation_layout` (`src/ir/types.rs:243-250`): `2 * word_width` → `3 * word_width`, plus a
  `disposer_offset`.
- `src/backend/qbe.rs:151`: the hardcoded `type :Q{idx} = {{ l, l }}` → `{{ l, l, l }}`. This
  is the only place the width is a literal string.
- `src/ir/types.rs:706`: the `assert_eq!(layout.size, 2 * WORD_WIDTH)` unit test.
- Blast radius is QBE IL goldens carrying the literal `{ l, l }`. Expect a sweep, not a design
  problem. The ABI shape does not change: `qbe.rs:408-413` already spells both quotation
  variants as an aggregate passed by value, and a 3-word aggregate is passed the same way.

New construction: the disposer synthesis itself, and the `emit_drop` arm that loads and calls
it. Deletion: the two twinned `drop` guards, and the struct/enum-field and cell-payload arms of
`reject_quotation_type_position`. Widening: `field_is_linear` / `layout_field_is_linear` to
report `true` for `Type::OwningQuotation`.

**Fixture bound.** A closure capturing an array or a slice ICEs today at
`src/backend/qbe.rs:531` (`an aggregate field is copied by blit, not scalar-stored`), because
the env store assumes one word per capture. Reproduced with a passing control. That is the env
block, not the closure value this slice widens, so it is orthogonal — but every fixture in this
slice must capture a scalar or a linear struct. Both work end to end: a linear struct capture
(`: mk ( P -- owning [ -- ] ) | p | [ p drop ] ;`) builds, runs and disposes.

**Tests to migrate, not delete** — 4, all in `tests/phase7_slice3h.rs`:

| Test | Line | Becomes |
|---|---|---|
| `an_owning_quotation_field_is_rejected` | 169 | asserts the field is admitted and disposed once |
| `an_owning_quotation_variant_field_is_rejected` | 187 | same, for an enum variant field |
| `dropping_an_owning_closure_is_a_located_rejection` | 335 | asserts `drop` disposes captures once |
| `dropping_an_owning_closure_in_a_generic_body_is_a_located_rejection` | 354 | same, generic path |

Note S3h never tested the array-element, slice-element or owned-cell-payload positions, so
there is nothing to migrate there and nothing asserting the array rejection that **P7.S5**
would later have to revisit.

## Ready to spec: yes, with five instructions for spec-writer

1. **Do not claim the array or slice element positions.** They are **P7.S5**. The exit criterion
   is the struct field, the enum variant field, and the owned-cell payload.
2. **The owned-cell payload needs a parser fix**, not only an audit change: an `owning` form
   after `^` currently fails with "unknown type owning". Scope it or rule it out explicitly;
   do not leave it implied, or it ships as whichever the implementation happens to do.
3. **Mutation-test both halves of the twinned `drop` guard** (`src/check.rs:3370` and
   `src/check/poly.rs:1255`). This repo has repeatedly shipped a twin pair with only one half
   covered. The generic-path test must go through a generic body, not a monomorphic one that
   happens to pass.
4. **The REPL epoch golden is a required deliverable, not a nice-to-have.** Its failure mode is
   an undefined symbol at `dlopen`, which no checker test will catch.
5. **The third word goes in the shared layout, so a non-owning quotation grows too.** State that
   as the ruling and update the doc comment at `src/ir/types.rs:185-192`, which currently asserts
   the two variants are byte-for-byte identical. That sentence stays true under this ruling and
   would become false under the rejected per-variant-width alternative, so it is the canary: if
   a phase finds itself editing it to say the widths differ, the ruling has been reversed by
   accident.
