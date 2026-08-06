# Phase 4 Slice 6a: quotation types in signatures, the inliner, and `each`/`map`/`fold`

**Status: specified.** Base `main` @ `d033b2d`. Makes a combinator an ordinary Sooth
library word. A quotation becomes nameable in a signature, a word may take one as a
parameter, and every call to such a word is inlined at the call site by term splicing.
Native only; the REPL is a located rejection at both chokepoints (D7, 6c lifts it). This
slice owns the compiler's **only** inliner: there is none today, and nothing downstream
adds one (QBE is per-function, `cc` runs with no `-O`), so the inliner is the *enabling*
mechanism, not an optimization laid over a working library. The library is not "writable
but slow" today, it is not expressible at all (recon 1, both walls re-verified against
the built compiler: `[ ... -- ... ]` dies in the parser as a malformed array count, and a
quotation to any user word is a hard located rejection).

## What this slice is and is not (the fixed constraints)

Measured against the built tree, three facts fix the shape:

1. **The whole gap is front-end.** What the inliner must *produce* is fully expressible
   today: hand-inlined `each`/`fold`/`map` over `[i64 4]` (a `times` loop, `>usize` on the
   index, `&a i &> @` reads, `&!b i &!> v !` writes) compile and run correctly (recon 2).
   The element/length polymorphism the library needs already works (`: size ( ['T 'N] --
   ['T 'N] usize ) len ;` runs at `[i64 4]` and `[f64 7]`, recon 3). There is no IR,
   lowering, or loop-primitive gap.

2. **The type must become nameable, and that is a `Type`/`PolyType` change, not an
   `IrType` one.** `Type` (`src/ast.rs:670`), `PolyType`, and `IrType` (`src/ir.rs:76`)
   have no quotation variant; slice 4 deliberately kept the marker on a `Slot`/`Binding`
   side-channel (`quot`, `QuotRef::Known`) so nothing downstream had to change. For `: each
   ( ['T 'N] [ 'T -- ] -- )` to be checkable standalone, `Type` and `PolyType` gain a
   quotation variant carrying a *declared effect*, with unification and `apply_subst`
   following. `IrType`, the calling convention, and the backend stay untouched: a
   quotation-taking word is never lowered standalone (recon 5), so its type never reaches
   the backend.

3. **The inliner attaches at check/lower time by term splicing, forced not chosen.** A
   quotation-taking word's body contains `f call`/`f times` where `f` has no literal and no
   runtime representation, so **no `IrFunc` for it can exist** (recon 5): there is nothing
   at the IR level to inline, and a post-lowering IR pass is impossible, not rejected. The
   attachment point is check/lower-time splicing, generalizing the `call`/`times` fusion
   (`src/check.rs:4763,4797`) that already clones a literal's AST body and re-runs
   `check_terms` against the live stack.

**Therefore, fixed:** `Type`/`PolyType` gain a quotation-effect variant (no `IrType`
variant, no "statically known" bit, D6); `call`/`times` accept an *abstract* quotation
(one typed only by a declared parameter) beside the literal they accept today; a user word
declaring a quotation parameter is checked standalone against that declared effect
(recon 7, D4) and inlined at every call site by substituting the caller's literal and
splicing the callee's body; total inlining, anything un-inlinable a located error (D5); the
REPL a located rejection (D7). No new `Instr`/`Terminator`, no `src/backend/qbe.rs` change.

## Locked decisions (from the brief, carried)

- **D1** The spelling stays `[ ... -- ... ]`, disambiguated on a top-depth `--`. An array
  type never contains `--`; a quotation effect always does (including the nil effect `[ --
  ]`). No new sigil. The malformed-array diagnostic stays sharp: `[i64]` still says the
  count must be a literal, not something quotation-flavoured.
- **D2** The inliner attaches at check/lower time by term splicing (forced by recon 5, not
  chosen). Retain the callee's AST body, substitute the caller's literal for the quotation
  parameter, splice at the call site, and let the existing `call`/`times` fusion fire on
  the substituted body.
