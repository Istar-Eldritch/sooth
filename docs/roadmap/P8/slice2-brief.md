# Phase 8 Slice 2: single-mode imports, the intrinsics module, wildcard, and re-export (brief)

**Split from a single, now-stale brief** (`slice1-brief.md`'s history, and
`../P8-packages-modules.md` is authoritative). This slice deletes the compiler-baked
prelude that every program silently depends on today, gates the compiler's own builtins
behind an `intrinsics` import, and adds the bare `*` wildcard import form and re-export
through `export:` — the two things that make single-mode imports and a gated intrinsics
module bearable rather than punishing. It lands after S1/S1b because, with the quoted-path
import form deleted from the language, there is no spelling for a cross-file import before
a manifest and module names exist: nothing can say `core::bool` without a `depends:` table
to resolve `core` against.

## Recon (measured against the built compiler, 2026-08-19, `main` at `c1a0883`)

1. **The prelude is compiled into the binary, not read from disk at build time.**
   `parser::prelude_words` (`src/parser.rs:488-494`) does `include_str!("../lib/core.sth")`
   and lexes/parses it at Rust compile time; `parser::parse` (`:496-500`) unconditionally
   extends every parsed module's word list with it, and `driver::assemble_module` does the
   same for a multi-file closure (`driver.rs:301-304`). There is no per-file opt-out and no
   import path a program can take to *not* get these words — deleting the prelude means
   deleting both call sites, not adding a flag to them.

2. **The mangling exemption for prelude words is one function, sized to shrink.**
   `resolve::mangle` (`src/resolve.rs:26-31`) exempts `main`, `drop`, and
   `is_prelude_word_name` from the `__m{module}` suffix every other decl gets, so a prelude
   word stays reachable by its bare name from every module at once. `is_prelude_word_name`
   (`:52-67`) reads its list off `parser::prelude_words()` rather than a hand-maintained
   set. Deleting the prelude call sites (finding 1) makes this function's body dead; the
   exemption list shrinks to exactly `main`/`drop`, which is where
   `project_repl_bypasses_module_checks` already flagged the REPL's own bypass of this same
   list as a related gap.

3. **`lib/core.sth`'s Sooth half splits into modules of one `core` package; its compiler
   half does not.** `branch`, `tag`, and the six `u`-prefixed comparisons are
   `BUILTIN_WORDS` (`src/check/declarations.rs:85-110`) entries, dispatched ahead of any
   environment lookup — not Sooth code, so there is no module for them and no dependency
   edge to draw. `bool`, `if`/`unless`, and the surface comparisons are ordinary words and
   split cleanly. Gating the builtins behind an `intrinsics` import (rather than leaving
   them ambient) is a design choice made explicit in `../P8-packages-modules.md`: the
   builtins are the language surface, but "reachable only via `import: intrinsics * ;`" is
   what makes a bare-metal file's intrinsic surface auditable in one line, which is the
   forcing consumer that justifies the gate.

## Resolved recon (probe-verified against the built compiler, 2026-08-19, `main` at

`e43df46`; probe files built, run, and deleted, `git status` and `cargo test` clean
afterward)

The prelude-deletion half was probed before speccing, because "every file just gains
`import:` lines" assumes the imported words behave like ordinary words, and they don't:
`if`/`unless` are row-typed inline combinators taking quotation parameters and
`=`/`<`/`>` are `'T: Copy Ord` inline words going through operator dispatch. Four of the
five probes confirmed the plan; one falsified a decision.

1. **An imported inline combinator splices correctly, qualified and selectively.** A copy of
   `if`/`unless` in a second module, exported and imported, produced correct branch
   selection through both a qualified call and a selectively-imported bare name.

