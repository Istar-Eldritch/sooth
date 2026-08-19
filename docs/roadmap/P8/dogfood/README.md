# P8 paper dogfood: the module system on paper

**None of this compiles.** It is the Phase 8 model written out as source before the model is
built, in the same spirit as `docs/roadmap/P7/slice3-dogfood.md`, which found two missing
slices by hand-writing a phase's target program against features that did not exist yet.
The design is in `../../P8-packages-modules.md`. Findings below are the point of the
exercise; the files are the means.

## Layout

```text
core/sooth.pkg              package: core, layer: core, module: bool cmp text
core/bool.sth               core::bool   -- if/unless over branch/tag
core/cmp.sth                core::cmp    -- eq/lt/gt/... over the u-prefixed flags
core/text.sth               core::text   -- a hub: no words of its own, re-exports below
core/text/ascii.sth         core::text::ascii
core/text/utf8.sth          core::text::utf8
core/detail/scratch.sth     core::detail::scratch -- package-private (not in module:)
collections/sooth.pkg       package: collections, layer: fixed, depends: core
collections/vec.sth         collections::vec
app/sooth.pkg               package: app, layer: hosted, depends: core collections
app/main.sth                a consumer naming modules across two packages
scratch.sth                 no ancestor manifest: resolves via the user-level manifest
global_sooth.pkg            stands in for $XDG_CONFIG_HOME/sooth/global_sooth.pkg
```

What each piece is there to exercise: path-derived nested names (`text/ascii.sth` is
`text::ascii`, with no manifest entry of its own), a hub curating two children, a
package-private module the manifest omits, both import modes (bare `*` for leaf code,
an explicit list where the intrinsic surface is worth reading), the layer ladder
core -> fixed -> hosted, and a file with no ancestor manifest falling back to the
user-level manifest. It is written in the model's *final* shape (P8.S1's packages already
in place), which is what surfaced F1 below: the slice that builds single-mode imports (S2)
has no earlier stage where this tree's imports would have been spellable at all.

## Findings

**F1. The corpus migration needed manifests to exist first — ruled, not left open.** With
the quoted-path form deleted there is no spelling for a cross-file import before a manifest
exists: nothing can say `core::bool` without a `depends:` table to resolve `core` against.
The ~460 inline test sources are what make it bite rather than the 48 `.sth` files, since an
inline source is written to a temp location and compiled, so it cannot carry a stable
reference to `lib/` and cannot inherit the user-level manifest without making CI depend on
developer config. Two ways out were weighed: reserving `core` as a compiler-known package
the way `intrinsics` is (cheap, but re-privileges the standard library this phase exists to
de-privilege) lost to reordering — manifests and module names first (P8.S1), single-mode
imports second (P8.S2), so the corpus migrates once to its final form. The phase file now
carries that order. The remaining cost is cheaper than first written down here: S1 also
adds an explicit `--manifest <path>` flag (`sooth build`/`run`), so the harness points every
inline fixture at one shared manifest on the command line instead of generating one per
fixture, and stays reproducible because the path is named at the call site rather than
discovered. The user-level manifest is still not an escape from that cost: a fixture
inheriting a developer's global config is a reproducibility hole, and the model forbids the
fallback for any file with an ancestor manifest; `--manifest` is a different thing, a named
override rather than a discovered one, which is why it's exempt from that rule.

**F2. Intra-package module references need a stated base.** `core/text/ascii.sth` says
`import: cmp c | lt gt | ;`, a sibling named without the package prefix, while the hub says
`text::ascii`. Both read naturally, and they are only consistent if the rule is *module names
are package-root-relative inside their own package, and `pkg::`-qualified across packages*.
That rule is not written down anywhere yet. The alternative (always fully qualified, even
internally) is more uniform and more verbose, and would make moving a package harder for no
gain.

**F3. Is `text.sth` beside `text/` legal?** The hub pattern needs it: `core::text` is a file
and `core::text::ascii` is a file in a directory of the same name. Nothing in the design
forbids it and everything in the design assumes it, but a name-derivation rule has to say so
explicitly, along with what happens if a package has both `text.sth` and `text/` where the
latter contains a file that would also derive `text` (it cannot, but the checker must say so
rather than pick one).

**F4. The minimum preamble is real and visible.** `app/main.sth` spends three import lines to
print two numbers. That is the honest cost of single-mode imports, and it is worth looking at
before committing to it rather than after migrating 500 sites.

**F5. Both import modes earn their place, which was not obvious in the abstract.**
`core/bool.sth` naming `| branch tag |` explicitly documents its entire intrinsic surface in
one line, which is exactly the bare-metal auditability argument for the intrinsics module.
Leaf code like `text/ascii.sth` using bare `*` for `intrinsics` would be unreadable spelled
out. Neither form is redundant.

**F6. `intrinsics` takes no `depends:` entry.** No manifest here declares it, on the grounds
that it is compiler-provided rather than resolved to a source location. That was decided by
writing the files rather than by ruling, so it needs ratifying. The related question the
layer check must answer: `intrinsics` has no manifest and therefore no `layer:`, so the
dependency-direction rule has to treat it as below every layer.

**F7. `module:` is optional and an application proves it.** `app/sooth.pkg` declares none: an
application exposes nothing to anyone. So the manifest's required fields are `package:` and
`layer:`, with `depends:` and `module:` both empty-by-default.

**F8. Untested by this scaffold, and worth a fixture when it is built:** a hub re-exporting a
*type* (the Elm-style whole-name-scope rule crossing two boundaries at once), two hubs
re-exporting the same word name into one consumer, and a `depends:` cycle between packages
(the file-level cycle checker should already catch it, but through a confusing message that
names files rather than packages).

**F9. The three-tier fallback needs a diagnostic per tier.** A file with no ancestor
manifest that names `core::bool` can fail two different ways: no user-level manifest at
all, or one that does not list `core`. Those are different remedies (write
`$XDG_CONFIG_HOME/sooth/global_sooth.pkg`, versus add a `depends:` line to it) and want
different messages. The implicit-anonymous-package case also has to name itself in the
error, or the user is told a package is missing without being told why their file is not
in one.
