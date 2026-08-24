# Phase 7 Slice 3q: an intrinsic gated into a module, re-exported through a hub

**Status:** specified, not implemented.
**Discovery:** `docs/roadmap/P7/slice3q-brief.md` (written against `e52fc8a`; this spec
re-verified every citation against `ab41270` and corrects three of them, below).
**Roadmap:** `docs/roadmap/P7-language-prereqs.md:644`.

## Problem

`import: intrinsics | drop | ;` flips a permission bit (`IntrinsicVisibility`,
`src/ast.rs:172-180`) on the importing module; it is not a real `Module` import.
`names_the_intrinsics` (`src/driver.rs:250`) recognises the form and routes it to
`widen_intrinsics` (`src/driver.rs:267`), which only ever folds that one file's own lines
into that one file's own bit (`src/driver.rs:416-425`, pushed at `:492`, landing on
`ModuleInfo.intrinsics` at `:608-613`). Nothing downstream of that field is import-aware,
so a hub that gates in an intrinsic and re-exports it hits a wall two gaps deep.

**Gap 1 — `export:` cannot name a gated-in intrinsic.** Re-probed green against `ab41270`:

```text
\ hub.sth                     \ main.sth
import: intrinsics | drop | ; import: intrinsics * ;
export: drop ;                import: "./hub.sth" hub | drop | ;
                              : main ( -- ) 1 drop ;

$ sooth build main.sth
error: error: `drop` in `export:` names nothing declared or imported in this module (line 2, col 9)
```

**Gap 2 — the caller-side gate reads one field and never walks an import.**
`intrinsic_is_gated_out` (`src/check/word_families.rs:1068`) is the sole gate, consulted from
`src/check/terms.rs:276` and `src/check/poly.rs:1141`. Its whole decision is
`!m[span.module as usize].intrinsics.admits(name)` (`:1080`) — the *calling* module's own bit.
A consumer reaching `drop` only through a hub is rejected by `ungated_intrinsic_error`
(`:1089`) regardless of what the hub admits.

**Not a general hub bug.** Re-probed against `ab41270`: an ordinary word re-exported through
a hub builds clean, and a diamond (two hubs re-exporting one word) is already a located
`selective_collision_error`. The type path was fixed separately in `5338c06`
(`resolve_type_export_origins`, `src/driver.rs:283-357`). Only the intrinsic path is unwired.

## Corrections to the brief (all re-verified against `ab41270`)

- **B1. The fix does not belong in `exportable_names`.** The brief targets
  `exportable_names` (`src/resolve.rs:570`), which returns `HashSet<&str>` — an *enumeration*.
  The gate set is not enumerable: `is_name_dispatched_builtin` (`src/ast.rs:1560`) is
  `BUILTIN_WORDS` membership *plus* every non-empty `>`-prefixed conversion
  (`is_builtin_word_name`, `src/ast.rs:1539`), an open set. Probe: `export: >i64 ;` under
  `import: intrinsics | >i64 | ;` reaches the same unknown-name error, so it parses and is a
  real member of the set. The check therefore has to be a *predicate* applied at the
  existence test inside `resolve_export_origins` (`src/resolve.rs:631-664`), not a widening of
  the name set. `exportable_names` is untouched by this slice.
- **B2. The brief's phase ordering is inverted.** It sequences gap 1 first "since gap 2's fix
  needs gap 1's resolved origin to exist". The opposite holds: an intrinsic has no origin
  *module*, so gap 1's existence check has nothing to resolve to and must instead ask "does
  this module effectively admit this intrinsic" — which is exactly gap 2's table. Gap 2's
  computation is the prerequisite; both then read one table. They ship as one phase (R7).
- **B3. Two line numbers moved.** `resolve_export_origins` is at `src/resolve.rs:619`, not
  `:590`; `rewrite` is at `src/resolve.rs:312`, not `:484-616` (that range is `Visibility` and
  its helpers). `Visibility::origin` (`:498`), `exportable_names` (`:570`),
  `export_unknown_name_error` (`:525`, raised at `:657`), `ModuleInfo` (`src/ast.rs:147`),
  `IntrinsicVisibility` (`src/ast.rs:172`), `names_the_intrinsics` (`src/driver.rs:250`),
  `widen_intrinsics` (`:267`) and the gate (`src/check/word_families.rs:1068`) are all as
  cited.

## Design

Option (b) of the brief, in the shape the code actually wants: one **effective**
`IntrinsicVisibility` per module, computed in `driver::assemble_module` and stored in the
existing `ModuleInfo.intrinsics` field. Every consumer of that field — the caller-side gate
above all — keeps working with no signature change, and gap 1 reads the same table.

