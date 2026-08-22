# P8.S2 spec — single-mode imports, the intrinsics module, wildcard, and re-export

**Slice:** P8.S2 (delivered) · **Design authority:** `docs/roadmap/P8-packages-modules.md`
(sections "Modules re-export", "The intrinsics are a module too", and the P8.S2 plan block)
· **Brief:** `docs/roadmap/P8/slice2-brief.md` · **Builds on:** P8.S1a (manifest grammar,
module names, `export:` gating, `resolve::NameTables`), P8.S1b (`--manifest`,
`ResolutionConfig`).
**Siblings (out of scope):** S1a/S1b (delivered); S3 (API description, semver,
`publish --check`).

---

## What shipped and why

Before this slice every program silently inherited `lib/core.sth` (`if`, `unless`, the six
surface comparisons) through a compiler-baked prelude, and the ~40 `BUILTIN_WORDS` dispatched
ahead of any environment lookup with nothing gating them. Now **no word resolves without an
`import:`** on the file/driver path, the intrinsics included:

1. The bare `*` wildcard, which used to be a hard build error against a real target, desugars
   to a selective import of every name on the target's `export:` list.
2. `export:` re-exports an imported name, via a new per-module exported-name → origin-module
   table resolved to its declaring module across hub-of-hubs chains, with located cycle,
   existence, and ambiguity rejections (`export:` did no existence check at all before).
3. The prelude is deleted (`parser::prelude_words`, both injection sites, the
   `is_prelude_word_name` mangling exemption, and the six surface comparisons' second
   un-mangling carve-out in `is_operator_dispatch_name`), and `BUILTIN_WORDS` *visibility* is
   gated behind an `intrinsics` import.
4. `lib/` became a real `core` package with a re-exporting hub (`core::prelude`), and every
   `.sth` file plus every file-based test source carries explicit imports.

Deleting the prelude narrowed one capability with no user: a **non-inline** polymorphic word
could call the prelude's poly comparison and cannot call an imported one. Per the brief's
decision (b), that is accepted with a located diagnostic and a mutation-guarded rejection
test, not fixed by dragging the P7 generic-calls-generic work into a packaging slice.

---

## Rulings

### R1 — Wildcard visibility desugars to an all-exports selective import

A wildcard of a resolved target `t` inserts every name of `exports_by_module[t]` into the
importing module's `selective_map`/`selective_entries`, reusing `NameTables::rewrite`'s
selective branch and `check::check_selective_imports` with no new resolution path. The
synthesized `SelectiveName` carries **no qualifier** (a wildcard binds none, so there is no
qualified spelling) and the importer's own `import: ... * ;` span, never the exporting file's
`export:` span, so a collision diagnostic lands in the right file. Consequences that had to be
built out:

- `SelectiveName::qualifier` became `Option<String>`, and `check_selective_imports`'
  collision/not-exported messages gained wildcard-specific wording
  (`wildcard import of `p`  collides with a local ...`) rather than a fabricated qualifier that
  would misdescribe the import shape.
- A target's `export:` list may repeat a name across two `export:` blocks, which
  `scan_exports` does not dedup; the desugaring dedups, otherwise a wildcard synthesizes two
  entries and falsely self-collides where the qualified and selective forms build fine.
- A wildcard-bound name colliding with a local declaration stays a hard error, inheriting the
  existing collides-with-local rule; no wildcard shadowing case was invented.
- The duplicate-qualifier scan is skipped (a wildcard binds no qualifier); the reserved
  `intrinsics` wildcard keeps its `continue`, and the no-target wildcard keeps its rejection.
- Wildcard re-export (`export: * ;`) is not provided, per the design doc.

The re-exporting-hub case (`import: core::prelude *`, the headline ergonomic) is a **phase-2**
capability: the selective branch only binds names the hub does not declare once R4's origin
table is threaded into it.

### R2 — The `intrinsics` module gates `BUILTIN_WORDS` visibility

