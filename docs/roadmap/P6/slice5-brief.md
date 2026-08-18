# Phase 6 Slice 5: nested tag paths (brief)

An eliminator arm's tag names one variant of the scrutinee's enum. When that variant's
payload is itself an enum, reaching the inner variant costs a second, hand-written
eliminator call inside the arm body. This slice lets one arm tag name a path through
both levels — `( Some[Circle] )` — and desugars it to exactly the nested call a program
writes by hand today.

This is surface sugar. It adds no checker mechanism, no lowering, and no new rule about
modes; every guarantee it produces is one the nested form already produces.

```text
: describe ( Wrapped -- i64 )
  ~[ ( Has[Circle] ) &r @ swap drop dup * 3 * ]
  ~[ ( Has[Rect] )   &w @ swap &h @ swap drop * ]
  ~[ ( Empty )       drop 0 ]
  Wrapped?
;
```

## Recon (measured against the built compiler, 2026-08-18, `main` at `69995c2`)

`cargo test` state and the two unrelated uncommitted edits in
`tests/phase4_slice12_partab.rs` / `tests/phase4_slice6g.rs` are as Slice 3b's brief
records them.

1. **The target form already works by hand, in both modes. Probed directly, not
   inferred.** Two concrete enums (`Shape` with `Circle`/`Rect`, `Wrapped` with
   `Has v Shape`/`Empty`), built and run:

   - Owning: `~[ ( Has ) Has> area ]` where `area` is an ordinary `( Shape -- i64 )`
     eliminator word. Built clean, printed `75 / 12 / 0`.
   - Reference: `~[ ( &Has ) &v area ]` with `area ( &Shape -- i64 )`. Built clean,
     printed `5`.

   So the desugar target is a form the checker and backend already accept. This slice's
   job is to *emit* that form, not to make it work.

2. **Mode is inherited from the projection, not decided by this slice.** Under a `&`/`&!`
   scrutinee the only route to the nested payload is `&v`/`&!v`, which yields a reference;
   there is no owning route out of a borrowed parent, exactly as for any other field
   projection. So the inner call's mode is forced by the outer arm's mode through
   machinery that already exists. There is no "modes must agree across levels" rule to
   invent, and no way to spell a disagreement that the existing projection rules do not
   already reject. (The `( &Has )` probe above is the witness.)

3. **The payload projection the desugar must emit already exists, per mode.** Owning:
   `variant_generated_sigs` (`src/check/declarations.rs:1366`) mints `Has>` with
   `inputs: [variant_ty]`, `outputs: field_types`. Reference: `&v`/`&!v` through
   `check_reference_word`'s Slice 2 arm. Both are named after the *field*, so the desugar
   needs the outer variant's field name from its declaration, which it has.

4. **The tag is already its own typed structure, carried separately from any resolved
   type.** Slice 3b landed `VariantTag { name: String, mode: VariantTagMode }`
   (`src/ast.rs:1794`, `36b45fb`), where `name` is the bare sigil-stripped spelling that
   both the checker's arm-to-variant routing and the IR's clause dispatch match against.
   A path tag is therefore an extension of that one struct (a segment list in place of a
   single `name`), not a new channel and not a change to how mode is carried.

5. **Slice 3b has landed (`9c3b6ca`), and it moved tag typing to check time — the
   mechanism this slice's resolution rides on.** The tag is recognized by name at parse
   time and resolved against the scrutinee's `EnumId` at check time. A path tag resolves
   the same way, one level down: the outer segment against the scrutinee's `EnumId`, each
   inner segment against the `EnumId` of the named field's type. The gate this slice was
   waiting on is therefore cleared, and generic elimination (`Option[Shape]`, the
   motivating case) is now testable, which it was not before 3b.

6. **The bracket spelling is free at tag position; the apparent collision with the
   instantiated-variant spelling is not reachable from the surface.** An instantiated
   variant's name *is* mangled with brackets — `instantiate_enum` (`src/ast.rs:713`)
   builds `Ok[i64 str]` via `type_instantiation_name(&variant.name, args, regs)` — which
   is what makes `( Some[Circle] )` look ambiguous at first read. It is not, because that
   spelling is internal: it keys the generated-constructor `Sig` and the lowering-side
   variant word map, and no surface tag position ever admits it. Slice 3b's rule is that
   generic elimination is spelled with *bare* tags and the instantiation comes from the
   scrutinee, with inference of an instantiation from an arm explicitly out of scope. A
   tag position is therefore never an instantiation position, so brackets there carry no
   competing meaning. The only way to reintroduce the ambiguity is to match a path tag
   against mangled names instead of surface names — which decision 7 forbids.

