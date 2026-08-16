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

Payload reaches an arm by **generated per-variant accessors** (`Circle>r` per field,
`Rect>` destructuring all of them), mirroring the struct accessors, which retires the
positional field binding clause elimination uses. A payload-free arm consumes its
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

**P6.S1 — Phase 6 Slice 1 — quotation effect annotations.** A quotation literal may carry a
declared stack effect (`[ ( ..a T -- ..b ) ... ]`), checked against its body and against
the context consuming it. Independently useful and independently testable; no enum
machinery involved.
**Exit:** an annotated quotation whose body contradicts its declared effect is a located
error.

**P6.S2 — Phase 6 Slice 2 — variant types and accessors.** `Type::Variant(EnumId, usize)`, legal
only as an arm's declared input and as the value inside that arm, so it never becomes a
general first-class type: the eliminator is its only introducer. Plus generated
per-variant field accessors and whole-variant destructures. Layouts already exist per
variant (`EnumLayout.variants[vi].fields`); nothing changes at runtime.
**Exit:** a variant-typed value is reachable only inside an arm, with its fields readable
by name.

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