`BUILTIN_WORDS` and `is_builtin_word_name` are unchanged, and `has_self_tail_call` keeps
reading them. Only visibility is gated, by `IntrinsicVisibility { All, Only(BTreeSet), None }`
on `ModuleInfo`, populated in `assemble_module`'s import loop:

- `import: intrinsics * ;` → all builtins visible; `| ... |` → that subset; nothing → none.
- Multiple `intrinsics` lines **accumulate** (union of selective names, any wildcard wins)
  rather than the last one winning, matching how `export:` and `module:` entries accumulate. A
  qualified `import: intrinsics i ;` with neither `*` nor a `| ... |` clause widens nothing:
  there is no qualified spelling for an intrinsic.
- The import is recognised exactly as `packages::resolve_import` recognises it
  (`ImportAnchor::Dependency`, `segments == ["intrinsics"]`, no closure edge);
  `self::intrinsics` stays an ordinary module name.

**Gate set:** `is_gated_intrinsic_name` = `is_builtin_word_name` **minus
`{eq, lt, gt, lte, gte, ne}`**. Those six are `lib/` words (they left `BUILTIN_TABLE` in slice
10c and sit in `BUILTIN_WORDS` only so `has_self_tail_call` does not misread a trailing `lt`),
so gating them would answer an unimported `lt` with "add `import: intrinsics *`", pointing at
the wrong module. `.` stays **in** the set: it is a genuine table intrinsic with a `Print` row
per printable type.

**Placement.** `check_term` computes `gated` once and skips every builtin-dispatch arm for a
gated name (including the `branch` interception), letting it fall through to the ordinary
env/overload path, which reports `ungated_intrinsic_error` instead of `unknown word`. The env
lookup never actually claims such a name: a module's own word under a builtin spelling arrives
mangled, and the two un-mangled categories are not `env` entries under the bare name, so the
fall-through is always the diagnostic. `check_poly_term` carries the **same gate as its own
first check**, without the env-candidate dance: a generic body dispatches the same builtins on
its own path, so without it an unimported `dup` would be gated in a monomorphic word and free
in a polymorphic one.

Two exemptions are load-bearing. The gate keys on **`span.module`** (where the term was
written), not `ctx.module()`: a caller's `~[ ... ]` argument spliced into a library combinator
is checked under the library's module and would otherwise be judged against the library's
imports. And a term with `span.line == 0` (compiler-synthesized, e.g. `bool_print_word_def`)
is exempt: there is no file to add an import to. The gate cannot fire on the REPL/`Ctx::Line`
path (`ctx.modules()` is `None`), the same exemption the `drop` visibility gate has, so the
invariant is a file/driver-path rule and the prompt keeps its existing module-check bypass.

### R3 — The prelude and its mangling exemption are deleted

`parser::prelude_words`, its injection in `parser::parse`, and the `assemble_module` twin are
gone; `is_prelude_word_name` is gone and `resolve::mangle` now exempts exactly `main` and
`drop`. Every live consumer was migrated, not merely made to compile:

- `repl.rs` `Session::new`'s prelude seed loop is deleted: a session that wants `if`/`lt`
  writes an import exactly as a file does, so a bare comparison with no import is
  `unknown word`, matching a compiled build. The neighbouring `bool_print_word_def` seed is
  not a prelude word and stays.
- `repl.rs`'s `is_prelude_word_name` clause in the import-rename filter is removed: it existed
  because prelude words were both session-seeded and closure-injected, a dual existence a
  rename would strand. An imported closure's `core` words are ordinary module-0 words now and
  epoch-rename like any other import; the `main`/`drop`/`.` exclusions beside it stay.
- The `word_entry.rs` test declares its own witness; the `parser.rs` word-count assertion
  drops the injected addend; `resolve.rs`'s `single_module_closure_is_left_unchanged` loses the
  filter and the stale premise comment; the `phase4_slice10c_primitives.rs` readers retired
  with the prelude. `check.rs`'s `infer_src` is a `#[cfg(test)]` bare-line inference helper,
  not the REPL seed, and keeps seeding in process (R7).

### R3a — `is_operator_dispatch_name` drops the six surface comparisons

