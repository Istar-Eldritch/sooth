# Phase 7 Slice 3i: `bool` as an ordinary `core::bool` enum (as shipped)

## Goal

The compiler no longer injects a `bool` enum into every program. `bool`, `False`, `True`,
and the `true`/`false` spellings are declared once as ordinary source in `lib/bool.sth`
(`core::bool`) and reach a module through an `import:` like any other type. This closes
P8.S2's "nothing resolves without an `import:`" for the one type it left behind: `bool` was
never a `BUILTIN_WORDS` entry but a second, independent injection mechanism.

## Landed shape

`lib/bool.sth` now holds, over `import: intrinsics i | branch tag drop . | ;` alone:

- `type: bool | False | True ;` (`False` = 0, `True` = 1, both payload-free, so the general
  zero-payload-enum rule lowers a bool to a bare scalar)
- `if` / `unless` (unchanged, over `tag`/`branch`)
- `: . ( bool -- )`, an ordinary library overload eliminating over the two variants and
  delegating to `str`'s still-primitive `.`

Deleted: `bool_enum_decl`, `BOOL_ENUM_ID`, `bool_print_word_def`, `Type::BOOL`,
`Type::from_name`'s `name == "bool"` special-case, both `vec![bool_enum_decl()]` injection
sites (`parser::parse`, `driver::assemble_module`) and the parser's `+1` user-enum rebase.
Single-file user enums now start at index 0.

## Rulings as implemented

**R1 — a boolean `static:` requires `core::bool` in scope.** Not an initializer-specific
check: the declared type annotation `bool` resolves through the module's own imports, so
without the import it is a located `unknown type bool` at the annotation and the
`StaticInit::Bool` branch is never reached. The former `ty == Type::BOOL` comparison is now
`Some(decl.ty) == resolve_bool_type(&module.enums)` in `check_static_decls`
(`src/check/declarations.rs:328`), so a same-named enum that is not the resolved two-variant,
payload-free bool -- whether it carries a payload or merely has a third variant -- is
rejected there too.

**R2 — the REPL seeds `core::bool`, embedded, not path-spliced.** `Session::new` lexes and
parses `include_str!("../lib/bool.sth")` (`src/repl.rs:1133`); `lib/bool.sth` stays the
single source of the declaration without the session needing to know where the library tree
sits relative to the binary. This replaced the plan's filesystem path splice (no lib-root
discovery was needed or added). Only the type and its `.` overload are seeded: a session
imports `if`/`unless` exactly as a file does. An explicit import is not an option, since the
REPL cannot resolve a package-name import at all.

Import folding: `EnumRemap` (`src/repl.rs:~215`) folds an imported closure's own `bool` onto
the session's seeded slot instead of appending a second copy, and the append skips the folded
slot so later enums shift by one less. The fold is keyed on `resolve_bool_type`'s **shape**
test, never on the name, so an imported payload-carrying or three-variant `bool` stays a
distinct type rather than being read at the session bool's width. `remap_type`'s old
`BOOL_ENUM_ID` arm and `splice_import`'s `.skip(1)` bool-dedup are gone; `format_stack`
renders `true`/`false` off the session's resolved `bool_enum`.

**R3 — the `$boolstrs` backend fast path is gone.** Probe-confirmed unreachable from source
first (a `true .` build emits no `$boolstrs`/`$true_str`/`$false_str`), then the
`IrType::Bool | IrType::Enum(BOOL_ENUM_ID)` Print arm, the data emission, and
`emit_print_on_bool_indexes_boolstrs_via_sfmt` were deleted. Source `.` on a bool routes
through the library overload; `printable_types()` excludes bool.

**R4 — one build-resolved bool, not a const and not per-module.**
`resolve_bool_type(&[EnumDecl]) -> Option<Type>` (`src/ast.rs:929`) returns the first enum
named `BOOL_TYPE_NAME` with exactly two payload-free variants. Every former `Type::BOOL` /
`BOOL_ENUM_ID` reader (operator/builtin tables, `check/engine.rs`, `check/declarations.rs`,
`check/poly.rs`, backend lowering, `ir/types.rs`) reads it from the merged registry it
already has; nothing new is threaded through `check_term`/`poly_term`.