## Decisions (settled here, not reopened by the spec)

1. **Desugar to the nested form, do not extend dispatch.** A path tag lowers to an outer
   eliminator arm containing a payload projection and an inner eliminator call over the
   payload. No change to `lower_clauses`, `ArmBinding`, tag-dispatch codegen, or
   `check_eliminator_call`'s dispatch rule. Recon 1 is why: the target is already green.

2. **Sibling arms sharing an outer tag are grouped into one synthesized outer arm.**
   `( Has[Circle] )` and `( Has[Rect] )` are two surface arms but one `Has` arm of
   `Wrapped?`, whose body is a `Shape?` call over the projected payload carrying both
   bodies as its arms. Grouping is by outer tag, and the group's arms keep their written
   order for reading; routing is by name at both levels, so order is not semantic (the
   Slice 3 rule, unchanged). Decision 6's every-field-named rule makes the group's shape
   uniform by construction: every arm in a group descends through the same field set, so
   there is never a merge of two differently-shaped decision trees to adjudicate.

3. **Exhaustiveness is enforced at every level, by the existing checks.** The synthesized
   outer call still requires an arm for every variant of the scrutinee's enum; each
   synthesized inner call still requires an arm for every variant of the payload's enum.
   A partially-nested group (`Has[Circle]` present, `Has[Rect]` absent) is a missing-arm
   error from the inner call, with no new check written.

4. **Mixing a plain outer arm with a nested group for the same outer variant is an
   error.** `( Has )` and `( Has[Circle] )` in one call is a duplicated `Has` arm, which
   Slice 3's duplication rule already names; the spec states it rather than leaving the
   grouping pass to silently pick one.

5. **No wildcard, no catch-all, at either level.** The inner level inherits the phase's
   exhaustiveness-by-name design point verbatim. A `( Has[_] )` form is not this slice
   and not this phase.

6. **A path segment names every field of the outer variant, exactly once, in declaration
   order.** The tag body reads like the `type:` declaration body it matches: `Both a Shape
   b Shape` is descended by `( Both[a Circle b Rect] )`. Each segment is a field name
   followed by either **a variant of that field's enum** (route on it) or **that field's
   own declared type** (do not route on it, hand the whole value to the arm body):
   `( Both[a Circle b Shape] )` routes on `a` and leaves `b` whole. A non-enum field can
   only take the type form (`( Tagged[n i64 s Circle] )`).

   This kills the inference problem: no rule ever has to guess which field a bare
   `( Both[Circle] )` meant, and no heuristic ("descend into the only enum-typed field")
   can silently stop working when a field is added.

   Two consequences the spec must carry. First, **exhaustiveness is a product** over the
   routed fields: routing on two `Shape`-typed fields requires all four combinations,
   since decision 5 allows no wildcard. The type form is the pressure valve, and it is
   the principled version of a wildcard — declining to route on a field is not a
   catch-all, because adding a variant to `Shape` still breaks every arm that *does*
   route on a `Shape`-typed field. Second, **a field named with a variant of the wrong
   enum, a missing field, a duplicated field, or an unknown field name is a located
   error**, each named as itself rather than surfacing as an arity mismatch from the
   synthesized projection.

   Rejected alternative: allowing omission (`( Both[a Circle] )`, `b` implicitly whole).
   It reintroduces per-group shape divergence, and an arm stops telling the reader the
   variant's arity.

7. **A path tag resolves against surface names at every level, and never denotes an
   instantiation.** Each segment is matched by `generic_surface_name` spelling (the
   routing rule Slice 3b already uses), never against a mangled instantiated variant name.
   This is what keeps recon 6's collision unreachable, and it costs nothing: the mangled
   name has no business at a surface tag position in the first place. A tag that looks
   like an instantiation (`( Ok[i64] )` naming a *type* argument rather than a nested
   variant) is an error, not a second meaning — the instantiation comes from the
   scrutinee.

8. **Guard/literal dispatch is out of the eliminator entirely.** Matching on a *value*
   (`0`, `n > 0`) has no exhaustiveness proof — Sooth has no SMT and declined refinement
   types — so it cannot share a construct with tag dispatch without making one keyword's
   safety guarantee depend on which arm form was written. If it is wanted, it is a
   separate, honestly-unchecked word (working name `cond`) in its own slice, requiring an
   explicit catch-all predicate.

