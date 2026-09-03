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
  named-slot attempt and errors sharply on the missing type; `( Foo: i64 -- )`
  (previously two slots) becomes one slot named `Foo` of type `i64`. Corpus exposure
  was zero (review-verified); such a type must be renamed to be usable bare. A
  registry-lookup guard was rejected — a fuzzy lookup would silently accept a typo
  like `i64:` as `i64`.
- **`::`-qualified and leading-colon tokens are exempt from the split.** The glued
  split does not fire when the name half itself contains `:` — a qualified-name-shaped
  token (`q::Point:`, or the degenerate `::`) falls through to the type resolver and
  dies as an unknown-type error, rather than minting a slot named `:` or `q::Point`.
  Likewise a leading-colon token (`:i64`) falls through to the type resolver instead of
  producing R2's hint error (a leading colon names nothing, so the hint would suggest
  an empty name).
- **Poly-effect slot names are a sharp reject, not sugar.** A word followed by `:` in a
  polymorphic-effect slot position produces a located "slot names are not supported in
  polymorphic effects" error. Tokens containing `'` are exempt — `'T :`, `'T:`, and
  sigil-glued `&!'T:` keep the pre-existing located bound-in-effect error. Quotation
  effect rows get no name path and no new reject (today's errors stand there).

## Scope boundaries (still true)

- Output-slot names stay doc-only (no binding, no duplicate check).
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
