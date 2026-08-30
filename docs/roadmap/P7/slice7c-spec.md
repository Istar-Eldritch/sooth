# Phase 7 Slice 7c: `Show`/`Write`, a sink-generic printing pair

**Status:** Ready for implementation
**Discovery:** `docs/roadmap/P7/slice7c-show-brief.md` (probe round complete, 260829)
**Probe log:** `docs/roadmap/P7/slice7c-probes.md` (23 live compile/run probes + paper recon)

## Problem Statement

S7b `[ done ]` shipped `hosted::testing`'s `expect`/`expect-eq`, but the assertions can
report only a *label*: a failing `expect-eq` cannot print the actual and expected values,
because Sooth has no value-to-text path. `.` is a compiler intrinsic that prints one
scalar and is being retired in S7d. The obvious want is a `Show` trait a value formats
itself through, and a `Write` sink the formatted bytes flush to.

The naive shape (`trait: Show ; show ( &'T -- ) ;` with an `impl: Show for i64` living
wherever prints to stdout) is unsound in Sooth's own trait system before it is unsound
anywhere else. The orphan rule
(`check_impl_decls`, src/check/declarations.rs:491-499) forces *any* impl for a scalar
target into the trait's own module, so a `hosted`-libc `Show for i64` and a hypothetical
embedded-UART `Show for i64` can never coexist: that is a `(TraitId, Type)` duplicate the
moment both are imported (probe P5b). Two further walls close off the original two-trait-
variable sketch entirely:

- **Multi-variable traits are a hard parse error** (`multi_variable_trait_error`,
  src/parser.rs:2687-2689 and :2849, with a third site at :2722; probes P1a/P1b/P1c). Sink-genericity through a
  second `Show` type variable is unavailable without a compiler extension this slice does
  not build.
- **`str` cannot be constructed at runtime and never crosses `extern:`.** Its only
  producers are string literals and `static:` (src/ir/func_builder/calls.rs:40-44);
  its only consumers are `len`/`cstr` (src/check/word_families.rs:706-742); the FFI
  boundary admits `Int/Float/Usize/Isize/Ref/Cstr/Bool` only
  (`is_extern_boundary_scalar`, src/check/declarations.rs:93-101; probes P6a/P6a2). A
  "formatting logic that produces `str` chunks handed to a sink" is unwritable for
  anything computed.

The design is the buffer-indirection model (R1 revised): a value formats into a fixed
in-memory `StrBuf` via `Show`, and a separate `Write` sink flushes that whole buffer to a
device. This mirrors Rust's `Display`-writes-into-`fmt::Write`-then-a-separate-I/O-layer
split, and lets an embedded package later add `impl: Write for Uart` in its own module
without touching `core::show`.

## Design Rulings

These are the settled decisions from the brief's probe round (R1–R5, D1–D3). They are
the spec; the original two-trait-variable/`str`-chunk/`puts` sketch is rejected context.

1. **Buffer indirection, not a second trait variable (R1).** `core::show` declares a
   fixed render buffer `type: StrBuf data array[u8 64] len usize ;` and a single-variable
   trait `Show['T]` whose member is `show ( 'T &!StrBuf -- )`. A value reaches a sink by
   formatting into `StrBuf`, then flushing `StrBuf` through a separate `Write['S]` trait.
   One value type has exactly one `Show` impl; sink variety comes from distinct sink
   *types*, each with its own `Write` impl.
2. **By-value receiver for every scalar target (R5).** `show ( 'T &!StrBuf -- )`, not
   `show ( &'T ... )`: scalars have no address
   (``cannot borrow the scalar local``, probe P7), the `core::cmp` `Ord` precedent
   (`cmp ( 'T 'T -- Ordering )`). A `Show for i64` with a reference receiver could never
   be called on a bare `42`.
3. **The sink is `write(2)`, not `puts` (R2).** `hosted::libc` declares `type: Stdout ;`
   and `extern: sys-write ( i32 &!array[u8 64] usize -- isize ) "write" ;`. The binding is
   named `sys-write`, not `write`: an impl body binds its own member name ahead of module
   scope (src/ast.rs:1688-1700), so a `Write` member named `write` shadows a same-named
   extern inside the very impl that must call it (reproduced this round; `sys-write`
   builds and runs). The array mode is `&!array`, not `&array`: the flush receives the
   buffer as `&!StrBuf`, and a shared field projection off a mutable ref is a mode error —
   the mutable form is `examples/resources.sth:4`'s `read` precedent. Probe P6c proved
   the shared spelling from an *owned* local; do not generalize it to a `&!` field
   projection. `puts` is wrong three ways: it needs NUL termination, appends a newline,
   and rides stdio buffering.
