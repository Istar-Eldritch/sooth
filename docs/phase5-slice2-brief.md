# Phase 5 Slice 2: Result and Option (brief)

`Result 'T 'E | Ok val 'T | Err val 'E ;` and `Option 'T | None | Some val 'T ;` as
ordinary generic enums, the convention that fallible words return `Result`, and
`Option['T]` importable from `core`. Slice 1 shipped generic `type:` declarations
(construction, structural dedup, generated words); a follow-up fix
(`3df4846`) shipped clause-style elimination over a generic enum's instantiation. Both
prerequisites are now in place. This slice is mostly about wiring what already exists
into a real, importable library type, not new compiler mechanism — except for one
genuine gap, cross-module generic import, which this brief's recon confirms is unbuilt.

## Recon (measured against the built compiler, 2026-08-16, `main` at `3df4846`)

`cargo test` is green at this HEAD (parallel run may hit the known, pre-existing,
unrelated `/tmp` exec-race flake in some `phase0` goldens; serial is always green).

1. **Branch-on-result codegen needs no new mechanism — confirmed by direct probe, not
   inference.** A fallible word returning `Result[i64 i64]`
   (`dup 0 < ~[ drop drop -1 Err ] ~[ + Ok ] if`) and a clause-style eliminator handling
   both arms (`| Ok |v| v | Err |e| e`) already build and run correctly today:

   ```
   type: Result 'T 'E | Ok val 'T | Err val 'E ;
   : safe_add ( i64 i64 -- Result[i64 i64] )
     dup 0 < ~[ drop drop -1 Err ] ~[ + Ok ] if ;
   : handle ( Result[i64 i64] -- i64 )
     | Ok  |v| v
     | Err |e| e ;
   : main ( -- ) 10 2 safe_add handle . 10 -20 safe_add handle . ;
   ```

   prints `12` then `-1`. ROADMAP's "branch-on-result codegen" line describes an outcome
   the existing generic-enum construct/eliminate machinery already delivers, not a
   distinct feature to build. This slice does not need new IR/checker work for control
   flow around `Result`.

2. **Cross-module generic application is unbuilt and explicitly rejected today, confirmed
   by an existing test.** `parse_generic_application_from_another_module_is_unknown`
   (`src/parser.rs:4121`) asserts `Box[i64]` used in a second module, with `Box`
   declared only in the first, fails with `unknown type \`Box\``. Root cause, read from
   source:
   - `resolve_type_or_apply` (`src/parser.rs:2708`) resolves a generic name via
     `self.generics.find_struct(name, self.module)`/`find_enum` — matched by bare name
     **and** the current module only, with no `q::Base` qualifier handling. Contrast
     `resolve_type_name_in_module` (`src/ast.rs:157`), which already splits a `q::Base`
     name, maps `q` through the import map, and resolves `Base` in the target module —
     that handling exists for concrete types and simply isn't consulted here.
   - Even with qualifier-splitting added, there's a second, more load-bearing gap:
     `GenericTypes` (`src/ast.rs:343`) is one registry shared across a whole
     multi-file closure (`src/driver.rs:207`), but a module's generic `type:` headers
     are registered into it only when *that module's own* `parse_bodies` call reaches
     `parse_generic_typedefs` (called at the top of each module's body parse,
     `src/parser.rs:337`) — i.e., in **discovery order**, not upfront. Concrete
     structs/enums avoid this exact hazard already: `assemble_module`
     (`src/driver.rs:169-179`) runs `prepass_and_register` over **every** file in the
     closure before any file's body parses, so a forward reference to another module's
     type already works regardless of discovery order. Generic headers have no
     equivalent whole-closure pre-pass; they're each visible only after their own
     module has been walked once.
   - Net effect: even a same-order fix restricted to qualifier splitting would leave
     cross-module generic use working only when the exporting module happens to parse
     before the importing one in discovery order — an accidental, fragile pass, not a
     guarantee, unless the pre-pass gap is closed too.

3. **`lib/core.sth`'s prelude injection discards everything but words — `Option` cannot
   live there as an automatic global the way `if`/`bool` do.** `prelude_words()`
   (`src/parser.rs:385`) parses `lib/core.sth` and returns only `.words`, silently
   dropping any `structs`/`enums`/`generic_structs`/`generic_enums` that file might
   declare. So declaring `Option` inside `lib/core.sth` would not make it visible the
   way `if` is visible everywhere with no `import:` — it would need to be reached
   through the ordinary file-import path instead (`import: opt | Option | "..." ;`),
   exactly like any other cross-module type. This matters for OQ2 below: "importable
   from core" most likely means "from a real, ordinary `.sth` file that ships with the
   compiler and is reachable by a relative or resolvable path," not "automatically
   present with no import statement," since the prelude mechanism has no path for a
   type at all.

