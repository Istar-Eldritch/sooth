# Phase 7 Slice 2: static storage and global sets (brief)

Module-level static storage — a *place*, not a value: never owned, moved, or dropped,
reached only through a second-class ref, constant-initialised — plus the per-word
**global set** that keeps it honest: which statics a word touches, and in what mode,
inferred within a module and declared on exported words. This is the plain, non-embedded
half of DESIGN.md's Embedded section: no MMIO overlay, no `volatile`, no fixed address, no
ISR. Those stay Phase 9, where their consumer is.

## Recon (measured against the built compiler, 2026-08-16, `main` at `5ee2796`)

`cargo test` is green at this HEAD. Claims below are read from source, not inferred.

1. **Nothing here exists yet.** `grep -rn static` across `src/` turns up only Rust's own
   `static`/`'static` (a lazily-built builtin table, a REPL counter, doc-comment prose) —
   no `static:` declaration keyword, no global-set type, no call-graph fixpoint pass
   anywhere in the tree. This is greenfield within the checker, not an extension of a
   half-built feature.

2. **The top-level declaration dispatch loop is one `while` in `parse_bodies`**
   (`src/parser.rs:363-383`): it peeks the next token and branches on `type:` / `extern:` /
   `import:` / `export:`, falling through to `parse_worddef` otherwise. A `static:` keyword
   slots in here as a fifth arm, parsed once per file the same way `extern:` is, with no
   change to the other four.

3. **The `&`/`&!` borrow sigil already exists and already means "second-class ref to a
   named place," today scoped to bound locals only.** Three sites strip it to recover the
   bare name: `ast::rename_local` (`src/ast.rs:1595-1602`, used when renaming a captured
   local), `resolve::strip_ref_sigil` (`src/resolve.rs:150-161`, used when rewriting an
   import-qualified call), and `check::engine::call_local` (`src/check/engine.rs:557-561`,
   used to decide whether a `Call` denotes a bound local at all). All three do the same
   thing: split `&!`/`&` off the front, look the remainder up as a **local**. None of the
   three has a fallback for "not a bound local, but a declared static" — today an
   unresolved bare name past that point is either a builtin/word call or an error. This is
   the reuse point: the ref-typing, no-escape, and no-store rules already built for `&T`/
   `&!T` (`check/declarations.rs:596`, "a reference cannot be stored") apply unchanged to a
   ref rooted in a static; only name *resolution* needs a third case (local, then static,
   then word-or-builtin).

4. **No call-graph or bottom-up fixpoint infrastructure exists to reuse.** Grepping for
   `CallGraph`/`call_graph`/`fixpoint` across `src/` returns nothing. The nearest relative
   is the existing per-word, single-pass linear/move checker in `check/engine.rs`, which
   walks one word body at a time and does not itself recurse into callees' bodies to
   compute a summary. The global-set computation (a fixpoint over the call graph: a word's
   set is its own direct accesses union every callee's set) is new machinery, not an
   extension of something adjacent. This is the slice's real size, and matches DESIGN.md's
   own framing ("this is what pushes the phase from `[L]` toward `[XL]`").

5. **The `inline` keyword is the one existing precedent for an optional post-name,
   pre-`(` word modifier** (`parser.rs:1328-1334`, `WordDef.declares_inline`) — but it sits
   in a slot that has nothing to do with the stack effect itself. DESIGN.md's own open item
   asks for the global clause to "attach to the stack effect... without turning a one-line
   signature into three," which points inside the parens, not at the `inline` slot: after
   the `--` outputs, before the closing `)`, the same place a declared quotation parameter's
   inner effect already nests grammar (`parse_effect`, `parser.rs:1565-1570`). (Reversed by
   Decision 4 below, which settles on the clause's own trailing keyword slot outside the
   parens, mirroring `inline`, because an in-parens placement reads as part of the stack
   shape.)

6. **`export:` is a separate top-level list, not a per-word flag on `WordDef`**
   (`parser.rs:1429`/`:570`, `ParsedBodies.exports: Vec<Vec<(String, Span)>>`), collected
   once per file and checked later against the word/type name tables built by the
   prepass. Whether a given `WordDef` is exported is a lookup against that list at
   check time, not a bit carried on the word itself — so "declared only where exported"
   means the checker cross-references two structures it doesn't currently cross-reference
   for any other purpose.

## Decisions (settled here, not reopened by the spec)

1. **Declaration form:** `static: NAME Type = <const-expr> ;`, parsed as a fifth arm of
   `parse_bodies`'s dispatch loop (recon 2), symmetric with `extern:`/`type:`. No
   initializer defaults to the type's zero value (`static: COUNT i64 ;` means `0`) — this
   is DESIGN.md's `Preelaborate` tier (constants/zero only, no comptime interpreter, no
   arbitrary startup code). One static per declaration, not a batch form, matching every
   other top-level form in the file.

