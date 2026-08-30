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
  src/parser.rs:2687-2689 and 2843-2845; probes P1a/P1b/P1c). Sink-genericity through a
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
   and `extern: write ( i32 &array[u8 64] usize -- isize ) "write" ;` (the proven binding,
   probe P6c; in-tree precedent `examples/resources.sth`'s `read`). `puts` is wrong three
   ways: it needs NUL termination, appends a newline, and rides stdio buffering.
4. **`core::show` stays `no_std` (no `extern:`).** The `Write for Stdout` impl and the
   `write(2)` extern live in `hosted::libc`. Orphan homes: every scalar-target `Show` impl
   lives in `core::show` (the trait's module, src/check/declarations.rs:491-499);
   `Write for Stdout` lives in `hosted::libc` (the target's module, the local-type orphan
   arm, probe P5a).
5. **Whole-buffer flush (D2).** `Write`'s member is `write ( &!'S &!StrBuf -- )`: one
   `write(2)` per flush, not one syscall per digit. The sink does not reset the buffer (a
   fresh `StrBuf` per print; a `reset` helper is optional). Short-write/EAGAIN handling is
   out of scope. Byte-at-a-time `write ( &!'S u8 -- )` is the probe-proven fallback (P7)
   only if the whole-buffer shape hits a checker wall at impl time.
6. **Integer formatting via pure-Sooth restoring division (D1).** A division-by-10 helper
   inside `core::show` (shift-subtract long division over the u64 bit pattern, ~64 bounded
   iterations using `shl`/`shr`/`mod`/sub/comparisons; no integer `div`/`mul` exists,
   src/check/operators.rs:219-229, src/check/builtins.rs:188-196) keeps S7c
   library-only, preserving S7d's "the only compiler-touching slice" sequencing note. Sign
   handling: capture the sign, take the magnitude as the u64 bit pattern (for negatives
   `not 1 +`, exact for every `i64` including `MIN`), render the magnitude, prepend `-`.
7. **`Show for str` is descoped (D3).** Every `str` is literal- or static-rooted, so
   `cstr` conversion is total; S7d prints strings through `cstr` at the boundary, not
   through `Show`. Revisit only if a runtime `str` becomes constructible.
8. **Members resolve only through the enclosing word's own bound.** A bare `show` on a
   concrete value is `unknown word` (probe P5a, the `lib/core/cmp.sth` `cmp` precedent).
   Consumers call through a bounded word (`: render['T: Show] ( 'T &!StrBuf -- ) show ;`).

## Requirements

Each is independently verifiable.

- **R1.** `lib/core/show.sth` declares `type: StrBuf data array[u8 64] len usize ;` and
  `trait: Show['T] : show ( 'T &!StrBuf -- ) ;`. `core::show` is added to
  `lib/core/sooth.pkg`'s `module:` line. The module contains no `extern:` (no_std
  invariant).
- **R2.** `core::show` provides a pure-Sooth base-10 digit-extraction helper (restoring
  division by 10 over a u64 bit pattern, per Ruling 6) used by the integer `Show` impls.
  It is library code only: no new rows in `builtin_table`, no new compiler surface.
- **R3.** `core::show` declares `impl: Show for i64`, `impl: Show for usize`,
  `impl: Show for isize`, and `impl: Show for bool`, each formatting into the `&!StrBuf`
  receiver: the integer impls append the decimal digits (signed impls prepend `-` for
  negatives per Ruling 6), the `bool` impl appends `true`/`false`. Every impl lives in
  `core::show` (scalar-target orphan arm).
- **R4.** `core::show` declares `trait: Write['S] : write ( &!'S &!StrBuf -- ) ;` (the
  whole-buffer flush, D2) and exports `StrBuf Show show Write write` (plus the bounded
  consumer word, R6). `Write` impls live in the sink's own module, not here.
- **R5.** `lib/hosted/libc.sth` declares `type: Stdout ;`,
  `extern: write ( i32 &array[u8 64] usize -- isize ) "write" ;`, and
  `impl: Write for Stdout` whose body flushes the buffer: it passes fd `1`, the buffer's
  interior `&data` ref, and the live `len` (`>usize`) to the extern, discarding the
  `isize` return. The extern's `array[u8 64]` capacity matches `StrBuf`'s declared
  capacity. `Stdout`, `write`, and the extern are exported as appropriate.
- **R6.** A bounded consumer word (`: render['T: Show] ( 'T &!StrBuf -- ) show ;` in
  `core::show`, and a flush word if needed) lets a program render a value into a `StrBuf`
  and flush it through a `Write` sink without importing the impl word names (dispatch is
  bound-directed, Ruling 8).
- **R7.** (Dogfood, golden) `examples/tests/show.sth` (or `examples/show.sth`) renders
  values at **two** `Show` instantiations (e.g. `i64` and one of `usize`/`isize`/`bool`)
  into a `StrBuf` and flushes each through `Stdout`, observing the bytes on stdout. `42`
  rendered through `Show for i64` flushes as the digits `42`, not a placeholder byte.
- **R8.** (NFR, no_std) `core::show` compiles as a `layer: core` module: no `extern:`, no
  dependency on `hosted`. The existing `lib/` corpus, `examples/`, and `examples/tests/`
  compile and run unchanged.
- **R9.** (NFR, buffering) Any dogfood that mixes `write(2)` output with the still-
  intrinsic `.` must not assert their interleaving (`.` is stdio-buffered, `write(2)` is
  unbuffered; probe P6c). S7d owns the post-retirement ordering golden.
- **R10.** (NFR, green) `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
  is green, including a golden asserting the flushed bytes for R7's two instantiations.

## Success Criteria

- `core::show` declares `StrBuf`, `Show['T]` (by-value receiver), and `Write['S]`
  (whole-buffer flush); `Show` is implemented once each for `i64`, `usize`, `isize`, and
  `bool`, every impl inside `core::show`. `str` is descoped.
- `hosted::libc` declares `type: Stdout` and `impl: Write for Stdout` over the real
  `write(2)` binding; a program renders a value into a `StrBuf` through a bounded word,
  flushes through `Write`, and the bytes appear on stdout.
- `42` rendered through `Show for i64` flushes as the ASCII digits `42`; a negative renders
  with a leading `-`; `bool` renders `true`/`false`.
- The dogfood exercises `Show` at two instantiations and the `Stdout` flush; its golden
  pins the flushed bytes.
- Each new word/impl has unit coverage beside it (happy path plus one error/edge case);
  full green.

## Scope & Boundaries

**In scope:** the `core::show` module (`StrBuf`, `Show['T]`, `Write['S]`, the four scalar
`Show` impls, the restoring-division helper, the bounded consumer word); the
`hosted::libc` `Stdout` type, `write(2)` extern, and `Write for Stdout` impl; package-file
wiring; the dogfood golden and unit tests.

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
  Overflow behaviour: a render that would exceed 64 bytes is undefined-but-bounded this
  slice (see Open Questions); no dogfood renders past capacity.
- Integer `div`/`mul` builtin rows (D1 chose library-only division; adding rows would be
  the compiler surface S7d's sequencing note reserves).
- `Slice[u8]` as a member-parameter or extern-boundary chunk type (not a legal member
  shape, `member_shape_is_supported`; not boundary-admissible).

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
- **`examples/resources.sth`** — in-tree `extern: read ( i64 &!array[u8 64] usize -- isize ) "read" ;`,
  the direct precedent for the R5 `write(2)` binding shape.
- **`src/parser.rs:2687-2689`, `:2843-2845`** — `multi_variable_trait_error`: why `Show`
  and `Write` are two single-variable traits, not one two-variable trait.
- **`src/parser.rs:1668`** — `parse_poly_ty_var`: a bound inside a stack effect is a
  located error (probe P2b); bounds ride the word's bound bracket.
- **`src/check/declarations.rs:93-101`** — `is_extern_boundary_scalar`: admits
  `Int/Float/Usize/Isize/Ref(..)/Cstr/Bool`; the R5 extern uses the `Ref(..)` (`&array`),
  `i32`, and `usize` arms.
- **`src/check/declarations.rs:491-499`** — `check_impl_decls` orphan rule: scalar-target
  impls must live in the trait's module (`core::show`); a local-type target (`Stdout`) may
  home in the target's module (`hosted::libc`), probe P5a.
- **`src/check/word_families.rs:706-742`** — `str` consumers (`len`/`cstr`) only: why the
  buffer is `array[u8 N]`, not `str`.
- **`src/check/operators.rs:219-229`** — `div_requires_float_error`; **`src/check/builtins.rs:188-196`**
  — the integer tower (`mod`/`and`/`or`/`xor`/`not`/`shl`/`shr`/`max`), no int `div`/`mul`:
  why D1's restoring division is library-only.
- **`src/check/poly.rs:1075-1086`** — `substitute_member_var`: single-variable member
  dispatch, the mechanism `Show`/`Write` dispatch through.
- **`src/ir/func_builder/calls.rs:40-44`, `src/ir/layout.rs:397`** — the only `str`
  producers (literal, `static:`): why `StrBuf` is a byte array with a `len`, not `str`.

## Open Questions & Risks

- **P-A (spec-time probe).** Does a provenance-carrying `&!data` interior ref (the P7.S1
  accessor output over `StrBuf.data`) satisfy the extern boundary's `Ref(..)` admission at
  the *call site*? P6c proved the plain `&buf` shape and P6c2 proved `is_extern_boundary_scalar`
  admits `Type::Ref(..)` at declaration level, but the interior-accessor call-site path is
  unproven. Run this before writing the `Write for Stdout` body: build a minimal impl
  passing `&!buf &data` to the extern. If it is rejected, fall back to passing `&buf`
  (the whole-struct ref) with the extern typed against the struct, or reshape.
- **P-B (spec-time probe).** Does the whole-buffer member `write ( &!'S &!StrBuf -- )`
  dispatch and lower end to end (probe P3a/P3b proved `&!'S` receivers with an extra
  concrete param; this adds a second `&!` aggregate param)? If it hits a checker wall, the
  D2 fallback is the byte-at-a-time `write ( &!'S u8 -- )` shape probe P7 ran end to end,
  with the impl looping the buffer. The fallback should be *named* in the phase plan, not
  silently substituted.
- **Overflow.** `StrBuf` is fixed at 64 bytes; a `u64` decimal is at most 20 digits + sign,
  well under 64, so no dogfood overflows. A render past capacity is out of scope this
  slice; the helper should not silently corrupt adjacent memory (bound the store index by
  capacity or leave the window to the P9 growable follow-up). Flag if the fixed cap forces
  a decision.
- **Buffering ordering.** `.` (stdio) and `write(2)` interleave unpredictably (P6c); the
  dogfood must route all output through one channel or not assert ordering (R9).

## Phased Delivery Plan

### Phase 1 — `core::show`: `StrBuf`, `Show`, and the division helper

**Goal.** Land the no_std buffer, the `Show['T]` trait, the four scalar impls, and the
pure-Sooth base-10 helper, verified in isolation (no sink yet).

**Scope.** New `lib/core/show.sth`: `type: StrBuf data array[u8 64] len usize ;`;
`trait: Show['T] : show ( 'T &!StrBuf -- ) ;`; the restoring-division-by-10 helper
(`shl`/`shr`/`mod`/sub/comparisons over the u64 bit pattern, Ruling 6, sign handling per
D1); `impl: Show for i64/usize/isize/bool`; the bounded `render` consumer word. Add `show`
to `lib/core/sooth.pkg`. Unit tests beside each word: digit extraction happy path, the
`MIN` magnitude edge, `bool` both arms.

**Entry conditions.** None (green tree at HEAD).

**Exit criteria.** `core::show` compiles as `layer: core` with no `extern:`; a unit test
renders `42`, a negative, and a `bool` into a `StrBuf` and inspects the bytes/`len`
directly (no sink); `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.

**Effort.** M. **Difficulty.** M (the restoring-division helper is the only real logic).
**Blockers.** None.

### Phase 2 — `hosted::libc`: `Stdout`, `write(2)`, and the flush

**Goal.** Land the sink and prove a value renders and flushes to stdout end to end.

**Scope.** First run probes P-A and P-B (interior-ref admission; whole-buffer member
dispatch); adopt the byte-at-a-time fallback only if P-B fails, recording which shape
shipped. Add to `lib/hosted/libc.sth`: `type: Stdout ;`,
`extern: write ( i32 &array[u8 64] usize -- isize ) "write" ;`, `trait: Write` import from
`core::show`, `impl: Write for Stdout` (fd `1`, interior `&data`, live `len`). Wire the
dogfood `examples/tests/show.sth` (or `examples/show.sth`): render two instantiations
(`i64` + one of `usize`/`isize`/`bool`), flush each through `Stdout`, one output channel
only (R9). Add the golden pinning the flushed bytes.

**Entry conditions.** Phase 1 green.

**Exit criteria.** The dogfood builds and runs; `42` flushes as `42`; the golden pins the
two-instantiation output; full green. If the fallback shipped, the spec/entry note records
it. Re-run the CLAUDE.md growth signals against `libc.sth` and `show.sth` at exit.

**Effort.** M. **Difficulty.** M (extern boundary + dispatch, de-risked by the probes).
**Blockers.** P-A/P-B outcomes; whole-buffer fallback is the escape hatch.

## Phases (JSON)

```json
[
  {
    "phase": 1,
    "focus": "core::show: StrBuf buffer, Show['T] trait with by-value receiver, four scalar impls (i64/usize/isize/bool), pure-Sooth restoring-division base-10 helper, bounded render word; no_std, no extern; unit tests beside each word",
    "effort": "M",
    "difficulty": "M"
  },
  {
    "phase": 2,
    "focus": "hosted::libc: Stdout type, write(2) extern, Write for Stdout whole-buffer flush impl (byte-at-a-time fallback if the member dispatch fails); spec-time probes P-A (interior &!data ref at extern boundary) and P-B (whole-buffer member dispatch); dogfood golden exercising Show at two instantiations and the Stdout flush",
    "effort": "M",
    "difficulty": "M"
  }
]
```
