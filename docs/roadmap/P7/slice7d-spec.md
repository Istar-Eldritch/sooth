# Phase 7 Slice 7d: retire the compiler-intrinsic `.` onto `hosted::show`

**Status:** Done
**Discovery:** [slice7d-dot-hosted-brief.md](./slice7d-dot-hosted-brief.md) (with its
260830 addendum), [slice7d-census.md](./slice7d-census.md) (migration surface), and
[slice7d-probes.md](./slice7d-probes.md) (compile-probe round) — kept as historical
evidence, not re-verified here. Predecessor: [slice7c-spec.md](./slice7c-spec.md)
(`core::show`'s `Show`/`Write` pair this slice prints through).

## Problem

`.` was a compiler-injected, name-dispatched builtin — `BUILTIN_TABLE` rows, a
hand-written `check_operator` arm, and an `Instr::Print` instruction the QBE backend
lowered straight to libc `printf` — invisible to the layer system every other hosted
capability reaches through `depends:`. A program that typed `42 .` into existence
imported nothing, which is exactly the implicit behavior the rest of Sooth refuses.
Meanwhile S7c had already built a real, layered printing vocabulary (`Show` formats a
value into a `StrBuf`, `Write` flushes it to a sink, `write(2)`-ordered rather than
buffered-stdio) that made the intrinsic redundant. Keeping `.` cost two things: a
permanent hole in the layer system, and a variadic-`printf` codegen path in the
compiler duplicating what the library now did.

## What landed

- **Per-type concrete dots, not a generic word.** The generic
  `: . ['T: Show] ( 'T -- )` parses and registers but its body is unwriteable: a
  locally built `&!StrBuf` doesn't unify with a poly callee's declared `&!StrBuf`
  (the parser's Slice-13 R-A4 fold makes declared refs `PolyType::Concrete(Type::Ref(…))`
  while local borrows are native `PolyType::Ref`), and field accessors/`+!` are located
  errors in generic bodies. `lib/hosted/show.sth` instead lands one concrete
  `: . ( T -- )` per printable type, each delegating through a distinctly named
  private helper (bare intra-module calls across same-arity overloads don't resolve).
  The generic dot is deferred, not abandoned — see [Deferred](#deferred).
- **`.` stays in `BUILTIN_WORDS`** even though it is excluded from
  `is_name_dispatched_builtin` and `is_operator_dispatch_name`: self-tail-call
  detection, the extern-redeclaration check, and the local-binding collision checks
  still need to recognize it as a builtin-shaped name. This is the one place the
  delete list stops short of a full delete.
- **`core::bool`'s `.` overload is deleted, not migrated.** Layering forces it: post-
  retirement, the overload body's inner str `.` call can't resolve from the core
  layer. Accepted consequence: `True .` now prints `true\n` through `Show for Bool`
  (lowercase) instead of the library overload's `True\n` — matching what the book
  already documented; the overload was the deviation.
- **Newline is a second `write(2)`, not an in-buffer append.** Numeric and `Bool`
  dots append `"\n"` as a second write through the str path; the read-modify-write
  append over one locally built `StrBuf` hits borrow-alias records that block the
  later flush borrow. All output is syscall-ordered, which incidentally removes the
  buffered-stdio interleaving hazard.
- **`Show` widened to the integer tower** (`u8/u16/u32/u64/i8/i16/i32`) over the
  existing `append-digits` path; signed impls must widen to i64 before the sign test
  (a bare `n 0 lt` on a narrower type resolves the literal ambiguously and errors).
  `str`/`cstr` stay out of `Show` — concrete dots only.
- **`str` dots are len-honest through interior NULs; `cstr` dots are strlen-bound.**
  The str dot writes all `len` bytes (Sooth's `len` counts embedded NULs, unlike C's
  `%.*s`/`printf`), a deliberate behavior change from today's printf-derived
  truncation. The cstr dot keeps the old strlen-bound semantics via its own
  `extern: sys-strlen` — `len` doesn't accept `cstr` (precedent: `examples/strings.sth`).
- **Floats print through hosted `snprintf`**, gated on an in-slice ABI fix (below).
  Both new externs (`write`, `snprintf`, `strlen`) live in `hosted::show` itself, not
  `hosted::libc` — the module that calls a C symbol declares its own binding to it;
  multiple Sooth bindings to one C symbol are fine since the declared C symbol is what
  links.
- **Migration is program-wide, atomic, no compatibility shim.** Every printing
  program now needs `depends: hosted` and `import: hosted::show | . | ;`. The test
  harness derives this for fixture-based tests the same way it already derived
  `core::bool`'s one-hop import; everything else (42 examples, library files,
  raw-written scratch programs, three golden sets) migrated by hand. There was no
  intermediate state where the tree was green with `.` half-migrated — the compiler
  deletions, the library, and the migration landed as one commit.
- **The R17 trace/dot split-stream reorder.** Backend-internal trace lines
  (`SOOTH_TRACE`) still ride buffered `printf` and flush at exit; dots now go out via
  unbuffered `write(2)` immediately. Programs mixing trace output with dots therefore
  reorder relative to before — an intended-to-change output delta, not a regression —
  and 31 goldens were migrated to the new order.

### The f64-argument ABI fix (phase 1, R14)

A user `extern:` declaring an `f64` parameter didn't deliver the double to the C
callee: the backend emitted user extern calls as fully-fixed QBE calls with no `...`
marker, so QBE emitted no `%al` setup, and whether a variadic callee spilled the xmm
register holding the double was undefined behavior (the previously observed "prints
0" was luck, not a guarantee). Fix: `Arity` gained a `CallKind` (`Word` | `Extern`)
threaded from extern registration through lowering to the backend, and the backend
spells `Extern` calls in QBE's all-args-variadic form (`call $sym(..., args…)`) while
user-word calls keep the fixed spelling. `snprintf` (an f64-taking variadic) and
`write`/`strlen` (non-variadic) all run correctly under the variadic spelling.

**Caveat, recorded not resolved:** this spelling is verified equivalent to the fixed
form only on amd64_sysv, arm64, and rv64 QBE, which all emit identical code for it
today. `arm64_apple`'s ABI passes variadic arguments on the stack, where the same
marker would misregister a non-variadic callee's arguments — no `-t` target selection
rides on this fix, so the caveat is inert until Sooth targets `arm64_apple`.

### R18 — poly-body verification

A required check, not a design choice: the migrated corpus was re-checked for any
`.` call inside a poly body (none exist — `expect-eq` never prints values). Had one
existed, the generic-dot checker fix would have been pulled into this slice instead
of deferred.

## Deferred

- **The generic `: . ['T: Show] ( 'T -- )`.** Blocked on two checker gaps tracked
  under **P7.S3w**: the parser's declared-ref fold (`PolyType::Concrete(Type::Ref(…))`
  vs. native `PolyType::Ref`) and poly-body field-accessor/`+!` support. Concrete dots
  are the load-bearing shape until P7.S3w lands.
- **Diagnostics.** The silently-ignored stale `import: intrinsics | . | ;`, the
  hintless `unknown word '.'`, and the `no overload of '.'` message naming no fix are
  left as a cross-cutting diagnostics track, alongside S8's unsatisfied-`Ord`
  attribution.
- **`arm64_apple` target selection.** See the ABI-fix caveat above — not started, and
  nothing in this slice depends on it being resolved.
- **libm linking for user programs** and a pure-Sooth `%g` renderer as a fallback to
  the `snprintf` extern — out of scope; the P8 libm control failed to link, and the
  ABI fix made the fallback unnecessary.
- **`hosted::libc` as the externs' home** instead of `hosted::show` — would typecheck
  by inspection but was never compile-verified by the probes, so the in-module shape
  shipped instead. Recorded as a settled default, not a blocker.

## Load-bearing invariants

- `.` stays in `BUILTIN_WORDS` (do not delete it outright — see above).
- The backend's own internal diagnostics (`$oobfmt`/`$subslicefmt`/`$allocfmt`/
  `$freefmt`/`$oomfmt`, trap/OOM/bounds/trace `printf`/`dprintf` calls) are untouched:
  they are panic/trace paths, not user-facing `.`.
- The IR stays backend-neutral: `CallKind` records a fact about the *callee* (a C
  prototype Sooth can't see), not about registers or targets.
- `core::show` stays `no_std` — the new externs live in `hosted::show` only.
- No compatibility shim: nothing resolves `.` without the `hosted::show` import.

## Implementation

- **Phase 1 — `a42e9fc`**: the user-extern f64-argument
  ABI fix. `CallKind { Word, Extern }` threaded through `Arity` to the backend;
  extern calls spelled all-args-variadic.
- **Phase 2 — `fa7c9c7`**: the atomic retirement. All
  builtin `.` surface deleted (with `.` kept in `BUILTIN_WORDS`);
  `lib/hosted/show.sth` added with its 15 concrete dots; `core::show`'s seven widened
  `Show` impls; `core::bool`'s overload deleted; the harness's printing-import rule;
  the program-wide migration (examples, tests, `SPY_DEF` split, regenerated goldens).
- **Phase 3 — `02b0034`**: docs migration (README, book,
  `DESIGN.md`, roadmap entry marked `[ done ]`).

Commit range from the branch point: `790b81c..02b0034`.
