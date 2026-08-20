# P8.S1b spec — the `--manifest` CLI flag and the fallback chain

**Slice:** P8.S1b · **Design authority:** `docs/roadmap/P8-packages-modules.md`
(section "A manifest is optional, and resolution falls back three ways")
· **Brief:** `docs/roadmap/P8/slice1b-brief.md` · **Builds on:** P8.S1a (`415fb60`)
**Siblings (out of scope):** S1 (`slice1-brief.md`: manifest grammar, package attribution,
module naming/visibility, cross-package resolution, layer check — all delivered by S1a),
S2 (`slice2-brief.md`: single-mode imports, intrinsics gating, wildcard semantics,
re-export, prelude deletion).

---

## Scope

The CLI-level question of *which manifest resolves a given invocation*. Exactly three
things, no new file and no new checker pass:

1. A `--manifest <path>` flag on `sooth build`/`run`, ranked above discovery.
2. The two fallback tiers S1a did not build: the user-level manifest at
   `$XDG_CONFIG_HOME/sooth/global_sooth.pkg` (tier 3), and the diagnostic-bearing form of
   the implicit-anonymous-package baseline (tier 4).
3. A distinct located diagnostic per tier (dogfood finding F9,
   `docs/roadmap/P8/dogfood/README.md`).

Not this slice: everything S1a already shipped (manifest grammar/parser, `PackageSite`,
`ManifestCache`, `resolve_import`'s per-anchor logic, `check_package_graph`, the layer
check); S2's single-mode imports and prelude deletion; the REPL's own resolution path
(see Out of scope).

---

## Recon (re-verified against `main` at `415fb60`, 2026-08-20)

Every entry checked against the live source, not the brief's line numbers.

| Fact | Site (verified) |
| --- | --- |
| CLI dispatches `build`/`run` on a bare `<file>`, no flag parsing | `src/main.rs:17-32` (`args.get(2)` straight into `Path::new`) |
| `driver::build(&Path)` → `emit_ssa` → `discover_closure(path)` | `src/driver.rs:477`, `464-465` |
| `driver::run(&Path)` calls `build` | `src/driver.rs:503` |
| `discover_closure(entry)` seeds a fresh `ManifestCache`, then audits | `src/driver.rs:73-78` |
| `discover_closure_with` calls `package_of` once per file, threads `Option<PackageSite>` into `resolve_import` | `src/driver.rs:82`, `99`, `104-111` |
| `ManifestCache::package_of` returns `Ok(None)` when `find_package_root` finds no ancestor manifest — the tier-4 baseline (tiers 2 and 4 already exist) | `src/packages.rs:126-133`, `43-52` |
| `resolve_import` short-circuits `intrinsics` to `Ok(None)` *before* the `site` check, so intrinsics already resolves manifest-less | `src/packages.rs:455`, `~486` |
| A module-name import with `site == None` errors today via `module_import_without_manifest_error` — the diagnostic S1b replaces with tiers 3/4 | `src/packages.rs:489`, `219-226` |
| `MissingDepends`/`PrivateModule` are recorded and rendered by `check_package_graph`, which runs the layer walk first (can mask an import error) | `src/packages.rs:331-387`, `22-40` |
| `parse_manifest` requires both `package:` and `layer:` (end-of-file error if absent) | `src/manifest.rs:166`, `~270-284` |
| S1a deferred a `depends:`-only entry point for `global_sooth.pkg` to this slice, explicitly not routed through `parse_manifest` | `docs/roadmap/P8/slice1a-spec.md`, `manifest.rs` section |
| `PackageSite` = `{ manifest_path, root, manifest }`, `module_file` joins root + segments + `.sth` | `src/packages.rs:56-88` |

S1a keeps quoted-path imports legal for manifest-less files (`resolve_import`'s
`ImportTarget::Path` arm, `src/packages.rs:463-475`). That is unchanged here: a scratch
file's siblings are reached by quoted path.

---

## Rulings

### R1 — Fallback order (locked by the brief, restated for reference)

Ranked, first that applies wins:

