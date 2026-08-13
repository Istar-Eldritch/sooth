# Phase 4 Slice 11: `inline` as a declared word property (brief)

Sooth already splices some words instead of calling them: a quotation-taking word
(a combinator) mints no `IrFunc` and has its body substituted at every call site
(`is_combinator`, `src/check/combinators.rs:66`, the single predicate `check` and
`ir::lower` share). That property is currently *inferred* — a word gets it by
happening to declare a quotation parameter — and is unavailable to any other word.

This slice makes it **declared**. `: ClkDiv inline ( -- u32 u32 ) 8 4 ;` is a word
the compiler must splice at every call site, checked at the definition, with no
silent fallback to a real call. The motivation is not micro-optimisation: it is that
on the embedded/RT target a reader must be able to tell from the source whether a
call site costs a call, rather than trusting that an optimiser recognised a shape
today and will still recognise it after an edit. The same reasoning already justifies
`~[ ... ]` (10a): a guarantee the type system states beats one that merely happens to
hold.

Two consequences fall out and are in scope because they are the same mechanism, not
adjacent features:

- **`~` stops being `times`-only.** 10a shipped the inline-only quotation type but
  its ROADMAP entry deliberately left "whether the rest of the library becomes the
  explicit combinator/closure boundary" open. Recon 4 below shows the *compiler* side
  of that is already done; what remains is a library decision.
- **An `inline` word may declare a reference output.** The rule that forbids it
  (`check_reference_free_signature`) is justified by a callee frame that an `inline`
  word does not have. Recon 5.

## Recon (measured against the built compiler, 2026-08-13, `main` at `86aee0a`

plus `0313b74`)

Two of this recon's findings began as wrong diagnoses that static reading produced and
running the compiler corrected. Both corrections are kept below, because in each case
the wrong answer is the plausible one.

1. **`is_combinator` is the whole gate, and it is one boolean away from serving this
   slice.** `matches!(word.body, WordBody::Terms { .. }) && word_declares_quotation_parameter(word)`
   (`src/check/combinators.rs:66-68`). Every consumer reads that one predicate:
   `check.rs:570`, `src/ir/driver.rs:59` and `:65` (which build `combinator_indices`
   and `combinator_bodies`, excluding such words from `IrFunc` minting), and the REPL
   (`src/repl.rs:157`, `:2490`). Widening it to `|| word.declares_inline` therefore
   reaches lowering and the REPL with no further plumbing.

2. **The splice machinery is already generic over the callee's shape.**
   `inline_combinator` (`src/check/combinators.rs:227`) validates declared inputs by
   iterating `comb.word.effect.inputs` and only branches into quotation-specific logic
   per input, via `is_quotation_type(*want)`; a word with no quotation input runs zero
   iterations of that branch. Local hygiene is already solved generically
   (`alpha_rename_locals`, keyed on a per-splice `prov.inline_uid`, so a callee's
   `| x |` cannot collide with a caller's or with an outer splice's). Cycle rejection
   (`check_combinator_cycles`, `:117`) walks the call graph by name and is not
   quotation-aware; its one relaxation (R4: a *self-tail-only* cycle is permitted,
   because the loop transform makes it finite) is likewise shape-agnostic.

3. **A monomorphic combinator's body is already checked twice, and that is the
   intended design.** Once standalone at its definition against its own declared
   effect (`check_word`, reached unconditionally for any `word.poly == None` word,
   `check.rs:614`), and once per call site against the caller's live stack
   (`inline_combinator` → `check_terms`). Nothing about that changes for an `inline`
   word.

4. **`~[ ... ]` on an ordinary, non-generic word already works. Corrected: an
   earlier reading of this recon said the parser blocked it.** `parse_slot` does
   reject `Token::TildeLBracket` outright (`src/parser.rs:1559`), and its comment says
   `~` is "only legal as a poly combinator's own declared parameter" — but
   `parse_worddef` never routes a `~`-bearing effect through `parse_slot` at all:
   `effect_has_variable` returns `true` on `Token::TildeLBracket`
   (`src/parser.rs:1224`), so any effect mentioning `~[` goes to `parse_poly_effect`
   and becomes a `PolySig` carrying no actual variables. The rejection in `parse_slot`
   is live only for the positions the error names (a field, output, referent, or
   `extern:` parameter), which should stay rejected. Verified end-to-end, including a
   capturing literal, which is the case that matters for a polling loop:

   ```
   : spin ( ~[ -- bool ] -- )
     | p | p call if else p spin end ;
   : main ( -- ) 42 | n | [ n . true ] spin ;
   => prints 42, exit 0
   ```

   So there is no compiler work in "`~` beyond `times`". What is left is a library
   decision (decision 4).

