> Pre-spec census for P7.S7d, run 260830 against the current tree (worktree
> `p7-s7d`, branch `vietnamese`) by one read-only subagent: every `.` print
> site, type coverage, layering conflicts, import/export surface, the
> `sooth test` driver contract, and doc debt. Companion to the compile-probe
> round in [slice7d-probes.md](./slice7d-probes.md). Nothing in the tree was
> modified.

# P7.S7d census: what prints, what migrates

## 1. Library-internal print sites (the hardest migrations)

| Site | Operand type | Newline reliance |
| --- | --- | --- |
| `lib/hosted/testing.sth:15` — `~[ "ok -- " . label . "\n" . ]` | `str` ×3 | str prints exact bytes; `\n` spelled explicitly. **This emits the TAP lines the driver parses.** |
| `lib/hosted/testing.sth:16` — `~[ "not ok -- " . label . "\n" . ]` | same | same |
| `lib/core/bool.sth:53` — `~[ ( False ) drop "False\n" . ]` | `str` literal | newline inside the literal, capitalized spelling |
| `lib/core/bool.sth:54` — `~[ ( True ) drop "True\n" . ]` | same | same |

`lib/core/show.sth`, `cmp.sth`, `combinators.sth`, `option.sth`, `result.sth`,
`prelude.sth`, `libc.sth`: zero print sites (verified by per-file grep).

## 2. Examples (42 of 48 print; all numeric sites rely on the baked-in `\n`)

Dominant type i64. Non-i64 sites:

- **u8**: `examples/refs.sth:30,31` — **usize**: `refs.sth:32`, `stack.sth:48`,
  `strings.sth:8,9` — **isize**: `resources.sth:21`
- **Bool**: `array_ctor.sth:42-45` (×4), `leap.sth:19-21` (×3, imports
  `core::bool` directly for `.` per the one-hop rule)
- **i8**: `array_ctor.sth:71` (`probe_i8 .` second dot; golden pins `5\n0`)
- **str**: `traits.sth:64` (×3 literals interleaved with field reads)
- **f64**: `shapes.sth:21`, `poly_if.sth:28`, `mean.sth:9` (the dedicated
  float-print dogfood)

No-print examples (7): `modules_ops.sth`, `modules_point.sth`, and all five
`examples/tests/*.sth` (print only through `expect`/`expect-eq`).

## 3. Rust test corpus (~715 print-shaped sites across 68 files)

- Dominant shapes: `@ .` (95), `call .` (45), destructure-then-print (60+),
  numeric literals (~90), `add .` (25), `>i64 .` (16).
- **`SPY_DEF` in 9 files** (`phase0.rs:1869,3292`, `phase3_locals.rs:47`,
  `phase3_refs.rs:57`, `phase4_combinators.rs`, `phase4_generics.rs:23`,
  `phase4_slice10b.rs`, `phase7_slice3h.rs:153`, `phase7_slice3v.rs:66`,
  `phase7_slice5_array_drop.rs:56`): `: drop ( Spy -- ) | s | "drop " . s Spy> . ;`
  — str + i64 prints inside a drop overload; every drop-order witness migrates.
- **Bool prints ~76 sites**, all riding the harness auto-import (§6).
- **str/cstr prints**: `phase3_strings.rs:96,106,118,154,141-160,161-175`
  (pins no-trailing-newline, escapes, interior-NUL truncation via the cstr
  row, struct-field round trip), `phase7_slice3v.rs:126,147`,
  `phase7_slice3r.rs:171`.
- Narrow/float conversion prints: `phase0.rs:921,938,955,1239,1270`
  (`>u8 >u32 >u64 >f32 >f64 >usize >isize`), `2.5 .` (`phase0.rs:955`),
  `symbol_hijack.rs:129` (`6.0 V div &x @ swap drop . 9.0 3.0 div .`).
- **Fixture-local `: . ( Bool -- )` overload copies**: `phase7_slice3d.rs:75,117`.
- Diagnostic-text pin: `tests/phase7_slice3i.rs:157` asserts `` `.` requires a
  printable scalar, found `Bool` `` — dies with the intrinsic.
