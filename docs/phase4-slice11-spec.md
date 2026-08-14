## What shipped

`inline` is now a declared word property, spelled between a word's name and its
effect:

```sooth
: ClkDiv inline ( -- u32 u32 ) 8 4 ;
```

`ClkDiv` mints no `IrFunc`, no symbol and no `Instr::Call` at any call site: it is
spliced at every call, exactly as a quotation-taking combinator already was. The
property is no longer *inferred* from "declares a quotation parameter"; it is
declarable independently of the word's shape. The guarantee is unconditional:
where splicing is impossible the definition is a located error, never a silent
fall-back to a real call (D2). Motivation is the embedded/RT target: a reader
tells from the source whether a call site costs a call, rather than trusting an
optimiser to keep recognising a shape after an edit (the argument that already
justifies 10a's `~[ ... ]`).

Two consequences of the same mechanism also shipped: `~` is no longer
`times`-only (Feature B, library-only), and a spliced word may declare a
reference output (Feature C).

## Decisions

1. `inline` is a property of the definition, not a per-call-site sigil.
2. The guarantee is unconditional; unmeetable shapes are located errors at the
   definition.
3. `inline` requires a term body and a monomorphic effect. Monomorphism is
   phrased over **declared variables** (`'T`/`'N`/`..s`), not `poly.is_some()`: a
   `~`-bearing effect is poly-forced by the parser (`effect_has_variable`) while
   carrying no variable, and is accepted. This rule is **policy, not soundness**:
   with the guard disabled, `: swp inline ( 'A 'B -- 'B 'A )` compiles and runs
   correctly. Do not carry "poly `inline` is unsound" forward.
4. `~` generalises by library migration, not compiler change. `times` excluded
   (10b owns it).
5. `check_reference_free_signature` is skipped exactly when `is_combinator(word)`
   holds, so the exemption covers a mono combinator, an `inline` word and a poly
   combinator uniformly.
6. The residual self-tail hoist gap from `0313b74` stays open.

## Delivered requirements

**R1, grammar and AST.** `pub declares_inline: bool` on `WordDef`
(`src/ast.rs:587`). `parse_worddef` peeks for `Word("inline")` after the name and
before `(` (`src/parser.rs:987`). A second `inline` is not consumed and fails at
`expect(LParen)`. The keyword is **not** reserved: the name slot is consumed
first, so `: inline ( -- ) ;` still defines a word called `inline`. The flag is
propagated by the poly stand-in (`src/check/poly.rs:169`) and the REPL's
`remap_imported_combinator` (`src/repl.rs:370`); synthesized words are `false`.

**R2, the widened predicate.** `is_combinator` (`src/check/combinators.rs:76`) is
now `WordBody::Terms && (word_declares_quotation_parameter(word) ||
word.declares_inline)`. This single read reaches `collect_combinators`,
`driver.rs`'s `combinator_indices`/`combinator_bodies`, and the REPL's
`checker_combinators`, with no further plumbing.

**R3, four definition-site rejections.** `check_inline_declaration`
(`src/check/word_entry.rs:66`) runs as a pre-pass before any body is checked and
before `is_combinator` is consulted, called from `check.rs:549` and from the REPL
at `repl.rs:2559`. In order: `main`, clause body, variable-bearing signature,
builtin-operator name. Two of these were found during implementation, not in the
brief:

- **`main`.** The entry point is called by the runtime shim; splicing it away
  otherwise dies as a raw `ld: undefined reference to 'sooth_main'` instead of a
  Sooth diagnostic. Same invariant `audit_word_quotation_positions` already
  enforces on the quotation route; the flag is a second way in.
- **A builtin-operator name.** `check_operator` claims the call site first,
  records `poly.builtin_overloads[span]` so lowering emits a real `Instr::Call`,
  and the call *then also* falls through to the combinator interception and
  splices. The stale record survives, lowering looks the symbol up in an `env` a
  combinator is excluded from, and panics downstream. R2 is what made the shape
  reachable: an operator call site rejects a quotation operand outright, so a
  builtin-name overload could not previously be a combinator. The name is
  demangled before the `BUILTIN_TABLE` lookup (`mangle` suffixes per module).

**R4, cycle message rewording.** `combinator_cycle_error` now says `an
always-spliced word cannot be recursive`, since an `inline` word need not take a
quotation. Mechanism unchanged, including the self-tail-only relaxation, which an
`inline` word inherits.

**R5, the reference-output exemption.** `check_word` guards the call with
`if !is_combinator(word)`. Correction to the drafted rationale: a poly word *does*
reach `check_word`. `check_poly_combinator_standalone` builds a concrete stand-in
`WordDef` with `poly: None`, keeping the quotation parameter and the flag, so it
takes the exemption by the same guard rather than by not arriving.
`check_extern_decls` has no splice to exempt and still always runs the check.

**R6, no lowering changes.** R2 is the whole lowering change.

**R7, REPL retention.** `eval_def` (`src/repl.rs:2570`) routes on
`word.declares_inline || word_declares_quotation_parameter(&word)`. Without it, an
`inline` word with no quotation parameter was lowered to a `.so` and a symbol,
violating D2 inside the REPL. The `defined {name}` output is unchanged.

**Feature B.** `each`, `map`, `fold`, `filter` and `while` in
`lib/combinators.sth` now declare `~[ ... ]` parameters. No compiler change: the
`parse_slot` rejection of `Token::TildeLBracket` is not on the poly path, a call
site passing a literal `[ ... ]` still checks, and since all five were already
inlined the emitted QBE is byte-identical. A stored or returned quotation still
requires `[ ... ]` and is rejected against a `~[ ... ]` parameter.

## Feature C: why the reference exemption is safe

R5 relaxes exactly one thing, the blanket structural rule that rejects any
`&T`/`&!T` output before the body is looked at. Its own message names the fault it
guards ("borrows a local of the callee's own frame, which is gone by the time the
caller reads it"), and a spliced word has no such frame: `alpha_rename_locals`
makes the callee's locals caller locals. Every lifetime and linearity pass still
runs on both the standalone body and each spliced copy: must-consume at
`leave_block`, the capture/escape guards, and `check_reference_across_back_edge`.
A `&T`/`&!T` is neither `Copy` nor linear, so the drop obligation lives on the
referent.

The adversarial shapes, as resolved:

- Reference derived from an input reference: outer-rooted, sound.
- Reference to a non-linear callee-declared local: post-splice it is a caller
  local, sound.
- Reference to a **linear** callee-declared local: rejected at the standalone
  check by the pre-existing must-consume rule. Strictly conservative and
  reject-safe, with no special case; the linear spine already forbids it.
- Transitive splicing: each layer alpha-renames, bottoming out at the outermost
  real word, whose frame owns every spliced local.
- A real (non-combinator) word declaring a reference output is still rejected.

The standalone check remains a type-plus-linearity check; the frame-lifetime
property is discharged structurally by the inliner, not asserted at the
definition.

## Coverage

Unit tests sit beside their stage (`parser.rs` for the keyword and the
`word named inline` / double-`inline` cases; `combinators.rs` for
`is_combinator` asserted both ways round on a directly-constructed `WordDef`;
`word_entry.rs` for the R5 guard). Exit witnesses live in
`tests/phase4_slice11_inline.rs`: no-symbol (via `nm`), no `Instr::Call` on
lowered IR, splicing across an import and `inline` calling `inline`, the five
located-error rejections, the `~`-bearing acceptance that discriminates D3's
declared-variable phrasing, self-tail-as-loop, the reference-output pair (whose
`pick` takes **no** quotation, or dropping `inline` would change nothing and the
pair would be a placebo), a reference output accepted on a quotation-taking word
with no keyword, the linear-referent negative, the non-linear-callee-local and
transitive positives (each writing through the returned reference and reading the
new value back), REPL retention and REPL poly rejection, and the byte-identical
Feature B corpus output.

## Deferred, with reasons

- **Making combinator resolution authoritative for an operator name.** The
  feature version (resolve an operator name against the live stack first, falling
  back to `check_operator`) is worth doing, an inline `+` on a struct is this
  slice's motivation, but is not a one-liner: suppressing the stale
  `poly.builtin_overloads` record alone moves the failure to
  `src/backend/qbe.rs` ("an aggregate is not a printable scalar"). Its own slice.
- **Name-keyed combinator graph vs overload sets.** Two `inline` `drop`
  overrides for different types trip a false cycle (`'drop' -> 'drop' ->
  'drop'`), because `check_combinator_cycles` keys on the bare name. Same root
  class as the operator rejection. A located error, not a miscompile.
- **`tree-sitter-sooth/grammar.js`.** Already behind (it does not know 10a's
  `~[`); `inline` widens the gap. Wants one re-sync sweep, not a per-slice patch.
- Call-site inlining requests or auto-folding of unmarked words (D1); interleaved
  quotation parameters in a self-tail combinator (D6); `times` (10b); `if`/`cond`
  as ordinary words (10c); statics, `volatile`, fixed-address MMIO.

## Growth structure

Edits landed in existing stage files (`parser.rs`, `check.rs`,
`check/word_entry.rs`, `check/combinators.rs`, `check/poly.rs`, `repl.rs`,
`ast.rs`); `ir/driver.rs` was reached but untouched. No import divergence, no
module doing unrelated work, no would-be circular dependency. `is_combinator`
stays the single shared predicate. No split warranted.
