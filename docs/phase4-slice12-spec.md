# Phase 4 Slice 12: combinator recognition becomes declared (spec)

`inline` becomes the *single* route by which a word is recognised as a combinator
(a word that mints no `IrFunc` and is spliced at every call site). The inference
route (`word_declares_quotation_parameter`'s leg in `is_combinator`,
`src/check/combinators.rs:170-173`) is retired. A word declaring a `~[ ... ]`
parameter must say `inline`; a located error where it does not. The tilde becomes
writable and required at a `~[ ... ]` argument. And, because retiring the
inference makes an ordinary `[ ... ]` *parameter* into a real call for the first
time, this slice is not done until that real-call path lowers and a witness runs.

Four parts in the ROADMAP's own lettering:

- **A** retire the inference leg of `is_combinator`.
- **B** require `inline` on a `~[ ... ]`-declaring word (located error otherwise);
  migrate the library.
- **C** add the `parse_term` arm so `~[ ... ]` is writable as a body literal, and
  require the tilde at a `~[ ... ]` parameter (a corpus-wide call-site migration).
- **D** make the ordinary-`[ ... ]` real-call path lower.

All claims below are anchored to `main` at `0eb7c84` and verified against the cited
source. The backend is QBE; IR stays backend-neutral; `core` stays `no_std`; the
linear spine is untouched (a quotation is neither `Copy` nor auto-dropped; the
drop obligation on any referent is unchanged).

## Settled open questions (from the brief)

These were the brief's six open questions. Each is settled here and does not
reopen the brief's Decisions section.

**OQ1 — where the callee parameter shape lives for part D: extend `Arity`.**
`Arity` is today `type Arity = (usize, usize, Option<IrType>)`
(`src/ir/types.rs:406`) = `(in_arity, out_arity, ret_ty)`. The ordinary user-word
dispatch (`src/ir/func_builder/calls.rs:649-680`) has only this name-keyed value
in scope; it does not hold the callee's `WordDef`. Threading `module.words` (or
the checker's typed env) down into `FuncBuilder` to re-read the callee signature at
each call site is a wider change than extending the value already looked up, and it
has no REPL analogue (the REPL has no `module.words`). Therefore `Arity` grows one
field naming, per callee, which input slots are quotations and each such slot's
quotation `IrType` (the `IrType` carries the interned effect the `(code, env)`
layout needs). `Arity` is converted from a bare tuple to a named struct so the new
field is not a fourth anonymous tuple element at the ~half-dozen destructuring
sites. The field **must** be populated in *both* env builders:

- the batch builder (`src/ir/driver.rs:109-140`), from `w.effect.inputs` /
  `decl.effect.inputs`;
- the REPL builder `ir_arity_env` (`src/repl.rs:142-150`), from the checker
  `Overload`'s `sig.inputs`.

R-D2 below makes the REPL population a hard requirement (a golden), so the REPL
half of part D cannot silently go unlowered.

**OQ2 — `lib/arrays.sth`'s `bin_search`/`sort`: retype to `~[ ... ]`.** The
comparator becomes `~[ 'T 'T -- i64 ]` and both words gain `inline`; they stay
spliced inline combinators. Rationale: leaving the comparator ordinary would route
the corpus's hardest word (`sort` — polymorphic, splices `times`, uses rows, calls
the comparator indirectly) through the least-tested part-D path, which is a risk
this slice explicitly declines. The `arrays.sth:1-20` comment gives no design
reason for the ordinary flavour, so this recovers no intent. The `sort` call site
in `tests/phase4_slice6g.rs:386` (`d s [ | x y | x y - ] a::sort`) migrates to
`~[ | x y | x y - ]` with the rest of the corpus (part C, phase P2).

