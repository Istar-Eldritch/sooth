# Phase 7 Slice 7c: `Show`/`Write`, a sink-generic printing pair (brief)

**Status: probe round complete (260829). R1 and R2 below are the revised
rulings.** The original sketch — `trait: Show['T] show ( &'T &!'S:Write -- ) ;`,
`str` chunks handed to the sink, a `puts` binding — is **unimplementable as
written** and was replaced after the probe round; the probe-establish facts
section records exactly why, with citations. Verbatim probe log:
[slice7c-probes.md](./slice7c-probes.md) (paper recon over the parser/checker
paths plus 23 live compile/run probes against the current tree). The spec must
be written from the revised rulings and the settled decisions D1–D3, not from
the original sketch.

S7b `[ done ]` shipped `hosted::testing`'s `expect`/`expect-eq`, label-only,
printing through the still-intrinsic `.` (`lib/hosted/testing.sth`); its
dogfood suite is `examples/tests/`. `Show` is the obvious next want once that
consumer's assertions can only report a label, not the actual/expected values.
The naive shape — `trait: Show ; show ( &'T -- ) ;` with an `impl: Show for
i64` living wherever prints to stdout — is unsound in Sooth's own trait system
before it's unsound anywhere else: the orphan rule
(`check_impl_decls_orphan_scalar_target_names_only_the_trait_module`) forces
*any* impl for a scalar target into the trait's own module, so a
`hosted`-libc `Show for i64` and a hypothetical embedded-UART `Show for i64`
can never coexist as two impls of the same trait — that's a `(TraitId,
PolyType)` duplicate the moment both are imported into one program, not a
hypothetical. That motivation survives the probe round unchanged; what changed
is the mechanism.

## Probe-established facts (the constraints the design must obey)

1. **No multi-variable traits.** `trait: Show['T 'S]` is a hard parse error
   (`multi_variable_trait_error`, src/parser.rs:2687-2689, S3e R16), enforced
   twice: on the header bracket and again on a member effect introducing a
   second variable (src/parser.rs:2843-2845). Dispatch grounds exactly one
   variable — `substitute_member_var` maps *every* `PolyType::Var(_)` onto the
   single chosen var (src/check/poly.rs:1075-1086) — and there is no
   partially-ground trait dispatch anywhere. Sink-genericity through a second
   `Show` type variable is unavailable without a compiler extension, which
   this slice does not build (see R3).
2. **`str` cannot be constructed at runtime.** The only producers are string
   literals and `static: X str` (src/ir/func_builder/calls.rs:40-44,
   src/ir/layout.rs:397); the only `str` consumers are `len` and `cstr`
   (src/check/word_families.rs:706-742). A `Slice[u8]` view over a filled
   buffer is not a `str` and cannot be converted to one (probes P6b2/P6b3);
   a slice's `len` sees the whole array, with no partial-fill notion (P6b).
   "Formatting logic that produces `str` chunks" is therefore unwritable for
   anything computed — the original R1's chunk model is dead.
3. **There is no integer division or multiplication.** `div` has float rows
   only (`div_requires_float_error`, src/check/operators.rs:219-229); the int
   tower is `mod`/`and`/`or`/`xor`/`not`/`shl`/`shr`/`max`
   (src/check/builtins.rs:188-196). A base-10 itoa is not directly
   expressible today — see R4/D1. No itoa helper exists anywhere in the
   corpus.
4. **Scalars cannot be borrowed.** A `&i64` operand is unnameable:
   ``cannot borrow the scalar local `n` of type `i64` … a scalar has no
   address; borrow a field or an aggregate instead`` (probe P7). `Show`'s
   receiver must be **by value** for every scalar target — the `core::cmp`
   `Ord` precedent (`cmp ( 'T 'T -- Ordering )`). The original R1's `&'T`
   receiver could never be called on a bare `42`.