2. **Self-tail-call-to-loop lowering survives an imported combinator.** A `gcd`-shaped
   `count` recursing 5,000,000 deep through an *imported* `if` ran to completion with no
   stack growth, matching its prelude-`if` control. This was the largest suspected risk
   (INV-INLINE-COMBINATOR has the checker read the callee's body, and a spliced inline
   combinator is re-scoped under the callee's module) and it is not a risk.

3. **The corpus's actual shape works.** An `inline` poly word (the shape of
   `examples/poly_if.sth`'s `mymax`, `lib/arrays.sth`'s `sort`/`bin_search`,
   `lib/combinators.sth`'s `each`/`map`/`fold`/`filter`) calling an imported comparison
   *and* an imported `if` monomorphized and ran correctly at both `i64` and `f64`.

4. **Falsified: the prelude mangling exemption is load-bearing, not merely a bare-name
   convenience.** A **non-inline** poly word calling the prelude's poly `<`
   (`: mylt ( 'T: Copy Ord 'T -- bool ) < ;`) works; the identical word calling an imported
   copy fails with `` unknown word `lt2__m1` ``. Same-module non-inline poly-to-poly fails
   identically, so the underlying defect is the already-known generic-calls-generic gap and
   the exemption is what has been hiding it. Deleting the prelude does not cause this bug;
   it exposes it.

5. **Blast radius of finding 4: no live corpus word, but the next one written.** Every poly
   word in `lib/` and `examples/` that uses a comparison or `if` is `inline`, so nothing in
   the tree breaks. The paper-grammar `bin_search` in the untracked `lib/binary_search.sth`
   is exactly a non-inline poly word over a comparison, so this gap is the first thing a
   real `bin_search` hits.

## Decisions (settled here, not reopened by the spec)

- **`is_prelude_word_name` and `parser::prelude_words` are deleted, not deprecated.** Every
  existing `.sth` file — every example, every golden, every `lib/` file that isn't the newly
  split `core` modules themselves — gains explicit `import:` lines for what it uses. A
  corpus file that still resolves `if` through the deleted prelude is a build failure, and
  the fix is adding the import, not restoring the exemption.

- **The intrinsics are gated behind `import: intrinsics * ;`; the wildcard form and
  re-export are what make that bearable.** Qualified `1 2 i::add` is unreadable in a stack
  language, and a selective list of the ~40 `BUILTIN_WORDS` names per file is worse than
  the problem. The selective form (`import: intrinsics | dup drop add | ;`) stays available
  for the case where the intrinsic surface is worth documenting explicitly (a bare-metal
  file proving in source that it never touches `fill`). `export:` accepting a name a file
  imported, not only one it declared, is what makes a hub module: `core` can import
  `intrinsics` and re-export only the subset it endorses.

- **The wildcard's grammar is `<target> * ;`, mutually exclusive with a selective list, and
  binds no qualifier.** `*` is only the wildcard keyword in this exact position (right
  after the target, nothing following but `;`); inside `| ... |` it is always an ordinary
  word, so `import: mod | * | ;` selectively imports the word literally named `*` (already
  live for multiplication in this symbol-operator style, alongside `+`/`-`/`<`/`>`), not a
  wildcard. A reused sentinel there would collide with a real, plausible word name; reusing
  the qualifier slot instead costs nothing, since nobody plausibly wants an import
  qualifier literally named `*`. Wanting both a wildcard and a bound qualifier for
  redundant qualified access is not supported: it added nothing a full wildcard didn't
  already give, since everything is already reachable unqualified.

- **The migration is mechanical for the live corpus but is not purely mechanical, and the
  spec must rule on the difference** (resolved recon, finding 4). Deleting the exemption
  removes one capability that exists today: a *non-inline* polymorphic word may call the
  prelude's poly comparison and may not call an imported one. No word in the tree uses that
  capability, so nothing breaks on migration, but the spec must choose explicitly between
  two options rather than discovering this in review: (a) declare the generic-calls-generic
  fix a hard prerequisite of this slice, which moves work into P7 and grows the slice, or
  (b) accept the narrowing for now, with a located diagnostic and a rejection test naming
  it, so the next author to write a non-inline poly `bin_search` gets an error that
  explains itself instead of `` unknown word `<__m1` ``. **(b) is the recommendation**: the
  capability has no current user, the diagnostic is small, and bundling a type-system fix
  into a packaging slice is how a slice stops being reviewable. A silent third option —
  leaving the exemption in place for comparisons only — is declined, since it keeps the
  hole this slice exists to close.

## Open questions for the spec

- **The wording of the narrowing diagnostic**, if the spec takes option (b) above: a
  non-inline poly word calling an imported poly word needs a located error naming the
  caller, the callee, and the reason (a polymorphic callee is not yet reachable from a
  polymorphic body across a module boundary), not the raw `` unknown word `<__m1` `` that
  leaks the mangled name today.
- **The test-fixture migration mechanics**: the ~460 inline test sources point at a shared
  manifest via `slice1b-brief.md`'s `--manifest` flag rather than each carrying its own, but
  the exact shape of that shared fixture manifest (one for the whole suite, or one per test
  module grouping) is not decided.

## Out of scope

- The manifest, package attribution, module naming/visibility, cross-package resolution,
  and the layer check are `slice1-brief.md`, not here — this slice depends on them existing
  (recon finding 3, decisions above) but adds no manifest-level mechanism of its own.
- The `--manifest` CLI flag and fallback chain are `slice1b-brief.md`.
- Semver enforcement, the serializable API description, and `sooth publish --check` stay
  P8.S3, per `docs/roadmap/P8-packages-modules.md`.
- A manifest flag making a dependency's exports visible unqualified package-wide (the
  documented escape hatch for prelude ergonomics) is not built in this slice.

## Sequencing

1. Add the bare `*` wildcard import form (parsed and its `*`/`| * |` disambiguation already
   in place per S1a; this step implements the visibility semantics) and re-export through
   `export:` (needed before the migration below, since intrinsics-heavy files depend on the
   wildcard to stay readable).
2. Delete the compiler-baked prelude (`parser::prelude_words`, both call sites, the
   `is_prelude_word_name` exemption), gate `BUILTIN_WORDS` visibility behind an
   `intrinsics` import, and add the narrowing diagnostic and its rejection test.
3. Split `lib/core.sth` into modules with a hub (recon finding 3), and migrate every corpus
   file and inline test source to explicit imports of it and of `intrinsics`.

## Exit

No word resolves without an `import:`, including the intrinsics; `is_prelude_word_name` and
`parser::prelude_words` are deleted; a hub module re-exports an imported word and a consumer
uses it; the corpus builds and every golden passes; a non-inline poly word calling an
imported poly word is a located error naming the caller, the callee, and the reason.

## Ready to spec?

Yes, with one ruling to make explicit (the narrowing-diagnostic option, (b) recommended)
and one open question (the shared test-fixture manifest's shape) that belongs in the spec.
Recon is source-verified and the prelude-deletion mechanics are additionally
probe-verified; the one thing probing falsified is already a stated decision above, not a
loose end.
