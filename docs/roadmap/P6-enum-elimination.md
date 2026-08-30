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

**P6.S3 — Phase 6 Slice 3 — the eliminator word.** `[ done ]` The generated per-enum
eliminator (`Shape?`), arms matched by declared variant via a leading `( Circle )`/
`( &Circle )`/`( &!Circle )` tag, exhaustiveness and duplication as named errors,
arm-position effect elision. Lowers to the existing N-way dispatch (`lower_clauses`) via
a new additive `ArmBinding::WholeValue`. The scrutinee is **mode-polymorphic**: owning,
`&`, or `&!`, resolved from the eliminator call's own operand type, with every arm's mode
matching that call's mode uniformly (not a per-arm independent choice). This reuses
`check_field_projection`'s and `lower_clauses`'s existing value-vs-reference branching
rather than inventing a third mechanism, and forces `ir_type_of(Type::Variant)` to stop
being an `unreachable!` and erase to its parent enum's `IrType::Enum(id)` (a reference to
a variant is now a real interned referent, not merely a hypothetical one). See
`docs/roadmap/P6/slice3-spec.md` for the full design and its review history (decision 6,
the mode-polymorphism, was added post-hoc after the first three review rounds and
re-reviewed on its own before implementation).
**Exit:** a match is a term usable mid-body and inside a quotation; reordering a `type:`
declaration's variants leaves every call site correct. Two goldens
(`examples/eliminator.sth`, `examples/eliminator_ref.sth`) cover owning- and
reference-mode dispatch.

**P6.S3b — Phase 6 Slice 3b — check-time arm-tag resolution.** `[ done ]` An arm's `( Ok )` tag is
resolved to a concrete `Type::Variant` by the *parser*, against the concrete enum
registry, which makes the eliminator unable to eliminate a **generic** enum at all: a bare
tag carries no type arguments and a generic header contributes no concrete variant to
scan. Tag *typing* moves to check time, against the `EnumId` the scrutinee operand already
carries (the rule Slice 2 settled for accessors, which the parser is the last site to
violate); tag *recognition* stays at parse time as a name-only, module-scoped predicate
over the concrete and generic enum registries alike (a variant of an unimported module's
enum is not a routing tag, and an in-scope type of the same name takes precedence). The
surface syntax does not change: the generic case must read exactly like the concrete one.
Required before Slice 4, because clause bodies are today the only working generic-enum
elimination mechanism (`tests/phase5_generic_enum_elimination.rs`), so deleting them first
would regress a shipped Phase 5 capability rather than migrate it. `eliminator_registry`
stays keyed by the base family and serves as a family gate; the operative `EnumId` comes
from the scrutinee slot, which is what makes two instantiations independent. See
`docs/roadmap/P6/slice3b-spec.md`.
**Exit:** a generic enum is eliminable with bare tags, `Result[i64 bool]` and
`Result[bool i64]` eliminate independently in one word, and every Slice 3 diagnostic
still fires with unchanged wording (the mode-mismatch check included, since it is the one
consumer of the annotation's parse-time input slot).

**P6.S4 — Phase 6 Slice 4 — migration.** `[ done ]` Landed in `8ba0477`: the eliminator is
the only enum-elimination mechanism, every clause-dispatch site moved, and
`examples/vm.sth`'s `Op` dispatch reads through `Op?`. Every clause-dispatch site moves to the eliminator, and
`WordBody::Clauses`/`parse_clauses` are deleted, including the `Bool` print word declared
in `src/ast.rs` and the generic-enum elimination witnesses in
`tests/phase5_generic_enum_elimination.rs`. Deleting the clause path also retires
`check_clause_word`, the `clause_bodied_quotation_word_error` rule (which exists only
because a clause body cannot be spliced by the inliner), and `ArmBinding`, whose
`Decompose` case has the clause path as its sole caller.
**Exit:** the clause word body no longer parses; the tree builds green without it.

**P6.S5 — Phase 6 Slice 5 — nested tag paths.** `[ deferred ]` An arm tag would name a path
through nested enums, `( Some[v Circle] )`, desugaring to one eliminator call nested
inside another's arm body -- exactly what a hand-written program already does today, by
probe, in both owning and reference mode, with the current eliminator and no compiler
change. **Deferred, not planned:** the entire payoff is cosmetic (an arm reads as one line
instead of a nested call the checker already accepts) while the honest cost is not --
review round 1 forced a choice between a new AST-rewrite pass run before `check`, or
check-time synthesis with no home in `check_eliminator_call`'s current signature, or a
split where check validates the path form and a separate pass rewrites it; every option
adds a first-of-its-kind mechanism to buy pure syntax sugar, the kind of complexity
CLAUDE.md's growth rules ask to notice before building, not after. See
`docs/roadmap/P6/slice5-brief.md` for the settled syntax (field name always required, since
a fieldless/tuple variant form may exist later and single-field elision would trap it) and
for the full recon. Revisit only if a second consumer motivates the rewrite-pass mechanism
itself; a rewrite pass justified by one cosmetic slice alone is not worth building.
**Exit (if revived):** a nested tag path type-checks and lowers to nested eliminator calls;
a program matching two levels of enum nesting reads as one `match`-shaped word body, not
two nested word bodies.

**Exit:** enum elimination is a term, matched by variant name, with the clause-style word
body deleted.
**Dogfood:** `examples/vm.sth`'s nine-variant `Op` dispatch, rewritten, reads no worse
than the clause form it replaces.