5. **`str` never crosses `extern:`.** The FFI boundary admits
   `Int/Float/Usize/Isize/Ref/Cstr/Bool` only (src/check/declarations.rs:93-101);
   a `str` parameter or return is a located error whose own text says to use
   `cstr` (probes P6a/P6a2). The proven sink binding against real
   `write(2)` is `extern: write ( i32 &array[u8 N] usize -- isize ) "write"`
   (probe P6c; in-tree precedent `examples/resources.sth`'s `read`). `cstr`
   works for literals only (the emitter NUL-terminates literals,
   src/backend/qbe.rs:841). `puts` is wrong three ways: it needs NUL
   termination, it appends a newline, and it rides stdio buffering.
6. **Single-variable member dispatch with extra concrete params works, at
   any position, with `&!` receivers.** Probes P3a/P3b/P4/P7:
   `trait: W['S] : write ( &!'S str -- ) ;` (and the receiver non-last) both
   dispatch through a bounded word, and `'S` grounds per call site across two
   distinct sink types. But the impl registry keys `(TraitId, Type)`: one
   value type has exactly **one** impl per trait — sink variety comes from
   distinct sink *types*, never two impls of one type.
7. **Orphan homes as the original brief assumed** — impl for a scalar or
   builtin-shaped target must live in the trait's module
   (src/check/declarations.rs:491-499; probe P5b's exact error captured); impl
   of an imported trait for a locally-declared type is legal in the local
   module and dispatches cross-module (P5a); a *generic* target post-S4 must
   also live in the trait's module (src/check/declarations.rs:3887).
8. **Member names resolve only through the enclosing word's own bound** — a
   bare `show`/`greet` on a concrete value is `unknown word` (P5a's recorded
   intermediary failure), the same rule `lib/core/cmp.sth` documents for
   `cmp`. Consumers call members through bounded words
   (`: render['T: Show] ( 'T &!StrBuf -- ) show ;`).
9. **Buffering/ordering hazard until S7d:** `.` lowers to stdio `printf`
   (buffered) while `write(2)` is unbuffered — probe P6c's output shows
   write(2) bytes landing *before* buffered `.` output. Any S7c dogfood that
   mixes both must not assert interleaving; S7d should add a golden for the
   post-retirement syscall ordering.

## R1 (revised) — buffer indirection, not a second trait variable

Rust's actual answer to "one value type, many possible sinks" is one
`Display for T` that never touches an I/O device: it writes into an
in-memory `fmt::Write` sink (a `String`), and the *I/O* target is a separate
layer. Sooth's version of the in-memory sink is a **fixed render buffer**
declared in `core::show` — no heap (the allocator is P9), so the buffer is an
ordinary struct over a fixed `array[u8 N]` plus a `len`:

```sooth
\ core::show — no_std, no extern:
type: StrBuf  data array[u8 64]  len usize ;

trait: Show['T]
  : show ( 'T &!StrBuf -- ) ;      \ by-value receiver (fact 4); ONE variable

impl: Show for i64                 \ decimal digits into buf — D1 decides how
impl: Show for usize
impl: Show for isize
impl: Show for bool                \ "true" / "false"

trait: Write['S]
  : write ( &!'S &!StrBuf -- ) ;   \ whole-buffer flush (D2)
```

`Show` impls format into `StrBuf` and never name an I/O target, so exactly one
impl per scalar type exists and the orphan rule puts them all in `core::show`
(fact 7). `Write` impls are per-sink; `type: Stdout` lives in `hosted::libc`
and `impl: Write for Stdout` there (the target-module orphan arm, probe P5a).
The `Write['S]` trait keeps the seam the original brief wanted — an embedded
package later impls `Write for Uart` in its own module for free — but the
per-call-site `'S` grounding the original sketch imagined is gone: with
single-variable traits a value reaches a sink by going *through* `StrBuf`, and
one sink type per `Write` impl (fact 6).

D2 settled on the whole-buffer flush (one `write(2)` per flush, not one per
digit); byte-at-a-time `write ( &!'S u8 -- )` is the probe-proven fallback (P7
ran exactly that shape end to end) if the flush member hits a checker wall at
impl time.

## R2 (revised) — hosted::libc's sink is `write(2)`, not `puts`

```sooth
\ hosted::libc
type: Stdout ;
extern: write ( i32 &array[u8 64] usize -- isize ) "write" ;
```

The array-length `N` in the extern signature must match `StrBuf`'s declared
capacity — the impl body passes `&!buf &data` (the interior `&array[u8 64]`)
and the live `len`. Whether a provenance-carrying `&!data` ref satisfies the
extern boundary's `Ref(..)` admission is a small spec-time probe (P6c proved
the plain `&buf` shape; `is_extern_boundary_scalar` admits `Type::Ref(..)` in
any mode at declaration level, per P6c2). `cstr` remains the literal-only
alternative (P6d/P6d2). See fact 9 for the buffering caveat and fact 5 for why
`puts` is out.

## R3 (resolved) — the scope check the original brief asked for

The original R3 asked whether `show`'s input-bound shape needs anything
S3o/S3p didn't land. Answer, from recon + P1/P2: the two-variable-trait shape
needs a compiler extension S3e explicitly declined (`multi_variable_trait_error`
predates it, and nothing since lifted it), and the sketched bound placement is
additionally a located error — a bound inside a stack effect is rejected for
trait members exactly as for words (src/parser.rs:1668 via
`parse_poly_ty_var`, probe P2b's verbatim text). The redesign within
single-variable traits (R1) is the resolution; extending the trait system is
explicitly out of scope.

## R4 — integer formatting is the second real blocker (D1 decides)

Even with R1's buffer, `Show for i64/usize/isize` needs base-10 digit
extraction, and fact 3 says the primitives do not exist. Options:

- **(a)** Add integer `div` (or a `divmod` pair) rows to `builtin_table` — a
  small compiler change, but the S7d brief calls S7d "the only one that
  touches the compiler"; taking (a) needs an explicit sequencing exception.
- **(b)** Pure-Sooth restoring division in the `Show for i64` impl via
  `shl`/`shr`/sub/comparisons (no `mul` exists, so no magic-number trick) —
  library-only, bounded (~64 iterations), no new compiler surface.
- **(c)** Descope integer `Show` — kills S7b's motivating consumer
  (`expect-eq` reporting actual/expected integers) and is not recommended.

**Decided: (b)** — see D1 below.

## R5 — by-value receivers for the scalar impls

`show ( 'T &!StrBuf -- )`, not `show ( &'T ...)`: scalars have no address
(fact 4), and the targets this slice covers are exactly the scalars that
motivated S7b. Aggregate receivers can come later with aggregate targets.

## Out of scope (updated)

- Multi-variable traits or partially-ground trait dispatch — a compiler
  extension; its own slice with its own recon if ever wanted (R3).
- Any sink beyond `Stdout` (a `Uart` impl is the R1 seam's future consumer,
  not a deliverable here).
- Retiring the compiler-intrinsic `.` — S7d. Note the interplay: S7d must not
  implement its sink on the `str` `Print` arm, which it deletes; S7c binds
  sinks to `write(2)` at the `extern:` boundary from the start.
- Derived/automatic `Show` for user structs/enums; hand-written scalar impls
  only.
- A growable/heap-backed buffer — needs the allocator (P9). `StrBuf` is fixed
  capacity with D3 governing overflow behaviour.
- `Slice[u8]` as a member-parameter or extern-boundary chunk type — not a
  legal member-parameter shape (`member_shape_is_supported`) and not extern
  boundary-admissible.

## Decisions (settled 260829, user-delegated)

- **D1 — division: option (b), pure-Sooth restoring division.** A
  division-by-10 helper inside `core::show` (shift-subtract long division
  over the u64 bit pattern, ~64 bounded iterations) keeps S7c library-only,
  preserving S7d's "the only compiler-touching slice" sequencing note;
  option (a) would add integer `div` rows to `builtin_table` plus per-width
  QBE lowering and tests — real compiler surface for a formatting helper.
  Sign handling: capture the sign, take the magnitude as the u64 bit pattern
  (for negatives `not 1 +`, exact for every `i64` including `MIN`, whose
  two's-complement pattern *is* its magnitude), render the magnitude,
  prepend `-`.
- **D2 — `Write` member: whole-buffer flush, `write ( &!'S &!StrBuf -- )`.**
  One `write(2)` per flush rather than one syscall per digit through the
  `Stdout` impl, the simplest call shape, and the closest match to the Rust
  `fmt::Write` precedent the design cites. The sink does not reset the
  buffer (a fresh `StrBuf` per print; a `reset` helper is optional).
  Short-write/EAGAIN handling is out of scope, consistent with P7.S3c's
  deferred fallible accessors. Fallback if the shape hits a checker wall at
  impl time: byte-at-a-time (probe-proven, P7).
- **D3 — `Show for str` descoped.** Every `str` is literal- or
  static-rooted (there is no runtime constructor and cannot be one before
  the allocator lands, P9), so `cstr` conversion is total for every `str`
  that can exist; S7d prints strings through `cstr` at the boundary, not
  through `Show`. Revisit only if a runtime `str` ever becomes
  constructible.

## Exit (revised)

1. `core::show` (no_std, no `extern:`) declares `StrBuf`, `Show['T]` with a
   by-value receiver, and `Write['S]` (D2's whole-buffer flush); `Show` is
   implemented once each for `i64`, `usize`, `isize` and `bool`, every impl
   inside `core::show` (orphan: scalar targets, fact 7). `str` is descoped
   per D3.
2. `hosted::libc` declares `type: Stdout` and its `Write` impl over the real
   `write(2)` binding (R2's extern shape); a program renders values into a
   `StrBuf` through a bounded word, flushes through `Write`, and observes the
   bytes on stdout.
3. Integer rendering actually formats: `42` rendered through `Show for i64`
   flushes as the digits `42` (D1 resolved — not a placeholder byte).
4. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