- **D3** A quotation literal passed to a user word may capture only `Copy` locals, by
  value; no borrows of enclosing places. This is what makes a declared effect a *complete*
  summary of a literal (D4's foundation), and it is exactly what discharges the `times`
  obligations at the inline site (see "The hardest question" below).
- **D4** Standalone checking of a combinator body is compositional. At `each`'s own def
  site, `f call`/`f times` check against `f`'s declared effect exactly as an ordinary word
  call checks against its `Sig`/`PolySig` (recon 7). The body is genuinely checkable
  standalone; the signature is not documentation over a macro.
- **D5** Inlining is total, not best-effort. Every call to a quotation-taking word must
  inline, transitively (`map` over `each` inlines twice). Anything un-inlinable is a located
  error, never a silent real call. Recursion among quotation-taking words is the first case.
- **D6** The new type variant must not bake in "a quotation is always statically known."
  That stays a predicate on the *value* (`Slot.quot`), never a property of the type, so
  slice 7 can admit a runtime closure without unpicking unification and the
  monomorphization walk.
- **D7** Native only. The REPL is a located rejection at every chokepoint, specified and
  tested (the 5a/slice 2 lesson: an unpinned REPL gap turned out to be a silent miscompile,
  not a clean deferral). 6c lifts the rejection.
- **D8** The library lives at `lib/combinators.sth`, imported by relative path like every
  other file today. No stdlib layout is invented here (Phase 6 owns layering).

## The hardest question: do `times`' three obligations discharge under D3?

`times` today (`src/check.rs:4797`) proves three obligations **by walking a body it can
see**, splicing the literal against the row plus a synthesized index and comparing scope
before and after:

- **move-state identity** (`src/check.rs:4864`): `scope.moves.states` equal before vs
  after, or an outer linear local the body consumed would be disposed N times
  (`times_body_consumes_local_error`);
- **borrow-state identity** (`src/check.rs:4882`): `live_derivs` equal before vs after, or a
  reference crosses the back-edge (`times_body_borrow_across_loop_error`);
- **row-effect equality** (`src/check.rs:4890`): the body's net row effect equals the row
  (`times_body_row_effect_error`).

The reviewer's first attack: with a quotation *parameter*, is there still a body to walk?
**Yes.** In `each`, `times` does **not** take the abstract `f`; it takes a *literal*
`[ ... f call ]` written at `each`'s own def site. That literal is fully visible. The only
opaque thing inside it is `f call`, which under D4/recon 7 checks against `f`'s *declared
effect* `[ 'T -- ]` exactly as an ordinary word call checks against a `Sig`. So the walk
still happens; the single novelty is that one term in the walked body (`f call`) transforms
the stack per a *declared effect* instead of a visible body. Take each obligation in turn.

1. **Move-state identity: implied by D3, checkable at the def site.** `f`'s declared effect
   `[ 'T -- ]` is a pure stack-to-stack row; it names no local, so walking `f call` at the
   def site records no move of any of `each`'s locals, and the before/after snapshot is
   unchanged. This is *sound for every inline site* because D3 restricts the real literal
   substituted for `f` to `Copy`-only captures: a linear (non-`Copy`) outer local can never
   be captured, so the spliced literal can never consume one, so it can never be disposed N
   times. A captured `Copy` local carries no consume obligation and has no destructor, so
   disposing it N times is a no-op. The obligation is discharged where the declared effect
   is checked; **it does not move to the inline site.**

2. **Borrow-state identity: enforced in two places, and both are needed.** At the def site,
   `f call`'s declared effect consumes its inputs and produces its outputs on the stack row
   and captures no borrow of an enclosing place (D3's "no borrows of enclosing places"), so
   `live_derivs` before vs after the `f call` term is unchanged and the def-site check is
   sound *for what it can see*. `each`'s own per-element borrow (`&arr i &>`) is created and
   consumed inside one iteration and is checked by the visible ops. But the def-site check
   is **not sufficient on its own**, and the spec states this rather than overclaiming: D3
   forbids *capturing* an enclosing borrow, and does not forbid a substituted literal from
   *creating* a borrow of a captured `Copy` local and leaving the resulting reference on its
   output row, which would ride the back-edge into the next iteration. That case is caught,
   but by the **splice-site re-check**: D2/R18 re-checks the spliced body in the caller's
   scope, where the existing `check_reference_across_back_edge` and `times`' own
   `live_derivs` comparison run against the now-concrete body. So obligation 2 is discharged
   by the def-site check plus the ordinary re-check the splice already performs, not by the
   def-site check alone. The diagnostic still lands at a call the **caller** wrote (their own
   literal created the borrow), never at a call `each`'s author never wrote, which is the
   property the brief actually asked for.

3. **Row-effect equality: expressible on the declared effect.** The times-body's net row
   effect is computed by composing its visible ops with `f`'s declared effect, entirely at
   the def site (`[ 'T -- ]` pops a `'T`, pushes nothing). No part of this needs the real
   literal. **It does not move to the inline site.**

**Conclusion, stated plainly and without papering over the set.** Obligations 1 and 3 are
discharged entirely at the def site: `Copy`-only capture (D3) is exactly the premise that
lets a declared effect stand in for every literal that will ever be substituted, so no
literal can consume an outer linear local, and the row effect composes from the declared
effect alone. Obligation 2 is discharged at the def site **for captured state** and by the
splice-site re-check **for borrows a literal creates internally** (above). No obligation is
*deferred* to the inline site in the sense the brief warned about — none of them produces a
diagnostic at a call the author of `each` never wrote. The checks that land at a call site
land on the caller's own literal: D3's capture restriction, the directional
literal-versus-declared-effect check (R11/R12), and a literal-created borrow crossing the
back-edge. The cost the brief warned about (a partial def-site check surfacing errors in
someone else's word) does not materialize; the weaker claim that every obligation is
*checkable from the declared effect alone* is not made, because it is not true of
obligation 2.

## Open questions the brief left for this spec (resolved here)

- **Q1 → R11/R12.** Directional checking of a literal against a declared parameter type:
  run the literal's body against the declared input row (its variables instantiated from
  the call), compare exit rows, no inference of a standalone effect (slice 4 D3). The
  diagnostic when the rows disagree names the word, the parameter position, and both rows.
- **Q2 → R16/R17.** The three `times` obligations at a *standalone* def site check against
  the declared effect, per the analysis above; the def-site check is total.
- **Q3 → R20.** A quotation-taking word mints **no** symbol and no `IrFunc` (recon 5): it is
  never monomorphized standalone, so `instantiation_symbol` is never computed for it, and
  slice 2's collision hazard is unreachable by construction. Two call sites passing
  *different* literals at the *same* concrete types each splice independently; there is no
  shared instance to collide.
- **Q4 → R21/R22.** Transitive inlining splices outermost-first and terminates because D5's
  cycle rejection makes the quotation-taking-word call subgraph a DAG. The cycle check and
  the splice agree on "un-inlinable": exactly *participates in a cycle in that subgraph*.
- **Q5 → R13.** A quotation parameter bound and used twice (`| f | ... f call ... f call`)
  is sound (D3 makes `f` `Copy`, so it may be called repeatedly) and splices the body once
  per use; code size is linear in the number of uses, with no dedup. Stated, not silent.
- **Q6 → R25.** Exit-criterion witnesses: a constant-stack run at 1M+ elements under a
  reduced `ulimit` is the primary witness (the `times` precedent); an `Instr::Call` count
  of zero in the lowered caller (precedent `src/ir.rs:4396`) is a *regression guard* against
  a future silent fallback, not the primary evidence.
- **Q7 → R23/R24.** REPL rejection sites: a session-typed definition naming a quotation
  parameter, and (pinned against what 5b actually shipped) an imported closure *exporting* a
  quotation-taking word. Both located, both lifted by 6c.
- **Q8 → R26.** The nine stale "Phase 6" diagnostics are corrected here (they are wording,
  this is the slice that makes them false, diagnostics are behaviour in this project).

## Requirements by stage

Diagnostics marked *(located)* are behavioural negatives asserting message text **and** the
named identifiers/positions, never an op name or an exit code.

### Surface syntax / parsing (`src/parser.rs`, `src/ast.rs`)

- **R1.** In **type position only**, a `[` is disambiguated by scanning to its matching `]`
  (tracking `[`/`]` depth) for a **top-depth `--`**: present routes to a new
  `parse_quotation_type_expr`, absent keeps today's `parse_array_type_expr`
  (`src/parser.rs:1445`). The scan is local and unambiguous (an array type can never
  contain `--`; arrays cannot hold quotations, slice 4, so nesting stays unambiguous). No
  new token, no new sigil.
- **R2.** `parse_quotation_type_expr` parses `[ <in-types> -- <out-types> ]` into the new
  quotation type (R4), each side a possibly-empty list of type expressions parsed by the
  existing `parse_type_expr` (so a `'T` element variable, a `&T`, an array, or a nested
  effect all reuse the current readers). The nil effect `[ -- ]` is legal (empty both
  sides). A row variable `..s` inside is **out of scope** (R28): a quotation effect in this
  slice is a fixed input/output list, matching what `each`/`map`/`fold` need.
- **R3** *(located)*. The malformed-array diagnostic stays sharp: `[i64]` (no top-depth
  `--`) still reaches `parse_array_count` and reports `array count must be a decimal
  literal` (`src/parser.rs:1479`), unchanged. An unterminated `[` in type position, and a
  `--` with a malformed type list on either side, are located parse errors naming the
  offending token. Golden asserts `[i64]` is *still* the array diagnostic, so the
  disambiguation cannot silently swallow it.

### Type representation (`src/ast.rs`, `src/ir.rs`)

- **R4.** `Type` (`src/ast.rs:670`) gains a `Quotation` variant carrying a `QuotEffectId`
  into an interned `(inputs: Vec<Type>, outputs: Vec<Type>)` registry, mirroring
  `Type::Array`'s interned `(element, count)` design so `Type` stays `Copy` and
  self-renders (a leaked `[ ... -- ... ]` spelling). **No "statically known" bit** (D6): the
  type says only "a quotation of this effect", never "a literal is known here"; knownness
  stays on `Slot.quot`.
- **R5.** `PolyType` gains the parallel case so a declared effect may mention the sig's
  type/length variables (`[ 'T -- ]` where `'T` is the element variable), with the interned
  effect's rows carrying `PolyType`s during a poly check.
- **R6.** Unification and `apply_subst`/`Subst` extend to the new variant: unifying two
  quotation effects unifies their input rows and output rows pointwise (equal arity
  required, else a located type mismatch), and `apply_subst` maps a substitution through
  both rows. This is what binds `'T` when a concrete literal or array instantiates a
  combinator. Golden/unit: `[ 'T -- ]` unified against `[ i64 -- ]` binds `'T = i64`; an
  arity mismatch (`[ 'T -- ]` vs `[ i64 i64 -- ]`) is a located mismatch.
- **R7.** `instantiation_symbol`/mangling and `IrType` gain **no** quotation case that is
  ever reached: a quotation-taking word mints no standalone instance (R20), so its type
  never reaches the backend. The mangling and `IrType`-lowering arms for a `Type::Quotation`
  are `unreachable!` with a comment pinning the reachable case to slice 7, guarded by a unit
  (R20u) asserting no quotation-taking word mints a symbol. This keeps D6 honest: the
  variant exists at the type layer without a runtime representation. **R7's `unreachable!`
  arms are only sound because of R7a; neither ships without the other.**
- **R7a** *(located)*. **The type-position audit: exactly one position accepts a quotation
  type, every other is a located rejection.** R2 parses a quotation effect through the
  ordinary `parse_type_expr`, so the new variant becomes writable in *every* type position
  the language has, and "out of scope" (R28) means unspecified, not rejected. Unspecified
  plus R7's `unreachable!` is a compiler panic, which is precisely the failure mode slice 4
  avoided for quotation *values* with its audit-table sweep
  (`quotation_as_operand_is_rejected_at_every_audited_site`, `src/check.rs:7001`). This is
  the same sweep for quotation *types*. The one legal position is a **direct input in a
  word's declared effect** (the quotation parameter this slice exists to add). Every other
  position is rejected with a located error naming the position and the offending type,
  before layout or lowering can see it:
  - a struct field or enum-variant payload field (`type: S f [ i64 -- ] ;`);
  - an **array element**, including the parse path R1's scan creates: `[ [ i64 -- ] 3 ]` has
    no *top-depth* `--` (the inner one sits at depth 1), so it takes the array branch and
    parses as an array of quotations. R1's justification ("arrays cannot hold quotations,
    slice 4") is a rule about quotation *values*; this is the first time a declared
    array-of-quotation *type* is expressible, and it must reject here rather than reach
    `check_no_linear_array_elements`-adjacent layout code;
  - an owned-cell payload (`^[ i64 -- ]`) and a reference referent (`&[ i64 -- ]`,
    `&![ i64 -- ]`);
  - a word's **output** position (there is no runtime value to return, D6/R28);
  - an `extern:` boundary type, in either direction (`check_extern_boundary_types`);
  - `main`'s signature;
  - **nested inside another quotation effect** (`[ [ i64 -- ] -- ]`, a quotation taking a
    quotation): unbudgeted here, deferred to slice 7, rejected rather than half-supported;
  - at the REPL, a session `type:` line and a session word signature reach the same
    rejections (R23 covers the parameter case; a `type:` field goes through the struct-field
    rejection above).
  Each rejection names slice 7 as the milestone that lifts it, matching R26's rewording so
  no diagnostic in the tree points at the wrong slice. The witness is one table-driven test
  in slice 4's audit shape, one row per position, asserting the message text and the named
  identifiers — not merely that compilation failed.

### Checking, monomorphic path (`src/check.rs`)

- **R8.** `call` (`src/check.rs:4763`) accepts an **abstract** quotation beside the literal
  it accepts today: if the popped `Slot.quot` is `Known(id)`, splice the literal (as today);
  else if `Slot.ty` is `Type::Quotation(eff)`, check against the declared effect directly
  (pop `eff.inputs` deepest-first with `match_slot`, push `eff.outputs`), no splice; else the
  reworded `call_needs_quotation_error` (R26). Bracketing/`tail = false` unchanged for the
  literal path; the abstract path splices nothing.
- **R9.** `times` (`src/check.rs:4797`) accepts an abstract quotation the same way: an
  abstract `f times` checks `f`'s declared effect must be row-preserving with a trailing
  `i64` index input (`[ ..row i64 -- ..row ]` shape, here the fixed-list form), and the three
  obligations reduce to *expressible-on-the-declared-effect* checks (row-effect equality
  from the declared rows; move/borrow identity trivially, since a declared effect names no
  local and captures no borrow, per "The hardest question"). The literal path is unchanged.
- **R10.** A quotation **argument** to a user word is no longer a blanket rejection. At the
  `env` argument loop (`src/check.rs:4427`, R9 of slice 4) and `check_poly_call`'s input
  loop (`src/check.rs:3289`), a popped quotation `Slot` whose target parameter position is a
  `Type::Quotation` is **accepted** and routed to the inliner (R18); a quotation against a
  *non*-quotation parameter is still rejected, with `reject_quotation_argument` reworded
  (R26) to name that the word does not take a quotation there.
- **R11** *(located)*. **Directional literal check.** A quotation *literal* passed to a
  declared quotation parameter is checked directionally: instantiate the parameter's
  declared input row from the call, run the literal's body against it via `check_terms`
  (bracketed like `call`, `tail = false`), and require the exit stack to match the declared
  output row. A mismatch is a located error naming the word, the parameter position, the
  declared effect, and the literal's actual effect. No standalone effect is inferred (slice 4
  D3): the input row is *given*, the output row is *checked*.
- **R12** *(located)*. **D3 capture restriction, enforced at the literal.** While checking a
  literal's body against a declared parameter (R11), a read that *consumes* a non-`Copy`
  enclosing local, or a `&`/`&!` borrow of an enclosing place, is a located rejection naming
  the local and the enclosing word. A `Copy` local read by value is allowed (D3). This is
  the one place D3 is enforced, and it lands at a call the caller wrote.
- **R13** *(located, positive+negative).** A quotation parameter is `Copy` (D3 forbids
  linear captures), so `| f | ... f call ... f call` is accepted and splices the body once
  per use (Q5); the positive golden asserts both splices run. A quotation parameter that a
  body *fails* to consume is not a leak (it is `Copy`), so no unconsumed-linear error fires
  on it; a unit pins that a quotation param never registers a move obligation.

### Checking, polymorphic path (`src/check.rs`)

- **R14.** `poly_term`'s `TermKind::Quotation` arm (`src/check.rs:3305`, slice 4's R5p
  outright rejection) is **lifted** for the immediately-consumed case: the poly stack gains
  a `quot` marker parallel to the monomorphic `Slot.quot` (a side vector on `PolyScope`/the
  poly stack keyed by stack position, since `poly_term` walks `Vec<PolyType>` with no
  `Slot`), so a quotation literal in a polymorphic body can be tracked to its `call`/`times`
  consumption and its identity is not erased into unification (the exact hazard slice 4
  cited). A quotation literal that reaches a position needing a runtime value in a poly body
  (a join, a store, a non-quotation argument) is still rejected, reusing the reworded
  wording (R26).
