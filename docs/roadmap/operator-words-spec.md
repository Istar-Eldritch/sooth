# Operators as words (spec)

## Goal

Retire every symbolic arithmetic/comparison spelling, replacing each with a word,
per the brief's Decision 2 table. No symbolic spelling survives as an alias. This
is a rename of table keys, `lib/` word names and call sites; no lexer, parser,
checker or lowering *mechanism* changes.

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

## Recon deltas (verified against `main` at `80de583`, 2026-08-18)

The brief's Recon 1-7 all hold: `operators.rs:86-101`, `declarations.rs:63-99`,
`resolve.rs:85-110`, `poly.rs:718`, `builtins.rs:132-137`
(`COMPARISON_PRIMITIVES`), `lib/core.sth:17-22`, and the lexer's indifference to
these characters (`lexer.rs:26-63,74`). Three findings the brief did not have:

1. **Two corpus words already bear a new spelling, and the rename silently
   changes what they are.** `examples/modules_ops.sth:8` declares
   `: add ( Point Point -- Point )` and `examples/vectors.sth:9` declares
   `: sub ( Vec2 Vec2 -- Vec2 )`. Post-rename both names are builtin operator
   names, so each declaration stops being a plain user word and becomes a
   Slice 8a phase-2 *overload of a builtin*: `resolve::is_operator_dispatch_name`
   would now match them and skip per-module mangling
   (`resolve.rs:342`, today's emitted `$add__m2` in
   `tests/qbe_baseline/modules.ssa`), and `examples/modules.sth:8`'s
   `import: ops | len2 add | ...` would selectively import a name the rewrite
   pass refuses to mangle. See R4.

2. **`docs/book/` is live user documentation, not phase history.** The brief's
   Recon 8 counted 17 `docs/` hits and named only `DESIGN.md` as a rewrite
   candidate. Nine of those hits are `docs/book/*.md` (the tutorial: 44 symbol
   occurrences in `numbers.md`, 19 in `the-stack.md`, 17 in `control-flow.md`,
   14 in `getting-started.md`, 11 in `words.md`, 6 in `move-by-default.md`, plus
   `preface.md` and `why-this-works.md`), all of it teaching current syntax. It
   is in the rewrite set. See R6.

3. **One Rust test pair loses its contrast.** `src/check/word_entry.rs:705`
   declares `: + inline ( A A -- i64 )` and `:716` declares
   `: add inline ( A A -- i64 )`: an operator-named inline word versus a
   plain-named one. A literal rename collapses both to `add`. See R5.

