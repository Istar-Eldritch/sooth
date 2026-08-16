# Phase 5 Slice 2: Result and Option (spec)

Ship `Result 'T 'E` and `Option 'T` as real, importable generic library enums, and
build the one genuinely-missing mechanism they need: **cross-module generic
application**. Slice 1 shipped generic `type:` declarations (construction, structural
dedup, generated words); `3df4846` shipped clause-style elimination over a generic
enum's instantiation. Both prerequisites are in place. This slice does not add
branch-on-result codegen (recon 1: it already works), and does not touch the
instantiation or elimination machinery (recon-confirmed correct for these shapes).

## Recon verification (against `main` HEAD, 2026-08-16)

The brief's recon was measured at `3df4846`; HEAD is now `2a5009d` (two docs-only
commits on top). Every load-bearing citation re-verified against current source:

- `resolve_type_or_apply` (`src/parser.rs:2712`) resolves a generic name via
  `self.generics.find_struct(name, self.module)` / `find_enum` with **no `q::Base`
  qualifier handling** — confirmed.
- `find_struct` / `find_enum` (`src/ast.rs:498`, `:505`) match on
  `d.name == name && d.module == module` — bare name **and** current module only —
  confirmed.
- `resolve_type_name_in_module` (`src/ast.rs:157`) already splits a `q::Base` name,
  maps `q` through the import map, and resolves `Base` in the target module (concrete
  types only) — confirmed; this is the pattern to copy.
- `prepass_and_register` runs over every file before any body parses
  (`src/driver.rs:180`), but its token pre-pass **skips generic headers** (the
  `continue` at `src/parser.rs:74`) — confirmed. Generic headers are registered only
  during each module's own `parse_bodies` via `parse_generic_typedefs`
  (`src/parser.rs:337`, method at `:2668`), i.e. in discovery order — confirmed.
- `prelude_words()` (`src/parser.rs:385`) returns only `.words`, dropping any
  structs/enums/generics from `lib/core.sth` — confirmed; a type cannot ride the
  no-import prelude path.
- `discover_closure` (`src/driver.rs:78`) joins every `import:` path to the importing
  file's own directory; no library search path — confirmed.
- `parse_generic_application_from_another_module_is_unknown` (`src/parser.rs:4121`)
  drives `parse_bodies` directly with two modules and asserts a cross-module
  `Box[i64]` fails — confirmed; this test encodes the old D4 "own module only" rule
  and is updated by this slice (see Phase 2).
- Clause elimination binds variant fields **positionally** (`|v|` over
  `variant.fields`, field name never referenced — `src/check/word_entry.rs:345`+),
  so attributeless variant fields need no elimination change — confirmed.

## Settled decisions (do not reopen)

- **OQ2 (settled in brief):** plain relative-path `import:`, no special resolution
  rule. `Result`/`Option` are reached exactly like any other cross-module type. The
  general "any program anywhere resolves this" case stays Phase 6.
- **OQ3 (settled in brief):** `Result` lives in `lib/result.sth`, `Option` in
  `lib/option.sth`, each holding only that type's `type:` declaration.
- No branch-on-result codegen work (recon 1, decision 1).
- No changes to `instantiate_struct`/`instantiate_enum` or to `check/word_entry.rs`
  elimination.
- No `?` sugar (out of scope, ROADMAP).

## Open questions, settled by this spec

### OQ1 — pre-pass shape for cross-module generic headers → whole-closure hoist of `parse_generic_typedefs`, made idempotent

**Settled:** run the *existing* generic-typedef parse for **every** module in the
closure before **any** module's body parses, by calling it a second time from a
dedicated whole-closure loop in `assemble_module` (`src/driver.rs`).