4. **`core::show` stays `no_std` (no `extern:`).** The `Write for Stdout` impl and the
   `write(2)` extern live in `hosted::libc`. Orphan homes: every scalar-target `Show` impl
   lives in `core::show` (the trait's module, src/check/declarations.rs:491-499);
   `Write for Stdout` lives in `hosted::libc` (the target's module, the local-type orphan
   arm, probe P5a).
5. **Whole-buffer flush (D2).** `Write`'s member is `write ( &!'S &!StrBuf -- )`: one
   `write(2)` per flush, not one syscall per digit. The sink does not reset the buffer (a
   fresh `StrBuf` per print; a `reset` helper is optional). Short-write/EAGAIN handling is
   out of scope. Byte-at-a-time `write ( &!'S u8 -- )` is the named contingency only if
   the whole-buffer member hits a checker wall at impl time; its probe-log evidence is
   partial (P7 recorded the outcome — `prints 88 twice` — not the member-signature
   source), so re-probe before relying on it. This review round compiled the whole-buffer
   shape end to end in this tree, so it is the required path.
6. **Integer formatting via pure-Sooth restoring division (D1).** A division-by-10 helper
   inside `core::show` (shift-subtract long division over the u64 bit pattern, ~64 bounded
   iterations using `shl`/`shr`/`mod`/`sub`/comparisons) keeps S7c
   library-only, preserving S7d's "the only compiler-touching slice" sequencing note.
   Premise, stated precisely: integer `div` is float-only (`div_requires_float_error`,
   src/check/operators.rs:219-229); integer `mul` exists for every numeric type
   (src/check/operators.rs:203-217), and D1 (settled) stays with restoring division rather
   than a reciprocal/magic-number `mul` trick — the shift-subtract form is exact and needs
   no high-half product. Sign handling, as steps: capture the sign; take the magnitude as
   the u64 bit pattern — for negatives `>u64 not 1 add`, where `>u64` is the bit-preserving
   reinterpret that makes the magnitude exact for every `i64` including `MIN` (it reads
   `i64::MIN` as `9223372036854775808`; the u64 tower is unsigned) and the word is `add`,
   not `+` (`+` is retired, src/check/operators.rs:529); render the magnitude; prepend `-`.
7. **`Show for str` is descoped (D3).** Every `str` is literal- or static-rooted, so
   `cstr` conversion is total; S7d prints strings through `cstr` at the boundary, not
   through `Show`. Revisit only if a runtime `str` becomes constructible.
8. **Members resolve only through the enclosing word's own bound.** A bare `show` on a
   concrete value is `unknown word` (probe P5a, the `lib/core/cmp.sth` `cmp` precedent).
   Consumers call through a bounded word (`: render['T: Show] ( 'T &!StrBuf -- ) show ;`).

## Requirements

Each is independently verifiable.

- **R1.** `lib/core/show.sth` declares `type: StrBuf data array[u8 64] len usize ;` and
  the trait `trait: Show['T]` with member `: show ( 'T &!StrBuf -- ) ;`, the trait block
  closed by its own `;` (the `cmp.sth` form — a one-line `trait: … : show ( … ) ;` is a
  parse error). `core::show` is added to
  `lib/core/sooth.pkg`'s `module:` line. The module contains no `extern:` (no_std
  invariant).
- **R2.** `core::show` provides a pure-Sooth base-10 digit-extraction helper (restoring
  division by 10 over a u64 bit pattern, per Ruling 6) used by the integer `Show` impls.
  It is library code only: no new rows in `builtin_table`, no new compiler surface.
