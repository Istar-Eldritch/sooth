# Spec: named effect-slot locals ("slot-name sugar")

**Status:** Draft
**Created:** 2026-09-02
**Discovery:** `docs/named-slot-locals-discovery.md` (conversation + 3 probe/paper rounds, all
probes verified 2026-09-02 at worktree `quotation-locals-sugar` @ `403618f`)

## Problem Statement

Sooth word definitions bind entry locals with an explicit body block —
`: myfunction ( i32 i32 -- i32 ) | a b | a b + ;` — so the names are written twice
conceptually: once by position in the effect, once in the body block. Users want
optional inline naming in the effect itself: `: myfunction ( a: i32 b: i32 -- i32 ) a b + ;`,
per-slot optional, composing with body-level `| ... |` blocks including out-of-order
naming (`( a: i32 i32 -- ) | b | ...`, where the named slot is deeper than an unnamed
one). Today the spaced form parses but the name is dead (doc-only except the X12
variant-collision check); the glued form errors confusingly as ``unknown type `x:```;
duplicate slot names are silently legal; and a poly-effect name attempt produces the
same confusing``unknown type `x``. The feature turns those dead or confusing names
into real, sharp, linear locals — one positional desugar rule, no new checker or IR
semantics.

## The desugar rule (settled, restated from discovery)

One positional rule: **each named input slot becomes a local bound to its slot's
value; unnamed input slots stay on the stack in their original relative order.**

- In `parse_worddef` only, after `parse_terms` returns (`src/parser.rs:3211`), extract
  the input-slot names and prepend a `TermKind::Bind(names)` term to the body —
  indistinguishable from a user-written leading `| a b |` (built by the Pipe arm,
  `src/parser.rs:8006-8018`).
- `Bind` pops from the top, leftmost name deepest (`src/check/terms.rs:148`), so when
  an unnamed slot sits above a named one, the desugar binds from the top down to the
  deepest named slot, giving the unnamed slots in that group fresh minted names, then
  immediately re-pushes those locals in original order (verified probe:
  `| a t | t | b | a b add` runs, prints 7).
- When named slots are top-contiguous, the desugar emits zero mints — a plain `Bind`
  prepend. When all slots are unnamed, the parse path is byte-identical to today.

## Fixed constraints

- **F1** No lexer change: both spellings arrive as existing word tokens (`a :` spaced,
  `a:` one glued token). Precedent for splitting a glued trailing colon in the parser:
  `parse_optional_bound_bracket` (`src/parser.rs:3246`), glued `'T:` handling.
- **F2** No checker-code change and no IR change: collision, rebind, X12, entry-arity,
  leak, and lowering semantics are all inherited from the existing `Bind` handling.
- **F3** `| a b |` block semantics are untouched; `parse_term`'s Pipe arm
  (`src/parser.rs:8006-8018`) is unchanged.
- **F4** QBE backend, `Ptr[T]` opaque, linear spine, `core`/`no_std` layering, no JIT —
  all load-bearing per CLAUDE.md; nothing here touches them.
- **F5** Green gate every phase: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Requirements

- **R1.** A word-definition input slot accepts both the spaced spelling `a : i64` and
  the glued trailing-colon spelling `a: i64`; both parse to
  `TypedSlot { name: Some("a"), ty: i64 }` with no lexer change.