Option (a) (teaching `intrinsic_is_gated_out` to walk imports) is rejected: it would need the
selective map threaded to a `&Ctx`-only helper called from two hot dispatch sites, it would
re-derive per call what is a per-module constant, and it would leave gap 1 with a second,
separate walk.

### R1 — Effective visibility, defined

For each module `m`:

```text
effective[m] = own[m] union { n : (n -> h) in selective[m]
                                , is_name_dispatched_builtin(n)
                                , effective[h].admits(n) }
```

`own[m]` is today's `widen_intrinsics` fold. `selective[m]` is `selective_maps[m]`
(`src/driver.rs:399`, populated at `:462` for the explicit `| n |` clause and at `:454-461`
for the wildcard desugar). Union is per name, never per bit: a hub contributes *names*, so
`All` is never propagated (R3).

The union is idempotent by construction — `IntrinsicVisibility::Only` holds a `HashSet` — so a
module that writes `import: intrinsics | drop | ;` itself *and* imports a hub admitting `drop`
sees `Only({drop})`, not a duplicate-entry error. That was the brief's open worry; it needs no
rule, only a golden.

### R2 — Routes: bare-name only, never qualified

Only `selective_maps` is walked, so `import: "./hub.sth" hub ;` (qualified only) contributes
nothing, matching `is_name_visible_to_module` (`src/check/word_families.rs:1044`) and
`widen_intrinsics`' own note that there is no qualified spelling for an intrinsic. Probe
confirms `hub::drop` is not a route today and this slice does not make it one: in `rewrite`'s
qualified branch (`src/resolve.rs:336-386`) `self.words[target]` misses, and
`vis.origin(target, "drop")` returns `None` because R4 records the hub as its own origin, so
the name falls to `Ok(None)` and then to the ordinary unknown-word path. Golden required.

### R3 — A wildcard `intrinsics` import does not leak through a hub

Ruling on the brief's open question, in the narrowing direction it guessed, and it falls out
of R1 for free: a hub's contribution is enumerated from *its `export:` list* (that is what
`selective_maps[m]` was built from, both for `| n |` and for the wildcard desugar), never from
`effective[h]`. So `hub` writing `import: intrinsics * ; export: drop ;` contributes exactly
`drop`. There is no code path by which `IntrinsicVisibility::All` crosses a hub. Golden: a hub
that wildcards intrinsics and exports `drop` does not let its consumer call `add`.

### R4 — `export:` accepts an effectively-admitted intrinsic, with origin = self

In `resolve_export_origins` (`src/resolve.rs:619`) the immediate-source loop's first branch is
`declared[m].contains(name)`. Add a second self-source condition ahead of the `selectives` and
`import_maps` branches:

```text
is_name_dispatched_builtin(name) && intrinsics[m].admits(name)   =>  source = m
```

`intrinsics: &[IntrinsicVisibility]` is a new parameter on `resolve_export_origins` and on
`build_exported_origin` (`src/resolve.rs:605`); the latter reads it off
`module.modules[m].intrinsics` at the single call site (`src/resolve.rs:733`), which by then
holds the R1 value. Origin = `m`, so `Visibility::origin` (`src/resolve.rs:498`) returns `None`
for it (`origin != target` fails) and no call site is ever mangled against a hub — correct,
since an intrinsic must stay bare for builtin dispatch.

Consequences, each a golden:

- A hub with no `import: intrinsics` line exporting `drop` still fails with
  `export_unknown_name_error`, byte-identical (probed live today).
- A hub-of-hubs chain resolves, because `intrinsics[m]` is the *effective* value: the middle
  hub effectively admits what it re-exported inward, so it may re-export it outward.
- `is_name_dispatched_builtin`, not `BUILTIN_WORDS`, is the predicate, so the accept set at
  `export:` is exactly the gate set at the call site. The six surface comparisons
  (`eq`/`lt`/…) are excluded from both (`src/ast.rs:1561`); they are `core::cmp` words and
  already re-export as ordinary words.

### R5 — An intrinsic-name selective entry is exempt from the two collision rules

`check_selective_imports` (`src/check/declarations.rs:670`) rejects a selective name that
collides with a local decl (`:689`) or with another selective import (`:696`). Both must skip
an entry satisfying `is_name_dispatched_builtin(name) && module.modules[target].intrinsics
.admits(name)`, evaluated *after* the not-exported check at `:681` (so a hub that does not
export the name still fails first, and the exemption cannot fail open on an unexported name).

This is not convenience, it is coherence: the entry binds no word and carries no module
identity, so neither rule has a subject.

