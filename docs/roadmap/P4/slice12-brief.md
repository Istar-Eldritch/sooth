# Phase 4 Slice 12: combinator recognition becomes declared (brief)

A word that mints no `IrFunc` and is spliced at every call site (a combinator) is
recognised today by **two** routes, joined in one predicate: it declares a quotation
parameter, *or* it declares `inline` (`is_combinator`, `src/check/combinators.rs:170-173`).
The first route is an **inference** from the shape of a signature; slice 11 added the
second, and slice 10c made it available to a polymorphic word. This slice retires the
inference, so the declaration is the single route: a word declaring a `~[ ... ]` parameter
must say so with `inline` (a located error where it does not), `word_declares_quotation_parameter`'s
leg in `is_combinator` is deleted, and every library combinator is migrated.

The motivation is the embedded/RT reading argument slice 11 was built on: a reader must
see at the definition whether a call site costs a call, rather than deriving it from `~`'s
non-representability. The second payoff is a capability. `is_quotation_type` matches
`Type::Quotation` and `Type::InlineQuotation` alike (`src/ast.rs:966-971`), so the
inference *also* splices a word taking an ordinary runtime `[ ... ]`, which means no word
taking a first-class capturing quotation (7b's territory) can be a genuine call today.
Retiring the inference makes that shape expressible — and, because the real-call argument
path has never been reached, this slice is not done until that path lowers and a witness
runs.

Four parts, in the ROADMAP's own lettering: **A** retire the inference leg; **B** require
`inline` on a `~[ ... ]`-declaring word and migrate the library; **C** add the `parse_term`
arm so `~[ ... ]` is writable as a literal and *require* the tilde at a `~` parameter (a
corpus-wide call-site migration); **D** make the ordinary-`[ ... ]` real-call path lower.

## Recon (measured against the built compiler, 2026-08-15, `main` at `0eb7c84`)

`cargo test` is green at this HEAD (every suite passes, 0 failed). Every claim below was
produced by opening the file cited or by running the compiler; the ROADMAP item 12 text
predates slice 10c's merge and several of its line anchors are stale, re-anchored here.
Two of its claims are **incomplete** rather than wrong (recon 3, 7): the migration surface
it lists omits `lib/arrays.sth` and the REPL, and both are load-bearing.

1. **`is_combinator` is one boolean disjunct, and the predicate it shares is used two
   different ways.** `matches!(word.body, WordBody::Terms { .. }) && (word_declares_quotation_parameter(word) || word.declares_inline)`
   (`src/check/combinators.rs:170-173`; the ROADMAP's `:74` is stale). Part A deletes the
   first disjunct, leaving `... && word.declares_inline`. The predicate
   `word_declares_quotation_parameter` (`:181`) itself **stays**: it folds over
   `word.effect.inputs` / `sig.inputs` asking `is_quotation_type(...).is_some()`
   (`:186`, via `poly_input_is_quotation`, `:195`), and `is_quotation_type`
   (`src/ast.rs:966-971`) returns `Some` for **both** `Type::Quotation` and
   `Type::InlineQuotation`. That "is this slot a quotation?" question has ~20
   parameter-level callers across `check/{poly,terms,audits,captures}.rs`; only the one
   "does this word splice?" use in `is_combinator` is being retired. Deleting the function
   would break the former; deleting only its leg in `is_combinator` is the change.

2. **Polymorphic `inline` works TODAY. Proven by a witness, not read.** `check_inline_declaration`
   (`src/check/word_entry.rs:69`) has exactly three gates — `main` (`:78`), a clause body
   (`:84`), and a builtin-operator name (`:99`) — and **no** monomorphic-signature gate;
   its own doc comment (`:63-66`) records that slice 10c lifted the poly rejection because
   the six comparison words needed a poly `inline` word as their first consumer. Built and
   ran:

   ```text
   : apply-twice inline ( 'T ~[ 'T -- 'T ] -- 'T ) | f | f call f call ;
   : main ( -- ) 3 [ 1 + ] apply-twice . ;
   => prints 5, exit 0
   ```

   Note the body still uses an *ordinary* `[ 1 + ]` literal, which the `~[ ... ]` parameter
   accepts silently (recon 4). So the capability question that would have been this slice's
   main risk is closed: parts A/B/C are a checker-gate-plus-migration, not new lowering.

3. **The library words that take `~[ ... ]` and declare no `inline` today — plus two the
   ROADMAP omits.** Confirmed by reading, not assumed:
   - `lib/combinators.sth`: `times-helper` (`:30`), `times` (`:37`), `each` (`:40`),
     `map` (`:45`), `fold` (`:50`), `filter` (`:63`), `while` (`:76`) — seven `~[ ... ]`
     words, none `inline`.
   - `lib/core.sth`: `if` (`:42`), `unless` (`:48`) — two `~[ ... ]` words, none `inline`.
     (The six comparison words `=`/`<`/`>`/`<=`/`>=`/`<>`, `:17-22`, are already `inline`
     and take **no** quotation parameter; they are untouched by part B.)

   **Correction to the ROADMAP: `lib/arrays.sth`'s `bin_search` (`:29`) and `sort` (`:70`)
   declare an *ordinary* `[ 'T 'T -- i64 ]` comparator, not `~[ ... ]`, and are combinators
   only by the inference part A retires.** They compile and lower today (arrays.sth builds
   through the compiler; the only error is a missing `main` at link). `sort` is exercised
   with an ordinary comparator **literal** in a golden — `d s [ | x y | x y - ] a::sort`,
   `tests/phase4_slice6g.rs:386`. When part A narrows `is_combinator`, these two words are
   neither `~`-parametered nor `inline`, so they cease to be combinators. That forces a
   decision the ROADMAP never raises (open question OQ2): retype the comparator to
   `~[ ... ]` (they stay spliced inline combinators), or leave it ordinary (they become the
   hardest possible part-D case — a *polymorphic* real-call word whose body splices `times`
   and uses rows, calling the comparator through a real indirect `call`).

4. **`~[ ... ]` is unwritable in a body, and an ordinary `[ ... ]` silently satisfies a
   `~` parameter. Both halves confirmed.** `Token::TildeLBracket` is consumed only by the
   signature/type parsers (`effect_has_variable` `:1313`; `parse_poly_slot` `:1397`;
   `parse_slot` `:1690`, `:1743`; `parse_quotation_type_expr`-adjacent `:2031`). `parse_term`
   (`src/parser.rs:2301`) handles `Token::LBracket` (`:2367`, an ordinary `TermKind::Quotation`)
   and has no `TildeLBracket` arm, so a `~[` in a body falls to the generic `other =>` arm:

   ```text
   : x ( 'T ~[ 'T -- 'T ] -- 'T ) | f | f call ;
   : main ( -- ) 3 ~[ 1 + ] x . ;
   => error: parse error: unexpected token TildeLBracket at line 2, col 17
   ```

   The silent-satisfaction half is recon 2's witness itself: `[ 1 + ]` satisfied a
   `~[ 'T -- 'T ]` parameter and ran. Part C adds the `parse_term` arm and makes the tilde
   required at a `~` parameter.

5. **The real-call gap is real, and its three pieces are exactly as the ROADMAP states.**
   (a) A quotation literal lowers to a **phantom** `Value` typed `IrType::I64` with no
   `Instr` (`src/ir/func_builder/calls.rs:76-83`, `TermKind::Quotation` arm). (b)
   `materialize_if_phantom` (`src/ir/func_builder/quotation.rs:15`) turns that phantom into
   the real `(code, env)` aggregate, and its only callers are the `&!` store
   (`word_families.rs:139`), the field store (`word_families.rs:710`), and the word-output
   boundary (`mod.rs:766`) — **not** the ordinary user-word dispatch, which pops
   `in_arity` values with `split_off` and pushes them straight into `Instr::Call(ret, sym, args)`
   with no materialization (`calls.rs:649-680`). (c) `env` is a `HashMap<String, Arity>`
   where `Arity = (usize, usize, Option<IrType>)` (`src/ir/types.rs:406`) — `(in_arity,
   out_arity, ret_ty)` — so a call site cannot tell which argument slot is a quotation.
   The **checker** half already works: `check_terms` materializes a `Type::Quotation`
   argument at an ordinary user-word boundary (`src/check/terms.rs:698-705`,
   `materialize_quotation_at_boundary` with `escaping = false`), gated on `Type::Quotation`
   — currently dead for user words because every quotation-taking word is a combinator, and
   made live by part A.

   Today the inference hides all of this: `apply` splices and runs.

   ```text
   : apply ( [ i64 -- i64 ] i64 -- i64 ) | n | | f | n f call ;
   : main ( -- ) [ 1 + ] 5 apply . ;
   => prints 6, exit 0;  nm shows no `apply` symbol (spliced)
   ```

   With part A this word declares neither `~` nor `inline`, so it becomes the part-D
   witness: a real `Instr::Call` whose caller must materialize the phantom argument and
   whose lowering must know the callee's parameter types.

6. **`=`/`<`/`>`/`<=`/`>=`/`<>` left `BUILTIN_TABLE` in 10c.** Only the `u`-prefixed
   primitives remain (`src/check/builtins.rs:132-137`), and a test pins the surface names'
   *absence* (`:507-514`, "`{op}` left `BUILTIN_TABLE` for `lib/`"). So the
   `inline`-vs-builtin-name collision `check_inline_declaration` guards (`word_entry.rs:99`)
   has one fewer live family; the remaining builtin names (`+`, `.`, `mod`, `shr`, …) still
   cannot carry a `~`-bearing overload (such an effect routes through the poly parser, and
   a builtin name's rows are concrete by construction), so a test suffices there, not a
   design question.

7. **The REPL gates combinator retention on `word_declares_quotation_parameter` directly,
   at four sites, and part A makes that diverge from `is_combinator`. The ROADMAP omits
   this.** `src/repl.rs:2565` (the definition path, `eval_def`) routes a word to
   `eval_combinator_def` iff `word.declares_inline || word_declares_quotation_parameter(&word)`
   — which for a `WordBody::Terms` word *is* today's `is_combinator`. Three import sites
   (`:1886`, `:1937`, `:2014`) use `word_declares_quotation_parameter` alone. After part A,
   a user-defined word declaring an ordinary `[ ... ]` parameter (the `apply` shape) still
   satisfies `word_declares_quotation_parameter`, so the REPL retains and **splices** it,
   while the batch compiler lowers it as a **real call** — a check/lowering divergence of
   exactly the kind `combinators.rs`'s own comments warn against. The predicate's doc
   comment (`combinators.rs:174-180`) records *why* the REPL uses the coarser gate: it
   "cannot retain any quotation-taking word's body past the defining line". Part D changes
   that premise (a quotation becomes a real ABI value), so the four sites are reconcilable,
   but the reconciliation is scope the ROADMAP does not budget (OQ4).

8. **The row-combinator quotation ICE is still live at HEAD, and its surface has narrowed
   since it was last recorded.** A plain-`[ ... ]` parameter carrying a **row** is now
   rejected at parse — `: apply-row ( ..s [ ..s -- ..s ] -- ..s ) ...` gives *"a quotation
   effect with a row (`..s`) must be inline (`~[ ... ]`)"* — so a row combinator can only be
   declared `~[ ... ]`. But a quotation riding *in* such a row still crashes the backend:

   ```text
   import: c | times | "comb.sth" ;
   : main ( -- ) [ + ] 3 [ drop ] c::times drop 0 . ;
   => error: "qbe" failed: qbe: invalid type for operand %v0 in phi %v4
   ```

   The checker admits it (it rejects a quotation left *on the stack*, not one carried in a
   declared row); QBE rejects the phi over a phantom. This slice neither widens nor narrows
   it by construction (recon-derived in decision 6), but part C's new `~[ ... ]` literal
   gives the shape a *second* spelling reaching the same unguarded hole, which the spec
   should note (OQ5).

## Decisions (settled here, not reopened by the spec)

1. **Part A deletes only the inference disjunct; `word_declares_quotation_parameter`
   survives.** `is_combinator` becomes `matches!(word.body, WordBody::Terms { .. }) && word.declares_inline`.
   The predicate stays for its ~20 "what is this slot?" callers (recon 1); a spec that
   deletes the function is wrong. Every consumer that today reads `is_combinator`
   (`collect_combinators`, `combinator_index`, `check.rs`, `ir/driver.rs`, and the REPL via
   `combinator_of`) inherits the new meaning with no further plumbing — except the REPL's
   four *direct* `word_declares_quotation_parameter` sites, which decision below and OQ4
   address.

2. **Part B: a `~[ ... ]` parameter without `inline` is a located error at the definition,
   not a silent real call.** The rule is phrased over "declares a `Type::InlineQuotation`
   parameter" (the `~` case), not over `word_declares_quotation_parameter` (which also
   catches ordinary `[ ... ]`): a `~` parameter is unrepresentable at runtime, so it can
   *only* be spliced, so `inline` is mandatory; an ordinary `[ ... ]` parameter is
   representable, so it is a real call by default and needs no `inline`. This is the
   diagnostic that makes "declared, not inferred" enforceable rather than merely available.
   The nine library words in recon 3 gain `inline`.

3. **Part C: the tilde is required exactly at a `~[ ... ]` parameter, and nowhere else.**
   A literal satisfying a `Type::InlineQuotation` parameter must be written `~[ ... ]`; a
   literal at any *ordinary* `Type::Quotation` boundary — a `Type::Quotation` parameter, a
   store into a field/array slot, a word output, a direct `[ ... ] call` — stays ordinary.
   This is the line that keeps 7b's first-class quotations (`examples/capturing_dispatch.sth`:
   `seed ( -- [ -- i64 ] ) [ 0 ] ;`, and the `[ r i >usize &> @ ]` closures stored into a
   table) written with ordinary `[ ... ]` and *unmigrated*. The parser gains a `parse_term`
   arm minting an `InlineQuotation`-flavoured literal; the flavour check fires at the same
   argument-matching sites that already resolve the parameter type
   (`inline_combinator` / the poly-combinator arg loop / `check_terms`' argument
   materialization), comparing literal flavour against parameter flavour.

4. **Part D adds no new machinery: materialize the phantom argument, and give lowering the
   callee's parameter types.** At the ordinary user-word dispatch (`calls.rs:649-680`),
   before building `Instr::Call`, materialize each argument the callee's signature declares
   as a quotation (the checker already produced the `(code, env)` at `terms.rs:698`); to
   know *which* argument, lowering needs the callee's parameter types, which `Arity`
   (`types.rs:406`) does not currently carry. The minimal extension is per the ROADMAP: the
   call site learns which slots are quotations. The exit witness is recon 5's `apply`,
   plus a two-real-call-level pass-down and a word that calls a quotation and returns.

5. **Migration keeps corpus output byte-identical where the word stays a combinator.** The
   nine `~[ ... ]` words gain `inline` (a splice before, a splice after — no codegen
   change); every combinator call site in `lib/` and `examples/` gains the tilde at the
   argument. `if`/`unless`/`while`/`times`/`each`/`map`/`fold`/`filter` remain spliced, so
   the 35 golden `.sth` files must produce identical output. Only the words that change
   *category* (recon 3's `arrays.sth` pair under OQ2; the `apply`-shaped part-D witnesses,
   which are new) produce new codegen.

6. **This slice does not add a guard for the row-combinator ICE (recon 8).** It is
   pre-existing, orthogonal to declared-vs-inferred recognition, and guarding it means a
   general check on a quotation carried in a combinator's row — a separate piece with its
   own three witnesses (see `[[project_row_combinator_quotation_ice]]`). The spec must note
   that part C's new `~[ ... ]` literal is a second way to reach it and decide whether the
   parse-term arm rejects a `~` literal in a non-argument position early (OQ5).

## Open questions for the spec

- **OQ1 — where the callee's parameter shape lives for part D.** `Arity` is
  `(usize, usize, Option<IrType>)`. Part D needs, per callee, which input slots are
  quotations (and, if a real quotation call needs it, the quotation's own effect for the
  `(code, env)` layout). Does `Arity` grow a per-slot-quotation mask, or does lowering read
  the `WordDef`/`Sig` it already has in scope? The REPL builds its own `env`
  (`repl.rs`, `ir_arity_env`), so whichever field is added must be populated there too or
  the REPL half of part D is silently unlowered.

- **OQ2 — `lib/arrays.sth`'s `bin_search`/`sort` (recon 3).** Retype the comparator to
  `~[ 'T 'T -- i64 ]` (they stay inline combinators, the call site becomes
  `d s ~[ | x y | x y - ] a::sort`, lowest risk, matches the rest of the migration), or
  leave it ordinary and make them genuine part-D real-call words (a first-class comparator,
  but a *polymorphic* one whose body splices `times` and uses rows and calls the comparator
  indirectly — untested, and the crash-adjacent shape). The comment in `arrays.sth:1-20`
  gives no design reason for the ordinary flavour, so this is a genuine choice, not a
  recovery of intent. Whichever is chosen, `tests/phase4_slice6g.rs:386`'s `sort` call site
  migrates with it.

- **OQ3 — the flavour of `branch`'s arms.** `branch` is a hardcoded primitive
  (`check/terms.rs:264`, `check_branch` `:1259`) that accepts a quotation *literal* or a
  forwarded `~` parameter in either arm; it is not a declared `~[ ... ]` word, so it has no
  signature flavour to read. `lib/core.sth`'s comparison words pass ordinary
  `[ true ] [ false ]` to it (`:17-22`), and `if`/`unless` forward their `~` locals to it.
  Under part C's required-tilde, must a `branch` *literal* arm be `~[ ... ]` (the arms are
  always jump-and-join spliced, so `~` is the honest flavour — but then all six comparison
  words migrate their bodies), or does `branch`, sitting below the typed layer, keep
  accepting an ordinary literal? Pick one; both are defensible and the choice decides
  whether `core.sth:17-22` is touched.

