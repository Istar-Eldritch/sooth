# Phase 7 Slice 7c: `Show`/`Write`, a sink-generic printing pair

Implemented on `mongols`: phase 1 `4c09566` (`core::show`), phase 2 `ae588fb`
(`hosted::libc` sink). Discovery brief: `slice7c-show-brief.md`; probe log:
`slice7c-probes.md`.

## Problem

S7b's `expect`/`expect-eq` can report only a *label*: a failing `expect-eq` cannot
print actual vs expected, because Sooth has no value-to-text path (`.` is a scalar-only
intrinsic being retired in S7d). The want is a `Show` trait a value formats itself
through and a `Write` sink the bytes flush to.

The naive `trait: Show ; show ( &'T -- )` with a stdout-printing `impl: Show for i64` is
unsound in Sooth's own trait system, and two walls close off the sink-generic sketch:

- **Orphan rule** (`check_impl_decls`, src/check/declarations.rs) forces any scalar-target
  impl into the trait's module, so a libc `Show for i64` and an embedded-UART one can
  never coexist: a `(TraitId, Type)` duplicate the moment both import.
- **Multi-variable traits are a hard parse error** (`multi_variable_trait_error`,
  src/parser.rs), so sink-genericity through a second `Show` type variable is unavailable
  without a compiler extension this slice does not build.
- **`str` cannot be built at runtime and never crosses `extern:`** (producers are literals
  and `static:` only; the FFI boundary admits only `Int/Float/Usize/Isize/Ref/Cstr/Bool`),
  so a "formatting logic that produces `str` chunks" is unwritable for anything computed.

The shipped design is buffer indirection: a value formats into a fixed in-memory `StrBuf`
via `Show`, and a separate `Write` sink flushes the whole buffer to a device. This mirrors
Rust's `Display`-into-`fmt::Write`-then-a-separate-I/O-layer split, and lets an embedded
package later add `impl: Write for Uart` in its own module without touching `core::show`.

## Design rulings

1. **Buffer indirection, not a second trait variable (R1).** `core::show` declares a fixed
   `type: StrBuf data array[u8 64] len usize ;` and single-variable `Show['T]`
   (`show ( 'T &!StrBuf -- )`). A value reaches a sink by formatting into `StrBuf`, then
   flushing through a separate `Write['S]`. Sink variety comes from distinct sink *types*,
   each with its own `Write` impl.
2. **By-value receiver for every scalar target (R5).** `show ( 'T &!StrBuf -- )`, not
   `show ( &'T ... )`: scalars have no address (`cannot borrow the scalar local`), matching
   the `core::cmp` `Ord` precedent. A reference-receiver `Show for i64` could never be
   called on a bare `42`.
3. **The sink is `write(2)`, not `puts` (R2).** `hosted::libc` declares `type: Stdout ;`
   and `extern: sys-write ( i32 &!array[u8 64] usize -- isize ) "write" ;`. Named
   `sys-write`, not `write`: an impl body binds its own member name ahead of module scope,
   so a `Write` member named `write` would shadow a same-named extern inside the impl that
   must call it. The array mode is `&!array` because the flush holds the buffer as `&!StrBuf`
   and a shared projection off a mutable ref is a mode error. `puts` is wrong three ways: NUL
   termination, an appended newline, stdio buffering.