- **Local collision.** A user destructor `: drop ( Fd -- )` and the intrinsic `drop` already
  coexist in one file today (`examples/resources.sth:1,10`), and `resolve::mangle` exempts
  `drop`, so nothing shadows. Without the exemption, phase 2 would make
  `import: core::prelude * ;` unusable in any program with a destructor — a regression
  invented by the routing, not by the language.
- **Diamond.** This is the ruling on the brief's third open question. Two hubs both admitting
  `drop` union to the same set; a collision error would be reporting an ambiguity that has no
  two answers. Ordinary words keep colliding (probed live: the two-hub `lw` case reports
  `selective_collision_error` naming both source modules), and that must stay byte-identical.

### R6 — REPL: out of scope, benignly, and stated so

`intrinsic_is_gated_out` never fires on the REPL path (`ctx.modules()` is `None`,
`src/check/word_families.rs:1078-1082`); probed: `1 drop` at a bare REPL prompt runs. The
REPL's `import:` binding loops (`src/repl.rs:2310`, `:2469`) `continue` past an export name
with no matching word, so an exported intrinsic is silently skipped — the known
"prints success, binds nothing" shape, here harmless because the REPL admits the name
unconditionally anyway. No REPL change, no REPL golden beyond a note.

### R7 — One phase, because the two gaps are one table

Neither gap is independently observable: gap 1 alone accepts `export: drop ;` but every
consumer without its own `import: intrinsics` line is still rejected; gap 2 alone is
unreachable because no hub can export the name. R1's table is the shared prerequisite for
both. Splitting would ship a phase whose exit criterion is a placebo.

### R8 — Computation order and cycles

`effective` needs `effective[h]` before `effective[m]`. `closure.nodes` is discovery order,
*not* topological (`src/driver.rs:383-386`). The import graph is acyclic —
`closure.reject_cycles()` (`src/driver.rs:181`) runs well before the assembly loop — but the
computation must not *assume* it: use a memoized DFS with a per-walk `visited` set, mirroring
`walk_type_export_origin` (`src/driver.rs:336-357`), treating a revisit as "contribute
nothing" rather than panicking. Stated, not assumed, per the brief.

## Codebase map

| Anchor | Role in this slice |
| --- | --- |
| `src/driver.rs:400-493` | the per-node import loop; `intrinsics_by_module` is `own[m]` |
| `src/driver.rs:454-462` | wildcard desugar and explicit selective clause, the two `selective_maps` sources |
| `src/driver.rs:492` | where `own[m]` is pushed; R1's fixpoint runs after this loop closes |
| `src/driver.rs:608-613` | `ModuleInfo` construction; `intrinsics:` takes `effective[m]` |
| `src/driver.rs:336-357` | `walk_type_export_origin`, the memoized-walk pattern to mirror (R8) |
| `src/ast.rs:172-192` | `IntrinsicVisibility` and `admits`; may gain a union helper |
| `src/ast.rs:1560` | `is_name_dispatched_builtin`, the one predicate for R1/R4/R5 |
| `src/resolve.rs:605-676` | `build_exported_origin` / `resolve_export_origins`, R4's new param and branch |
| `src/resolve.rs:733` | the single `build_exported_origin` call site |
| `src/check/declarations.rs:681-703` | R5's two exempted rejections |
| `src/check/word_families.rs:1068-1082` | the gate — **unchanged**, and that is the design's point |
| `lib/prelude.sth:7-15,19` | the comment naming P7.S3q, and the export list phase 2 edits |
| `tests/common/mod.rs:108-148` | `fixture_imports`; see the test hazard below |

## Test hazard (read before writing a single golden)

`common::fixture_source` (`tests/common/mod.rs:214`) appends `import: intrinsics * ;` to any
`.sth` whose text does not already contain `import: intrinsics` (`:145-147`). Every consumer
fixture in this slice is defined by *not* having that line. Routing one through
`fixture_source` — as `tests/phase8_slice2.rs`'s `Tree::write` does — silently converts every
golden into a placebo that passes today.

**Rule: `tests/phase7_slice3q.rs` writes fixture bytes with `std::fs::write`, verbatim, and
must not call `common::fixture_source` or `common::write_fixture` for any consumer file.** Its
own `Tree` helper is a copy of `phase8_slice2.rs`'s with that call removed, and carries a
comment saying why. Every negative golden pins the exact diagnostic string, never `is_err()`.

## Tests

End-to-end, `tests/phase7_slice3q.rs` (all through the real binary):

- `hub_re_exporting_a_gated_intrinsic_is_callable_bare` — the headline: consumer has no
  `import: intrinsics` line, calls `drop`, builds and runs.