`rewrite` guards both its own-module and selective branches on
`!is_operator_dispatch_name(core)`, so while the six were listed a bare comparison call stayed
unrewritten. That was harmless only because the *declaration* was unmangled too; once R3
mangles the decl to `lt__mN`, the bare call resolves to nothing. Dropping them regresses no
dispatch: they are not `BUILTIN_TABLE` keys (their rows moved to the `u`-prefixed primitives in
slice 10c), `check_operator::is_operator` does not list them, and `scoped_operator_overloads`
early-returns for any name with no table row. The genuinely overloaded names stay
(`add sub mul div mod and or xor not shl shr`, the `u`-prefixed comparisons, `max`,
`max-total`, `.`). The six are ordinary words now, reached by import through the normal
machinery.

### R4 — Re-export: an exported-name → origin-module table with a per-pair walk

`Visibility { exports, exported_origin: Vec<HashMap<String, u32>> }` bundles the cross-module
lookup tables into one `rewrite`/`rewrite_terms` parameter (both already sit at clippy's
argument ceiling). Built in `resolve_modules` before any body is rewritten, since a body may
reference a name through a hub whose export list has not been reached yet. Per `(m, name)` on
an export list, the immediate source is:

- `m` itself, if `name` is in `exportable_names(m)`;
- else `m`'s selective map (already populated for wildcards by R1);
- else a scan of the modules `m`'s `import_maps` **values** point at, testing each one's own
  declaration set. This is not a lookup of `name` *in* `import_maps` (keyed by qualifier, so
  it always misses). A hub that imports a dependency qualified-only reaches `lw` as `dep::lw`
  alone, and the compiler must not restrict which import shapes are re-exportable.

`exportable_names` is deliberately **wider than `NameTables`**: words, `extern:`s, structs,
generic structs, enums, generic enums, and the variant constructors of both enum kinds.
`NameTables` holds only what a call site can be mangled against, but `lib/result.sth` exports
`Result`, `Ok`, and `Err`, so an existence check keyed on the mangling tables alone would
reject the whole shape. `static:` names are excluded: a static is module-private and reachable
only by `&NAME` in its own module, so exporting one promises what no importer can reach.

Origins from the qualified scan are keyed **by module id**, so one module bound under two
qualifiers is one origin, not an ambiguity; the lower qualifier spelling is shown so the
diagnostic does not depend on hash order. Two or more distinct origin modules is a located
`ambiguous_re_export_error` (R6c), not a silent first-import-wins pick: `export:` is a flat
name list, there is no `export: dep1::lw ;` to disambiguate with, and a tiebreak would
privilege declaration order.

Chains are resolved by `walk_to_origin`, following immediate sources with a **per-resolution
visited set keyed on the `(module, name)` pair**. Pair-keying is load-bearing: a hub
re-exporting two names through one downstream hub is a diamond, not a cycle. A revisit before
reaching a declaration is a located `re_export_cycle_error`. (A "repeat passes until nothing
changes" fixpoint was rejected: on a cycle it never stabilises and has no per-pair structure
at which to notice the revisit.)

`rewrite` consults `Visibility::origin(target, name)` in both the qualified and selective
branches, for words and for types, after its own local-decl branches. No separate export gate:
an `exported_origin` entry exists only for a name on the target's own `export:` list, so the
entry *is* the promise. Own-module resolution still runs first, so a hub's re-export never
shadows a consumer's same-named local word. **No new AST**: a re-export is detected purely by
"not a local decl, so its origin is in the import map."

### R5 — `export:` existence validation lands here

A name that is neither declared by nor imported into the exporting module has no origin and is
now `export_unknown_name_error`, raised where the table is built (`export: nonexistent ;` built
and ran clean before). Both this and the ambiguity check fall out of the same construction
pass. `check_exported_signatures` keeps its distinct private-type-in-signature job. The
pre-landing corpus grep found no `export:` relying on the old silent pass.

### R6 — Diagnostic wording

