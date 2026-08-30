# Phase 7 Slice 7d: retire the compiler-intrinsic `.` onto `hosted::show`

**Status:** Draft
**Created:** 260830
**Discovery:** [slice7d-dot-hosted-brief.md](./slice7d-dot-hosted-brief.md) — the **260830
addendum (R1′–R7′, revised exit) supersedes R1–R3 where they conflict and is treated as
decided here.** Evidence companions: [slice7d-census.md](./slice7d-census.md) (full
migration surface) and [slice7d-probes.md](./slice7d-probes.md) (disposable compile-probe
round; the candidate `lib/hosted/show.sth` and extern sources are inline there and
verified working except where a probe says otherwise). Predecessor:
[slice7c-spec.md](./slice7c-spec.md) (`core::show`'s `Show`/`Write` pair this slice
prints through).

## Problem

`.` is a compiler-injected, name-dispatched builtin: fourteen `BUILTIN_TABLE` rows over
`printable_types()` (`src/check/builtins.rs:197`), a hand-written `check_operator` arm
(`src/check/operators.rs:349`), and an `Instr::Print` instruction the QBE backend lowers
straight to libc `printf` (`src/backend/qbe.rs:1246`) — an OS dependency baked into the
compiler itself, invisible to the layer system every other hosted capability reaches
through `depends:`. A program that types `42 .` into existence imports nothing, which is
exactly the implicit behavior the rest of Sooth refuses. Meanwhile the `printf` rows sit
beside a real, layered printing vocabulary S7c just built: `Show` formats a value into a
`StrBuf`, `Write` flushes it to a sink, and the whole path is `write(2)`-ordered rather
than buffered-stdio. The cost of keeping `.` is twofold: the layer system has a permanent
hole in it, and the compiler carries a variadic-`printf` codegen path (with its
`$fmt`/`$ufmt`/`$ffmt`/`$sfmt`/`$strfmt` data and per-type newline conventions) that
duplicates what the library now does. Retirement moves `.` onto the S7c path and deletes
the special case — but per R3′ the migration is program-wide: every printing example,
golden, and test breaks at once, so the slice is atomic by construction.

## Design rulings

1. **The landing shape is per-type concrete dots, not the generic word (R1′).** The
   generic `: . ['T: Show] ( 'T -- )` parses and registers but its body is unwriteable
   today (probe P2): a locally built `&!StrBuf` does not unify with a poly callee's
   declared `&!StrBuf` (the `parser.rs:4117` Slice-13 R-A4 fold makes declared refs
   `PolyType::Concrete(Type::Ref(…))` while local borrows are native `PolyType::Ref`), and
   field accessors (`&!len`, `&!data`) and `+!` are located errors in generic bodies. What
   the probes proved instead (P3d/P3f): several same-arity **concrete** `: . ( T -- )`
   candidates in one module are legal, and a caller importing that one module dispatches
   per-site on the bare call. So `hosted::show` lands one concrete dot per printable type,
   with internal delegation through distinctly named private helpers (P3e: intra-module
   bare cross-overload calls do not resolve). The generic dot is recorded follow-up, not a
   blocker.
2. **The delete list is the brief's R2′ list plus the resolve.rs predicate.** All
   fourteen builtin rows, `printable_types`, `BuiltinLower::Print`, the `check_operator`
   `.` arm and its `is_operator`/`is_unary` entries, `Instr::Print` with its producer and
   emit arms, the five user-facing format data rows — and `.`'s entry in
   `resolve.rs`'s `is_operator_dispatch_name` (`src/resolve.rs:72`), a fourth site the
   original R2 missed: with `.` still listed, a selectively imported `hosted::show` `.`
   stays unrewritten expecting builtin dispatch and every bare call is
   `unknown word '.'` (probe P3f). `.` **stays in `BUILTIN_WORDS`** (self-tail-call
   detection, the extern-redeclaration check at `src/check/declarations.rs:38`, and the
   local-binding collision checks at `src/check/terms.rs:158`/`src/check/poly.rs:940`
   still read it), excluded from `is_name_dispatched_builtin` exactly like the six
   surface comparisons.
3. **`core::bool`'s overload is deleted, not migrated (R2′/P4a).** Layering forces it:
   post-retirement the overload body's inner str `.` cannot resolve from the core layer.
   Its `import: intrinsics i | branch tag drop . | ;` (`lib/core/bool.sth:4`) and
   `export: … . ;` (line 6) are trimmed. Consequence, deliberately accepted (R4′):
   `True .` prints `true\n` through `Show for Bool` — lowercase, matching
   `docs/book/numbers.md:239-256`, which already documents lowercase; the *library
   overload* was the deviation.
4. **Newline contract: byte-compatibility, spelled as a second write (R5′).** Numeric and
   Bool dots append the newline; str and cstr dots print exact bytes with none —
   reproducing today's per-type behavior (`%ld\n` vs `%.*s`/`%s`). The newline is a second
   `write(2)` of `"\n"` through the str path, **not** an in-buffer append (P2e/P2h:
   borrow-alias records block the read-modify-write append over one locally built StrBuf).
   All output is syscall-ordered, so the buffered-stdio interleaving hazard is gone by
   construction (P9).
5. **`Show` impls widen to the integer tower (R6′).** `core::show` gains
   `impl: Show for` u8, u16, u32, u64, i8, i16, i32 — trivial widenings onto the existing
   `append-digits` path. Gotcha from P7: the i8/i16/i32 sign test must widen first
   (`n >i64 0 lt`; a bare `n 0 lt` resolves the literal ambiguously and errors). str/cstr
   stay out of `Show` (S7c D3) — they are concrete dots only. Every new impl needs its
   matching dot, since dispatch is per concrete candidate.
6. **Floats print through hosted `snprintf`, after an in-slice ABI fix (R7′).** The
   user-extern f64-argument ABI is broken today: a probe-bound
   `extern: g-fmt ( &!array[u8 64] usize cstr f64 -- i32 ) "snprintf"` compiles and runs
   but the f64 arrives as 0. Spec-writing diagnosis (this worktree, verified by building
   the probe and reading the emitted assembly): the backend emits user extern calls as
   fully-fixed QBE calls with no `...` marker (`src/backend/qbe.rs:1162`), so QBE emits
   no `%al` setup at all for that spelling — whether the variadic callee spills the xmm
   register holding the double is then undefined behavior (the probe's observed 0 was
   luck; a minimal fixed-spelling repro can pass the double through, which is why R14's
   end-to-end regression test is the essential guard, not a formality);
   `Instr::Print`'s float arm passes `d` args to variadic `printf` correctly
   because it spells the `...` form. QBE accepts an all-args-variadic spelling
   (`call $sym(..., args…)`) and both a float-taking (snprintf) and int-only (write)
   extern run correctly under it. The fix is backend work with its own regression tests
   and its own early phase so it can land or be descoped independently. `snprintf` is
   libc and links fine — no libm story is needed (the P8 libm control failed to link; that
   is out of scope).
