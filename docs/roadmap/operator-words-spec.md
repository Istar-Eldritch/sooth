# Operators as words (spec, delivered)

## Goal

Every symbolic arithmetic/comparison spelling is retired in favour of a word. No
symbolic spelling survives as an alias.

| Old | New | Old | New |
| --- | --- | --- | --- |
| `+` | `add` | `>=` | `gte` |
| `-` | `sub` | `<>` | `ne` |
| `*` | `mul` | `u=` | `ueq` |
| `/` | `div` | `u<` | `ult` |
| `=` | `eq` | `u>` | `ugt` |
| `<` | `lt` | `u<=` | `ulte` |
| `>` | `gt` | `u>=` | `ugte` |
| `<=` | `lte` | `u<>` | `une` |

Scope was table keys, `lib/core.sth` word names, call sites and docs. One
mechanism change was forced (R10).

## Rulings that outlive the migration

**R3 — which tests survive by name.** `qbe_name_distinct_operator_names_never_collide`
and `qbe_name_plus_and_minus_no_longer_collide` keep their symbolic fixtures:
`qbe_name` still sanitizes genuinely symbolic *user* word names (`~`, `?`,
`max-total`), which is now the only remaining case. `lex_negative_integer_is_int`
and the lexer token tests stand unchanged: the literal/word boundary did not
move, and a bare `-` is still a legal, now-unbound word token.

**R4 — two corpus words were renamed, not promoted.** `examples/modules_ops.sth`'s
`add` → `point-add`, `examples/vectors.sth`'s `sub` → `vec2-sub` (with
`examples/modules.sth`'s import/call sites and `tests/phase1.rs:333`'s REPL
mirror). Both files exist to exercise a *plain* user word: export, selective
import, per-module mangling. Left as-is they would have become builtin-name
overloads and changed the mechanism under test; that path is already covered by
`own_module_operator_overload_reachable_bare_in_multi_module`.

**R5 — `word_entry.rs`'s inline pair keeps its contrast.** Operator-named case is
`add`; the plain-named case became `bump`.

**R7 — the three keep-in-sync lists kept their shape.** `operators.rs::is_operator`,
`declarations.rs::BUILTIN_WORDS`, `resolve.rs::is_operator_dispatch_name`: values
changed, structure and comments did not. `qbe_name` was not removed.

**R8 — `>`-as-suffix and `>T` conversions untouched.** Only the bare comparison
word `>` retired. Conversion names, the `Point>` destructure suffix, the
eliminator path and the `&`-family sigils are unchanged.

**R10 — an operator decl now mangles like any other word.** The rename made four
dispatch names alphabetic, and `div` is a libc function. `resolve::resolve_modules`
used to skip per-module mangling for an operator-named decl in a single-module
closure, so a user's `div` emitted a bare strong `$div` and interposed libc's for
every shared library linked in. The exemption is gone; every decl mangles, one-file
closures included. The bare *call* still stays unrewritten (that is what
`check_operator`'s operand-type dispatch keys on) and reaches the decl via
`scoped_operator_overloads`, which now assembles candidates under `mangle(name, m)`
and no longer needs its `modules.len() < 2` bail. Alternative rejected: a
hand-maintained list of libc names.

**R6 — doc rewrite boundary.** Rewritten: `DESIGN.md`, `README.md`, all of
`docs/book/`, and any `docs/` snippet presented as currently valid syntax. Left
alone: `docs/roadmap/P*` phase briefs/specs and the completed-work briefs, which
are faithful records of what shipped and would be forged by a rewrite.

## Delivered shape

One atomic code phase (tables + prelude + R4 renames + whole corpus + embedded
Rust sources + R10), then a documentation-only phase. Splitting the code phase was
never viable: with no aliasing period, any intermediate commit has a red suite and
an uncompilable corpus.

`.sth` migration was scripted whole-token-exact (a character-level `sed` would
have corrupted `>i64` conversions, `Point>` suffixes, `&!x` and `max-total`), with
every comment hunk read by hand. Rust was hand-edited grep-driven, with the suite
as oracle.

## Verification notes worth keeping

- The operator grep over `*.sth` does **not** return empty by design: English and
  math prose inside `\` comments (`\ a + (b - a) * t`, `\ pc = 0`) survives.
  Confirm each hit sits after the `\`.
- The qbe baseline is not a sufficient net for accidental operator-overload
  promotion: it covers multi-module closures, while R10's exemption applied only
  to single-module ones, so the offending symbol never reached a baseline. `nm` on
  a one-file build declaring `: div ( V V -- V )` is the check.
- Baseline diff was confined to the two R4 renames plus R10's consequence in
  `leap.ssa` (`$.2e.` → `$.2e.__m0`). Instruction selection never saw the surface
  spelling.
- `check_add_word_dispatches_on_operand_type` deliberately does not pin
  `BUILTIN_TABLE`'s rows: the `"add" | "sub" | "mul"` coercion arm answers a
  homogeneous pair identically, so a table cut back to `i64` still passes. Row
  coverage stays `builtin_table_plus_has_a_row_per_numeric_type`'s job.

New guards: `check_symbolic_plus_is_unknown_word`,
`check_symbolic_comparison_is_unknown_word` (the `lib/` half),
`check_add_word_dispatches_on_operand_type`, `check_ueq_family_lowers_to_cmpop`.
R10's mangle-like-any-other-word mechanism is covered by
`check_operator_overload_is_visible_without_the_mangling_pass`
(`src/check/operators.rs`: a builtin-name overload like `add` is a real,
discoverable candidate, not silently dropped) and `tests/symbol_hijack.rs`
(a one-file `div` overload no longer hijacks the libc symbol).

## Accepted costs and residual debt

- `add sub mul div eq lt gt lte gte ne ueq ult ugt ulte ugte une` are now
  reserved-ish: a user word so named is an overload, not a fresh word. Accepted.
- Two book claims the compiler already rejects, pre-existing and left as found:
  `control-flow.md` (all of it), `getting-started.md:28-34` and `words.md:73,92,122`
  teach `if … else … end` (parser: `` `else` is not a word ``); `words.md:27-52`
  teaches that a local shadows a same-named word (rejected as a callable-name
  collision). These want their own item.

## Out of scope

Collapsing the three keep-in-sync lists; removing `qbe_name`; the `>`-suffix
destructure, `>T` conversions and `&`-family sigils; `mod and or xor not shl shr
max max-total .` (already words); rewriting historical phase docs.