All located, all naming the surface spelling. The two `export:` messages identify the
exporting module by its located site rather than by name: a module carries no name in the
resolved closure, only importers spell one as a qualifier.

```
error: `<word>` is an intrinsic and is not imported in `<caller>` (line L, col C)
  add `import: intrinsics * ;` (or `import: intrinsics | <word> ... | ;`) to this file

error: `<caller>` cannot call the polymorphic word `<callee>` (line L, col C)
  a polymorphic word is not yet reachable from another polymorphic word across a module boundary
  inline the caller, make the callee concrete, or call the callee from a monomorphic word

error: `<name>` in `export:` names nothing declared or imported in this module (line L, col C)
error: `<name>` re-exports itself through a cycle of `export:` chains (line L, col C)
error: `<name>` in `export:` is declared by more than one qualified-imported module (`<q1>`, `<q2>`) and cannot be re-exported without disambiguation (line L, col C)
```

(b) replaces the raw `` unknown word `<__m1` `` that leaked a mangled name; it fires ahead of
`poly_call_term`'s fall-through when the demangled call names a polymorphic word, and the
same-module non-inline poly-to-poly gap emits the identical message, since both are the one
underlying generic-calls-generic gap.

### R7 — One shared test-fixture manifest, and imports appended by the harness

Every migrating file-based source depends on the same two things (`core` and `intrinsics`), so
there is one shared fixture manifest (`tests/fixtures/sooth.pkg`, plus `fixture_package` for
trees that need their own), passed through `build_example`'s switch to
`driver::build_with_manifest`. Because several hundred goldens would otherwise each need hand
edits, `tests/common/mod.rs` gained a private `fixture_imports`, appended by exactly two
wrappers (`write_fixture` and `fixture_source`), so the "which imports" rule has one
implementation. It is explicitly **not** for a fixture whose subject is `import:` itself:
`tests/phase8_slice2.rs` writes those verbatim through its own `write_raw`. In-process
`check_src`/`check_error` tests, `infer_src` included, never resolve `import:` and keep seeding
in process; they are not part of the `--manifest` fixture set.

### R8 — `lib/` is the `core` package, with `core::prelude` as the hub

`lib/sooth.pkg` declares `package: core ; layer: core ; module: bool cmp prelude combinators
option result ;`. `bool` and `cmp` hold the Sooth half of the old `lib/core.sth`; the compiler
half (`branch`, `tag`, the `u`-prefixed comparisons) did not move and is reached via
`import: intrinsics`. `core::prelude` declares nothing and re-exports `if unless eq lt gt lte
gte ne`, so a consumer spends one `import: core::prelude * ;` on the typed surface and
`import: intrinsics *` only where it wants the raw builtins. That hub path works only because
of R3a. `core` is an ordinary package, not a compiler-reserved name. `examples/` likewise
became a package with its own committed `sooth.pkg` (source, not an artifact, per `.gitignore`).

---

## Implementation

