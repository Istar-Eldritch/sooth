# Phase 6 Slice 1: quotation effect annotations (brief)

A quotation literal (`[ ... ]`/`~[ ... ]`) carries no declared effect of its own today;
every one is checked only against an effect supplied externally, by whichever declared
quotation parameter it fills. This slice adds an optional effect annotation *inside* the
literal itself (`[ ( ..a T -- ..b ) ... ]`), checked against the literal's own body and,
where a consuming context also declares one, reconciled against that context. It is the
one piece of Phase 6's design with no enum-elimination content: nothing here mints an
eliminator, a variant type, or a per-variant accessor. Its exit case is a standalone
annotated quotation, not an enum arm.

## Recon (measured against the built compiler, 2026-08-16, `main` at `c96f722`)

`cargo test` is green at this HEAD. Claims below are read from source, not inferred.

1. **A quotation literal's AST has no effect slot.** `TermKind::Quotation(Vec<Term>, bool)`
   (`src/ast.rs:1556`) carries only its body terms and the `~`/ordinary flavour flag. Both
   parse sites (`Token::LBracket`, `Token::TildeLBracket`, `src/parser.rs:2992-3018`) read
   straight into the body reader (`parse_terms`, stopping at `Token::RBracket`); neither
   reads anything before the body. There is no notion of "this literal declares its own
   effect" anywhere in the grammar today.

2. **A declared effect (`QuotEffect`) exists only in signature/type position, never term
   position.** `Type::Quotation`/`Type::InlineQuotation` (`src/ast.rs:1241-1296`) wrap a
   leaked `&'static QuotEffect { inputs, outputs, name_static }`, built by
   `quotation_type`/`inline_quotation_type`. The only parser path that produces one is
   `parse_quotation_type_expr` (`src/parser.rs:2236`), reached exclusively from
   type-expression contexts: a word's own declared parameter/return slot, a struct field
   typed as a quotation, an array element type. It is never reached from `parse_term`, so
   `[ ( i64 -- bool ) ... ]` is unparseable today — the `(` right after `[` falls straight
   into the ordinary term reader and is read as a stray `Token::LParen` (only otherwise
   legal at a word/extern header, `src/parser.rs:1282`/`:1408`), a located parse error, not
   silently accepted or misparsed.

3. **The one checker path, `check_literal_against_declared_effect`, always requires an
   externally-supplied effect and never checks a literal standalone.** Its signature takes
   `eff: &QuotEffect` as a caller-supplied parameter (`src/check.rs:1391`); every call site
   funnels through one of two sources: a combinator's declared quotation parameter (mono or
   poly) or a `Type::Quotation` materialization boundary (assigning a literal into a
   quotation-typed place — an array slot, a struct field). Verified directly: a bare
   `[ dup 10 < ] | q |` with no consuming declared-quotation context binds `q` and type-checks
   fine with no effect ever computed for it; there is no code path that asks "what is this
   literal's effect" except in service of comparing it to someone else's declared one
   (R11's `check_effect_mismatch_error`, `:1639`, only ever fires from within that same
   comparison).

4. **A quotation effect's rows are scoped to the enclosing signature, not to the literal.**
   `PolyType::Quotation`'s trailing `Option<u32>` row ids (`src/ast.rs:973`) are documented
   as sharing the *signature's* row id space (`PolySig::row_in`/`row_out`) rather than
   minting their own (R4, `ast.rs:970-971`): "a row inside a quotation effect can only ever
   denote the signature's top-level row." An annotation written directly on a literal has no
   enclosing `PolySig` to borrow a row id from — this slice's `( ..a T -- ..b )` needs a row
   variable scoped to the literal's own annotation, a genuinely new row-variable home, not a
   reuse of the existing one.

5. **Verified empirically: nothing about closures or materialization changes this slice's
   scope.** A materialized (loaded-from-storage) quotation value is rejected on sight at
   every `~[ ... ]` parameter regardless of Slice 7b's closures existing (repro confirmed
   directly this session: `error: while expects a quotation ~[...] here, found [...]`). This
   slice's annotated literals are checked as ordinary compile-time literals, the same
   `quot_bodies`-tracked phantom shape `5749a14` already fixed for row-carried quotations —
   no materialization boundary is exercised by this slice's own exit case.

6. **Row-carrying quotations are usable as row-typed combinator parameters today, confirmed
   live.** `tests/phase4_slice10b.rs`'s three goldens (cited above) pass at this HEAD:
   `times`/`while` over a row-carried literal quotation already lowers and runs correctly.
   So an annotated literal used the same way (as an ordinary `[ ... ]`/`~[ ... ]` argument to
   an existing combinator) has no known lowering obstacle; this slice is a checker/parser
   feature, with no `src/ir/`-side change implied by its own exit case.

## Decisions (settled here, not reopened by the spec)

1. **The annotation is an optional leading parenthesized effect inside the brackets**, read
   by the same reader that already produces `parse_quotation_type_expr`'s `Vec<Type>`/`--`/
   `Vec<Type>` shape, reused rather than reinvented: `[ ( ..a T -- ..b ) term* ]`. A literal
   with no leading `(` parses exactly as it does today (recon 1) — this is additive, not a
   breaking re-parse of every existing quotation literal.

