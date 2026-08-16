# Phase 4 Slice 6h: a raw array constructor, and `fill` demoted to a library word

Continues 6d/6e/6f/6g's shape: a gap the combinator-library dogfood surfaced, not the
library itself. `fill` is today the compiler's sole array constructor, a hardcoded
primitive, unreachable from a polymorphic body, and the root cause of a long-standing
measured performance defect that turned out not to be about arrays at all.

## Grounding facts

**`fill` is the sole array constructor, and its checks are fill-specific, not general
array-type invariants.** `check_array_word`'s `"fill"` arm (`src/check.rs:10226-10275`)
requires: a `Copy` seed element (`fill_of_linear_element_error`, `:10262`), no
bare-reference element (`constructed_reference_error`, `:10254` — the arm's own comment:
"a construction site the declaration-site rule cannot reach"), and a literal count in
`1..=u32::MAX` (`fill_count_not_literal_error`/`fill_count_out_of_range_error`). None of
this is enforced anywhere else: `is_copy(Type::Array(id, _))` (`:503`) just *derives* an
array's own Copy-ness from its element's, the same structural derivation a struct/enum
gets from its fields/variants — it is not a construction-time gate. Since `fill` has been
the only way to make an array, a linear-element or bare-reference-element array has never
existed, and nothing downstream (`collect_drop_targets`'s array arm, `:4361`, walks the
element's own drop target once, structurally; there is no per-index runtime disposal loop
anywhere in `ir.rs`) has ever needed to handle one.

**`fill` (and every other array word) is unreachable from a polymorphic body — a dispatch
omission, not a designed rejection.** `check_array_word` is called only from `check_term`
(`:8561`, the concrete/monomorphic path). `poly_term`/`poly_call_term` (`:5145-5626`) have
no array-word dispatch at all — confirmed by grep across the full poly-checking function
range: zero hits for `"fill"` or `check_array_word`. There is no explicit "`fill` inside a
polymorphic body" diagnostic anywhere; a poly body calling `fill` just falls through
`poly_call_term`'s dispatch chain (locals → `dup`/`over`/`swap`/`rot`/`drop`/`len` →
comparisons → `env` lookup → `poly_delegate_op`) to `unknown_word_error`.

**`fill`'s compile cost is a documented, unresolved, pre-existing defect — root-caused this
session, and it is not about arrays at all.** ROADMAP (line ~432) and
`docs/phase4-slice6a-spec.md:353` record it as superlinear (10k ~ 0.36s, 100k ~ 25s, 1M >
300s), attributed to "the array machinery," deferred as "a future-slice item." Measured
directly: a Sooth word body consisting of nothing but a flat chain of N `1 +` calls (zero
arrays) reproduces the identical scaling — 5k → 0.16s, 10k → 0.31s, 20k → 0.92s, 40k →
3.35s, 80k → 13.5s, the ratio climbing from ~2x to ~4x per doubling, converging on O(N²).
Instrumented every one of Sooth's own compiler stages (`driver::emit_ssa`:
`discover_closure`/`assemble_module`/`check`/`lower`/`emit`) at N = 20k/40k/80k: every
stage is fast and scales linearly (roughly 10–80ms total, across all five stages, at every
N). Dumped the generated `.ssa` text and timed the external `qbe` binary on it in
isolation, outside our own compiler entirely: 20k → 0.96s, 40k → 3.62s, 80k → 14.4s — this
is where essentially all the wall-clock time lives, at the same ~4x-per-doubling rate. Root
cause: QBE itself is quadratic (or worse) on one very large straight-line basic block.
`fill`'s own lowering (`lower_array_word`, `ir.rs:4100-4130`, "alloc + N unrolled stores")
is exactly that shape: one giant block with 2N–3N sequential instructions for an N-element
fill.

**The array-type spelling `[ 'T 'N ]` already exists in a signature, and a body-level
constructor can reuse it unambiguously.** `parse_field_type_expr` (`src/parser.rs:1680`)
already parses `[ elem count ]` (space-separated, no `;`) wherever a type is expected. At
the term level, `parse_term`'s `Token::LBracket` arm (`:2026`) is unconditional today — it
always starts a quotation literal, and its own comment states why that is currently safe:
"every type-position bracket reader is reached only from signature/type parsing, never
from `parse_term`." Critically, `;` has no arm anywhere in `parse_term`'s match — a bare
`;` inside `[ ... ]` today always falls to the `other => Err(...)` "unexpected token"
arm, so it is currently a parse error in every quotation literal, with no exceptions.
A lookahead for `;` before the matching `]`, at the exact point `parse_term` branches on
`Token::LBracket`, is therefore completely unambiguous against every existing use of `[`
— it can never collide with a real quotation literal, since one containing a bare `;`
does not compile today.

## Decisions

