# Operators as words (brief)

Retire every symbolic arithmetic/comparison spelling in favour of a word. `+ - * /`
become `add sub mul div`; `= < > <= >= <>` become `eq lt gt lte gte ne`; the
`u`-prefixed unsigned-compare intrinsics `u= u< u> u<= u>= u<>` become `ueq ult ugt
ulte ugte une`. No symbolic spelling survives as an alias; this is a rename, not an
addition.

## Recon (measured against `main`, 2026-08-18)

1. **The symbolic surface is small and already table-driven.** `+ - * /` are
   `BUILTIN_TABLE` rows dispatched by `check_operator` (`src/check/operators.rs:86-101,190`).
   `= < > <= >= <>` are **not** table rows: Slice 10c moved them to `lib/core.sth:17-22`,
   each a one-line `inline` word delegating to its `u`-prefixed primitive
   (`u=`/`u<`/... , themselves table rows dispatched at `operators.rs:275` via
   `crate::ir::CmpOp`, `src/check/builtins.rs:132-137`). `mod and or xor not shl shr max
   max-total .` are already bare words and are unaffected.

2. **The lexer has zero special-casing for any of these symbols.** `src/lexer.rs:74`
   tokenizes on whitespace/delimiters only; `+`, `-`, `<`, etc. already lex as ordinary
   `Word` tokens, identically to `mod` or `dup`. `is_int_literal`/`is_float_literal`
   (`lexer.rs:26-63`) strip a leading `-` only when digits follow, so a bare `-` token
   never collides with a negative literal (`-5` glues into one token with no space,
   confirmed by the existing `lex_negative_integer_is_int` test). **No lexer change is
   needed**; this is purely a rename of table keys and lib word names.

3. **Three hand-maintained "keep in sync" lists carry the full symbol set today**, each
   with an explicit comment warning it must be updated when the operator set changes:
   `check_operator::is_operator` (`operators.rs:86-101`), `check::declarations::BUILTIN_WORDS`
   (`declarations.rs:63-99`), and `resolve::is_operator_dispatch_name`
   (`resolve.rs:82-110`, which also documents *why* the six lib-word comparison names stay
   listed even though they're no longer table rows: unmangled bare-name dispatch, and
   membership in the self-tail-call exclusion set). All three are rename targets, not
   structural changes.

4. **`check/poly.rs:718`** gates the comparison six by name for poly-call operand
   dispatch (see the standing note that generic calls bypass operand guards elsewhere in
   this codebase — this particular site already covers them, just needs the name list
   updated).

5. **`qbe_name` sanitizes non-alphanumeric word names into safe QBE symbols**
   (`backend/qbe.rs`, exercised by `qbe_name_distinct_operator_names_never_collide` and
   `qbe_name_plus_and_minus_no_longer_collide`). Renaming to plain-word spellings makes
   this sanitization moot for the renamed operators (a word like `add` needs no escaping)
   but does not require removing `qbe_name` itself, since it still sanitizes real
   symbolic word names like `max-total`'s hyphen or arbitrary user words such as `~`/`?`.

6. **`>` remains reserved outside this rename.** The character is separately used as a
   struct/variant destructure suffix (`Point>`) and inside receiver-directed accessor
   syntax (`&hp` family, unrelated sigil). Only the *bare* comparison word `>` retires;
   the destructure-suffix mechanism is untouched and must not be confused with it during
   implementation (`resolve.rs:176-240`, `check/poly.rs:1168-1173` already treat `>` as
   two distinct roles at different call sites).

7. **Corpus footprint:** 40 `.sth` files across `examples/`+`lib/` contain at least one
   symbolic operator token; 2 Rust test files construct source strings using the
   unsigned-compare intrinsics by name. `lib/core.sth`'s six comparison words are the
   only *definitions* that need re-spelling (both their own name and their internal
   `u<`-etc. call); every other `.sth`/test site is a call-site rewrite.

8. **Documentation footprint is wide but mostly historical.** 17 files under `docs/`
   mention one or more of these symbols, the bulk being phase-completion narration in
   `docs/roadmap/P0`–`P8-*.md` describing what already shipped, plus a few architecture
   briefs. Per this project's own convention, ROADMAP/DESIGN documents current design
   only, not history — historical phase files narrating already-shipped work are not
   candidates for rewrite just because they use the old spelling in a quoted snippet.
   `DESIGN.md` itself needs checking for any live code example using the old spellings.

## Decisions (settled here, not reopened by the spec)

1. **No aliasing period.** Both spellings never coexist; every call site (compiler
   source, `lib/`, `examples/`, tests, doc examples showing current syntax) moves in the
   same change. Per the user's own framing: drop the symbols outright.

2. **Exact name table** (final, not to be relitigated by the spec):

   | Old | New | Old | New |
   | --- | --- | --- | --- |
   | `+` | `add` | `u=` | `ueq` |
   | `-` | `sub` | `u<` | `ult` |
   | `*` | `mul` | `u>` | `ugt` |
   | `/` | `div` | `u<=` | `ulte` |
   | `=` | `eq` | `u>=` | `ugte` |
   | `<` | `lt` | `u<>` | `une` |
   | `>` | `gt` | | |
   | `<=` | `lte` | | |
   | `>=` | `gte` | | |
   | `<>` | `ne` | | |

3. **The three "keep in sync" lists (`operators.rs`, `declarations.rs`, `resolve.rs`)
   keep their current shape**; this is a value rename inside each, not a restructuring.
   Do not use this slice to collapse them into one source of truth — that's a separate,
   unrelated refactor and out of scope.

## Open questions for the spec

1. **Golden test names.** Several unit tests are named after the symbol they exercise
   (`qbe_name_distinct_operator_names_never_collide`, assertions matching on `w == "+"`,
   etc., per `CLAUDE.md`'s `thing_condition_expected` convention). The spec should decide
   whether test names get re-derived from the new word spelling or whether the existing
   names are left as historical labels for what they test.

2. **Doc rewrite boundary.** Given Recon 8, which of the 17 `docs/` hits are "current
   design description" (rewrite) versus "phase history" (leave as-is)? `DESIGN.md` and
   any *currently accurate* code snippets are the clear rewrite set; the spec should draw
   the line explicitly rather than leave it to whoever implements.

3. **Migration mechanics.** 40 `.sth` files plus compiler-internal Rust string literals:
   is the call-site rewrite scripted (e.g. a whole-word token-boundary sed per mapping,
   run once, then `cargo test` as the check), or hand-edited? Given the mapping is a
   fixed, non-overlapping table and every old spelling already lexes as a standalone
   `Word` token, a scripted pass is low-risk, but the spec should say so rather than
   leave it implicit.

## Out of scope

- Any structural change to `is_operator_dispatch_name`/`BUILTIN_WORDS`/`is_operator`'s
  three-list duplication (Decision 3).
- The struct/variant destructure-suffix `>` and the `&`-family accessor sigils (Recon 6).
- `mod and or xor not shl shr max max-total .`: already words, unaffected.
- Historical phase-completion doc narration (Recon 8, pending OQ2's line-drawing).

## Ready to spec?

Yes. The rename surface is fully enumerated (Recon 1, 3), the one real risk (lexer
literal/operator collision) is checked and clear (Recon 2), and the only genuine
decisions left are documentation scope and migration mechanics, both named as open
questions above.