**OQ3 — `branch`'s arms stay ordinary `[ ... ]`.** `branch` is a hardcoded
primitive (`src/check/terms.rs:264`, `check_branch` `:1259`), not a declared
`~[ ... ]` word, so it has *no signature flavour to read*. The required-tilde rule
of part C keys on a **declared `Type::InlineQuotation` parameter**; `branch` has
none, so a `branch` literal arm is neither required nor forbidden to carry the
tilde. Consequence: `lib/core.sth:17-22`'s six comparison words keep their
`[ true ] [ false ] branch` bodies **unchanged** (no file edit), and `if`/`unless`
keep forwarding their `~` locals to `branch` (a forwarded parameter, not a literal,
so the tilde rule does not apply). Requiring `~` at `branch` arms would force
migrating those six bodies for zero capability gain, since `branch`'s own lowering
splices both arms regardless of spelling. **Files changed by OQ3: none** (the
point of the decision is that `core.sth:17-22` is *not* touched).

**OQ4 — the REPL's four retention sites: move the gate to `is_combinator`; reject
the real-call shape.** Real REPL support for a quotation-taking word that lowers to
a real call is **out of scope** for this slice: it requires the `(code, env)` ABI
to work across a `dlopen` boundary (the quotation built on a later line, its code
pointer resolved through `RTLD_GLOBAL`), an untested surface that deserves its own
slice. Instead, the four REPL sites that gate combinator retention on
`word_declares_quotation_parameter` (the definition path `eval_def`
`src/repl.rs:2565`, and the three import sites `:1886`, `:1937`, `:2014`) move to
`is_combinator`, so REPL retention can never silently diverge from the batch
compiler's recognition. A word declaring an ordinary `[ ... ]` parameter (the
`apply` shape) is then **not** retained as a combinator, would route to the REPL's
ordinary `.so`-minting path, and instead produces a **located** "not supported in
the REPL" error (E4). This is a scope boundary, not a softened diagnostic tier: the
batch compiler lowers the shape fully (part D); only the REPL refuses it.

**OQ5 — the row-combinator ICE: part C adds no positional reject.** The
`parse_term` arm mints an `InlineQuotation`-flavoured literal and does **not**
reject a `~[ ... ]` literal "outside an argument position". Two reasons: (1)
`parse_term` (`src/parser.rs:2301`) parses a flat term stream and has no notion of
"argument position" — that is a check-time property, so a positional reject would
be a new *checker* rule on a shape orthogonal to declared-vs-inferred recognition;
(2) the row-carried quotation ICE (recon 8, `[[project_row_combinator_quotation_ice]]`)
is out of scope (brief decision 6), and the ordinary `[ ... ]` spelling already
reaches it, so gating only the `~[ ... ]` spelling protects nothing while adding a
guard the slice must then mutation-test. A `~[ ... ]` literal left *on the stack*
is caught by the existing quotation-on-stack rejection, which keys on
`is_quotation_type` (`src/ast.rs:966-971`) and so already covers both flavours; the
only remaining unguarded hole is the row-carried case, which this slice notes and
does not touch. The spec records: part C gives that hole a second spelling; its fix
stays a future slice.

**OQ6 — diagnostic wording:** pinned as E2 (part B), E3a and E3b (part C flavour
mismatches), and E4 (part C / OQ4 REPL boundary) below.

## Requirements

Numbered and traceable. Each maps to a named test or golden in the exit criteria.

### Part A — retire the inference leg

- **R-A1.** `is_combinator` (`src/check/combinators.rs:170-173`) becomes
  `matches!(word.body, WordBody::Terms { .. }) && word.declares_inline`. The
  `word_declares_quotation_parameter` disjunct is deleted.
- **R-A2.** `word_declares_quotation_parameter` (`:181`) and `poly_input_is_quotation`
  (`:195`) **survive unchanged**: they retain ~20 "is this slot a quotation?"
  callers across `check/{poly,terms,audits,captures}.rs`. Only the one "does this
  word splice?" use in `is_combinator` is retired. A spec that deletes the function
  is wrong.