**Corrected during review (round 1): the per-module call inside `parse_bodies`
(`src/parser.rs:337`) is NOT removed.** Two callers reach `parse_bodies` without ever
going through `assemble_module`: the single-file path `parse_without_prelude`
(`src/parser.rs:415`), and every direct-`parse_bodies`-driving unit test (including
`parse_generic_application_from_another_module_is_unknown`, `:4121`). Removing the
in-body call, as originally drafted, silently stops registering generic types on both
paths and regresses this slice's own "all pre-existing tests pass unchanged" exit
criterion. Instead, make registration **idempotent**: `parse_generic_typedefs` skips a
header whose `(name, module)` is already present in `self.generics.structs`/`enums`
(via `find_struct`/`find_enum`) before parsing and pushing it. With that guard, the
whole-closure pre-pass registers first and the in-body call becomes a safe no-op on
the `assemble_module` path, while `parse_without_prelude` and every direct-`parse_bodies`
test keep working exactly as today through their own in-body call, unmodified.

Rationale, and why a header-only skeleton pre-pass is rejected: an applying module B
must be able to *instantiate* `q::Box[i64]`, and the pre-pass has to run the full field
parse (not just name/arity) because that's what `parse_generic_typedefs` already does
as one step — there's no cheaper partial parse available to reuse instead. (Fields are
stored **unresolved**, as `PolyType::Var`/`PolyType::Concrete`, `src/parser.rs:2813`;
resolution against concrete arguments happens later, at instantiation time in the
applying module, not during this parse. The reason to run the full header parse
upfront is reuse of existing machinery, not a resolved-poly-type requirement.) Within
this slice's scope a generic decl's fields reference concrete types (already
registered by `prepass_and_register`), including imported and selectively-imported
ones, plus the decl's own type variables — never another generic's instantiation
(nested generics are out of scope). The pre-pass gets the declaring module's real
name environment (below), so an imported field type resolves there exactly as it does
in the body pass.

Mechanics:

- Add a whole-closure step in `assemble_module` (`src/driver.rs`), inserted **after**
  `let mut generics = crate::ast::GenericTypes::with_bases(structs.len(), enums.len());`
  (`src/driver.rs:210`) and **before** the `for (m, node) in closure.nodes.iter()...`
  body-parse loop — not "immediately after `prepass_and_register`", which runs before
  `generics`, `arrays`/`owned_cells`/`refs` even exist. For each module `m` in
  discovery order, construct a `Parser` sharing the *same* `&mut arrays`/`owned_cells`/
  `refs` vecs the body loop will use (so ids stay in sync across the two passes), and
  with that module's own import/exports/selective maps — the same ones the body loop
  passes it — so a generic field naming an imported concrete type resolves in both
  passes. Then call `parse_generic_typedefs()` on it.
- Extract a new `pub(crate) fn prepass_generic_typedefs(tokens, structs, enums, module,
  imports, exports, selective, arrays, owned_cells, refs, generics)` in `parser.rs`
  that constructs exactly this `Parser` and calls `parse_generic_typedefs()` —
  `driver.rs` cannot reach a private
  `Parser`/method directly, only a `pub` function (mirroring how it already calls
  `parser::parse_bodies`/`parser::prepass_and_register`), so name this wrapper rather
  than leaving its shape to the implementer.
- Keep the per-module `parse_generic_typedefs()` call at the top of `parse_bodies`
  (`src/parser.rs:337`) exactly where it is; make it idempotent per the correction
  above rather than removing it. **The skip branch must still advance `self.pos` past
  the already-registered header to its terminating `;`** (reuse `skip_typedef`'s
  terminator scan) before continuing the scan loop — skipping the push without
  advancing the cursor infinite-loops.
- Update the two now-stale comments: the `continue` comment at `src/parser.rs:74` and
  the `GenericTypes` doc-comment at `src/ast.rs:327-341` still say generic headers
  are *not* pre-pass-registered; after this slice they are, for a closure assembled
  through `assemble_module` (single-file/direct-`parse_bodies` callers still register
  them only in-body, which is unchanged and sufficient there). Per this project's
  no-history convention, state the new design plainly, do not narrate the change.

