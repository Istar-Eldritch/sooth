## Problem

`import: intrinsics | drop | ;` flipped a permission bit (`IntrinsicVisibility`,
`src/ast.rs:172`) on the importing file only. `widen_intrinsics` (`src/driver.rs:267`) folded
one file's own lines into one file's own bit, and nothing downstream was import-aware, so a hub
that gated in an intrinsic and re-exported it hit two gaps:

1. **`export:` could not name a gated-in intrinsic** — `export_unknown_name_error`.
2. **The caller-side gate read one field and never walked an import** —
   `intrinsic_is_gated_out` (`src/check/word_families.rs:1068`) decided on
   `!m[span.module].intrinsics.admits(name)`, the *calling* module's own bit, so a consumer
   reaching `drop` only through a hub was rejected.

Ordinary word re-export through a hub already worked; the type path was fixed in `5338c06`.
Only the intrinsic path was unwired.

## Design

One **effective** `IntrinsicVisibility` per module, computed in `driver::assemble_module` and
stored in the existing `ModuleInfo.intrinsics` field. Every consumer of that field keeps
working with no signature change, and both gaps read the same table.

Rejected: teaching `intrinsic_is_gated_out` to walk imports (threads the selective map into a
`&Ctx`-only helper at two hot dispatch sites, re-derives a per-module constant per call, and
leaves gap 1 with a second walk). Also rejected: widening `exportable_names`
(`src/resolve.rs:570`) — it returns a `HashSet<&str>`, but the gate set is not enumerable
(`is_name_dispatched_builtin` is `BUILTIN_WORDS` membership *plus* every non-empty
`>`-prefixed conversion, an open set), so the check has to be a predicate at the existence
test. `exportable_names` was untouched.

### R1 — Effective visibility

```text
effective[m] = own[m] union { n : (n -> h) in selective[m]
                                , is_name_dispatched_builtin(n)
                                , effective[h].admits(n) }
```

`own[m]` is the `widen_intrinsics` fold; `selective[m]` is `selective_maps[m]` (explicit
`| n |` clause and wildcard desugar). Union is per name, never per bit —
`IntrinsicVisibility::admitting` (`src/ast.rs:198`) widens `Only` by one name and leaves `All`
alone, so `All` is reachable only by writing `import: intrinsics * ;` and no other way.
Idempotent by construction (`Only` holds a `HashSet`), so a module that writes its own
`import: intrinsics | drop | ;` *and* imports a hub admitting `drop` sees `Only({drop})`.

### R2 — Bare-name routes only, never qualified

Only `selective_maps` is walked, so `import: "./hub.sth" hub ;` contributes nothing and
`hub::drop` is not a route: `rewrite`'s qualified branch misses `self.words[target]`, and
`vis.origin` returns `None` because R4 records the hub as its own origin, so the name falls to
the ordinary unknown-word path.

### R3 — A wildcard `intrinsics` import does not leak through a hub

A hub's contribution is enumerated from *its `export:` list*, never from `effective[h]`, so a
hub writing `import: intrinsics * ; export: drop ;` contributes exactly `drop`. No code path
carries `IntrinsicVisibility::All` across a hub.

### R4 — `export:` accepts an effectively-admitted intrinsic, origin = self

In `resolve_export_origins` (`src/resolve.rs:637`), a second self-source branch after
`declared[m].contains(name)`:

```text
is_name_dispatched_builtin(name) && intrinsics[m].admits(name)   =>  source = m
```

`intrinsics: &[IntrinsicVisibility]` is a new parameter on `resolve_export_origins` and
`build_exported_origin`, read off `module.modules[m].intrinsics` at the single call site.
Origin = `m`, so `Visibility::origin` returns `None` and no call site is mangled against a hub
— required, since an intrinsic must stay bare for builtin dispatch. Consequences: a hub with
no `import: intrinsics` line still fails byte-identically; a hub-of-hubs chain resolves because
`intrinsics[m]` is the *effective* value; the six surface comparisons (`eq`/`lt`/…) are
excluded from both the accept set and the gate set, being `core::cmp` words.

### R5 — An intrinsic-name selective entry is exempt from the two collision rules

`check_selective_imports` (`src/check/declarations.rs:705`) skips both the local-decl and
the duplicate-selective rejection for an entry satisfying

```text
is_name_dispatched_builtin(name)
  && module.modules[target].intrinsics.admits(name)
  && !local_decl_names(module, target).contains(name)
```

evaluated *after* the not-exported check, so a hub that does not export the name still fails
there first. The exemption is coherence, not convenience: such an entry binds no word and
carries no module identity, so neither rule has a subject. Without it,
`import: core::prelude * ;` would break in any program with a destructor (a local
`: drop ( Fd -- )` and the intrinsic already coexist, `resolve::mangle` exempting `drop`), and
a diamond of two hubs both admitting `drop` would report an ambiguity with no two answers.

The `local_decl_names` clause is the review-added half: a source that *declares* the name
exports that declaration, so the entry binds a word again and both rules get their subject
back. Without it the exemption swallows a real ambiguity (an importer's own `dup` silently
shadowing an imported `dup`). Ordinary words keep colliding, `selective_collision_error`
byte-identical.