7. **The two new externs live in `hosted::show` itself, not `hosted::libc`.** The
   verified shapes (P1, P8) declare `extern: sys-write-str ( i32 cstr usize -- isize )
   "write" ;` and `extern: g-fmt ( … f64 -- i32 ) "snprintf" ;` in the module that calls
   them; `lib/hosted/libc.sth` is untouched (`Stdout` is already exported for the flush).
   Two Sooth bindings for the C symbol `write` are fine — the declared C symbol is what
   links. (`hosted::libc` as the home would also typecheck by inspection — extern names
   are in `resolve.rs`'s word tables (`src/resolve.rs:274`) — but that exact module shape
   was never compiled by the probes; the in-module shape was. Recorded in Open Questions
   as a settled default.)
8. **Migration is program-wide, no shim (R3′).** Once the delete list lands, every
   printing program must `depends: hosted` and `import: hosted::show | . | ;`. The
   harness does most of the work: `tests/common/mod.rs`'s `fixture_imports` machinery
   appends the import to file-based fixtures the way it already appends `core::prelude`
   lines, and `tests/fixtures/sooth.pkg` gains `depends: hosted`. Examples, raw-written
   scratch programs, and library files migrate by hand. There is no compatibility shim —
   CLAUDE.md's magicless-over-convenience rule applies directly.
9. **Phasing is atomic where R3 forces it.** The compiler deletions, the resolve.rs
   predicate fix, the new `hosted::show`, the `core::show` widenings, and the
   program-wide migration land as one phase — no ordering leaves the tree green
   otherwise, the same shape P8.S2's prelude deletion took ("the third was necessarily
   atomic", `docs/roadmap/P8/slice2-spec.md`). The f64 ABI fix is its own earlier phase
   with its own regression test; the doc pass follows last.

## Requirements

Numbered, one verifiable claim each. R′ references point at the brief's addendum.

- **R1.** `is_name_dispatched_builtin` (`src/ast.rs:1706`) excludes `.` the way it
  excludes the six surface comparisons; `.` stays in `BUILTIN_WORDS`
  (`src/ast.rs:1629`); the gate-set unit test (`src/ast.rs:3100`) is updated to the
  seven-name exclusion set and flips its `.` assertion (`src/ast.rs:3122`).
- **R2.** `builtin_table` carries no `.` rows: the printable loop
  (`src/check/builtins.rs:197`), `printable_types` (`src/check/builtins.rs:130`), and
  `BuiltinLower::Print` (`src/check/builtins.rs:94`) are deleted, with the surrounding
  doc comments updated to the new reality.
- **R3.** `check_operator` has no `.` arm: the `"."` entries in `is_operator`
  (`src/check/operators.rs:106`) and `is_unary` (`src/check/operators.rs:114`), the
  `"."` arm (`src/check/operators.rs:349`), and `print_requires_printable_error`
  (`src/check/operators.rs:459`) are deleted.
- **R4.** `is_operator_dispatch_name` (`src/resolve.rs:72`) does not list `.`; after one
  selective `import: hosted::show | . | ;`, a bare `.` call rewrites through the
  ordinary own-module/selective-import branches (`src/resolve.rs:420`, `:460`) and
  dispatches per call site across the concrete candidates (probe P3f).
- **R5.** The IR has no print instruction: `Instr::Print` (`src/ir/types.rs:423`), its
  producer arm (`src/ir/func_builder/calls.rs:639`), and its backend emit arm
  (`src/backend/qbe.rs:1246`) are deleted; the `$fmt`/`$ufmt`/`$ffmt`/`$sfmt`/`$strfmt`
  data rows (`src/backend/qbe.rs:72`, `:103`) are deleted. The backend-internal trap/OOM/
  bounds/trace diagnostics (`$oobfmt`/`$subslicefmt`/`$allocfmt`/`$freefmt`/`$oomfmt`,
  the `dprintf`/`printf` helpers at `src/backend/qbe.rs:894`–`:987`) are untouched.
- **R6.** `lib/hosted/show.sth` provides one concrete `: . ( T -- )` per printable type —
  Show-backed dots for i64, i8, i16, i32, isize, u8, u16, u32, u64, usize, Bool (each:
  fresh `StrBuf`, `render`, `flush` through `Stdout`, then `"\n"` via the str path);
  str (all `len` bytes via the write extern — R17's NUL note rules the interior-NUL
  delta — no newline); cstr (strlen-bound bytes via a third in-module extern,
  `extern: sys-strlen ( cstr -- usize ) "strlen"` — `len` does not accept `cstr`;
  precedent `examples/strings.sth:3` — no newline); f64/f32
  (`%g\n` via the snprintf extern, f32 widening through `>f64`) — delegating through
  distinctly named private helpers (P3e), and the module is registered in
  `lib/hosted/sooth.pkg`.
- **R7.** `lib/core/show.sth` gains `impl: Show for` u8, u16, u32, u64, i8, i16, i32
  over the existing `append-digits` path, with signed impls widening before the sign
  test (P7 gotcha); `core::show` stays `no_std` (no new externs there).
- **R8.** `core::bool`'s `.` overload (`lib/core/bool.sth:52`), its import entry
  (`lib/core/bool.sth:4`), and its export entry (`lib/core/bool.sth:6`) are deleted;
  `True .` and `False .` print `true\n`/`false\n` through `Show for Bool` (R4′, accepted
  capitalization change).
- **R9.** `hosted::testing`'s `expect` emits byte-identical TAP lines
  (`ok --`/`not ok --` + label + `\n`), with its import line migrated from
  `import: intrinsics | . | ;` (`lib/hosted/testing.sth:7`) to the sibling
  `self::show` module; the driver's only output contract (`count_protocol`,
  `src/driver/toolchain.rs:139`) is unchanged.
- **R10.** Every committed example that prints imports `hosted::show` explicitly (42 of
  the 49 committed examples — 44 top-level plus 5 under `examples/tests/` — per census
  §2); `examples/leap.sth` and `examples/array_ctor.sth` drop their
  `import: core::bool … | . | ;` lines and their stale one-hop-rule comments
  (`examples/leap.sth:8`, `examples/array_ctor.sth:16`); no printing program compiles
  via a compiler intrinsic.
- **R11.** The test harness derives the printing import: `fixture_imports`
  (`tests/common/mod.rs:79`) emits `import: hosted::show | . | ;` for fixtures whose
  text prints and do not declare their own `.`; the `bool_imports` heuristic
  (`tests/common/mod.rs:162`) and its one-hop doc comments
  (`tests/common/mod.rs:152`–`:161`) are deleted; `tests/fixtures/sooth.pkg` and
  `fixture_package` (`tests/common/mod.rs:51`) gain `depends: hosted`. The emission is
  presence-guarded like `bool_imports` — mandatory, not incidental: `Scratch::write`
  routes through `common::fixture_source`, whose harness appends the printing import,
  so fixture source that also hand-writes `import: hosted::show | . | ;` would trip
  `duplicate_qualifier_error` (`src/driver.rs:235`).