### OQ4 — attributeless variant grammar → positional field, no accessor, enums only

**Settled:** a variant field may omit its name; an unnamed field is a positional type
with **no accessor word generated**. Elimination already destructures positionally
(`| Ok |v|`), so no elimination or accessor change is needed. Scope is **enum
variants only** — that is exactly what `Result`/`Option` need; struct positional
fields are not in scope (no consumer this slice).

Concrete grammar: inside a `| Variant ...` arm, a field is either the existing
`name 'T` / `name ConcreteType` spelling **or** a bare type with no leading name.
So all of these parse:

```sooth
type: Result 'T 'E | Ok 'T | Err 'E ;        # both arms attributeless
type: Option 'T | None | Some 'T ;           # None: zero fields; Some: one attributeless field
```

Implementation: an unnamed field is stored with an internal placeholder name in
`GenericVariantDecl.fields` (`Vec<(String, PolyType)>`, `src/ast.rs:322`) that is not a
parseable identifier (so it can never be typed as a field/accessor reference, and reads
sensibly if it ever surfaces in the `payload field` diagnostic at
`src/check/declarations.rs:556`). **Corrected during review (round 1): no accessor
suppression is needed, because none exists to suppress.** Enum variant fields already
mint no accessor words today — only a per-variant constructor (`src/ir/layout.rs`'s
`ewords` loop); Get/Set/Peek accessors are struct-only (`src/ir/layout.rs:494`). So a
placeholder-named variant field needs no new suppression logic; it only needs a name
that can never collide with anything (there is no variant duplicate-field-name check
today, `src/check/declarations.rs:554` is a diagnostic string, not a check, and layout
is positional by type, `:1271`, so a placeholder cannot be referenced or collide either
way). The named-field spelling (`val 'T`) continues to work unchanged. `Result`/
`Option` ship using the attributeless spelling from the start (decision 4), which is
their natural exit witness for the sugar.

Disambiguation rule (a field with no name vs. a named field missing its type): a
`'`-prefixed token can never be a field name (`reject_ty_var_field_name`,
`src/parser.rs:175`), so `Ok 'T` is unambiguous — the only shape `Result`/`Option`
actually need. A bare *concrete* type name (`Some Point` with no preceding field name)
is ambiguous with a named field missing its type, and is resolved by the existing
token-count-to-the-next-`|`/`;` logic (`src/parser.rs:2497`, `:2538`,
`generic_odd_field_count_error` at `:1092`): an odd token count before the next
separator is a named field, even is a bare positional type. `Result`/`Option` never
exercise the concrete-type case (their fields are always `'T`-style), so implement the
unambiguous ty-var case fully; the concrete-type disambiguation only needs to not
regress the existing odd/even arity check, not grow new behavior this slice can't test.

## Phases

### Phase 1 — attributeless variant grammar

Parser sugar over the existing named-field variant mechanism. A variant field with no
leading name parses as a positional field with an internal placeholder name. Named
fields unchanged. No accessor-suppression work is needed (OQ4 correction): enum variant
fields mint no accessor words today regardless of field name, only a constructor.

- Extend generic variant-field parsing (ty-var fields only, per OQ4's disambiguation
  rule) to accept a bare `'T`-style type with no field name, stored under a
  non-parseable placeholder name.
- Unit tests beside the parser (`#[cfg(test)] mod tests`): happy path
  (`type: Option 'T | None | Some 'T ;` parses with a one-field `Some` and a zero-field
  `None`); at least one error/edge case (e.g. the placeholder name can never be
  referenced as an accessor / does not collide with a real field name, and a mix of a
  named field and an attributeless field in the same variant is rejected or handled
  per whatever the implementer's lookahead naturally does — assert it explicitly
  either way rather than leaving it unexercised).

Exit: attributeless enum variants parse and construct; named-field spelling still parses;
`cargo test` green for the new unit tests plus all pre-existing parser tests.

### Phase 2 — cross-module generic application (the real engineering)

