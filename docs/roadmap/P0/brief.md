# Phase 0 brief — codegen spine

Input for spec-writer. Resolves the decisions DESIGN.md left as "illustrative" for the
Phase 0 slice. Read alongside [../DESIGN.md](../DESIGN.md), [../ROADMAP.md](../ROADMAP.md),
and [../CLAUDE.md](../CLAUDE.md). Everything here is scoped to Phase 0 only.

## Goal and exit criteria

Prove the core architectural bet end to end: compile-time virtual stack → backend-
neutral IR → QBE IL → `qbe` → system `cc` → native binary.

**Exit:** `gcd` and `factorial`, written as complete programs with a `main`, compile
and run and print the correct integer. Plus one negative golden: a stack-effect
mismatch produces the expected diagnostic.

## Surface subset (Phase 0 only)

- **Word definition:** `: name ( effect ) | locals | body ;`
- **Stack effect:** `( in... -- out... )`, slots written as bare types (`int`). Phase 0
  has one type, `int`; the checker verifies **arity** (slot count), not richer types
  yet. A slot may carry a name (`a:int`) as caller-facing documentation, but a slot
  bound by `| … |` stays a bare type so a name is never written twice.
- **Locals:** `| a b |` binds the top N stack items left-to-right, matching the effect
  order; the items are consumed. Referenced by name in the body. `int` is `Copy`, so a
  local may be used any number of times. Names live here, not in the effect comment.
- **Literals:** decimal `i64`, optional leading `-`.
- **Builtins:**
  - arithmetic `+ - * mod`, each `( int int -- int )`
  - comparison `= < >`, each `( int int -- int )` returning `1`/`0`
  - stack shuffles `dup ( int -- int int )`, `drop ( int -- )`,
    `swap ( int int -- int int )`, `over ( int int -- int int int )`,
    `rot ( int int int -- int int int )`. Monomorphic and int-only here; `dup`/`drop`
    gain affine meaning in Phase 3 and all of them gain polymorphic signatures in
    Phase 4. They lower to pure stack juggling (reorder/reuse/discard value ids), no IR
    op of their own.
  - print `.` `( int -- )`, lowered to a libc `printf("%ld\n", ...)` FFI call
- **Locals are opt-in:** with the shuffles above, one- and two-value words stay
  point-free (`square` is `dup *`); reach for `| … |` only when shuffling reads worse
  than names (roughly three-plus reused values, like `lerp`).
- **Truth is int (derived default):** since Phase 0 has only `int`, there is no `bool`
  type yet. Comparisons yield `1`/`0`, and `if` pops an `int` and treats nonzero as
  true. `bool` arrives in Phase 2. (Forth-idiomatic; keeps Phase 0 to a single type.)
- **Control flow:** `if ... else ... then`. Consumes one `int`. Sooth has no loop
  keywords by design (iteration is combinators over quotations, arriving Phase 4);
  Phase 0 iterates only via shallow recursion.
- **Recursion:** a word may call itself; the self-call is checked against the word's
  declared effect. No tail-call optimisation in Phase 0 (goldens are shallow; TCO is a
  recorded deferred item, see DESIGN "Open / deferred").
- **Comments:** `\` to end of line. `( ... )` is reserved for the stack effect in the
  `:` header.
- **Entry point:** a word `main ( -- )`. The driver emits a C `main` that calls it.

## IR and codegen notes

- Compile-time virtual stack of typed slots (Phase 0: all `int`). Each word lowers to a
  QBE function taking N `l` (i64) parameters and returning its outputs. Phase 0 words
  return a single `int` or nothing (`main`); multi-value return is deferred (goldens
  don't need it).
- `if/else/then` → QBE blocks + conditional branch; the join point requires equal stack
  depth on both arms (the arity checker enforces this, and a mismatch is a compile
  error, not a silent bug).
- Locals → QBE temporaries.
- `.` → `call $printf(l $fmt, ..., l %v)` with `data $fmt = { b "%ld\n", b 0 }`.
- `Ptr[T]` is not exercised in Phase 0 (no heap/pointers) but the IR type model must
  already represent it as an opaque handle, per the backend-neutral invariant.
- Driver: emit `.ssa`, run `qbe` → `.s`, run `cc .s -o <binary>`.

## Stack-effect checker (Phase 0: arity only)

Simulate the virtual stack through each word body:

- a literal pushes 1; a local binding pops N; a local reference pushes 1;
- each builtin/word applies its known `(in, out)` arity;
- `if/else/then` requires both branches to leave equal net depth, and unifies them;
- a call (including a self-call) uses the callee's declared effect.

The declared effect must equal the computed net effect; a mismatch is a compile error
that names the location and the discrepancy (see the format in DESIGN, "Surface
language").

## Out of scope for Phase 0

Heap, structs/enums, types beyond `int`, the `bool` type, affine/move semantics,
polymorphism, quotations and combinators (iteration; Phase 4), tail-call
optimisation, the REPL, multi-value returns,
WASM, and `no_std` packaging. All arrive in later phases.

## Test plan

- Golden programs: `gcd` and `factorial` (each a complete program with `main`),
  asserting stdout.
- Unit tests per stage (lex / parse / check / lower / emit): happy path + at least one
  error/edge case, per CLAUDE.md.
- One negative golden: a stack-effect mismatch produces the expected diagnostic (the
  error is part of the spec, not just "it fails").