- **R-A3.** Every consumer that reads `is_combinator` (`collect_combinators`,
  `combinator_index`, `check.rs`, `ir/driver.rs`'s `combinator_indices`, and the
  REPL via `combinator_of`) inherits the new meaning with no further plumbing. The
  REPL's four *direct* `word_declares_quotation_parameter` gates are handled by
  R-D3, not here.
- **R-A4.** The doc comment on `is_combinator` is updated to state the current
  design (declared, single route), with no history narration.

### Part B — require `inline` on a `~[ ... ]` parameter, migrate the library

- **R-B1.** A word whose declared effect names a `Type::InlineQuotation`
  (`~`) input parameter and does not declare `inline` is a located error at the
  definition (E2). The rule is phrased over the `~` case (`Type::InlineQuotation`),
  **not** over `word_declares_quotation_parameter` (which also matches ordinary
  `Type::Quotation`): a `~` parameter is unrepresentable at runtime, so it can only
  be spliced, so `inline` is mandatory; an ordinary `[ ... ]` parameter is
  representable, so it is a real call by default and needs no `inline`.
- **R-B2.** The R-B1 check runs in `check_inline_declaration`'s neighbourhood
  (`src/check/word_entry.rs:66`, the pre-body pre-pass), as a new gate distinct
  from slice 11's four (`main`, clause body, variable-bearing signature,
  builtin-operator name). It must run on both the batch path (`check.rs:549`) and
  the REPL definition path (`repl.rs:2559`).
- **R-B3.** Library migration: the nine `~[ ... ]` words that declare no `inline`
  today gain it — `lib/combinators.sth`: `times-helper` (`:30`), `times` (`:37`),
  `each` (`:40`), `map` (`:45`), `fold` (`:50`), `filter` (`:63`), `while` (`:76`);
  `lib/core.sth`: `if` (`:42`), `unless` (`:48`). (The six comparison words
  `core.sth:17-22` are already `inline` and take no quotation parameter; untouched.)
- **R-B4.** OQ2 migration: `lib/arrays.sth`'s `bin_search` (`:29`) and `sort`
  (`:70`) retype their comparator parameter from `[ 'T 'T -- i64 ]` to
  `~[ 'T 'T -- i64 ]` and gain `inline`. This keeps them combinators across part A
  (a splice before, a splice after), so their emitted QBE is byte-identical.
- **R-B5.** Byte-identical guarantee: the nine R-B3 words and the two R-B4 words
  are all splices before and after, so the 35 golden `.sth` files and the `sort`
  golden produce byte-identical output. Only words that change *category* (the
  part-D `apply`-shaped witnesses, which are new) produce new codegen.

### Part C — write and require the tilde

- **R-C1.** `parse_term` (`src/parser.rs:2301`) gains a `Token::TildeLBracket` arm
  minting an `InlineQuotation`-flavoured quotation literal (the ordinary
  `Token::LBracket` arm at `:2367` mints a `Type::Quotation`-flavoured
  `TermKind::Quotation`; the new arm mints the `~` flavour). A `~[ ... ]` in a body
  no longer falls to the generic `other =>` arm's "unexpected token TildeLBracket".
- **R-C2.** The tilde is required exactly at a `Type::InlineQuotation` parameter and
  nowhere else. At the argument-matching sites that already resolve the parameter
  type (`inline_combinator` / the poly-combinator argument loop / `check_terms`'
  argument materialization at `src/check/terms.rs:698-705`), the literal's flavour
  is compared against the parameter's flavour:
  - an ordinary `[ ... ]` literal at a `~[ ... ]` parameter is a located error (E3a);
  - a `~[ ... ]` literal at an ordinary `Type::Quotation` boundary (parameter,
    field/array store, word output) is a located error (E3b).

  A direct `[ ... ] call` is **not** one of those boundaries. Nothing is
  materialized there: `call` on a literal splices it under either spelling, so
  the flavour decides nothing and there is no resolved parameter type to compare
  against. The rule could only be enforced syntactically, by inspecting the term
  adjacent to the `call` — the same mechanism OQ5 declines for the positional
  case, and holed the same way (`~[ ... ] 5 swap call` reaches the identical
  splice through an intervening term). Both spellings are accepted at a direct
  `call`.
