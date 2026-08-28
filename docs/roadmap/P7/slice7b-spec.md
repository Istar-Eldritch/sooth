## Problem

Sooth could build and run programs but could not test Sooth code *in Sooth*. Two halves,
neither needing a type-system or backend change: an assertion vocabulary the language
already had every prerequisite for (`Ord` dispatch, enum elimination, `Bound::User`
generics, string printing through the intrinsic `.`), and a driver subcommand that
discovers, builds, and runs test programs the way `sooth build`/`run` already did, then
reads their stdout as a pass/fail signal. Assembly over machinery that existed end to end,
not extension.

## Design rulings (R-IDs carried from the brief; other docs cite them)

### R1 -- the pass/fail signal is a line protocol, not an exit code

Each assertion prints exactly one line on stdout:

```text
ok -- <label>
not ok -- <label>
```

Numberless TAP at line granularity. `count_protocol` classifies `not ok -- ` **before**
`ok -- `, both as line prefixes, so a `not ok` line is never miscounted as a pass. An
entry fails if any `not ok` line appears, the child exits non-zero, or its build fails.
No in-language counters, no ordering contract; the same output reads correctly when a test
file is run by hand.

- **R1.1** A trap/abort exits non-zero and is reported as failure; a build error is a
  failure on its own channel. As shipped this is a *channel split in the driver*, not just
  a convention: `test` takes `report` and `diagnostics` writers, sends per-entry verdicts
  and the summary to `report`, and forwards compiler errors and a failed child's own stderr
  verbatim to `diagnostics`. The CLI passes stdout and stderr respectively.
- **R1.2** A suite aborting itself deliberately reaches for S7a's `exit`, not a new
  primitive. `exit` does not diverge: the calling word still satisfies its declared effect
  and drops its linear values on the path past the call. No `!`/`Never` shape exists.
- **R1.3** `.` on a `Str` prints through `%.*s` and appends **no** newline (`src/backend/qbe.rs`,
  `$strfmt` vs `$fmt`), so each protocol line's trailing `\n` is written explicitly by the
  vocabulary (`"\n" .`).

### R2 -- the vocabulary is `hosted::testing`, two words

`lib/hosted/testing.sth`, as shipped:

```text
import: intrinsics | . | ;
import: core::bool | Bool if | ;
import: core::cmp | Ord eq | ;

export: expect expect-eq ;

: expect ( Bool str -- )
  | label |
  ~[ "ok -- " . label . "\n" . ]
  ~[ "not ok -- " . label . "\n" . ]
  if ;

: expect-eq ['T: Ord] ( 'T 'T str -- )
  | label |
  eq label expect ;
```

- **R2.1** `expect` consumes its `Bool` and label and branches with `core::bool`'s `if`.
  `label` is captured into both `~` arms; only one path runs, so `str` is used exactly once
  per path (the standard branch-join pattern).
- **R2.2** `expect-eq` consumes **both** operands (correct under the linear spine: a test
  restates the values it checks), compares through the library `Ord`, and delegates.
- **R2.3** `expect-eq` is deliberately **not** `inline`: its own body binds `Bound::User`
  (the `'T: Ord` instantiation), independent of its callee `eq` (which is itself `inline`
  again at HEAD after P7.S8 reverted P7.S3s). Failures report the label alone; generic
  value printing wants the `Show`/`Write` pair (S7c/S7d).
- **R2.4** `testing` joins `hosted`'s `module:` list, not `core`'s. Printing is an OS-facing
  capability; the `hosted` tag is a statement of intent, enforced retroactively once S7d
  makes `.` an ordinary word.