The shape test is load-bearing, not decoration: callers that treat a bool as a
register-resident scalar (logical operators, the `extern:` boundary set) rest on it, so a
same-named payload-carrying enum cannot inherit that treatment by naming alone. First match
wins over the whole merged registry: a program declaring its own payload-free `bool` ahead of
`core::bool` in discovery order takes the logical operators with it, yielding a *refused*
`and` (an operand mismatch naming `bool` twice), never a miscompiled one.

**R5 — the index-shift tests migrated in the atomic phase**, plus `src/test_support.rs`'s
`core_bool_enums()` (parses `lib/bool.sth`) as the shared fixture for unit tests that need a
real bool registry.

## Deviation: the prelude hub carries words only

The plan's "extend `core::prelude` to re-export `bool True False` and the `.`" is not
achievable. `lib/prelude.sth` re-exports `if unless False True` only:

- a **type name** resolves against the module that declares it, and
- an **operator overload**'s candidate lookup considers the importing module's direct
  imports,

so neither crosses a hub. A program that spells `bool` in an effect or prints one imports
`core::bool` directly. `lib/cmp.sth` and `lib/combinators.sth` take `import: self::bool ...`;
`examples/array_ctor.sth`, `bool_abi.sth`, `leap.sth`, `poly_if.sth`, `slices.sth` were
updated accordingly.

## Incidental cleanups the switch enabled

- The redundant bool fast path in `check_operator` (`src/check/operators.rs`,
  `src/check/builtins.rs`) is gone.
- The module-0 exemption in the uncalled-overload filter (`src/ir/driver.rs`) is gone.
- Phase 1 also dropped the `$boolstr` literals and pinned the `$strfmt` call pattern
  (`tests/phase3_strings.rs`, `tests/qbe_baseline/*.ssa`).

## Tests (`tests/phase7_slice3i.rs`)

- No import: bare `true` → `unknown word`; `bool` in an effect → `unknown type`.
- The prelude hub carries the constructors but not the type name.
- `import: core::bool` gives type, constructors, branch, and print, and runs.
- Boolean `static:`: without the import → located `unknown type bool`; with it → holds its
  initializer; at a payload-carrying enum named `bool` → error.
- `not` on a payload-carrying / three-variant enum named `bool` → error (the shape test's
  two guards, each mutation-killed).
- REPL: bare `true` on line 1; an imported `core::bool` folds onto the seed; an imported
  enum merely *named* `bool` does not; an enum declared after the folded slot shifts
  correctly; a session enum constructs and eliminates across lines.
- Unit: `resolve_bool_type` finds the declaration at its own position, returns `None` on an
  empty registry, and rejects payload-carrying / three-variant same-named enums.

## Out of scope

Renaming `bool` → `Bool`; `branch`/`tag` and the logical/comparison intrinsics' gating; any
new syntax; teaching the REPL package-name imports.

## Exit criteria (met)

1. `bool_enum_decl`, `BOOL_ENUM_ID`, `bool_print_word_def`, `Type::BOOL`, the
   `from_name("bool")` special-case, and both injection sites no longer exist.
2. `bool`/`True`/`False`/`true`/`false` resolve only through an import (the type name and the
   `.` overload from `core::bool` directly, the constructors and `if`/`unless` also via the
   prelude hub).
3. A boolean `static:` requires `core::bool` in scope, enforced by annotation resolution.
4. The REPL seeds `core::bool` from the embedded `lib/bool.sth`; the smoke tests pass with no
   import written.
5. The `$boolstrs` fast path and its test are gone; bool prints through the eliminator.
6. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green at each phase exit;
   every `examples/*.sth` builds. DESIGN.md states bool's import shape; P7.S3i marked done.
