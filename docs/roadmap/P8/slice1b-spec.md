# P8.S1b spec — the `--manifest` CLI flag and the fallback chain

**Slice:** P8.S1b (delivered) · **Design authority:** `docs/roadmap/P8-packages-modules.md`
(section "A manifest is optional, and resolution falls back three ways")
· **Brief:** `docs/roadmap/P8/slice1b-brief.md` · **Builds on:** P8.S1a
**Siblings (out of scope):** S1 (manifest grammar, package attribution, module
naming/visibility, cross-package resolution, layer check — all delivered by S1a),
S2 (single-mode imports, intrinsics gating, wildcard semantics, re-export, prelude deletion).

---

## What shipped and why

S1a resolved a module import against its file's *nearest ancestor manifest* but left two of
the design's four resolution tiers unbuilt and had no way to name a manifest from the command
line. This slice answers the CLI-level question of *which manifest resolves a given
invocation*, with no new file and no new checker pass — one function added to each of three
existing modules plus a parser entry point:

1. A `--manifest <path>` flag on `sooth build`/`run`, ranked above ancestor discovery.
2. The two missing fallback tiers: the user-level manifest at
   `$XDG_CONFIG_HOME/sooth/global_sooth.pkg` (tier 3), and the diagnostic-bearing implicit
   anonymous-package baseline (tier 4).
3. A distinct located diagnostic per tier (dogfood finding F9).

---

## Rulings

### R1 — Fallback order

Ranked, first that applies wins: (1) `--manifest <path>` on the invocation; (2) the nearest
ancestor manifest (S1a's `package_of`); (3) the user-level manifest at
`$XDG_CONFIG_HOME/sooth/global_sooth.pkg`; (4) an implicit anonymous package with no
dependencies (`intrinsics` and quoted-path siblings resolve; a `self::` or dependency module
name is a located error). `--manifest` unconditionally and silently overrides an ancestor
manifest and is exempt from the rule barring the user-level fallback inside a package.

### R2 — Per-tier diagnostic wording

Three located diagnostics, all sharing S1a's `import_header`. **(a)** A `--manifest` that
fails to resolve an import reuses S1a's diagnostics verbatim (`missing_depends_error` /
`private_module_error` / `module_not_found_error`), with the flag's manifest path substituted
for the ancestor path — a tier-1 site is a real package, so it flows through
`check_package_graph` exactly as an ancestor site does. **(b)** A user-level manifest present
but lacking the imported package raises `user_manifest_missing_depends_error` inline, naming
the user-level manifest path and its remedy. **(c)** No user-level manifest raises
`anonymous_package_error`, naming the anonymous status and the three-way remedy (write
`global_sooth.pkg`, add an ancestor `sooth.pkg`, or pass `--manifest`); a bare `self::` in a
manifest-less file lands here too. This retired `module_import_without_manifest_error`, whose
single message predated the user-level tier.

### R3 — Closure scope of the override

`--manifest` overrides resolution for the **entry file only**, not the whole discovered
closure. Every transitively-imported file re-derives its own package via `package_of`; a
nested package's own ancestor manifest still wins for its own files. The override is
**dependency-anchored only** — a `self::` import from a flag-overridden entry file is a
located error (`self_import_under_flag_manifest_error`), since `self::` names a module of the
file's own package, an identity a flag manifest does not supply.

### R4 — Site model

Site selection stays a `resolve_import` input; the net-new work is choosing it. A
`SiteOrigin { Ancestor, Flag, UserLevel }` distinguishes three origins (tier 4 is the absence
of a site). Tiers 1 (`Flag`) and 2 (`Ancestor`) are *real packages*: the manifest is loaded
**through the `ManifestCache`** (not parsed standalone), so it is seeded into
`check_package_graph`'s layer and depends-name-mismatch audit (F1). Tier 3 (`UserLevel`) is a
`depends:`-only site never seeded into that audit — it has no layer to violate — but the
dependencies it resolves load their own real manifests, so layer relations between them are
still checked. `Ancestor` and `Flag` behave identically in resolution and audit; the
distinction exists only for future diagnostics.