- **R2.5** The import list names exactly what the body uses. `Bool` comes from `core::bool`
  (`core::cmp`'s `export:` line does not carry it); `Ord` is named explicitly because
  `expect-eq`'s bound needs the capability in scope, not just `eq`. No `hosted::libc`
  import: `exit` is a suite author's tool (R1.2), not this vocabulary's dependency.

### R3 -- discovery is convention, never manifest grammar

`sooth test [path...]`, in `discover_test_entries(cwd, paths)`:

- **R3.1** No path: probe `cwd.join("sooth.pkg").is_file()` first, then fall back to
  `packages::find_package_root(&cwd.join("_"))` for an ancestor walk (that walk starts at
  `file.parent()` and would otherwise skip the cwd's own manifest, the `cd examples &&
  sooth test` case). Every `*.sth` under `<pkgroot>/tests/` is an entry. No `sooth.pkg` at
  or above the cwd, a missing `tests/`, and a present-but-empty `tests/` are all
  usage-level errors, never a silent green.
- **R3.2** With path(s): each named `.sth` file is an entry; a named directory contributes
  every `*.sth` directly under it, non-recursive. A directory argument is always literal --
  `sooth test <pkg-dir>` does **not** mean "that package's tests", which is why the dogfood
  suite is invoked as `sooth test examples/tests`.
- **R3.3** Each entry defines `main ( -- )` and builds as its own program against its own
  package resolution, so no new import-resolution rule. A `tests:` manifest section is
  rejected: it bakes a zero-consumer convention into the grammar P8.S3 is baselining and
  duplicates R3.1's fixed `<pkgroot>/tests/` rule. This mirrors Rust's `tests/*.rs`
  integration-test shape, which matches whole-program (`main`-per-file) entries exactly. A
  colocated in-module test story is a distinct, larger feature (new grammar plus
  conditional exclusion) and not a substitute. An optional override may come later, once a
  second consumer's layout needs one.
- **R3.4** Entries are sorted by path for determinism (`collect_sth_files` sorts; the
  explicit-path arm sorts the merged set). R1 asserts no ordering *within* a file.

### R4 -- compiled test binaries never touch the source tree

- **R4.1** `build`/`run` land their binary beside the source (`binary_path` is
  `source.with_extension("")`). `sooth test` instead builds each entry into a
  `tempfile_dir()` scratch directory, runs it by that absolute path (`Command::new` needs a
  separator-bearing or absolute path; a bare relative name resolves against `PATH`),
  captures stdout, and lets `TempDir`'s `Drop` remove the binary.
- **R4.2** One compile + run per file, sequential, captured stdout.
- **R4.3** The output-path choice was extracted: `build_into(path, out, manifest)` and
  `link_shimmed_binary(ssa, out)` hold the qbe/cc plumbing with an explicit output path;
  `build_with_manifest` keeps passing `binary_path(source)`, so `build`/`run` are unchanged.

### R5 -- the exit criterion is a dogfood suite for `core` itself

- **R5.1** `examples/tests/` holds five suites green under `sooth test examples/tests`
  (31 assertions across `bool`, `cmp`, `option`, `result`, `combinators`). `impl: Ord`
  exists only for the numeric scalars, so `bool` leans entirely on `expect` (each arm
  chosen so the dispatch under test only reads back `True` on the right branch), `option`
  and `result` reduce a case to a scalar through a hand-written eliminator
  (`unwrap-or`; `safe-mod`/`to-int`) before `expect-eq`, and `cmp`/`combinators` give
  `expect-eq` real work. `cmp.sth` probes every comparison at all three `Ordering`
  positions, since one probe per operator lands where that operator agrees with a
  neighbour (an equal-operand probe cannot separate `lte` from `eq`).
- **R5.2** The suite lives in the `examples` package (layer `hosted`), not inside `core`: a
  test file inside package `core` would need to name its own package's modules.
- **R5.3** `examples/sooth.pkg` gained `depends: hosted path "../lib/hosted" ;` -- this
  slice is the first consumer S7a deferred that entry to.

## Delivered shape

`lib/hosted/testing.sth` with the two words above, plus `module: testing ;` in
`lib/hosted/sooth.pkg`.

**`driver.rs` split into `src/driver.rs` + `src/driver/toolchain.rs`** (the Phase-2
growth-signal re-run: import divergence and a file doing pipeline orchestration *and*
native-toolchain work). `driver.rs` keeps the stage wiring and re-exports
`build`, `build_with_manifest`, `compile_so`, `discover_test_entries`, `run`,
`run_with_manifest`, `test`, `Library`. `toolchain.rs` holds the qbe/cc plumbing,
`dlopen`/`TempDir` machinery, and:

- `build_into` / `link_shimmed_binary` (R4.3, private).
- `discover_test_entries(cwd, paths) -> Result<Vec<PathBuf>, String>` (pub) and
  `count_protocol(&str) -> (usize, usize)` (private), both process-free and unit-tested in
  place.
- `test(cwd, paths, report, diagnostics) -> Result<i32, String>`: discovery, temp-dir build
  and run per entry, protocol counting, a per-entry `ok   <path>` / `FAIL <path> -- <why>`
  line and a `N entries, F failed (O ok, N not ok assertions)` summary on `report`, exit
  code 0 iff every entry passed. `cwd` and both writers are injected, so the integration
  tests drive the no-path case without mutating the test process's working directory.

`src/main.rs` gains a `test` subcommand, `parse_test_paths` (a `[path...]` list, no
`--manifest`; any `--flag` is a usage error), a `usage()` line, and wiring that reads
`current_dir()` once and exits with `driver::test`'s return.

`.gitignore` gained the three-line re-exclusion (`!/examples/tests/`, `/examples/tests/*`,
`!/examples/tests/*.sth`): a single un-exclusion would readmit any binary a hand-run
`sooth build examples/tests/x.sth` leaves behind.

## Tests

`tests/phase7_slice7b.rs` (scratch `Tree` package trees, per `phase7_slice7a.rs`):

- Phase 1 golden: `hosted_testing_expect_and_expect_eq_print_the_r1_protocol` -- one passing
  and one failing `expect` and `expect-eq`, four exact protocol lines with newlines.
- `driver_test_pass_reports_green`;
  `driver_test_fail_by_protocol_is_reported_failed_despite_zero_exit` (the discriminator:
  the child exits 0, and the fixture routes through the real vocabulary rather than a
  hand-written `"not ok -- x\n" .`);
  `driver_test_fail_by_crash_and_clean_exit_are_discriminated` (a `0 >i32 exit` entry must
  be reported *passed*, ruling out "any exit call" as the failure key);
  `driver_test_build_failure_is_reported_failed`;
  `driver_test_forwards_a_trapping_childs_stderr` (R1.1's channel split);
  `driver_test_summary_line_reports_totals`.
- Discovery: `..._no_path_finds_pkgroot_tests_dir` (asserts the entry set),
  `..._explicit_path_resolves_file_and_dir`, `..._missing_tests_dir_is_error`,
  `..._empty_tests_dir_is_error`.
- CLI: `cli_test_with_no_path_discovers_pkgroot_tests_dir` (the only exercise of the CLI's
  own `current_dir()` read), `cli_test_with_explicit_path_runs_that_entry` (cwd
  deliberately holds no `sooth.pkg`, so a dropped `[path...]` fails instead of passing),
  `cli_test_with_failing_suite_exits_nonzero`.
- Dogfood: `dogfood_suite_examples_tests_is_green` (spawns the real binary from the checkout
  root and pins `5 entries, 0 failed (31 ok, 0 not ok assertions)`) and
  `dogfood_suite_broken_twin_is_reported_failed` -- the twin is the *committed* `cmp.sth`
  read and copied into a scratch tree with one literal flipped, so the regression is
  dogfood-shaped and never lands under `examples/tests/` (R3.1/R3.2 sweep every `.sth`
  present, with no skip mechanism).

Unit tests beside the code: `count_protocol_counts_ok_and_not_ok_separately`,
`count_protocol_does_not_miscount_not_ok_as_ok`, `count_protocol_ignores_non_protocol_lines`,
`discover_test_entries_{no_path_reads_pkgroot_tests_dir, explicit_file_and_dir,
no_ancestor_pkg_is_error, missing_tests_dir_is_error, empty_tests_dir_is_error}` in
`toolchain.rs`; `test_subcommand_{collects_paths, no_paths_is_ok, rejects_flag}` in `main.rs`.

## Mutation recipe (each maps to a named test)

1. `count_protocol` counting `ok` before `not ok`, or by substring:
   `count_protocol_does_not_miscount_not_ok_as_ok` plus the fail-by-protocol test.
2. `test` keying failure on the child exit code alone: only the fail-by-protocol test dies
   (child exited 0) -- it, not the crash test, is the discriminator.
3. Dropping `testing` from `lib/hosted/sooth.pkg`'s `module:` list: `PrivateModule` kills the
   Phase-1 golden and the dogfood run.
4. Swapping `expect`'s two `~` arms, or dropping a `"\n" .`: both the pass and
   fail-by-protocol tests die. (Dead alternatives: mis-ordering `expect-eq`'s operands --
   `eq` is symmetric; dropping the label capture -- a compile error killing every entry; a
   dogfood witness case expecting `not ok` -- contradicts R5.1.)

## Out of scope

The `Show`/`Write` pair (S7c) and retiring the intrinsic `.` (S7d); manifest-level test
declaration (R3.3); failure-only output, filtering, watch mode, golden-stdout comparison,
property testing, fixtures, parallel runs; any `hosted::libc` binding beyond S7a's `exit`.

## Bookkeeping

P7.S7b `[ done ]` in the roadmap; the S7c brief records this slice as the consumer whose
label-only assertions motivate `Show`. Growth-signal re-runs: `driver.rs` at Phase 2 exit
(fired -- the `toolchain.rs` split), `main.rs` at Phase 4 exit (did not fire; it gained one
parser and one match arm). Neither corpus list
(`phase4_slice10c_corpus_stdout.rs`, `qbe_baseline.rs`) was touched: the dogfood suites are
not corpus examples.
