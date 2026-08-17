# Phase 7 Slice 1: accessors as lenses

**Status: delivered.**

Field access is a mode-carrying projection word (`&hp` / `&!hp`) resolved against the
receiver's type, replacing the fused type-and-field-and-operation tokens (`Sprite>hp`,
`Sprite<hp`, `Sprite|>hp`, `&Sprite>hp`, `&!Sprite>hp`). The consuming getter, functional
setter, and peek are gone in favour of borrow-plus-`@`/`!`, deleting the two implicit
disposals those words performed.

Generated words per field: five to two. Per type (`S`, `S>`): unchanged. Design rationale
is in [slice1-brief.md](./slice1-brief.md).

## D1: accessor grammar

```text
projection := ("&" | "&!") field-name
```

The `&`/`&!` sigil is load-bearing for more than mode. `&`-led names are dispatched ahead
of `check_operator` and `check_array_word`, so a projection cannot be shadowed by a
builtin of the same name (`len` is both a dispatched builtin and a field of `Buf`). There
is no bare-marker form.

`&>` and `&!>` keep their array-only meaning, bounds check included. There is no
struct/array unification: a struct selector is a name, an array selector is a runtime
value.

## D2: projection semantics

| receiver | effect | note |
| --- | --- | --- |
| `&S` | `( &S -- &A )` consuming | chains: `u &stats &hp @` |
| `&!S` | `( &!S -- &!A )` consuming | as above, mutable |
| owned `S` | `( S -- S &A )` **non-consuming** | leaves the receiver in place |

The owned case must be non-consuming: consuming it would require disposing the unextracted
fields, the implicit behaviour D3 deletes. The reference case must be consuming: a
reference left on the stack is a surplus value, so a non-consuming chain would strand an
intermediate at every nesting level. Behaviour varying by operand type here is deliberate.

The owned-receiver arm generalizes the old `S|>fi` peek, which already did a non-consuming
projection off an anonymous top-of-stack value through `Provenance`'s region/parent
machinery. It does **not** inherit `Peek`'s `is_copy` gate: a projection borrows the field
rather than duplicating its value, so producing the reference is legal for a linear field
too. Extracting the value still bottoms out at `S>` destructure, because `@`/`!` refuse a
linear referent, which is where that gate belongs.

Two live `&!` projections into one region off an owned receiver (`p &!hp swap &!hp`) are a
checked conflict, not a silent alias: the owned arm interns the projection as a child
region of the receiver's own, so the existing overlap check fires.

## D3: what retired

`StructWord::Get`, `Set`, and `Peek` are deleted, along with their `Sig` synthesis and
lowering arms. `StructWord` is now `Construct` | `Destructure`.

| retired | replacement |
| --- | --- |
| `s S>f` (Copy field) | `s &f @` |
| `s S\|>f` | `s &f @`, peek is no longer a separate word |
| `s S<f v` | `s &!f v !` |
| `s S>f` (linear field) | `s S>` destructure, extraction is explicit |

Two implicit disposals go with them: `S>fi` dropping every non-extracted linear field, and
`S<fi` dropping the overwritten value. Both contradicted the invariant `!` itself enforces
(storing a linear value through a reference is refused rather than silently leaking the
overwritten one). The language now has one rule instead of two.

`Construct` (`S`), `Destructure` (`S>`), and cell projection `&^`/`&!^` are unchanged.

## Resolution

### R1: receiver-directed field resolution (check side)

`check_field_projection` (`src/check/word_families.rs`) takes the top-of-stack type,
strips `&`/`&!` to its referent, requires a `Type::Struct` (or `Type::Variant`, R4), and
finds the field by name in that decl. Mode checking against `recv_mut` and `intern_ref_type`
for the output are unchanged. A non-struct receiver reuses `reference_word_operand_error`.

Inside a generic body the old guard tested `rest.contains('>')` on the fused name, which
`&f` does not contain. The projection case is caught after both the locals and statics
lookups miss, via `receiver_is_aggregate_projection` on the `PolyType` stack, and rejects
with `poly_unsupported_accessor_error` rather than falling through to "`f` is not a local".

### R2: the resolved-field side table (check to IR)

The checker records its resolution and lowering reads it back per call site:
`resolved_fields: HashMap<Span, (StructId, usize)>`, keyed on the whole `Span`
(`{line, col, module}`; a bare `(line, col)` key cross-file-collided in multi-module builds
and silently misdispatched). It rides the `builtin_overloads` path: a mutable channel from
the `&`-dispatch site in `check/terms.rs` into `check_reference_word`, a `PolyCtx` field
beside `builtin_overloads` so it survives the same `if`-arm-cloning hazard, drained onto
`Module::resolved_fields` and threaded through `ir/driver.rs` into `FuncBuilder`.

`structs.words` (keyed on the globally unique fused name) keeps only `Construct` and
`Destructure`; `hp` is not unique, and lowering has no checker stack to re-derive a
receiver type from.

**Span-keying invariant.** `poly_reference_word` rejects any struct-field accessor inside a
generic body, and a generic body is checked once, never re-walked per instantiation. So a
projection is only ever resolved in the monomorphic path, where the receiver carries the
concrete `StructId` and the call site's span is unique. If a later slice allows generic-struct
field projection inside a generic body, `Span`-keying alone silently misdispatches across
instantiations.

