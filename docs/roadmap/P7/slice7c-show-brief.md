# Phase 7 Slice 7c: `Show`/`Write`, a sink-generic printing pair (brief)

S7b `[ done ]` shipped `hosted::testing`'s `expect`/`expect-eq`, label-only, printing
through the still-intrinsic `.` (`lib/hosted/testing.sth`); its dogfood suite is
`examples/tests/`. `Show` is the obvious next want once that consumer's assertions can
only report a label, not the actual/expected values. The naive shape — `trait: Show ; show ( &'T -- ) ;`
with an `impl: Show for i64` living wherever prints to stdout — is unsound in
Sooth's own trait system before it's unsound anywhere else: the orphan rule
(`check_impl_decls_orphan_scalar_target_names_only_the_trait_module`) forces *any*
impl for a scalar target into the trait's own module, so a `hosted`-libc `Show for
i64` and a hypothetical embedded-UART `Show for i64` can never coexist as two
impls of the same trait — that's a `(TraitId, PolyType)` duplicate the moment both
are imported into one program, not a hypothetical.

Rust's actual answer to "one value type, many possible sinks" is not multiple
`Display` impls: it's one `Display for T` that never touches an I/O device,
parameterized over an abstract sink (`fmt::Write`) whose *impls* vary per target.
This slice ports that split, not the naive one-trait version.

## Design rulings

### R1 — Two traits, not one: `Write` is the sink, `Show` calls it

    trait: Write['S] write ( &!'S str -- ) ;
    trait: Show['T]  show  ( &'T &!'S:Write -- ) ;

`Show for i64`/`str`/`bool`/etc. has exactly one impl, living in `core::show`
alongside the trait declarations (the orphan rule's legal home for a scalar
target), and it never names a concrete sink — it calls `write` on whatever `'S:
Write` the caller bound. `core::show` itself stays `no_std`: no `extern:`, no I/O,
just formatting logic that produces `str` chunks and hands them to the bound sink.
The only per-target thing left to vary is `Write`'s impl, and *that* has an owning
module per target (`hosted::libc`'s `Stdout`, an embedded package's `Uart`), so the
orphan rule is satisfied for free — no coherence conflict, because `Write for
Stdout` and `Write for Uart` are impls of *different types*, not the same type
twice.

### R2 — `hosted::libc` provides the one sink this phase needs

    type: Stdout ;
    impl: Write for Stdout write ( &!Stdout str -- ) | s | drop puts drop ;

(`puts`/equivalent already exists per-example as an `extern:` binding; S7c
promotes it into `hosted::libc` alongside `exit` if it doesn't already live
there.) No other sink ships in this slice — a UART `Write` impl is a real want for
an eventual embedded package, but nothing in P7 has an embedded target yet, so
it's a pointer, not a deliverable here.

### R3 — Scope check against P7.S3o/S3p before committing to the signature

`show`'s signature binds a trait as an *input parameter's* bound (`&!'S:Write`),
which is exactly the shape P7.S3o/S3p were about (a member declaring its bound
variable at an input position, and dispatch on a poly combinator's own type
variable). Confirm against those slices' closed mechanism before writing `Show`'s
checker path — if the two-parameter-trait shape needs anything S3o/S3p didn't
already land, that gap is this slice's real risk, not the trait declarations
themselves.

## Out of scope

- Any sink beyond `Stdout` (R2).
- Retiring the compiler-intrinsic `.` in favor of `Show`/`Write`: S7d.
- Derived/automatic `Show` for user structs/enums (a `derive`-shaped mechanism);
  this slice hand-writes `Show` for the scalar core types only.

## Exit

1. `core::show` declares `Write` and `Show`; `Show for i64/usize/isize/bool/str`
   (the printable core scalars) is implemented once, in `core::show`, with no
   `extern:` and no sink-specific code.
2. `hosted::libc` implements `Write for Stdout`; a program can write
   `42 &Stdout show` (or the equivalent call shape the checker settles on) and see
   `42` on stdout.
3. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
