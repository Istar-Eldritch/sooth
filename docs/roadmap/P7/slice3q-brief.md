# P7.S3q brief — An intrinsic gated into a module cannot be re-exported through a hub

## Problem, confirmed live against current `main` (`e52fc8a`)

`import: intrinsics | drop | ;` flips a permission bit (`IntrinsicVisibility`,
`src/ast.rs:173-190`) on the importing module; it is not a real `Module` import.
`driver.rs::names_the_intrinsics` (`src/driver.rs:250`) recognizes the `import: intrinsics`
form and routes it to `widen_intrinsics` (`src/driver.rs:267`), which only ever mutates that
one module's own `IntrinsicVisibility` field — it never touches the `declared`/`selectives`/
`import_maps` tables `resolve.rs::resolve_export_origins` (`src/resolve.rs:590`) and
`exportable_names` (`src/resolve.rs:570`) use to decide what an `export:` list may name. So a
hub that gates in an intrinsic and tries to re-export it hits a wall two gaps deep, both
probe-confirmed live:

**Gap 1 — `export:` cannot name a gated-in intrinsic at all.**

```sooth
\ hub.sth
import: intrinsics | drop | ;
export: drop ;
```

```sooth
\ main.sth
import: intrinsics * ;
import: "./hub.sth" hub | drop | ;
: main ( -- ) 1 drop ;
```

```
$ sooth build main.sth
error: error: `drop` in `export:` names nothing declared or imported in this module (line 2, col 9)
```

`exportable_names` walks `module.words`/`.externs`/`.structs`/`.enums`/`.traits` for names
`module == m` declares (`src/resolve.rs:570-593`); an intrinsic is none of these, it is a
compiler-provided `BUILTIN_WORDS` entry gated by a bit on `ModuleInfo`, so it is invisible to
this scan regardless of what `hub`'s own `intrinsics` field admits. The error is
`export_unknown_name_error` (`src/resolve.rs:525`, raised at `:657`).

**Gap 2 — even with the name accepted, the caller-side gate does not walk the import.**
Confirmed by temporarily forcing `exportable_names` to admit `drop` (reverted; not a proposed
fix, just isolating gap 2): `export: drop ;` then resolves and `hub` re-exports `drop` at the
name level, but a caller that reaches `drop` *only* through the hub — writing no
`import: intrinsics` line of its own — is still rejected:

```sooth
\ main.sth (no import: intrinsics line)
import: "./hub.sth" hub | drop | ;
: main ( -- ) 1 drop ;
```

```
error: error: `drop` is an intrinsic and is not imported in `main` (line 2, col 17)
  add `import: intrinsics * ;` (or `import: intrinsics | drop ... | ;`) to this file
```

`intrinsic_is_gated_out` (`src/check/word_families.rs:1068-1083`) is the sole gate, consulted
from `check/terms.rs:276` and `check/poly.rs:1141`. It reads exactly one field:
`ctx.modules()[span.module as usize].intrinsics.admits(name)` — the *calling* module's own
gate, nothing upstream. It has no notion of "reached via a one-hop selective import of a
module that itself admits this name."

**This is specific to the intrinsic-gate mechanism, not a general hub/`export:` bug.**
Probe-confirmed: a hub that imports and re-exports `Bool`/`True` (an ordinary type, imported
the normal `Module`-import way, not through the `intrinsics` pseudo-import) works with no
error at all — `export:`, the selective-import chain, and the call site all resolve clean.
The type/word path already has the machinery this needs (see "What already exists" below);
the intrinsic path was simply never wired into it, because `import: intrinsics` predates
`export:`'s re-export mechanism and was never revisited when P8.S2 added it.

## What already exists to build on

`resolve.rs`'s `Visibility::origin` (`src/resolve.rs:498-507`) is the working pattern for "a
name X does not declare but re-exports": `exported_origin: Vec<HashMap<String, u32>>` maps,
per module, each of its `export:`-listed names to the module id that actually declares it.
`rewrite`'s qualified- and selective-import branches (`src/resolve.rs:484-616`) both consult
`vis.origin(target, name)` as a fallback after checking the target's own declarations, and
that is exactly the shape a fixed `exportable_names`/`resolve_export_origins` pair would need
for intrinsics too — except the "declares" predicate for an intrinsic is `IntrinsicVisibility
::admits`, not membership in `module.words`/etc, so it cannot reuse `exportable_names` as
written; it needs its own admits-based check threaded through the same origin-resolution
walk.

The caller-side gate (`intrinsic_is_gated_out`) is structurally different from `rewrite`'s
qualifier-walk: `rewrite` has the full `imports`/`selective` maps and `Visibility` table in
scope at the call site, because rewriting a qualified/selective name *is* an import-chain
walk. `intrinsic_is_gated_out` today takes only `ctx: &Ctx` and a bare `name`, checked at the
term-walk site after any qualifier has already been stripped and the call is being treated as
bare/intrinsic — it has no access to *why* the name resolved to an intrinsic call, i.e.
whether it arrived via `main`'s own `import: intrinsics` or via a selective import of a hub.
Wiring gap 2 shut needs either (a) giving `intrinsic_is_gated_out` the same one-hop
selective-import table `rewrite` already has and having it walk to the origin module's own
`intrinsics` field, checked at `resolve`/`check` time before the term ever reaches the
builtin-dispatch gate, or (b) resolving intrinsic re-export at `resolve_export_origins` time
(gap 1's fix) and propagating an explicit "this module's own effective `IntrinsicVisibility`
includes names admitted transitively via `export:`" value onto `ModuleInfo` itself, so
`intrinsic_is_gated_out`'s existing single-field check keeps working unchanged. (b) needs a
ruling on order-of-computation (a module's `intrinsics` field is built during import
assembly; the transitive-admit set needs the exporting module's `intrinsics` field to already
be final, which is fine for an acyclic import DAG but should be stated, not assumed) and
whether it double-counts if a module both writes `import: intrinsics | drop | ;` itself *and*
imports a hub that also admits `drop` (should be idempotent — the union, not a duplicate-entry
error).

## Not yet recon'd / open for the spec phase

- Whether (a) or (b) above is the right split, and which files own the new state (`ast.rs`'s
  `ModuleInfo`, `resolve.rs`'s `Visibility`, or a new small table threaded to `check::terms`).
- Whether `import: intrinsics * ;` (wildcard, admits `IntrinsicVisibility::All`) re-exported
  through a hub should re-export *all* intrinsics or require the hub to still narrow its own
  `export:` list — probably the latter (an `export:` list already names what a hub promises;
  nothing should let a wildcard silently leak the compiler's entire intrinsic surface through
  one hub import), but this needs a decision and a golden, not an assumption.
- Whether a diamond (two hubs both re-exporting `drop` from the same or different underlying
  admits) needs any special handling, or whether `Visibility::origin`'s existing one-owner-
  per-name resolution already covers it for free.

## Scope guess for the spec phase (not binding)

Likely two phases: (1) `export:`/`exportable_names`/`resolve_export_origins` learn to accept
and resolve a gated-in intrinsic name (gap 1), sequenced first since gap 2's fix needs gap 1's
resolved origin to exist; (2) the caller-side gate at `intrinsic_is_gated_out` walks that
resolution (gap 2). No lowering/IR involvement expected — this is check-time visibility only,
the same layer S3e's whole-program trait registry and P8.S2's `export:` origin resolution
already live in.
