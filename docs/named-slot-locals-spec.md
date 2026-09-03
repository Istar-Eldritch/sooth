# Named effect-slot locals ("slot-name sugar")

**Status:** Implemented — all three phases landed on branch `quotation-locals-sugar`,
each phase reviewed and approved after fixes.

- Phase 1 (parse + desugar core): `58dece5`
- Phase 2 (semantics + diagnostics pins): `bb9ab68`
- Phase 3 (docs + bookkeeping): `1c48977`

Branch point: `403618f`. Discovery: `docs/named-slot-locals-discovery.md`.

## Problem Statement

Sooth word definitions bound entry locals with an explicit body block —
`: myfunction ( i32 i32 -- i32 ) | a b | a b + ;` — so names were written twice
conceptually: once by position in the effect, once in the body block. Users wanted
optional inline naming in the effect itself: `: myfunction ( a: i32 b: i32 -- i32 ) a b + ;`,
per-slot optional, composing with body-level `| ... |` blocks including out-of-order
naming (`( a: i32 i32 -- ) | b | ...`, where the named slot is deeper than an unnamed
one). Before this work, the spaced form parsed but the name was dead (doc-only except
the X12 variant-collision check); the glued form errored confusingly as
``unknown type `x:```; duplicate slot names were silently legal; and a poly-effect
name attempt produced the same confusing``unknown type `x```. The feature turns those
dead or confusing names into real, sharp, linear locals — one positional desugar rule,
no new checker or IR semantics.

## The desugar rule

One positional rule: **each named input slot becomes a local bound to its slot's
value; unnamed input slots stay on the stack in their original relative order.**

- In `parse_worddef` only, after the body is parsed, extract the input-slot names and
  prepend a `TermKind::Bind(names)` term to the body — indistinguishable from a
  user-written leading `| a b |`.
