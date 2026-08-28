# P7.S7a -- `lib/core` / `lib/hosted` package split, and `lib/hosted/libc.sth` (spec)

Status: **shipped** (phases 1--3). `lib/core/` and `lib/hosted/` exist as sibling packages;
tree green. Input: [slice7a-libc-brief](./slice7a-libc-brief.md). Roadmap:
[P7-language-prereqs](../P7-language-prereqs.md), P7.S7a.

## Problem

`lib/` was one package (`package: core ; layer: core ;`) holding every stdlib module. S7b
adds `testing`, S7d adds a `Write` impl over stdio; both are `hosted`-layer content with no
`hosted` package to live in. Splitting the directory before that content exists is cheaper
than retrofitting the layer boundary underneath it.

No compiler change: `PackageLayer::Hosted` exists (`src/manifest.rs`), the layer check
already fires from a `depends:` entry (`src/packages.rs`), and `extern:` is live grammar
(`examples/strings.sth`, `examples/resources.sth`). A directory move, a manifest, one
`extern:` line, and a path-rename sweep.

## Delivered shape

`lib/core/` holds `bool.sth`, `cmp.sth`, `prelude.sth`, `combinators.sth`, `option.sth`,
`result.sth` and the unchanged `package: core ; layer: core ;` manifest. No `lib/sooth.pkg`
remains: `find_package_root` is nearest-ancestor, so each sibling finds its own manifest,
and an intermediate manifest would shadow both.

`lib/hosted/sooth.pkg`:

```text
package: hosted ;
layer: hosted ;
depends: core path "../core" ;
module: libc ;
```

`lib/hosted/libc.sth`:

```text
extern: exit ( i32 -- ) "exit" ;
export: exit ;
```

