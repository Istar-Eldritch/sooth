## Part A — recognition is declared

- **R-A1.** `is_combinator` (`src/check/combinators.rs:167`) is
  `matches!(word.body, WordBody::Terms { .. }) && word.declares_inline`.
- **R-A2.** `word_declares_quotation_parameter` (`:179`) and `poly_input_is_quotation`
  survive: ~20 "is this slot a quotation?" callers across
  `check/{poly,terms,audits,captures}.rs` plus the REPL's own boundary. Only the
  "does this word splice?" use was retired.
- **R-A3.** All consumers (`collect_combinators`, `combinator_index`, `check.rs`,
  `ir/driver.rs`'s `combinator_indices`, the REPL via `combinator_of`) inherit the
  meaning with no extra plumbing.

## Part B — `inline` required on a `~[ ... ]` parameter

- **R-B1/R-B2.** `check_inline_quotation_requires_inline`
  (`src/check/word_entry.rs:118`) rejects a word naming a `Type::InlineQuotation`
  input without `inline` (**E2**, located at the word-name span). It sits beside
  slice 11's `check_inline_declaration` and runs on both the batch path and the REPL
  definition path (`repl.rs:2587`). Phrased over `Type::InlineQuotation`, not
  `word_declares_quotation_parameter`: a `~` parameter is unrepresentable at runtime
  so it can only be spliced; an ordinary `[ ... ]` parameter is representable, so it
  is a real call by default and needs no `inline`.
- **R-B3.** Migrated to `inline`: `lib/combinators.sth` `times-helper`, `times`,
  `each`, `map`, `fold`, `filter`, `while`; `lib/core.sth` `if`, `unless`.
- **R-B4.** `lib/arrays.sth` `bin_search` (`:29`) and `sort` (`:70`) retyped their
  comparator to `~[ 'T 'T -- i64 ]` and gained `inline`, keeping them splices across
  part A rather than routing the corpus's hardest word through the newest path.
- **R-B5.** All eleven are splices before and after, so corpus output stayed
  byte-identical for them.

`branch` is a hardcoded primitive with no signature flavour to read, so its arms stay
ordinary `[ ... ]`: `lib/core.sth`'s six comparison words keep
`[ true ] [ false ] branch` unchanged, and `if`/`unless` forward their `~` locals to
`branch` (a forwarded parameter, not a literal).

## Part C — the tilde is writable and required

- **R-C1.** `parse_term`'s `Token::TildeLBracket` arm (`src/parser.rs:2383`) mints the
  same `TermKind::Quotation` shape as the ordinary arm with the literal's spelling
  recorded as an `is_inline` flag (`src/ast.rs:1206`).
- **R-C2.** Flavour is enforced in one funnel,
  `check_literal_against_declared_effect` (`src/check.rs:1400`), which every
  argument-matching site and every ordinary `Type::Quotation` materialization boundary
  (parameter, field/array store, word output) reaches:
  - ordinary `[ ... ]` at a `~[ ... ]` parameter → **E3a**, located at the literal;
  - `~[ ... ]` at an ordinary `Type::Quotation` boundary → **E3b**, located at the literal.
- **R-C3.** Every combinator call site in `lib/`, `examples/`, and
  `tests/phase4_slice6g.rs:386` (`a::sort`) carries the tilde.
- **R-C4.** `examples/capturing_dispatch.sth` stays unmigrated and runs; E3b is what
  makes the ordinary flavour a real distinction rather than an available one.
- **R-C5.** No positional reject of a `~[ ... ]` literal. A literal left on the stack
  is caught by the existing `is_quotation_type`-keyed residue rejection, which covers
  both flavours. The row-carried quotation ICE keeps its second spelling and stays out
  of scope.

## Part D — the ordinary `[ ... ]` real call lowers

- **R-D1.** `Arity` (`src/ir/types.rs:406`) is a named struct with
  `quot_inputs: Vec<(usize, IrType)>` naming the callee's ordinary quotation slots and
  each slot's quotation `IrType`. A call site holds only the name-keyed env, so the
  shape travels here rather than being re-read from a `WordDef` (which lowering never
  has, and the REPL has no module to read one from). `quot_input_slots` derives it; a
  `~[ ... ]` slot never appears, since such a word is a combinator absent from every
  lowering env.
- **R-D2.** Populated in the batch env builder (`src/ir/driver.rs`) and in the REPL's
  `ir_arity_env` (`src/repl.rs:142`, from the checker `Overload`'s `sig.inputs`), the
  latter guarded by its own unit test so the REPL half cannot go silently unlowered.
- **R-D3.** At the ordinary user-word dispatch (`src/ir/func_builder/calls.rs:666`)
  **and at the self-tail-call back edge** (`:650`), `materialize_quot_args`
  (`quotation.rs:31`) runs `materialize_if_phantom` at each marked slot, turning the
  phantom `I64` into the real `(code, env)` aggregate before it enters `Instr::Call`
  or the loop-header blit.
- **R-D4.** No closure capture, storage, or dispatch beyond 7b; the `(code, env)`
  layout and `IrType::Quotation` handling are the existing ones.
- **R-D5.** The REPL's retention gates are `is_combinator` (`repl.rs:2601`, and the
  import sites `:1916`, `:1967`, `:2046`), so REPL retention cannot diverge from batch
  recognition. The ordinary-`[ ... ]`-parameter shape is refused with **E4** at both
  the definition path (`:2610`) and the import path (`:1690`); it is never routed to
  the untested cross-`dlopen` `(code, env)` surface.
- **R-D6.** Witnesses in `examples/quotation_argument.sth`: `apply` (real call, prints
  `6`, mints an `apply` symbol), `apply2` (a quotation forwarded down a second level),
  `run` (calls a quotation and returns).

## Located errors

- **E2** — `word `{name}` declares an inline-quotation parameter `{param}` but is not
  `inline`; a `~[ ... ]` quotation can only be spliced, so the word must declare
  `inline``, at the definition.
- **E3a** — `this argument is an ordinary `[ ... ]` quotation but `{word}` declares
  parameter `{param}` as inline `~[ ... ]`; write it `~[ ... ]``, at the literal.
- **E3b** — `this quotation is inline `~[ ... ]` but `{word}` expects `{param}`, an
  ordinary `[ ... ]`; write it `[ ... ]``, at the literal.
- **E4** — `word `{name}` takes a `[ ... ]` quotation parameter and lowers to a real
  call, which is not supported in the REPL`, at the definition; the import-path
  variant also names the library file, since the span is not in the session's text.

## Deltas from the pre-implementation spec

- **`call` is not a flavour boundary.** The spec listed a direct `[ ... ] call` among
  E3b's ordinary boundaries; the implementation accepts both spellings there
  (`src/check/terms.rs:312`), because nothing is materialized at a `call` — a literal
  is spliced under either spelling, so the flavour decides nothing. Covered by
  `phase4_slice12_partc.rs::both_quotation_flavours_are_accepted_at_a_direct_call`.
- **E3b's wording** is `... but `{word}` expects `{param}`, an ordinary `[ ... ]``,
  not the spec's `... declares parameter `{param}` as an ordinary `[ ... ]``. Both E3a
  and E3b carry the enclosing-word context and a line-only location.
- **Two sites part D did not anticipate**: the self-tail-call back edge needed the
  same materialization (a phantom as blit source is rejected by QBE), and `eval_import`
  needed its own E4 gate (an imported real-call word otherwise died later in `ld` on a
  non-PIC `__quot0` relocation rather than at the boundary).
- **`examples/quotation_argument.sth`** is a new corpus/QBE-baseline entry;
  `array_ctor`, `capturing_dispatch`, and `capturing_dispatch_hand` baselines moved.

## Guards that discriminate

- Part A's `is_combinator` is asserted both ways on directly-constructed `WordDef`s
  (`combinators.rs` unit tests). An end-to-end "it still builds" test passes either
  way. The nine migrated library words are asserted at the predicate, not via `nm`:
  all nine are polymorphic and `poly_indices` already excludes a polymorphic word from
  symbol minting, so a symbol witness cannot discriminate them; the no-symbol
  end-to-end witness lives on a monomorphic `inline` word.
- E2, E3a, E3b, and both E4s assert exact text, not "rejected".
- The `apply` witness asserts the `Instr::Call` argument is the materialized
  `(code, env)` on lowered IR, not just exit 0.

## Out of scope

- The row-carried quotation ICE's fix (`[[project_row_combinator_quotation_ice]]`).
- Real REPL support for a quotation-taking word that lowers to a real call: the
  `(code, env)` ABI across a `dlopen` boundary
  (`[[project_repl_materialized_quotation_link_failure]]`). E4 is the boundary.
- First-class runtime quotations and closures generally (7b).
- `cond`, the variadic multi-way branch: not fixed-arity, cannot gain `inline`.
