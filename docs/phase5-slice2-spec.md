# Phase 5 Slice 2: Result and Option (spec, delivered)

`Result 'T 'E` and `Option 'T` ship as real, importable generic library enums
(`lib/result.sth`, `lib/option.sth`), on top of the one mechanism they needed:
**cross-module generic application**. Slice 1 supplied generic `type:` declarations;
`3df4846` supplied clause-style elimination over a generic instantiation. No
branch-on-result codegen work (already worked), no changes to
`instantiate_struct`/`instantiate_enum` or to `check/word_entry.rs` elimination.

## Settled decisions

- **OQ2:** plain relative-path `import:`, no special resolution rule. `Result`/`Option`
  are reached like any other cross-module type. "Any program anywhere resolves this"
  stays Phase 6.
- **OQ3:** `Result` in `lib/result.sth`, `Option` in `lib/option.sth`, each holding its
  `type:` declaration plus an `export:` line (an unexported generic fails at the
  importer with ``not exported from module `r` ``).
- **OQ1 — whole-closure generic-header pre-pass, idempotent.** `assemble_module` runs
  `parser::prepass_generic_typedefs` over every module in the closure before any body
  parses, after `generics` is constructed and before the body-parse loop, sharing the
  body loop's `arrays`/`owned_cells`/`refs` (ids stay in sync) and each module's real
  import/exports/selective maps (a generic field naming an imported concrete type
  resolves identically in both passes). The per-module call inside `parse_bodies` is
  **kept**: `parse_without_prelude` and every direct-`parse_bodies` test never reach
  `assemble_module`, and removing it would silently stop registering generics there.
  `parse_generic_typedefs` instead skips a header already registered *before this pass
  began* (`already` = the `(structs.len(), enums.len())` snapshot at entry) and advances
  via `skip_typedef` to the terminating `;`. The snapshot is load-bearing: without it a
  header's own push makes a genuine second declaration of the same name look
  pre-registered, and a real duplicate never reaches `check_duplicate_type_names`.
- **OQ4 — attributeless variant fields, enums only.** A variant field may omit its name;
  it is stored under `parser::POSITIONAL_FIELD_NAME` (`"$positional field$"`, contains a
  space, so the lexer can never produce it as a `Word` and it can neither be referenced
  nor collide). No accessor suppression exists or is needed: enum variant fields mint
  only a per-variant constructor, Get/Set/Peek are struct-only. Elimination already
  destructures positionally, so nothing there changed. Only the unambiguous `'`-prefixed
  case is sugar: a bare *concrete* type name still parses as a named field missing its
  type and hits `generic_odd_field_count_error`. Struct positional fields are out of
  scope.

## Phases

### Phase 1 — attributeless variant grammar

Delivered in `parse_generic_variant_fields`: a `'`-prefixed token opens a positional
field (no lookahead needed), everything else takes the existing named-field path.
`type: Option 'T | None | Some 'T ;` and `type: Result 'T 'E | Ok 'T | Err 'E ;` parse.

Because the stored name is an internal placeholder, every diagnostic printing a variant
field name was routed through the new `check::variant_field_desc`, which renders a
positional field as `field {idx}`. Tests in `check.rs`, `check/audits.rs` and
`check/declarations.rs` assert both the by-index wording and that the placeholder never
leaks. A dead stored-reference validation path was removed in the same sweep.

### Phase 2 — cross-module generic application

1. The whole-closure pre-pass and idempotency guard above (`src/driver.rs`,
   `src/parser.rs`); the stale "generic headers are not pre-pass-registered" comments in
   `src/parser.rs` and the `GenericTypes` doc-comment in `src/ast.rs` now state the new
   design.
2. `resolve_type_or_apply` splits a `q::Base` application, maps `q` through the import
   map, and enforces export visibility (`not_exported_error`) before
   `find_struct`/`find_enum`. A **bare** name resolves own-module-first and then through
   the selective-import map (`bare_generic_owner`), so a local header shadows a
   selectively imported one of the same name; a name that is neither stays `unknown
   type`, and `parse_generic_application_from_another_module_is_unknown` keeps asserting
   that on the direct-`parse_bodies` harness.

