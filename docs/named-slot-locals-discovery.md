# Discovery: named effect-slot locals ("slot-name sugar")

Status: discovery complete (conversation + 3 probe/paper rounds, 2026-09-02).
Worktree: `quotation-locals-sugar` @ 403618f. Output spec: `docs/named-slot-locals-spec.md`.

## Problem statement

Sooth word definitions bind entry locals with an explicit body block:

```sooth
: myfunction ( i32 i32 -- i32 ) | a b | a b + ;
```

The names are written twice conceptually — once by position in the effect, once in
the body block. We want optional inline naming in the effect itself:

```sooth
: myfunction ( a: i32 b: i32 -- i32 )  a b + ;
```

Naming is optional per slot; unnamed slots stay on the stack. Mixed forms compose
with body-level `| ... |` blocks, including out-of-order naming where the named
slot is deeper than an unnamed one:

```sooth
: myfunction ( a: i32 i32 -- i32 ) | b | a b add ;   \ a = slot 1, b = slot 2
```

## The desugar rule (settled)

One positional rule: **each named input slot becomes a local bound to its slot's
value; unnamed input slots stay on the stack in their original relative order.**

Implementation shape (all verified legal today by paper probes):

- Parse the effect as usual (`TypedSlot.name` already stores spaced `name : type`).
- In `parse_worddef` only, after `parse_terms` returns (src/parser.rs:3211), extract
  the input-slot names and prepend a `TermKind::Bind(names)` term to the body —
  indistinguishable from a user-written leading `| a b |` (built by the Pipe arm,
  src/parser.rs:8008-8019).
- `Bind` pops from the top, leftmost name deepest (src/check/terms.rs:148), so when
  an unnamed slot sits *above* a named one, the desugar binds from the top down to
  the deepest named slot, giving the unnamed slots in that group fresh minted names,
  then immediately re-pushes those locals in original order:

  ```sooth
  \ ( a: i64 i64 -- i64 ) | b | a b add
  : f ( i64 i64 -- i64 ) | a t | t | b | a b add ;   \ verified: runs, prints 7
  ```

  When named slots are top-contiguous (both simple examples above) the desugar emits
  zero mints — a plain `Bind` prepend.

## Settled decisions

1. **Spelling.** Support both the glued trailing-colon form `a: i64` and the spaced
   form `a : i64`. Today only the spaced form parses (spaced → `TypedSlot{name:
   Some}` at src/parser.rs:5593-5597; glued errors `unknown type`x:``). Glued
   support is a parser-level split of a word ending in `:` — precedent:
   `parse_optional_bound_bracket` already handles glued `'T:` (src/parser.rs:~3277).
   No lexer change. The fully-glued form `a:i64` (one word) stays an error but gets
   a sharp located hint ("put a space after `:`") instead of today's confusing
   `unknown type `a:i64``.
2. **Scope: word definitions, input slots, monomorphic effects only.**
   - `extern:` shares `parse_effect` (src/parser.rs:3558/4425) but has no Sooth
     body/frame to bind — excluded; its slot names remain doc-only.
   - Output slots already accept names today (src/parser.rs:4428, live probe: builds
     clean) — they stay doc-only; no binding semantics this feature.
   - Poly effects have no name path at all (`parse_poly_slot`, src/parser.rs:4542,
     returns `RawTy`; `PolyType` has no name field). `( x : 'T -- )` errors today as
     a confusing `unknown type`x``. Sugar is mono-only; instead of leaving that
     error, `parse_poly_slot` gets a located reject: a word followed by `:` in a
     poly slot position is always an attempted slot name (checked: no legal poly
     slot form has `:` there) → "slot names are not supported in polymorphic
     effects". Quotation effect rows (`parse_quotation_effect_rows`, :5874) excluded
     likewise.
   - The concrete-effect routing guarantee (R1/R2/R15, src/parser.rs:3188) holds:
     `effect_has_variable` (:4448) is unaffected; with no names the parse path is
     byte-identical.
3. **Keep, don't strip, `TypedSlot.name`.** The desugar derives the `Bind` from the
   names but leaves them in the slots. Only live consumer today is the X12
   variant-collision check (src/check/word_entry.rs:33-38), which keeps firing with
   its "parameter" wording; the existing pin at src/parser.rs:13347
   (`name == Some("array")`) survives unchanged. The `TypedSlot` doc comment
   (src/ast.rs:2944-2947, "a name is never written twice") is now stale and gets
   updated.
4. **Duplicate input slot names** (`( x : i64 x : i64 -- )`) are silently legal
   today; under the sugar they would surface as a confusing `rebound_local_error`.
   Reject at parse level in `parse_worddef` with a located duplicate-slot-name error
   (input slots only; output names stay doc-only and unchecked).