- **OQ4 — the REPL's four retention sites (recon 7).** After part A, `repl.rs:2565` and
  `:1886`/`:1937`/`:2014` must switch from `word_declares_quotation_parameter` to
  `is_combinator` to avoid splicing an ordinary-`[ ... ]` word the batch compiler
  real-calls. Doing so routes such a word to the REPL's ordinary `.so`-minting path, which
  requires part D's `(code, env)` ABI to work *across a `dlopen` boundary* (the quotation
  built on a later line, its code pointer resolved through `RTLD_GLOBAL` to that line's
  module) — untested. Is REPL support for a real quotation-taking word in scope for this
  slice, or does the REPL emit a located "not supported in the REPL" error for the
  ordinary-`[ ... ]`-parameter shape (keeping the retention gate at `is_combinator` so it
  never silently diverges)?

- **OQ5 — the row-combinator ICE and the new `~[ ... ]` literal (recon 8).** Should part
  C's `parse_term` arm reject a `~` literal outside an argument position (closing one
  spelling of the ICE at parse time), or is the ICE left wholly to a future slice? A pure
  "add the arm, require it at `~` parameters" reading leaves both spellings reaching the
  crash.

- **OQ6 — diagnostic wording and numbering** for part B's new rejection (a `~[ ... ]`
  parameter without `inline`) and part C's flavour mismatch (an ordinary literal at a `~`
  parameter, and a `~` literal at an ordinary one). The part-B message should name the
  parameter and cite that a `~` quotation can only be spliced.