- **R3.** `core::show` declares `impl: Show for i64`, `impl: Show for usize`,
  `impl: Show for isize`, and `impl: Show for Bool`, each formatting into the `&!StrBuf`
  receiver. Writes begin at the buffer's incoming `len` (append semantics; a
  fresh buffer starts at 0): the integer impls append the decimal digits (signed impls
  prepend `-` for negatives per Ruling 6), the `Bool` impl appends `true`/`false`. Every impl lives in
  `core::show` (scalar-target orphan arm). The target is the enum `Bool` from `core::bool`
  (`lib/core/bool.sth:15`) — the surface name `bool` does not exist (no `bool`
  type alias exists in the compiler — the only `"bool"` strings in `src/` are
  `core::bool` module paths — and the bare name is rejected outright as `unknown type`);
  `src/check/declarations.rs:93-95` is `resolve_bool_type` admitting the *enum*
  inside `is_extern_boundary_scalar`, not a `bool` spelling. Phase 1 imports
  `Bool` and `if` from `core::bool`.
  Impl members inherit the trait member's signature — restating it is an error (`impl
  member show must not restate its signature`, reproduced this round). Bytes are stored
  through the `&!data` projection with `&!>` indexed refs; `len` through `&!len`.
- **R4.** `core::show` declares the trait `trait: Write['S]` with member
  `: write ( &!'S &!StrBuf -- ) ;` (the whole-buffer flush, D2; same block form as R1) and
  exports `StrBuf Show Write render flush` — types, trait names, and the two bounded
  consumer words (R6). The member names `show`/`write` are deliberately **not** exported:
  a trait member name cannot appear in `export:` (`error: … names nothing declared or
  imported`), matching the `cmp` precedent (`lib/core/cmp.sth:19` exports `Ord` and the
  surface words but not the member `cmp`) and Ruling 8's resolution rule. `Write` impls
  live in the sink's own module, not here.
- **R5.** `lib/hosted/libc.sth` declares `type: Stdout ;`,
  `extern: sys-write ( i32 &!array[u8 64] usize -- isize ) "write" ;`, and
  `impl: Write for Stdout` whose body flushes the buffer: it drops the `&!Stdout`
  receiver (the sink is stateless), reads the live `len` through `&!len @`, and passes
  fd `1` (`1 >i32`), the interior mutable `&!data` projection, and that `len` to the
  extern, discarding the `isize` return. That order is load-bearing: `len` must be
  read and bound **before** the `&!data` projection is taken — taking `&!data` first
  makes the later `&!len @` fail with ``cannot reborrow … while a reference derived
  from it is live`` (verified live this round). The extern's `array[u8 64]` capacity matches
  `StrBuf`'s declared capacity. `hosted::libc` exports `exit` and `Stdout`; the extern
  binding stays module-local (flushes go through `Write`, R6).
- **R6.** Two bounded consumer words in `core::show` —
  `: render['T: Show] ( 'T &!StrBuf -- ) show ;` and
  `: flush['S: Write] ( &!'S &!StrBuf -- ) write ;` — let a program render a value into a
  `StrBuf` and flush it through a `Write` sink without importing the impl word names
  (dispatch is bound-directed, Ruling 8; member names are not importable, which is why
  the flush word is not optional).
- **R7.** (Dogfood, golden) A Rust integration test `tests/phase7_slice7c.rs` — the S7b
  precedent (`tests/phase7_slice7b.rs:72-90`,
  `hosted_testing_expect_and_expect_eq_print_the_r1_protocol`, builds a program and
  asserts its stdout bytes) — builds and runs a dogfood program that renders values at **two** `Show`
  instantiations (e.g. `i64` and `Bool`) into a `StrBuf` and flushes each through
  `Stdout`, and pins the exact flushed bytes: `42` rendered through `Show for i64` flushes
  as the digits `42`, not a placeholder byte. S7c adds **no** entry under `examples/tests/`
  (S7b's pinned `sooth test` summary, `tests/phase7_slice7b.rs:486`, counts five entries
  and must stay untouched) and **no** `examples/` corpus entry (that harness asserts only
  hand-listed `CORPUS` members). Ownership in the dogfood is explicit: the program
  constructs a fresh `StrBuf`, renders, flushes, then `drop`s the buffer and the sink —
  construct/use/drop, the linear spine made visible; the sink never resets or retains the
  buffer (D2).
- **R8.** (NFR, no_std) `core::show` compiles as a `layer: core` module: no `extern:`, no
  dependency on `hosted`. The existing `lib/` corpus, `examples/`, and `examples/tests/`
  compile and run unchanged.
- **R9.** (NFR, buffering) Any dogfood that mixes `write(2)` output with the still-
  intrinsic `.` must not assert their interleaving (`.` is stdio-buffered, `write(2)` is
  unbuffered; probe P6c). S7d owns the post-retirement ordering golden.
- **R10.** (NFR, green) `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
  is green, including a golden asserting the flushed bytes for R7's two instantiations.
- **R11.** (Overflow clamp) The render path clamps its store index to `StrBuf` capacity:
  a write past the last byte is discarded, `len` never exceeds the capacity, and no
  out-of-bounds store is reachable. The store index starts at the buffer's incoming
  `len` (append semantics, R3; a fresh buffer starts at 0). The four scalar impls
  cannot reach 64 bytes (worst case `i64::MIN` at 21 bytes), so the clamp is pure
  defensive bound — reachable only by appending several renders into one buffer (the
  Phase 1 clamp-edge golden does exactly that) — stated as a
  requirement because "undefined-but-bounded" would let two workers make opposite
  choices.

## Success Criteria

- `core::show` declares `StrBuf`, `Show['T]` (by-value receiver), and `Write['S]`
  (whole-buffer flush); `Show` is implemented once each for `i64`, `usize`, `isize`, and
  `Bool`, every impl inside `core::show`. `str` is descoped.
- `hosted::libc` declares `type: Stdout` and `impl: Write for Stdout` over the real
  `write(2)` binding; a program renders a value into a `StrBuf` through a bounded word,
  flushes through `Write`, and the bytes appear on stdout.
- `42` rendered through `Show for i64` flushes as the ASCII digits `42`; a negative renders
  with a leading `-`; `Bool` renders `true`/`false`.
- The dogfood exercises `Show` at two instantiations and the `Stdout` flush; its golden
  pins the flushed bytes.
- Each new word/impl has golden coverage in `tests/phase7_slice7c.rs` (happy path plus
  one edge case; a `.sth` module has no Rust stage file to host `#[cfg(test)]` tests —
  `tests/phase7_slice7b.rs:1-3` is the precedent statement); full green.