- **R12.** Goldens match the new reality: `tests/corpus_stdout/*.txt` regenerated
  (`REGEN_CORPUS_STDOUT=1`, `tests/phase4_slice10c_corpus_stdout.rs:12`),
  `tests/fill_corpus/*.stdout` updated where output changed, `tests/qbe_baseline/*.ssa`
  regenerated (`REGEN_QBE_BASELINE=1`, `tests/qbe_baseline.rs:8`); the intended
  program-output changes are exactly two — the accepted Bool lowercase (R8) and the
  trace-line reorder of R17 — and `sooth test` driver output is byte-identical
  otherwise (probe P10 capture). Inline expected-stdout assertion literals spelling
  `True\n`/`False\n` flip to lowercase too: 34 expected-stdout assertion literals across
  14 test files (grep-verified: `phase0.rs` ×8, `phase7_slice3f.rs` ×7, `phase4_generics.rs`
  ×4, `phase7_slice3a/d/h/i.rs` ×2 each, and one each in `phase4_quotations.rs`,
  `phase4_slice6h.rs`, `phase5_slice1.rs`, `phase6_slice1.rs`, `phase7_slice2.rs`,
  `phase7_slice3g.rs`, `phase7_slice3m.rs`). Fixture-source Bool tokens — e.g.
  `phase7_slice3s_flip.rs:460`'s `True` constructor in program text — migrate as program
  text riding the harness import, not as assertions.
- **R13.** Unit tests on the deleted surface die with it: `builtin_table_has_a_row_per_printable_type_for_print`
  (`src/check/builtins.rs:561`), the eight `.` checker tests
  (`src/check/operators.rs:508`, `:840`, `:844`, `:1002`, `:1013`, `:1041`, `:1048`,
  `:1055`), the `emit_print_*` backend tests (`src/backend/qbe.rs:1611`–`:1730`), the
  IR tests counting `Instr::Print` (`src/ir/func_builder/calls.rs:1295`–`:1322`,
  `src/ir/func_builder/quotation.rs:755`, `src/ir/destructors.rs:549`–`:619` — the
  `FILE_RESOURCE` stand-in gets a non-print observable effect), and the diagnostic pin
  `tests/phase7_slice3i.rs:157`.
- **R14.** An f64 argument declared on a user `extern:` reaches the C callee: the
  snprintf control program prints `2.5\n` (today it prints `0\n`, probe P8 — an
  undefined-`%al` occurrence, not a guaranteed zero; see ruling 6), with a
  backend unit test pinning the emitted extern-call IL form and an end-to-end
  regression test (R7′).
- **R15.** The documentation surface that shows printing is migrated: README
  (`README.md:26`, `:130`, `:155`, `:188`), the book chapters of census §8 (only where
  the print rewrite touches them — the str print contract gains the len-honest NUL
  sentence of R17), `DESIGN.md:233`'s `.` mention, and
  `docs/roadmap/P8/dogfood/` annotated as no-longer-compiling (not migrated); the
  roadmap's S7d entry is marked `[ done ]` with its deliverables summarized.
- **R16.** The prelude's one-hop note (`lib/core/prelude.sth:7`) is rewritten: the
  operator-overload one-hop paragraph dies (its subject is deleted); what remains, if
  anything, states where printing lives now (hosted-only, explicit import).
- **R17.** Byte compatibility (NFR), with two named exceptions. Numeric dots reproduce
  `%ld`/`%lu`/`%g`-with-newline semantics; the cstr dot reproduces the `%s`
  terminator-bound semantics; every output path is `write(2)` (buffered stdio is no
  longer involved in user printing at all). The exceptions:
  (a) **Trace-line reorder (intended-to-change)**: programs mixing `SOOTH_TRACE` trace
  lines with dots reorder — trace lines ride buffered stdio (`emit_trace_event` calls
  `$printf`, `src/backend/qbe.rs:976`–`:987`, untouched per R5) and now flush at exit,
  while dots go out via unbuffered `write(2)` immediately. The trace-mixing goldens are
  updated to the new order as intended-to-change: the `run_owned_traced_golden` family
  (28 call sites plus the helper in `tests/phase0.rs`, e.g. the peek-twice golden whose
  assert at `:2674` expects `"alloc 8\n5\nTrue\nfree 8\n"`), `run_src_traced` (3 call
  sites plus the helper in `tests/phase3_refs.rs`, e.g. the assert at `:544` expecting
  `"alloc 8\nfree 8\n7\n"`), and any other trace-mixing assert the diff review finds.
  (b) **Str interior NUL (ruled, newly pinned)**: the str dot writes all `len` bytes —
  today's `%.*s` str row stops at the first NUL, a printf artifact; Sooth's `len`
  deliberately counts embedded NULs (pinned by `tests/phase3_strings.rs`'s
  `interior_nul_diverges_sooth_len_from_c_strlen_native`: `len` reads 5, cstr `strlen`
  reads 2, the cstr dot prints `ab`). The delta (`"ab\0cd" .` changes from `ab` to
  `ab\0cd`) breaks no existing golden (none pins the str row with a NUL); phase 2 adds
  the pin test for the new behavior. The cstr dot keeps today's strlen-bound semantics.
- **R18.** Poly-body verification (NFR, R1′ verification duty): the migrated corpus is
  re-checked to contain no printing inside a poly body (census §3 found none;
  `expect-eq` never prints values). If one exists, the generic-dot checker fix
  (`parser.rs:4117` declared-ref unfold + poly-body accessor ruling) is pulled into this
  slice instead of deferring it.

## Success criteria

- [ ] `cargo run -- build examples/gcd.sth` and every committed example build and run
      with `import: hosted::show | . | ;` in source; grep finds no `import: intrinsics`
      line naming `.` anywhere under `lib/` or `examples/`.
- [ ] A program importing `hosted::show` prints every printable type with today's bytes:
      the probe all-paths baseline (`42 -7 255 -5 100000 3.5 2.5 true false a<TAB>b hi
      one<TAB>two`) reproduces except `True`/`False` → `true`/`false` (probe baseline od).
- [ ] `src/` greps clean: no `printable_types`, no `BuiltinLower::Print`, no
      `Instr::Print`, no `$fmt = { b "%ld` / `$ufmt` / `$ffmt` / `$sfmt` / `$strfmt` data
      rows; `is_operator_dispatch_name` and `is_name_dispatched_builtin` do not match
      `.`; `BUILTIN_WORDS` still contains `.`.
- [ ] `True .` / `False .` print `true\n` / `false\n`; `cargo run -- test
      examples/tests/bool.sth` prints the byte-identical driver summary
      (`ok   examples/tests/bool.sth` + `1 entries, 0 failed (4 ok, 0 not ok
      assertions)`).