### R6 — REPL out of scope, benignly

`intrinsic_is_gated_out` never fires on the REPL path (`ctx.modules()` is `None`), and the
REPL's `import:` loops `continue` past an export name with no matching word, so an exported
intrinsic is silently skipped — harmless, since the REPL admits the name unconditionally. No
REPL change.

### R7 — One phase, because the two gaps are one table

Neither gap is independently observable: gap 1 alone accepts `export: drop ;` while every
consumer is still rejected; gap 2 alone is unreachable because no hub can export the name.
Splitting would ship a placebo exit criterion.

### R8 — Computation order and cycles

`closure.nodes` is discovery order, not topological. `effective_intrinsics_of`
(`src/driver.rs:324`) is a memoized DFS with a per-walk `visiting` set, mirroring
`walk_type_export_origin`, treating a revisit as "contribute nothing" rather than panicking —
stated, not assumed, though `closure.reject_cycles()` runs first.

## Test hazard

`common::fixture_source` appends `import: intrinsics * ;` to any `.sth` whose text lacks
`import: intrinsics`, and *not having that line* is this slice's whole subject. So
`tests/phase7_slice3q.rs` writes every fixture verbatim with `std::fs::write`, declares no
`mod common;` at all, and carries a copy of `phase8_slice2.rs`'s `Tree` helper with the
`fixture_source` call removed (the one prelude golden writes its own `sooth.pkg`). Every
negative golden pins the exact diagnostic, never `is_err()`.

## Tests

`tests/phase7_slice3q.rs`, all through the real binary:

- `hub_re_exporting_a_gated_intrinsic_is_callable_bare` — the headline.
- `a_hub_of_hubs_carries_the_intrinsic` (R4, depth two).
- `a_wildcard_intrinsics_import_does_not_leak_through_a_hub` (R3 mutation witness).
- `a_hub_without_an_intrinsics_import_cannot_export_one` (R4, pinned).
- `a_qualified_hub_import_is_not_a_route` (R2).
- `an_own_intrinsics_import_and_a_hub_admitting_the_same_name_agree` (R1 idempotency).
- `a_local_destructor_coexists_with_a_wildcard_hub_import`,
  `two_hubs_admitting_one_intrinsic_are_a_union_not_a_collision` (R5).
- `two_hubs_re_exporting_one_ordinary_word_still_collide` (R5 blast radius, pinned).
- `a_conversion_intrinsic_re_exports_through_a_hub` (`>i64`, the open-set member).
- `a_consumer_of_core_prelude_calls_drop_bare` (phase 2).

Review-added goldens beyond the spec's list:

- `a_hub_admitted_intrinsic_and_another_modules_real_word_share_a_name` — both entries run:
  the `Fd` goes to the imported destructor, the bare `1` to the hub-admitted intrinsic.
- `a_source_declaring_an_intrinsic_name_still_collides_with_a_local` — R5's other side.
- `an_operator_name_dispatches_the_same_through_a_hub_as_it_does_alone` — for the operator
  names `rewrite` leaves a bare call unmangled and `check_operator` dispatches on operand
  types, so a same-named word loses to the builtin *in its own module* with no import in
  sight; the hub only supplies the admission that lets the call reach dispatch.
- `an_overload_on_a_user_type_crosses_a_hub_and_dispatches` — what that crediting buys: an
  exported `add ( Vec2 Vec2 -- Vec2 )` is callable by the consumer (pre-slice,
  `ungated_intrinsic_error`).

Unit: the R1 fold in `src/driver.rs` (own-only, hub-only, union, wildcard-narrowing, depth-two
chain, and an R8 fabricated back edge asserting termination); `resolve_export_origins` with an
`intrinsics` slice (accept with `origin == m`, reject when unadmitted, `Visibility::origin`
returning `None`); the R5 exemption both ways on one fixture pair, plus the declaring-hub case.

R3, the R5 exemption and the R5 blast-radius guard were mutation-tested before phase exit.

## Phases

1. **Effective-visibility table and both gaps** (M, hard) — `src/driver.rs` (R1, R8),
   `src/ast.rs` (`admitting`), `src/resolve.rs` (R4), `src/check/declarations.rs` (R5),
   `tests/phase7_slice3q.rs`. `src/check/word_families.rs` stayed unchanged: the gate is
   untouched, and that is the design's point.
2. **`core::prelude` re-exports `drop`** (S, standard) — `lib/prelude.sth` gains
   `import: intrinsics | drop | ;` and `drop` on the `export:` list; the header comment's
   "`drop` cannot, at all" paragraph removed while the `.`-overload paragraph stays true
   (P7.S3i/S3e-follow territory). Zero corpus fallout, as predicted.

## Out of scope

- The `.` operator-overload hub gap: an overload's candidate lookup is one hop by design.
- Any qualified spelling for an intrinsic (R2); REPL behaviour (R6).
- Lowering, IR, monomorphization — check-time visibility only.
- `exportable_names` and the ordinary word/type re-export path, which already worked.