- **R-C3.** Corpus call-site migration: every combinator call site in `lib/` and
  `examples/` gains the tilde — `[ ... ] if` → `~[ ... ] if`, `[ ... ] times` →
  `~[ ... ] times`, and the `c::fold`/`c::map`/`c::each`/`c::filter`/`c::while`,
  `if`/`unless`/`while`, and `a::sort`/`a::bin_search` sites. The `sort` call site
  in `tests/phase4_slice6g.rs:386` migrates from `[ | x y | x y - ]` to
  `~[ | x y | x y - ]`.
- **R-C4.** First-class ordinary quotations stay **unmigrated**:
  `examples/capturing_dispatch.sth` (`seed ( -- [ -- i64 ] ) [ 0 ] ;`, the
  `[ r i >usize &> @ ]` closures stored into a table) keeps ordinary `[ ... ]`
  spelling and must still run. R-C2's E3b protects this line: a `~[ ... ]` at an
  ordinary boundary is rejected, so the distinction is enforced, not merely
  available.
- **R-C5.** No positional reject of a `~[ ... ]` literal is added (OQ5). A
  `~[ ... ]` left on the stack is caught by the existing `is_quotation_type`-keyed
  stack-residue rejection; the row-carried ICE stays out of scope.

### Part D — lower the ordinary-`[ ... ]` real call

- **R-D1.** `Arity` (`src/ir/types.rs:406`) grows a field naming which input slots
  are quotations and each such slot's quotation `IrType` (OQ1). `Arity` is
  converted from a tuple to a named struct; all destructuring sites (including
  `calls.rs:648`, `:658`) are updated.
- **R-D2.** The new field is populated in the batch env builder
  (`src/ir/driver.rs:109-140`, from `w.effect.inputs` / `decl.effect.inputs`) **and**
  in the REPL builder `ir_arity_env` (`src/repl.rs:142`, from `sig.inputs`). The
  REPL population is exercised by a golden (part of R-D6 / the REPL branch of OQ4),
  so the REPL half cannot go silently unlowered.
- **R-D3.** At the ordinary user-word dispatch (`src/ir/func_builder/calls.rs:649-680`),
  before building `Instr::Call(ret, sym, args)`, each argument the callee's `Arity`
  marks as a quotation slot is materialized via `materialize_if_phantom`
  (`src/ir/func_builder/quotation.rs:15`) with that slot's quotation `IrType`,
  turning the phantom `I64` `Value` (`calls.rs:76-83`) into the real `(code, env)`
  aggregate. The checker half already produces the `(code, env)` at the boundary
  (`src/check/terms.rs:698-705`, `materialize_quotation_at_boundary` with
  `escaping = false`, gated on `Type::Quotation` — dead until part A, live after).
- **R-D4.** No new machinery beyond R-D1..R-D3: no closure capture, storage, or
  dispatch beyond what 7b already ships. `IrType::Quotation` handling and the
  `(code, env)` layout are the existing ones. The IR stays backend-neutral.
- **R-D5.** REPL reconciliation (OQ4): the four retention gates
  (`repl.rs:2565`, `:1886`, `:1937`, `:2014`) move from
  `word_declares_quotation_parameter` to `is_combinator`. A word that declares an
  ordinary `[ ... ]` parameter but is not a combinator (the `apply` shape) is
  rejected in the REPL with located error E4; it is not routed to the untested
  cross-`dlopen` `(code, env)` path.
- **R-D6.** Witnesses: `apply` (recon 5) lowers to a real `Instr::Call` (asserted
  on lowered IR, `nm` shows an `apply` symbol), runs, prints `6`; a quotation
  passed down two real-call levels; and a word that calls a quotation and returns.