4. **`core::show` stays `no_std`.** No `extern:`. The `write(2)` extern and `Write for Stdout`
   live in `hosted::libc` (the target's own module, the local-type orphan arm). Every
   scalar-target `Show` impl lives in `core::show` (the trait's module).
5. **Whole-buffer flush (D2).** `write ( &!'S &!StrBuf -- )`: one `write(2)` per flush, not
   per digit. The sink does not reset the buffer (fresh `StrBuf` per print). Short-write /
   EAGAIN out of scope. The whole-buffer shape compiled end to end; the byte-at-a-time
   contingency was never needed.
6. **Integer formatting via pure-Sooth restoring division (D1).** A shift-subtract long
   division by ten over the u64 bit pattern (~64 bounded iterations) keeps S7c library-only,
   preserving S7d's "only compiler-touching slice" note. Integer `div` is float-only
   (`div_requires_float_error`); the reciprocal/magic-number `mul` trick was rejected for the
   exact shift-subtract form. Sign handling: capture the sign, take the magnitude as the u64
   bit pattern (`>u64 not 1 add`, exact for `i64::MIN`), render, prepend `-`. The word is
   `add`, not the retired `+`.
7. **`Show for str` is descoped (D3).** Every `str` is literal- or static-rooted, so `cstr`
   is total; S7d prints strings through `cstr` at the boundary. Revisit only if a runtime
   `str` becomes constructible.
8. **Members resolve only through the enclosing word's own bound.** A bare `show` on a
   concrete value is `unknown word`; consumers call through a bounded word. The member names
   `show`/`write` are deliberately not exported (a trait member cannot appear in `export:`,
   matching `cmp`), which is why the `render`/`flush` words are not optional.
9. **Overflow clamp (R11).** `append-byte` clamps the store index to `StrBuf` capacity: a
   write past byte 64 is discarded and `len` never exceeds capacity. The four scalar impls
   cannot reach 64 bytes (worst case `i64::MIN` at 21), so the clamp is defensive, reachable
   only by appending several renders into one buffer. Stated as a requirement because
   "undefined-but-bounded" would let two workers make opposite choices.

## Delivered shape

- **`lib/core/show.sth`** (`4c09566`): `StrBuf`; `Show['T]` and `Write['S]` traits;
  `append-byte` (clamped append), `divmod10` (restoring division), `append-digits`
  (MSB-first recursion, lone `0` for zero); `impl: Show for i64/usize/isize/Bool`; bounded
  `render['T: Show]` and `flush['S: Write]`. Imports `self::bool`, `self::cmp`, and
  `self::combinators` (`times` only). `show` added to `lib/core/sooth.pkg`.
- **`lib/hosted/libc.sth`** (`ae588fb`): `type: Stdout ;`, the `sys-write` extern, and
  `impl: Write for Stdout` (drop the stateless receiver, read `len` through `&!len @`
  **before** taking the `&!data` projection — the reverse order trips a reborrow error —
  pass fd `1 >i32`, discard the `isize`). `Stdout` added to `export:`; the extern stays
  module-local (flushes go through `Write`).

The source carries the per-decision rationale inline; see the commits for the full bodies.

## Tests

`tests/phase7_slice7c.rs` (scratch-tree build-and-run goldens; a `.sth` module has no Rust
stage file for `#[cfg(test)]`):

- Phase 1 (render into a `StrBuf`, inspect bytes/`len` directly, no sink):
  `show_i64_renders_positive_digits`, `show_i64_negative_prepends_minus`,
  `show_i64_min_magnitude_is_exact`, `show_bool_renders_both_arms`,
  `show_usize_and_isize_render_digits`, `show_overflow_clamps_len_at_capacity` (the R11 edge,
  several renders appended into one buffer).
- Phase 2: `stdout_flush_renders_two_instantiations` — renders `i64` and `Bool`, flushes each
  through `Stdout`, pins the exact flushed bytes (R7); one output channel only (R9).

No `examples/tests/` or `examples/` corpus entry (would perturb S7b's pinned summary and the
corpus harness).

## Out of scope

Multi-variable traits / partial trait dispatch; any sink beyond `Stdout`; retiring `.` (S7d);
derived `Show` for user structs/enums; `Show for str` (D3); a growable/heap-backed buffer
(needs the P9 allocator); an integer `div` builtin row (D1 chose library-only); `Slice[u8]`
as a member/boundary chunk type. Known diagnostic warts (`error: error:` doubling, mangled
names in boundary errors, the glued `'S:W` lexing surprise) stay unfixed.

## Bookkeeping

P7.S7c `[ done ]` in the roadmap. S7b's label-only assertions are the motivating client;
S7d retires `.` and prints strings through `cstr`, and must not build its sink on the `str`
`Print` arm it deletes (S7c already binds sinks to `write(2)` at the `extern:` boundary).
Growth-signal re-run at Phase 2 exit did not fire: `show.sth` and `libc.sth` each stayed a
single-responsibility module.
