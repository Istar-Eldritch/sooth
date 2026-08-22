# Phase 7 Slice 3i: `bool` as an ordinary `core::bool` enum (spec)

## Goal

Delete the compiler-baked `bool` enum injection and declare the boolean type once as
ordinary source in `core::bool`, so a module resolves `bool`, `True`, `False`, and the
`true`/`false` literal spellings the same way it resolves any other imported enum: through
`import: core::bool ;` or a re-exporting hub (`import: core::prelude * ;`). This finishes
P8.S2's own rule ("nothing resolves without an `import:`") for the one type it left behind:
P8.S2 gated `BUILTIN_WORDS` and deleted the prelude's *word* injection, but `bool` was
never a `BUILTIN_WORDS` entry: it is a second, independent injection mechanism P8.S2 did
not touch.

`lib/bool.sth` already exists and declares `if`/`unless` over the `branch`/`tag`
intrinsics; `core::prelude` already imports and re-exports those two words. What this slice
adds to `lib/bool.sth` is the `type: bool | False | True ;` declaration itself (plus the
bool `.` print word, moved out of the compiler), and what it removes is the two
unconditional registry-injection sites and every code path that reads a fixed
`BOOL_ENUM_ID` / `Type::BOOL` constant.

This is a global compiler-behaviour change of the same class as P8.S2's prelude deletion,
so its core switch is a single atomic phase (see Implementation).

## Citation corrections (verified against live source)

Every brief citation was checked against the current tree; corrections:

- **Type name is `bool`, not `Bool`.** The brief's header `type: Bool | False | True ;` is
  a typo. `bool_enum_decl()` (`src/ast.rs:915`) builds an `EnumDecl` whose `name`/
  `name_static` is `"bool"` (lowercase), with variants `False` (index 0) and `True` (index
  1). Every existing signature writes `bool` (e.g. `and ( bool bool -- bool )`), and
  `Type::BOOL.name()` renders `"bool"`. The `core::bool` source declaration is therefore
  `type: bool | False | True ;`, spelled to match the existing `bool_enum_decl` name
  exactly, so no signature across the corpus needs a rename. Renaming the type to `Bool`
  is explicitly **out of scope** (would be a corpus-wide signature migration unrelated to
  removing the injection).
- `bool_enum_decl()`: brief `src/ast.rs:914-933` → live `src/ast.rs:915` (fn), body through
  ~938. `BOOL_ENUM_ID` const at `src/ast.rs:908`.
- Single-file injection: brief `src/parser.rs:508-510` → live `src/parser.rs:518`
  (`let mut enums = vec![crate::ast::bool_enum_decl()];`); the `+1` user-enum rebase brief
  `src/parser.rs:536-539` → live `src/parser.rs:539`.
- Multi-file injection: brief `src/driver.rs:290-293` → live `src/driver.rs:296`
  (`let mut enums = vec![crate::ast::bool_enum_decl()];`), comment at 293-295.
- `from_name` special-case: brief `src/ast.rs:1857-1863` conflates two symbols. Live: the
  `Type::BOOL` const is at `src/ast.rs:1863`; the load-bearing gate is
  `Type::from_name` at `src/ast.rs:1866-1868`, whose `if name == "bool" { return
  Some(Type::BOOL); }` returns the fixed enum unconditionally, ahead of any
  module/registry lookup.
- Backend `.` carve-out: brief `src/backend/qbe.rs:1236-1251` → live
  `src/backend/qbe.rs:1235-1251` (the `IrType::Bool | IrType::Enum(BOOL_ENUM_ID)` Print
  arm), with the `$boolstrs` data emitted at `src/backend/qbe.rs:82` and its unit test
  `emit_print_on_bool_indexes_boolstrs_via_sfmt` at `src/backend/qbe.rs:1778`.
- REPL sites: `remap_type`'s `id == BOOL_ENUM_ID => Type::BOOL` arm live at
  `src/repl.rs:214`; `format_stack`'s `_ if *ty == Type::BOOL` render arm live at
  `src/repl.rs:~633` (fn opens 618; brief said 628); `Session::new`'s
  `enums: vec![bool_enum_decl()]` seed live at `src/repl.rs:~1080` and the
  `eval_def(bool_print_word_def())` seed at `src/repl.rs:~1112` (brief said 1082);
  `splice_import`'s `.skip(1)` bool-dedup live at `src/repl.rs:2119`.