1. **`--manifest <path>`** on the invocation (tier 1).
2. The **nearest ancestor manifest** (tier 2, S1a's `package_of`).
3. The **user-level manifest** at `$XDG_CONFIG_HOME/sooth/global_sooth.pkg` (tier 3).
4. An **implicit anonymous package** with no dependencies (tier 4): `intrinsics` and
   quoted-path siblings resolve; a `self::` or dependency *module name* is a located error.

`--manifest` unconditionally and silently overrides an ancestor manifest (brief decision,
not reopened): a named act at the call site beats a discovered one, and is exempt from the
rule barring the user-level fallback inside a package (a flag pinned in CI is as
reproducible as pinning the entry file).

### R2 — Open question 1: per-tier diagnostic wording

Three located diagnostics, all sharing S1a's `import_header` (`src/packages.rs:151-159`:
`` error: import `<target>` at line L, col C in <importer>: ``).

**(a) A named `--manifest` that does not resolve an import.** No new category: reuse S1a's
existing diagnostics verbatim, with the flag's manifest path substituted for the ancestor
manifest path. A missing `depends:` entry → S1a's `missing_depends_error` (A); a
non-public module → `private_module_error` (C); nothing at the joined path → D1
`module_not_found_error`. A tier-1 site is a *real package* (see R4) whose manifest is
loaded into the `ManifestCache`, so these keep flowing through `check_package_graph`
exactly as an ancestor site's do — layer check and depends-name-mismatch audit included
(see R4/F1: the flag manifest is loaded, not parsed standalone, so it is seeded into the
audit walk). This confirms the brief's "same failure shape as an ancestor-manifest
failure, not a new diagnostic category" — the only change is the manifest path named in
the remedy line.

**(b) Tier 3, user-level manifest present but lacking the package.** A manifest-less file
naming `<pkg>::<module>` where the user-level manifest has no `depends:` entry for `<pkg>`:

```
error: import `<pkg>::<module>` at line L, col C in <importer>:
  <importer> resolves against the user-level manifest <user-manifest-path>, which has no `depends:` entry for `<pkg>`
  add `depends: <pkg> path "<path>" ;` to <user-manifest-path>
```

Raised **inline** at resolution (not recorded for `check_package_graph`): a user-level
site is not a real package (R4), so there is no layer graph that could mask it and nothing
to defer for. `<path>` is a literal placeholder, as in S1a's A diagnostic.

**(c) Tier 4, no user-level manifest — the implicit-anonymous-package error.** A
manifest-less file naming any module name (`self::` or dependency) with no user-level
manifest present. Names the anonymous status (F9: "must name itself") *and* the remedy:

```
error: import `<target>` at line L, col C in <importer>:
  <importer> has no ancestor `sooth.pkg` and no user-level manifest, so it is an implicit anonymous package that can only import `intrinsics` and its own quoted-path siblings
  `<pkg-or-self>` cannot be resolved; write $XDG_CONFIG_HOME/sooth/global_sooth.pkg with a `depends:` entry, add an ancestor `sooth.pkg`, or pass `--manifest <path>`
```

This replaces `module_import_without_manifest_error` (`src/packages.rs:219`), whose
current single message ("add a manifest, or use a quoted-path import for now") predates
the user-level tier and no longer states the whole remedy set. A bare `self::<name>` in a
manifest-less file also lands here (a `self::` name needs a package identity, which the
user-level manifest does not supply — see R3), with `<pkg-or-self>` rendered as the
written target.

### R3 — Open question 2: closure scope of the `--manifest` override

**`--manifest` overrides `package_of` for the entry file only, not for every file in the
discovered closure.** A transitively-imported file re-derives its own package via the
existing per-file `package_of` walk; a nested package's own ancestor manifest still wins
for its own files.

Reasoning: the flag answers "which manifest resolves *this* invocation's entry file,"
matching the brief's framing and the reproducibility argument (a flag is as reproducible
as pinning the entry file — one file, not a build-wide rewrite of every package boundary).
A global override would make a dependency's layer and `depends:` table depend on the
caller's flag, which is incoherent: cross-package resolution and the layer check between
two *dependencies* must be a function of their own manifests, or S1a's guarantees
dissolve. It also matches the S2 use case exactly: a fixture is a single entry file whose
imports resolve via the shared manifest's `depends:`; the imported `lib/` modules resolve
against `lib/`'s own package manifest (tier 2), unchanged.

