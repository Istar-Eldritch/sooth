
# Phase 7 Slice 3i: `Bool` as an ordinary enum, not a compiler-injected one (brief)

Delete the compiler-baked `bool` enum injection (`BOOL_ENUM_ID`, `bool_enum_decl()`)
and declare it once as ordinary source in `core::bool`: `type: Bool | False | True ;`.
A module resolves `Bool`, `True`, `False`, and the `true`/`false` literal spellings the
same way it resolves any other imported enum — through `import: core::bool ;` or
`import: prelude ;` — rather than having them seeded into every module's registry
unconditionally. This is P8.S2's own rule ("no word resolves without an `import:`")
finishing the one type it left behind: P8.S2 gated `BUILTIN_WORDS` and deleted the
prelude's word injection, but `bool` was never a `BUILTIN_WORDS` entry — it is a
second, independent injection mechanism P8.S2 didn't touch.

## Recon

1. **`bool_enum_decl()` already produces an ordinary `EnumDecl`.** `src/ast.rs:914-933`
   builds exactly `type: Bool | False | True ;`'s shape (two zero-payload variants,
   `False` at index 0, `True` at index 1) — there is no representation gap between
   what is compiler-injected today and what a user enum declaration already
   expresses. The only special thing is *how* it enters a module's registry.

2. **Injection happens at two independent sites, both unconditional.** `parser::parse`
   (`src/parser.rs:508-510`, single-file path) and `driver::assemble_module`
   (`src/driver.rs:290-293`, multi-file path) both seed `enums = vec![bool_enum_decl()]`
   before any user enum, then rebase every subsequent user `EnumId` by one
   (`src/parser.rs:536-539`: `enums[BOOL_ENUM_ID.index() + 1 + idx]`). Neither site
   consults imports; `bool` is present in every module's registry regardless of
   what that file imports, same shape the deleted prelude injection had for words.

3. **`true`/`false` are already ordinary word calls, not a literal term kind.**
   `src/parser.rs:3708-3716`: `Token::Word(w) if w == "true"` lowers to
   `TermKind::Call("True".to_string())` — the same `TermKind` an ordinary enum
   variant constructor call produces. There is no `TermKind::BoolLit` left (retired
   in Slice 9); `True`/`False` resolve as calls against whatever enum-variant
   constructor words the registry seeded. The literal-keyword-to-call rewrite needs
   no change; what changes is whether `True`/`False` exist to resolve *at all*
   without an import.
   One exception: `src/parser.rs:1761-1764`, a `static:` initializer's `true`/`false`
   parsing branches directly on `ty == Type::BOOL` and builds a distinct
   `StaticInit::Bool(bool)` payload (not a `Call`), since static initializers are
   evaluated at parse time with no checker pass to resolve a call against. This path
   does not go through `env`/import resolution today and needs its own decision (see
   OQ2).

4. **`Type::from_name("bool")` and `Type::BOOL` are the two remaining global-constant
   readers.** `src/ast.rs:1857-1863`: `Type::from_name` special-cases the string
   `"bool"` to return the fixed `Type::Enum(BOOL_ENUM_ID, "bool")` regardless of
   caller or module. `Type::BOOL` is a `pub const` read directly (not through any
   resolution path) at roughly 80 sites across `check/operators.rs` (comparison and
   `and`/`or`/`xor` result typing), `check/builtins.rs` (the `is_copy` table,
   `BUILTIN_TABLE` argument/return types for comparisons), `check/engine.rs`,
   `check/declarations.rs`, `check/poly.rs`, `backend/qbe.rs` (`IrType::Enum(BOOL_ENUM_ID)`
   lowering), and `repl.rs` (session-to-session pinning, detailed in finding 5).
   Every one of these is a *compiler-internal* consumer (a comparison's result type,
   `branch`'s condition type, `tag`'s discriminant) rather than a source-level name
   lookup, so most of them plausibly stay untouched: `branch`/`tag`/comparisons are
   genuine intrinsics and can keep referring to *a* concrete enum type by construction
   — the open question is whether that type is still a global constant or something
   resolved per-module (see OQ1).

5. **The REPL has its own `BOOL_ENUM_ID` pinning logic that assumes global injection.**
   `src/repl.rs:206-214`, `624`, `1078`, `2112-2115`: because every session line is
   parsed as its own single-file module (each minting its own registry via
   `parser::parse`), the REPL normalizes any freshly-parsed `Type::Enum(id, _)` where
   `id == BOOL_ENUM_ID` back to the canonical `Type::BOOL`, and refuses to let a
   session line re-declare `"bool"` a second time. This logic is a direct consequence
   of "every registry has bool at slot 0 unconditionally" — if `Bool` instead arrives
   only via `import: core::bool` like any other cross-module enum, the REPL's existing
   cross-line type-identity machinery for *any* imported enum (used for `import:`
   sessions already, per P8.S1a/S1b) should cover this case too, and the
   `BOOL_ENUM_ID`-specific pinning becomes dead code to delete — but this needs a
   probe against the REPL's actual import-session machinery, not an assumption
   (flagged as OQ3).