### R5 — `--manifest` and user-level manifest parsing

`--manifest <path>` names a full package manifest loaded through the `ManifestCache` (parsed
by the existing `parse_manifest`); the entry file need not sit under the manifest's directory
(the temp-fixture case). The user-level manifest is `depends:`-only: a new
`parse_user_manifest` reuses the shared `depends:` line grammar (with the `check_pkg_name` and
`depends: intrinsics` rejections) but rejects `package:`/`layer:`/`module:` lines as not
allowed in a user-level manifest. This is the entry point S1a deferred here rather than
routing a `package:`-less file through `parse_manifest`.

### R6 — `$XDG_CONFIG_HOME` resolution and test injectability

The user-level manifest path is `${XDG_CONFIG_HOME}/sooth/global_sooth.pkg`, falling back to
`${HOME}/.config/sooth/global_sooth.pkg` (the XDG default); a missing file is tier 4, not an
error. Resolution reads a `ResolutionConfig { manifest_override, user_manifest }` threaded
from the entry point, never `std::env` deep in `packages.rs`, so parallel tests neither mutate
shared process env nor read a developer's real config. `ResolutionConfig::from_env()` and the
XDG lookup live in `driver.rs` (process/env concerns); `select_site` lives in `packages.rs`
(pure resolution) — the import-divergence line S1a's split was drawn to hold.

---

## Implementation

- **User-level manifest parser (R5):** `parse_user_manifest` and the extracted shared
  `depends:` line body in `src/manifest.rs` — `ed7c461`.
- **Flag parsing and `ResolutionConfig` plumbing (R6):** `--manifest` parsed out of the
  `build`/`run` argument vectors (before or after the entry file; missing-path and duplicate
  flags are usage errors) in `src/main.rs`, with `ResolutionConfig`/`from_env` and the
  `*_with_manifest` driver variants plus `discover_closure_configured` in `src/driver.rs` —
  `ff3da78`, refined by `137843d` and `1fe0321`.
- **Fallback chain (R1/R3/R4):** `SiteOrigin` on `PackageSite` and `select_site` (entry-only
  flag override, tier 2/3/4 selection) in `src/packages.rs`, wired into
  `discover_closure_configured` — `9143e05`, with the config/`select_site` placement in
  `df98ace` and `--manifest` path canonicalization before caching in `c711581`.
- **Per-tier diagnostics (R2):** `anonymous_package_error`,
  `user_manifest_missing_depends_error`, and `self_import_under_flag_manifest_error` added and
  `module_import_without_manifest_error` deleted in `src/packages.rs` — `1b7f1d7`.
- **Golden tests:** `tests/phase8_slice1b.rs` — `0af4d13`, with the `run`-subcommand golden
  and empty-stderr check in `5469abd` and review fixes in `8ae5a87`.

---

## Exit criteria (met)

- `sooth build entry.sth --manifest path/to/sooth.pkg` resolves the entry file's
  dependency-anchored imports against the named manifest regardless of the entry's own
  directory, unconditionally overriding an ancestor manifest, for the entry file only (R3). A
  `self::` import under `--manifest` is a located error.
- A manifest-less, flag-less file resolves against the user-level manifest, then falls back to
  an implicit anonymous package with no dependencies.
- Each fallback tier's failure names its own remedy (R2 a/b/c), and the anonymous case names
  itself.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green; all goldens pass;
  the changed S1a tests pass against the new messages.

---

## Out of scope

- **`self::` imports under `--manifest`** — the override is dependency-anchored only (R3/R5);
  `self::` requires a real ancestor-package identity, which the S2 fixture use case
  (dependency-only) does not need.
- **The REPL's resolution path** — the REPL keeps S1a's located module-name rejection; wiring
  it to the user-level manifest reuses this slice's `select_site`/`parse_user_manifest` with
  no new mechanism, deferred to when the REPL slice needs it.
- Manifest grammar, package attribution, module naming/visibility, cross-package resolution,
  the layer check (S1a, done); single-mode imports, wildcard visibility, intrinsics gating,
  re-export, prelude deletion (S2); git dependency paths, semver, API descriptions (later).