4. **Import paths are relative-to-the-importing-file, with no library search path.**
   `discover_closure` (`src/driver.rs:78`) resolves every `import:` path by joining it
   to the *importing file's own directory* (`dir.join(&imp.path)`) and canonicalizing —
   there is no compiler-relative or installed-library search path (that's Phase 6's
   dependency-management territory, per ROADMAP). A user program anywhere on disk
   cannot write a single stable `import: opt | Option | "core.sth" ;` today; it would
   need a path relative to wherever the compiler's `lib/` directory happens to sit
   relative to that program, which is not portable. This is a real open question this
   slice must settle, not a detail: either the driver gains a small special-cased
   resolution rule for a `lib/`-shipped file (a new, narrow mechanism), or Option's
   "importable from core" claim is satisfied only for programs inside this repository
   (examples, tests) until Phase 6's package system exists, and that limitation should
   be stated plainly rather than implied.

5. **Multi-variable and single-variable instantiation both already work end-to-end
   through elimination**, not merely construction. Beyond the `Result 'T 'E` probe
   above, the existing `tests/phase5_generic_enum_elimination.rs` and
   `tests/phase5_slice1.rs` suites cover 2-variable enum elimination
   (`two_generic_enum_instantiations_eliminate_independently`) and destructor
   synthesis on a generic instantiation. Nothing about `Option 'T`'s single variable is
   an untested shape; it is the "cheapest second consumer" ROADMAP already calls it,
   and this slice's own goldens should say so by testing it directly rather than
   inferring it from `Result`'s tests.

6. **No `?` sugar remains dropped, unconditionally.** ROADMAP's Phase 5 text already
   settles this ("no DESIGN.md mandate ... addable later without touching this phase's
   exit criteria"). Nothing in this slice reopens it; noted here only so the spec
   doesn't need to re-litigate it.

7. **Attributeless (tuple-style) variant sugar was discussed and explicitly deferred to
   fold into this slice cheaply, not to gate it.** `type: Box 'T ;` constructed as
   `32 Box`, or `type: Result 'V 'E | Ok 'V | Err 'E ;` with no field name, is pure
   parser sugar over the existing named-field mechanism (a field with no declared name
   gets an internal placeholder, or the accessor is simply not generated) — it changes
   no runtime shape and no elimination behavior. Whether it lands in Slice 2 or is
   deferred again is this brief's call to make, not a blocking dependency either way,
   since `Result`/`Option` can ship with the named-field spelling (`val`) already
   proven working in every test so far.

## Decisions (settled here, not reopened by the spec)

1. **No branch-on-result codegen work is scoped in this slice.** Recon 1 confirms it
   already works. The spec should not include a phase for it; it should instead
   golden-test the existing mechanism against `Result`/`Option` specifically (recon 5),
   since that's this slice's actual exit witness, not new lowering work.

2. **Cross-module generic import is in scope and is this slice's real engineering
   work.** Two changes, both confirmed necessary by recon 2:
   - Extend `resolve_type_or_apply` (or an equivalent entry point) to split a `q::Base`
     qualified generic-application name and resolve `q` through the import map, the
     same way `resolve_type_name_in_module` already does for concrete types.
   - Add a whole-closure generic-header pre-pass, run before any module's body parses
     (alongside the existing `prepass_and_register` loop in `assemble_module`), so a
     generic `type:` header is visible to every other module in the closure regardless
     of discovery order — closing the exact gap concrete types don't have.

3. **`Result`/`Option` ship in a real file under `lib/` reachable by ordinary
   `import:`, not through the no-import prelude path.** Recon 3 rules out the prelude
   mechanism (it drops types). Whichever file they live in, `lib/core.sth` or a new
   `lib/result.sth`/`lib/option.sth`, a consuming program reaches them via
   `import: r | Result | "..." ;` like any other cross-module type — the spec must
   name the actual file and settle OQ2 (see below) on how a program outside this
   repository resolves that path.

4. **Attributeless/tuple-style variant sugar is folded into this slice's scope**, since
   it's small (recon 7) and its natural exit witness is `Result`/`Option` themselves —
   proving the sugar against a throwaway type would be doing the work twice. It ships
   as part of this slice's `Result`/`Option` declarations, using the sugar from the
   start rather than the named `val` spelling used in every test so far.

## Open questions for the spec

- **OQ1 — pre-pass shape for cross-module generic headers.** Recon 2's fix needs a
  whole-closure scan of every file's generic `type:` headers before any body parses.
  Does this reuse `prepass_type_decls`'s existing token-stream scanning (extended to
  not skip a generic header, only to register its name/arity/variant-names rather than
  mint anything), or is a separate, generics-specific pre-pass function cleaner given
  the existing one already has a documented reason to skip generics (Slice 1's own
  `continue` at `src/parser.rs:74`, whose comment would need updating either way)?