6. **`is_gated_intrinsic_name`/`BUILTIN_WORDS` never listed `bool`, `True`, or `False`.**
   `src/check/declarations.rs:63-134`: the P8.S2 intrinsics gate only ever covered
   `BUILTIN_WORDS` (the 40-ish shuffle/arithmetic/`branch`/`tag`/`.`/`fill`/`len`
   names). `bool`/`True`/`False` were never routed through that gate at all — they
   bypass it entirely via the registry-injection mechanism in finding 2, a completely
   separate code path from the intrinsics-visibility check. This slice does not
   touch the intrinsics gate; it removes a different, independent global-visibility
   mechanism.

7. **The `.` operator's `bool` handling is a backend-level carve-out, not a checker
   overload.** `src/backend/qbe.rs:1236-1251` special-cases `IrType::Enum(BOOL_ENUM_ID)`
   in the `.`/print lowering to index the 2-entry `$boolstrs` table (`"false\0"`/
   `"true\0"`) instead of the general enum-variant print path every other enum goes
   through. This is a real, separate carve-out this slice must decide about: either
   keep a backend-level fast path keyed on whatever `EnumId` `core::bool`'s `Bool`
   resolves to per compilation (no representation change, just losing the fixed
   constant), or drop the fast path and let `Bool` print through the same general
   enum-variant mechanism any other two-variant enum already uses. Worth comparing
   the generated code size/perf difference before deciding, not just defaulting to
   "delete the carve-out for uniformity."

8. **`branch`'s condition is typed `Type::U32`, not `Type::BOOL`.** `src/check/terms.rs:1503`
   confirms `branch` consumes the raw unsigned condition flag a comparison primitive
   yields, never a `Bool`-typed value. `and`/`or`/`xor`/`not`, by contrast, are declared
   over `Type::BOOL` operands and results (`src/check/builtins.rs:193-209`). So the two
   intrinsic families are typed differently today: `branch`/`tag` sit below `Bool`
   entirely (a `U32` flag), while the logical operators and every comparison's *result*
   sit at `Bool`. This resolves OQ4's bootstrap worry for `branch`/`tag` themselves (they
   need no `Bool` type in scope at all) but confirms the logical operators do, and are
   therefore squarely inside this slice's ~80-site migration, not a bystander.

## Open questions

