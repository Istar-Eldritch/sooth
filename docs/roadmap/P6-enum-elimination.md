[← ROADMAP](./ROADMAP.md)

### Phase 6 — Term-level enum elimination  `[L]`

Eliminating an enum is a **term**, not a word-body form. A match composes mid-body,
nests inside a quotation, and needs no helper word and no bundle struct, the way every
other construct in a concatenative language does. `WordBody::Clauses` and
`parse_clauses` are deleted; the clause-style word is replaced, not joined, since two
elimination forms in the language permanently is worse than one migration.

The form is a generated per-enum eliminator word taking one quotation per variant, each
arm annotated with the variant it handles. Arms are matched **by declared variant, not
by position**, so reordering or inserting a variant in a `type:` declaration cannot
silently rebind a call site; a missing or duplicated arm is a named error. Arms are
ordinary quotation values, so an arm can be named and reused.

```
: area ( Shape -- f64 )
  [ ( Circle )  Circle>r dup * 3.14159 * ]
  [ ( Rect )    Rect> * ]
  Shape? ;
```

An arm's annotation is a stack effect with an **elision rule scoped to arm position**:
`( Circle )` names the variant and unifies inputs-below and outputs across sibling arms,
escalating to `( Vm Push -- Vm )` and then `( ..a Vm Push -- ..b )` when an arm needs to
say more. The elision is arm-only; a word signature never infers its outputs. Because
elision moves a shape error off-site, the disagreement diagnostic names both arms by
variant.

Payload reaches an arm by **generated per-variant accessors**: `&r`/`&!r` per field
(receiver-directed projection, P7.S1's shape, not the retired `Circle>r` fused spelling)
and `Rect>` destructuring all of them, which retires the positional field binding clause
elimination uses. A payload-free arm consumes its
variant explicitly (`Halt>`): the linear spine does not auto-drop.

**No open lowering prerequisite.** The row-carried-quotation backend crash this phase's
design discussion once assumed open (a `times`/`while` self-tail loop with a quotation
riding untouched in the row) is already fixed and covered by a green golden (`5749a14`,
`tests/phase4_slice10b.rs`). One narrower case stays unconfirmed rather than fixed: `while`
over a *materialized* (closure-captured) quotation. Slice 7b's capturing closures are
built (`examples/capturing_dispatch.sth`); the dead-code conclusion holds for a narrower
reason, confirmed by direct repro: every combinator's `~` parameter rejects a materialized
quotation value on sight regardless of whether one exists elsewhere in the program — not a
blocker for Slice 1 below, which checks annotated quotations against ordinary contexts, not
materialized ones.

**P6.S1 — Phase 6 Slice 1 — quotation effect annotations.** `[ done ]` A quotation literal
may carry a declared stack effect (`[ ( ..a T -- ..b ) ... ]`), checked against its own
body standalone and, where it fills a declared quotation parameter, reconciled against
that parameter. Reconciliation only does independent work for a poly parameter whose
type variable the annotation also names (a concrete/mono parameter's disagreement is
already caught by the standalone body check); a shape-changing row (`..a -- ..b`) in an
annotation is rejected outright, standalone or filling a parameter, since nothing in this
slice grounds it. Independently useful and independently testable; no enum machinery
involved. See `docs/roadmap/P6/slice1-spec.md` for the full design.
**Exit:** an annotated quotation whose body contradicts its declared effect is a located
error, standalone or parameter-filling.

**P6.S2 — Phase 6 Slice 2 — variant types and accessors.** `[ done ]` `Type::Variant(EnumId, usize,
&'static str)`, legal only as an arm's declared input and as the value inside that arm, so
it never becomes a general first-class type: nothing in this slice can construct one from
surface syntax, since the eliminator (Slice 3) is its only introducer. The leaked
`&'static str` carries the stable `Enum.Variant` display name (`display_static` on
`VariantDecl`, one source of truth read by the sole constructor `variant_type`), needed so
two `Type::Variant`s for the same variant always compare equal. Generated per-variant field
accessors and whole-variant destructures follow P7.S1's struct shape (a scalar/aggregate
projection via `&field`/`&!field`, resolved per call site against the receiver, not a
fused `&Variant>field` spelling), split across three existing mechanisms rather than one:
a scalar getter and the whole destructure go through ordinary `Sig` dispatch
(`variant_generated_sigs`), an aggregate getter through a new `check_variant_get_word`
(interior-address aliasing, like the struct getter), and reference-mode access
(`&field`/`&!field` on a variant receiver) through a new arm in
`check_reference_word`. All three resolve the variant from the `EnumId` the operand
already carries, not a global name scan: variant names are not unique across enums (only
type names are deduped), so a scan would mis-resolve the second enum's same-named variant.
Layouts already existed per variant (`EnumLayout.variants[vi].fields`); nothing changed at
runtime. See `docs/roadmap/P6/slice2-spec.md` for the full design and its review history
(three rounds; the operand-EnumId resolution rule and the single-source-of-truth display
name both replaced earlier, unsound drafts).
**Exit:** a variant-typed value is reachable only inside an arm, with its fields readable
by name. Verified by unit tests against hand-built checker state, not an `.sth` golden,
since no program can construct a `Type::Variant` until Slice 3 ships; `examples/vm.sth` is
untouched this slice.

**P6.S3 — Phase 6 Slice 3 — the eliminator word.** The generated per-enum eliminator, arms
matched by declared variant, exhaustiveness and duplication as named errors, arm-position
effect elision. Lowers to the existing N-way dispatch (`lower_clauses`).
**Exit:** a match is a term usable mid-body and inside a quotation; reordering a `type:`
declaration's variants leaves every call site correct.

**P6.S4 — Phase 6 Slice 4 — migration.** Every clause-dispatch site moves to the eliminator, and
`WordBody::Clauses`/`parse_clauses` are deleted, including the `Bool` declaration
injected in `src/ast.rs` and Phase 5's `Result`/`Option` consumers.
**Exit:** the clause word body no longer parses; the tree builds green without it.

**Exit:** enum elimination is a term, matched by variant name, with the clause-style word
body deleted.
**Dogfood:** `examples/vm.sth`'s nine-variant `Op` dispatch, rewritten, reads no worse
than the clause form it replaces.