## Open questions for the spec

- **OQ1 — settled: the separator is the bracket, `( Some[Circle] )`** (recon 6 plus
  decision 7). Retained here only as a note of what the spec must still *test*: a generic
  enum whose payload is itself an enum (`Option[Shape]`), which is the case that would
  expose a path tag accidentally matching a mangled name. `( Some Circle )`
  (space-separated) stays illegal regardless: under the existing arm-position elision
  rule it already means two stack inputs.

- **OQ2 — can a segment's variant form and type form collide?** Decision 6 reads a
  segment as a variant of the field's enum *or* as the field's own declared type name.
  Those sets are disjoint unless an enum declares a variant with the same name as the
  enum itself (`type: Shape | Shape … ;`, a newtype-ish shape that nothing forbids today),
  where `( Both[a Shape] )` reads both ways. The spec must state the tiebreak and test it
  directly; the same family as recon 6, but this one is reachable from the surface.

- **OQ3 — depth.** Is a path exactly two levels, or arbitrary (`( A[B[C]] )`)? Grouping
  (decision 2) is recursive if depth is, which multiplies the synthesized-call count but
  needs no new rule. Cheapest defensible answer is arbitrary-depth by construction with a
  two-level test plus one three-level test; the spec should confirm the grouping pass
  actually composes before promising it.

- **OQ4 — where does the desugar run, parse or check?** Grouping needs to know each outer
  variant's field type to resolve the inner tag, which after Slice 3b is check-time
  knowledge (recon 5). Parse time can plausibly do the grouping and leave resolution to
  check time. The spec must place both halves explicitly, and say which diagnostics come
  out of which, since a synthesized inner call must still report against the *written*
  arm's span, never a synthesized one.

- **OQ5 — do the synthesized calls disturb the drop/linearity accounting?** The probes in
  recon 1 consume the payload explicitly (`Has>`, then the inner arms consume the
  variant). A synthesized projection must leave the same obligations in the same places;
  confirm against a payload whose variant is payload-free (`Empty`-shaped), which is the
  case where "consume it there" bites (recon: the `&Empty` probe needed an explicit
  `drop`).

## Out of scope

- `cond` / guard / literal dispatch (decision 8), in any spelling.
- Wildcards or catch-all arms at either level (decision 5).
- Any change to dispatch lowering, `ArmBinding`, or codegen (decision 1).
- Any change to accessor generation or `Type::Variant` (Slice 2 shipped both; recon 3
  finds what this slice needs already minted).
- Nesting through a *struct* field rather than an enum payload. Structs have no variants,
  so there is nothing to route on; a struct field is reached by ordinary projection.

## Sequencing

Gate cleared: **Slice 3b is merged** (`9c3b6ca`, recon 5). Independent of **Slice 4** —
nothing here touches the clause path. Touches `src/parser.rs` (path tag parsing),
`src/ast.rs` (`VariantTag` carries a segment list, recon 4), and `src/check.rs` (grouping
and synthesis in/around `check_eliminator_call`).
No backend file is expected to change; if one does, decision 1 was wrong and the slice
should stop.

## Exit

A nested tag path type-checks and runs, producing the same output as the hand-nested form
it desugars to (recon 1's two probes become the goldens, one owning and one reference, each
paired with its hand-written equivalent). Exhaustiveness fires at both levels, a
partially-covered group is a located error naming the missing inner variant, and a
duplicated outer variant across plain and nested arms is a located error. Every existing
Slice 3 / 3b diagnostic still fires with unchanged wording. Decision 6 carries its own
witnesses: a two-field variant routed on both fields (the full product, all four arms), the
same variant routed on one field with the other left whole by its type name, and located
errors for a missing, duplicated, unknown, or wrong-enum field segment.

## Ready to spec?

**Yes.** The syntax question is settled (recon 6, decision 7): brackets are free at tag
position because a tag never denotes an instantiation, so `( Some[Circle] )` stands and
needs no exotic separator. Field selection is settled too (decision 6): every field named
exactly once, each either routed on by variant or left whole by naming its type.

The remaining questions are ordinary spec work. The mechanism itself is as low-risk as this phase gets:
decision 1's target form is already built, already green, and already probed in both
modes.