1. ~~Do the ~80 checker/backend sites that read the global `Type::BOOL` constant
   need to become module-scoped lookups, or can they stay a global constant?~~
   **Resolved: they stay a single value, not per-module** (probed:
   `docs/roadmap/P7/slice3i-brief.md`'s probe report, `/tmp/s3i-probe-report.md`,
   kept in the resolving session). `assemble_module` (`src/driver.rs:290-296`)
   builds one whole-build merged enum vector; `find_type_in_module`
   (`src/ast.rs:343-380`) resolves a cross-module type reference to the *origin*
   module's index in that one shared vector, never a per-module rebase — confirmed
   by a real two-module build (module `a` declares `Thing`, module `b` imports and
   eliminates it, builds and runs, prints `10`) plus an instrumented `assemble_module`
   id dump showing both modules reference the same `EnumId`. So a `Bool` declared once
   in `core::bool` gets exactly one `EnumId` for the entire build, and the ~80 sites
   need no new per-module parameter threaded through `check_term`/`poly_term`/the
   operator dispatch table. What does change: `Type::BOOL` and
   `Type::from_name("bool")` stop being a compile-time `const EnumId(0)` and become a
   single **build-time-resolved** value (looked up once, after assembly, since bool's
   slot is now discovery-order-dependent rather than fixed) that every site reads from
   the check/backend context instead of a `const`. Any static table currently baking
   in `Type::BOOL`/`BOOL_ENUM_ID` (`BUILTIN_TABLE` in `check/builtins.rs`, the
   backend's `IrType::Enum(BOOL_ENUM_ID)` match in `backend/qbe.rs`) needs this same
   const-to-resolved-read rewrite.

2. **What happens to a `static:` boolean initializer once `true`/`false` require an
   import?** Finding 3's `StaticInit::Bool` path parses `true`/`false` directly
   against `ty == Type::BOOL` with no checker pass and no import awareness (statics
   are parsed before the ordinary body-checking pass runs). Does this path need the
   enclosing module to have `core::bool` imported before a boolean static can be
   declared at all (consistent with the rest of the slice), or is a `static:`
   initializer allowed to keep resolving `true`/`false` unconditionally as a
   parse-time literal spelling, on the theory that a static initializer is closer to
   a raw literal grammar than a call site? This is a real design fork, not a detail —
   rule it explicitly.

3. ~~Does the REPL's `BOOL_ENUM_ID`-specific pinning logic actually become
   redundant, or does it need its own migration?~~ **Resolved: needs its own
   migration, but bounded** (probed, `/tmp/s3i-probe-report.md`). Cross-line identity
   is already stable for any user type today (a session-declared `type: Color | Red
   | Green ;` on line 1 is constructible and eliminable on line 2, prints correctly),
   and a quoted-path import persists across lines too — so `core::bool` does not need
   re-importing per line, no new REPL mechanism required there. But the
   `BOOL_ENUM_ID`-specific code does not fall out for free; four sites need action:
   (1) `remap_type`'s `id == BOOL_ENUM_ID => Type::BOOL` arm (`src/repl.rs:214`) must
   be *deleted* — left in place, it would wrongly force whatever enum lands at an
   imported module's slot 0 to `Type::BOOL`; (2) `splice_import`'s `skip(1)`
   bool-dedup (`src/repl.rs:2119`, which assumes an imported module's slot 0 is always
   the injected bool) must be removed; (3) `format_stack`'s `true`/`false` render arm
   (`src/repl.rs:628`, keyed on `*ty == Type::BOOL`) must be *migrated* to key on the
   session's resolved `Bool` id rather than deleted, or `:stack` regresses to printing
   a raw enum tag instead of `true`/`false`; (4) `Session::new`'s startup seeding
   (`src/repl.rs:1082`, `bool_enum_decl()` plus `bool_print_word_def()`) must be
   converted, and this is the one genuine design fork left for the spec to rule on.
   Today `bool` is the *only* type usable on a session's first line with no import
   written (bare `true` works; `lt`/`if` are `unknown word` per P8.S2's existing
   baseline) — confirmed live. **Rule one of:** (A) auto-seed `core::bool` at session
   startup, preserving today's no-import `true`/`false`/stack-render UX (probe's
   recommendation) — with the wrinkle that the REPL cannot resolve a package-name
   import at all yet (`import: core::bool ;` fails with "the REPL cannot resolve a
   module-name import yet"), so the seed must splice `lib/bool.sth` by path, not by
   package name; or (B) require the user to import it like any other core word,
   consistent with P8.S2's treatment of `if`/comparisons, accepting a `true`/`false`
   UX regression from today's REPL.

4. **Resolved by finding 8: no bootstrap problem for `branch`/`tag` themselves.**
   `branch` consumes a raw `Type::U32` condition flag (`check/terms.rs:1503`), never a
   `Bool`, so it needs no boolean type in scope at all and can stay exactly as
   compiler-intrinsic as it already is. The remaining bootstrap question narrows to
   the logical operators (`and`/`or`/`xor`/`not`, declared over `Type::BOOL` operands)
   and every comparison's result type: do these stay genuine intrinsics that hand back
   *whatever the calling module's `core::bool` import resolves to* (answered by OQ1), or
   does `core::bool` itself need special bootstrap handling to be checked before any
   other file can import it? Plausibly no special handling is needed — `core::bool` is
   an ordinary file with no dependencies of its own, so nothing stops it being checked
   first in whatever order the corpus resolves imports today — but confirm this against
   the actual module-closure discovery/ordering code, not by assumption.

## Out of scope

- Any change to `branch`/`tag`'s own status as compiler intrinsics gated by
  `import: intrinsics ...` — this slice only changes what `Bool` (the *type* their
  results/operands are typed as) resolves against, not whether `branch`/`tag`
  themselves need gating (they already are, per P8.S2).
- Changing the `and`/`or`/`xor`/comparison intrinsics' own gating or spellings.
- Any new syntax. `type: Bool | False | True ;` is the existing enum-declaration
  grammar; this slice is purely about deleting a compiler-side injection and adding
  a `core::bool` source file plus updating every call site that currently reads a
  global constant.

## Ready to spec?

**Yes.** Both load-bearing open questions are resolved by probe (`/tmp/s3i-probe-report.md`,
kept in the resolving session): OQ1 is a single build-time-resolved id, not per-module
threading, so the checker/backend side of this slice is a mechanical const-to-resolved-read
rewrite; OQ3 is a bounded, enumerated REPL migration (two deletions, one render migration,
one startup-seed decision) with exactly one remaining design fork for the spec to rule on
(auto-seed `core::bool` at REPL startup vs. require an explicit import). OQ2 (the `static:`
boolean-initializer path) is unresolved and untouched by this probe pass — the spec must
rule on it directly; it was not blocking enough to warrant its own probe.