- **OQ2 — how a program outside this repository resolves `import: ... | "core.sth"`.**
  Recon 4: import paths are relative to the importing file, with no search path. Three
  honest options: (a) state plainly that Option's "importable from `core`" guarantee
  only holds for programs inside this repo (examples/tests) until Phase 6's dependency
  management ships, and defer the general case; (b) add one narrow driver-level rule —
  a `core::`-prefixed or otherwise distinguished import path that resolves against the
  compiler's own `lib/` directory rather than the importing file's directory,
  independent of and much smaller than a full package system; (c) require every
  consuming program to vendor/copy `lib/result.sth` next to itself for now. This is a
  real design fork, not a detail, and should be settled explicitly rather than left to
  the implementer.

- **OQ3 — does `Result`/`Option` live in `lib/core.sth` itself, or a new file?**
  `lib/core.sth` currently holds only `if`/`unless`/comparisons — words with no
  `type:` declarations at all. Since recon 3 shows the prelude path can't carry a type
  regardless of which file it's in, there's no automatic-visibility argument for
  putting `Result`/`Option` in `core.sth` specifically; a dedicated file may be
  clearer file-organization (mirrors how a real stdlib would eventually split
  `Vec`/`Map`/`Option` into their own modules in Phase 6) at the cost of one more file
  to resolve per OQ2. Recommend a narrow reading: whichever the spec picks, name it
  explicitly and treat it as a decision, not an implementation detail discovered mid-
  phase.

- **OQ4 — attributeless variant syntax specifics.** Recon 7 folds tuple-style variants
  in; the spec needs to settle the concrete grammar: is a field with no name simply
  omitted (`type: Box 'T val 'T ;` becomes `type: Box 'T 'T ;`, single positional type
  with no accessor generated at all — matching over-consuming instead), or does it get
  an auto-generated positional accessor name (`Box._0`)? The former is simpler and
  matches how elimination already destructures by position via `| Ok |v|`, not by
  field name; the latter adds an accessor nobody asked for. Recommend the former unless
  the spec finds a concrete need for a positional accessor.

## Out of scope

- `?` short-circuit sugar: dropped from Phase 5 entirely (ROADMAP, recon 6).
- New branch-on-result IR/checker work: none needed (recon 1, decision 1).
- Rebuilding the allocator's OOM trap to return `Option`/`Result`: a future consumer
  (Phase 3 Slice 2's allocator), not a consequence of this slice existing.
- A general package/dependency-management system for resolving library imports outside
  this repository: Phase 6, per ROADMAP's own `docs/dependency-management.md`
  reference. OQ2's option (b), if chosen, is a narrow special case, not this system.
- Bounds, recursion, nested generics, or any other Slice 1 out-of-scope item: still out
  of scope; `Result`/`Option` don't need any of them.

## Sequencing

No gate from any open Phase 4 item. Builds directly on Slice 1
(`999456e`/`d7e36cc`/`1586494`/`24d2732` and the generic-typedef commits it condensed
to) and the elimination fix (`3df4846`). Touches `src/parser.rs` (qualified generic
application resolution, the whole-closure generic-header pre-pass, attributeless
variant parsing if OQ4 lands as proposed), `src/driver.rs` (the pre-pass call site,
alongside the existing concrete-type one), and a new or existing `lib/` file for the
`Result`/`Option` declarations themselves. No changes expected to
`src/check/word_entry.rs` (elimination) or `src/ast.rs`'s instantiation machinery
(construction) — both are recon-confirmed already correct for this slice's shapes.

## Exit

`Result 'T 'E` and `Option 'T` are declared as real library types, each with at least
one golden test that constructs, monomorphizes, and eliminates an instantiation with
a concrete stdout assertion (not merely "it builds") — including a 2-variable
(`Result`) and a 1-variable (`Option`) case, per recon 5. A generic type imported
across a module boundary and applied at the importing module (`v::Vec[i64]`-shaped, or
whatever the actual test type is named) monomorphizes correctly regardless of which
module is discovered first — a direct test of decision 2's fix, not an inferred
consequence. No exception/unwind path exists anywhere in the compiler (already true;
this slice must not introduce one). All pre-existing Slice 1 and elimination tests
continue to pass unchanged.

## Ready to spec?

**Yes, with four open questions handed to the spec.** OQ2 and OQ3 are the most
consequential — they decide what "importable from `core`" actually means operationally
and where the file lives — and should be settled before implementation starts, not
discovered mid-phase, since they affect where every other decision in the spec points.
OQ1 and OQ4 are contained, mechanical questions with an obvious narrow answer each.
Recon's main finding is that this slice is smaller than ROADMAP's phrasing implies:
branch-on-result codegen is already delivered, and the real unbuilt piece is
cross-module generic import, which existing concrete-type machinery already models
closely enough to copy.
