### R2 — only fully concrete annotations are admitted

`resolve_annotation` rejects, at intern time and before any consuming parameter
is known:

- **any type variable** — `unbound_effect_variable_error`. A literal's own body
  supplies no instantiation. This is unconditional: a `'T` never survives to
  reach R4, so there is no deferred binding against a poly parameter's ground.
- **a shape-changing row** (`..a -- ..b`) — `shape_changing_row_unbound_error`.
  No fixed point to check a body against. Rendering `trim()`s so an unnamed
  side (`( ..a -- )`) leaves no stray space.
- **a passthrough row** (`..a -- ..a`) — `row_annotation_unsupported_error`
  ("row ... is not supported"), a distinct wording for a distinct reason: there
  *is* a fixed point, but `AnnotEffect` carries no row to hold it in. Admitting
  one would let a body smuggle anything through an uncompared row-typed prefix,
  and would pass R4 vacuously against a parameter declaring no row at all,
  which R5's strict equality forbids.

Giving rows real meaning (and a real R4 bridge to a consuming parameter's row)
needs an `AnnotEffect` row field this slice does not build.

### R4/R5 — reconciliation against a declared parameter

`reconcile_annotation_with_parameter` (`src/check.rs:1633`), called from
`check_literal_against_declared_effect`. By that point the parameter's
`QuotEffect` is already grounded by `PolyCtx`'s substitution, and the
annotation is fully-resolved `Vec<Type>`, so the comparison is plain structural
equality of the two vectors: **strict, no subtyping, no narrowing**, an
identical annotation being a no-op confirmation.

This is the one comparison R3 and R11 cannot make: a polymorphic/identity body
(`dup drop`) absorbs the annotation's claim and the parameter's ground alike,
so only holding the two *declarations* against each other sees the conflict.
There is consequently no concrete-parameter branch and no concrete-parameter
test: with a mono parameter, R3 plus the pre-existing R11 already force
`body == annotation == parameter` transitively, so such a test is a placebo.

For a **shape-changing** parameter (`~[ ..i -- ..o ]`, `..i != ..o`) the check
still runs, over the overlapping tails (`tails_agree`). Full vector equality
would be wrong: the declaration names only the slots above the row, so a
literal may legitimately reach past them into the row (`( i64 -- )`) or leave
more behind than the declaration names (`( -- i64 )`). Skipping the parameter
entirely, as first built, let a flat contradiction
(`~[ ( bool -- bool ) dup drop ]` at a declared `~[ ..i i64 -- ..o i64 ]`) pass
every check.

Disagreement is `annotation_parameter_mismatch_error`, naming both effects:
