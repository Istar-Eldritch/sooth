# Phase 4 Slice 11: `inline` as a declared word property (spec)

## Goal

Turn the *inferred* "this word is spliced at every call site" property into a
**declared** one. Today a word is spliced iff `is_combinator` holds, and
`is_combinator` is true iff the word has a `WordBody::Terms` body and declares a
quotation parameter (`src/check/combinators.rs:66`). This slice adds a keyword,
`inline`, that makes the property declarable independently of whether the word
takes a quotation:

```sooth
: ClkDiv inline ( -- u32 u32 ) 8 4 ;
```

`ClkDiv` mints no `IrFunc`, no symbol, and no `Instr::Call` at any call site; it
is spliced. The guarantee is **unconditional**: where splicing is impossible the
definition is a located error, never a silent fall-back to a real call
(Decision 2). The motivation is the embedded/RT target, where a reader must be
able to tell from the source whether a call site costs a call, rather than
trusting an optimiser to recognise a shape and keep recognising it after an edit
(the same argument that already justifies `~[ ... ]` from slice 10a).

Two consequences fall out of the *same* mechanism and are in scope:

- **`~` stops being `times`-only** (Feature B). This needs **no compiler
  change**: recon 4 (verified below) shows `~[ ... ]` already parses and
  type-checks on an ordinary word. It reduces to a library retyping decision in
  `lib/combinators.sth`.
- **A spliced word may declare a reference output** (Feature C). The rule that
  forbids it, `check_reference_free_signature`, is justified by a callee frame
  that a spliced word does not have. This is real but was provisional in the
  brief; §"Feature C" below resolves it: it survives **without a special case**,
  because the only thing relaxed is the blanket structural rule, while every
  lifetime/linearity pass that makes the relaxation safe still runs.

## Recon (re-verified against `main` at `86aee0a` plus `0313b74`, 2026-08-13)

Each anchor below was re-read, not trusted from the brief.

1. **`is_combinator` is the whole gate.**
   `matches!(word.body, WordBody::Terms { .. }) && word_declares_quotation_parameter(word)`
   at `src/check/combinators.rs:66-68`. Its readers are `check.rs:571`
   (`if is_combinator(word)`, the poly branch), `src/ir/driver.rs:59` and `:65`
   (which build `combinator_indices` / `combinator_bodies`, excluding such words
   from `IrFunc` minting), `collect_combinators` via `src/check/combinators.rs:29`
   (the call-site splice env), and the REPL projections
   (`checker_combinators`, `src/repl.rs:160`; `combinator_of`, `:2482`).
   Widening the predicate reaches the batch checker, batch lowering, and the
   REPL's *view* construction. **Correction to the brief:** it does **not** reach
   the REPL's *retention decision* — see recon 8.

2. **The splice machinery is generic over the callee's shape.**
   `inline_combinator` (`src/check/combinators.rs:227`) validates a monomorphic
   callee by iterating `comb.word.effect.inputs`; a callee with no inputs runs
   zero iterations of the quotation branch and then splices its body
   (`src/check/combinators.rs:334` onward). Local hygiene is solved generically
   (`alpha_rename_locals`, keyed per-splice on `prov.inline_uid`). The cycle
   check `check_combinator_cycles` (`:159`) walks the call graph by name and is
   not quotation-aware; its one relaxation (R4: a self-tail-only cycle is legal,
   because the loop transform makes it finite) is shape-agnostic.

3. **A monomorphic combinator's body is checked twice, by design.** Once
   standalone at its definition against its own declared effect (`check_word`,
   reached for any `word.poly == None` word via `check.rs`'s final loop, which
   dispatches to `check_word` in the non-poly arm), and once per call site
   against the caller's live stack (`inline_combinator` → `check_terms`). An
   `inline` word inherits both, unchanged.

4. **`~[ ... ]` on an ordinary word already works.** `parse_slot` rejects
   `Token::TildeLBracket` (`src/parser.rs:1559`, `tilde_quotation_position_error`),
   but `parse_worddef` never routes a `~`-bearing effect through `parse_slot`:
   `effect_has_variable` returns `true` on `Token::TildeLBracket`
   (`src/parser.rs:1224`), so any effect mentioning `~[` takes `parse_poly_effect`.
   The `parse_slot` rejection is live only for the positions its message names (a
   field, output, referent, or `extern:` parameter), which stay rejected. So
   there is no compiler work in "`~` beyond `times`"; what remains is the library
   decision (Feature B).

5. **`check_reference_free_signature` applies to combinators today, and its
   stated reason does not.** It runs unconditionally from `check_word`
   (`src/check/word_entry.rs:24`), so a word that mints no `IrFunc` is still
   rejected for a `&T`/`&!T` output. Its own message names the fault: "a `&T`/`&!T`
   borrows a local of the callee's own frame, which is gone by the time the
   caller reads it" (`word_entry.rs:52`). After `alpha_rename_locals`, a spliced
   callee local *is* a caller local, living as long as any caller local — so the
   named frame does not exist for a spliced word.

6. **The `dup`-shaped self-tail ICE is fixed** (`0313b74`, ahead of this brief),
   so this slice's own self-tail witnesses are writable. The residual hoist gap
   (only a contiguous top run of quotation phantoms is hoisted out of a self-tail
   loop's carried row) stays open (Decision 6, out of scope).

7. **Nothing in the corpus is named `inline`.** No `.sth` under `lib/` or
   `examples/` defines or calls such a word (grep for `inline` returns only this
   slice's docs). The keyword need not be globally reserved: its grammar slot is
   fixed, between a word's name and its `(`, where nothing else can appear. The
   name slot is consumed *first* (`expect_word_any_spanned`, `src/parser.rs:977`),
   so `: inline ( -- ) ;` still defines a word named `inline`.

8. **New finding — the REPL retention gate is `word_declares_quotation_parameter`,
   not `is_combinator`.** In `eval_def` the retention route is
   `if check::word_declares_quotation_parameter(&word) { return self.eval_combinator_def(...) }`
   (`src/repl.rs:2560`, gate declared at `word_entry.rs`/`combinators.rs:78`).
   For an `inline` word that declares **no** quotation parameter (the `ClkDiv`
   case), that gate is `false`, so the REPL would fall through to the ordinary
   lowering path and mint a `.so` and a symbol — directly violating Decision 2 in
   the REPL. Recon 1's claim that widening `is_combinator` reaches the REPL "with
   no further plumbing" is therefore imprecise: the retention gate must be
   widened too (Requirement R7).

## Decisions (from the brief; settled, not reopened)

1. `inline` is a property of the definition, spelled between the name and the
   effect. No per-call-site sigil.
2. The guarantee is unconditional; unmeetable shapes are located errors at the
   definition, never a silent fall-back.
3. `inline` requires a term body and a monomorphic effect. The monomorphism rule
   is phrased over **declared variables** (`'T`/`'N`/`..s`), not over
   `word.poly.is_some()` — a `~`-bearing effect sets `poly` but carries no
   variable (recon 4).
4. `~` generalises by library migration, not compiler change. `times` is
   excluded (10b owns it concurrently).
5. `check_reference_free_signature` is skipped exactly when `is_combinator(word)`
   is true, phrased over the shared predicate so the exemption covers every
   always-spliced word uniformly.
6. The residual hoist gap from `0313b74` stays open.

## Requirements

### R1 — grammar and AST field

- Add `pub declares_inline: bool` to `WordDef` (`src/ast.rs:563`). Every
  construction site is updated (parser `src/parser.rs:1002`; test helpers and
  the two real non-parser constructors `src/check/poly.rs:164` and
  `bool_print_word_def` `src/ast.rs:358`; REPL `remap_imported_combinator`
  `src/repl.rs:363`; and every `#[cfg(test)]` `WordDef { .. }` literal). A
  synthesized or remapped word carries the source word's flag where one exists
  and `false` otherwise; `bool_print_word_def` is `false`.
- `parse_worddef` (`src/parser.rs:975`) reads an **optional** `inline` keyword
  between `expect_word_any_spanned` (name, `:977`) and `expect(Token::LParen)`
  (`:984`): peek for `Some((Token::Word(w), _))` with `w == "inline"`, consume it
  and set `declares_inline = true` if present. A second `inline` is not consumed,
  so it falls through to `expect(Token::LParen)` and fails with the existing
  located parse error — one optional keyword only.
- `inline` is **not** added to any reserved-name list (recon 7): the position is
  unambiguous because the name is consumed first.

### R2 — widen the shared predicate

`is_combinator` (`src/check/combinators.rs:66`) becomes:

```rust
matches!(word.body, WordBody::Terms { .. })
    && (word_declares_quotation_parameter(word) || word.declares_inline)
```

This is the single load-bearing read of the new field. It reaches, with no
further plumbing: `collect_combinators` (call-site splicing in the batch
checker), `driver.rs`'s `combinator_indices`/`combinator_bodies` (no `IrFunc`,
body registered for the inliner), and the REPL's `checker_combinators`. The
`WordBody::Terms` conjunct is retained so a clause-bodied word is never a
combinator regardless of the flag; R3 rejects a clause-bodied `inline` word
before this predicate is consulted for lowering, so the two never disagree in a
way that reaches codegen.

### R3 — four new definition-site rejections

All four run as a pre-pass over `module.words` (`check_inline_declaration`,
`src/check/word_entry.rs`), before any body is checked and before
`is_combinator` is consulted, located at `word.span`, and phrased so the
diagnostic tests assert the *right* error:

- **`inline` on `main`.** The entry point is called by the runtime shim, so
  splicing it away leaves that call unresolved: without this the program dies as
  a raw `ld: undefined reference to 'sooth_main'`, not a Sooth diagnostic. This
  is the same `main`-is-not-a-combinator invariant
  `audit_word_quotation_positions` already enforces on the *quotation* route
  ("an input of `main`", D6/R28); the declared flag is a second route to it.
  Message:
  `error:`inline` on `main`, which is the program entry point; the entry point is called by the runtime shim and cannot be spliced`.
- **`inline` on a builtin-operator name.** When the demangled name is a
  `BUILTIN_TABLE` key (`mangle` suffixes an operator name per module, so the
  comparison must demangle first). `check_operator` claims the call site first,
  resolves the user overload and records `poly.builtin_overloads[span]` so
  lowering emits a real `Instr::Call` (`src/check/terms.rs:556`); the call then
  *also* falls through to the combinator interception (`terms.rs:676`), which
  splices. The record survives the splice, and lowering trusts it and looks the
  symbol up in an `env` a combinator is excluded from (`src/ir/driver.rs:117`) —
  a checker contradicting itself, and a panic in `ir/func_builder/calls.rs`
  downstream. Widening `is_combinator` (R2) is what made the shape reachable: an
  operator call site rejects a quotation operand outright, so a builtin-name
  overload could not previously be a combinator. Message:
  `error:`inline` on `{name}`, which overloads a builtin operator name; a call site of a builtin operator name dispatches through a real call and cannot be spliced`.
  Making the splice win instead (resolving an operator name against the live
  stack before `check_operator`) is the better feature but not a rewording — see
  "Out of scope".
- **`inline` on a clause body.** When `word.declares_inline` and
  `matches!(word.body, WordBody::Clauses(_))`:
  `error:`inline` on `{name}`, which has a clause body;`inline`requires a term body`.
  (An `inline` clause word would otherwise be `is_combinator == false` and lower
  as an ordinary clause word — a silent fall-back Decision 2 forbids, so it is a
  located error instead.)
- **`inline` on a variable-bearing signature.** Phrased over declared variables,
  not `poly.is_some()` (Decision 3): reject when `word.declares_inline` and the
  effect declares any of `'T`/`'N`/`..s`. Concretely, when `word.poly` is `Some`
  **and** the `PolySig` carries at least one variable (a non-empty
  `ty_var_names`, `len_var_names`, or a row variable). A `~`-bearing but
  otherwise concrete effect has `poly = Some` with all three empty and is
  **accepted**. Message:
  `error:`inline` on `{name}`, which declares a polymorphic signature;`inline`requires a monomorphic effect`.
  This one is **policy, not soundness** (Decision 3): with the guard disabled,
  `: swp inline ( 'A 'B -- 'B 'A )` compiles and runs correctly, so do not carry
  "poly `inline` is unsound" forward as a reason for it.
  Because a poly word is not routed through `check_word` (the poly arm of
  `check.rs`'s final loop calls `check_poly_body` / `check_poly_combinator_standalone`),
  this specific rejection is placed at the point where `word.poly.is_some()` is
  first known for an `inline` word — i.e. a guard at the top of `check`'s per-word
  loop (`check.rs`, before the `if let Some(sig) = &word.poly` branch), or an
  equivalent pre-pass over `module.words`. It must fire for a poly `inline` word
  *before* the poly checker runs, so a variable-bearing `inline` word never
  reaches a code path that assumes it is a legitimate poly combinator.

### R4 — reword the cycle rejection

`combinator_cycle_error` (`src/check/combinators.rs:201`) currently reads
`error: a quotation-taking word cannot be recursive (the inliner would splice it
forever): ...` (`:214`). An `inline` word need not take a quotation, so reword
the umbrella term to `an always-spliced word`:
`error: an always-spliced word cannot be recursive (the inliner would splice it
forever): ...`. The mechanism (`check_combinator_cycles`, its R4 self-tail
relaxation, the located chain) is unchanged; only the message string changes.
An `inline` word thus inherits cycle rejection verbatim, including the R4
allowance for a self-tail-only cycle (Decision 2).

### R5 — skip `check_reference_free_signature` for spliced words

At `src/check/word_entry.rs:24`, guard the call:

```rust
if !is_combinator(word) {
    check_reference_free_signature(&word.name, &word.effect, structs, enums, arrays)?;
}
```

Phrased over the shared predicate (Decision 5), so the exemption covers a mono
combinator, an `inline` word, and a poly combinator too, uniformly. **Correction
to the drafted rationale:** a poly word *does* reach `check_word` —
`check_poly_combinator_standalone` (`src/check/poly.rs:164`) builds a concrete
stand-in `WordDef` with `poly: None` and hands it to `check_word`. The stand-in
keeps the quotation parameter and the `declares_inline` flag, so it satisfies
`is_combinator` and takes the exemption by the same guard rather than by
not arriving. The rest of `check_word` (the input-slot variant-name check,
`word_entry.rs:19-23`) still runs. See "Feature C" for why this is sound.

### R6 — no lowering changes

`driver.rs` already excludes every `is_combinator` word from `env`, the per-word
`funcs` pass, `combinator_indices`, and mints its body into `combinator_bodies`
(`src/ir/driver.rs:55-72`). Widening `is_combinator` (R2) is the whole lowering
change; there is no new code in `driver.rs` or the inliner. `lower_call` splices
a zero-quotation-input combinator (recon 2) with no special case.

### R7 — widen the REPL retention gate

In `eval_def` (`src/repl.rs:2560`) the retention route becomes:

```rust
if word.declares_inline || check::word_declares_quotation_parameter(&word) {
    return self.eval_combinator_def(word, writer);
}
```

Without this, an `inline` word with no quotation parameter is lowered by the
REPL to a `.so` and a symbol (recon 8), violating Decision 2. Inside
`eval_combinator_def`, the mono branch (`check::check_def`) already checks such a
word identically to any term word, and `combinator_of` / `combinator_bodies`
project it by its `WordBody::Terms` shape, so no further REPL change is needed.
The `defined {name}` output is unchanged: an `inline` word takes the identical
retained-combinator path and prints the same line (this resolves brief open
question 4 — no new REPL rendering, which would be scope creep with no exit
criterion; R7 is a required plumbing fix, not a rendering feature).

### Feature B — library retype (`lib/combinators.sth`)

Retype the quotation parameters of the five hand-written combinators
(`each`, `map`, `fold`, `filter`, `while`) from `[ ... ]` to `~[ ... ]`,
e.g. `: each ( ['T 'N] ~[ 'T -- ] -- )`. These are poly combinators, so the
`~` sits in each word's own declared parameter and routes through the poly
parser / `parse_poly_slot` (recon 4; the `parse_slot` rejection at
`parser.rs:1559` is not on this path). No compiler change; a call site passing a
literal `[ ... ]` still type-checks against a `~[ ... ]` parameter, and because
all five are already inlined, the emitted QBE is byte-identical. A genuinely
first-class capturing/stored quotation still requires ordinary `[ ... ]` (7b's
territory) and is rejected against a `~[ ... ]` parameter by 10a's inline-only
rule. `times` is not touched (Decision 4; 10b owns it).

## Feature C — reference outputs on spliced words (resolving brief open questions 1 and 2)

R5 relaxes exactly one thing: the blanket structural rule
`check_reference_free_signature`, which rejects *any* `&T`/`&!T` output before
the body is even looked at. Every other pass that constrains a reference's
lifetime still runs on both the standalone def-site body and each spliced copy:

- the linear must-consume rule (`leave_block` at frame end);
- the capture/escape guards (`past_owning_frame_error`,
  `multi_capture_escaping_error`, `src/check/word_entry.rs` and `captures.rs`);
- the loop back-edge reference guard (`check_reference_across_back_edge`,
  `src/check.rs:1264`).

A `&T`/`&!T` is neither `Copy` nor linear (`src/check/builtins.rs:244`), so a
returned reference carries no drop obligation of its own; the obligation lives on
its *referent*.

**The adversarial shapes (brief open question 1), resolved:**

- **Reference derived from an input reference** (the recon-5 witness,
  `: pick ( &!Buf ~[ -- ] -- &!u32 ) | b f | f call b &!Buf>n ;`). The referent
  is outer-rooted (the caller's `Buf`). Sound; this is the positive exit witness.
- **Reference to a callee-declared local, referent non-linear.** Post-splice the
  local is a caller-frame local (`alpha_rename_locals`), living as long as any
  caller local, so the returned reference does not outlive its frame. Sound.
- **Reference to a callee-declared local, referent linear.** The referent carries
  a drop obligation. At the standalone def-site check, `leave_block` sees the
  linear local borrowed but not consumed and rejects it as unconsumed **by the
  pre-existing must-consume rule** — R5 does not touch that path. This is
  strictly conservative: post-splice the caller would inherit the obligation and
  it would be safe, but rejecting it is *reject-safe*, never accept-unsafe. This
  is the narrowing the brief asked for, achieved with **no special case**: the
  linear spine already forbids the dangerous shape.
- **Transitive inlining** (an `inline` word returning a reference, called by
  another `inline` word). Each splice layer alpha-renames; the chain bottoms out
  at the outermost non-spliced (real) word, whose frame owns every spliced local.
  Sound.
- **The outermost boundary.** A real (non-combinator) word declaring a reference
  output is still rejected: R5's guard is `if !is_combinator(word)`, and a real
  word is not a combinator. Unchanged.

**What the standalone def-site check verifies for such a word (brief open
question 2):** with `check_reference_free_signature` skipped, the standalone
check (recon 3) still verifies (a) body type-correctness against the declared
effect, including that the returned reference's referent is a live slot of the
matching referent type (`check_outputs`), and (b) the full linear discipline over
every local, including a linear referent (`leave_block`). What it does **not**
verify is the blanket "no reference output" — because that property is not true
for a spliced word. The frame-lifetime guarantee is not a check at the standalone
site (there is no caller to check against); it is a *theorem about the splice*:
`alpha_rename_locals` makes the referent a caller local, so the returned
reference cannot outlive its caller frame. The standalone check therefore stays
meaningful as a type-plus-linearity check, and the lifetime property is
discharged structurally by the inliner rather than asserted at the definition.

**Conclusion:** Decision 5 survives as stated. No special case is added; the
apparent adversarial holes are already closed by the must-consume rule and by
R5's `is_combinator` scoping.

## Growth-structure check (per CLAUDE.md, re-run at phase exit)

The edits land in existing stage files (`parser.rs`, `check/word_entry.rs`,
`check/combinators.rs`, `ir/driver.rs` untouched-but-reached, `repl.rs`,
`ast.rs`). None introduces import divergence, a module doing unrelated X/Y/Z, or
a would-be circular dependency. `is_combinator` stays the single shared
predicate — the opposite of a split. No module split is warranted; re-check at
exit.

## Test plan

Unit tests beside each stage function (`thing_condition_expected` naming),
happy path plus at least one error/edge case; every new test is mutation-tested
(reverting the guarded change must fail it).

**Parser (`src/parser.rs` `#[cfg(test)]`)**

- `parse_worddef_inline_keyword_sets_flag` — `: ClkDiv inline ( -- u32 u32 ) 8 4 ;`
  parses with `declares_inline == true`.
- `parse_worddef_no_inline_keyword_flag_false` — an ordinary word has
  `declares_inline == false`.
- `parse_worddef_word_named_inline_is_not_inline` — `: inline ( -- ) ;` names a
  word `inline` with `declares_inline == false` (name consumed first, recon 7).
- `parse_worddef_double_inline_is_parse_error` — `: foo inline inline ( -- ) ;`
  fails at `expect(LParen)`.

**Checker (`src/check/word_entry.rs` and `src/check/combinators.rs`
`#[cfg(test)]`)**

- `is_combinator_true_for_inline_non_quotation_word` — a direct `WordDef`
  construction with `declares_inline == true`, `WordBody::Terms`, no quotation
  input, returns `true`; flipping the flag to `false` returns `false`. (Direct
  construction, not e2e, per the resolve-mangle / discrimination memory.)
- `check_inline_clause_body_is_error` — asserts the exact R3 clause-body message.
- `check_inline_builtin_operator_overload_is_error` — an `inline` overload of
  `+` asserts the exact R3 operator-name message; the identical shape under a
  non-operator name is accepted (the rejection is keyed on the name, not the
  overload).
- `check_inline_polymorphic_signature_is_error` — a `'T`-bearing `inline` word
  asserts the exact R3 monomorphism message; a `~`-bearing (variable-free)
  `inline` word is **accepted** (the discriminating pair for "phrased over
  declared variables, not `poly.is_some()`").
- `check_inline_self_nontail_cycle_is_error` — asserts the R4-reworded
  `always-spliced word cannot be recursive` message; a self-*tail* `inline` word
  is accepted.
- `check_reference_free_signature_skipped_for_combinator` — a direct-construction
  test that the guard is keyed on `is_combinator`: an `inline` word with a `&!u32`
  output passes `check_word`, and the same effect on a non-combinator word still
  errors with the `word_entry.rs:52` message.

**REPL (`src/repl.rs` `#[cfg(test)]`)**

- `repl_inline_word_is_retained_not_lowered` — defining `ClkDiv inline` retains it
  in the combinator store and prints `defined ClkDiv`; asserts no `.so`/symbol is
  produced (mirroring how the existing retained-combinator tests assert
  retention, and asserting the exact stack/`defined` line per the REPL-placebo
  memory).

**Goldens (exit criteria)**

- `inline_word_mints_no_symbol` — `: ClkDiv inline ( -- u32 u32 ) 8 4 ;` compiles
  and `nm` shows no symbol for it (built the way
  `quotation_taking_word_mints_no_symbol`, `src/check/combinators.rs:539`,
  checks its own).
- `inline_word_caller_emits_no_call` — a caller of `ClkDiv` has no `Instr::Call`
  for it, asserted on lowered IR, not inferred from output.
- The five R3/R4 rejections as located-error goldens (source in → exact
  diagnostic out): `main`, builtin-operator name, clause body,
  variable-bearing signature, and the reworded cycle. The operator-name golden
  pairs its rejection with the same overload *without* `inline` building and
  running, so the rejection is shown to be the keyword's.
- `inline_reference_output_pair` — `pick` (Feature C positive witness) compiles
  and its caller reads the returned reference with the right value; the *same*
  word without `inline` still fails with the `check_reference_free_signature`
  message. The pair is the witness that Decision 5 is scoped to the splice.
  Note the pair's `pick` must take **no** quotation: the recon-5 shape
  (`&!Buf ~[ -- ]`) is a combinator with or without the keyword, so dropping
  `inline` from *it* changes nothing and the pair would be a placebo. That shape
  earns a separate positive golden instead — a reference output accepted on a
  word that declares no `inline` at all is what discriminates R5's
  `is_combinator` phrasing from a `declares_inline` one (Decision 5).
- The caller's half of the positive witness writes through the returned
  reference and reads the new value back out of its own struct, so the golden
  fails if the reference points at anything but the caller's live local.
- The surviving adversarial shape from Feature C (the linear-referent rejection)
  gets its own negative golden: an `inline` word borrowing-and-returning a
  reference to a linear callee-declared local is rejected by must-consume.
- The two remaining Feature C shapes get positive goldens, since the shapes the
  witness pair covers are all caller-rooted through an *input* reference and so
  never exercise `alpha_rename_locals`:
  `inline_reference_to_nonlinear_callee_local_is_accepted` (referent is a
  non-linear struct the callee itself pushes) and
  `transitive_inline_reference_output_is_accepted` (an `inline` word returning a
  reference, called by another `inline` word that returns it on). Both write
  through the returned reference and read the new value back; in the transitive
  one a write in the middle layer and a write in `main` must land in the same
  referent, and dropping `inline` from either layer rejects the program.
- `combinators_retype_output_byte_identical` — after Feature B, a corpus program
  exercising `each`/`map`/`fold`/`filter`/`while` emits QBE byte-identical to the
  pre-retype baseline; a stored/returned quotation still requires `[ ... ]`
  (rejected against a `~[ ... ]` parameter).

## Out of scope

- Any call-site inlining request or automatic folding of an unmarked word
  (Decision 1).
- Interleaved quotation parameters in a self-tail combinator (Decision 6).
- `times` and `lib/combinators.sth`'s ownership of it (10b).
- `if`/`cond` as ordinary words (10c).
- Statics, `volatile`, fixed-address MMIO (separate, larger language design).
- **Making combinator resolution authoritative for an operator name.** R3
  rejects an `inline` overload of a builtin operator name rather than making it
  work. The feature version — resolve an operator name against the live stack as
  a combinator first, falling back to `check_operator` when no candidate matches
  — is worth doing (an inline `+` on a struct is exactly this slice's embedded
  motivation) but is not a one-liner: suppressing the stale
  `poly.builtin_overloads` record alone moves the failure to
  `src/backend/qbe.rs:1144` ("an aggregate is not a printable scalar"), so the
  splice does not land correctly at an operator site either. Its own slice.
- **The name-keyed combinator graph vs. overload sets.** Two `inline` `drop`
  overrides for different types are rejected as
  `an always-spliced word cannot be recursive: 'drop' -> 'drop' -> 'drop'` — a
  false cycle, because `check_combinator_cycles` keys on the bare name while
  `drop` is an overload set (the non-`inline` pair is fine). Same root class as
  the operator rejection above. A located error, not a miscompile, so it is
  deferred with it.
- **`tree-sitter-sooth/grammar.js`.** The editor grammar has not tracked the
  language since it was added: it already does not know 10a's `~[`, and `inline`
  widens the gap. A single sweep that re-syncs it with the real lexer/parser,
  not a per-slice patch.

## Sequencing

Depends on 10a (`~`, shipped `e87bcae`) and `0313b74` (the self-tail ICE fix).
**Starts after 10b merges**: nothing here depends on 10b's outcome, but both edit
`lib/combinators.sth` and must not race on it. Phase 1 (the mechanism) and
Phase 2 (the reference exemption) are compiler-only and independent of 10b;
Phase 3 (the library retype) is the piece that must wait for 10b.

## Phases

| Phase | Focus | Difficulty |
| --- | --- | --- |
| 1 | `inline` keyword: grammar, `WordDef.declares_inline`, widen `is_combinator`, the five definition-site rejections (`main`, builtin-operator name, clause body, variable-bearing signature, reworded cycle), and REPL retention. Goldens: no-symbol, no-`Instr::Call`, five located-error rejections, plus R2's two unpinned splice routes (an imported `inline` word, and `inline` calling `inline`). | hard |
| 2 | Reference outputs on spliced words: skip `check_reference_free_signature` under `is_combinator`; the with/without-`inline` witness pair and the linear-referent adversarial negative golden; the standalone-check treatment. | hard |
| 3 | Library retype (Feature B): `lib/combinators.sth` quotation parameters to `~[ ... ]`; byte-identical corpus-output golden; stored/returned quotation still requires `[ ... ]`. | standard |

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "inline keyword grammar, WordDef.declares_inline field, widen is_combinator, five definition-site rejections, REPL retention",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "reference outputs on spliced words: skip check_reference_free_signature under is_combinator, witness pair and adversarial golden",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "library retype of lib combinators quotation parameters to tilde-bracket, byte-identical corpus output",
      "difficulty": "standard"
    }
  ]
}
```
