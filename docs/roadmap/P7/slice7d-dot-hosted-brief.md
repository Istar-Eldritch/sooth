# Phase 7 Slice 7d: retire the compiler-intrinsic `.`, in favor of a `hosted` word (brief)

`.` is a compiler-injected, name-dispatched builtin (`is_name_dispatched_builtin`,
`resolve.rs`) that lowers straight to libc `printf`/`dprintf` regardless of which
package calls it — an OS dependency baked into the compiler itself, invisible to
the layer system every other hosted capability goes through `depends:` to reach.
S7c gives `Show`/`Write` a real, layered home for printing; this slice moves `.`
onto that path and deletes the special case.

Ordered last among the S7 subslices because it's the only one that touches the
compiler rather than the standard library, and every other S7 deliverable
(`exit`, `expect`/`expect-eq`, `Show`/`Write`) works whether or not this one ships
— nothing here blocks S7a–S7c, and if this slice turns out larger than expected it
can slip without stalling the testing vocabulary that motivated the whole split.

## Design rulings

### R1 — `.` becomes `hosted::show`'s word, not a new compiler primitive

`. ( 'T:Show -- )` is an ordinary `hosted` word, two steps under the hood: `show` the
value into a fresh `StrBuf` (`'T` resolved through `Show` (S7c) the same way any other
trait member call resolves), then flush the buffer through `Write` (`&!Stdout` receiver;
S7c's `Write for Stdout` and the `sys-write` extern live in `hosted::libc`).
Every existing call site (`42 .`, `core::bool`'s `.` overload, the QBE emitter's own
diagnostic/trace paths at `qbe.rs:892-1283` that currently call `printf` directly)
either goes through the new word or, for the compiler's own trap/OOM/bounds
diagnostics, stays a direct `printf`/`dprintf` call — those are backend-internal
panic messages, not user-facing `.`, and are explicitly not in scope for
rerouting through a user-level trait.

### R2 — What actually gets deleted

`is_name_dispatched_builtin`'s `.` arm, the `resolve.rs` special case that lets
`.` bypass ordinary env lookup, and whatever check.rs/lowering path currently
special-cases `.`'s effect. `core::bool`'s existing `.` overload note in
`prelude.sth` ("One of `core::bool`'s names does not appear here... an operator
overload's candidate lookup considers the calling module and the module it
selectively imported the name from") needs re-verification once `.` is an
ordinary word going through the same operator-dispatch machinery as everything
else it documents — confirm the one-hop rule still produces the same outcome
before assuming the note's reasoning survives unchanged.

### R3 — Migration is program-wide, not opt-in

Once this slice lands, every program that prints must `depends: hosted` and
`import: hosted::show ...`. No compatibility shim keeps `.` working without the
import — CLAUDE.md's magicless-over-convenience rule applies directly: a program
that types a value into existence without importing anything is exactly the
implicit behavior the rest of Sooth refuses. `examples/*.sth` and every golden
that currently prints without an explicit import migrates in this slice, not
gradually.

## Out of scope

- Any sink beyond `Stdout` (S7c's scope, unchanged here).
- The backend's own internal `printf`/`dprintf` diagnostics (trap messages, OOM,
  bounds) — R1.

## Exit

1. `is_name_dispatched_builtin` and the `resolve.rs`/check.rs special cases for
   `.` are deleted; `.` resolves as an ordinary `hosted::show` word.
2. Every example, golden, and test that prints imports `hosted::show` explicitly;
   none compile via an implicit intrinsic.
3. `core::bool`'s `.` overload note in `prelude.sth` is re-verified or corrected
   (R2).
4. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.

## Addendum — probe/recon round for S7d (260830): rulings revised

Run 260830 in this worktree: one read-only census ([slice7d-census.md](./slice7d-census.md)) and one disposable compile-probe round ([slice7d-probes.md](./slice7d-probes.md) — the `.` intrinsic deleted in-place, candidate `hosted::show` shapes iterated, all edits reverted). The findings revise R1–R3; the rulings below supersede them where they conflict.

### R1′ — the landing shape is per-type concrete dots, not the generic word

R1's `: . ['T: Show] ( 'T -- )` **parses and registers but its body is unwriteable today** (probe P2): (a) a local-borrowed `&!StrBuf` does not unify with a poly callee's declared `&!StrBuf` — `parser.rs:4117` (Slice 13 R-A4) folds a fully-concrete `&!T` slot to `PolyType::Concrete(Type::Ref(…))` while local borrows produce native `PolyType::Ref`, so `poly_cross_match`'s Ref arm sees a look-alike mismatch (`render` expected `&!StrBuf`, found `&!StrBuf`); (b) field accessors (`&!len`, `&!data`) and `+!` are located errors in generic bodies (`poly_unsupported_accessor_error`). Even in mono bodies, the read-modify-write append over one locally-built StrBuf hits borrow-alias records (P2e/P2h).

What the probes proved instead (P3): **several same-arity concrete `: . ( T -- )` candidates in ONE module are legal, and a caller importing that one module dispatches per-site on the bare call** (P3f: `42 . / -7 . / "hi" . / True . / 100000 >usize .` all resolve). A generic and a concrete candidate cannot mix in one module (`a name cannot mix a generic and a concrete candidate`), and two modules both exporting `.` collide for an importer. **Ruling: `hosted::show` lands one concrete dot per printable type** (each body: fresh `StrBuf`, `render`, `flush` through `Stdout`, then `"\n"` via the str path — byte-identical to today's baked-in newline, and it dodges the in-buffer append friction), with internal delegation through distinctly-named private helpers (intra-module bare cross-overload calls don't resolve, P3e). The generic dot is a recorded follow-up, not a blocker: it needs the declared-ref unfold at `parser.rs:4117` (or normalization at the match sites) and a ruling on poly-body accessors — both pre-existing poly-body limitations, P7.S3-family, filed as their own item. Verification duty for the implementer: grep the corpus for printing inside poly bodies (census found none — `expect-eq` never prints values, combinator splices check against the caller's concrete stack); if one exists, it forces the checker fix into this slice.

### R2′ — the delete list, confirmed and extended

All of the 260829 addendum's list, plus:

- **`src/resolve.rs`'s operator-name predicate is a fourth deletion site the original R2 missed** (probe P3f): with `.` still listed in `is_operator_dispatch_name`, a selectively imported `hosted::show` `.` stays unrewritten expecting builtin dispatch and every bare call is `unknown word '.'`. With it dropped, the P3f import shape works.
- `.` stays in `BUILTIN_WORDS` (self-tail-call detection and the S3r R4 trait-member-name rejection still want it), exactly like the six surface comparisons: excluded from `is_name_dispatched_builtin`, kept in the list.
- `core::bool`'s overload is **deleted, not migrated** — layering forces it (probe P4a: post-retirement its body's inner str `.` cannot resolve from the core layer). Its `import: intrinsics i | branch tag drop . | ;` (line 4) and `export: … . ;` (line 6) are trimmed.
- Unit tests on the deleted surface die with it: `src/check/builtins.rs:561`, `src/check/operators.rs:508,840,844,1002,1013,1041,1048,1055`, and the diagnostic pin `tests/phase7_slice3i.rs:157`.

### R3′ — migration mechanics, verified

The working import shape (P3f, bonus probe): `import: hosted::show | . | ;` + bare calls, per-site dispatch across the concrete candidates. Migration details the census pins down:

- Two selective intrinsics imports name `.` (`lib/hosted/testing.sth:7`, `lib/core/bool.sth:4`) and one export list (`bool.sth:6`); a stale `import: intrinsics | . | ;` is **silently ignored** and the failure surfaces as a bare `unknown word '.'` at the call site with no hint (probe P5) — the lines must be fixed, and the diagnostic gap goes to the cross-cutting diagnostics track, not this slice.
- `tests/common/mod.rs`'s `fixture_imports`/`bool_imports` (line 193) are migration surface: post-retirement every printing fixture imports `hosted::show`, the bool-ness heuristic collapses (bools print through the same `.`), and the one-hop-rule doc comment (118-123, 162-165) dies. `tests/fixtures/sooth.pkg` needs `depends: hosted` (fixtures are already `layer: hosted`); `examples/sooth.pkg` needs nothing.
- ~715 test-side print sites across 68 files, `SPY_DEF` in 9 files, 34 `corpus_stdout` + 14 `fill_corpus` goldens (regenerable) and 34 `qbe_baseline` .ssa snapshots (regenerable, and they pin the deleted `$fmt` data rows).
- Docs: README (4 sites) and the book's printing chapters migrate in-slice; the book's pre-existing staleness (REPL alive, `else`/`end`, the nonexistent `examples/print-if-positive.sth`) is corrected or annotated only where the print rewrite touches it. `docs/roadmap/P8/dogfood/` no longer compiles — annotate, don't migrate.

### R4′ — Bool prints lowercase; the capitalization change is accepted

With `core::bool`'s overload deleted, `True .` goes through `Show for Bool` → `true\n` (probe P4b) vs today's `True\n`. Accepted deliberately: `docs/book/numbers.md:239-256` already documents lowercase `true`/`false` — the *library overload* was the deviation, and S7c's `Show for Bool` already committed to the lowercase spelling. Goldens update accordingly; the ~76 bool print sites ride the harness rewrite.

### R5′ — newline contract: byte-compatibility, spelled as a second write

Numeric and Bool dots append the newline; str and cstr dots print exact bytes with none — reproducing today's per-type behaviour (`%ld\n` vs `%.*s`) so all 48 goldens and `expect`'s TAP spellings migrate without output changes (the driver's `ok --`/`not ok --` prefix parse is the only output contract, `src/driver/toolchain.rs:139-151`). Implementation note from P2e/P2h: the newline is spelled as a second `write(2)` of `"\n"` through the str path, **not** an in-buffer append (borrow-alias friction blocks the read-modify-write append over one locally-built StrBuf; a `core::show` append helper could revisit this later). The cstr dot keeps today's `%s` terminator-bound semantics via a `strlen` extern (already demonstrated as a user extern in `examples/strings.sth`). All paths are `write(2)`, so the buffered-stdio interleaving hazard is gone by construction (probe P9: strict call-order interleaving).

### R6′ — `Show` impls widen to the integer tower

S7c's impl set (i64, usize, isize, Bool) leaves eight printable types without a Show path, six with live corpus sites. **Ruling: `core::show` gains `impl: Show for` u8, u16, u32, u64, i8, i16, i32** — trivial widenings onto the existing `append-digits` path (probe P7 verified u8/i8/u64 byte-exact). Gotcha recorded: the i8 sign test must widen first (`n >i64 0 lt`; a bare `n 0 lt` resolves the literal ambiguously and errors). str/cstr stay out of `Show` (D3 stands) — they are concrete dots only. All dots delegate through `render`, so each new impl needs its matching dot.

### R7′ — floats: fix the user-extern f64 ABI in-slice, print via hosted `snprintf`

Probe P8: the pure-user-land shape compiles and runs — `extern: g-fmt ( &!array[u8 64] usize cstr f64 -- i32 ) "snprintf"` — but **the f64 argument arrives as 0** (snprintf returned 2, wrote `"0\n"`). `Instr::Print`'s float arm passes `d` args to variadic `printf` correctly, so the ABI is expressible in QBE and this is a diagnosable compiler gap in user-extern call lowering, not a language limit. (A libm control failed to *link* — libm isn't linked for user programs — but `snprintf` is libc and links fine, so the float path needs no libm story.) **Ruling: S7d includes the user-extern f64-argument ABI fix** (with `Instr::Print`'s correct `d`-form as the reference and a regression test), then floats print through hosted concrete dots over `snprintf` with `%g\n` — byte-identical to today's `$ffmt`. This is a pre-existing backend bug any user extern taking f64 would hit; it is in-scope because without it the intrinsic retirement silently drops a documented, dogfooded capability (`mean.sth` exists to exercise float printing; `numbers.md` documents `%g`). **Fallback if the fix balloons**: descope f32/f64 dots to a named compile error (`no overload of '.' accepts these operands`, probe P6), record the ABI bug and the pure-Sooth `%g` alternative as follow-ups, and migrate the four float-printing corpus sites to the descope.

### Exit (revised)

1. The delete list of R2′ is gone from the compiler: builtin rows, the `check_operator` `.` arm, `is_name_dispatched_builtin`'s and `resolve.rs` `is_operator_dispatch_name`'s `.` entries, `Instr::Print`'s user-facing arms, the `$fmt`/`$ufmt`/`$ffmt`/`$sfmt`/`$strfmt` data, `printable_types`; `.` stays in `BUILTIN_WORDS` only. `core::bool`'s overload is deleted.
2. `hosted::show` provides one concrete `.` per printable type (integers via `Show` impls per R6′, `str`/`cstr` via the write(2)/strlen paths per R5′, floats per R7′), registered in `lib/hosted/sooth.pkg`; bare calls resolve per-site after one selective import (P3f shape).
3. Every example, golden, harness fixture, and doc site that prints imports `hosted::show` explicitly (census §2–§4, §8); `sooth test` output is byte-identical except the accepted Bool lowercase change (R4′); goldens regenerated (`REGEN_CORPUS_STDOUT=1`, fill_corpus, qbe_baseline).
4. The prelude note (R2) is rewritten — the one-hop paragraph dies; the harness's one-hop comment and the leap/array_ctor comment copies go with it.
5. A unit test covers the user-extern f64-arg ABI fix (R7′); the corpus has no printing inside poly bodies (verified, or the generic-dot checker fix is pulled into the slice — R1′).
6. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.

### Explicitly deferred (recorded, not owed by S7d)

- The generic `: . ['T: Show] ( 'T -- )`: the `parser.rs:4117` declared-ref fold and poly-body accessor/`+!` support — P7.S3-family poly-borrow work, its own item (R1′).
- Diagnostics: the silently-ignored stale `import: intrinsics | . |`, the hintless `unknown word '.'`, and the `no overload of '.'` message naming no fix (P5/P6) — cross-cutting diagnostics track, beside S8's unsatisfied-`Ord` attribution.
- README's user word named `show` colliding with the trait member name (census §8) — rename or footnote at the implementer's judgement; not a compiler concern.

## Addendum — probe/recon round for S7c (260829)

Facts S7d's spec will need, from the S7c probe round
([slice7c-probes.md](./slice7c-probes.md)):

- **The delete list is confirmed and larger than R2 listed.** All 14
  `builtin_table` rows over `printable_types()` (12 numerics + `str` +
  `cstr`; `bool` deliberately has no row — `core::bool`'s `: . ( Bool -- )`
  overload is reached via the `builtin_overloads` exact-miss path and
delegates to the `str` row), `printable_types` itself,
  `BuiltinLower::Print`/`Instr::Print`'s user-facing arms, and the
  `$fmt`/`$ufmt`/`$strfmt` data — including the **newline baked into the
  numeric format strings** (`%ld\n`). The `str` arm prints with `%.*s`, no
  newline. Any `hosted::show` replacement must reproduce or consciously
  change the per-type newline behaviour (`expect`'s `"\n" .` spellings
  depend on it).
- **Do not implement the sink on the `str` `Print` arm** — S7c's sinks bind
  to `write(2)` at the `extern:` boundary instead (`extern: sys-write ( i32
  &!array[u8 64] usize -- isize ) "write"` — the binding is named `sys-write` because the
  `Write` member `write` shadows a same-named extern inside the impl, and the array mode
  is mutable because the buffer arrives as `&!StrBuf`), because a `str` value can only
  ever be a literal or a static and has no extern ABI at all.
- **Output ordering becomes observable.** `.` rides buffered stdio;
  `write(2)` does not. After retirement all printing is syscall-ordered — a
  behaviour change worth a golden (probe P6c demonstrated the interleaving
  hazard directly).
- **Diagnostics warts observed during the probes** (candidates for the
  cross-cutting diagnostics track, not this slice): every compiler error
  prints as `error: error: …` (`src/main.rs` wraps an already-prefixed
  message), and `extern:` boundary errors display mangled names
  (`emit__m0`).
- **S7c descopes `Show for str` (D3, settled 260829):** every `str` is
  literal- or static-rooted, so S7d prints strings through `cstr` at the
  boundary rather than through `Show`. S7d's `hosted::show` dispatch is
  therefore: `str` → the `cstr` path, everything else → `Show` + `Write`.
  R1's "`. ( 'T:Show -- )`" needs that carve-out spelled in the S7d spec.