### R3: shadowing and unknown fields

- A projection resolves against the receiver first.
- Receiver lacks the field, a local/static of that name exists: `&hp` is that borrow,
  unchanged.
- Both apply: a located error naming both, not silent precedence. Field and local names in
  this corpus are short and collide easily (`arr`, `acc`, `key`, `n`).
- Neither: `` `Buf` has no field `lenn` ``, not `unknown word`. The diagnostic moved from
  resolve-time to check-time, where the receiver type is known.

### R4: variant accessors, checker-only

`variant_generated_sigs` and `variant_accessor_field` take the same shape: `&r` on a
`Type::Variant` receiver. This is **checker-only**. `resolved_fields` is `StructId`-keyed
and cannot represent a variant, and `EnumWord` has only `Construct`, so a variant
projection is check-legal but not lowerable and must never reach the `resolved_fields`
insert. Tests for it stay check-only; a build/run golden would panic on the missing
lowering arm.

P6.S3's R6 (specified to add `EnumWord::Get`/`Destructure` "mirroring `StructWord`'s
registration exactly") must be re-pointed at this shape before implementation, including
its own `EnumId`-keyed lowering-side table rather than a widened `resolved_fields`.

### R5: the REPL path

The REPL builds its session env independently, so `resolved_fields` reaches its lowering
path too. Covered by a REPL session golden, not an assumption.

## Lowering

### R6: `lower_struct_word` keeps two arms

`lower_reference_word`'s struct branch reads `resolved_fields[span]` for the
`(StructId, field index)` instead of consulting `structs.words` by name, then emits the
same `field_ptr` + `push_reference`. The owned-receiver case pushes the projection without
popping the receiver. `field_is_linear`'s drop loops went with the `Get`/`Set` arms; no
other `emit_drop` site changed.

### R7: the materialized-quotation escape check

A materialized quotation whose declared effect returns a reference used to panic on
`checked: every reference value records its referent`. Second-class references are a
DESIGN.md invariant and the check already existed for ordinary words, so a quotation's
declared effect now gets the same check and the same wording (`a reference cannot be
stored: … take the reference as an input instead`). This is what makes the brief's
Decision 1 sound: a selector-as-quotation cannot be stored, so no first-class lens value is
needed.

## Migration

558 sites (86 across 13 `.sth` files, 472 across 43 Rust files). Both spellings coexisted
for two phases so every phase stayed green, and the fused spelling was deleted once its
call sites had moved.

## Out of scope

- Any first-class `Lens` type, composition word, or storable selector.
- Struct/array unification of `&>`.
- Structural row-polymorphism (projecting a field of an unresolved `'S`). A marker needs a
  concrete receiver; `~[ ]` parameters force `inline`, so quotation bodies are spliced at
  concrete call sites and abstraction-over-field still works.
- `Construct`/`Destructure`, `&^`/`&!^`, array bounds checking.
- P6.S3 itself, beyond R4's shape change.

## Roadmap correction

`docs/roadmap/P7-stdlib-nostd.md` specified the lens words as `&>`/`<`/`|>` and required
"one `&>` accepting both an array and a struct". Both were wrong and the text now states
the delivered design:

- `<` and `>` are `lib/` words, not name-dispatched builtins (they stay in the fixed-name
  list only for `has_self_tail_call`), so they cannot be bare field operations.
- `&>` is array-only, so unification would *add* static overloading rather than remove a
  wart.
- The slice therefore has no dependency on Phase 4 Slice 8.

## Exit (met)

Every struct and variant field access in the corpus goes through `&f` / `&!f`; `Get`,
`Set`, `Peek` and their two implicit disposals are deleted; `&>` remains array-only; a
materialized quotation returning a reference is a located error rather than a panic; and
the P7.S1 roadmap and DESIGN.md text match the delivered design.

## Tests

Check (`src/check/word_families.rs`): projection resolves against the receiver type;
non-struct receiver, unknown field, field shadowed by local, and mode mismatch are errors;
fallback to local borrow when the receiver lacks the field; same field name on two structs
resolves by receiver; variant receiver accepted (check-only, must not build or run); two
mutable projections off one anonymous receiver conflict.

Poly (`src/check/poly.rs`): `projection_on_generic_receiver_body_is_error`.

Table: `resolved_fields_key_includes_module` (two modules, same line/col, distinct fields:
the silent-misdispatch guard) and `resolved_fields_records_one_entry_per_call_site`.

Goldens (`tests/phase7_slice1.rs`): read/write through projections on both receiver shapes,
a nested chain (`&stats &hp @`), `projection_resolves_per_instantiation` with **asymmetric**
type arguments (`Box[i64]` and `Box[str]`, since a symmetric instantiation cannot
discriminate a correct resolution from a swapped one), a REPL session, and dropping an
owned receiver while a projection is live.

Lowering: owned receiver stays, reference receiver is consumed;
`quotation_effect_returning_a_reference_is_a_located_error` asserts the wording and that it
does not panic.

The R9/R11 deletion guard is asymmetric by necessity: R9 (sibling drop) is a runtime
disposal-count golden, checked by reverting the deletion commit locally and confirming the
golden fails. R11 (overwrite drop) cannot be, since storing a linear value through
`&!f … !` is already a compile error, so its guard is a diagnostic test asserting that
rejection.