- [ ] The snprintf f64-extern control program (probe P8 shape) prints `2.5\n`.
- [ ] `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green, with
      corpus_stdout, fill_corpus, and qbe_baseline regenerated deliberately and their
      diffs reviewed as output-intended-to-change.

## Scope & boundaries

**In scope:**

- The compiler delete list of R1–R5 (check/builtins, check/operators, resolve, ast,
  ir, backend/qbe) with `.` retained in `BUILTIN_WORDS`.
- The new `lib/hosted/show.sth`, its `sooth.pkg` registration, the two in-module
  externs, and the seven widened `Show` impls in `lib/core/show.sth`.
- Deletion of `core::bool`'s overload; the prelude note rewrite; the
  `hosted::testing` import migration.
- The whole-program migration: 42 examples, the harness rule + fixture manifest, the
  ~715 test-side print sites across 68 files (mostly via the harness; raw sources by
  hand, census §3), the `SPY_DEF` split, `depends: core hosted` for the ad-hoc/pkg-less
  printing test manifests (map row), and golden regeneration (census §2–§4).
- The user-extern f64-ABI backend fix with its regression tests (R7′).
- README/book/DESIGN/roadmap doc migration and P8/dogfood annotations (census §8).

**Out of scope** (brief's explicit exclusions, unchanged):

- Any sink beyond `Stdout` (S7c's scope).
- The backend's own internal `printf`/`dprintf` diagnostics (trap messages, OOM, bounds,
  trace) — they keep their direct calls and their data rows (R1/R5).
- The generic `: . ['T: Show] ( 'T -- )`: the `parser.rs:4117` declared-ref fold and
  poly-body accessor/`+!` support — P7.S3-family poly-borrow work, its own item (R1′),
  subject only to R18's pull-in trigger.
- Diagnostics: the silently ignored stale `import: intrinsics | . | ;`, the hintless
  `unknown word '.'`, and the `no overload of '.'` message naming no fix (P5/P6) — the
  cross-cutting diagnostics track, beside S8's unsatisfied-`Ord` attribution.
- libm linking for user programs (the P8 control); a pure-Sooth `%g` renderer in
  `core::show` (the R7′ fallback alternative, only if the ABI fix is descoped).
- README's user word named `show` colliding with the trait member name — rename or
  footnote at the implementer's judgement (census §8).

## Solution approach

The slice is a deletion with a library replacement riding behind it. The compiler side
is a pure subtractive change across four layers that already share one concept:
the name-dispatch gate (`ast`), the operator table (`check/builtins`, `check/operators`),
the name-rewrite predicate (`resolve`), and the print instruction (`ir`, `backend/qbe`).
Each layer keeps its invariants: `.` stays in `BUILTIN_WORDS` so self-tail-call
detection, extern-redeclaration, and local-binding collision checks keep treating it as
a builtin-shaped name; the IR stays backend-neutral (deleting `Instr::Print` removes a
backend-dispatched instruction rather than adding one); and the backend's internal
diagnostics keep their own `printf`/`dprintf` paths untouched. The one addition on the
compiler side is the R14 ABI fix, which marks extern calls at the lowering boundary —
`src/ir/driver.rs:167` is where externs enter the lowering env, so that is where the
callee's extern-ness is known — and spells them in the all-args-variadic QBE form so
variadic libc callees see the right `%al` count. User-word calls keep today's fixed
form: the `arm64` caveat already recorded at `src/backend/qbe.rs:1237` (variadic args go
on the stack there) is why the marker must not be applied to calls the compiler itself
defines.

The library side is the probe-verified candidate nearly verbatim: `hosted::show`
declares its two externs (`write` bound with a `cstr` parameter for the str/cstr/newline
path, `snprintf` for floats), imports `Stdout` from `hosted::libc` and the bounded
`render`/`flush` consumers from `core::show`, and exports one concrete dot per printable
type. Bodies repeat the fresh-buffer/render/flush/newline shape rather than sharing a
poly helper — that repetition is the P2/P3e constraint, not sloppiness, and a
`core::show` append helper may revisit it later (R5′ note). The seven integer `Show`
impls are widenings onto `append-digits` with the P7 widen-first sign-test gotcha.
`core::bool`'s overload simply dies; `Show for Bool` already prints the lowercase
spelling the book documents.

The migration rides the harness. `fixture_imports` already computes and appends the
imports a fixture's text implies; replacing `bool_imports`' bool-ness heuristic with a
"prints and does not declare its own `.`" rule migrates the ~715 test-side sites without
touching most of them, and one `depends: hosted` line in the shared fixture manifest
makes the import resolvable. The residual hand-migration is enumerable from the census:
library files, 42 examples, raw-written scratch programs (`write_raw`/`Scratch` sites),
the compiler unit tests that die with the deleted surface, and the three golden sets,
regenerated deliberately under their documented env vars. The doc pass then brings the
prose up to the same reality, correcting only what the print rewrite touches.

## Codebase Map

The ground truth for the implementer. Every anchor below was read in this worktree at
spec-writing time; line numbers may drift — re-locate by symbol.

### Compiler — deleted surface

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/ast.rs:1629` | `BUILTIN_WORDS` | `.` **stays listed** (self-tail-call detection, `is_builtin_word_name` consumers) |
| `src/ast.rs:1684` | `is_builtin_word_name()` | Untouched; still true for `.` |
| `src/ast.rs:1706` | `is_name_dispatched_builtin()` | Gains `.` in the exclusion `matches!` beside the six comparisons; its doc comment (the "`.` is *not* in that exclusion set" paragraph above it) is rewritten |
| `src/ast.rs:3100` | `the_gate_set_excludes_exactly_the_six_surface_comparisons()` | Renamed/updated: exclusion set becomes seven names; `src/ast.rs:3122`'s `assert!(is_name_dispatched_builtin("."))` flips |
| `src/resolve.rs:72` | `is_operator_dispatch_name()` | Drops the `"."` arm; the doc comment above it notes why the six left — extend for `.` |
| `src/resolve.rs:420` | `rewrite()` own-module branch | After R4, a module's own `.` decl rewrites here (own decl shadows an import for bare calls) |
| `src/resolve.rs:460` | `rewrite()` selective-import branch | After R4, the P3f import shape rewrites here; per-site family dispatch follows |
| `src/check/builtins.rs:94` | `BuiltinLower::Print` | Variant deleted; `BuiltinRow.lower`'s doc comment references it |
| `src/check/builtins.rs:130` | `printable_types()` | Deleted (14-row doc comment above it dies with it) |
| `src/check/builtins.rs:197` | `builtin_table()` printable loop | The `row(".", …)` loop deleted; `is_builtin_operator_name` (`src/check/builtins.rs:206`) stops matching `.` |
| `src/check/builtins.rs:561` | `builtin_table_has_a_row_per_printable_type_for_print()` | Deleted |
| `src/check/operators.rs:106` | `is_operator` list | `.` entry deleted |
| `src/check/operators.rs:114` | `is_unary` list | `.` entry deleted |
| `src/check/operators.rs:349` | `check_operator()` `"."` arm | Deleted (the P7.S3c slice-decision comment inside it dies; the slice ruling itself moves to library territory) |
| `src/check/operators.rs:459` | `print_requires_printable_error()` | Deleted (only caller is the `.` arm) |
| `src/check/operators.rs:508,840,844,1002,1013,1041,1048,1055` | `dot_printable_set_slice_decision`, `check_usize_print_is_type_directed_ok`, `check_print_on_array_is_error`, `check_print_accepts_every_printable_scalar`, `check_print_of_a_bool_needs_the_core_bool_overload`, `check_print_accepts_str_and_cstr`, `check_print_on_empty_stack_is_underflow_error`, `check_print_on_linear_value_is_error` | Deleted per R2′ (names via grep; re-locate if drifted) |
| `src/check/terms.rs:1196`, `src/check/word_families.rs:1180`, `src/check.rs:34`, `src/driver.rs:340`, `src/check/declarations.rs:788`, `src/parser.rs:2772` | `is_name_dispatched_builtin` consumers | No edits — behavior shifts with the predicate (the `intrinsics` gate stops covering `.`, so a bare `.` with no import becomes the ordinary unknown-word error, P5's observed shape) |
| `src/ir/types.rs:423` | `Instr::Print` | Variant deleted |
| `src/ir/func_builder/calls.rs:639` | `lower_call()` `"."` arm | Deleted (sole `Instr::Print` producer) |
| `src/backend/qbe.rs:72`–`:103` | `$fmt`/`$ufmt`/`$ffmt`/`$sfmt`/`$strfmt` data rows | Deleted; `$oobfmt`/`$subslicefmt`/`$allocfmt`/`$freefmt`/`$oomfmt`/`$tracenv` stay |
| `src/backend/qbe.rs:1246` | `emit_instr()` `Instr::Print` arm | Deleted (including the `IrType::Bool` unreachable and the `...`-marker comment at `:1234`, which partially survives on the trace paths) |
| `src/backend/qbe.rs:1611`–`:1730` | `emit_print_uses_printf_and_fmt`, `emit_print_on_float_uses_ffmt_and_d_arg`, `emit_print_on_f32_widens_before_calling_printf`, `emit_print_on_unsigned_uses_ufmt`, `emit_print_on_isize_uses_fmt_signed`, `emit_print_on_subword_unsigned_widens_via_extuw`, `emit_print_on_subword_signed_widens_via_extsw` | Deleted |
| `src/ir/func_builder/calls.rs:1295`–`:1322` | `lower_print_emits_print_instr`, `lower_print_on_str_and_float_emits_same_print_instr` | Deleted |
| `src/ir/func_builder/quotation.rs:755` | `lower_call_of_two_output_word_unpacks_the_bundle_onto_the_stack()` | Retarget: keep the `Call`/`FieldLoad` counts, replace the two trailing `.` with consumptions that emit nothing (`drop drop`) |
| `src/ir/destructors.rs:549`–`:619`, `src/ir/test_helpers.rs:31` | `FILE_RESOURCE` and the drop-glue tests counting `Instr::Print` | `FILE_RESOURCE`'s drop body gets a non-print observable effect (an `extern:` call is the closest substitute — an `Instr::Call` no synthesized glue emits either); the counting asserts follow |
| `tests/phase7_slice3i.rs:207` | `boolean_static_with_the_import_holds_its_initializer` | Bool-printing golden, `write_raw` so harness-independent hand-migrated to the hosted import and flips to `true\n10\n` (its corebool import line at `:212` names `.`) |
| `tests/phase7_slice3i.rs:157` | `the_prelude_hub_carries_the_constructors_and_the_type_name_but_not_the_print_overload` pin | The `` `.` requires a printable scalar, found `Bool` `` pin dies; `:167`'s G2 (`import: core::bool … \| . \| ;` golden pinning `True\nFalse\nFalse\n1\n`) is deleted or retargeted to the hosted::show lowercase reality |

### Compiler — the R14 ABI fix (phase 1)

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/ir/types.rs:524` | `Arity` | Gains an extern-callee flag; its doc comment states why (Sooth externs cannot express where C's `...` begins, and the backend must spell variadic calls so QBE counts the FP args for `%al`) |
| `src/ir/driver.rs:167` | extern registration into the lowering env | Sets the flag `true` here; the user-word entries at `src/ir/driver.rs:149` and the monomorphized-instantiation entries at `src/ir/driver.rs:304` set it `false` |
| `src/ir/func_builder/calls.rs:893` | `emit_user_call()` | Emits the marked call (extend `Instr::Call` or add a sibling variant — either is fine, the flag must survive to the backend) |
| `src/ir/func_builder/mod.rs:1000` | test-env `Arity` construction | Kept consistent with the new field |
| `src/backend/qbe.rs:1162` | `emit_instr()` `Instr::Call` arm | Extern calls: all-args-variadic spelling `call $sym(..., args…)` (verified: QBE accepts it; snprintf receives the double; `write`/`strlen`/`puts` unaffected — a non-variadic callee ignores `%al`). User-word calls unchanged. New unit test beside `emit_instr` (`src/backend/qbe.rs:994`) pinning the form |
| `src/driver/toolchain.rs:139` | `count_protocol()` | Untouched; the driver contract `sooth test` depends on |
| `tests/phase7_slice7c.rs:14` | `Tree` build-and-run pattern | Model for the new regression test file |

### Library

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `lib/hosted/show.sth` (new) | module `show` | The 15 concrete dots + `print-str` and the float helper; modeled on the probe's final candidate source (slice7d-probes.md) and on `lib/core/show.sth`'s conventions; imports `core::show \| StrBuf Show Write render flush \|`, `hosted::libc \| Stdout \|`, `core::bool \| Bool \|`, `intrinsics *`; declares `extern: sys-write-str ( i32 cstr usize -- isize ) "write"`, `extern: sys-strlen ( cstr -- usize ) "strlen"` (precedent `examples/strings.sth:3`), and `extern: g-fmt ( &!array[u8 64] usize cstr f64 -- i32 ) "snprintf"`; `export: . ;` |
| `lib/hosted/sooth.pkg:6-7` | module list | Gains `module: show ;` |
| `lib/hosted/libc.sth:6,14,19` | `sys-write`, `Stdout`, `Write for Stdout` | Unchanged — the S7c sink this slice flushes through |
| `lib/core/show.sth:79`–`:136` | `impl: Show for i64/usize/isize/Bool` | Gains the seven integer impls (R6′); the i8-pattern sign test widens first; `append-digits` (`lib/core/show.sth:73` area) is the shared path |
| `lib/core/bool.sth:4` | `import: intrinsics i \| branch tag drop . \| ;` | Drops `.` |
| `lib/core/bool.sth:6` | `export: Bool False True if unless . ;` | Drops `.` |
| `lib/core/bool.sth:52` | `: . ( Bool -- )` overload | Deleted (probe P4a: layering forces it) |
| `lib/core/prelude.sth:7` | the one-hop note | Rewritten per R16; `export:` (line 16) never named `.` — nothing to trim |
| `lib/hosted/testing.sth:7` | `import: intrinsics \| . \| ;` | Becomes `import: self::show \| . \| ;`; the `expect` body (`lib/hosted/testing.sth:13`) stays byte-identical |

### Migration surface (tests, examples, docs)

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `tests/common/mod.rs:79` | `fixture_imports()` | Emits `import: hosted::show \| . \| ;` when the fixture prints (`tokens.contains(&".")`), does not declare its own `.`, and does not already carry the import — the presence guard is mandatory (as in `bool_imports`): `Scratch::write` routes through `common::fixture_source`, whose harness appends the printing import, so fixture source that also hand-writes the hosted import would trip `duplicate_qualifier_error` (`src/driver.rs:235`) |
| `tests/common/mod.rs:162` | `bool_imports()` | Deleted — the bool-ness heuristic collapses (bools print through the same dot) |
| `tests/common/mod.rs:152`–`:161` | one-hop doc comments | Deleted with the heuristic |
| `tests/common/mod.rs:51` | `fixture_package()` | Gains `depends: hosted path "{}/lib/hosted" ;` |
| `tests/fixtures/sooth.pkg:10` | `depends: core …` | Gains `depends: hosted path "../../lib/hosted" ;` |
| `tests/phase7_slice3d.rs:75`, `:117` | fixture-local `: . ( Bool -- )` copies | Reworked: with `.` an ordinary name, a fixture's own `.` shadows the imported one for bare calls (`src/resolve.rs:420`), so the local overload must be renamed/qualified and the i64 prints given the hosted dot explicitly |
| Raw-source scratch programs | `write_raw`/`Scratch` sites, `SPY_DEF` in 9 files (`phase0.rs:1869,3292`, `phase3_locals.rs:47`, `phase3_refs.rs:57`, `phase4_combinators.rs`, `phase4_generics.rs:23`, `phase4_slice10b.rs`, `phase7_slice3h.rs:153`, `phase7_slice3v.rs:66`, `phase7_slice5_array_drop.rs:56`) | Sources get `import: hosted::show \| . \| ;` (harness-independent; census §3 has per-file counts). **`SPY_DEF` splits in two** (required): the run-witness files (`phase4_slice10b.rs`, `phase7_slice3v.rs`, `phase7_slice5_array_drop.rs` — no `parse_with_core` uses; they build+run through the harness, whose import covers `.`) keep the printing drop and their witness stdout (`"drop 7\ndrop 7\n"` `tests/phase7_slice5_array_drop.rs:69`, `"drop 1\ndrop 2\n"` `tests/phase0.rs:2145`) is unchanged; the check-only consumers get a **silent-drop variant** — they parse in-process against `test_support::parse_with_core`, whose `CORE_SOURCES` is core-only (`src/test_support.rs:30`–`:33`; hosted is unreachable there), so the printing overload would fail as `unknown word '.'` before the pinned diagnostic: `phase0.rs`'s `linear_check_error` (`:1858`; 32 `parse_with_core` uses), plus the `check_error`/`parse_error` helpers of `phase3_locals.rs`, `phase3_refs.rs`, `phase4_combinators.rs`, and `phase4_generics.rs`, plus `phase7_slice3h.rs`'s `emit_il` (`:512`, whose `parse_with_core` at `:515` is the same core-only parse) |
| `src/test_support.rs:26`–`:28` | `CORE_SOURCES` doc comment | Names "the bool `.` overload" — the mention goes stale when R8 lands; update the comment alongside the migration |
| Ad-hoc / pkg-less printing tests | `tests/phase7_slice3e.rs`: `tree_with_core` (`:93`, pkg written at `:96` with `depends: core` only) whose `sort3` fixture (`:608`, six prints) golden `"1\n2\n3\n3\n2\n1\n"` at `:611`; plus two `single_file` printing tests with **no pkg at all** (`single_file` `:108` writes only `main.sth` + an unconditional `import: intrinsics * ;`): `trait_member_with_a_non_trailing_receiver_dispatches` (`:161`, golden `"7\n"` at `:172`) and `trait_member_named_after_a_surface_comparison_dispatches` (`:269`, golden `"5\n"` at `:280`). `tests/phase7_slice3q.rs`: eight pkg-less selective `import: intrinsics … \| . …\|` fixtures (`:225`, `:248`, `:269`, `:280`, `:298`, `:352`, `:402`, `:429`) with run goldens at `:231`, `:271`, `:282`, `:300`, `:354`, `:404`; exactly one in-file `sooth.pkg` write exists (`:368`, the non-printing core-prelude drop test, `depends: core` only) — the in-file precedent | `sort3`'s pkg gains `depends: core hosted` (mirroring `examples/sooth.pkg`); the two `single_file` tests and the eight pkg-less fixtures migrate to the `:368`-style pkg-writing pattern (`depends: core hosted`; stale `import: intrinsics … \| . …\|` lines become the hosted::show selective import, keeping still-needed intrinsics names — `drop`, `add`, `dup`, `>i64` — in their own intrinsics import line); goldens verified unchanged. The verbatim/no-manifest premise stays for tests whose subject is manifest-less import resolution (their prints are dead source — e.g. `phase7_slice3q.rs:334`'s selective-`lw` collision fixture asserts a build error, which fires before word resolution). Where a test's *subject* is the retired surface — `phase7_slice3q.rs:347`'s `a_conversion_intrinsic_re_exports_through_a_hub`, premised on the selective intrinsics import covering the print — die/retarget per R13's pattern |
| Manifest-less / driver-test entries | `tests/phase8_slice2.rs:305`–`:318` (`write_raw` goldens `"3\n"` at `:311` and `:318`), `tests/phase7_slice7b.rs:150`–`:162` (trapping entry prints via intrinsics `.`; no `mod common`) | Gain a manifest with `depends: hosted` or a vehicle swap (e.g. `common::fixture_source`/`manifest_for`); `phase7_slice7b`'s stderr-trap subject is unaffected — its one `@ .` print (`:160`; the `:156` match is the import list) migrates |
| `tests/phase4_slice10c_corpus_stdout.rs:12` | `REGEN_CORPUS_STDOUT=1` | Regeneration gate for the 34 goldens |
| `tests/qbe_baseline.rs:8` | `REGEN_QBE_BASELINE=1` | Regeneration gate for the 34 `.ssa` snapshots (they pin the deleted `$fmt` rows and — after phase 1 — the extern call spellings) |
| `tests/phase4_slice6h_fill_corpus.rs:14` | `FILL_EXAMPLES` | The 14 goldens update only if a fill example prints a Bool (none do per census §2 — verify, don't assume) |
| `examples/*.sth` (42 printing) | — | Gain `import: hosted::show \| . \| ;`; `examples/leap.sth:8` and `examples/array_ctor.sth:16` lose the corebool dot-import + one-hop comments; `examples/tests/*.sth` need nothing (they print only through `expect`) |
| `examples/strings.sth:3` | `extern: strlen ( cstr -- usize ) "strlen"` | Precedent for the cstr dot's strlen extern |
| `examples/mean.sth:9`, `examples/shapes.sth:21`, `examples/poly_if.sth:28` | float print sites | Ride the f64/f32 dots; pinned byte-exact by the probe baselines (`2.5\n`, `12.5664\n12\n5\n7\n`) |
| `README.md:26,130,155,188` | print samples | Doc pass: imports added; `:130`'s user word named `show` footnote-or-rename at implementer's judgement |
| `DESIGN.md:233` | `.` listed among control primitives | Doc pass: updated |
| `docs/book/*` (census §8 list) | printing chapters | Doc pass: print rewrite + import lines; `numbers.md:239` becomes *correct* about Bool casing (R4′) and gains the len-honest str-NUL sentence (R17); pre-existing staleness only where the rewrite touches; `docs/roadmap/P8/dogfood/` (`app/main.sth:6`–`:7`, `scratch.sth:11`) annotated, not migrated |
| `docs/roadmap/P7-language-prereqs.md:1130` | S7d entry | Marked `[ done ]` with deliverables at slice exit |

### Load-bearing constraints

- **Do not remove `.` from `BUILTIN_WORDS`** (`src/ast.rs:1629`): `has_self_tail_call`
  reads it, `check_extern_decls` (`src/check/declarations.rs:38`) rejects an extern
  named `.` with it, and the local-binding collision checks
  (`src/check/terms.rs:158`, `src/check/poly.rs:940`) use it.
- **Do not touch the backend-internal diagnostics** (`src/backend/qbe.rs:894`–`:987`,
  the `$oobfmt`-family data): R1/R2′ scope them out; they are panic/trace paths, not
  user-facing `.`.
- **Keep the IR backend-neutral**: the ABI fix's extern flag records a fact about the
  *callee* (a C function whose prototype Sooth cannot see), not about register classes
  or targets; no `-t` target selection may ride in (the `src/backend/qbe.rs:1237`
  caveat stands).
- **`core::show` stays `no_std`**: the new externs live in `hosted::show`
  (`lib/core/show.sth` gains only impls).
- **No compatibility shim** (R3′): nothing may keep `.` resolving without the import.
- **The linear spine is untouched**: dots consume exactly their operand; nothing
  auto-drops.

## Delivery Plan

Exit-list mapping: revised exit 1, 2, 4, 5 (poly half), 6 → phase 2; exit 3 → phase 2
(examples/harness/corpus/goldens) + phase 3 (doc sites); exit 5 (f64 unit test half) →
phase 1; exit 6 (green gate) is every phase's own exit.

### Phase 1: Fix the user-extern f64-argument ABI (R7′)

- **Goal**: A user extern declaring an f64 parameter delivers that double to the C
  callee — the snprintf control program prints `2.5\n` instead of `0\n`.
- **Requirements Covered**: R14.
- **Scope**:
  - `src/ir/types.rs:524` (`Arity`): add the extern-callee flag.
  - `src/ir/driver.rs:167` (extern env registration): set it; `:149`/`:304`
    (user-word and monomorphized entries) and `src/ir/func_builder/mod.rs:1000` (test
    env) set it false.
  - `src/ir/func_builder/calls.rs:893` (`emit_user_call()`): emit the marked call.
  - `src/backend/qbe.rs:1162` (`emit_instr()` `Instr::Call` arm): spell extern calls
    all-args-variadic (`call $sym(..., args…)`); leave user-word calls fixed; add a
    unit test beside `emit_instr` (`src/backend/qbe.rs:994`) pinning the extern form
    for an f64-taking extern against a user call without the marker.
  - Create `tests/phase7_slice7d.rs` (pattern: `tests/phase7_slice7c.rs:14`): the P8
    probe shape (`extern: g-fmt ( &!array[u8 64] usize cstr f64 -- i32 ) "snprintf"`)
    builds and prints `2.5\n`.
  - Regenerate `tests/qbe_baseline/*.ssa` (`REGEN_QBE_BASELINE=1`) — at least
    `strings.ssa` and `resources.ssa` change extern call spellings (the only examples
    declaring `extern:`; `modules.sth` declares none, so `modules.ssa` cannot change) —
    and review the diff as intended-to-change.
  - Out of bounds for this phase: no library changes, no `Instr::Print` changes
    (phase 2 deletes it), no target-selection work (`src/backend/qbe.rs:1237` caveat
    stands), no libm linking.
- **Entry Conditions**: The tree is green at the spec's code-state `HEAD` (`790b81c`;
  the docs-only commit `a02f2f2` on top does not touch code); the probe
  captures in slice7d-probes.md (P8) are the behavioral reference.
- **Exit Criteria / Verifiable Artifacts**:
  - `tests/phase7_slice7d.rs` passes with `2.5\n` (end-to-end, exact bytes).
  - The new `qbe.rs` unit test passes, pinning the extern-call IL form.
  - `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green with the
    regenerated baselines committed.
- **Parallelism**: SEQUENTIAL, first — its float-dot consumer (phase 2) depends on it
  or on its recorded descope, and it shares `src/backend/qbe.rs` and the qbe_baseline
  set with phase 2, so concurrent landing would collide.
- **Relative Effort**: S — one flag threaded through one lowering path plus one emit
  arm; the diagnosis is done and recorded in Design ruling 6.
- **Difficulty**: `hard` — variadic C-ABI codegen; a wrong spelling compiles, links,
  and silently misprints (exactly today's bug), so the regression tests are the
  deliverable, not a formality.
- **Open Questions / Blockers**: If the fix balloons beyond the flag + spelling
  (e.g. QBE rejects the form on some construct), invoke R7′'s fallback: record the ABI
  bug and the pure-Sooth `%g` alternative as follow-ups and tell phase 2 to descope
  f32/f64 dots to the named `no overload of '.'` error (probe P6), migrating the five
  float-printing sites to the descope (`examples/mean.sth:9`, `examples/shapes.sth:21`,
  `examples/poly_if.sth:28`, `tests/phase0.rs:955`, `tests/symbol_hijack.rs:129`).
  This decision must be made before phase 2 starts.

### Phase 2: Retire `.` — compiler deletion, `hosted::show`, and the program-wide migration

- **Goal**: `cargo run -- build examples/gcd.sth` (and every example) compiles only
  with `import: hosted::show | . | ;` in source, prints today's bytes except R12's two
  named output changes (Bool lowercase, trace-line reorder), and the compiler carries
  no print intrinsic; the full test suite is green on regenerated goldens.
- **Requirements Covered**: R1–R13, R16, R17, R18.
- **Scope** (atomic — no intermediate state compiles, R3′/ruling 9):
  - Compiler deletions per the Codebase Map's first table: `src/ast.rs:1706`
    (`is_name_dispatched_builtin`) + `:3100` (gate-set test), `src/resolve.rs:72`
    (`is_operator_dispatch_name`), `src/check/builtins.rs:94,130,197,561`,
    `src/check/operators.rs:106,114,349,459` + the eight tests,
    `src/ir/types.rs:423`, `src/ir/func_builder/calls.rs:639`,
    `src/backend/qbe.rs:72,1246` + the `emit_print_*` tests, and the IR test retargets
    (`calls.rs:1295`, `quotation.rs:755`, `destructors.rs:549`/`test_helpers.rs:31`).
  - `lib/hosted/show.sth` created (probe candidate source in slice7d-probes.md is the
    verified starting point; add the u16/i16/u32/i32 dots per R6′ and the cstr dot per
    R5′ via the strlen extern pattern of `examples/strings.sth:3`); registered in
    `lib/hosted/sooth.pkg:6-7`.
  - `lib/core/show.sth:79`–`:136`: the seven widened impls (P7 widen-first gotcha).
  - `lib/core/bool.sth:4,6,52` trimmed/deleted; `lib/core/prelude.sth:7` note rewritten;
    `lib/hosted/testing.sth:7` import migrated (R9: the expect body must stay
    byte-identical).
  - Harness: `tests/common/mod.rs:79,51,162,150` and `tests/fixtures/sooth.pkg:10`
    (R11); `examples/*.sth` per census §2 (R10); raw scratch programs and `SPY_DEF`
    per census §3 — `SPY_DEF` splits per the map's row (silent-drop variant for the
    `parse_with_core` check-only consumers; run-witness files keep the print);
    the ad-hoc/pkg-less printing tests (`phase7_slice3e.rs:93` and its two `single_file`
    tests, `phase7_slice3q.rs`'s eight pkg-less dot fixtures) migrate to the pkg-writing
    pattern per the map's row, and the manifest-less entries (`phase8_slice2.rs:305`,
    `phase7_slice7b.rs:150`) gain `depends: hosted` or a vehicle swap; `tests/phase7_slice3d.rs:75,117` reworked
    (own-module `.` now shadows imports); `tests/phase7_slice3i.rs:157,167,207`
    die/retarget/migrate (R13).
  - Goldens: `REGEN_CORPUS_STDOUT=1` (34), fill_corpus review, `REGEN_QBE_BASELINE=1`
    (34) — diffs reviewed, and the only expected output deltas are R12's two (Bool
    lowercase, trace-line reorder) plus phase-1's extern spellings; the inline
    `True\n`/`False\n` assertion literals (34, across 14 files) flip with them (R12).
  - New coverage beside the changed stage functions: the gate-set test flip (R1), and
    file-based goldens in `tests/phase7_slice7d.rs` for the all-paths dot baseline
    (probe od), the P3f import shape, and the P6 no-overload diagnostic (a type with
    no dot, e.g. an array, with the import present); plus the str-NUL pin (R17b:
    `"ab\\0cd" .` prints all 5 bytes) and a witness-stdout check that the SPY_DEF
    split changed no run-witness output.
  - R18 verification recorded in the phase notes: a grep over the migrated corpus
    confirming no `.` call inside a poly body; if one exists, stop and pull the
    checker fix in (R1′ trigger).
  - Out of bounds for this phase: the backend-internal diagnostics
    (`src/backend/qbe.rs:894`–`:987`), the S7c `Write`/`Stdout` surface
    (`lib/hosted/libc.sth`), the generic-dot checker work (unless R18 fires), the
    diagnostics track (P5/P6 messages), `lib/core/show.sth`'s `no_std` line, and any
    shim keeping import-less `.` working.
- **Entry Conditions**: Phase 1 landed (or its descope decision recorded per its Open
  Questions), i.e. the float-dot strategy for this phase is known.
- **Exit Criteria / Verifiable Artifacts**:
  - Revised exit 1: greps confirm the delete list is gone and `.` remains only in
    `BUILTIN_WORDS` (see Success criteria).
  - Revised exit 2: `tests/phase7_slice7d.rs`'s new goldens pass — the all-paths
    baseline (od-verified bytes, Bool lowercase), the bare-call import shape, and the
    no-overload diagnostic.
  - Revised exit 3: every committed example builds **and runs** via an explicit
    pass over `examples/*.sth` (build each, run where a golden pins the output) —
    note `examples/tabulate.sth` (five `@ .` print sites) is covered by **no** test
    today (`grep -rn tabulate tests/` finds only slice5's own Spy arrays), so this
    step is its only verification; fold it into a suite or let this step bind it.
    `cargo test`'s corpus/fill/qbe suites are green on regenerated goldens;
    `cargo run -- test examples/tests/bool.sth` output is
    byte-identical to the P10 capture (`ok   examples/tests/bool.sth` /
    `1 entries, 0 failed (4 ok, 0 not ok assertions)`).
  - Revised exit 4: the prelude note and all one-hop comment copies are gone
    (`grep -rn "one hop" lib/ examples/ tests/common/` is empty or documents only the
    new wording).
  - Revised exit 6: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
    green.
  - The phase notes record the R18 poly-body verification result.
- **Parallelism**: SEQUENTIAL after phase 1 (float dots consume the ABI fix or its
  descope; shared files `src/backend/qbe.rs` and the qbe_baseline set).
- **Relative Effort**: L — the compiler delta is moderate but the program-wide
  migration spans ~715 print sites across 68 test files, 42 examples, 9 `SPY_DEF`
  files, and three golden sets; the harness absorbs most of it, the rest is census-
  enumerated hand work.
- **Difficulty**: `hard` — cross-cutting refactor through shared control flow
  (name dispatch, rewrite, table, lowering, emit), semantic shifts with observable
  diagnostics (own-module `.` shadowing), and a no-shim atomicity constraint that
  forbids incremental landings.
- **Open Questions / Blockers**: Phase 1's descope decision (R7′ fallback) must be
  resolved before this phase's float-dot work starts. None otherwise — the probes
  settled the module shape, the import shape, the newline contract, and the Bool
  casing.

### Phase 3: Docs — README, book, and roadmap annotations

- **Goal**: Every document that shows printing shows the migrated program — `import:
  hosted::show | . | ;` present, Bool examples lowercase — and a reader of README/the
  book can compile what the docs print.
- **Requirements Covered**: R15.
- **Scope**:
  - `README.md:26,130,155,188` (print samples gain the import; the `:130` user word
    named `show` footnote-or-rename at implementer's judgement, per the brief's
    deferral).
  - The book chapters of census §8 — `getting-started.md`, `numbers.md:239`,
    `the-stack.md`, `branching.md`, `control-flow.md`, `move-by-default.md`,
    `quotations-and-loops.md`, `why-this-works.md`, `preface.md` — print rewrite only:
    import lines added, casing updated; pre-existing staleness (REPL, `else`/`end`,
    the nonexistent `examples/print-if-positive.sth`) corrected or annotated only
    where the print rewrite touches it.
  - `DESIGN.md:233` (the `.` mention among control primitives) updated.
  - `docs/roadmap/P8/dogfood/` (`main.sth:6`, `scratch.sth:11` per census §8):
    annotated as no-longer-compiling, not migrated.
  - `docs/roadmap/P7-language-prereqs.md:1130`: S7d marked `[ done ]` with
    deliverables and the follow-up ledger (generic dot, diagnostics, `%al` on other
    targets).
  - Out of bounds: no library/compiler/test changes; no new staleness fixes beyond the
    print rewrite's reach.
- **Entry Conditions**: Phase 2 complete (the docs describe the landed reality and
  must compile against it).
- **Exit Criteria / Verifiable Artifacts**:
  - Every `.sth` sample block in README/the touched book chapters carries the
    `hosted::show` import where it prints; Bool samples show `true`/`false`.
  - `docs/roadmap/P8/dogfood/` carries the annotation.
  - The roadmap entry reads `[ done ]`.
  - `cargo fmt --check && cargo clippy -- -D warnings && cargo test` still green
    (docs-only diff).
- **Parallelism**: SEQUENTIAL after phase 2 (its content is phase 2's output; no
  genuine parallelism exists in this slice — phase 1 must precede phase 2 for the
  float dots and the shared backend files, and phase 3 must follow phase 2).
- **Relative Effort**: M — nine book chapters plus README plus annotations; the census
  §8 line list makes it mechanical, but each sample must actually compile against the
  migrated library.
- **Difficulty**: `standard` — prose and sample migration with a clear file list; no
  compiler or concurrency surface.
- **Open Questions / Blockers**: None identified.

### Parallelism Summary

- Phase 1 → Phase 2 → Phase 3, strictly sequential: phase 2 consumes phase 1's fix
  (or its recorded descope) and shares `src/backend/qbe.rs` + the qbe_baseline set;
  phase 3 documents phase 2's landed reality. No two phases of this slice can run
  concurrently without colliding on shared files or describing a tree that does not
  exist.

### Effort Summary

- Phase 1: S (hard) — the ABI fix with its regression tests.
- Phase 2: L (hard) — the atomic retirement + program-wide migration + goldens.
- Phase 3: M (standard) — the doc pass.
- Total: S + L + M; the corpus migration dominates and is irreducible per R3′.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "user-extern-f64-abi-fix", "effort": "S", "difficulty": "hard" },
    { "phase": 2, "focus": "retire-dot-intrinsic-and-migrate-programs", "effort": "L", "difficulty": "hard" },
    { "phase": 3, "focus": "print-site-docs-migration", "effort": "M", "difficulty": "standard" }
  ]
}
```