Two changes, both recon-confirmed necessary:

1. **Whole-closure generic-header pre-pass, made idempotent** (OQ1): add the
   `assemble_module` pre-pass loop at the corrected insertion point (after `generics`
   is constructed, before the body-parse loop); make `parse_generic_typedefs` skip an
   already-registered `(name, module)`; do **not** remove its in-body call. Update the
   stale comments at `src/parser.rs:74` and `src/ast.rs:327-341`.
2. **Qualified generic application resolution**: extend `resolve_type_or_apply`
   (`src/parser.rs:2712`) to split a `q::Base` application name and resolve `q` through
   the import map before `find_struct`/`find_enum`, mirroring
   `resolve_type_name_in_module` (`src/ast.rs:157`). A **bare** name resolves against
   the applying module first and then, failing that, against the module it is
   selectively imported from (`import: q | Box | "box.sth"`) — the same two-step a bare
   concrete name already gets, so a local header shadows an imported one of the same
   name. A bare name that is neither declared locally nor selectively imported stays
   `unknown type`.

Update `parse_generic_application_from_another_module_is_unknown`
(`src/parser.rs:4121`): it currently asserts the old D4 "own module only" rule with a
bare `Box[i64]` and no imports, driving `parse_bodies` directly. Keep this bare-unqualified
case (still correct on the direct-`parse_bodies` harness, since that harness never runs
the `assemble_module` pre-pass and a bare name is own-module-only regardless). **The
new positive qualified case, and the discovery-order-independence assertion, must be a
separate test built on `discover_closure`/`assemble_module` over a real two-file closure
(mirroring the existing `src/driver.rs` test style)** — not bolted onto the
direct-`parse_bodies` test, which never exercises the whole-closure pre-pass the
discovery-order guarantee depends on. Assert both discovery orders (owner file first,
and importer file first) resolve identically through the real `assemble_module` path.
Do not weaken the bare-name rejection.