Both ship under `\` header comments, per `lib/core/sooth.pkg`'s convention. **No
`import: intrinsics * ;` in `libc.sth`**: an `extern:`-and-`export:`-only module names no
builtin. `module: libc ;` is what makes it importable; an unlisted module is
`UnresolvedKind::PrivateModule`.

Call sites cast: `1` is `i64` and `exit` takes `i32`, so it is `7 >i32 exit`
(`examples/rgb_bits.sth` is the precedent).

## Rulings that constrain later slices

- **No committed `depends: hosted` entry.** `examples/` calls no `hosted` word, no fixture
  imports one, and `fixture_package` would hand every fixture tree a dependency exactly one
  test uses. The entry lands with its first consumer (S7b/S7d).
- **No second libc binding, and no centralizing** of `examples/strings.sth`'s
  `strlen`/`puts` or `examples/resources.sth`'s `open`/`read`/`close-fd`: nothing shares
  them.
- **No committed nonzero-exit example.** `phase4_slice10c_corpus_stdout.rs` asserts
  `out.status.success()` per program and `qbe_baseline.rs` pins emitted QBE per program;
  both carry a list whose doc comment claims *every* standalone example. The exit witness is
  a harness-written package tree instead. No new file under `examples/`, neither `CORPUS`
  list changed.
- **`exit` does not diverge.** Measured: `: main ( -- ) 5 >i32 exit 9 . ;` builds and prints
  nothing before `EXIT=5`, and `: main ( -- ) 1 5 >i32 exit ;` is still
  `body leaves 1 values, but ( … ) declares 0 outputs`. A caller must satisfy its declared
  effect and drop its linear values on a path that never executes. A `!`/`Never` output
  shape is a type-system change, out of scope. S7b's "abort a suite early" hits this.
- **A `depends:` table is not transitive.** The lookup resolves an import's first segment
  against the *importer's own* table, so a package importing `hosted::libc` needs entries
  for both `hosted` and `core`.

## Path sweep, and the line the inventory got wrong

Every functional `lib/` reference rewrote to `lib/core/` with no compatibility alias: the
two committed `depends:` manifests, three generated-manifest sites, two `include_str!`s in
`src/test_support.rs`, the `qbe.rs` unit fixture, five `combinators.sth` drift guards, and
~15 quoted-path `import:` helpers across the integration suite. The gate caught every miss
loudly (build error, `include_str!` failure, or a drift guard panicking on a missing file).

The original inventory counted only *paths* and treated all prose as out of sweep. Wrong in
one direction, at the cost of two follow-up commits (`ae65acce`, `c4880c3b`): prose inside
**shipped artifacts** is user-facing and had to move too, namely `lib/core/cmp.sth`'s own
header comments, `tree-sitter-sooth/queries/highlights.scm`, seven `examples/*.sth` headers,
and a `tests/common/mod.rs` assertion message. Rust-internal `///` and `//` comments (~50
sites) stay deliberately stale. **The dividing line is *ships to a user*, not *is a path*.**

Live docs took the rename (`docs/design/memory-model.md`, `docs/design/control-flow.md`,
`docs/book/quotations-and-loops.md`, `docs/book/words.md`). Historical specs and briefs under
`docs/roadmap/P4/`--`P7/` are not rewritten.

## Tests

`src/packages.rs` unit tests: `lib_holds_no_manifest_and_lib_core_manifest_parses`,
`lib_hosted_manifest_parses_as_hosted_layer` (assert on the parsed layer, not the text),
`find_package_root_lib_hosted_libc_returns_lib_hosted`. No manifest-grammar test: the split
adds no grammar. The existing corpus (34 examples plus every fixture tree) is the witness
that every `depends: core` entry still resolves.

`tests/phase7_slice7a.rs`, each against a temp tree with the ad-hoc `s7a` manifest:

- `hosted_libc_exit_{nonzero,zero}_code_observed`: `7 >i32 exit` gives `Some(7)`, `0 >i32
  exit` gives `Some(0)`, both asserting the stdout printed *before* the call, so a program
  that never ran cannot pass by exiting 7 for another reason. The zero case is what stops any
  nonzero-exit failure mode from passing the first.
- `exit_without_cast_is_located_type_error`: the only guard that the exported signature is
  `( i32 -- )` and not something wider (a build-and-run golden cannot tell `i32` from `i64`).
  Asserts two measured lines, not the known doubled `error:` prefix:

  ```text
  type mismatch in `main` (line 2)
    `exit` expected `i32`, found `i64`
  ```

  The `(line 2)` locator is weak alone: line 2 is the fixture's last line, so a span bug
  collapsing every location onto it would pass. The signature half is the point.
- `layer_core_depends_on_real_hosted_is_error`: a harness-written `layer: core` package whose
  `depends: hosted` path is the **real** `<checkout>/lib/hosted`. Deliberately one test:
  it strictly subsumes pointing at a fixture dependency, and
  `packages::tests::check_package_graph_layer_violation_is_error` already pins the full
  message (both layers plus the trailing rule line) against a pure sandbox.
- `hosted_import_without_depends_entry_is_error`: against the **real** `lib/hosted`. Asserts
  the remedy's `to <manifest>` tail as well, since a remedy naming the wrong manifest is
  useless, and `<path>` is a literal placeholder while the manifest path is not.
- `private_module_is_error_against_a_fixture_dependency`: **never against `lib/hosted/`**.
  The real package cannot witness this: it lists `libc`, no second module is allowed, and
  resolution requires the file to exist before consulting `module:` (`existing_module_file`
  fails first with `module_not_found_error`, never `PrivateModule`).

Phase 1 landed with no test *assertion* changed, only path strings and two drift-guard
messages (`tests/common/mod.rs:319`, `src/ir/func_builder/calls.rs:1930`).

## Mutation recipe (all three caught)

Classified on a named `test result: FAILED`, in an isolated copy with the mutated binary
confirmed rebuilt.

1. Widen `exit` to `( i64 -- )`: three failures. `exit_without_cast_is_located_type_error`
   dies on "build should have failed"; both exit-code cases spell `7 >i32 exit` and are now
   rejected with `` `exit` expected `i64`, found `i32` ``.
2. Drop `libc` from the `module:` list: the *same three names*, all dying on the
   `PrivateModule` error (R5.4 imports `hosted::libc` too, so it never reaches the type
   check). This is what proves `exit_fixture`'s `import: intrinsics * ;` does not disable the
   gate: the `hosted::libc` import is load-bearing. The private-module test staying green is
   not a discriminator, it reads a fixture package.
3. Flip `lib/hosted/sooth.pkg` to `layer: core`: `lib_hosted_manifest_parses_as_hosted_layer`
   and `layer_core_depends_on_real_hosted_is_error` both fail. It survived before phase 2,
   nothing observed the shipped manifest's layer.

**Mutations 1 and 2 share a named-failure set**; only the failure *mode* separates them. The
set is a guard, not a discriminator. Reverting the rename at one site is deliberately not a
mutation: it is a compile error or a panicking drift guard.

## Out of scope

Any libc binding beyond `exit`; centralizing the per-example `extern:` bindings; a
divergence/`Never` type for `exit`; `testing` (S7b), `Show`/`Write` (S7c), the hosted `.`
(S7d); a `fixed` or `alloc` package; Rust-internal doc comments and the ~160
`lib/<module>.sth` mentions across delivered P4--P7 documents.

## Exit criteria (all met)

- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- `lib/` holds no manifest, `lib/core/` is a package, every `depends: core` entry resolves.
- No new file under `examples/`, neither corpus list changed.
- `lib/hosted/` exports `exit ( i32 -- )`; a dependent program's observed exit code is its
  argument, for a nonzero and a zero value.
- A `layer: core` package depending on `hosted` is rejected against the real `lib/hosted`,
  and `lib/hosted/sooth.pkg` parses as `layer: hosted`.
- `1 exit` without `>i32` is a located type error.
- All three mutations fail a named test.
- P7.S7a marked `[ done ]`; `exit`'s non-divergence noted in the S7b brief. No growth-signal
  re-run due: `src/packages.rs` gained only unit tests.