Consequently the user-level (tier 3) and anonymous (tier 4) fallbacks apply **per file**
wherever `package_of` returns `None` — a scratch file importing another manifest-less
scratch sibling gets the same fallback — while the flag override is entry-only. The two
rules do not conflict: the flag is consulted only for the entry file; every other file
walks tiers 2→3→4.

The override affects **dependency-anchored imports only** (bare first segment). A `self::`
import from a flag-overridden entry file is not supported: `self::` names a module of the
file's own package, which requires a real ancestor `sooth.pkg` identity, not a manifest
named at the call site (the live `resolve_self_module` owner guard, `src/packages.rs:502`,
requires the resolved module's owning `sooth.pkg` to equal the site's manifest, which a
flag manifest placed anywhere does not satisfy). Such an import is a located error
(`self_import_under_flag_manifest_error`, see the resolve_import bullet), directing the
invoker to an ancestor `sooth.pkg` or a dependency-anchored import. This matches the S2
fixture use case, which is dependency-only.

### R4 — Site model

Site selection stays a `resolve_import` input; the net-new work is choosing it. Three
site origins, distinguished so R2's diagnostics and the deferral behaviour differ:

- **Tier 1 (`--manifest`) and tier 2 (ancestor):** a *real package* `PackageSite`, rooted
  at the manifest's directory. The manifest is **loaded through the `ManifestCache`**
  (`manifests.load(p)`, which parses via `parse_manifest` — `package:`+`layer:` mandatory),
  not parsed standalone, so it becomes a key in `known_manifest_paths()` and
  `check_package_graph`'s layer-violation and depends-name-mismatch walk audits it exactly
  as it audits an ancestor manifest (tier 2 already loads through the cache via
  `package_of`, `src/packages.rs:126-133`). Parsing standalone would leave the flag
  manifest out of `known_manifest_paths()` and silently skip those two walk-only checks
  (F1). `MissingDepends`/`PrivateModule` are recorded and deferred to
  `check_package_graph`, unchanged from S1a. Layer checks apply.
- **Tier 3 (user-level):** a `PackageSite` rooted at the user-level manifest's directory
  (`$XDG_CONFIG_HOME/sooth/`), carrying only a `depends:` table (no `package:`, no
  `layer:`). It is **not** seeded into `check_package_graph`'s manifest walk, so it never
  acts as a *declaring* package in the layer check (a scratch file has no layer to
  violate). Dependency imports still load each dependency's *real* manifest, which does
  enter the walk, so layer relations *between the resolved dependencies* are still checked.
  A `MissingDepends` on a tier-3 site is raised inline as R2(b), not recorded.
