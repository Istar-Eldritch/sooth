
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

1. **Do the ~80 checker/backend sites that read the global `Type::BOOL` constant
   need to become module-scoped lookups, or can they stay a global constant?**
   `branch`, `tag`, and every comparison operator are genuine compiler intrinsics
   that must type their result as *some* boolean type regardless of the calling
   module's imports — the same way `intrinsics` words work today without per-import
   variance in their own signatures. If `Bool`'s `EnumId` varies per module (each
   module importing `core::bool` gets whatever `EnumId` that module's own registry
   assigns it, via the ordinary cross-module enum-import machinery P8.S1a already
   built), then every one of these ~80 sites needs the *current checking module's*
   resolved `Bool` type, not a fixed constant — otherwise a comparison in module A
   would type its result against module B's registry slot. Needs a probe: does the
   existing cross-module enum resolution (`resolve.rs`'s `exported_origin`/import
   machinery, P8.S2) already give every checker call site cheap access to "the
   concrete `Type` this module's `Bool` name currently resolves to," or does this
   slice need to thread a new parameter through `check_term`/`poly_term`/the operator
   dispatch table?

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

3. **Does the REPL's `BOOL_ENUM_ID`-specific pinning logic actually become
   redundant, or does it need its own migration?** Finding 5 is a plausible read,
   not yet a probe. Before speccing, build a small REPL session that imports a
   *user* two-module enum today (module A declares `type: Color | Red | Green ;`,
   module B does `import: A ;` and constructs `A::Red`) across two session lines and
   confirm the existing session-import machinery already gives `Color` a stable
   cross-line identity the way `BOOL_ENUM_ID`'s special-case code does for `bool`
   today. If it doesn't, this slice inherits a larger, REPL-specific migration cost
   than the checker/backend side does.

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

**Not yet.** OQ1 (whether ~80 call sites need module-scoped resolution or can stay a
global constant) is the load-bearing question that decides this slice's actual size,
and OQ3 (REPL migration cost) could not be answered by reading alone in the time this
brief took — both need a short probe (a real two-module cross-import enum session in
the REPL; a trace through one checker call site to see what's actually available to it
today) before a spec author commits to an approach. Recommend a probe pass on OQ1/OQ3
next, then write the spec once both come back.