- **R2.** In a slot position, a single word token containing exactly one `:` that is
  not trailing (`a:i64`) produces a located parse error whose message directs the
  user to put a space after `:` (replacing today's ``unknown type `a:i64``).
  Multi-colon spellings are exempt — `::`-qualified types (`q::Point`,
  `r::Result[i64 i64]`) keep resolving as types — a leading-colon token (`:i64`)
  falls through to the type resolver (a leading colon names nothing, and the hint
  would suggest an empty name), and a token ending in `:` is R1's
  glued split, not this error.
- **R3.** In a word definition, each named input slot becomes a local bound to its
  slot's value before the body runs, and unnamed input slots stay on the stack in
  their original relative order; a sugar program and its explicit `| ... |` twin
  produce identical runtime output.
- **R4.** The desugar is emitted in `parse_worddef` only, after body parse, as a
  prepended `TermKind::Bind` term (plus immediate re-push `Call` terms when mints
  exist) that is indistinguishable from a user-written leading `| ... |` binding.
- **R5.** When an unnamed input slot sits above or between named ones, the desugar
  binds it to a fresh positional mint and immediately re-pushes the minted local,
  preserving the unnamed slots' original relative stack order.
- **R6.** The mint base candidate is `__slot{k}` with k = the unnamed slot's 0-based
  input index, and k increments by one until the candidate is fresh against every
  name bound anywhere in the body (all `TermKind::Bind` terms, nested quotations
  included), the word's own name, the module's enum variant names, every other
  input-slot name in the effect, and every name minted earlier in this same desugar
  (mints resolve deepest-first; each mint's final name joins the freshness set
  before the next candidate is chosen). A user word named `__slot{k}` collides with
  a mint deterministically and dies sharply at bind time with
  `callable_local_error` — the exposure class discovery decision 5 accepts.
- **R7.** When the named input slots are exactly the top-contiguous run of the effect,
  the desugar emits a single `Bind` term and zero mints.
- **R8.** An `extern:` declaration with named slots parses with names intact and
  performs no binding (externs have no Sooth body/frame; names stay doc-only).
- **R9.** A named output slot (`( -- x : i64 )`) parses and stays doc-only — no
  binding, no duplicate-name check, no behaviour change from today.
- **R10.** Quotation effect rows (`parse_quotation_effect_rows`) are untouched: no name
  path exists there and none is added.
- **R11.** In a polymorphic-effect slot position, a word containing no `'` followed
  by `:` (spaced `x : 'T` or glued `x: 'T`) produces the located parse error "slot
  names are not supported in polymorphic effects" instead of today's
  ``unknown type `x``. Tokens containing `'` are exempt — `'T :`, `'T:`, and
  sigil-glued `&!'T:` all keep the existing located bound-in-effect error
  (`src/parser.rs:5042-5076`, pinned by `parse_worddef_bound_in_effect_is_error`,
  `src/parser.rs:12963`). The reject fires on every route into `parse_poly_slot`:
  `effect_has_variable`, `force_poly` (any non-empty bound bracket,
  `src/parser.rs:3182-3186`), trait member signatures (`parse_poly_slots` calls at
  `src/parser.rs:3776/3781`), and poly quotation-effect rows (they recurse through
  `parse_poly_slot`, `src/parser.rs:5082-5083` — they inherit the sharper reject).
- **R12.** Two input slots with the same name in one word-definition effect produce a
  located parse error in `parse_worddef` naming the duplicated slot (input slots only;
  output names remain unchecked).
- **R13.** The desugar must not clear `TypedSlot.name`: after parsing, named slots keep
  their names, so the existing checker-side consumers keep working unchanged (the
  `array`-slot pin at `src/parser.rs:13347` passes unmodified).
- **R14.** Slot names obey the inherited local rules, with no checker code change:
  callable-name collision (`callable_local_error`, `src/check.rs:1235`), rebind against
  a body `| a |` block (`rebound_local_error`, `src/check.rs:2936`), and the X12
  variant-collision wording "parameter" (`src/check/word_entry.rs:33-38`) — each
  behaviour pinned by a diagnostic test.
- **R15.** A named input slot that the body never consumes fails the linear
  bound-but-unused check (`Scope::leave`, `src/check/engine.rs:557-570`) with a
  diagnostic naming the slot; copy-typed named slots impose no use obligation.
- **R16.** The desugared leading `Bind` never fires the entry-arity diagnostic
  "locals bind N value(s), but only M input(s) are declared"
  (`src/check/word_entry.rs:200-213`); that diagnostic remains reachable only for
  hand-written blocks.
- **R17.** A desugared leading `Bind` lowers identically to a hand-written one,
  including on the self-tail-combinator leading-Bind path
  (`src/ir/func_builder/calls.rs:100-135`); no IR code changes.
- **R18.** With no slot names anywhere in an effect, the parse path is byte-identical
  to today — except the trailing-colon-type corner ruled in Open Questions:
  `parse_effect_without_global_clause_unchanged` (`src/parser.rs:12338`) and
  the Phase 0 `gcd`/`factorial` goldens pass unchanged.
- **R19.** The stale `StackEffect`/`TypedSlot` doc comment at `src/ast.rs:2944`
  ("a name is never written twice") is updated to describe worddef-input binding.
- **R20.** `docs/book/words.md` gains a named-slot-locals section (syntax, the
  desugar rule, collision rules) and its stale "locals shadow words" section
  (`docs/book/words.md:83-99`) is corrected to match the checker (a local may NOT
  shadow a word; the section's own example fails today).
- **R21.** `docs/book/the-stack.md`'s effect-notation section mentions optional input
  slot naming.
- **R22.** A one-line bookkeeping note in `docs/roadmap/ROADMAP.md`'s "Current status /
  next action" section records the feature.

## Success Criteria

- [ ] A Sooth program using named input slots (spaced and glued spellings, all-named,
      out-of-order mixed with `| ... |` blocks, mint-collision case) compiles, runs,
      and prints byte-identical output to its explicit `| ... |` twin (goldens).
- [ ] Bad spellings produce sharp located diagnostics: fully-glued hint error (R2),
      poly reject (R11), duplicate-slot reject (R12) — each a diagnostic golden.
- [ ] Inherited checker behaviours hold for slot names: callable collision, rebind,
      X12 "parameter" wording, unused-slot linear leak (R14/R15 diagnostic goldens).
- [ ] Desugared binds never trip entry-arity (R16) and lower identically through the
      self-tail path (R17 golden).
- [ ] No regression: `parse_effect_without_global_clause_unchanged`, Phase 0 goldens,
      full suite green under F5.
- [ ] Docs match the shipped behaviour: `words.md` named-slots section + corrected
      shadowing section, `the-stack.md`, `ast.rs` doc comment, ROADMAP line.

## Scope & Boundaries

**In scope:**

- Word-definition input-slot naming for monomorphic effects: spelling support
  (`parse_slot`), the desugar in `parse_worddef`, positional mints with a freshness
  scan, duplicate-slot and poly-slot rejects, parser unit tests, goldens, checker
  behaviour pins, book docs, and bookkeeping.

**Out of scope** (discovery "Non-goals"):

- Output-slot binding (names stay doc-only, R9).
- Poly-effect slot names (sharp reject only, R11).
- Extern sugar (R8). Trait impl member bodies are structurally excluded too:
  `parse_impl_member_body` (`src/parser.rs:4095`) parses bodies without an effect
  (restating one is rejected) and grounds slots `name: None` — the sugar cannot
  reach them.
- Lexer changes (F1) and any change to `| a b |` block semantics (F3).
- Reserved `__`-namespace hardening (pre-existing wart, accepted by discovery
  decision 5).
- LSP/editor work; `tree-sitter-sooth` changes (highlighting-only grammar needs zero
  changes — discovery decision 9).
- New rejects inside quotation effect rows (R10: untouched; today's errors stand).

## Solution Approach

The desugar lives entirely in the parser, at one point: `parse_worddef`, after the
body has been parsed (`src/parser.rs:3211`). The effect is already parsed by then, so
the input slot names are available on `StackEffect.inputs` (`src/ast.rs:2946-2951`);
the desugar extracts them, checks them for duplicates (R12), and prepends terms to
`body` before `WordDef` is constructed. The prepended `Bind` is built exactly like the
Pipe arm builds one (`src/parser.rs:8006-8018`), which is what makes every downstream
behaviour — pop order, rebind/collision diagnostics, linear use-exactly-once,
entry-arity, and the self-tail leading-Bind lowering — inherited rather than invented
(F2). Minted names for the out-of-order case follow the discovery's settled scheme:
`__slot{k}` from the slot's input index, bumped until fresh against body-bound names
(a recursive walk of all `Bind` terms, mirroring `rename_terms`, `src/ast.rs:3590`),
the word's own name, and enum variant names (available on `Parser.enums`, pre-populated
by the variant pre-pass — `src/parser.rs:1176`, `src/ast.rs:497`). Minted locals are
consumed exactly once by their re-push, so they never render in diagnostics on
well-formed paths, and the desugar `Bind` can never underflow (its arity is known from
the effect).

Spelling support lands in `parse_slot` (`src/parser.rs:5533`), not the lexer: the tail
of `parse_slot` already reads the leading word and checks for a standalone `:` — a
word ending in `:` is split there (precedent: the glued `'T:` handling inside
`parse_optional_bound_bracket`, `src/parser.rs:3246`), and a one-word `a:i64` gets the
new located hint error. Because `parse_slot` also serves extern declarations and
output slots, the split is name-half-only and behaviour-preserving for them (their
names remain doc-only, R8/R9); the no-name path through `parse_effect` stays
byte-identical (R18), which `parse_effect_without_global_clause_unchanged` witnesses.
Poly slots take a separate reader (`parse_poly_slot`, `src/parser.rs:4542`) with no
name path, so the poly reject is a small guard there plus nothing else — quotation
rows and the concrete/poly routing (`effect_has_variable`, `src/parser.rs:4448`) are
not touched.

The checker and IR get no code changes: phase 2 only pins the inherited behaviours
with diagnostic goldens so the semantics cannot drift later, and phase 3 reconciles
the book (which currently claims locals shadow words — the checker wins; the book's
own example fails today) and the now-stale `TypedSlot` doc comment.

## Codebase Map

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/parser.rs:3162` | `parse_worddef()` | Desugar insertion point: extract slot names, duplicate reject, prepend desugar terms to `body` before `WordDef` construction |
| `src/parser.rs:3211` | `parse_terms` call in `parse_worddef` | Anchor line: body is complete here; the desugar runs immediately after |
| `src/parser.rs:5521` | `parse_slots()` | Slot-list loop shared by inputs and outputs; unchanged, but the desugar's data source |
| `src/parser.rs:5533` | `parse_slot()` | Gains the glued trailing-colon split and the fully-glued hint error (serves worddef effects *and* extern slots via `parse_effect`) |
| `src/parser.rs:4425` | `parse_effect()` | Unchanged; builds `StackEffect { inputs, outputs }` from `parse_slots` |
| `src/parser.rs:4448` | `effect_has_variable()` | Unchanged; concrete/poly routing guarantee behind R18 |
| `src/parser.rs:3558` | `parse_extern_decl()` | Unchanged; externs share `parse_effect` and keep doc-only names (R8) |
| `src/parser.rs:4542` | `parse_poly_slot()` | Gains the located poly slot-name reject (R11) |
| `src/parser.rs:5042` / `:12963` | `parse_poly_ty_var()` / `parse_worddef_bound_in_effect_is_error` | `'T:` bound attempts keep their existing located error — the R11 exemption's witness |
| `src/parser.rs:5874` | `parse_quotation_effect_rows()` | Unchanged; out of scope (R10) |
| `src/parser.rs:8006-8018` | `parse_term()` Pipe arm | The user-written `TermKind::Bind` construction the desugar must be indistinguishable from (do not modify) |
| `src/parser.rs:1176` | `scan_variant_names()` | Pre-pass populating variant names before word bodies parse — evidence variant names are cheaply reachable |
| `src/parser.rs:3010` | `struct Parser` | Holds `enums: &'t [EnumDecl]` (variant names always populated) — the mint freshness scan's variant source |
| `src/parser.rs:8123` | `mod tests` (parser) | New parser unit tests land beside the existing ~600 |
| `src/parser.rs:12338` | `parse_effect_without_global_clause_unchanged` | Regression witness — must pass unchanged (R18) |
| `src/parser.rs:13347` | `parse_slot_named_array_with_type_annotation_parses` | Existing slot-name pin (`name == Some("array")`) — must survive unchanged (R13) |
| `src/ast.rs:2944-2955` | `StackEffect` / `TypedSlot` | `TypedSlot { name: Option<String>, ty }`; names stay populated (R13); stale doc comment updated in Phase 3 (R19) |
| `src/ast.rs:3499` | `TermKind::Bind(Vec<String>)` | The term the desugar prepends; pops top-down, leftmost name deepest |
| `src/ast.rs:3554` | `INLINE_SUFFIX = "__inl"` | Naming precedent for the `__slot{k}` mint scheme |
| `src/ast.rs:3590` | `rename_terms()` | Precedent for walking all `Bind` terms nested-included (the freshness scan mirrors this walk) |
| `src/ast.rs:497` / `src/ast.rs:511` | `EnumDecl` / `VariantDecl` | `variants` list with `name` fields read by the freshness scan |
| `src/ast.rs:1785` / `:1796` | `WordDef` / `WordDef.body: Vec<Term>` | The desugar prepends into this `Vec<Term>` |
| `src/check/terms.rs:148` | `TermKind::Bind` check arm | Pop semantics the desugar relies on (leftmost name deepest); unchanged |
| `src/check/word_entry.rs:33-38` | X12 variant-param check over `effect.inputs` | Keeps firing with "parameter" wording for slot names (R14) |
| `src/check/word_entry.rs:200-213` | entry-arity diagnostic | Must never fire for a desugared `Bind` (R16) |
| `src/check/engine.rs:557-570` | `Scope::leave()` | Bound-but-unused linear leak check that now names unused slot locals (R15) |
| `src/check.rs:1235` | `callable_local_error()` | Callable-collision wording pinned by Phase 2 (R14) |
| `src/check.rs:2936` | `rebound_local_error()` | Rebind wording pinned by Phase 2 (R14) |
| `src/ir/func_builder/calls.rs:100-135` | leading-Bind split in `lower_self_tail_combinator()` | Handles a desugar `Bind` identically — no IR change (R17) |
| `tests/phase3_locals.rs:11-41` | `run_src` / `check_error` / `parse_error` | Harness model for the new golden file (compile+run, in-process check error, in-process parse error) |
| `tests/phase0.rs:60-67` | `gcd` / `factorial` goldens | Phase 0 regression goldens (R18) |
| `tests/slot_locals.rs` *(new)* | — | New golden file in `tests/`, modelled on `tests/phase3_locals.rs` |
| `docs/book/words.md:83-99` | "locals shadow words" section | Stale (checker wins; the section's example fails today) — corrected in Phase 3 (R20) |
| `docs/book/the-stack.md` | effect-notation section | Mentions optional input naming in Phase 3 (R21) |
| `docs/roadmap/ROADMAP.md:6` | "Current status / next action" | Bookkeeping line appended in Phase 3 (R22) |

## Open Questions

- [x] ~~Do quotation effect rows also get the poly-style sharp reject?~~ Resolved: no —
  discovery decision 2 gives the located reject only to `parse_poly_slot`; quotation
  rows are "excluded likewise" from the *sugar*, and today's errors stand there (R10).
- [x] ~~Exact duplicate-slot error wording?~~ Not pinned by discovery; implementer
  writes it in the existing located parser-error style (precedent:
  `duplicate_generic_ty_var_error` inside `parse_optional_bound_bracket`,
  `src/parser.rs:3246`), naming the duplicated slot. Note `TypedSlot` carries no span,
  so the error cites the word definition's span — precise per-slot spans would require
  widening the AST and are out of scope. Not blocking.
- [x] ~~Mint base index 0- or 1-based?~~ Resolved deterministically: 0-based input-slot
  index (deepest = 0), matching the discovery's "k = slot index".
- [x] ~~A type declared with a trailing-colon name (`type: Foo:`) is legal today; does
  the glued split reinterpret it?~~ Resolved (review round 1): the sugar owns `:` in
  slot position — the split is unconditional. `( Foo: -- )` reads as a named-slot
  attempt and errors sharply on the missing type; `( Foo: i64 -- )` (today two
  slots) becomes one slot named `Foo` of type `i64`. Corpus exposure is zero
  (review-verified); such a type must be renamed to be usable bare, and both
  corners are pinned by unit test. A registry-lookup guard was rejected (a fuzzy
  lookup would silently accept a typo like `i64:` as `i64`). Resolved further
  (review round 2): the split does not fire when the name half itself contains
  `:` — a qualified-name-shaped token (`q::Point:`, or the degenerate `::`)
  falls through to the type resolver and dies as an unknown-type error instead
  of minting a slot named `:` or `q::Point`. All other rulings above stand.

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Line drift in the 14.8k-line `src/parser.rs` between spec and phase run | Med | Every anchor pairs `path:line` with a symbol name; implementer re-locates by symbol, never by line alone |
| Glued-split in `parse_slot` leaks into extern/output slots in a visible way | Low | The split only affects the name half; R8/R9 exclusion tests and `parse_effect_without_global_clause_unchanged` (R18) catch any drift |
| Freshness scan misses a `Bind` nested in a quotation → mint collides with a body local | Low | Walk mirrors `rename_terms` (`src/ast.rs:3590`) which already recurses; the checker's `rebound_local_error` remains a sharp backstop (discovery decision 6) |
| Phase 2 pins reveal a Phase 1 desugar defect | Low | Phase 1's exit criteria already include the twin-equivalence goldens that exercise the same paths; a failing pin is escalated as a Phase 1 defect, never patched with new checker code |
| Phases 2 and 3 running in parallel collide on files | Low | Disjoint footprints: Phase 2 touches only `tests/`, Phase 3 only `docs/`, `src/ast.rs` (comment), `ROADMAP.md` |

## Delivery Plan

### Phase 1: Parse + desugar core

- **Goal**: A Sooth program using named input slots — spaced or glued, all-named or
  out-of-order mixed with `| ... |` — compiles, runs, and prints byte-identical output
  to its explicit `| ... |` twin, while bad spellings produce sharp located parse
  errors.
- **Requirements Covered**: R1, R2, R3, R4, R5, R6, R7, R8, R9, R10, R11, R12, R13, R18
- **Scope**:
  - Modify `src/parser.rs:5533` (`parse_slot()`): at the tail's name read
    (`expect_word_any_spanned` + standalone-`:` peek), split a word ending in `:`
    (len > 1) into name + consumed colon, then read the type — modelled on the glued
    `'T:` handling inside `src/parser.rs:3246` (`parse_optional_bound_bracket`). The
    split is unconditional in slot position — the sugar owns `:` there (corner: a
    type declared with a trailing-colon name is reinterpreted; see the Open
    Questions ruling, both corners pinned by unit tests), except that the split
    does not fire when the name half itself contains `:` — a qualified-name-shaped
    token (`q::Point:`, the degenerate `::`) falls through to the type resolver
    and dies as an unknown-type error, pinned by its own unit tests. The split skips
    `^`/`&`-led words (sigil-led slot names are illegal in every spelling — the
    spaced form dies in the reserved-name arm, `src/parser.rs:5566-5580`, and a
    skipped glued form dies in `resolve_type_or_apply` as ``unknown type `&x:```);
    do not chase. A token with exactly one colon, not trailing (`a:i64`), becomes the
    located hint error of R2; double-colon-qualified types are exempt, and a
    leading-colon token (`:i64`) falls through to the type resolver instead. On the
    extern path (the only route where type-variable tokens reach `parse_slot`), a
    glued type-variable token (`'T:`) now splits
    into a doc-only `'T`-named slot instead of today's unknown-type error — benign,
    R8 keeps it doc-only. The shared reader also newly legalizes glued spellings on
    extern and output slots (doc-only, R8/R9): add one glued exclusion test each.
  - Modify `src/parser.rs:3162` (`parse_worddef()`), insertion point
    `src/parser.rs:3211`: after `parse_terms` returns, extract input-slot names from
    `effect.inputs`, reject duplicates (R12), and prepend the desugar terms (R4/R5/R7)
    to `body`. Add a desugar helper fn adjacent to `parse_worddef` (e.g.
    `desugar_slot_locals(effect: &StackEffect, body: Vec<Term>, word_name: &str, enums: &[EnumDecl], span: Span) -> Result<Vec<Term>, String>`,
    `span` = the word definition's `name_span` (`src/parser.rs:3164`) — it spans the
    prepended terms and the R12 duplicate error), following existing private-helper
    style in the file; the mint freshness scan (R6) walks all `Bind` terms in `body`
    (mirror `rename_terms`, `src/ast.rs:3590`), includes `word_name`, the other
    input-slot names, previously-minted names, and reads variant names from
    `self.enums` (`src/parser.rs:3010`, `src/ast.rs:497/511`).
  - Modify `src/parser.rs:4542` (`parse_poly_slot()`): reject a word token followed by
    `:` (or a glued word ending in `:`) with the R11 message.
  - Create `tests/slot_locals.rs` (in `tests/`, modelled on `tests/phase3_locals.rs`'s
    `run_src`/`check_error`/`parse_error` harness): source-equivalence goldens and
    parse-diagnostic goldens listed under Exit Criteria.
  - Add parser unit tests in `src/parser.rs` `mod tests` (`src/parser.rs:8123`): the
    desugar shapes, name preservation (R13), mint bumping, extern/output exclusion.
  - Explicitly out of bounds for this phase: `src/check/**` and `src/ir/**` (no
    checker or IR code changes — F2), `src/ast.rs` (no changes this phase, doc
    comment is Phase 3), the lexer, `src/parser.rs:8006-8018` (Pipe arm), `parse_effect`
    (`src/parser.rs:4425`) and `effect_has_variable` (`src/parser.rs:4448`) internals,
    `parse_quotation_effect_rows` (`src/parser.rs:5874`), docs.
- **Entry Conditions**: Discovery settled (it is, probe-verified); tree green at
  baseline `403618f` under F5. No dependency on other phases.
- **Exit Criteria / Verifiable Artifacts** (goldens: source in → expected output, or
  source in → expected diagnostic; unit tests named `thing_condition_expected`):
  - `slot_sugar_all_named_matches_explicit_twin_expected` —
    `: f ( a: i64 b: i64 -- i64 ) a b add ;` on inputs `3 4` prints `7`, byte-identical
    to the `| a b |` twin (R3).
  - `slot_sugar_out_of_order_matches_explicit_twin_expected` —
    `: f ( a: i64 i64 -- i64 ) | b | a b add ;` on `3 4` prints `7`, identical to
    `: f ( i64 i64 -- i64 ) | a t | t | b | a b add ;` (R5).
  - `slot_sugar_mint_survives_user_slot_name_collision_expected` — body binding a
    user-named `__slot1` still compiles and prints the correct value (mint bumped, R6).
  - `slot_sugar_mint_bumped_on_sibling_mint_collision_expected` — cascading case:
    `( a: i64 i64 i64 -- i64 )` with a body binding `__slot1` yields two distinct
    mints (idx 1 and idx 2 bump in sequence) and compiles like its explicit twin (R6).
  - `slot_sugar_mint_bumped_on_sibling_slot_name_collision_expected` —
    `( __slot1: i64 i64 -- i64 )` compiles: the idx-1 mint is fresh against the
    effect's own named slot (R6).
  - `parse_slot_qualified_type_slot_unaffected_by_glued_hint_expected` — a
    `run_src` fixture with its own imports (`import: core::result r ;`) still
    resolves `( r::Result[i64 i64] -- )` as a type (R2 exemption; qualified names
    need an imported module — the single-file harness cannot resolve them).
  - `parse_worddef_trailing_colon_type_name_is_slot_name_expected` — with
    `type: Foo: … ;` declared, `( Foo: -- )` is a named-slot attempt that errors
    sharply on the missing type, and `( Foo: i64 -- )` (today: two slots, a
    `Foo:`-typed one and an `i64`) becomes one slot named `Foo` of type `i64` —
    both corners pinned by unit test (Open Questions ruling).
  - `parse_poly_slot_bound_attempt_keeps_bound_in_effect_error` — `'T :` and `'T:`
    in a poly effect keep the existing bound-in-effect message (R11 exemption).
  - `parse_quotation_row_name_attempt_unchanged_error` — a `x : i64` attempt inside a
    CONCRETE quotation-effect row (e.g. `( [ x : i64 -- i64 ] -- )`) still errors
    exactly as today with ``unknown type `x``` (R10 pin; poly quotation rows are
    covered by R11's reject instead).
  - `parse_extern_glued_named_slot_stays_doc_only_expected` and
    `parse_worddef_output_glued_named_slot_stays_doc_only_expected` — glued
    spellings on extern/output slots parse and stay doc-only (R8, R9).
  - `slot_sugar_glued_spelling_matches_spaced_expected` — `a: i64` and `a : i64` twins
    identical (R1).
  - `parse_worddef_slot_sugar_prepends_bind_in_slot_order_expected`,
    `parse_worddef_slot_sugar_top_contiguous_named_zero_mints_expected`,
    `parse_worddef_slot_sugar_out_of_order_mints_and_repushes_expected`,
    `parse_worddef_slot_sugar_mint_bumped_on_body_collision_expected`,
    `parse_worddef_slot_sugar_leaves_slot_names_populated_expected` — unit tests on
    the desugared `WordDef.body` (R4, R6, R7, R13).
  - `parse_slot_fully_glued_name_is_located_hint_error` — `( x:i64 -- )` errors with
    the space-after-`:` hint (R2).
  - `parse_worddef_duplicate_input_slot_name_is_error` — `( x : i64 x : i64 -- )`
    errors, naming `x` (R12).
  - `parse_poly_slot_named_is_rejected_with_located_error` — `( x : 'T -- )` and
    `( x: 'T -- )` produce the R11 message (R11).
  - `parse_extern_named_slots_stay_doc_only_expected` and
    `parse_worddef_output_slot_name_stays_doc_only_expected` — extern and output
    behaviour unchanged (R8, R9).
  - Regression: `parse_effect_without_global_clause_unchanged` (`src/parser.rs:12338`),
    `parse_slot_named_array_with_type_annotation_parses` (`src/parser.rs:13347`),
    Phase 0 `gcd`/`factorial` goldens — all pass unmodified (R13, R18).
  - Full green gate F5.
- **Parallelism**: SEQUENTIAL — first phase; Phases 2 and 3 both build on its desugar.
- **Relative Effort**: M — the desugar logic, spelling split, two rejects, unit tests,
  and goldens are all parser-local but touch a shared parse path and must preserve
  byte-identical no-name behaviour; roughly a week including review.
- **Difficulty**: `hard` — it modifies shared slot-parsing control flow
  (`parse_slot` serves extern and output slots too), introduces new desugar semantics
  with collision-safety logic, and carries the byte-identical regression guarantee.
- **Open Questions / Blockers**: None identified. (Duplicate-error wording is
  implementer's choice in house style, per Open Questions.)

### Phase 2: Semantics + diagnostics pins

- **Goal**: Every inherited checker/IR behaviour for slot names is pinned by a
  diagnostic or equivalence golden, so the sugar's semantics cannot drift.
- **Requirements Covered**: R14, R15, R16, R17
- **Scope**:
  - Extend `tests/slot_locals.rs` (created in Phase 1) with checker-behaviour pins
    using the `check_error` in-process harness and run-goldens via `run_src`.
  - No production-code changes are expected in this phase. If any pin fails, that is a
    Phase 1 defect: escalate and fix in the Phase 1 surface (`src/parser.rs`), never
    by adding checker code.
  - Explicitly out of bounds for this phase: all of `src/**` (zero production diffs),
    docs (Phase 3), and any new diagnostic wording (the pins assert *existing*
    inherited messages).
- **Entry Conditions**: Phase 1 committed — `tests/slot_locals.rs` exists and its
  Phase 1 goldens pass; the desugar is live in `parse_worddef`.
- **Exit Criteria / Verifiable Artifacts**:
  - `slot_local_named_like_word_is_callable_collision_error` —
    `: f ( add : i64 -- i64 ) add ;` errors "local `add` in f collides with the
    callable name `add`" (R14; `src/check.rs:1235`).
  - `slot_local_rebound_by_body_block_is_rebind_error` —
    `: f ( a : i64 -- i64 ) | a | a ;` errors "`a` is already bound in `f`" (R14;
    `src/check.rs:2936`).
  - `slot_local_named_like_variant_is_x12_parameter_error` — a slot named like an
    enum variant errors with the X12 "parameter ... collides with the variant name"
    wording (R14; `src/check/word_entry.rs:33-38`).
  - `slot_local_unused_named_slot_is_linear_leak_error` —
    `( x : ^i64 -- )` with a body that never consumes `x` fails the
    bound-but-unused check naming `x` (owned cells are always linear); a
    copy-typed unused slot — `array[i64 4]` is Copy — still compiles (R15;
    `src/check/engine.rs:557-570`).
  - `slot_sugar_never_fires_entry_arity_diagnostic_expected` — every sugar worddef in
    the golden corpus builds clean while a hand-written over-arity `| a b |` on a
    1-input word still errors with the entry-arity message (R16;
    `src/check/word_entry.rs:200-213`).
  - `slot_sugar_tail_call_matches_explicit_twin_expected` — a self-tail-recursive word
    using named slots prints byte-identical output to its explicit twin (R17;
    `src/ir/func_builder/calls.rs:100-135` path).
  - Full green gate F5 (all Phase 1 goldens still pass).
- **Parallelism**: SEQUENTIAL after Phase 1 (pins the semantics Phase 1 produces);
  PARALLEL with Phase 3 (disjoint files).
- **Relative Effort**: S — test-only pinning against a harness that already exists;
  a day or two of writing and running.
- **Difficulty**: `standard` — no concurrency, no migrations, no new surfaces; the
  security-sensitive linear-leak pin merely asserts existing behaviour.
- **Open Questions / Blockers**: None identified.

### Phase 3: Docs + bookkeeping

- **Goal**: The book and code comments match the shipped feature; the roadmap records
  it.
- **Requirements Covered**: R19, R20, R21, R22
- **Scope**:
  - Modify `src/ast.rs:2944` (doc comment above `StackEffect`/`TypedSlot`): replace
    the stale "a name is never written twice" wording with the worddef-input binding
    rule (R19).
  - Modify `docs/book/words.md`: add a named-slot-locals section (syntax, desugar
    rule, collision rules) and correct the stale shadowing section at
    `docs/book/words.md:83-99` — the checker wins (a local may NOT shadow a word);
    fix or replace the section's example, which fails today (R20).
  - Modify `docs/book/the-stack.md`: the effect-notation section mentions optional
    input slot naming (R21).
  - Modify `docs/roadmap/ROADMAP.md`: one bookkeeping line in the "Current status /
    next action" section (`docs/roadmap/ROADMAP.md:6`) (R22).
  - Explicitly out of bounds for this phase: any `src/**` behaviour change (the
    `ast.rs` edit is comment-only), any `tests/**` change, and per CLAUDE.md docs-lag
    caution: verify every book claim against the parser before writing it (no new
    syntax in examples that Phase 1 did not ship).
- **Entry Conditions**: Phase 1 committed (the syntax and desugar the docs describe
  exist and are golden-covered). No dependency on Phase 2.
- **Exit Criteria / Verifiable Artifacts**:
  - `words.md` contains the named-slots section; the shadowing section no longer
    claims locals shadow words and its example either compiles as shown or is
    corrected (inspection against the checker's actual behaviour, R20).
  - `the-stack.md` mentions optional input naming (R21).
  - `grep -n "never written twice" src/ast.rs` returns nothing; the new doc comment
    describes input-slot binding (R19).
  - ROADMAP line present (R22).
  - Full green gate F5 (comment-only `src` change must not break `cargo fmt`/clippy).
- **Parallelism**: PARALLEL with Phase 2 (both depend only on Phase 1; disjoint file
  footprints).
- **Relative Effort**: S — a handful of doc edits against already-shipped behaviour;
  a day including the docs-vs-parser verification pass.
- **Difficulty**: `standard` — documentation only.
- **Open Questions / Blockers**: None identified.

### Parallelism Summary

- Phase 1 first, alone (everything depends on the desugar).
- Phase 2 and Phase 3 can run concurrently after Phase 1: Phase 2 touches only
  `tests/slot_locals.rs`; Phase 3 touches only `docs/`, `src/ast.rs` (comment), and
  `ROADMAP.md`.

### Effort Summary

- Phase 1: M (`hard`)
- Phase 2: S (`standard`)
- Phase 3: S (`standard`)
- Total: M + S + S ≈ one M-sized slice with two small tails.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "parse and desugar core: slot-name spelling, Bind prepend, positional mints, rejects", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "semantics and diagnostics: checker-behaviour pins and equivalence goldens", "effort": "S", "difficulty": "standard" },
    { "phase": 3, "focus": "docs and bookkeeping: book chapters, TypedSlot doc comment, roadmap line", "effort": "S", "difficulty": "standard" }
  ]
}
```