## Scope & Boundaries

**In scope:** the `core::show` module (`StrBuf`, `Show['T]`, `Write['S]`, the four scalar
`Show` impls, the restoring-division helper, the bounded consumer word); the
`hosted::libc` `Stdout` type, `write(2)` extern, and `Write for Stdout` impl; package-file
wiring; the dogfood golden and the build-and-run goldens (`tests/phase7_slice7c.rs`).

**Out of scope (per the brief):**

- Multi-variable traits or partially-ground trait dispatch (a compiler extension; R3 of
  the brief).
- Any sink beyond `Stdout` (a `Uart` impl is the R1 seam's future consumer, not a
  deliverable).
- Retiring the intrinsic `.` (S7d). S7c binds sinks to `write(2)` at the `extern:`
  boundary from the start, so S7d must not implement its sink on the `str` `Print` arm it
  deletes.
- Derived/automatic `Show` for user structs/enums; hand-written scalar impls only.
- `Show for str` (D3).
- A growable/heap-backed buffer (needs the P9 allocator); `StrBuf` is fixed capacity.
  Overflow behaviour: clamped, per R11 — writes past capacity are discarded and `len`
  never exceeds capacity. The Phase 2 dogfood renders into a fresh buffer per print and
  never approaches capacity; the Phase 1 clamp-edge golden is what exercises the edge
  (several renders appended into one buffer).
- An integer `div` builtin row (D1 chose library-only restoring division; adding the row
  would be the compiler surface S7d's sequencing note reserves. Integer `mul` already
  exists — src/check/operators.rs:203-217 — and is deliberately not repurposed into a
  reciprocal trick; see Ruling 6).
- `Slice[u8]` as a member-parameter or extern-boundary chunk type (not a legal member
  shape, `member_shape_is_supported`; not boundary-admissible).
- The known diagnostics warts stay unfixed here: `error: error:` doubling (`src/main.rs`
  wraps an already-prefixed message), mangled names in extern-boundary errors (e.g.
  `emit__m0`), and the glued `'S:W` lexing surprise (probe P2 — write bound params with a
  space, `'S: W`).

## Codebase Map

Anchors are path:line + symbol against the `mongols` worktree at spec time.

- **`lib/core/cmp.sth`** — the trait-as-library precedent this slice copies: `trait: Ord['T]`
  with a by-value member, one `impl:` per scalar width, bound-directed dispatch through
  surface words (`eq`/`lt`/…). `Show`/`Write` mirror its structure exactly.
- **`lib/core/sooth.pkg`** — `module: bool cmp prelude combinators option result ;`; add
  `show`.
