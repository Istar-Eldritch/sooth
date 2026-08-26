# Phase 7 Slice 5: a testing vocabulary in `core`, and a `sooth test` command (brief)

Unit-testing Sooth code in Sooth itself. Two halves: an assertion vocabulary the
language already has every prerequisite for (traits, enums, generics, string printing,
modules), and a driver subcommand that discovers, builds, and runs test programs the
way `sooth build`/`run` already do, then interprets their output as a pass/fail
signal. Neither half needs a type-system or backend change.

Placed in P7 rather than P9 because the vocabulary is a first real *stdlib* consumer
of the P7 machinery (`Ord` dispatch, enum elimination, `Bound::User` generics,
string printing through the intrinsic `.` row) and because the CLI is ordinary
driver work over machinery P8.S1a/S1b landed (manifest resolution, package
discovery) — both halves exist today, this slice is assembly, not extension.

## Design rulings

### R1 — The pass/fail signal is a line protocol, not an exit code

Each assertion prints exactly one line on the assertion channel:

    ok -- <label>
    not ok -- <label>

Numberless TAP at line granularity: the runner counts `ok`/`not ok` prefixes in the
process's stdout, any `not ok` line or any non-zero exit marks the run failed. No
in-language counters (nothing threads test state across assertions, nothing needs
static storage), no ordering contract, and the same protocol reads correctly when a
test file is run by hand — a property that matters for a language whose feedback
loop is run-the-program.

A test program that traps or aborts already exits non-zero, which the runner
reports as failure; build errors are failures on their own channel.

### R2 — No `exit` intrinsic in this slice

A test program cannot yet exit non-zero by its own choice; it does not need to. The
protocol line carries the failure signal (R1), the runner interprets it, and
aborting a suite early is an unforced requirement. If a future consumer wants
process-level failure (a golden harness, CI without the runner), a one-word
`exit ( i32 -- )` intrinsic is a separate, tiny slice. Deferring keeps this slice
zero-language-change.

### R3 — The vocabulary is `core::testing`, two words

    expect    ( Bool str -- )
    expect-eq ( 'T: Ord 'T str -- )

`expect-eq` consumes both operands (correct under the linear spine: a test restates
the values it checks rather than holding them), compares through the library `Ord`,
prints per R1. It is deliberately non-`inline` for the same reason S3s made the
comparisons non-`inline`: its body binds `Bound::User`. Reported on failure is the
label alone — generic value printing wants a `Show` trait, which is P7.S4
(generic `impl:` targets, briefed) territory and correctly ordered after this
slice's own consumer exists. Label-only output matches what the Rust-side goldens
have proven is workable.

`lib/testing.sth` joins `core`'s `module:` list; layer `core` is satisfied (the
protocol prints through the intrinsic `.` row, same as `core::bool`'s overload).

### R4 — Discovery is convention, never manifest grammar

`sooth test [path...]`:

- With no path: resolve the package containing the cwd (nearest ancestor
  `sooth.pkg`, the discovery P8.S1a already performs), and take every `*.sth`
  under `<pkgroot>/tests/` as a test entry.
- With path(s): each named `.sth` file is a test entry; with a directory,
  every `*.sth` under it.

Each entry must define a `main ( -- )` and builds as its own program against its
own package resolution — a test file in a consumer package imports `core::*`
through `depends:` like any other program, so no new import-resolution rule is
introduced anywhere. Explicitly *not* a manifest section: a `tests:` line in
`sooth.pkg` would bake the convention into the grammar P8.S3 is about to baseline,
for a convention that has had zero consumers until now.

### R5 — Compiled test binaries never touch the source tree

`build`/`run`'s pid-suffixed binary lands beside the source (the litter in
`examples/` is the visible evidence). `sooth test` builds each entry into a temp
directory and deletes it after the run: test binaries are byproducts, not files a
reader of the tree should ever see. One compile+run per file, sequential, captured
stdout; parallelism is unforced at this scale.

### R6 — The exit criterion is a dogfood suite for `core` itself

`examples/tests/` — placed in the `examples` package (layer `hosted`,
`depends: core` already declared) rather than inside `core`, because a test file
inside package `core` would need to refer to its own package's modules by name and
that is a resolution question this slice does not want to force. Suites for `bool`,
`cmp`, `option`, `result`, `combinators`, authored in `core::testing`, green under
`sooth test examples`. This is the half that turns the runner from machinery into a
thing you use; per the roadmap's own rule, a slice that produces no runnable
program isn't done.

## Out of scope

- A `Show`/value-printing trait family: P7.S4's consumer, ordered after this.
- An `exit` intrinsic: R2.
- Manifest-level test declaration: R4.
- Failure-only output, filtering, watch mode, golden-stdout comparison (the Rust
  corpus harness already owns source-in/stdout-out), property testing, fixtures.
  All unforced.

## Exit

1. `core::testing` exports `expect` and `expect-eq` as specified in R3; a program
   importing it compiles and prints the R1 protocol.
2. `sooth test` behaves per R4/R5: discovery by convention, temp-dir builds, counts
   `ok`/`not ok`, non-zero exit on any `not ok`, non-zero process exit, or failed
   build; a summary line reports totals.
3. The R6 dogfood suite runs green via `sooth test examples`, and its deliberately-
   broken twin (not committed, or committed as an ignored fixture asserted by the
   Rust integration test) shows the runner catching it.
4. Rust integration coverage for the driver halves (pass, fail-by-protocol,
   fail-by-crash, discovery, explicit path, empty tests dir) beside the existing
   `tests/` harness; P7 doc and ROADMAP updated.