Test placement is load-bearing: the qualified positive case, the discovery-order
independence, and the no-double-registration witness all run through
`discover_closure`/`assemble_module` over a real two-file closure. On the
direct-`parse_bodies` path `parse_generic_typedefs` runs exactly once regardless of the
guard, so an "exactly one entry" assertion there is a placebo.

### Phase 3 — library files and goldens

`lib/result.sth` and `lib/option.sth` as above; goldens in `tests/phase5_slice2.rs`, all
asserting exact stdout:

- `Result[i64 i64]` constructed via `if` and eliminated over both arms (`12`, `-1`).
- `Result[i64 str]` — the **asymmetric** instantiation. Every other instantiation in the
  repo is symmetric and cannot tell `Ok 'T | Err 'E` from its swap.
- `Option[i64]` imported from the committed `lib/option.sth` and eliminated over `Some`
  and `None` (`5`, `9`), which is also the only witness of that file's `export:` line.
- `Option[^Node]` — the pointer/nullability shape DESIGN.md names as `Option`'s reason
  for existing; builds and runs clean (`7`, `0`), no `#[ignore]` needed.
- Cross-module: `r::Result[i64 i64]` applied qualified against the real library file,
  plus a two-file closure built in **both** discovery orders. The applier-first order is
  what the whole-closure pre-pass exists for.

DESIGN.md and ROADMAP.md now point at `lib/option.sth`. ROADMAP's "`Option['T]`
importable from `core`" clause is satisfied by the relative-path import golden; the
general case is Phase 6's.

## Out of scope / known limitations

- `?` short-circuit sugar; branch-on-result IR work (already delivered); struct
  positional fields; bounds, recursion, nested generics; the allocator returning
  `Option`/`Result`.
- Library import resolution outside this repo, and any `lib/`-relative or `core::`
  import rule (Phase 6).
- **Bare `None` is not inferred from context.** It binds to exactly one `Option[T]` per
  program: with `Option[i64]` and `Option[^Node]` both in scope, `0 None unwrap-node`
  fails ``expected `Option[^Node]`, found `Option[i64]` ``. A nullable pointer and a
  nullable int in one program is therefore not expressible, which blunts `Option`'s
  headline use. Needs generic argument inference or a qualified/annotated `None`.
- **A generic instantiation cannot appear in a cross-module word effect.** An exported
  word whose effect names `Box[i64]` is rejected with ``exported word `make` names
  private type `Box[i64]`, which is not exported``, and the advice is unfollowable
  (`export: Box[i64] ;` does not parse). So a fallible word in one module cannot yet
  *return* a `Result` to another. Fix it together with `instantiate_struct`'s dedup key
  `(generic_idx, applying_module, args)`: two importers of one `Box` mint two
  non-identical `Box[i64]` types, which only the export wall keeps unreachable today.

## Files touched

`src/parser.rs` (positional fields, `prepass_generic_typedefs`, idempotency, qualified
and selective generic resolution), `src/driver.rs` (pre-pass call site), `src/ast.rs`
(doc-comment), `src/check.rs` + `check/{audits,declarations}.rs`
(`variant_field_desc`), `lib/result.sth`, `lib/option.sth`, `tests/phase5_slice2.rs`,
DESIGN.md, ROADMAP.md.

```json
{
  "phases": [
    { "phase": 1, "focus": "attributeless variant grammar (positional field, no accessor, enums only)", "difficulty": "standard" },
    { "phase": 2, "focus": "cross-module generic application: whole-closure header pre-pass and qualified resolution", "difficulty": "hard" },
    { "phase": 3, "focus": "Result and Option library files with construct/monomorphize/eliminate and cross-module goldens", "difficulty": "standard" }
  ]
}
```