- **`lib/hosted/libc.sth`** — currently `extern: exit ( i32 -- ) "exit" ;` + `export:`; add
  `Stdout`, the `write(2)` extern, and `impl: Write for Stdout`.
- **`lib/hosted/testing.sth`** — the S7b consumer (`expect`/`expect-eq`) that will later
  print actual/expected via `Show`; not modified this slice but the motivating client.
- **`examples/tests/cmp.sth`** — the dogfood-golden style to imitate for `show.sth`.
- **`examples/resources.sth:4`** — in-tree `extern: read ( i64 &!array[u8 64] usize -- isize ) "read" ;`,
  the direct precedent for the R5 `write(2)` binding shape, mutable array mode included.
- **`src/parser.rs:2687-2689`, `:2849` (a third site at `:2722`)** —
  `multi_variable_trait_error`: why `Show`
  and `Write` are two single-variable traits, not one two-variable trait.
- **`src/parser.rs:1668`** — `parse_poly_ty_var`: a bound inside a stack effect is a
  located error (probe P2b); bounds ride the word's bound bracket.
- **`src/check/declarations.rs:93-101`** — `is_extern_boundary_scalar`: admits
  `Int/Float/Usize/Isize/Ref(..)/Cstr/Bool`; the R5 extern uses the `Ref(..)` (`&!array`),
  `i32`, and `usize` arms.
- **`src/check/declarations.rs:491-499`** — `check_impl_decls` orphan rule: scalar-target
  impls must live in the trait's module (`core::show`); a local-type target (`Stdout`) may
  home in the target's module (`hosted::libc`), probe P5a.
- **`src/check/word_families.rs:706-742`** — `str` consumers (`len`/`cstr`) only: why the
  buffer is `array[u8 N]`, not `str`.
- **`src/check/operators.rs:219-229`** — `div_requires_float_error` (integer `div` is
  float-only; integer `mul` exists for all numerics, `:203-217`); **`src/check/builtins.rs:188-196`**
  — the integer tower (`mod`/`and`/`or`/`xor`/`not`/`shl`/`shr`/`max`): why D1's restoring
  division is library-only.
- **`src/check/poly.rs:1075-1086`** — `substitute_member_var`: single-variable member
  dispatch, the mechanism `Show`/`Write` dispatch through.
- **`src/ir/func_builder/calls.rs:40-44`, `src/ir/layout.rs:397`** — the only `str`
  producers (literal, `static:`): why `StrBuf` is a byte array with a `len`, not `str`.

## Open Questions & Risks

- **P-A (Phase 2 entry check, not an open question).** Confirm at impl time, before
  writing the `Write for Stdout` body, that the provenance-carrying `&!data` interior ref
  (the P7.S1 accessor output over `StrBuf.data`) is admitted at the extern boundary call
  site in the required mode: `extern: sys-write ( i32 &!array[u8 64] usize -- isize )`
  with the body passing `&!data`. This is settled enough to be the *required* path — this
  review round compiled the corrected two-package design end to end in this tree
  (`core::show` trait+impls, `hosted::libc` extern+impl, app-side `render`+`flush`
  printing the rendered digits). The old shared-ref fallback (`&buf`, extern typed
  `&array`) is dead: a shared projection off a `&!StrBuf` receiver is a mode error, and
  P6c's shared spelling was proven from an owned local, not a `&!` field projection.
- **P-B (Phase 2 entry check, not an open question).** Confirm at impl time that the
  whole-buffer member `write ( &!'S &!StrBuf -- )` dispatches and lowers end to end
  (probe P3a/P3b proved `&!'S` receivers with an extra concrete param; the added second
  `&!` aggregate param ran end to end in this review round). If it hits a checker wall at
  impl time, the D2 contingency is the byte-at-a-time `write ( &!'S u8 -- )` shape, with
  the impl looping the buffer — named here and in the phase plan so it is never silently
  substituted; its probe-log evidence is partial (P7 recorded the outcome, not the
  source), so re-probe before relying on it.
- **Overflow (resolved as R11).** `StrBuf` is fixed at 64 bytes; a `u64` decimal is at
  most 20 digits + sign, well under 64, so the Phase 2 dogfood (fresh buffer per print)
  never approaches capacity. Rather than leave
  "undefined-but-bounded" (two workers would make opposite choices), the render path
  clamps its store index to capacity: writes past capacity are discarded, `len` never
  exceeds capacity, and the Phase 1 clamp-edge golden reaches the edge by appending
  several renders into one buffer.