- **Tier 4 (anonymous):** no site (`None`, exactly today's baseline). `intrinsics` and
  quoted-path resolve; a module name is R2(c).

The origin is carried on `PackageSite` (a `SiteOrigin { Ancestor, Flag, UserLevel }`
field; tier 4 is the absence of a site). `Ancestor` and `Flag` behave identically in
resolution and the audit — the distinction exists only so a future diagnostic can name the
flag if needed; today both are "real package."

### R5 — `--manifest` and user-level manifest parsing

- **`--manifest <path>`** names a full package manifest and is loaded through the
  `ManifestCache` (`manifests.load(p)`, which parses via the existing `parse_manifest`,
  `src/manifest.rs:166`, and seeds it into the audit walk — see R4/F1). A parse error or
  unreadable file surfaces that function's located error, prefixed to name the flag. The
  entry file need not sit under the manifest's directory (the temp-fixture case).
  Dependency imports (bare first segment) resolve against its `depends:`. `self::` imports
  are **not** supported under `--manifest` (R3): the override is dependency-anchored only,
  and a `self::` import from the flag-overridden entry file is a located error
  (`self_import_under_flag_manifest_error`).
- **The user-level manifest** is `depends:`-only. A new `parse_user_manifest(src, path)`
  in `manifest.rs` reuses the shared `depends:` line grammar (`expect_word`/`expect_str`/
  `expect_semicolon`, the `check_pkg_name` and `depends: intrinsics` rejections) but
  accepts **no** `package:`/`layer:`/`module:` lines (each is a located error naming the
  keyword as not allowed in a user-level manifest). It returns the `Vec<DependsEntry>`
  only. This is the entry point S1a's spec deferred here rather than routing a
  `package:`-less file through `parse_manifest`.

### R6 — `$XDG_CONFIG_HOME` resolution and test injectability

The user-level manifest path is `${XDG_CONFIG_HOME}/sooth/global_sooth.pkg`, falling back
to `${HOME}/.config/sooth/global_sooth.pkg` when `XDG_CONFIG_HOME` is unset or empty (the
XDG Base Directory default). A missing file is tier 4, not an error.

To keep tests race-free (parallel Rust tests must not mutate a shared process env, nor
read a developer's real config), resolution reads its inputs from a
`ResolutionConfig { manifest_override: Option<PathBuf>, user_manifest: Option<PathBuf> }`
threaded from the entry point, not from `std::env` deep in `packages.rs`.
`ResolutionConfig::from_env()` populates `user_manifest` from the XDG path above (present
only if the file exists) and `manifest_override` from the flag. `build`/`run` build it
once; tests construct it explicitly, pointing `user_manifest` into a fixture sandbox.

---

## Implementation

### `src/main.rs`

Parse `--manifest <path>` out of the `build`/`run` argument vectors before dispatch. The
flag may appear anywhere after the subcommand (accept both `build a.sth --manifest m.pkg`
and `build --manifest m.pkg a.sth`); the first non-flag argument is the entry file. A
`--manifest` with no following path, or a second `--manifest`, is a usage error through
the existing `usage()` path. `repl` takes no `--manifest` (Out of scope). Thread the
parsed `Option<PathBuf>` into `driver::build_with_manifest`/`driver::run_with_manifest`
(the new manifest-carrying variants, see below).

Extend `usage()` to document `--manifest <path>`.

### `src/driver.rs`

- Keep `pub fn build(path)`, `run(path)`, `emit_ssa(path)` (`src/driver.rs:477`, `503`,
  `464`) with their **existing** signatures, unchanged, as thin wrappers. Each delegates
  to a new manifest-carrying variant — `build_with_manifest(path, manifest: Option<&Path>)`,
  `run_with_manifest(...)`, `emit_ssa_with_manifest(...)` — forwarding
  `ResolutionConfig::from_env()` (i.e. `manifest: None`). This keeps the ~74 existing
  `driver::build`/`run`/`emit_ssa` call sites in `tests/*.rs` and `src/main.rs` untouched,
  the same wrapper discipline used for `discover_closure` below. `main.rs` calls the
  `*_with_manifest` variant directly, passing the parsed flag. The three new variants chain
  the same way the existing three do (`run` → `build` → `emit_ssa` →
  `discover_closure`, `src/driver.rs:503,494,465`): `run_with_manifest` calls
  `build_with_manifest`, which calls `emit_ssa_with_manifest`, which calls
  `discover_closure_configured` directly — never dropping down to the `None`-only wrappers,
  or the flag would be lost before it reaches resolution.
- `discover_closure_configured(entry: &Path, config: &ResolutionConfig, manifests: &mut
  ManifestCache) -> Result<Closure, String>` is a **3-argument** rename of today's
  `discover_closure_with(entry, manifests)` (`src/driver.rs:82`): same body, same
  caller-supplied `ManifestCache`, plus the new `config` parameter. It does **not** itself
  run `check_package_graph` (matching `discover_closure_with` today).
- Keep `pub(crate) fn discover_closure(entry)` as a thin wrapper: it still creates its own
  `ManifestCache`, calls `discover_closure_configured(entry, &ResolutionConfig::from_env(),
  &mut manifests)`, then runs `packages::check_package_graph(&mut manifests,
  &closure.unresolved_imports)?` — exactly what `discover_closure` does today
  (`src/driver.rs:73-78`), just with the rename inserted in the middle. This keeps S1a's
  ~20 existing `discover_closure(&entry)` test call sites and `repl.rs`'s two untouched, and
  keeps the package-graph audit intact (dropping the cache argument or the
  `check_package_graph` call, as an earlier reading of this section could be misread to
  imply, is not what happens).
- The one existing direct caller of `discover_closure_with`,
  `discover_closure_manifest_cache_reads_once` (`src/driver.rs:1044-1055`, which asserts
  `manifests.parses == 1` against a caller-supplied cache), is renamed to call
  `discover_closure_configured(entry, &ResolutionConfig::from_env(), &mut manifests)` with
  no behavioural change — it is the rename's only non-`discover_closure` consumer.
- **Phase 2** creates `discover_closure_configured` exactly as specified above (the 3-arg
  rename), but still calling `manifests.package_of(&canon)?` in its per-file loop (tier-2
  only, behaviourally identical to today) so phase 2 compiles and stays green on its own,
  including the renamed `discover_closure_manifest_cache_reads_once`. **Phase 3** then
  replaces that one `package_of` line with
  `packages::select_site(&canon, &entry_canon, config, manifests)?`, which applies R3
  (entry-only override) and R4 (tier selection).
- `ResolutionConfig` and `ResolutionConfig::from_env()` live in `driver.rs` (they own
  process/env concerns, which `packages.rs` must not import — CLAUDE.md growth structure);
  `select_site` lives in `packages.rs` (pure resolution).

### `src/packages.rs`

- Add `SiteOrigin { Ancestor, Flag, UserLevel }` and a field on `PackageSite`; `new`
  gains the origin (or a defaulted `new`/`new_with_origin` pair to avoid churning S1a's
  call sites). `package_of` yields `Ancestor`; the dependency site minted in
  `resolve_dependency_module` (`src/packages.rs:577`) is also `Ancestor` — it is a real
  package.
- `select_site(file, entry, config, manifests) -> Result<Option<PackageSite>, String>`:
  1. If `file == entry` and `config.manifest_override` is `Some(p)`: load `p` through the
     cache (`manifests.load(p)`, which parses via `parse_manifest`), return a `Flag` site
     rooted at `p`'s dir. Loading (not a standalone `parse_manifest`) is what seeds the
     flag manifest into `known_manifest_paths()`, so `check_package_graph` audits its layer
     and `depends:` name-mismatch exactly as for an ancestor (F1).
  2. Else `package_of(file)?` → `Some` returns the `Ancestor` site.
  3. Else if `config.user_manifest` is `Some(p)` and it exists: parse via
     `parse_user_manifest`, return a `UserLevel` site rooted at `p`'s dir (synthesizing a
     `Manifest` with an empty package name and `depends` from the file; it is never seeded
     into `check_package_graph`).
  4. Else `Ok(None)` (tier 4).
- `resolve_import`: the `intrinsics` short-circuit and the `ImportTarget::Path` arm are
  unchanged. Replace the `let Some(site) = site else { … }` branch and the `MissingDepends`
  handling so origin drives the diagnostic:
  - **tier 4** (`None`): a module name → R2(c) `anonymous_package_error`.
  - **`SelfPackage` on a `UserLevel` site** → R2(c) form as well (a `self::` name has no
    package identity under the user-level manifest; render `<pkg-or-self>` as the target).
  - **`SelfPackage` on a `Flag` site** → a located error
    (`self_import_under_flag_manifest_error`): `self::` names a module of the file's *own*
    package, an identity `--manifest` does not supply (F2, R3/R5). The message says a
    `self::` import needs an ancestor `sooth.pkg`, not a flag manifest, and suggests either
    adding an ancestor `sooth.pkg` or rewriting the import as dependency-anchored. Placed
    ahead of the `resolve_self_module` dispatch so it never reaches the owner guard.
  - **`Dependency` on a `UserLevel` site** → resolve as today, but a missing `depends:`
    entry raises R2(b) `user_manifest_missing_depends_error` inline instead of recording
    `MissingDepends`.
  - **`SelfPackage`/`Dependency` on an `Ancestor` site, `Dependency` on a `Flag` site**
    → unchanged S1a behaviour (record-and-defer).
- Delete `module_import_without_manifest_error`; add `anonymous_package_error`,
  `user_manifest_missing_depends_error`, and `self_import_under_flag_manifest_error`.

### `src/manifest.rs`

Add `parse_user_manifest(src, path) -> Result<Vec<DependsEntry>, String>` over the shared
`depends:` grammar, rejecting `package:`/`layer:`/`module:` with a located "not allowed in
a user-level manifest" message and keeping the `depends: intrinsics` and `check_pkg_name`
rejections. Factor the `depends:` line body out of `parse_manifest`'s match arm so the two
entry points share it rather than duplicating the `path`/name/semicolon sequence.

### `src/repl.rs`

Untouched (see Out of scope). S1a's `repl_module_name_import_is_rejected` (`repl.rs:4625`)
and the rejection at `repl.rs:1761-1767` stand.

---

## Tests

Unit (`packages.rs`), each pinning the exact located message with line/col and the named
path, and each mutation-tested by deleting the branch it guards:

- `select_site_flag_overrides_entry_ancestor` — an entry file *with* an ancestor manifest
  plus `--manifest` resolves against the flag's manifest, silently (no conflict message).
  Mutate: drop the `file == entry && override` check → falls back to the ancestor.
- `select_site_flag_ignored_for_non_entry_file` — a two-file closure where the imported
  file has its own ancestor manifest resolves that file against its own package, not the
  flag (R3). Mutate: apply the override to every file → the imported file mis-resolves.
- `select_site_user_manifest_resolves_dependency` — a manifest-less entry with a
  `user_manifest` listing `core` resolves `import: core::bool`. Mutate: make `select_site`'s
  tier-3 branch return `Ok(None)` regardless of the user manifest's contents → the import no
  longer resolves and the test fails.
- `user_manifest_missing_depends_is_error` — same, but the user-level manifest lacks
  `core`; pins R2(b) including the user-manifest path. Mutate: delete the inline raise.
- `anonymous_package_module_import_is_error` — manifest-less entry, no user-level
  manifest, `import: core::bool`; pins R2(c) naming the anonymous status. Mutate: return
  `Ok(None)` (would leave the import silently unbound).
- `anonymous_package_self_import_is_error` — the `self::` form of the above lands R2(c).
- `self_import_under_flag_manifest_is_error` — an entry file resolved via `--manifest` (a
  `Flag` site) with a `self::mod` import lands `self_import_under_flag_manifest_error`,
  pinned with line/col (F2). Mutate: delete the `Flag`+`SelfPackage` check so it falls
  through to `resolve_self_module` → the `self::` wrongly resolves against the flag root or
  trips the owner guard, which the test catches.
- `anonymous_package_quoted_path_and_intrinsics_still_resolve` — a regression fence: tier
  4 still resolves a quoted-path sibling and `import: intrinsics * ;` (no killed mutant;
  guards the unchanged arms).
- `flag_manifest_missing_depends_reuses_ancestor_diagnostic` — a `--manifest` whose
  manifest lacks the imported package produces S1a's A wording with the flag's path (R2a).

`manifest.rs`: `parse_user_manifest_depends_only_ok`,
`parse_user_manifest_rejects_package_line`, `_rejects_layer_line`,
`_rejects_module_line`, `_rejects_depends_intrinsics`, `_empty_is_ok` (no depends).

`main.rs` (or a thin `driver`-level arg test): `manifest_flag_parsed_before_entry`,
`manifest_flag_after_entry`, `manifest_flag_missing_path_is_usage_error`,
`duplicate_manifest_flag_is_usage_error`.

`driver.rs`: `discover_closure_configured_flag_override_entry_only`,
`discover_closure_configured_user_manifest_fallback`,
`discover_closure_configured_anonymous_fallback` — each via a fixture tree on disk through
`discover_closure_configured` with an explicit `ResolutionConfig` (never mutating process
env), so tier 3 is exercised without an XDG env race.

S1a test churn (behaviour changed, not placebos):

- `driver::module_import_outside_a_package_is_error` and the packages-level test pinning
  `module_import_without_manifest_error` are **rewritten**: a manifest-less module import
  is no longer an immediate error; with no user-level manifest it is now R2(c) (tier 4),
  and with one it resolves or is R2(b). Keep them as located-error tests against the new
  messages.

---

## Golden tests (`tests/phase8_slice1b.rs`)

Through `driver::build_with_manifest`/`emit_ssa_with_manifest` (goldens 1-2, which carry a
`--manifest`) or `driver::build`/`emit_ssa` (goldens 3-5, which don't) on a fixture tree,
never `select_site`/`resolve_import` directly (S1a's lesson: a direct-call test leaves the
CLI wiring unguarded). Every error golden pins the exact message substring with line/col;
`is_err()` alone is a placebo.

1. `flag_resolves_entry_outside_its_package_tree` — an entry `.sth` in directory A, a
   package manifest in directory B whose `depends:` grants the import; `build entry.sth
   --manifest B/sooth.pkg` builds, runs, exits clean. This is the S2 fixture pattern.
2. `flag_overrides_ancestor_manifest_silently` — the entry file sits inside package P
   (ancestor manifest present) but `--manifest Q/sooth.pkg` is given; resolves against Q,
   with no conflict diagnostic emitted.
3. `user_level_manifest_resolves_scratch_file` — a manifest-less entry, `ResolutionConfig`
   pointing `user_manifest` at a fixture `global_sooth.pkg` that lists the dependency;
   builds. (Driven through `discover_closure_configured`, not a real XDG path.)
4. `user_level_manifest_missing_depends_names_its_remedy` — same, user-level manifest
   lacks the package; pins R2(b) and its user-manifest path.
5. `anonymous_package_names_itself` — manifest-less entry, no user-level manifest, a
   dependency module import; pins R2(c) (the anonymous status and the three-way remedy).

---

## Exit criteria

- `sooth build entry.sth --manifest path/to/sooth.pkg` resolves the entry file's
  **dependency-anchored** imports against the named manifest regardless of `entry.sth`'s
  own directory, unconditionally overriding an ancestor manifest, and overriding it for the
  entry file only (R3). A `self::` import under `--manifest` is a located error, not a
  resolution (F2).
- A manifest-less, flag-less file resolves against the user-level manifest, then falls back
  to an implicit anonymous package with no dependencies.
- Each fallback tier's failure names its own remedy (R2 a/b/c), and the anonymous case
  names itself.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green; all five goldens
  pass; the changed S1a tests pass against the new messages.

---

## Growth structure (CLAUDE.md)

`select_site` is resolution, so it lives in `packages.rs` beside `package_of`;
`ResolutionConfig`/`from_env` own the flag and the XDG lookup, which are process/env
concerns, so they live in `driver.rs` (which already imports `std::env`/`Command`).
`packages.rs` must not gain an env dependency — that is the import-divergence signal S1a's
split was drawn to avoid. No new file: this slice adds one function to each of three
existing modules and a parser entry point, well under a split threshold.

---

## Out of scope

- **`self::` imports under `--manifest`.** The flag override is dependency-anchored only;
  a `self::` import from a flag-overridden entry file is a located error (R3/R5). `self::`
  requires a real ancestor-package identity, which the S2 fixture use case (dependency-only)
  does not need.
- **The REPL's resolution path.** The design notes tier 3 is "the same file the REPL reads
  for a session," but this slice's sequencing and exit are build/run only; the REPL keeps
  S1a's located module-name rejection (`repl.rs:1761`). Wiring the REPL to the user-level
  manifest reuses this slice's `select_site`/`parse_user_manifest` with no new mechanism,
  and is deferred to when the REPL slice needs it.
- Manifest grammar, package attribution, module naming/visibility, cross-package
  resolution, the layer check (S1a, done).
- Single-mode imports, wildcard visibility, intrinsics gating, re-export, prelude deletion
  (S2).
- Git dependency paths, semver, API descriptions (later).

---

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "manifest.rs: parse_user_manifest (depends-only, rejects package:/layer:/module:), factor the shared depends: line body, unit tests",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "main.rs --manifest flag parsing (before/after entry, usage errors) threaded via new driver::build_with_manifest/run_with_manifest/emit_ssa_with_manifest variants (existing build/run/emit_ssa kept as thin None-forwarding wrappers, ~74 call sites untouched); ResolutionConfig + from_env in driver.rs; discover_closure_configured created as a rename of discover_closure_with still calling package_of (tier-2 only, green on its own); arg tests",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "packages.rs: SiteOrigin on PackageSite, select_site (entry-only flag override loading the flag manifest through the cache, tier 2/3/4 selection), replace the package_of line in discover_closure_configured with select_site; unit tests including the entry-only mutation guard",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "packages.rs: per-tier diagnostics (delete module_import_without_manifest_error, add anonymous_package_error, user_manifest_missing_depends_error, and self_import_under_flag_manifest_error, tier-1 reuse of A/C/D1), rewrite the changed S1a tests, unit tests pinning each message",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 5,
      "focus": "golden tests (flag resolves outside tree, flag overrides ancestor silently, user-level resolves, user-level missing depends, anonymous names itself) and exit-criteria sweep",
      "effort": "M",
      "difficulty": "standard"
    }
  ]
}
```