## Located error definitions

House form: a located error carries a source span and exact message text pinned by
a diagnostic golden.

- **E2 (part B, R-B1).** A `~[ ... ]` parameter without `inline`. Located at the
  word definition (the word-name span, matching slice 11's `check_inline_declaration`
  rejections). Text:

  > word `{name}` declares an inline-quotation parameter `{param}` but is not
  > `inline`; a `~[ ... ]` quotation can only be spliced, so the word must declare
  > `inline`

- **E3a (part C, R-C2).** An ordinary `[ ... ]` literal at a `~[ ... ]` parameter.
  Located at the argument literal. Text:

  > this argument is an ordinary `[ ... ]` quotation but `{name}` declares parameter
  > `{param}` as inline `~[ ... ]`; write it `~[ ... ]`

- **E3b (part C, R-C2).** A `~[ ... ]` literal at an ordinary `[ ... ]` boundary.
  Located at the literal. `{name}` is the parameter's word, the returning word, or
  the store operator, so the text names the *expectation* rather than a parameter
  declaration (unlike E3a, which can only ever fire at a declared parameter: a
  `Type::InlineQuotation` is unrepresentable, so it is never an output or a store
  target). Text:

  > this quotation is inline `~[ ... ]` but `{name}` expects `{param}`, an ordinary
  > `[ ... ]`; write it `[ ... ]`

- **E4 (part C / OQ4, R-D5).** An ordinary-`[ ... ]`-parameter word (a real-call
  quotation-taking word) defined in the REPL. Located at the definition. Text:

  > word `{name}` takes a `[ ... ]` quotation parameter and lowers to a real call,
  > which is not supported in the REPL

## Mutation-testing requirements

Every new guard must be proven capable of failing when the change it guards is
reverted. Reading the test does not catch a placebo; this project has shipped
placebo tests repeatedly. Each item below names the mutation and the test that must
turn red under it.