- Per-file counts: phase0.rs 100, phase3_refs.rs 57, phase4_combinators.rs 48,
  phase4_generics.rs 46, phase4_modules.rs 45, phase7_slice12.rs 30,
  phase4_quotations.rs 28, … remaining ~35 files ≤8 each.

## 4. Goldens

- `tests/corpus_stdout/*.txt` — 34 goldens, byte-identical assert
  (`phase4_slice10c_corpus_stdout.rs:59`), regenerable `REGEN_CORPUS_STDOUT=1`.
- `tests/fill_corpus/*.stdout` — 14 goldens (`phase4_slice6h_fill_corpus.rs:41`).
- `tests/qbe_baseline/*.ssa` — 34 emitted-IL snapshots pinning the
  `call $printf(l $fmt …)` sequences and `$fmt`/`$ufmt`/`$ffmt`/`$sfmt`/
  `$strfmt` data rows the delete list removes (regenerable, "intended to change").

## 5. Type coverage vs `Show` impls

`lib/core/show.sth` impls: i64, usize, isize, Bool only. Printed today with no
impl: **str, cstr, f32, f64, u8, i8, u32, u64** (live sites above); i16, u16,
i32 have no corpus sites but print today.

## 6. Layering and import/export surface

- `lib/core/bool.sth:52-55` **cannot survive**: post-retirement its body's
  inner str `.` cannot resolve from the core layer. Its `import: intrinsics i
  | branch tag drop . | ;` (line 4) and `export: Bool False True if unless . ;`
  (line 6) both name `.`.
- `lib/hosted/testing.sth:7` — `import: intrinsics | . | ;` (the other
  selective intrinsics import naming `.`). All other intrinsics imports are
  `*`.
- Harness auto-import: `tests/common/mod.rs:193` `bool_imports` emits
  `import: core::bool corebool | . | ;` whenever a fixture prints and produces
  a bool; its doc comment (118-123, 162-165) states the one-hop rule. Both the
  heuristic and the comment are migration surface; `tests/fixtures/sooth.pkg`
  needs `depends: hosted` (fixtures are already `layer: hosted`).
  `examples/sooth.pkg` already depends on core+hosted.
- Prelude note, `lib/core/prelude.sth:7-11`: the one-hop paragraph dies with
  the operator-overload special case; `export:` line 16 does not name `.`.
- Stale-comment copies of the one-hop rule: `examples/leap.sth:8-9`,
  `examples/array_ctor.sth:16-18`.
- Compiler-internal unit tests on the deleted surface:
  `src/check/builtins.rs:561`, `src/check/operators.rs:508,840,844,1002,1013,
  1041,1048,1055`.

## 7. `sooth test` driver contract

`src/driver/toolchain.rs`: the entire output contract is `count_protocol`
(139-151) — a line passes iff it starts with `ok --`, fails iff
`not ok --` (checked first). Nothing else in stdout is parsed. An entry fails
on build failure, non-zero exit, or ≥1 `not ok --` line. `expect-eq` never
prints values. So the driver constrains only that `expect`'s three str prints
stay byte-exact — the str path is load-bearing for the whole TAP suite.

## 8. Docs

- `README.md:26,130,155,188` (line 130 defines a user word named `show` —
  rename candidate or footnote once `show` is a trait member name).
- `docs/book/`: printing demonstrated in nearly every chapter;
  `getting-started.md:37,66-75,91,102,114,132,166` (documents the removed
  REPL as live), `numbers.md:239-256` (the full print contract — **already
  wrong about bool casing**: claims lowercase `true`/`false`, compiler prints
  capitalized), `the-stack.md:48,77,90-91,171,190`, `branching.md:74,77,80,
  98-105` (references `examples/print-if-positive.sth`, which does not exist),
  `control-flow.md:39,100,169` (`else`/`end` syntax no longer parses),
  `move-by-default.md:20,33,76,83,101`, `quotations-and-loops.md:24,35,51,
  116,266,305`, `why-this-works.md:13`, `preface.md:47`.
- `docs/roadmap/P8/dogfood/` prints `bool`/i64 but no longer compiles
  (`main.sth:6,7`, `scratch.sth:11`) — annotation targets, not migration.