## Out of scope

- The row-combinator quotation ICE's actual fix (decision 6, OQ5). A guard on a quotation
  carried in a combinator's row is a separate slice with its own witnesses.
- First-class runtime quotations and closures generally: 7b. This slice makes an ordinary
  `[ ... ]` *parameter* real-callable (part D) but does not add closure capture, storage,
  or dispatch beyond what 7b already ships; `examples/capturing_dispatch.sth` must keep
  working unchanged (decision 3).
- `cond`, the variadic multi-way branch. It was never built (10c D6: nested `if`/`unless`
  covers its use) and is not a fixed-arity word, so it cannot gain an `inline` declaration
  here.
- `lib/binary_search.sth` and `lib/uart_mmio.sth` (both **untracked** in the working tree).
  The first is a design sketch over non-existent grammar (`==`, `arr[idx]`, `#arr`, a
  generic `type:` with `'T`); the second needs `static:`/`volatile`/`at` (deferred, slice
  11's out-of-scope) and already contains a hand-written `~[ ... ]` body literal previewing
  part C. Neither builds today, so neither is part of the migration corpus, though
  `uart_mmio.sth`'s `spin ( ~[ -- bool ] -- )` is a preview of exactly a part-B migration.

## Sequencing

Gates on 10c only in that `if`/`unless` (`lib/core.sth`) exist and must gain `inline` like
every other library combinator; 10c introduced no clause-body or dispatch-based combinator
mechanism for this slice to build on, and slice 11's `inline` (shipped) is the declaration
part B requires. Touches `src/check/combinators.rs` (the predicate), `src/check/word_entry.rs`
(the part-B gate), `src/parser.rs` (the `parse_term` arm and the flavour requirement),
`src/ir/func_builder/calls.rs` + `types.rs` (part D), `src/repl.rs` (OQ4), and `lib/` +
`examples/` (the corpus migration).

Natural phasing: **P1** part A + B + the library `inline` migration (recon 3, keeping
output byte-identical for the nine words that stay spliced); **P2** part C (the
`parse_term` arm, the required-tilde flavour check, and the corpus-wide call-site
migration); **P3** part D (the real-call lowering and its witnesses). P1 and P2 each stand
alone with visible value; P3 delivers the capability part A makes reachable and must not be
deferred out of the slice.

## Exit

- **P1**: the nine `~[ ... ]` library words declare `inline`, and a `~[ ... ]` parameter
  without it is a located-error golden. `nm` shows no symbol for any of them. `is_combinator`
  is `WordBody::Terms && declares_inline`, asserted directly (a word with an ordinary
  `[ ... ]` parameter and no `inline` is *not* a combinator — the mutation witness for part
  A). The 35 golden `.sth` files' output is byte-identical.
- **P2**: `~[ ... ]` parses as a term-level literal; an ordinary `[ ... ]` at a `~`
  parameter and a `~[ ... ]` at an ordinary parameter are each located errors (a positive
  and two negative goldens). The migrated corpus (`[ ... ] if` → `~[ ... ] if`, `[ ... ] times`
  → `~[ ... ] times`, `c::fold`/`c::map`/`c::each`/`c::filter`/`c::while` call sites) stays
  byte-identical. `examples/capturing_dispatch.sth`'s stored/returned ordinary quotations
  are *unmigrated* and still run.
- **P3**: `: apply ( [ i64 -- i64 ] i64 -- i64 ) | n | | f | n f call ;` with
  `: main ( -- ) [ 1 + ] 5 apply . ;` lowers to a real `Instr::Call` (asserted on IR, not
  inferred from output), runs, prints `6`; `nm` shows an `apply` symbol. A quotation passed
  down two real-call levels, and a word that calls a quotation and returns, are the corpus
  additions proving it beyond the single level.
- OQ2's `arrays.sth` decision has a golden either way (`sort` runs with its migrated
  comparator). OQ4's REPL behaviour has a test: either a real quotation-taking word works
  in the REPL, or the located "not supported" error fires — never a silent
  splice/real-call divergence.
- Every new test is mutation-tested: reverting the change it guards must fail it. This
  slice's placebo hazards are part A (a test that passes whether or not the inference leg is
  deleted) and part C (a flavour check that passes whether or not the tilde is required).

## Ready to spec?

**Yes, with the six open questions handed to the spec, none of them blocking.** The
recon falsified nothing structural in ROADMAP item 12 — poly `inline` works, the parser
and lowering gaps are exactly as described, the primitives left `BUILTIN_TABLE` — but it
found two omissions the spec must not inherit silently: `lib/arrays.sth`'s ordinary-`[ ... ]`
comparator words (OQ2) and the REPL's four direct retention sites (OQ4). Both are settleable
by the spec; neither requires a spike. OQ2 in particular should be settled toward retyping
to `~[ ... ]` unless a first-class-comparator use case is named, because leaving it ordinary
routes the corpus's hardest word (`sort`) through the least-tested part-D path.
