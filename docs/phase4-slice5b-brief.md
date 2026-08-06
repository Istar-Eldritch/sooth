# Phase 4 Slice 5b — REPL imports (brief)

Slice 5a gave the native path `import:`/`export:`: a file is a compilation unit, one
shared pre-pass resolves the whole import closure before any body parses, and
`export:` makes a type or word transparently crossable. The REPL rejects `import:`
today with a located `` `import:` is not supported at the REPL yet `` error (R23/D7),
deliberately, so no phase ships a degraded import path. This slice makes it work.

## Recon: what already exists (measured, not assumed)

**1. Bulk-compiling a module is a small extension of the REPL's existing per-line
path, not new machinery.** `Session::eval_def` (`src/repl.rs:923`) already does: lower
one word's body to an `ir::Func` via `ir::lower_word`, append struct/enum destructor
glue via `ir::synthesize_aggregate_destructors`, wrap in one `IrModule`,
`backend::qbe::emit`, then `driver::compile_so` (`src/driver.rs:303`) — which takes
arbitrary QBE SSA text and produces a `.so`, with no assumption that it came from one
word. Compiling *N* words (an imported file's whole word set) plus their destructor
glue into one `IrModule` and one `.so` is the same call sequence with a longer
`funcs` vector, not a new compilation strategy.

**2. The import-closure resolution the native path built is directly reusable.**
`driver::discover_closure`/`assemble_module` (`src/driver.rs:71,151`) parse an import
graph rooted at a file, dedupe by canonical path, reject cycles, run one shared
pre-pass into one shared registry set, then `resolve::resolve_modules`
(`src/resolve.rs:171`) rewrites bodies and mangles decl names by owning module. None
of this is native-specific; it produces a checked `Module` (structs, enums, words,
each carrying a `module: u32` tag) that the REPL can bulk-lower exactly as recon #1
describes, once it has one for the imported file.

**3. The "frozen callee generation, no late binding" convention (DESIGN.md, decided)
answers the reload-vs-frozen question this slice was split off to answer, for free.**
Every REPL word today is frozen at whichever generation of its callees existed when
it compiled; redefining a word never retroactively changes an already-compiled
caller. Treating a re-run `import:` line as an ordinary redefinition event — mint new
generations for everything the fresh closure defines, existing callers stay bound to
whichever generation they already resolved — needs no new rule, only applying the
existing one to a batch of names instead of one.

**4. A real, previously-unflagged edge case: `main` collides across modules today,
native path included.** `mangle` (`src/resolve.rs:32`) exempts `main` (and `drop`)
from mangling; `check_main_effect` (`src/check.rs:2192`) finds the *first* word
literally named `main` in the whole module's word list, with no check that only one
module declared one. Slice 5a shipped with this latent (a library file has no reason
to declare `main`, so nothing surfaced it); this slice makes it a live risk, since an
imported file's stray `main` would otherwise sit in the checked `Module` unrejected.

**5. Registry growth has no precedent to measure against.** `session.structs`/
`session.enums`/`session.arrays` are flat, append-only, positionally-indexed
(`StructId = index`) so an existing carried value or `drop_overloads` entry keeps
meaning what it meant when minted. Native never re-parses a file twice in one
process; a REPL session re-running the same `import:` line (edited or not) would, on
the "reload is an ordinary redefinition" model, append a fresh batch of struct/enum
entries every time. There is no size cap or dedup-by-content anywhere in the session
today (redefining an ordinary word doesn't dedup either), so this is consistent with
the existing model, just newly visible at import-batch scale.

## Decided (locked, one at a time)

**D1. Registry growth on re-import is accepted, not deduped.** Re-running an
`import:` line mints a fresh batch of struct/enum/array entries every time, exactly
as a redefined word mints a fresh generation every time. No content-hash dedup, no
cap. Matches the dumb-default philosophy 5a already used for import collisions
(error at the second, no precedence) — simplicity over cleverness until a real
session hits a real problem.

**D2. Selective import (`import: q | a b | "path.sth" ;`) ships at full parity with
native.** Same additive-to-the-qualifier semantics, same collision rule (error at the
second, naming both sources). Not a REPL-only reduced form: it is the same
mangle-and-splice mechanism recon #1/#2 already cover, not new design.

**D3. A REPL session overriding an imported type's `drop` is out of scope for this
slice.** Native's rule ("disposal crosses the export boundary for free, no override
needed") stays as-is. Whether a session can *later* type a new `: drop` line naming
a struct that arrived via import is deferred to whoever asks for it; this slice does
not design for the hypothetical.

**D4. An imported file declaring `main` is a located rejection, at import time.**
Turns the latent native-path collision (recon #4) into a diagnostic naming the file
and the word, rather than leaving a landmine for whichever module happens to parse
second. Native's own exposure to this stays unfixed (out of scope: a library
declaring `main` is not a REPL-only risk, but this slice only has to answer it for
the path it's building); note it on ROADMAP for whoever picks up the native side.

## Open questions the spec must answer

- **Qualifier rebinding.** Re-running `import: q "same.sth" ;` is the reload case
  (D-covered above). What does `import: q "different.sth" ;` do when `q` is already
  bound? Likely: treated identically (rebind the qualifier, mint new generations for
  whatever the new closure defines), with no special "you changed the target" error —
  but the spec must say so plainly, including what happens to a call already bound to
  `q::oldname` if `different.sth` doesn't define it (a frozen caller keeps working
  under `RTLD_GLOBAL`; a *new* `q::oldname` reference after rebinding is `unknown`,
  not a stale hit on the old file).
- **Path resolution frame of reference.** Native resolves `import:`'s path relative to
  the *importing file*. The REPL has no file; the natural answer is the process's
  current working directory, but the spec must state it as a rule, not leave it
  implicit.
- **Transitive re-export stays closed, consistent with 5a's declined re-exports.** If
  an imported file itself imports a third file, that third file's names stay
  invisible to the REPL session unless the imported file re-exports them (which 5a
  doesn't support either) — no new rule needed, but the spec should say this
  explicitly rather than let it be discovered by a failing test.
- **Diagnostics fall out mostly for free.** Once `eval_line`'s pre-parse rejection
  (`src/repl.rs:499`) is removed, a malformed `import:`/`export:` at the REPL already
  gets R9's construct-naming located error via the ordinary parser path (`src/parser.rs`,
  fixed this session in `82b7a19`) — confirm this in the spec rather than assume it,
  since the REPL's line-based lexing/parsing entry point may differ from the native
  driver's in some other way recon hasn't hit yet.
- **Dogfood shape.** A REPL-session-style test (piped input) that imports a real
  library file, calls both a qualified word and a qualified accessor, redefines/edits
  the file and re-imports to observe a frozen existing caller alongside a fresh
  resolution, and exercises the `main`-in-a-library rejection (D4) and a selective
  import (D2).

## Out of scope

Everything 5a already deferred (a package/registry layer, re-exports, aliasing an
import to a different local qualifier, generic types crossing files) stays deferred
here too. REPL-side override of an imported type's `drop` (D3). Fixing the `main`
collision on the *native* path (recon #4) — record it on ROADMAP for a later slice,
don't fix it as a drive-by here.
