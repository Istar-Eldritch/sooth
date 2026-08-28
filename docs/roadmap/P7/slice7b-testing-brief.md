# Phase 7 Slice 7b: a testing vocabulary in `hosted`, and a `sooth test` command (brief)

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

Depends on **S7a** for the `lib/core`/`lib/hosted` split (this vocabulary lives in
`hosted`, not `core` — see R3) and, if a suite wants to abort early rather than
report through the protocol, for `hosted::libc`'s `exit`.

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
reports as failure; build errors are failures on their own channel. A suite that
wants to abort itself deliberately (a fatal precondition, not an assertion) reaches
for S7a's `exit`, not a new primitive here. `exit` does not diverge (S7a R3.3): the
checker still requires the calling word's own body to satisfy its declared effect at
body end, including dropping any linear values already pushed before the call, even
though that path never runs. In particular, values pushed before the call must still
be disposed after it, and declared outputs must still be produced after it:
`: give-up ( -- i64 ) 1 >i32 exit 0 ;` builds, `: give-up ( -- i64 ) 1 >i32 exit ;`
fails with `body leaves 0 values, but ( … ) declares 1 outputs`. A suite-abort helper
is therefore ordinary code with an ordinary effect, not a hole in the linear spine; no
`!`/`Never` output shape exists to let unreachable code skip the checker.

### R2 — The vocabulary is `hosted::testing`, two words

    expect    ( Bool str -- )
    expect-eq ( 'T: Ord 'T str -- )

`expect-eq` consumes both operands (correct under the linear spine: a test restates
the values it checks rather than holding them), compares through the library `Ord`,
prints per R1. It is deliberately non-`inline` for the same reason S3s made the
comparisons non-`inline`: its body binds `Bound::User`. Reported on failure is the
label alone — generic value printing wants the `Show`/`Write` trait pair, which is
S7c/S7d territory, correctly ordered after this slice's own consumer exists. Label-
only output matches what the Rust-side goldens have proven is workable.

`lib/hosted/testing.sth` joins `hosted`'s `module:` list, not `core`'s. Printing —
today the compiler-injected `.` row, after S7d an ordinary `hosted` word over
`Show`/`Write` — is inescapably an OS-facing capability; the layer this vocabulary
declares should say so regardless of which mechanism happens to back it on a given
day. (Before S7d lands, `.` remains callable from any layer as a compiler
intrinsic, same as `core::bool`'s own overload; the `hosted` tag here is a
statement of intent, enforced retroactively once S7d makes `.` an ordinary word.)

### R3 — Discovery is convention, never manifest grammar

`sooth test [path...]`:

- With no path: resolve the package containing the cwd (nearest ancestor
  `sooth.pkg`, the discovery P8.S1a already performs), and take every `*.sth`
  under `<pkgroot>/tests/` as a test entry.
- With path(s): each named `.sth` file is a test entry; with a directory,
  every `*.sth` under it.

Each entry must define a `main ( -- )` and builds as its own program against its
own package resolution — a test file in a consumer package imports `hosted::*`
through `depends:` like any other program, so no new import-resolution rule is
introduced anywhere. Explicitly *not* a manifest section: a `tests:` line in
`sooth.pkg` would bake the convention into the grammar P8.S3 is about to baseline,
for a convention that has had zero consumers until now.

### R4 — Compiled test binaries never touch the source tree

`build`/`run`'s pid-suffixed binary lands beside the source (the litter in
`examples/` is the visible evidence). `sooth test` builds each entry into a temp
directory and deletes it after the run: test binaries are byproducts, not files a
reader of the tree should ever see. One compile+run per file, sequential, captured
stdout; parallelism is unforced at this scale.

### R5 — The exit criterion is a dogfood suite for `core` itself

`examples/tests/` — placed in the `examples` package (layer `hosted`,
`depends: core hosted` already declared) rather than inside `core`, because a test
file inside package `core` would need to refer to its own package's modules by
name and that is a resolution question this slice does not want to force. Suites
for `bool`, `cmp`, `option`, `result`, `combinators`, authored against
`hosted::testing`, green under `sooth test examples`. This is the half that turns
the runner from machinery into a thing you use; per the roadmap's own rule, a
slice that produces no runnable program isn't done.

## Out of scope

- The `Show`/`Write` trait pair: S7c.
- Retiring the compiler-intrinsic `.`: S7d.
- Manifest-level test declaration: R3.
- Failure-only output, filtering, watch mode, golden-stdout comparison (the Rust
  corpus harness already owns source-in/stdout-out), property testing, fixtures.
  All unforced.

## Exit

1. `hosted::testing` exports `expect` and `expect-eq` as specified in R2; a program
   importing it compiles and prints the R1 protocol.
2. `sooth test` behaves per R3/R4: discovery by convention, temp-dir builds, counts
   `ok`/`not ok`, non-zero exit on any `not ok`, non-zero process exit, or failed
   build; a summary line reports totals.
3. The R5 dogfood suite runs green via `sooth test examples`, and its deliberately-
   broken twin (not committed, or committed as an ignored fixture asserted by the
   Rust integration test) shows the runner catching it.
4. Rust integration coverage for the driver halves (pass, fail-by-protocol,
   fail-by-crash, discovery, explicit path, empty tests dir) beside the existing
   `tests/` harness; P7 doc and ROADMAP updated.