- **R15.** `poly_call_term` (`src/check.rs:3315`) intercepts `call`/`times` in a poly body:
  `call` against a `Known` marker splices the literal (poly `check_terms`); `call` against a
  `PolyType` quotation parameter checks against the declared effect (R8's poly twin);
  `times` likewise (R9's poly twin). A quotation *argument* to another poly word whose
  parameter is a quotation is accepted (R10's poly twin, routing to the inliner).
- **R16** *(located)*. **The standalone `times` obligations in a poly body.** With R14/R15,
  `each`'s body (`[ ... f call ] times`, poly) is walked at its def site and the three
  obligations run against `f`'s declared effect exactly as "The hardest question" derives:
  move-state identity, borrow-state identity, row-effect equality, all discharged at the def
  site. The negative goldens: a poly combinator whose times-body consumes an outer linear
  local (`times_body_consumes_local_error`), and one whose times-body borrow crosses the
  back-edge (`times_body_borrow_across_loop_error`), each located, named.
- **R17.** `each`/`map`/`fold` type-check standalone at `lib/combinators.sth`
  (`: each ( ['T 'N] [ 'T -- ] -- )`, `: map ( ['T 'N] [ 'T -- 'T ] -- ['T 'N] )`,
  `: fold ( ['T 'N] 'A [ 'A 'T -- 'A ] -- 'A )`), compositionally (D4): `map`/`fold` check
  `each` against its `PolySig`, never its body. Standalone checking is the D4 witness that
  the signature is not documentation over a macro.

### The inliner (`src/check.rs`, `src/ir.rs`)

- **R18.** **D2 term-splice at the call site.** When a call resolves to a quotation-taking
  word, the caller's quotation literal(s) are bound to the callee's quotation parameter
  name(s) as `Known` markers, a clone of the callee's AST body is spliced against the live
  stack (bracketed like `call`, `tail = false`), and the existing `call`/`times` fusion
  fires on the now-concrete `f call`/`f times`. This generalizes slice 4's intra-word fusion
  across a `:` boundary (slice 4 D5 deferred exactly this). Substitution is *binding the
  parameter to the caller's literal*, not string rewriting.
- **R19.** **The same at lowering.** `lower_call` (`src/ir.rs:2860`, which emits
  `Instr::Call` for every user word) takes a quotation-taking-word branch that inlines the
  callee body with the quotation params mapped to the caller's phantom quotation `Value`s,
  emitting **no `Instr::Call`**, transitively (R21). This is the mono equivalent of slice
  4's `call`-fusion across a `:` boundary.
- **R20.** **A quotation-taking word mints no symbol and no `IrFunc`** (recon 5, Q3): it is
  excluded from standalone monomorphization/emission; it exists only as an AST body to
  splice. `instantiation_symbol` is never computed for it, so slice 2's collision hazard is
  unreachable by construction. Unit R20u asserts the lowered module contains no `IrFunc`
  and no `Instr::Call` for a quotation-taking word, at both monomorphic and polymorphic
  consumers.
- **R21.** **Transitive inlining, outermost-first.** Splicing `map`'s body encounters
  `... each` (a quotation-taking-word call) and splices `each`'s body in turn (`map` over
  `each` inlines twice). Termination rests on R22: the quotation-taking-word call subgraph
  is a DAG, so the recursion bottoms out. The splice may assume acyclicity because R22 runs
  first.
- **R22** *(located)*. **D5 recursion rejection.** A pre-lowering pass builds the call graph
  over words that take a quotation parameter (edge A→B if A's body calls B passing a
  quotation) and rejects any cycle with a located error naming the cycle members, reusing
  the 3-colour DFS shape of `check_tail_call_cycles` (`src/check.rs:2434`) — the third
  instance of that precedent (recon 8), not a fourth invention. "Un-inlinable" is defined as
  exactly *participates in a cycle in this subgraph*, so the cycle check and the splice agree
  on the term. A self-recursive quotation-taking word is the minimal case.

### REPL (`src/repl.rs`), per D7 and against what 5b shipped

- **R23** *(located)*. A session line whose word signature names a `Type::Quotation`
  parameter is rejected at the definition chokepoint, with a located error naming the word
  and stating quotation-taking words are not yet supported at the REPL (6c). The inliner
  needs the callee body threaded in, and a session discards word bodies once a line compiles
  (the 6c retention problem); pinning it here rather than leaving it to a silent miscompile
  is the D7/5b discipline.
- **R24** *(located)*. An imported closure (5b's `import:` path) *exporting* a
  quotation-taking word is rejected at import time, naming the file and the word. Pinned
  against what 5b actually shipped: a quotation-taking word used purely *internally* to an
  imported closure inlines fine during that closure's native compilation; only *exporting*
  it to the session (where a later line would call it, needing the discarded body) is the
  chokepoint. This is a second located rejection beside R23, both lifted by 6c.
  *Known gap found in review:* this slice's implementation branch is based at `d033b2d`,
  which predates the slice-5b "REPL imports" merge (`df3cee0`, now on `main`). On that
  base, `import:` at the REPL is still 5a's blanket located rejection, so 5b's per-name
  import path R24 targets does not exist and R24 is unreachable, not merely untested.
  Left unimplemented rather than faked; whoever rebases/merges this slice onto a
  post-5b `main` must add R24's import-time rejection then, before the phase is
  considered closed, since the gap is a silent-miscompile hazard the moment the 5b
  import path returns (an imported closure could retain its `PolyWordEntry` while
  discarding the body the inliner needs).

### Stale diagnostics (recon 9) and dogfood/docs

- **R26.** The nine "Phase 6" diagnostics (`src/check.rs:2163`, `4395`–`4414`, `5621`,
  `5647`, `5659`, `5670`) are corrected. Eight are pure wording: a quotation reaching a
  position that needs a runtime *value* (a join, a store, an operator operand, a residual
  line stack, `call`/`times` without a resolvable quotation) is rejected because runtime
  quotation *values* are **slice 7**, not "Phase 6". The ninth, `reject_quotation_argument`
  (`src/check.rs:5659`), changes *behaviour* (R10: accept a literal for a declared quotation
  parameter) and its still-rejecting wording is retargeted to "this word does not take a
  quotation" rather than "higher-order user words are Phase 6". The slice-4 goldens asserting
  the old wording (the R9/R11/R19 negatives) are updated in lockstep (sanctioned edit,
  called out per phase).
- **R27.** Dogfood: an earlier program is rewritten to use `each`/`map`/`fold` from
  `lib/combinators.sth` (ROADMAP's 6a dogfood), building and running to the same result as
  its hand-threaded original. Docs: ROADMAP slice-6a marked implemented; DESIGN.md's
  "Control flow and iteration" gains a paragraph recording that combinators are now library
  words inlined by term splicing (D2), the type became nameable at the `Type`/`PolyType`
  layer only (D6, `IrType` deferred to slice 7), inlining is total (D5), and the REPL is a
  located rejection (D7).

### Out of scope, stated so a phase cannot drift into it

- **R28.** Slice 7's runtime-quotation representation stays out: no `IrType` variant, no
  calling convention, no per-literal env struct, no `(code, env)` value, no escape rules, no
  quotation in an array element / struct field / branch join, no dispatch table, no
  non-inlined higher-order call, no upward closure, and no row variable `..s` *inside* a
  quotation effect type. 6b stays out: the polymorphic-`if` and polymorphic-self-tail gaps,
  and `filter`/`while` on top of them. 6c stays out except R23/R24's located rejections.
  `while` as a second floor intrinsic stays declined. Generic `type:` declarations are
  Phase 6. The pre-existing native holes on ROADMAP for slice 8 (duplicate `main`,
  duplicate word names in one file, destructure-bypasses-`drop`) stay recorded, not fixed.

## Load-bearing invariants preserved

Backend stays QBE; `Ptr[T]` opaque; no LLVM, native backend, JIT, or comptime. `IrType`
gains no quotation variant (D6); the runtime type is slice 7. The linear spine is
untouched: `dup`/`drop`, move-by-default, use-exactly-once; a quotation parameter is `Copy`
(D3) and needs neither. `core` stays `no_std` (a non-inlined-away quotation never exists in
this slice, so nothing new escapes to `alloc`). Constant stack is preserved: an inlined
combinator lowers to the same `times` back-edge machinery the hand-inlined form uses
(recon 2). No new `Instr`/`Terminator`; no `qbe.rs` change. A program using no quotation
parameter lowers byte-for-byte as today (the inliner branch is reached only for a
quotation-taking callee).

## Phase sequencing

1. **(hard)** The type + the monomorphic inliner. Parse disambiguation (R1–R3); the
   `Type`/`PolyType` quotation variant with unification/`apply_subst` and the
   unreachable-and-guarded mangling/`IrType` arms (R4–R7); `call`/`times` accept an abstract
   quotation (R8/R9); the argument sites accept a literal for a declared quotation parameter
   with the directional check and D3 capture check (R10–R13, monomorphic); the
   interprocedural term-splice inliner for a *monomorphic* quotation-taking word (R18–R20);
   the D5 cycle rejection (R22); `reject_quotation_argument`'s behaviour+wording change and
   the slice-4 goldens it touches (part of R26/R10). Exit: a monomorphic
   `: apply ( i64 [ i64 -- i64 ] -- i64 ) call ;` with `3 [ 1 + ] apply` inlines, emits no
   `Instr::Call`, and runs; recursion among quotation-taking words is located. This phase is
   `hard`: it is the slice-1-shaped type change plus the compiler's first inliner.
2. **(hard)** The polymorphic path + the library. Lift R5p for the immediately-consumed
   literal and carry the poly `quot` marker (R14); abstract `call`/`times` and quotation
   arguments in a poly body (R15); the standalone `times` obligations against a declared
   effect (R16, the hardest-question requirements); `each`/`map`/`fold` at
   `lib/combinators.sth` checkable standalone and inlining at concrete call sites (R17);
   transitive inlining (`map` over `each`, R21); the constant-stack witness at 1M+ elements.
   Exit: `each`/`map`/`fold` written in Sooth, inline to a tight loop, run in constant stack.
   `hard` because the poly path is the `Vec<PolyType>`/no-`Slot` extension slice 4 flagged.
3. **(standard)** REPL located rejections (R23/R24, pinned against 5b) and the eight
   remaining "Phase 6" wording corrections (R26). Isolated: diagnostics only, no codegen.
4. **(standard)** Dogfood + docs (R27): rewrite an earlier program over `each`/`map`/`fold`,
   mark ROADMAP 6a implemented, extend DESIGN.md's control-flow section.

The type and the inliner land together in phase 1 because a quotation type with no inliner
compiles no call site at all (a monomorphic quotation-taking word would emit an
`Instr::Call` to a word with no `IrFunc`, a link error) — it must inline to lower. The poly
path is a strict addition on top, and the library is its first real consumer. Each phase
leaves the tree green.

## Exit criteria

Goldens in `tests/phase4_combinators.rs` (value/effect via `run_src`; constant-stack via
`run_stack_bounded_src`, `ulimit -s`, returning `Option<i32>` only so a value claim cannot
ride it alone; diagnostic goldens via a `check_error`/`parse_error` helper added to the
file, sanctioned). Units sit beside their stage functions per the CLAUDE.md convention.
Every negative golden asserts the message text and the named identifiers, never an op name
or an exit code.

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | `[ i64 -- i64 ]` in a signature parses to a quotation type; `[ -- ]` (nil effect) parses | `quotation_type_in_signature_parses` | golden | 1 |
| 1b | `[i64]` (no top-depth `--`) is *still* the array-count diagnostic, unchanged | `array_type_without_arrow_stays_array_diagnostic` | golden | 1 |
| 1c | malformed quotation effect (`[ i64 -- ]` unterminated, bad type list) is a located parse error naming the token | `malformed_quotation_type_is_located_parse_error` | golden | 1 |
| 2 | `[ 'T -- ]` unifies against `[ i64 -- ]` binding `'T = i64`; arity mismatch is a located type mismatch | `quotation_effect_unifies_and_binds_variable` | unit (check) | 1 |
| 2b | a quotation type in every audited non-parameter position (struct field, enum payload, array element, cell payload, reference referent, word output, `extern:` either direction, `main`, nested inside another effect) is a located rejection naming the position and slice 7 | `quotation_type_is_rejected_at_every_audited_position` | golden (table-driven) | 1 |
| 2c | `[ [ i64 -- ] 3 ]` (no top-depth `--`, so it takes the array branch) is a located rejection naming the array element position — not an array-count error, not a panic | `array_of_quotation_type_is_located_rejection` | golden | 1 |
| 2d | every R7 `unreachable!` arm stays unreached: no audited position reaches mangling or `IrType` lowering | `quotation_type_never_reaches_mangling_or_irtype` | unit (ir) | 1 |
| 3 | `: apply ( i64 [ i64 -- i64 ] -- i64 ) call ;` with `3 [ 1 + ] apply .` prints `4` | `monomorphic_quotation_taking_word_inlines_and_runs` | golden | 1 |
| 3b | the lowered caller of `apply` contains no `Instr::Call` and no `IrFunc` named `apply` | `quotation_taking_word_emits_no_call_and_no_irfunc` | unit (ir) | 1 |
| 4 | a literal whose effect disagrees with the declared parameter is a located error naming word, parameter, both effects | `literal_effect_mismatch_against_parameter_is_error` | golden | 1 |
| 5 | a literal capturing (consuming) a linear enclosing local, or borrowing an enclosing place, is a located D3 rejection naming the local | `quotation_literal_capturing_linear_local_is_error` | golden | 1 |
| 5b | a literal reading a `Copy` enclosing local by value is accepted and runs | `quotation_literal_capturing_copy_local_runs` | golden | 1 |
| 6 | `\| f \| ... f call ... f call` splices the body twice; both runs observed | `quotation_parameter_used_twice_splices_twice` | golden | 1 |
| 6b | a quotation parameter registers no move obligation (it is Copy) | `quotation_parameter_is_copy_no_move_obligation` | unit (check) | 1 |
| 7 | a quotation against a *non*-quotation parameter is still rejected, reworded (not "Phase 6") | `quotation_against_non_quotation_parameter_is_error` | golden | 1 |
| 8 | self-recursive quotation-taking word is a located cycle rejection naming it | `recursive_quotation_taking_word_is_located_error` | golden | 1 |
| 8b | a two-word cycle among quotation-taking words names both members | `quotation_taking_word_cycle_names_members` | golden | 1 |
| U20 | no quotation-taking word mints a symbol / reaches mangling | `quotation_taking_word_mints_no_symbol` | unit (check) | 1 |
| 9 | `each` type-checks standalone at `lib/combinators.sth` | `each_checks_standalone` | golden | 2 |
| 9b | `map`/`fold` check `each`/`times` compositionally against a signature, not a body | `map_and_fold_check_compositionally` | golden | 2 |
| 10 | `arr [ . ] each` over `[i64 4]` prints each element (inlined) | `each_over_array_inlines_and_runs` | golden | 2 |
| 10b | `map` over `each` inlines twice; the lowered caller has no `Instr::Call` | `map_over_each_inlines_transitively` | unit (ir) | 2 |
| 11 | `fold` sums `[i64 4]` to `28` | `fold_computes_sum` | golden | 2 |
| 12 | a poly combinator whose times-body consumes an outer linear local is located, naming it (fires at the combinator's own def site) | `poly_combinator_consuming_local_is_error` | golden | 2 |
| 12b | a poly combinator whose times-body borrow crosses the back-edge is located (def site) | `poly_combinator_borrow_across_loop_is_error` | golden | 2 |
| 12c | a caller's literal that creates a borrow of a captured `Copy` local and leaves the reference on its output row is located at the **splice site**, naming the caller's own literal | `literal_created_borrow_across_loop_is_error_at_splice_site` | golden | 2 |
| 13 | a quotation literal at a runtime-value position in a poly body is rejected, reworded | `quotation_at_runtime_position_in_poly_body_is_error` | golden | 2 |
| 14 | `each` over 1_000_000+ elements runs in constant stack under a reduced `ulimit` (`Some(0)`) | `each_over_a_million_runs_in_constant_stack` | golden | 2 |
| 14b | the inlined `each` lowers to a loop header/back-edge, no per-element `Instr::Call` | `each_lowers_to_a_loop_not_a_per_element_call` | unit (ir) | 2 |
| 15 | a session line defining a quotation-taking word is a located REPL rejection naming the word | `repl_quotation_taking_definition_is_rejected` | golden | 3 |
| 16 | importing a closure that *exports* a quotation-taking word is a located rejection naming file and word; a purely-internal one imports fine — **deferred**, see R24's known-gap note (unreachable on this branch's pre-5b base) | `repl_import_exporting_quotation_word_is_rejected` | golden | 3 |
| 17 | the eight reworded diagnostics name slice 7 (runtime values), not "Phase 6" | `stale_phase6_diagnostics_are_reworded` | golden | 3 |
| 18 | an earlier program rewritten over `each`/`map`/`fold` builds and matches its hand-threaded result | `combinators_dogfood_matches_hand_threaded` | golden | 4 |

Load-bearing units (mutation-test the guards): 2, 2b, 2c, 2d, 3b, 6b, U20, 10b, 14b.
Criteria 2b/2c/2d are what make R7's `unreachable!` arms sound rather than hopeful, so
deleting any one audited rejection must make its row fail, not merely change a message.
Criterion 14 is the
primary constant-stack witness (the `times` precedent); 3b/10b/14b are regression guards
against a future silent fallback (with total inlining, "it compiled" already implies "it
inlined", so a zero-`Call` count exists to catch a fallback being added silently). Criteria
5/5b are the D3 pair (a linear capture rejects, a Copy capture runs); 12/12b are the
regression witnesses for the obligations that "The hardest question" resolves at the def
site (they must fire from `each`'s own def-site check, not at a call site), and 12c is its
counterpart for the one case obligation 2 leaves to the splice-site re-check — the three
together pin *where* each check lives, which is the claim a reviewer should attack.

## Sanctioned edits to existing tests

The slice-4 negatives that asserted the old "Phase 6" wording change with R26: the R9
`quotation_passed_to_user_word_…` golden (now the callee's parameter decides accept vs
reject), the R11 audited-site wording, and the R19 residual-line wording. No behaviour a
non-quotation program relies on changes. Each edit is called out in its phase's commit, the
way slice 3 called out its phi-count edits.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "title": "Quotation type + the monomorphic inliner",
      "focus": "Disambiguate a type-position `[` on a top-depth `--` (quotation effect vs array), keeping the malformed-array diagnostic sharp; add a `Type`/`PolyType` quotation-effect variant carrying an interned declared effect with unification and apply_subst, and unreachable-and-guarded mangling/IrType arms (no runtime representation, D6); sweep every other type position with a located rejection naming the position and slice 7 (R7a: struct field, enum payload, array element including the `[ [ i64 -- ] 3 ]` parse path the top-depth scan creates, cell payload, reference referent, word output, extern either direction, main, and nesting inside another effect), table-driven in slice 4's audit shape, since without it the unreachable arms are a panic rather than a guarantee; make `call`/`times` accept an abstract quotation checked against its declared effect beside the literal they accept today; make the user-word and poly-call argument sites accept a quotation literal for a declared quotation parameter, checked directionally against the declared effect, enforcing D3's Copy-only capture restriction at the literal; build the interprocedural term-splice inliner for a monomorphic quotation-taking word so it emits no Instr::Call and no IrFunc (total inlining, forced by recon 5); reject recursion among quotation-taking words with a located cycle error reusing the 3-colour DFS precedent; change reject_quotation_argument's behaviour and wording and update the slice-4 goldens it touches. Exit: a monomorphic quotation-taking word inlines and runs, recursion is located. R1-R13, R18-R22, part of R26.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "title": "The polymorphic path and the combinator library",
      "focus": "Lift slice 4's outright rejection of a quotation literal in a polymorphic body for the immediately-consumed case, carrying a quot marker parallel to Slot.quot on the poly stack so the literal's identity is not erased into unification; intercept abstract call/times and quotation arguments in poly_call_term; run the three standalone times obligations (move-state identity, borrow-state identity, row-effect equality) against a declared effect at the combinator's own def site, per the hardest-question analysis, so the check is total and no diagnostic lands at a call the author never wrote; write each/map/fold at lib/combinators.sth, checkable standalone and compositional (map/fold check each against its signature, not its body); inline them transitively at concrete call sites (map over each inlines twice); witness constant-stack iteration at 1M+ elements under a reduced ulimit and a zero per-element Instr::Call count. R14-R17, R21.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "title": "REPL located rejections and stale-diagnostic wording",
      "focus": "Reject a session line defining a quotation-taking word with a located error naming the word (the inliner needs a body the session discards, the 6c retention problem); reject an imported closure that exports a quotation-taking word with a located error naming file and word, pinned against what slice 5b shipped so a purely-internal quotation-taking word in a closure still imports fine; correct the eight remaining stale Phase 6 diagnostics to name slice 7 (runtime quotation values) instead. Diagnostics only, no codegen. R23, R24, remainder of R26.",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 4,
      "title": "Dogfood and docs",
      "focus": "Rewrite an earlier program to use each/map/fold from lib/combinators.sth, verifying it builds and runs to the same result as its hand-threaded original; mark ROADMAP slice-6a implemented; extend DESIGN.md's control-flow section to record that combinators are now library words inlined by term splicing, the quotation type became nameable at the Type/PolyType layer only with the runtime representation deferred to slice 7, inlining is total, and the REPL stays a located rejection until 6c. R27.",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