Three phases; the third was necessarily atomic. Deleting the prelude strips `if` and the
comparisons from every closure and activating the gate strips ambient `BUILTIN_WORDS`, so every
golden fails until the corpus gains its imports, and the imports cannot go first (while the
prelude still injects `if`, a file's own import double-binds it). No ordering leaves the tree
green except deletion, gating, split, and migration together.

- **Wildcard visibility (R1):** the desugaring in `driver::assemble_module`'s import loop, with
  the `Option<String>` qualifier and wildcard-specific `check_selective_imports` wording
  (`src/check/declarations.rs`, `src/repl.rs`) in `f85bd23`, repeated-export-name dedup in
  `ab0eb43`, and the exact-diagnostic assertion in `d741b79`. `wildcard_import_is_error` is
  retargeted to the no-target case and its test asserts the wildcard binds.
- **Re-export (R4/R5):** `Visibility`, `exportable_names`, `build_exported_origin` /
  `resolve_export_origins` / `walk_to_origin`, the three new errors beside
  `not_exported_error`, and the origin consults in `rewrite`'s four branches in `src/resolve.rs`
  — `9598c57`, with the golden suite in `tests/phase8_slice2.rs` (hub qualified and bare,
  wildcard-of-a-re-export, hub-of-hubs, qualified-only re-export, consumer's own word
  outranking a re-export, a re-exported type through both branches, enum-variant export,
  unknown name, two-dep ambiguity, and a name the hub does not export).
- **Prelude deletion, gating, split, migration (R2/R3/R3a/R6/R7/R8):** `d539032d` deletes the
  prelude and its consumers, adds `IntrinsicVisibility`/`widen_intrinsics`/`names_the_intrinsics`
  and the `check_term` gate, drops the six from `is_operator_dispatch_name`, splits `lib/` into
  the `core` package, and migrates every `.sth` file and file-based test source.
  `92cd951` retargets the prelude's stale references in `DESIGN.md`/`README.md` and
  mutation-guards the new gates; `74ccba3` scopes `fixture_imports` to its two wrappers;
  `8098ca1` drops a permissive `import: intrinsics *` that had blinded a gating test to its own
  claim; `9c9981b` addresses review cycle 1; `fe0e09c` prunes the dead env lookup from the poly
  gate and documents why there is nothing there to defer to.

---

## Exit criteria (met)

- No word resolves without an `import:` on the file/driver path, intrinsics included; the
  REPL/`Ctx::Line` prompt is exempt because the gate cannot fire where `ctx.modules()` is
  `None`. `parser::prelude_words` and `is_prelude_word_name` are gone, `mangle` exempts only
  `main`/`drop`, and `is_operator_dispatch_name` no longer lists the six comparisons.
- A hub re-exports an imported word (and an imported type, and an enum's variants) and a
  consumer uses it qualified and bare, through two hops and through a qualified-only import; an
  unresolvable name and a two-origin ambiguity are each located errors, not a hang or a
  first-wins pick. The `re_export_cycle_error` guard in `walk_to_origin` is *not reachable from
  a source program*: a re-export cycle cannot arise without an import cycle (an immediate source
  only exists via a real import edge, and a self-source ends the walk), and import cycles are
  caught by module-closure construction before `resolve_export_origins` runs. It is exercised
  only by `export_re_export_cycle_is_a_located_error`, which feeds a hand-built cyclic
  `immediate` table straight into `resolve_export_origins`; no end-to-end golden reaches it,
  because no source can. The guard stays as defensive code — diagnostic, not safety — and its
  pair-keyed visited set is still the sound way to tell a diamond from a cycle should that
  reasoning ever change.
- A wildcard binds every exported name of its target, including names the target re-exports;
  the reserved `intrinsics` wildcard makes the builtins visible, and a selective `intrinsics`
  import is a real subset in both monomorphic and polymorphic bodies.
- A non-inline poly word calling an imported or same-module poly word is a located error naming
  caller, callee, and reason; the branch is mutation-guarded (deleting it falls back to the raw
  unknown-word and fails the test).
- The corpus builds, every golden passes, and
  `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.

---

## Out of scope

- The generic-calls-generic P7 fix (brief decision (b): narrowed with a diagnostic).
- Manifest grammar, package attribution, module naming/visibility, cross-package resolution,
  the layer check (S1a); `--manifest` and the fallback chain (S1b) — depended on, and modified
  only in one respect: commit `9c9981b` narrowed the in-package quoted-path rejection guard in
  `packages.rs::resolve_import` from `site.origin != SiteOrigin::UserLevel` to
  `site.origin == SiteOrigin::Ancestor`, so a `--manifest` (Flag-origin) entry file may now
  import siblings by quoted path where before only non-`Ancestor`/non-`UserLevel` sites could.
  This is what R7's shared fixture manifest harness needs (temp-dir entry files passed
  `--manifest` that then import sibling fixtures by quoted path); it is tested by
  `flag_site_still_allows_a_quoted_path_import`.
- Wildcard re-export (`export: * ;`), manifest path tables, and a package-wide
  unqualified-exports escape hatch (design doc: declined / deferred).
- Semver, the serialisable API description, `sooth publish --check` (S3).