Corpus footprint, re-measured: **38 tracked `.sth` files** carry at least one
symbolic operator token (36 under `examples/`, 2 under `lib/`, plus any untracked
sibling work present at implementation time, e.g. `lib/binary_search.sth`,
`lib/uart_mmio.sth`). No `.sth` string literal contains a bare operator token;
8 files carry operators inside `\` comments.

## Rulings

**R1 (OQ3, migration mechanics for `.sth`): scripted, token-exact, then reviewed
by diff.** The mapping is fixed and non-overlapping and every old spelling
already lexes as a standalone `Word` token, so the rewrite is a whitespace-token
split with an exact-match table lookup per token, applied to every `.sth` file in
the tree. Constraints:

- Exact whole-token match only. A regex/`sed` character substitution is
  forbidden: it would corrupt `>i64`-family conversions, `Point>` destructure
  suffixes, `&!x` setters, `->`-ish prose and `max-total`.
- Comment (`\`) lines are rewritten too, since they narrate the code beside
  them, but every comment hunk in the resulting diff is read by hand: prose using
  `>` or `-` as English punctuation must not be mangled.

**R2 (OQ3, migration mechanics for Rust): hand-edited, grep-driven, suite as
oracle.** Rust sources are *not* scripted. A token pass over `.rs` files cannot
distinguish a sooth source string from `"/"` in a path join, `"-"` in a CLI flag
or `=` in a diagnostic. The edit set is enumerated by grep in two classes:
(a) quoted symbol literals `"+" "-" "*" "/" "=" "<" ">" "<=" ">=" "<>" "u=" ...`
(~110 hits across 20 files, concentrated in `operators.rs`, `declarations.rs`,
`resolve.rs`, `builtins.rs`, `calls.rs`, `qbe.rs`); (b) embedded sooth program
text inside string literals (lines containing a `:` word definition, `dup`,
`| a b |` etc.).
The test suite is a strong oracle for both: a missed table key fails a dispatch
test, and a missed embedded operator fails as ``unknown word `+` ``.

**R3 (OQ1, test names): rename a test whose subject is the retiring spelling;
keep a test whose subject is a mechanism.** Concretely:

- Rename: assertions and test names naming the surface operator being retired,
  where the new spelling is the thing under test (e.g. `parser.rs:4059,4093,4577`
  `w == "+"` assertions keep their host test names but the literal changes;
  `tests/phase4_slice10c_primitives.rs:270`'s name list changes in place).
- Keep: `qbe_name_distinct_operator_names_never_collide` and
  `qbe_name_plus_and_minus_no_longer_collide` (`backend/qbe.rs:1413,1455`).
  `qbe_name` still sanitizes genuinely symbolic user word names (`~`, `?`,
  `max-total`'s hyphen), which is what these two test; their fixtures keep the
  symbolic inputs, because those inputs are now *user* words rather than
  builtins, which is exactly the surviving case. The second test's name mentions
  `plus`/`minus` as a historical regression label and stays.
- Keep: `lex_negative_integer_is_int` and the `lexer.rs:263,292` token tests.
  They assert the lexer's literal/word boundary, which is unchanged, and a bare
  `-` remains a legal (now unbound) word token.

**R4 (Recon delta 1): rename the two colliding corpus words rather than let them
become builtin overloads.** `examples/modules_ops.sth`'s `add` becomes
`point-add`; `examples/vectors.sth`'s `sub` becomes `vec2-sub`. Rationale: both
files exist to exercise a *plain* user word (export/selective-import by name,
per-module mangling, componentwise field arithmetic). Letting them drift onto the
builtin-overload path changes the mechanism under test, duplicates coverage that
`src/check/declarations.rs:2252` and `tests/phase0.rs:3818` already own for
builtin-name overloads, and routes a selectively imported name through a rewrite
pass that deliberately declines to mangle it. Paired sites that must move with
them: `examples/modules.sth:8` (`import: ops | len2 add |` and the qualified call
sites), `examples/vectors.sth`'s `span` body, and `tests/phase1.rs:333` (the REPL
mirror of `vectors.sth`'s `sub`).

**R5 (Recon delta 3): preserve the `word_entry.rs` contrast.**
`src/check/word_entry.rs:705` becomes `: add inline ( A A -- i64 )` (the
operator-named case); `:716`'s word is renamed to a spelling that is *not* a
builtin name (`bump`), so the pair still contrasts operator-name inline dispatch
against plain-name inline dispatch. Do not delete either test.

**R6 (OQ2, doc rewrite boundary).** Rewrite:

- `DESIGN.md` (40 occurrences) — every live code example and every prose
  reference to the operator set as *current* design.
- `README.md` (6 occurrences).
- `docs/book/*.md` — all nine files (Recon delta 2). This is current-syntax
  teaching material; a book that does not compile against the language is worse
  than no book.
- Any `.sth` snippet elsewhere in `docs/` that is presented as *currently valid*
  syntax rather than as a record of what a past phase shipped.

Leave as-is:

- `docs/roadmap/P*/**` and `docs/roadmap/P*-*.md` phase briefs/specs/completion
  narration. These record what shipped at a point in time; per this project's
  convention roadmap docs state design, and a historical snippet quoting `+` is a
  faithful record. Rewriting them would forge history.
- `docs/check-modularisation-*.md`, `docs/ir-modularisation-*.md`,
  `docs/repl-ux-spec.md`, `docs/concurrency-implementation-notes.md`,
  `docs/dependency-management.md`: completed-work briefs/specs in the same
  category.
- This brief and this spec, which must keep both spellings to be readable.
- `CLAUDE.md`: zero occurrences, no edit.

**R7: no structural change.** The three keep-in-sync lists
(`operators.rs::is_operator`, `declarations.rs::BUILTIN_WORDS`,
`resolve.rs::is_operator_dispatch_name`) keep their exact shape and their
explanatory comments; only the string values change (Decision 3). Comment text
inside them that names a symbol (`resolve.rs:82`'s "every other module's bare
`=`", `declarations.rs:92-98`'s "a trailing `<` inside a user's own `Vec2 <`")
is re-spelled with the new names so the comment still describes the code.
`qbe_name` is not removed (Recon 5).

**R8: `>`-as-suffix and the `>T` conversions are untouched.** Only the bare
comparison word `>` retires. `conversion_target_name`/`is_conversion_name`
(`operators.rs:62`, `declarations.rs:118`), the `Point>` destructure suffix
(`resolve.rs:176-240`), the eliminator path (`poly.rs:1168-1173`) and the
`&`-family sigils keep every current behaviour. `declarations.rs:112-116`'s
doc comment ends "Bare `>` is the comparison operator, and is in the list" —
update the sentence to name `gt`.

**R9: one atomic code phase, deliberately.** Decision 1 forbids an aliasing
period, so any split between "rename the tables" and "migrate the call sites"
leaves an intermediate commit whose test suite is red and whose corpus does not
compile: the prelude, every example and every embedded test source break the
moment `BUILTIN_TABLE`'s keys move. Phase 1 below therefore carries the tables,
the prelude, the two R4 renames and the whole scripted call-site migration, and
exits fully green. Phase 2 carries documentation only. This is the one place this
spec departs from the suggested three-way phasing, and the reason is the
red-commit cost, not scope.

## Phases

### Phase 1: rename (compiler, prelude, corpus, tests) — exits green

Ordered steps, all in one phase:

1. Table/list values: `check/operators.rs` (`is_operator` list, the `match name`
   arms at `:190` and `:275`, `BUILTIN_TABLE` keys), `check/builtins.rs`
   (`COMPARISON_PRIMITIVES`, builtin table rows), `check/declarations.rs`
   (`BUILTIN_WORDS` and its comments), `resolve.rs`
   (`is_operator_dispatch_name` and its comments), `check/poly.rs:718` (the
   comparison-six name gate), plus any remaining quoted-symbol site surfaced by
   R2's grep (`ir/driver.rs`, `ir/func_builder/calls.rs`,
   `ir/func_builder/word_families.rs`, `check/word_families.rs`,
   `check/drop_graph.rs`, `check.rs`, `parser.rs`, `backend/qbe.rs`).
2. `lib/core.sth:17-22`: the six comparison words are renamed (the definition
   `: =` becomes `: eq`, and so on) and each body's `u`-primitive call is renamed (`u=` → `ueq`, …). The
   surrounding comment block is re-spelled.
3. R4's two corpus word renames and their paired call sites.
4. R5's `word_entry.rs` pair.
5. Scripted `.sth` corpus migration per R1 (38+ files), then hand review of the
   comment hunks.
6. Embedded sooth sources in `src/**/*.rs` and `tests/*.rs` per R2.
7. Test-name/assertion pass per R3.
8. `REGEN_QBE_BASELINE=1 cargo test --test qbe_baseline`, then **review the
   diff**: the only expected changes are the symbol names for R4's two renamed
   words (`$add__m2` → the sanitized `point-add` symbol in `modules.ssa`, and the
   `vectors.ssa` counterpart). Instruction selection is unaffected — QBE's own
   `add`/`sub` opcodes never saw the surface spelling. **Any other baseline hunk
   is a bug, not a rename.**

Exit criteria:

- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- `cargo run -- build examples/gcd.sth` and the corpus stdout goldens
  (`tests/phase4_slice10c_corpus_stdout.rs`) pass unchanged in *output*.
- `git grep -nE '(^|[[:space:]])(\+|-|\*|/|=|<|>|<=|>=|<>|u=|u<|u>|u<=|u>=|u<>)([[:space:]]|$)' -- '*.sth'`
  returns nothing.
- No `"+"`, `"u<"`-family quoted literal remains in `src/` or `tests/` naming a
  builtin; the ones that remain (`qbe_name` fixtures, lexer token tests) are
  exactly R3's keep list.
- The qbe baseline diff is confined to step 8's two words.

New tests (Phase 1):

- `check_symbolic_plus_is_unknown_word`: `1 2 +` fails with ``unknown word `+` ``,
  not an arity or type error. Guards that the retirement is real rather than a
  surviving alias. Mutation check: restoring `"+"` to `BUILTIN_TABLE` must make
  this test fail.
- `check_symbolic_comparison_is_unknown_word`: same for `<` (the `lib/` half),
  proving the prelude no longer defines it.
- `check_add_word_dispatches_on_operand_type`: `add` resolves the same
  `BUILTIN_TABLE` rows `+` did, over `i64` and `f64`, and reports the same
  operand-mismatch diagnostic text with the new name.
- `check_ueq_family_lowers_to_cmpop`: the renamed unsigned six still map to
  their `CmpOp` (extends `tests/phase4_slice10c_primitives.rs:270`'s loop).
- A plain word named exactly `: add ( Vec2 Vec2 -- Vec2 )` still dispatches
  through `resolve::is_operator_dispatch_name` as a builtin-name overload rather
  than an ordinary per-module-mangled word: `resolve::mangle` still gives its
  own-module declaration a `__m{k}` suffix in a multi-module build (confirmed by
  `nm`), but the call-site rewrite deliberately leaves the bare call unmangled so
  `check_operator`'s candidate scan finds it. This is byte-identical, mechanism
  and all, to `own_module_operator_overload_reachable_bare_in_multi_module`
  (Phase 4 slice 8b, R13), so it is that test's coverage, not a separate one --
  no new test is added for it.

### Phase 2: documentation

R6's rewrite set only: `DESIGN.md`, `README.md`, `docs/book/*.md`, and any
`docs/` snippet presented as currently valid syntax. Every code block in the
rewrite set that is a complete program is compiled (`cargo run -- build`) or
hand-checked against the new spellings; the book's prose that reads as English
around an operator (e.g. "less than") is left alone.

Exit criteria:

- No occurrence of a retired spelling in `DESIGN.md`, `README.md` or
  `docs/book/` as sooth code or as a description of current syntax.
- `docs/roadmap/` and the completed-work briefs/specs are byte-identical to
  Phase 1's tree (verify with `git diff --stat`).
- Suite still green (no code change in this phase; a red suite means Phase 1
  leaked).

## Risks

- **Scripted-pass overreach.** Mitigated by R1's whole-token rule and by the
  comment-hunk hand review. The `>`/`-` characters carry three other roles
  (conversion prefix, destructure suffix, negative literal); a character-level
  substitution would break all three silently.
- **A rename that changes a mechanism.** Recon delta 1 is exactly that class of
  bug: a word that was plain becomes an operator overload with different
  mangling. R4 handles the two known cases; the Phase 1 exit grep plus the qbe
  baseline review is the net for any missed one, since an unmangled symbol shows
  up in the baseline diff.
- **New-spelling collisions in future work.** After this change, `add sub mul
  div eq lt gt lte gte ne ueq ult ugt ulte ugte une` are reserved-ish names: a
  user word so named is an overload, not a fresh word. That is a real ergonomic
  cost of the rename and is accepted, not mitigated.

## Out of scope

- Collapsing the three keep-in-sync lists (Decision 3, R7).
- Removing `qbe_name` (Recon 5).
- The `>`-suffix destructure mechanism, the `>T` conversions and the `&`-family
  sigils (R8).
- `mod and or xor not shl shr max max-total .`: already words.
- Rewriting historical phase docs (R6).