2. **`<const-expr>` is a literal only** (an integer/bool/string literal, or bare
   zero-value elision per decision 1) — no arithmetic, no referencing another static, no
   struct-literal aggregate initializer. If a real client needs an aggregate static before
   this ships, that is new information for the spec to weigh; the brief's default is the
   narrowest form DESIGN.md's `Preelaborate` tier actually requires.

3. **Access reuses the existing `&`/`&!` sigil grammar verbatim**, extending only *name
   resolution* (recon 3): a bare name after sigil-stripping resolves, in order, to (a) a
   bound local (today's only case, unchanged), (b) a static declared in the accessing
   module, (c) whatever an unresolved name means today (word call or error). No new sigil,
   no new `Type::Ref` variant — a static's ref is exactly `&T`/`&!T` for the static's
   declared `T`, and every existing rule that already governs a borrow's lifetime and
   no-escape behaviour applies unchanged, because those rules are keyed on the type, not on
   what's on the other end of it.

4. **The global clause is its own trailing keyword clause, outside the effect parens,
   after they close and before the body** — not nested inside the stack-effect parens:
   `: NAME ( inputs -- outputs ) global: STATIC mode, STATIC2 mode ; body... ;` (settled
   after review: an in-parens placement, leading or trailing, reads as if the clause were
   part of the stack shape — a return value or an argument — when it is neither; nothing
   is pushed or popped. Sitting in its own slot right after `)`, headed by its own
   colon-suffixed keyword, reads the same way every other declaration keyword in this
   language already does (`type:`/`extern:`/`import:`/`export:`), and mirrors the existing
   `inline` keyword's own bespoke slot rather than growing the effect grammar. This still
   satisfies DESIGN.md's "not three clauses" concern, which is about line count, not
   paren-nesting: the whole declaration stays one line.). Comma-separated, one entry per
   static the word's *own inferred set* contains — see decision 6 for what "own" excludes.
   No new punctuation (no `;`-as-separator): the keyword itself is the unambiguous
   boundary, the same way `--` already separates inputs from outputs with no extra token.
   Explicitly rejected: a fully general "compiler annotation" mechanism with `global:` as
   its first instance — one consumer with a checked, structured payload doesn't justify
   inventing a generic marker syntax; `inline` already shows the pattern this project uses
   for a word-level marker (a narrow, bespoke keyword, not a general mechanism), and a
   plausible second consumer (Phase 9's ISR symbol/section export) has a different enough
   shape (a name/string pair, not a checked NAME-mode list) that generalizing now wouldn't
   even validate the abstraction.

5. **Mode is derived, never hand-authored per access.** A static's mode in a word's global
   set is computed from which sigil reached it transitively: `r` if only `&NAME` occurs
   anywhere in the reachable body, `w` if `&!NAME` ever does (subsuming `r`; there is no
   write-without-read distinction here, matching how `&!` already means "may read or
   write" for an ordinary mutable ref). The global clause only ever *declares the aggregate
   the checker already computed*, it never assigns a mode independently the way Ada's
   `Global => (In => X)` does by hand — one fewer thing to keep in sync, and consistent
   with "global sets are inferred everywhere, declared only at the boundary" (DESIGN.md,
   Embedded).

6. **A word's global set only counts *direct* static names, not refs threaded through as
   ordinary parameters.** `uart-init ( &!Uart -- )` receiving a ref some caller already
   took off `UART` is ordinary second-class-ref parameter passing and contributes nothing
   to `uart-init`'s own global set; only the caller that wrote `&!UART` (naming the static
   directly) accrues `UART` in its set. This matches Ada's `Global` aspect exactly (a
   subprogram needs `Global` only for objects it names, not ones handed in as parameters)
   and is what keeps the set closed and monomorphic under higher-order code the same way
   combinator inlining does for the rest of DESIGN.md's argument (recon 4's fixpoint is
   over *direct-access-or-calls-a-word-that-does*, not over every ref-typed parameter).

7. **Declared vs. checked, by boundary (recon 6):** the global clause may be written on
   any word, but is *mandatory and checked for exact match* only on a word appearing in
   the file's `export:` list; on a private word it is optional, and if present is still
   checked for exact match (cheap consistency, no special-cased "written but ignored"
   state) but its absence is never an error. Match is **exact**, not superset: a declared
   clause missing an entry, naming a wrong mode, or naming a static the inferred set
   doesn't actually contain, are all the same located-error family, naming the static and
   the disagreement.