- **Buffering ordering.** `.` (stdio) and `write(2)` interleave unpredictably (P6c); the
  dogfood must route all output through one channel or not assert ordering (R9).

## Phased Delivery Plan

### Phase 1: `core::show` — `StrBuf`, `Show`, and the division helper

**Goal.** Land the no_std buffer, the `Show['T]` trait, the four scalar impls, and the
pure-Sooth base-10 helper, verified in isolation (no sink yet).

**Scope.** New `lib/core/show.sth`: `type: StrBuf data array[u8 64] len usize ;`;
`trait: Show['T]` with member `: show ( 'T &!StrBuf -- ) ;` (block closed by its own `;`);
the restoring-division-by-10 helper (`shl`/`shr`/`mod`/`sub`/comparisons over the u64 bit
pattern, Ruling 6, sign handling per D1); `impl: Show for i64/usize/isize/Bool` (impl
members inherit the member signature — do not restate it); the bounded `render` and
`flush` words (R6). Add `show` to `lib/core/sooth.pkg`. Imports: `core::bool` (`Bool`,
`if`), `core::cmp` (comparisons), and `core::combinators` (`times` **and**
`times-helper` — `times-helper` must be imported at the splice site,
`lib/core/combinators.sth:30-32`, since every `times` caller splices its body), so
`core::show` depends on three sibling core modules; `times`'s shape is
`( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )` — count under quotation, the body receives the
0-based index on top. Coverage is build-and-run goldens in `tests/phase7_slice7c.rs`
(there is no Rust stage file for a `.sth` module — `tests/phase7_slice7b.rs:1-3` says so
and is the precedent): scratch-tree modules render into a `StrBuf` and read the
bytes/`len` back directly (no sink; the still-live `.` may surface values for the
harness), covering digit extraction (`42`), a negative, `Bool` both arms, the `i64::MIN`
magnitude edge, and the overflow-clamp edge (R11 — several renders appended into one
buffer until the store index reaches capacity).

**Entry conditions.** None (green tree at HEAD).

**Exit criteria.** `core::show` compiles as `layer: core` with no `extern:`; a golden in
`tests/phase7_slice7c.rs` renders `42`, a negative, and `Bool` both arms into a `StrBuf`
and inspects the bytes/`len` directly (no sink), including the `i64::MIN` and clamp
edges; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.

**Effort.** M. **Difficulty.** standard (the restoring-division helper is the only real
logic). **Blockers.** None.

### Phase 2: `hosted::libc` — `Stdout`, `write(2)`, and the flush

**Goal.** Land the sink and prove a value renders and flushes to stdout end to end.

**Scope.** First run the two entry checks P-A and P-B (interior-ref admission in the
corrected mode; whole-buffer member dispatch — both compiled clean in this tree during
review); adopt the byte-at-a-time fallback only if a check fails, recording which shape
shipped. Add to `lib/hosted/libc.sth`: `type: Stdout ;`,
`extern: sys-write ( i32 &!array[u8 64] usize -- isize ) "write" ;`, the `Write` trait
import from `core::show`, and `impl: Write for Stdout` (drop the `&!Stdout` receiver,
read `len` through `&!len @`, pass fd `1` via `1 >i32`, interior `&!data`, discard the
`isize`). Build the dogfood program under the test's scratch tree and pin its flushed
bytes in `tests/phase7_slice7c.rs` (R7): render two instantiations (`i64` + `Bool`),
flush each through `Stdout`, one output channel only (R9).

**Entry conditions.** Phase 1 green.

**Exit criteria.** The dogfood builds and runs; `42` flushes as `42`; `tests/phase7_slice7c.rs`
pins the two-instantiation output; full green. If the fallback shipped, the spec/entry
note records it. Re-run the CLAUDE.md growth signals against `libc.sth` and `show.sth` at exit.

**Effort.** M. **Difficulty.** standard (extern boundary + dispatch, de-risked: both
entry checks compiled clean in review). **Blockers.** None expected; the byte-at-a-time
fallback is the escape hatch.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "core::show: StrBuf, Show trait and scalar impls, restoring-division helper, render/flush words", "effort": "M", "difficulty": "standard" },
    { "phase": 2, "focus": "hosted::libc: Stdout, sys-write extern, Write for Stdout flush, two-instantiation golden", "effort": "M", "difficulty": "standard" }
  ]
}
```