- `bool_print_word_def()` live at `src/ast.rs:948`: a source-shaped `.` overload that, per
  variant arm, `drop`s the bool then prints the literal string `"false"`/`"true"` via
  `str`'s own `.`.
- `printable_types()` live at `src/check/builtins.rs:121`: numeric types + `str`/`cstr`
  only, **no bool** (relevant to Ruling 3).

## Findings carried from the brief (confirmed)

1. `bool_enum_decl()` already produces an ordinary two-variant zero-payload `EnumDecl`;
   there is no representation gap between the injected form and a user declaration. Only
   *how* it enters the registry is special.
2. Injection happens at two independent, unconditional sites (`parser::parse`,
   `driver::assemble_module`), neither consulting imports.
3. `true`/`false` are already ordinary word calls lowered to `TermKind::Call("True"/
   "False")`; there is no `TermKind::BoolLit`. The one exception is the `static:`
   initializer, which parses `true`/`false` at parse time keyed on `ty == Type::BOOL` into
   `StaticInit::Bool`, with no checker pass to resolve a call against (drives Ruling 1).
4. `Type::from_name("bool")` and the `Type::BOOL` const are the two remaining global
   readers; ~80 compiler-internal consumers read `Type::BOOL` (comparison/`and`/`or`/`xor`
   result typing, `branch`/`tag` intrinsics, backend lowering, REPL pinning).
5. The REPL has its own `BOOL_ENUM_ID` pinning that assumes global injection (drives
   Ruling 2 and the phase-2 REPL migration).
6. The P8.S2 intrinsics gate never listed `bool`/`True`/`False`; this slice removes a
   different, independent global-visibility mechanism and does not touch that gate.
7. The `.`/print backend carve-out on `IrType::Enum(BOOL_ENUM_ID)` (drives Ruling 3).
8. `branch`'s condition is `Type::U32`, not `Type::BOOL` (`src/check/terms.rs:1503`); the
   logical operators (`and`/`or`/`xor`/`not`) and every comparison *result* are the
   `Bool`-typed sites in the ~80-site migration.

