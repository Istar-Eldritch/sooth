# P8.S1b spec — the `--manifest` CLI flag and the fallback chain

**Slice:** P8.S1b (delivered) · **Design authority:** `docs/roadmap/P8-packages-modules.md`
(section "A manifest is optional, and resolution falls back three ways")
· **Brief:** `docs/roadmap/P8/slice1b-brief.md` · **Builds on:** P8.S1a
**Siblings (out of scope):** S1 (manifest grammar, package attribution, module
naming/visibility, cross-package resolution, layer check — all delivered by S1a),
S2 (single-mode imports, intrinsics gating, wildcard semantics, re-export, prelude deletion).

---

## Scope

The CLI-level question of *which manifest resolves a given invocation*. Three things, no new
file and no new checker pass:

1. A `--manifest <path>` flag on `sooth build`/`run`, ranked above discovery.
2. The two fallback tiers S1a did not build: the user-level manifest at
   `$XDG_CONFIG_HOME/sooth/global_sooth.pkg` (tier 3), and the diagnostic-bearing form of the
   implicit-anonymous-package baseline (tier 4).
3. A distinct located diagnostic per tier (dogfood finding F9).

Not this slice: everything S1a shipped; S2's single-mode imports and prelude deletion; the
REPL's own resolution path.

---

## Rulings

### R1 — Fallback order

Ranked, first that applies wins:

1. **`--manifest <path>`** on the invocation (tier 1).
2. The **nearest ancestor manifest** (tier 2, S1a's `package_of`).
3. The **user-level manifest** at `$XDG_CONFIG_HOME/sooth/global_sooth.pkg` (tier 3).
4. An **implicit anonymous package** with no dependencies (tier 4): `intrinsics` and
   quoted-path siblings resolve; a `self::` or dependency *module name* is a located error.

`--manifest` unconditionally and silently overrides an ancestor manifest, and is exempt from
the rule barring the user-level fallback inside a package: a named act at the call site beats
a discovered one, and a flag pinned in CI is as reproducible as pinning the entry file.

### R2 — Per-tier diagnostic wording

Three located diagnostics, all sharing S1a's `import_header`
(`` error: import `<target>` at line L, col C in <importer>: ``).

**(a) A named `--manifest` that does not resolve an import.** No new category: S1a's
diagnostics verbatim, with the flag's manifest path substituted for the ancestor manifest
path. Missing `depends:` entry → `missing_depends_error` (A); non-public module →
`private_module_error` (C); nothing at the joined path → D1 `module_not_found_error`. A tier-1
site is a real package (R4) loaded into the `ManifestCache`, so these keep flowing through
`check_package_graph` exactly as an ancestor site's do, layer check and depends-name-mismatch
audit included (F1).

**(b) Tier 3, user-level manifest present but lacking the package**
(`user_manifest_missing_depends_error`):