**D1 — the new primitive is a body-level term `[ Type ; Count ]`, not a named word.**
`Type` is a concrete type name (resolved the same way `parse_field_type_expr`/
`resolve_type_name_in_module` already resolve a struct field's type — read directly out of
the term, no unification) or, inside a polymorphic body, a bound type-variable name
matching the enclosing word's own signature (`'T`). `Count` is a literal integer in
`1..=u32::MAX` (mirroring `fill`'s existing range) or, inside a polymorphic body, a bound
length-variable name matching the enclosing word's own signature (`'N`) — whether that
second form is load-bearing for this slice or deferrable is an open question below.
Disambiguated from both the existing `[ 'T 'N ]` type-signature spelling and from an
ordinary quotation literal by requiring the `;` before the matching `]`, per the grounding
fact above: the parser adds one lookahead branch at `parse_term`'s existing
`Token::LBracket` arm, mirroring `quotation_type_ahead()`'s own lookahead style at the
type level.

**D2 — `[ Type ; Count ]` owns the construction-time gates `fill`'s own check currently
enforces, in both the concrete and polymorphic path.** The `Copy`-only and
no-bare-reference restrictions re-home from `check_array_word`'s `"fill"` arm onto this
term's own check — not duplicated onto `fill`'s rewritten body, which needs neither, once
`fill` is ordinary code that can only ever obtain an array through this term. This is
decided, not left open: a linear-element or bare-reference-element array stays
unconstructible. Reaching that case for the first time — and building the disposal/move
machinery it would need, which does not exist anywhere in `ir.rs` today — is not this
slice's job. Unlike `fill`'s existing single-path check, this gate must genuinely work in
*both* `check_term` and `poly_call_term` from day one, since reachability from a
polymorphic body is the whole point of this slice.

**D3 — contents are unspecified; lowering is `alloc_array` alone, no loop, no store.**
This is what actually closes the QBE-quadratic-blowup finding: the term lowers to exactly
one `Instr::Alloc`, O(1) regardless of `Count`. Whatever populates the array afterward
(D4's rewritten `fill`, or any future generic array word) does so with an ordinary runtime
`times` loop, so QBE only ever compiles a small, fixed-size loop body once — never N
unrolled instructions.

**D4 — `fill` becomes an ordinary Sooth-defined word**, e.g.:

```sooth
: fill ( 'T: Copy usize -- ['T 'N] )
  | v n | [ 'T ; n ] | arr |
  0 n [ | i | &!arr i &!> v ! ] times
  arr
;
```

reachable from a polymorphic body for free, since it is now just a poly word like any
other. The compiler-known `"fill"` arms in `check_array_word` and `lower_array_word` are
retired entirely. Every existing `fill` call site in the corpus is unaffected
syntactically — still spelled `v n fill` — since it becomes an ordinary `env` lookup.

## Open questions for the spec

- Where does `fill`'s rewritten definition live? `lib/combinators.sth` does not fit
  subject-wise; is there already a `core`-shaped library home for it, or does this slice
  need to create the first one?
- Does `Count` actually need to accept a bound length-variable name (`'N`) for *this*
  slice's own exit criterion? `fill`'s own rewrite only ever needs a plain `usize` local
  (`n`). Confirm whether supporting the `'N` sigil in `Count` is load-bearing here or a
  deferrable nice-to-have with no current consumer (YAGNI risk either way — name it
  explicitly rather than silently deciding).
- Slice 8b's `check_destructure_drop_guard` is the precedent for a single check called from
  both `check_term` and `poly_call_term` (D2's own shape). Confirm that is the right
  template here, or whether a cleaner shared helper is warranted.
- Re-verify the QBE finding empirically once implemented: re-run `docs/phase4-slice6a-spec.md`'s
  own 10k/100k/1M `fill` timings and confirm they are now flat/linear, closing that spec's
  "recorded on ROADMAP as a future-slice item" note for real.

## Out of scope

- Linear-element or bare-reference-element arrays (D2 keeps them unconstructible; the
  disposal/move machinery a real one would need does not exist and is not this slice's
  problem).
- `lib/arrays.sth`'s own fate (`sort`/`bin_search`) — separate, already-flagged decision,
  tied to slice 6g's landing, not entangled with this one.
- Runtime/heap allocation. This stays a stack-frame `Alloca`, exactly as `fill` already is
  — nothing here is `Vec`/Phase 6's `alloc`/`free`.

## Exit criteria

- `[ i64 ; 10 ]` (concrete) and `[ 'T ; n ]` (inside a polymorphic body, `'T`/`n` bound by
  the enclosing word) both compile and produce a well-typed, `Copy`-only array; a linear or
  bare-reference element type is a located error at the constructor, in both the concrete
  and polymorphic path.
- `fill` is an ordinary Sooth-defined word — no compiler-special-cased `"fill"` arm left in
  `check_array_word`/`lower_array_word` — reachable from a polymorphic body, with every
  existing `fill`-using golden in the corpus unchanged in observable behavior.
- `docs/phase4-slice6a-spec.md`'s superlinear-compile-cost numbers (10k/100k/1M) are
  re-measured and shown to be linear/flat, not quadratic, closing that pre-existing defect.
- A polymorphic word can construct its own array internally via `[ 'T ; n ]` where it
  previously could not (dogfooded by at least one such word, e.g. a minimal generic
  constructor that needs no caller-supplied scratch buffer).