- `Bind` pops from the top, leftmost name deepest, so when an unnamed slot sits above
  a named one, the desugar binds from the top down to the deepest named slot, giving
  the unnamed slots in that group fresh minted names (`__slot{k}`, k = the unnamed
  slot's 0-based input index, bumped until fresh), then immediately re-pushes those
  locals in original order.
- When named slots are top-contiguous, the desugar emits zero mints — a plain `Bind`
  prepend. When all slots are unnamed, the parse path is byte-identical to today.

## Fixed constraints

- **F1** No lexer change: both spellings (`a :` spaced, `a:` glued) arrive as existing
  word tokens.
- **F2** No checker-code change and no IR change: collision, rebind, X12, entry-arity,
  leak, and lowering semantics are all inherited from the existing `Bind` handling.
- **F3** `| a b |` block semantics are untouched.
- **F4** QBE backend, `Ptr[T]` opaque, linear spine, `core`/`no_std` layering, no JIT —
  all load-bearing per CLAUDE.md; nothing here touches them.
- **F5** Green gate every phase: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Load-bearing rulings

- **Trailing-colon type names are reinterpreted.** A type declared with a
  trailing-colon name (e.g. `type: Foo: … ;`, legal before this work) is affected: the
  sugar owns `:` in slot position unconditionally. `( Foo: -- )` now reads as a
  named-slot attempt: the glued split fires and mints a slot named `Foo`, then the
  missing type surfaces as the resolver's ordinary `unknown type` error on the token
  that follows (the effect closer `--`, so the message names `--`, not `Foo`) — there
  is no dedicated diagnostic for this case. `( Foo: i64 -- )` (previously two slots)
  becomes one slot named `Foo` of type `i64`. Corpus exposure was zero
  (review-verified); such a type must be renamed to be usable bare. A registry-lookup
  guard was rejected — a fuzzy lookup would silently accept a typo like `i64:` as
  `i64`.
- **A glued name half that cannot be a body-block name is rejected.** `| … |` only
  ever binds `Word` tokens, so the glued split re-lexes the sliced name half and
  requires it to be exactly one `Word` token — a name half that re-lexes as a
  number (`1:`, `1.5:`, `-1:`) or as a lexer line comment (`\:` — a standalone
  `\` skips to end of line) can never be spelled or referenced by any body term;
  the glued split rejects it with a located error naming the offending spelling,
  rather than silently minting an unreachable local and swallowing the slot's
  argument.
- **`::`-qualified and leading-colon tokens are exempt from the split.** The glued
  split does not fire when the name half itself contains `:` — a qualified-name-shaped
  token (`q::Point:`, or the degenerate `::`) falls through to the type resolver and
  dies as an unknown-type error, rather than minting a slot named `:` or `q::Point`.
  Likewise a leading-colon token (`:i64`) falls through to the type resolver instead of
  producing R2's hint error (a leading colon names nothing, so the hint would suggest
  an empty name).
- **Poly-effect slot names are a sharp reject, not sugar.** A word followed by `:` in a
  polymorphic-effect slot position produces a located "slot names are not supported in
  polymorphic effects" error, on either side of the effect (an input or an output
  slot) and inside any quotation-effect row nested in a poly effect, whether that
  row is itself poly or concrete. Tokens containing `'` are
  exempt — `'T :`, `'T:`, and sigil-glued `&!'T:` keep the pre-existing located
  bound-in-effect error. A row in a *concrete* effect is unaffected — `unknown type`
  stands there, since the concrete slot reader has no R11 path.
- **The R11 reject does not fire inside an `impl:` target pattern.** `forbid_bounds`
  (set only while parsing an `impl:` target, `src/parser.rs:4067`) suppresses the
  reject at its one call site (`src/parser.rs:4930`): that route shares the same
  slot reader for a bare/concrete target pattern that has no slot-name concept at all,
  and the token immediately after the target can legitimately be the impl body's own
  leading `:` — without the exemption, every `impl:` declaration would misparse. This
  is a parser-internal exemption invisible to the sugar's own diagnostics, not a
  fourth spelling exempt from R11.

## Scope boundaries (still true)

- Output-slot names stay doc-only (no binding, no duplicate check) *in a monomorphic
  effect only* — a named output slot in a polymorphic effect hits R11's blanket
  reject exactly as a named input slot does.
- The duplicate-slot check (R12) scans input slots only: the same name on an input
  slot and an output slot is legal (`( x: i64 -- x: i64 )`), since only two input
  slots sharing a name are actually ambiguous to bind.
- The mint-freshness scan (R6) is bounded by what the parser can see at desugar time:
  the word's own name, its own other slot names, every PLAIN (non-generic) enum
  variant in the module (the whole-file prepass registers these before any word
  body is parsed, so declaration order does not matter), and every name the body
  itself binds (including inside nested quotations). It cannot see other
  top-level callables -- words register at check time and struct constructor
  names are not in the scan either -- nor GENERIC-enum variants, which register
  in a separate later prepass. The generic-enum gap is harmless: a generic
  enum's variant name does not collide with a local at all (verified: `| Vv |`
  compiles under `type: G['T] | Vv 'T | ;`, where the plain-enum twin
  `type: E | Vv | ;` errors), so there is nothing for the scan to protect
  against there. A user-declared callable — a word or a struct constructor — named
  exactly like the mint the desugar would otherwise choose (e.g. a word named
  `__slot1` in a word whose first unnamed slot would mint `__slot1`) is not
  excluded by the freshness scan and collides at the
  checker's ordinary callable-collision guard instead — an accepted edge, consistent
  with the `__`-namespace wart below, not a defect.
- Extern declarations parse with names intact but perform no binding (no Sooth
  body/frame).
- Trait impl member bodies are structurally excluded: `parse_impl_member_body` parses
  bodies without an effect and grounds slots `name: None`.
- Reserved `__`-namespace hardening is a pre-existing wart, not addressed here.
- No LSP/editor or `tree-sitter-sooth` changes were needed (highlighting-only grammar).

## Where it lives

See the three commits above for the parser desugar and spelling support
(`src/parser.rs`), the diagnostic/equivalence goldens pinning inherited checker/IR
behaviour (`tests/slot_locals.rs`), and the docs/bookkeeping updates (`docs/book/words.md`,
`docs/book/the-stack.md`, `src/ast.rs` doc comment, `docs/roadmap/ROADMAP.md`).
