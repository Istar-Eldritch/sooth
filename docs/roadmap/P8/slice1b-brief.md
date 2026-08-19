# Phase 8 Slice 1b: the `--manifest` CLI flag and the fallback chain (brief)

Split out from `slice1-brief.md`: this piece touches `main.rs`/`driver.rs` CLI wiring and
answers "which manifest resolves this file," while the rest of S1 is checker/resolver work
answering "what does a manifest mean once found." They share the manifest grammar and
nothing else — no file, no checker pass, and this slice's one open question (diagnostic
wording per fallback tier) doesn't touch any of S1's five. The design is in
`../P8-packages-modules.md`, under "A manifest is optional, and resolution falls back three
ways."

## Recon (measured against the built compiler, 2026-08-19, `main` at `c1a0883`)

1. **The CLI takes a bare entry file, not a project root.** `main.rs:16-35` dispatches
   `build <file.sth>` / `run <file.sth>` straight to `driver::build`/`driver::run`, both
   typed `&Path` to a single file (`driver.rs:403`). There is no existing notion of "the
   current package" or "a named manifest" for either to resolve against, and no flag
   parsing beyond positional arguments today.

## Decisions (settled here, not reopened by the spec)

- **Resolution falls back four ways, ranked**: an explicit `--manifest <path>` on the CLI
  invocation; failing that, the nearest ancestor manifest; failing that, the user-level
  manifest at `$XDG_CONFIG_HOME/sooth/global_sooth.pkg` (the same file the REPL reads for a
  session, retiring the separately-deferred "REPL user-level manifest" question); failing
  all three, an implicit anonymous package with no dependencies (can import `intrinsics`
  and its own path-derived siblings; naming any other package is a located error).

- **Ruled: `--manifest` unconditionally overrides an ancestor manifest**, silently, with no
  separate conflict diagnostic. A named, deliberate act at the call site beats a discovered
  one, full stop — this is also why the flag is exempt from the rule that bars the
  user-level manifest for any file with an ancestor manifest (that rule exists to keep a
  package's build independent of machine-local config; a CLI flag pinned by the invoker,
  or checked into a script/CI config, is exactly as reproducible as pinning the entry file
  itself).

- **This is what makes S2's test-fixture migration affordable.** The ~460 inline test
  sources are written to a temp location per fixture and cannot carry a stable relative path
  or inherit the user-level manifest without making CI depend on developer config. With
  `--manifest`, the harness points every fixture at one shared manifest on the command line
  instead of generating one per fixture.

## Open questions for the spec

- **The ordinary located error when a named `--manifest` doesn't resolve the file's
  imports** (a module or package the flag's manifest doesn't grant) — same failure shape as
  an ancestor-manifest failure today, not a new diagnostic category, but needs its wording
  confirmed.
- **A diagnostic per fallback tier** (paper dogfood finding F9, `P8/dogfood/README.md`). A
  file naming `core::bool` with no ancestor manifest can fail two different ways — no
  user-level manifest at all, or one that doesn't list `core` — and each wants a different
  remedy stated in the message (write `$XDG_CONFIG_HOME/sooth/global_sooth.pkg`, versus add
  a `depends:` line to it). The implicit-anonymous-package case must name itself in the
  error, or the user is told a package is missing without being told why their file isn't
  in one.

## Out of scope

- The manifest grammar itself, package attribution, module naming/visibility, cross-package
  resolution, and the layer check are `slice1-brief.md`, not here.
- Single-mode imports and the prelude deletion are `slice2-brief.md`, not here.

## Sequencing

1. Add `--manifest <path>` parsing to `main.rs`'s `build`/`run` dispatch.
2. Implement the four-tier resolution order in `driver.rs`, ahead of `discover_closure`'s
   existing walk.
3. Add the per-tier diagnostics.

## Exit

`sooth build entry.sth --manifest path/to/sooth.pkg` resolves against the named manifest
regardless of `entry.sth`'s own directory; a manifest-less, flag-less file resolves against
the user-level manifest, then falls back to an implicit anonymous package with no
dependencies; each fallback tier's failure names its own remedy.

## Ready to spec?

Yes. One open question (per-tier diagnostic wording) and a small, source-verified recon; it
depends on `slice1-brief.md`'s manifest grammar existing but adds no shape of its own beyond
what's decided above.