- `a_hub_of_hubs_carries_the_intrinsic` (R4, chain depth two — the depth-two probe rule).
- `a_wildcard_intrinsics_import_does_not_leak_through_a_hub` (R3): the same hub, consumer
  calls `add`, still `ungated_intrinsic_error`. Mutation witness for R3.
- `a_hub_without_an_intrinsics_import_cannot_export_one` (R4): unchanged
  `export_unknown_name_error`, pinned byte-for-byte.
- `a_qualified_hub_import_is_not_a_route` (R2): `import: "./hub.sth" hub ;` plus `hub::drop`
  stays rejected.
- `an_own_intrinsics_import_and_a_hub_admitting_the_same_name_agree` (R1 idempotency).
- `a_local_destructor_coexists_with_a_wildcard_hub_import` (R5, local-collision exemption) and
  `two_hubs_admitting_one_intrinsic_are_a_union_not_a_collision` (R5, diamond).
- `two_hubs_re_exporting_one_ordinary_word_still_collide` — the R5 exemption's blast-radius
  guard, pinning `selective_collision_error` byte-for-byte.
- `a_conversion_intrinsic_re_exports_through_a_hub` (`>i64`, B1's open-set member).

Unit, beside the code:

- `src/driver.rs`: the R1 fold over a hand-built closure — own-only, hub-only, union,
  wildcard-narrowing, and a depth-two chain; plus an R8 case with a fabricated back edge
  asserting termination rather than a hang.
- `src/resolve.rs`: `resolve_export_origins` with an `intrinsics` slice — accept with
  `origin == m`, reject when the module does not admit, and `Visibility::origin` returning
  `None` for the accepted entry (the no-mangling guarantee).
- `src/check/declarations.rs`: the R5 exemption both ways on one fixture pair.

**Mutation-test the R3, R5-exemption and R5-blast-radius guards** before phase exit (delete
what each guards, prove the test fails). Sooth has shipped placebo tests repeatedly; R3's
"wildcard does not leak" and R5's "ordinary words still collide" are precisely the shapes that
survive a naive implementation.

## Phase 1 — the effective-visibility table and both gaps (hard)

**Scope.** `src/driver.rs` (R1, R8), `src/ast.rs` (union helper if wanted),
`src/resolve.rs:605-676,733` (R4), `src/check/declarations.rs:681-703` (R5),
`tests/phase7_slice3q.rs` (new), plus the unit tests above.

**Out of bounds.** `src/check/word_families.rs` (the gate is unchanged — an edit there means
the design went wrong), `exportable_names`, `src/repl.rs`, `src/ir/`, `lib/`, `examples/`, and
`tests/common/mod.rs`.

**Entry.** None; `ab41270` is green.

**Exit.** Every test above passes; the three named mutation checks fail when their guard is
removed; `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
--no-fail-fast` green; the diff touches no file outside Scope.

## Phase 2 — `core::prelude` re-exports `drop` (standard)

**Scope.** `lib/prelude.sth`: add `import: intrinsics | drop | ;`, add `drop` to the `export:`
list (`:19`), and rewrite the `:7-15` comment — the "`drop` cannot, at all" paragraph is now
false, while the `.`-overload paragraph stays true (that gap is P7.S3i/S3e-follow territory,
untouched here). Add one golden asserting a consumer with only
`import: core::prelude | drop | ;` calls `drop` and runs. This is the roadmap exit criterion's
literal wording.

**Out of bounds.** Adding `drop` to `CORE_WORDS` in `tests/common/mod.rs` (it would rewrite
the whole corpus's import blocks for no gain, and `fixture_imports` already injects
`import: intrinsics * ;`); any other `lib/` word; sweeping `examples/` for now-redundant
`import: intrinsics` lines. Verified: no in-tree file both declares `: drop` and wildcards
`core::prelude`, so this phase should have zero corpus fallout — if it has any, stop and
report rather than sweeping.

**Entry.** Phase 1 landed and green.

**Exit.** The new golden passes; full green; `git diff --stat` shows `lib/prelude.sth` and one
test file only.

## Out of scope

- The `.` operator-overload hub gap (`bool_imports`, `tests/common/mod.rs:168-180`): an
  overload's candidate lookup is one hop by design and is a different mechanism.
- Any qualified spelling for an intrinsic (R2).
- REPL behaviour (R6).
- Lowering, IR, monomorphization: this is check-time visibility only.
- `exportable_names` and the ordinary word/type re-export path, which already work.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "effective intrinsic visibility table with hub re-export at export and at the caller gate", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "core prelude re-exports drop", "effort": "S", "difficulty": "standard" }
  ]
}
```