Unit tests: qualified cross-module application resolves and monomorphizes (via
`assemble_module`, both discovery orders); bare cross-module application still rejects
(via direct `parse_bodies`, unchanged). **The no-double-registration witness must also
be built on `assemble_module`, not the single-file/direct-`parse_bodies` path**
(correction, round 2): on the direct-`parse_bodies` path `parse_generic_typedefs` runs
exactly once (no pre-pass exists there), so an "exactly one entry" assertion on that
path passes whether or not the idempotency guard exists — it is a placebo, since
double-registration is only reachable where the pre-pass *and* the in-body call both
fire on the same shared `generics` (the `assemble_module` path). Build this as a
two-file `assemble_module` closure (reusing the qualified-application test's closure)
and assert `generic_structs`/`generic_enums` has exactly one entry for the owner
module's declared type, not two.

Exit: `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green; a
qualified cross-module generic application resolves and monomorphizes regardless of
discovery order; a bare cross-module application still reports `unknown type`; no
double-registration.

Difficulty: **hard** (whole-closure ordering + resolution, the slice's core mechanism).

### Phase 3 — Result and Option library files plus goldens

- Add `lib/result.sth` containing exactly
  `type: Result 'T 'E | Ok 'T | Err 'E ;` (attributeless spelling, Phase 1).
- Add `lib/option.sth` containing exactly `type: Option 'T | None | Some 'T ;`.
- Golden tests (source in → concrete stdout, not merely "it builds"):
  - **2-variable (`Result`)**: a fallible word returns `Result[i64 i64]`
    (e.g. `dup 0 < ~[ drop drop -1 Err ] ~[ + Ok ] if`) and a clause eliminator
    (`| Ok |v| v | Err |e| e`) handles both arms; assert exact stdout (`12` then `-1`
    from the brief's probe, or the chosen inputs).
  - **1-variable (`Option`)**: construct, monomorphize, and eliminate an
    `Option[i64]` instantiation with a concrete stdout assertion (both `Some` and
    `None` arms). **Also instantiate `Option` over a pointer type
    (`type: Node val i64 ;` or any one-field struct defined in the golden's own
    source, then `Option[^Node]`), the nullability shape DESIGN.md names as `Option`'s
    actual reason for existing** (`^T` stays non-null; `Option['T]` is the named
    answer). Every existing generic-instantiation test in this codebase uses
    `i64`/`bool`/nested aggregates only — never a pointer type argument — so this
    shape is currently unwitnessed. **Explicit disposition (correction, round 2):** if
    `Option[^Node]` builds and runs cleanly, assert its exact stdout like every other
    golden here. If it does **not** build or run cleanly, commit the golden anyway as
    a `#[ignore = "<verbatim compiler/runtime error>"]`-annotated test (so `cargo test`
    stays green and the phase still exits) and state the limitation as the first line
    of the phase's summary; do not delete the golden and do not substitute a
    non-pointer instantiation to make the criterion pass quietly. Either outcome
    satisfies this bullet; silence about which one occurred does not.
  - **Cross-module import**: a program that `import:`s `Result` (or `Option`) from its
    `lib/` file by ordinary relative path, applies it qualified at the importing module,
    and monomorphizes correctly — a direct witness of Phase 2, exercising both discovery
    orders.

Exit: both library types build and run through construction → monomorphization →
elimination with concrete stdout assertions, including `Option` instantiated over a
pointer type; the cross-module golden passes in both discovery orders; all
pre-existing Slice 1 and elimination tests pass unchanged. ROADMAP's "`Option['T]`
importable from `core`" exit clause is satisfied by the in-repo relative-path import
golden here — the general case (any program anywhere resolving this with no shared
directory) is explicitly Phase 6's, not re-attempted by this slice.

## Out of scope

- `?` short-circuit sugar (dropped from Phase 5).
- New branch-on-result IR/checker work (already delivered).
- Library import resolution for programs outside this repo / any `lib/`-relative or
  `core::`-prefixed special import rule (Phase 6 dependency management).
- Struct positional (unnamed) fields — only enum variant fields are sugared here.
- Bounds, recursion, nested generics — still Slice 1 out-of-scope.
- The allocator returning `Option`/`Result` (a future consumer, not this slice).
- **A generic instantiation in a cross-module word effect** (later slice; it gates the
  same Phase 6 territory as the general import rule). An exported word whose effect
  names `Box[i64]` is rejected with ``exported word `make` names private type
  `Box[i64]`, which is not exported``, and that advice is unfollowable: `export:
  Box[i64] ;` does not parse (`expected ';' terminating 'export:', found LBracket`).
  This predates Phase 2 and Phase 3's goldens do not need it (they apply an imported
  generic within one module), but it means a fallible word in one module cannot yet
  *return* a `Result` to another — most of why `Result` exists. Fix it together with
  `instantiate_struct`'s dedup key, `(generic_idx, applying_module, args)`
  (`src/ast.rs:535`): two importers of one `Box` mint two non-identical `Box[i64]`
  types, which only the export wall keeps unreachable today.

## Sequencing

No gate from any open Phase 4 item. Builds on Slice 1 and the elimination fix
(`3df4846`). Touches `src/parser.rs` (attributeless variants, qualified generic
resolution, `parse_generic_typedefs` extraction), `src/driver.rs` (whole-closure
generic pre-pass call site), `src/ast.rs` (stale doc-comment), and two new `lib/`
files. No changes to `src/check/word_entry.rs` or the instantiation machinery.

```json
{
  "phases": [
    { "phase": 1, "focus": "attributeless variant grammar (positional field, no accessor, enums only)", "difficulty": "standard" },
    { "phase": 2, "focus": "cross-module generic application: whole-closure header pre-pass and qualified resolution", "difficulty": "hard" },
    { "phase": 3, "focus": "Result and Option library files with construct/monomorphize/eliminate and cross-module goldens", "difficulty": "standard" }
  ]
}
```