5. **`check_reference_free_signature` applies to combinators today, and its stated
   rationale does not.** It runs unconditionally from `check_word`
   (`src/check/word_entry.rs:24`), so a word that mints no `IrFunc` and is spliced at
   every call site is still rejected for declaring a `&T`/`&!T` output. Verified:

   ```
   : pick ( &!Buf [ -- ] -- &!u32 ) | b f | f call b &!Buf>n ;
   => error: a reference cannot be stored: `pick__m0` declares the output `&!u32`
        a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the
        time the caller reads it; take the reference as an input instead
   ```

   The diagnostic's own reason names a frame that a spliced word does not have: after
   `alpha_rename_locals`, a callee local *is* a caller local, living exactly as long
   as any local the caller declared itself. The rule remains correct and unchanged at
   every real function boundary, because it is applied per `WordDef` and only a
   non-spliced word has a frame that ends.

6. **A `dup`-shaped self-tail combinator used to ICE, which would have made this
   slice's own exit witnesses unwritable. Fixed in `0313b74`, ahead of this brief.**
   `is_aggregate` answers `true` for `IrType::Quotation`
   (`src/ir/func_builder/mod.rs:46`) — right for a materialized `(code, env)` closure,
   wrong for a phantom that owns no bytes. A self-tail combinator hoists its quotation
   out of the carried row, but that hoist was recognised only when the body *named*
   the parameter with a leading `| p |`; `dup call if ... else self end` left the
   phantom in the row, where `begin_loop` staged it as an aggregate and blitted from
   nothing, so the first `call` reached `lower_indirect_call` with a pointer and hit
   its `unreachable!`. **Corrected: the first diagnosis of this blamed
   `materialize_join_quotations`' use of raw `Value` identity at a branch join, which
   is not on the crash path at all** — a backtrace, not more reading, is what
   distinguished them. The fix hoists any contiguous top run of phantoms; a quotation
   parameter positioned *below* a non-quotation input would still reach the row
   (out of scope, decision 6).