**OQ1 (resolved by the brief's probe):** the ~80 sites stay a *single value*, not
per-module threading. `Type::BOOL`/`from_name("bool")` stop being a compile-time
`const EnumId(0)` and become one **build-time-resolved** value (bool's slot is now
discovery-order-dependent), looked up once after assembly and read from the check/backend
context instead of a `const`. No new per-module parameter is threaded through
`check_term`/`poly_term`/the operator table.

**OQ4 (resolved by finding 8):** no bootstrap problem. `core::bool` imports only
`intrinsics` (`branch`/`tag`, both `U32`-typed), depends on nothing else, and needs no
boolean type in scope to be checked, so it resolves in whatever order the corpus already
resolves imports; no special first-check handling.

## New finding (beyond the brief)

**F-new: the injection removal shifts every single-file user enum from index ≥1 to index
0**, breaking a body of unit tests that hardcode "user enum at index 1" /
`enums.len() == N+1` / `IrType::Enum(BOOL_ENUM_ID)`. The brief's finding 4 counts the ~80
`Type::BOOL` *readers* but undercounts this index-shift test surface. Confirmed sites:
`src/parser.rs:4860` (`module.enums.len() == 2`, `enums[1]` is `Shape`),
`src/ir/types.rs:596-601` (`ir_type_of(Type::BOOL) == IrType::Enum(BOOL_ENUM_ID)`),
`src/ir/layout.rs:980` ("Shape is enum 1"), `src/ir/func_builder/quotation.rs:758` and
`src/ir/func_builder/calls.rs:1221`/`1243` (assert `IrType::Enum(BOOL_ENUM_ID)`), plus the
`src/ir/destructors` comments. These flip together and **must be migrated in the same
atomic phase** that removes the injection, or that phase is not green.

No *shipping-code* slot-0 assumption survives beyond the two injection sites, the REPL
`remap_type`/`splice_import` arms (already flagged), and these tests: `enum_base` in
`assemble_module` is offset-relative (`enum_base.push(enums.len())`), so it self-corrects
once the injected head is gone.

## Rulings

### R1 — a boolean `static:` initializer REQUIRES `core::bool` in scope

A `static: b bool = true ;` requires the enclosing module to import `core::bool` (directly
or via a re-exporting hub). This is **not** a new initializer-specific import check: it
falls out of ordinary type-name resolution. Once `Type::from_name`'s `name == "bool"`
special-case (`src/ast.rs:1866-1868`) is removed, the static's declared type annotation
`bool` resolves through `resolve_type_name_in_module` / `find_type_in_module`
(`src/ast.rs:319`/`343`), which finds the imported `core::bool` enum **only if imported**;
without the import it is a located `unknown type bool` **at the annotation**, and the
`StaticInit::Bool` initializer branch is never reached.

*Why this way:* P8.S2's "nothing resolves without an import". A body `true` is `unknown
word` without an import; letting a `static:` `true`/`false` resolve for free would make the
same spelling resolve in one grammar position and not another purely because statics skip
the checker, which is exactly the implicit-because-provably-safe convenience that loses the
tie in this project. Because the static path has no checker pass to resolve a call against
(finding 3), the *type annotation* is the correct, already-existing gate rather than an
invented parse-time import check for the initializer body.

*Mechanism note:* the `StaticInit::Bool` branch's `ty == Type::BOOL` comparison must move
off the deleted const to a check against the build-resolved bool enum (match the enum's
`name_static == "bool"`, or compare against the resolved id threaded into the static
parse). This migration is forced into phase 2 (it will not compile otherwise).

### R2 — the REPL auto-seeds `core::bool` by splicing `lib/bool.sth` by path

At `Session::new`, splice `lib/bool.sth` by filesystem path, preserving today's no-import
`true`/`false`/`:stack`-render UX.

*Why not require an explicit import:* the REPL **cannot** resolve a package-name import at
all (`import: core::bool ;` errors "the REPL cannot resolve a module-name import yet"), so
requiring it is not a UX regression, it is impossible: `true`/`false` would be unusable in
the REPL until a separate capability lands, a hard regression from today's working bare
`true`. Auto-seeding replaces the *existing* startup seed (`Session::new` already injects
`bool_enum_decl()` + `bool_print_word_def()`); converting that in-memory injection to a
path-splice of the real `lib/bool.sth` is strictly more honest (single source of truth,
dogfoods the real source) and lets us delete `bool_enum_decl`/`bool_print_word_def`. This
is the REPL's prelude-equivalent, not new hidden magic.

*Residual risk:* the REPL needs a filesystem path to `lib/bool.sth` (lib-dir discovery). If
the REPL has no robust lib-root, phase 2 must establish one (or reuse the path splice
already used for quoted-path sibling imports).

### R3 — DROP the `$boolstrs` backend fast path

Delete the `IrType::Enum(BOOL_ENUM_ID)` Print arm (`src/backend/qbe.rs:1239`), the
`$boolstrs`/`$true_str`/`$false_str` data emission (`src/backend/qbe.rs:82`), and the
hand-built-IR unit test `emit_print_on_bool_indexes_boolstrs_via_sfmt`
(`src/backend/qbe.rs:1778`).

*Why:* source-level `.` on a bool routes through `bool_print_word_def` (`src/ast.rs:948`), a
library `.` overload that eliminates over `True`/`False` and prints the literal string via
`str`'s `.` — it never emits `Instr::Print` on a bool value. `printable_types()`
(`src/check/builtins.rs:121`) excludes bool, so no builtin `.` row targets a bool either.
The Print arm and `$boolstrs` data are therefore **already unreachable from source**, kept
alive only by the one hand-built-IR test. The arm must change regardless (it can no longer
be a `const` pattern once `BOOL_ENUM_ID` stops being a const); keeping it would convert
dead const-pattern code into a dead runtime-id comparison — pure cost, zero benefit, since
the general mechanism (source eliminator + `str` print) is what real programs already use.

*Gate (mutation-test discipline):* phase 1 must first probe-confirm unreachability (build a
`true .` program, dump the IL, assert no `$boolstrs`/`$true_str`/`$false_str` symbol) before
deleting.

### R4 — OQ1: `Type::BOOL` becomes a build-time-resolved read, one value per build

`Type::from_name("bool")`'s special-case is deleted; every `Type::BOOL`/`BOOL_ENUM_ID`
const reader becomes a read of a single build-resolved bool enum id/type carried in the
check/backend context (looked up once after assembly). Not per-module. Any static table
that bakes in `Type::BOOL` (`BUILTIN_TABLE`, the backend `IrType::Enum(BOOL_ENUM_ID)`
match) gets the same const-to-resolved-read rewrite.

### R5 — F-new: migrate the index-shift tests in the atomic phase

Every unit test asserting a fixed enum index / `enums.len()` off-by-one / `BOOL_ENUM_ID`
(the F-new sites) is migrated in phase 2 alongside the injection removal, since user enums
move to index 0 the moment the injection is gone.

## Implementation

CLAUDE.md per-phase-green rule holds: each phase's own goldens pass at its own exit. The
core switch is **irreducibly atomic** (phase 2), for the same reason P8.S2's phase 3 was:
there is no honest pre-stage. The const→resolved plumbing cannot land ahead of the switch
(`from_name` cannot consult a registry at parse time, and an unused context `bool_id` field
is clippy-fatal), and the REPL cannot be split off (it references `bool_enum_decl`, which
phase 2 deletes; leaving it as dead code fails clippy). Only the genuinely-independent
dead-code drop (R3) separates cleanly, and it lands first to shrink phase 2 by one
`BOOL_ENUM_ID` const consumer.

### Phase 1 — drop the dead `$boolstrs` fast path (R3)

- Probe first: build a `true .` program, dump the IL, assert no `$boolstrs`/`$true_str`/
  `$false_str` and that bool print goes through the `bool_print_word_def` eliminator.
- Delete the `IrType::Bool | IrType::Enum(BOOL_ENUM_ID)` Print arm, the `$boolstrs` data
  emission, and the `emit_print_on_bool_indexes_boolstrs_via_sfmt` test.
- Independent of phase 2; green on its own (BOOL_ENUM_ID is still a const here, so the
  deleted arm is still a valid pattern until removed).

### Phase 2 — the atomic global switch (R1 behaviour, R2, R4, R5)

Source + resolution:

- Add `type: bool | False | True ;` to `lib/bool.sth` (name `bool`, variants `False` then
  `True`, matching current discriminants).
- Move the bool `.` print word into `lib/bool.sth` as a source `: . ( bool -- ) ...`
  overload (the shape `bool_print_word_def` builds); delete `bool_print_word_def`.
- Extend `core::prelude` to import and re-export `bool True False` (and the bool `.`
  overload) so `import: core::prelude * ;` keeps every example compiling.

Injection removal:

- Delete `enums = vec![bool_enum_decl()]` at both `src/parser.rs:518` and
  `src/driver.rs:296`, and the `BOOL_ENUM_ID.index() + 1 + idx` rebase at
  `src/parser.rs:539` (user enums now start at index 0).
- Delete `bool_enum_decl()` and `BOOL_ENUM_ID`; remove `Type::from_name`'s
  `name == "bool"` special-case.

Const→resolved rewrite (R4):

- Replace every `Type::BOOL`/`BOOL_ENUM_ID` reader (checker operator/builtin tables,
  `check/engine.rs`, `check/declarations.rs`, `check/poly.rs`, `backend/qbe.rs` lowering,
  `ir/types.rs`) with a read of the single build-resolved bool enum id/type from context.

Static gate (R1):

- Migrate `StaticInit::Bool`'s `ty == Type::BOOL` comparison off the deleted const; the
  boolean-static-requires-import behaviour then falls out of the removed `from_name`
  special-case.

REPL migration (R2):

- `Session::new`: replace the `bool_enum_decl()`/`bool_print_word_def()` seed with a splice
  of `lib/bool.sth` by path.
- Delete `remap_type`'s `id == BOOL_ENUM_ID => Type::BOOL` arm (`src/repl.rs:214`).
- Delete `splice_import`'s `.skip(1)` bool-dedup (`src/repl.rs:2119`).
- Migrate `format_stack`'s `_ if *ty == Type::BOOL` render arm to key on the session's
  resolved bool id (not delete: `:stack` must still print `true`/`false`).

Test migration (R5): update every F-new index-assuming test.

Green at exit: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`, every
`examples/*.sth` builds, and a REPL smoke (`true`, `true .`, `:stack`) works with no import
written.

*Residual risk to watch in phase 2:* re-exporting the bool `.` **operator overload** across
modules exercises the operator-overload cross-module scoping machinery, which has had
scoping/mangling gaps historically. If re-exporting the overload proves to need its own
slice, the phase-2 fallback is to keep the bool `.` print in `core::bool` and require
importing it (consistent with the rest of the slice) rather than re-exporting it through
prelude — but the default is to re-export it so `import: core::prelude *` stays sufficient.

## Tests

Phase 1:

- `qbe` probe/unit: a source `true .` build emits no `$boolstrs`/`$true_str`/`$false_str`
  and prints via the eliminator (the deletion's justification, kept as a regression guard).

Phase 2 (goldens + unit):

- **G1** a program that never imports `core::bool` (nor a hub) and writes `true` gets a
  located `unknown word True`/`unknown type bool` (per grammar position) — bool no longer
  ambient.
- **G2** a program with `import: core::bool ;` (or `import: core::prelude * ;`) constructs
  `True`/`False`, uses `if`/`unless`, prints `true`/`false` via `.`, and runs. (`gcd.sth`
  and `factorial.sth` continue to pass unchanged via prelude.)
- **G3 (R1)** `static: b bool = true ;` **without** `core::bool` in scope → located
  `unknown type bool` at the annotation; **with** it → builds and the static holds `true`.
- **G4 (R5)** unit: a single-file `type: Shape | ... ;` now lands at enum index 0
  (`enums.len() == 1`), replacing the old index-1 assertions.
- **REPL** smoke (R2): first-line `true`, `true .` prints `true`, `:stack` renders
  `true`/`false`, and a session `type: Color | Red | Green ;` still constructs/eliminates
  across lines (bool no longer shadows slot 0).
- **Mutation checks:** delete R1's type-annotation gate → G3's without-import case must
  fail; restore the `$boolstrs` arm without a caller → phase-1 probe must still show it
  unreached (documents deadness).

## Out of scope

- Renaming the type from `bool` to `Bool` (corpus-wide signature migration; unrelated to
  removing the injection).
- Any change to `branch`/`tag`'s own status as `intrinsics`-gated compiler intrinsics, or
  to the `and`/`or`/`xor`/comparison intrinsics' gating or spellings.
- Any new syntax: `type: bool | False | True ;` is the existing enum grammar.
- Teaching the REPL to resolve package-name imports (`import: core::bool ;` in a session
  stays unsupported; R2 works around it by path-splice).

## Exit criteria

1. `bool_enum_decl`, `BOOL_ENUM_ID`, `bool_print_word_def`, and the `from_name("bool")`
   special-case no longer exist; neither injection site remains.
2. `bool`/`True`/`False`/`true`/`false` resolve only through an import (directly or via a
   hub); G1 and G2 pass.
3. A boolean `static:` requires `core::bool` in scope (G3), enforced by type-annotation
   resolution.
4. The REPL auto-seeds `core::bool` by path splice; the smoke test passes with no import
   written.
5. The `$boolstrs` fast path and its test are gone; bool prints through the eliminator.
6. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green at each
   phase's exit, and every `examples/*.sth` builds.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "R3: probe-confirm the backend `$boolstrs` fast path is unreachable from source (source `true .` routes through the `bool_print_word_def` eliminator + `str` print; `printable_types()` excludes bool), then delete the `IrType::Bool | IrType::Enum(BOOL_ENUM_ID)` Print arm (qbe.rs:1239), the `$boolstrs`/`$true_str`/`$false_str` data (qbe.rs:82), and the `emit_print_on_bool_indexes_boolstrs_via_sfmt` unit test (qbe.rs:1778). Independent of phase 2 and green on its own; shrinks phase 2 by one BOOL_ENUM_ID const consumer. Adds the probe as a regression guard.",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Atomic global switch (irreducible like P8.S2 phase 3, no honest pre-stage). Source+resolution: add `type: bool | False | True ;` to lib/bool.sth, move the bool `.` print in as a source overload (delete bool_print_word_def), extend core::prelude to re-export `bool True False` and the `.`. Injection removal: delete the vec![bool_enum_decl()] seed at parser.rs:518 and driver.rs:296, the +1 rebase at parser.rs:539, and bool_enum_decl/BOOL_ENUM_ID; remove Type::from_name's `bool` special-case. R4 const->resolved: rewrite every Type::BOOL/BOOL_ENUM_ID reader (checker tables, engine/declarations/poly, backend qbe, ir/types) to read one build-resolved bool id/type from context. R1 static: migrate StaticInit::Bool's `ty == Type::BOOL` comparison off the const so a bool static requires core::bool via type-annotation resolution. R2 REPL: Session::new splices lib/bool.sth by path, delete remap_type's BOOL arm (repl.rs:214), delete splice_import's skip(1) (repl.rs:2119), migrate format_stack's render arm (repl.rs:~633) to the resolved id. R5: migrate every enum-index-shift unit test (parser.rs:4860, ir/types.rs:596-601, ir/layout.rs:980, ir/func_builder quotation.rs:758 & calls.rs:1221/1243). Goldens G1-G4 + REPL smoke + mutation checks. Watch the operator-overload cross-module re-export risk; fallback is import-not-reexport the bool `.`.",
      "effort": "L",
      "difficulty": "hard"
    }
  ]
}
```
