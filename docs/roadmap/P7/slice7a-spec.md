# P7.S7a -- `lib/core` / `lib/hosted` package split, and `lib/hosted/libc.sth` (spec)

Status: **shipped.** `lib/core/` and
`lib/hosted/` both exist as sibling packages and the tree is green on them. Input:
[slice7a-libc-brief](./slice7a-libc-brief.md). Roadmap:
[P7-language-prereqs](../P7-language-prereqs.md), P7.S7a.

## Problem

`lib/` was one package (`package: core ; layer: core ;`) holding every stdlib module. S7b
adds `testing`, S7d adds a `Write` impl over stdio; both are `hosted`-layer content with no
`hosted` package to live in. Splitting the directory before that content exists is cheaper
than retrofitting the layer boundary underneath it.

No compiler change. `PackageLayer::Hosted` exists (`src/manifest.rs`), the layer check
already fires from a `depends:` entry (`src/packages.rs`), and `extern:` is live grammar
used by `examples/strings.sth` and `examples/resources.sth`. This slice is a directory move,
a manifest, one `extern:` line, and a path-rename sweep.

### What was probed (load-bearing, confirmed by the shipped phase 2)

Built and run against `main` at `e5bfd6e` in a throwaway sibling-package tree
(`$T/core` + `$T/hosted` + `$T/app`, exactly R1/R2's shape):

- **The whole R2/R3 mechanism already works.** A `hosted/libc.sth` holding only
  `extern: exit ( i32 -- ) "exit" ; export: exit ;`, imported from a third package as
  `import: hosted::libc l | exit | ;`, builds, and `5 >i32 exit` gives `EXIT=5`. Nothing in
  resolution, mangling or linking needs work.
- **`libc.sth` needs no `import: intrinsics * ;`.** An `extern:`-and-`export:`-only module
  names no builtin. Do not add one.
- **`>i32` is the cast.** `1` is `i64`, `exit` takes `i32`, so the call site is
  `1 >i32 exit` (`examples/rgb_bits.sth` is the precedent). A bare `1 exit` is a type error,
  which R5.4 pins.
- **`lib/` losing its own `sooth.pkg` is fine.** `find_package_root` is nearest-ancestor,
  so each sibling finds its own manifest. An intermediate `lib/sooth.pkg` must **not** be
  left behind.
- **The layer check fires on the inverted edge.** `depends: hosted path "../hosted"` in a
  `layer: core` manifest is rejected with `layer violation in .../core/sooth.pkg`.

## Requirements

### R1 -- `lib/core` is a rename (shipped)

- **R1.1** `bool.sth`, `cmp.sth`, `prelude.sth`, `combinators.sth`, `option.sth`,
  `result.sth` and `sooth.pkg` moved into `lib/core/` with no content change beyond two
  doc-comment path strings in `cmp.sth` itself (it names its own pre-move path in prose,
  which would otherwise ship stale) -- no other `.sth` bodies changed. `lib/core/sooth.pkg`
  keeps `package: core ; layer: core ;` and its
  `module:` list verbatim; its header comment now opens "`core`: the `no_std` bottom layer
  of the standard library. `lib/` is not itself a package -- `lib/hosted/` is a sibling
  package one layer up."
- **R1.2** No `lib/sooth.pkg` remains.
- **R1.3** Every functional `lib/` reference rewrote to `lib/core/`, with no compatibility
  alias: the two committed `depends:` manifests (`examples/`, `tests/fixtures/`),
  `fixture_package`'s generated manifest and the two other generated-manifest sites, the
  two `include_str!`s in `src/test_support.rs`, the `qbe.rs` unit fixture, the five
  `combinators.sth` drift guards, and the ~15 quoted-path `import:` helpers and call sites
  across the integration suite. The gate caught every miss loudly (build error,
  `include_str!` compile error, or a drift guard panicking on a missing file), as R1.3
  predicted.
- **R1.4** Unrelated working-tree edits to `examples/option.sth` / `lib/option.sth` were
  kept out of the commits.

**Correction, cost two follow-up commits.** The inventory ("29 tracked sites across 20
files", plus R6's claim that four prose sites "are the whole set") counted only *paths* and
treated all prose as out of sweep. That split was wrong in one direction: prose inside
**shipped artifacts** is user-facing and had to move too --
`lib/core/cmp.sth`'s own header comments, `tree-sitter-sooth/queries/highlights.scm`, seven
`examples/*.sth` header comments, and a `tests/common/mod.rs` assertion message
(`ae65acce`, `c4880c3b`). Rust-internal `///` and `//` doc comments (~50 sites) remain
deliberately stale and are still out of scope. The dividing line is *ships to a user*, not
*is a path*.

### R2 -- `lib/hosted` is a new sibling package (shipped)

- **R2.1** `lib/hosted/sooth.pkg`:

  ```text
  package: hosted ;
  layer: hosted ;
  depends: core path "../core" ;
  module: libc ;
  ```

  Shipped above two `\` header-comment lines ("`hosted`: bindings to the host platform's C
  runtime, for programs that need it. / Depends on `core`; a `layer: core` package may not
  depend on this one."), matching `lib/core/sooth.pkg`'s convention. Same exception as
  R1.1: the block above is the directive content, not the whole file.

- **R2.2** No committed manifest gains a `depends: hosted` entry in this slice. `examples/`
  calls no `hosted` word (R4), no fixture imports one, and `fixture_package` would hand
  every fixture tree in the suite a dependency exactly one test uses. Pre-staging for
  S7b/S7d is elevation above the lowest common ancestor; the entry lands with its first
  consumer.
- **R2.3** `tests/phase7_slice7a.rs` writes its own ad-hoc manifest into a temp tree:
  `package: s7a ; layer: hosted ; depends: core path "<checkout>/lib/core" ;
  depends: hosted path "<checkout>/lib/hosted" ;`. **Both** entries: a `depends:` table is
  not transitive (the lookup resolves the import's first segment against the importer's own
  table), so naming `hosted` alone would not reach `core`.
- **R2.4** `module: libc ;` is required for importability; an unlisted module is
  `UnresolvedKind::PrivateModule`.

### R3 -- `lib/hosted/libc.sth` holds `exit`, and nothing else (shipped)

- **R3.1** The directives, no `import:` line:

  ```text
  extern: exit ( i32 -- ) "exit" ;
  export: exit ;
  ```

  Shipped under one `\` header-comment line ("`libc`: raw extern bindings to host C
  functions."), per the R2.1 exception. "No `import:` line" is the load-bearing part and
  holds.

- **R3.2** `examples/strings.sth`'s `strlen`/`puts` and `examples/resources.sth`'s
  `open`/`read`/`close-fd` stay put. Nothing shares them, so elevating them has no consumer.
- **R3.3 (`exit` does not diverge, and this slice does not make it)** Measured:
  `: main ( -- ) 5 >i32 exit 9 . ;` builds and prints nothing before `EXIT=5`; code after
  `exit` is checked as reachable and dead at runtime, and `: main ( -- ) 1 5 >i32 exit ;` is
  still `body leaves 1 values, but ( … ) declares 0 outputs`. A caller must satisfy its
  declared effect and drop its linear values on a path that never executes. A real wart, and
  **out of scope**: a `!`/`Never` output shape is a type-system change. Stated because
  S7b's "abort a suite early" hits it.
- **R3.4** No second libc binding until it has a second consumer.

### R4 -- no committed nonzero-exit example (ruling stands, honoured)

Both `examples/` corpus sweeps would reject one: `phase4_slice10c_corpus_stdout.rs` asserts
`out.status.success()` per program, `qbe_baseline.rs` pins emitted QBE per program, and both
carry an explicit list whose doc comment claims *every* standalone example.
`examples/exit_code.sth` would fail the first or force a third exclusion, for a witness that
is not example-shaped anyway. Ruling: the R5.1 witness is a harness-written package tree
with its own manifest (R2.3). No new file under `examples/`, neither `CORPUS` list changed.

### R5 -- tests

- **R5.2a (shipped, phase 1)** `src/packages.rs` gained one unit test: `lib/` holds no
  `sooth.pkg`, and the committed `lib/core/sooth.pkg` parses. The existing corpus (34
  examples plus every fixture tree) is the witness that every `depends: core` entry still
  resolves.
- **R5.7 (shipped, phase 1)** Full suite green with no test *assertion* changed, only path
  strings and two drift-guard messages (`tests/common/mod.rs:319`,
  `src/ir/func_builder/calls.rs:1930`).
- **R5.1 (shipped, phase 2)** `tests/phase7_slice7a.rs`: a fixture tree whose entry does
  `import: hosted::libc l | exit | ;` and calls `7 >i32 exit`, asserting
  `status.code() == Some(7)` **and** the stdout printed before the call (so a program that
  never ran cannot pass by exiting 7 for another reason). Second case: `0 >i32 exit` gives
  `Some(0)` with the same stdout -- without it, any nonzero-exit failure mode passes case one.
- **R5.2b (shipped, phase 2)** `lib/hosted/sooth.pkg` parses.
- **R5.3 (shipped, phase 2, folded into R5.8(2))** A fixture tree at `layer: core` declaring
  `depends: hosted path ...` is rejected with the `layer violation` message naming both
  layers. Deliberately one test, not two: R5.8(2)'s tree *is* a harness-written `layer: core`
  fixture, and pointing it at the real `lib/hosted` strictly subsumes pointing it at a
  fixture dependency. `packages::tests::check_package_graph_layer_violation_is_error` already
  pins the full message (both layers plus the trailing rule line) against a pure sandbox, so
  a separate R5.3 body would only duplicate it.
- **R5.4 (shipped, phase 2)** `1 exit` without `>i32` is a located type error; measured verbatim
  against `main`, assert on these two lines (not on the known doubled `error:` prefix):

  ```text
  type mismatch in `main` (line 2)
    `exit` expected `i32`, found `i64`
  ```

  This is the only guard that the exported signature is `( i32 -- )` and not something
  wider; a build-and-run golden cannot tell `i32` from `i64` here.
  The `(line 2)` locator is weak on its own -- line 2 is the last line of the 2-line fixture,
  so a span bug collapsing every location onto the final line would still pass. Kept as the
  measured text rather than strengthened: the signature half
  (`` `exit` expected `i32`, found `i64` ``) is what this test exists for.

- **R5.5 (shipped, phase 2, two independent cases)** Both messages quoted from `src/packages.rs`
  (`missing_depends_error`, `private_module_error`):
  - *Missing `depends:`*, against the **real** `lib/hosted`: a temp package naming only
    `core` imports `hosted::libc`, and the error contains
    `` package `s7a` has no `depends:` entry for `hosted` `` plus its
    ``add `depends: hosted path "<path>" ;` to <manifest>`` line, asserted with the real
    `to <manifest>` tail: the remedy is useless if it names the wrong manifest, and the
    `<path>` placeholder is literal while the manifest path is not.
  - *Private module*, against a **harness-written fixture package, never `lib/hosted/`**: a
    temp dependency whose `module:` list omits a module file present on disk. The real
    `lib/hosted` cannot witness this -- R2.1 lists `libc`, R3.4 forbids a second module, and
    resolution requires the file to exist before consulting the `module:` list
    (`existing_module_file` fails first with `module_not_found_error`, never
    `PrivateModule`).
- **R5.6 (shipped, phase 2)** `src/packages.rs` gains `find_package_root` on
  `lib/hosted/libc.sth` returning `lib/hosted`, not `lib`. No manifest-grammar unit test:
  the split adds no grammar.
- **R5.8 (shipped, phase 2)** R5.3 pins the layer *check*, not what the shipped manifest declares;
  confirmed live that flipping `lib/hosted/sooth.pkg` to `layer: core` would leave the gate
  green. Two assertions: (1) the committed file parses to `PackageLayer::Hosted` (assert on
  the parsed layer, not the text); (2) a harness-written `layer: core` package whose
  `depends: hosted` path is the **real** `<checkout>/lib/hosted` is rejected. The second
  half is what a fixture tree cannot stand in for.

### R6 -- live documentation (shipped, set was undercounted)

The four named prose sites (`docs/design/memory-model.md`, `docs/design/control-flow.md`,
`docs/book/quotations-and-loops.md`, `docs/book/words.md`) took the path rename. They were
not "the whole set" -- see the R1.3 correction. Historical specs and briefs under
`docs/roadmap/P4/`--`P7/` are not rewritten.

### R7 -- mutation recipe (shipped, phase 2; all three caught)

Three, each classified on a named `test result: FAILED`, against a committed tree with the
mutated binary confirmed rebuilt. Measured in an isolated copy, not the worktree.

1. Widen `exit` to `( i64 -- )` -- **three** named failures, the same set as mutation 2:
   `exit_without_cast_is_located_type_error` (its `1 exit` now typechecks, so the build
   succeeds and the test dies on "build should have failed") plus both R5.1 cases, which
   spell `7 >i32 exit` and so are rejected with `` `exit` expected `i64`, found `i32` ``.
2. Drop `libc` from `lib/hosted/sooth.pkg`'s `module:` list -- the same three names, all
   dying on the `PrivateModule` error: both R5.1 cases and R5.4 import `hosted::libc` from
   the real package, so R5.4 never reaches the `exit` type check. R5.5's private-module case
   staying green is *not* a discriminator (it reads a fixture package, never the real
   manifest). This mutation is what proves `exit_fixture`'s `import: intrinsics * ;` does not
   disable the gate: the `hosted::libc` import is load-bearing.
3. Flip `lib/hosted/sooth.pkg` to `layer: core` -- both halves of R5.8 fail
   (`lib_hosted_manifest_parses_as_hosted_layer`, `layer_core_depends_on_real_hosted_is_error`).
   It survived before phase 2: nothing observed the shipped manifest's layer.

Mutations 1 and 2 share a named-failure *set*; only the failure *mode* tells them apart.
So the set is a guard, not a discriminator -- do not use it to identify which of the two
regressed.

Reverting the rename at a single site is deliberately not a mutation: it is a compile error
or a panicking drift guard (R1.3).

## Out of scope

Any libc binding beyond `exit` (R3.4); centralizing the per-example `extern:` bindings
(R3.2); a divergence/`Never` type for `exit` (R3.3); `testing` (S7b), `Show`/`Write` (S7c),
the hosted `.` (S7d); a `fixed` or `alloc` package; Rust-internal doc comments and the ~160
`lib/<module>.sth` mentions across delivered P4--P7 documents.

## Phasing

**Phase 1 -- R1 + R6 (M). Shipped.** The rename, the path sweep and the four live doc
strings as `0063dfeb`, plus two follow-ups for the shipped-artifact prose the inventory
missed (`ae65acce`, `c4880c3b`). Pure move, independently revertable; full gate green with
no assertion edits, plus R5.2a.

**Phase 2 -- R2 + R3 + R4 (S). Shipped.** The `hosted` manifest and `libc.sth` as `9012ef1`,
plus review follow-ups (`3ace465` and this one). Exit met: R5.1, R5.2b, R5.3 (folded into
R5.8(2)), R5.4, R5.5, R5.6, R5.8, and all three R7 mutations caught.

**Phase 3 -- bookkeeping (S). Shipped.** P7.S7a marked `[ done ]` in
[P7-language-prereqs](../P7-language-prereqs.md); R3.3 noted in the S7b brief so its `exit`
call site is written knowing `exit` does not diverge. No growth-signal re-run due: no Rust
module gained code (`src/packages.rs` gained only two unit tests).

## Exit criteria

All met. By phase 1:

- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green, with no test
  assertion changed (only path strings and two drift-guard messages).
- `lib/` holds no manifest, `lib/core/` is a package, and every `depends: core` entry in the
  tree resolves under the new path.
- No new file under `examples/`, and neither corpus list changed.

By phase 2:

- `lib/hosted/` exists as a sibling package exporting `exit ( i32 -- )`; a program depending
  on `hosted` calls it and the observed exit code is the argument, for a nonzero and a zero
  value.
- A `layer: core` package depending on `hosted` is rejected, for a fixture tree and for the
  real `lib/hosted` path alike, and `lib/hosted/sooth.pkg` parses as `layer: hosted`.
- `1 exit` without `>i32` is a located type error.
- All three R7 mutations fail a named test.