7. **Nothing in the corpus is named `inline`.** No `.sth` file under `lib/` or
   `examples/` defines or calls such a word, so the keyword costs no migration. It
   need not become globally reserved either: the grammar position is fixed (between a
   word's name and its `(`), where nothing else can appear.

## Decisions (settled here, not reopened by the spec)

1. **`inline` is a property of the definition, not of a call site.** Spelled between
   the name and the effect: `: ClkDiv inline ( -- u32 u32 ) 8 4 ;`. Parsed at
   `parse_worddef` (`src/parser.rs:975`) between `expect_word_any_spanned` and
   `expect(Token::LParen)`, recorded as a new `WordDef` field. A per-call-site sigil
   is rejected: the point is that reading the declaration answers the question for
   every call site at once, which a per-site marker cannot do.

2. **The guarantee is unconditional, and unmeetable shapes are rejected at the
   definition.** Every call to an `inline` word is a splice; no path emits a real
   call. Where that is impossible the definition is a located error, never a silent
   fallback — a fallback would reintroduce exactly the "did the optimiser take it
   today?" question the slice exists to remove. Concretely, `inline` inherits
   `check_combinator_cycles` verbatim, including its R4 self-tail relaxation, so
   mutual or non-tail self recursion is the existing "the inliner would splice it
   forever" rejection and a self-tail-only cycle stays legal as a loop.

3. **`inline` requires a term body and a monomorphic effect in this slice.** A
   clause-bodied word is already outside `is_combinator` (`WordBody::Terms`); combining
   `inline` with a variable-bearing signature (`'T`, `'N`, `..s`) is a located error,
   deferred rather than designed here. A `~`-bearing effect is *not* a poly signature
   for this purpose even though `effect_has_variable` routes it through the poly parser
   (recon 4), so the spec must phrase this rule over declared variables, not over
   `word.poly.is_some()`.

4. **`~` generalises by library migration, not by compiler change.** Since recon 4
   shows the mechanism is already available, this slice retypes the quotation
   parameters of the hand-written combinators in `lib/combinators.sth` to `~[ ... ]`
   and keeps ordinary `[ ... ]` for genuinely first-class capturing quotations (7b's
   territory), which is the boundary 10a's ROADMAP entry left open. `times` is
   excluded: 10b is moving it to the library concurrently and the two must not race on
   the same file.

5. **`check_reference_free_signature` is skipped exactly when `is_combinator(word)`
   is true.** One condition at `src/check/word_entry.rs:24`, phrased over the shared
   predicate rather than over the new `inline` flag alone, so the exemption covers
   every always-spliced word uniformly (an `inline` word, a mono combinator, a poly
   one) and rests on a single invariant: a word that mints no `IrFunc` has no frame
   whose end a reference could outlive. This widens what is *accepted*; no existing
   corpus word declares a reference output, so corpus output stays byte-identical.

6. **The residual hoist gap from `0313b74` stays open.** Only a contiguous top run of
   quotation phantoms is hoisted out of a self-tail loop's carried row. Widening that
   to an interleaved quotation parameter also means changing `lower_call`'s back-edge
   `drop_n` contract, which assumes invariant args sit on top. No corpus combinator has
   that shape.

## Open questions for the spec

- **Whether decision 5's exemption is actually sound under adversarial shapes, and
  what witnesses prove it.** The frame argument is clean for a reference derived from
  the caller's own operand (`&!Buf>n` on an input). It needs explicit treatment for: a
  reference to a *callee-declared* local, which post-splice is a caller-frame local
  that outlives the splice and must still be answerable to the ordinary must-consume
  rule; a **linear** (non-`Copy`) such local, whose drop obligation now belongs to the
  caller; transitive inlining, where an `inline` word returning a reference is called
  by another `inline` word; and the outermost boundary, where the existing rule must
  still fire for the real word that tries to make such a reference *its* output. If
  any of these does not hold, decision 5 narrows rather than the slice growing a
  special case.
- **Whether an `inline` word's standalone definition-site check (recon 3) stays
  meaningful when its effect declares a reference output.** That check runs against
  the declared effect with no caller context, which is precisely the context the
  frame argument needs. The spec should say what it verifies for such a word rather
  than let it fall out.
- **Diagnostic wording and numbering** for the three new rejections: `inline` on a
  clause body, `inline` on a variable-bearing signature (decision 3), and `inline` in
  a splice cycle (decision 2, if its inherited message needs rewording now that the
  member is not necessarily quotation-taking — its current text says "a quotation-taking
  word cannot be recursive").
- **Whether `inline` should appear in the REPL's `defined`/signature rendering**, given
  a retained combinator already mints no `.so` and no symbol (`eval_combinator_def`,
  `src/repl.rs:2468`) and an `inline` word joins that path.

## Out of scope

- Any *call-site* inlining request, and any heuristic or automatic folding of a word
  the author did not mark. Decision 1.
- Interleaved quotation parameters in a self-tail combinator. Decision 6.
- `times` itself, and `lib/combinators.sth`'s ownership of it: 10b.
- `if`/`cond` as ordinary words: 10c.
- Statics, `volatile`, and fixed-address MMIO. The driver sketch that motivated this
  slice (`lib/uart_mmio.sth`) needs those too, but they are a separate, larger piece of
  language design (DESIGN.md's "Embedded: statics, MMIO, and interrupts") and nothing
  here depends on them.

## Sequencing

Depends on 10a (shipped, `e87bcae`) for `~`, and on `0313b74` for a `dup`-shaped
self-tail combinator that does not ICE. Touches `check/combinators.rs`'s predicate,
`check/word_entry.rs`'s signature gate, `parser.rs`'s word-definition grammar, and
`lib/combinators.sth`.

10b is in flight against `lib/combinators.sth` and `src/check/terms.rs`, and 10c will
touch the dispatch spine. Decision 4 already keeps this slice off `times`, but the
library edits still collide, so **this slice starts after 10b merges**. Nothing here
depends on 10b's outcome, only on not editing the same file concurrently.

## Exit

- `: ClkDiv inline ( -- u32 u32 ) 8 4 ;` compiles, and `nm` shows no symbol for it
  (the `is_combinator`/no-`IrFunc` contract, checked the way
  `quotation_taking_word_mints_no_symbol` already checks its own).
- A caller of an `inline` word emits no `Instr::Call` for it, asserted on lowered IR
  rather than inferred from output.
- The three decision-2/3 rejections are located-error goldens.
- An `inline` word declaring `&!u32` as an output compiles and its caller reads
  through the returned reference with the right value; the same word *without*
  `inline` still fails with the existing `check_reference_free_signature` message
  (the pair is the witness that decision 5 is scoped to the splice, not a blanket
  relaxation).
- Whichever adversarial shape from open question 1 survives review has a golden of its
  own, positive or negative.
- `lib/combinators.sth`'s retyped parameters (decision 4) leave corpus output
  byte-identical, and a stored/returned quotation still requires ordinary `[ ... ]`.
- Every new test is mutation-tested: reverting the change it guards must fail it.