## Open questions for the spec

- **OQ1 — is a struct-typed static (not just a scalar) in scope for this slice's exit
  case, or does the exit witness use scalars only?** Decision 2 restricts the initializer
  to a bare literal, which types a scalar static trivially but leaves a struct-typed
  static's initializer underspecified (a struct literal is not "a literal" in the
  `Term`/parser sense used elsewhere). Leaning: this slice's exit uses scalar statics only
  (`i64`, `u32`, `bool`); a struct-typed static (needed by `lib/uart_mmio.sth`'s `Uart`
  sketch) is Phase 9's problem once `at <addr>` exists anyway, since a struct static with
  no fixed address and no MMIO consumer has no motivating client yet.

- **OQ2 — does the private-word "declare if you like, checked if present" allowance
  (decision 7) pull its weight, or is it simpler to disallow the clause entirely on a
  non-exported word until someone asks for it?** Leaning towards allowing it: forbidding it
  would need its own rejection error ("global clause not allowed on a private word") which
  is more new surface than just checking it uniformly wherever it appears. But this is a
  genuine judgment call the spec should re-examine once it has the exact-match diagnostic
  shape worked out, not a foregone conclusion.

- **OQ3 — what exactly is the fixpoint's unit of recursion, and does it need
  memoisation/cycle-breaking for mutual recursion?** Recon 4 confirms no existing
  call-graph pass to model this on. A direct word-calls-word cycle (not the mutual-tail-
  recursion case DESIGN.md defers, just an ordinary non-tail mutual pair) needs the
  fixpoint to converge rather than infinitely recurse the first time it's computed. The
  spec needs to work out the actual algorithm (worklist over SCCs, or a visited-set guard
  during a plain recursive walk) — this brief only established *that* a fixpoint is needed
  and *what* it computes, not its termination strategy.

## Out of scope

- MMIO: the `volatile` aspect, `at <addr>` fixed-address overlay, bit-level register
  layout. All Phase 9, where the target-facing consumer lives.
- ISR export (fixed symbol name/section) and the ISR/mainline shared-state wrapper
  (protected-type-shaped or otherwise). Phase 9.
- The link-time-vs-per-module question for an ISR's global set under separate compilation
  (DESIGN.md Open/deferred) — moot here, since this slice has no ISR.
- Any arithmetic, non-literal, or cross-static initializer (decision 2's narrower reading).
- `Copy`-marker interaction beyond "a static is its own carve-out, not routed through
  `Copy`": DESIGN.md already settles this (Embedded section), nothing new for the spec to
  decide.

## Sequencing

No gate from Phase 6 or from Slice 1 (field accessors) either direction — restated from
the phase file, still true. Touches `src/parser.rs` (new `static:` dispatch arm alongside
`type:`/`extern:`/`import:`/`export:`, recon 2; the global clause as its own trailing
keyword clause read right after the effect's closing `)` — outside the parens, mirroring the
`inline` slot, per Decision 4 — with `parse_effect`/`parse_poly_effect` unchanged), `src/ast.rs` (a new top-level static-declaration node; extending
`rename_local`'s sigil-aware name handling, recon 3), `src/resolve.rs` (extending
`strip_ref_sigil`'s fallthrough, recon 3), `src/check/engine.rs` (extending `call_local`'s
resolution, recon 3, plus the new fixpoint pass, recon 4, plus the exact-match diagnostic
at the `export:` boundary, recon 6), and the export-list cross-reference machinery
(recon 6).

## Exit

A module with private static state exports a word whose declared global set the checker
verifies exactly against the inferred one; a mismatch (missing entry, wrong mode, or an
extra entry the inferred set doesn't contain) is a located error naming the static. A
static accessed only through `&`/`&!` behaves exactly as any other ref-typed value under
every existing borrow rule (no escape, no storage) with no new exceptions carved for it.

## Ready to spec?

**Yes, no open question here needs your input.** OQ1 and OQ2 are judgment calls the spec
can settle on its own reading of the corpus (OQ1 in particular is answered the moment the
spec's recon confirms no struct-literal-initializer machinery exists to reuse, the same way
P6 Slice 1's OQ2 resolved itself against existing grammar). OQ3 is pure algorithm design,
not a design fork — the spec works out the fixpoint's termination strategy the same way it
works out any other new pass's internals. Proceeding to spec-write.