2. **An annotation may use the row/variable grammar a word signature already supports**
   (`'T`, `..a`), scoped to a *new* per-literal row-variable space (recon 4), not the
   enclosing word's `PolySig`. Two sibling literals inside the same word body each get
   independent row ids; nothing unifies across them except where an existing combinator
   parameter (e.g. `times`'s `~[ 'a -- 'a bool ]`) already ties them together today.

3. **Checking is bidirectional but asymmetric.** The literal's own body is always checked
   against its own annotation (new: recon 3 has no such standalone path today). When the
   literal also fills a declared quotation parameter, the existing
   `check_literal_against_declared_effect` path (recon 3) additionally reconciles the
   annotation against the parameter's declared effect — reusing R11's existing mismatch
   error shape (`:1639`) rather than inventing a second diagnostic for the same disagreement.

4. **An unannotated literal is unaffected.** This slice adds a capability, not a requirement:
   every existing golden, dogfood, and example keeps working with no annotation anywhere.

## Open questions for the spec

- **OQ1 — annotation vs. declared-parameter disagreement, which wins as the checked
  target?** If `[ ( i64 -- bool ) dup 10 < ]` fills a parameter declared `~[ i64 -- i64 ]`,
  is that a located mismatch error (both types named, R11's existing shape), or does the
  annotation only ever *narrow/confirm* and get silently ignored where the parameter's own
  declared effect is authoritative? Decision 3 assumes the former (an error) but doesn't
  settle whether the annotation is ever informative when disagreement, not just identity, is
  possible — e.g. is a *compatible-but-not-identical* annotation (a supertype-shaped
  wildcard some day) conceivable, or is this slice strictly an equality check with no
  subtyping? Recon found no subtyping concept anywhere in `Type`, so leaning toward "strict
  equality, no narrowing" unless the spec finds a reason otherwise.

- **OQ2 — is a **bare row with no named type** (`( ..a -- ..b )`, no `T` at all) legal, and
  what does it mean?** Phase 6's own eliminator-arm design (`( Circle )`) elides everything
  down to just the variant name, which isn't the same grammar as `( ..a T -- ..b )` — that
  elision is Slice 3's problem, scoped to arm position specifically. Does Slice 1 need to
  parse *any* elided form at all (partial effects, missing outputs), or is its own exit case
  deliberately the full, unelided four-part effect (`( ..a T -- ..b )`), leaving every
  elision shape to whichever slice actually consumes it (arm position in Slice 3)? Leaning
  towards: Slice 1 ships only the full form; eliding is out of scope here and belongs to
  Slice 3's own grammar, which needs it for a different reason (arm-to-variant binding, not
  literal self-documentation).

- **OQ3 — does an annotated literal's row variable ever need to unify against an *enclosing*
  word's own row (`..a` matching the outer word's declared `..a`)?** Decision 2 assumes
  independent row ids per literal with unification only through an existing combinator
  parameter. But a literal sitting directly in a word body with no consuming combinator (a
  standalone annotated quotation bound to a local, then `call`ed later) has its row checked
  against *what*, if nothing external supplies one? Does `..a`/`..b` in a freestanding
  annotation mean "matches whatever the stack looks like at this literal's own position" (an
  implicit unification against the call site's live stack, the same way a plain word's
  declared effect unifies against its own call site) or is a freestanding annotated literal
  with a row actually meaningless until something consumes it? This is the question closest
  to affecting Slice 1's actual exit witness, since the exit case needs a concrete "annotated
  quotation checked against its context" example, and this decides what "context" means for
  a literal with no combinator around it.

## Out of scope

- Anything enum/eliminator-shaped: `Type::Variant`, per-variant accessors, the eliminator
  word itself, arm-position elision (`( Circle )`). All later slices in this phase.
- Any IR/backend change: recon 5-6 found no lowering gap this slice's own exit case
  exercises. If the spec's recon finds one, that is new information, not an assumption
  carried in from this brief.
- Subtyping or effect-narrowing between an annotation and a declared parameter (OQ1's
  leaning): out of scope unless the spec finds a concrete need.
- Rewriting `while`'s materialized-quotation rejection (recon 5): unrelated, no consequence
  of this slice existing.

## Sequencing

No gate from any open Phase 4/5 item: recon 5-6 close out the only two prerequisites this
phase's own text once named. Touches `src/parser.rs` (a new leading-`(`-effect reader inside
the quotation-literal arm, reusing `parse_quotation_type_expr`'s inner list parser),
`src/ast.rs` (wherever the annotation is carried on `TermKind::Quotation` — likely a new
optional field, not a new `Type` variant per decision 4's exit criterion), and `src/check.rs`
(the new standalone-check path, plus the existing `check_literal_against_declared_effect`
reconciliation per decision 3).

## Exit

An annotated quotation literal whose body's actual effect disagrees with its own declared
annotation is a located error, independent of whether it fills any parameter. An annotated
literal that also fills a declared quotation parameter and disagrees with that parameter's
effect is a second, distinct located error (or the same error naming both, per OQ1). An
unannotated literal anywhere in the tree is unaffected: the full existing test suite stays
green with no changes to any file outside the new annotation path.

## Ready to spec?

**Yes, with three open questions handed to the spec, none blocking a narrow reading.** OQ2
should settle toward "no elision in this slice" — Phase 6 Slice 3 needs its own elision
grammar for a different reason (variant binding) and conflating the two risks shipping a
general partial-effect mechanism nobody asked for. OQ3 is the one question that actually
bears on what this slice's exit example looks like and should be settled first, before OQ1
(which only matters once OQ3 establishes that a parameter-filling literal is even in scope).