5. **Minted names for out-of-order desugars.** Positional mints `__slot{k}` (k =
   slot index), fresh-scanned at desugar time against every name bound anywhere in
   the body (walk all `Bind` terms, nested included), the word's own name, the
   module's PLAIN (non-generic) enum variant names (cheap: the whole-file prepass
   already registers these before any body parses, so declaration order does not
   matter; generic-enum variants register in a later, separate prepass and are not
   in the scan -- harmless, since a generic enum's variant name does not collide
   with a local at all: `| Vv |` compiles under `type: G['T] | Vv 'T | ;`, where the
   plain-enum twin errors),
   the effect's other input-slot names, and every name minted earlier in the same
   desugar (deepest-first; amended in review round 2 to match spec R6);
   bump k on collision. The checker's rebind and X12 checks remain backstops. There
   is no unspellable charset (delimiters are only `; ( ) | [ ]`; even `| a:b |`
   binds — live-verified), so collision safety must come from the scan, not the
   spelling. Exposure class is identical to the existing `{name}__inl{uid}` inliner
   scheme (user-spellable, live collisions caught sharply) — an accepted,
   pre-existing wart, not a new one. Minted locals are consumed exactly once by
   their re-push, so on a well-formed path where the mint binds and stays a plain
   local, it never renders in diagnostics; the desugared `Bind` can never underflow
   (arity is known from the effect). The freshness scan itself cannot see this wart:
   it has no visibility into other top-level callables (words register at check
   time, after the parser has already desugared; struct constructor names are not
   in the scan either), so a user-declared callable — a word or a struct
   constructor — named exactly like the chosen mint is not excluded and the mint's
   name *does* then render, in the checker's
   ordinary callable-collision error — an accepted edge, not a scan defect to fix
   (test: `slot_sugar_mint_collides_with_user_callable_named_like_mint_error`,
   `tests/slot_locals.rs`).
6. **Collision/rebind semantics are inherited, not invented.** Slot names obey the
   local rules: callable-name collision (`callable_local_error`, src/check.rs:1235 —
   a local may NOT shadow a word/builtin; note docs/book/words.md:83-99 claims the
   opposite and its own example fails today — the checker wins, and since the docs
   phase rewrites this chapter anyway it fixes that stale section), rebind vs body
   locals (`rebound_local_error`, src/check.rs:2936 — e.g. effect names `a`, body
   has `| a |` → sharp error), X12 variant names.
7. **Binding takes ownership — an intentional behavior change** for previously
   doc-only names: `( x : ^i64 -- )` with a body that never consumes `x`
   now fails the linear use-exactly-once check (bound-but-unused, `Scope::leave`,
   src/check/engine.rs:557-570) instead of compiling silently. That is the point of
   naming: the sharp leak diagnostic names the slot. (Corrected in review round 1:
   the original example used `array[i64 4]`, but arrays of Copy elements are Copy —
   no leak; the leak case needs an owned cell like `^i64`.)
8. **Entry-arity diagnostic is inherited for free**: a leading `Bind` gets the
   dedicated "locals bind N value(s), but only M input(s) are declared" check
   (src/check/word_entry.rs:207-213); the desugar's Bind has exactly the declared
   input count, so it can only fire for hand-written blocks — but the IR's
   leading-Bind consumption (`lower_self_tail_combinator`, src/ir/func_builder/
   calls.rs:100-135) handles a desugar Bind identically.
9. **tree-sitter-sooth needs zero changes** (highlighting-only grammar, no slot
   rule; `a : i64` already parses as atoms; glued `x:` glues into one word node,
   highlighting unaffected). Docs DO need updates: docs/book/words.md (new named-
   slots section + fix the stale "locals shadow words" section) and the effects
   part of docs/book/the-stack.md.

## Verified evidence (probe round, 2026-09-02; all probes /tmp-only, tree clean)

| Probe | Result |
|---|---|
| spaced `( x : i64 -- )` worddef + extern | parses, checks, builds, lowers today; name dead except X12 |
| glued `x: i64` / `x:i64` | `error: unknown type`x:`` / `` `x:i64`` |
| poly `( x : 'T -- )` / `( x: 'T -- )` | `error: unknown type`x`` / `` `x:`` (no name path) |
| `| a t | t | b | a b add` (mint+repush) | checks, builds, runs, prints 7 |
| plain twin `| a b | a b add` | identical output (7) — the golden equivalence |
| user-spelled `a__inl0`, `a:b`, `x__slot` | all bind/run today (no reserved namespace) |
| live `__inl{uid}` collision | sharp rebind error, but span points into library source (pre-existing wart) |
| `\| a \| \| a \|` | `error:`a` is already bound in `f`...` (rebound_local_error) |
| `\| add \|` / book's own shadow example | `error: local ... collides with the callable name ...` — checker wins; book stale |
| duplicate `( x : i64 x : i64 -- )` | silently legal today |
| output-side `( -- x : i64 )` | parses today, semantically nothing |
| slot named like an enum variant | X12 fires: "parameter `Equal` ... collides with the variant name" |

## Phase sketch (3 phases, one implement/review/commit cycle each)

1. **Parse + desugar core** — spelling support in `parse_slot` (glued trailing-colon
   split, spaced already there, fully-glued hint error), worddef-only desugar
   (extract input names, duplicate-slot reject, prepend `Bind`, positional
   `__slot{k}` mint + re-push with freshness scan), parser unit tests for every
   shape, source-equivalence goldens (sugar vs explicit block), regression corpus
   green (`parse_effect_without_global_clause_unchanged`, phase0 gcd/factorial).
2. **Semantics + diagnostics** — checker behaviors pinned by tests: entry-arity,
   callable collision, rebind (effect name vs body block), X12 variant wording,
   linear-leak for unused named slots; poly-slot sharp reject; extern/output
   exclusion tests.
3. **Docs + bookkeeping** — docs/book/words.md named-slots section (+ fix stale
   shadowing section), the-stack.md effects touch-up, `TypedSlot` doc comment
   (src/ast.rs:2944), roadmap bookkeeping line.

## Non-goals

Output-slot binding; poly-effect slot names; extern sugar; lexer changes; reserved
`__`-namespace hardening (pre-existing wart, out of scope); LSP/editor work;
changes to `| a b |` block semantics.