- **M-A (part A placebo, the brief's named hazard).** Construct a `WordDef`
  directly (not via an end-to-end build) with an ordinary `[ ... ]` parameter and
  no `inline`, and assert `is_combinator` returns `false`; construct one with
  `inline` and assert `true`. Mutation: re-add the `word_declares_quotation_parameter`
  disjunct to `is_combinator`. The ordinary-`[ ... ]` case must flip to `true` and
  the test must fail. An end-to-end "it still builds" test passes either way and is
  a placebo.
- **M-B (part B).** The E2 diagnostic golden (a `~[ ... ]` word without `inline`).
  Mutation: delete the R-B1 gate. The word compiles instead of erroring; the golden
  must fail. Assert the exact E2 text, not merely "rejected".
- **M-C (part C placebo, the brief's named hazard).** The E3a negative golden (an
  ordinary `[ ... ]` at a `~` parameter). Mutation: remove the flavour comparison in
  R-C2. The ordinary literal is silently accepted; the golden must fail. Assert the
  exact E3a text at the argument site. A separate E3b negative guards the other
  direction under the same mutation.
- **M-D (part D).** The `apply` IR witness (R-D6). Mutation: skip
  `materialize_if_phantom` at the R-D3 dispatch. The phantom `I64` reaches
  `Instr::Call`; the witness (which asserts the `Instr::Call` argument is the
  materialized `(code, env)` value on lowered IR, and that the program prints `6`)
  must fail. "It builds" or "exit 0" passes under the un-materialized path in some
  configurations and is a placebo.

Because R-A1 also flows through the shared predicate into `ir/driver.rs`'s
`combinator_indices` and the REPL, the M-A direct-construction test is the
authoritative discriminator; do not substitute an end-to-end build for it.

## Exit criteria

Each criterion maps to a named test or golden in `tests/phase4_slice12_*.rs` (unit
tests sit beside their stage per house convention).

### P1 (parts A + B + library migration)

- **X1.** `is_combinator` is `WordBody::Terms && declares_inline`, asserted both
  ways on directly-constructed `WordDef`s (`combinators.rs` unit test; = M-A).
- **X2.** The nine R-B3 library words declare `inline`, asserted on
  `is_combinator` over the words as `lib/` spells them, and a program calling all
  nine still builds and runs (`tests/phase4_slice12_partab.rs`). Not asserted via
  `nm`: all nine are polymorphic, so `ir/driver.rs`'s `poly_indices` already
  excludes them from the symbol-minting env whether or not they are combinators,
  which makes a symbol-table witness a placebo *for these nine*. The end-to-end
  no-symbol witness belongs to the one shape where minting tracks
  combinator-ness, a monomorphic `inline` word
  (`phase4_slice11_inline.rs::inline_word_mints_no_symbol`).
- **X3.** A `~[ ... ]` parameter without `inline` is the E2 located-error golden
  (= M-B).
- **X4.** R-B4: `arrays.sth`'s `bin_search`/`sort` retype to `~[ ... ]` + `inline`;
  `sort` still runs (the `slice6g:386` golden, still passing because in P1 an
  ordinary `[ ... ]` literal silently satisfies a `~[ ... ]` parameter).
- **X5.** The 35 golden `.sth` files' output is byte-identical to the P1 base
  (`tests/corpus_stdout` comparison).

### P2 (part C)

- **X6.** `~[ ... ]` parses as a term-level literal (positive golden;
  `parser.rs` unit test for the new `parse_term` arm).
- **X7.** An ordinary `[ ... ]` at a `~` parameter (E3a) and a `~[ ... ]` at an
  ordinary parameter (E3b) are each located-error goldens (= M-C). E3b gets a
  golden at each of R-C2's other two boundaries as well — a word output and an
  array store — since all three ride one funnel and only a test says so.
- **X8.** The migrated corpus (R-C3, including the `slice6g:386` `sort` call site
  now `~[ ... ]`) stays byte-identical.
- **X9.** `examples/capturing_dispatch.sth`'s stored/returned ordinary quotations
  are unmigrated and still run (R-C4).

### P3 (part D + OQ4 REPL)

- **X10.** `: apply ( [ i64 -- i64 ] i64 -- i64 ) | n | | f | n f call ;` with
  `: main ( -- ) [ 1 + ] 5 apply . ;` lowers to a real `Instr::Call` (asserted on
  lowered IR, not inferred from output), runs, prints `6`; `nm` shows an `apply`
  symbol (= M-D).
- **X11.** A quotation passed down two real-call levels, and a word that calls a
  quotation and returns, both run (corpus additions proving R-D6 beyond one level).
- **X12.** OQ4 REPL behaviour has a test: the ordinary-`[ ... ]`-parameter shape
  in the REPL fires the E4 located error, never a silent splice/real-call
  divergence (the retention gate is at `is_combinator`, R-D5).
- **X13.** `Arity`'s new field is populated by `ir_arity_env` (R-D2), exercised so
  the REPL half of part D is not silently unlowered.

P1 note for this phase: to stay green under R-B1, P1 added `inline` to
`examples/array_ctor.sth`'s `withbuf` and to ~20 test-source words with ordinary
`[ ... ]` parameters, including `src/ir/func_builder/calls.rs`'s `apply` doc
fixture, the exact source X10 names as its witness. So on entry to P3 the
codebase has **no** coverage of the ordinary-`[ ... ]`-parameter real call: part D
must reintroduce the non-inline shape at those sites, not assume one survives.

## Out of scope

Echoing the brief's Out of scope and OQ4/OQ5 boundaries:

- The row-combinator quotation ICE's actual fix (brief decision 6, OQ5). Part C
  gives the shape a second spelling reaching the same unguarded hole; the guard is
  a separate slice with its own witnesses (`[[project_row_combinator_quotation_ice]]`).
- Real REPL support for a quotation-taking word that lowers to a real call (OQ4).
  The `(code, env)` ABI across a `dlopen` boundary is an untested surface for its
  own slice; the REPL emits E4 for the shape instead.
- First-class runtime quotations and closures generally (7b). This slice makes an
  ordinary `[ ... ]` *parameter* real-callable (part D) but adds no capture,
  storage, or dispatch beyond 7b; `examples/capturing_dispatch.sth` keeps working
  unchanged.
- `cond`, the variadic multi-way branch: never built, not fixed-arity, cannot gain
  an `inline` declaration.
- `lib/binary_search.sth` and `lib/uart_mmio.sth` (both untracked): the first is a
  sketch over non-existent grammar; the second needs `static:`/`volatile`/`at`
  (deferred). Neither builds today, so neither is in the migration corpus, though
  `uart_mmio.sth`'s `spin ( ~[ -- bool ] -- )` previews a part-B migration and its
  hand-written `~[ ... ]` body literal previews part C.

## Growth structure

Edits land in existing stage files: `src/check/combinators.rs` (R-A1),
`src/check/word_entry.rs` (R-B1/R-B2), `src/parser.rs` (R-C1/R-C2),
`src/ir/types.rs` + `src/ir/driver.rs` + `src/ir/func_builder/calls.rs` (part D),
`src/repl.rs` (R-D2/R-D5), and `lib/` + `examples/` + `tests/` (the corpus). Re-run
the CLAUDE.md split signals at phase exit against any file part D grows
(`calls.rs`, `driver.rs`); no split is anticipated — `is_combinator` stays the
single shared predicate and `Arity` stays a single owned type.

## Phased delivery plan

Sequenced so each phase is independently green and runnable. Part D (the codegen
risk) is isolated in its own phase and not entangled with the mechanical migration.

- **Phase 1 — parts A + B + the `inline` library migration.** Retire the
  `is_combinator` inference leg (R-A1..R-A4), add the E2 missing-`inline` rejection
  (R-B1..R-B2), and migrate the nine library `~[ ... ]` words plus the two
  `arrays.sth` comparator words to `inline ~[ ... ]` (R-B3..R-B5). All eleven stay
  splices, so output is byte-identical. This is where the part-A placebo hazard
  lives (M-A). Standalone value: recognition is declared, not inferred.
- **Phase 2 — part C.** Add the `parse_term` arm (R-C1), the required-tilde flavour
  check (R-C2, both directions), and the corpus-wide call-site migration
  (R-C3..R-C5) including the `slice6g:386` `sort` site. Byte-identical corpus
  output; `capturing_dispatch.sth` unmigrated. This is where the part-C placebo
  hazard lives (M-C). Standalone value: the tilde is writable and enforced.
- **Phase 3 — part D + the REPL reconciliation.** Extend `Arity` (R-D1),
  populate it in both env builders (R-D2), materialize the phantom argument at the
  ordinary dispatch (R-D3..R-D4), move the REPL retention gates to `is_combinator`
  with the E4 boundary error (R-D5), and land the witnesses (R-D6). This carries the
  codegen risk (M-D) and delivers the capability part A makes reachable; it must not
  be deferred out of the slice.

```json
{
  "phases": [
    { "phase": 1, "focus": "Retire the is_combinator inference leg, require inline on ~[ ... ] parameters with a located error, and migrate the library plus arrays.sth comparators to inline ~[ ... ] keeping output byte-identical", "difficulty": "standard" },
    { "phase": 2, "focus": "Add the parse_term ~[ ... ] literal arm and the required-tilde flavour check both directions, and migrate every combinator call site in lib, examples, and tests", "difficulty": "standard" },
    { "phase": 3, "focus": "Lower the ordinary [ ... ] real call: extend Arity with the quotation-slot shape populated in both env builders, materialize the phantom argument at dispatch, reconcile the REPL retention gates to is_combinator with a located not-supported error, and prove the apply witnesses", "difficulty": "hard" }
  ]
}
```
