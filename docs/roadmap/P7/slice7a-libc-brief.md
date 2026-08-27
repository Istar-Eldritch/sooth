# Phase 7 Slice 7a: `lib/core` / `lib/hosted` package split, and `lib/hosted/libc.sth` (brief)

Prerequisite for S7b–S7d. Today `lib/` is one package (`layer: core`) holding every
stdlib module, including `bool`/`cmp`/`prelude` (genuinely `no_std`) alongside
whatever `hosted`-only content lands next (testing, `exit`, eventually `Write`'s
libc backend). Splitting the directory now, before that content exists, avoids
retrofitting the layer boundary DESIGN.md already promises ("fix the layer
boundaries ... on day one").

## Design rulings

### R1 — `lib/core` is a rename, not a rewrite

`lib/bool.sth`, `lib/cmp.sth`, `lib/prelude.sth`, `lib/combinators.sth`,
`lib/option.sth`, `lib/result.sth` and `lib/sooth.pkg` move to `lib/core/` with no
content change beyond `sooth.pkg`'s own path bookkeeping. Every `depends: core path
"..."` entry across `examples/` and any future consumer package updates its
relative path. `layer: core` is unchanged.

### R2 — `lib/hosted` is a new package, one layer step up

`lib/hosted/sooth.pkg` declares `package: hosted ; layer: hosted ; depends: core
path "../core" ;`. Its `module:` list starts with `libc` (this slice) and grows
with `testing` (S7b) and eventually the `Write` impl for stdio (S7d) — every
`hosted`-layer stdlib module lives here, not scattered per-example the way
`extern:` bindings are today.

### R3 — `lib/hosted/libc.sth` holds `exit`, and only what's actually shared

    extern: exit ( i32 -- ) "exit" ;
    export: exit ;

Nothing else moves here yet. `examples/strings.sth`'s `strlen`/`puts` and
`examples/resources.sth`'s `open`/`read`/`close-fd` stay where they are: per the
project's elevate-to-lowest-common-ancestor rule, nothing today shares those
bindings, so centralizing them has no consumer to justify it. `exit` is the one
`extern:` binding this phase's own work (S7b's `sooth test`, S7d's diagnostics)
will actually want from more than one place.

### R4 — Why `exit` at all, now

The original testing brief (R2, pre-split) deferred `exit` as "a separate, tiny
slice" once a *future consumer* wanted process-level failure rather than the TAP
protocol. That consumer is S7b: a test file wanting to abort a suite early (a
fatal precondition, not an assertion) has no way to signal it beyond a trap, which
prints a Rust-side panic message instead of a test-shaped one. Landing `exit` first
as its own tiny slice keeps that decision out of S7b's diff.

## Out of scope

- Any other libc binding beyond `exit` (R3).
- The `Show`/`Write` trait pair (S7c) and the hosted `.` (S7d) — this slice is pure
  package plumbing plus one `extern:` line.

## Exit

1. `lib/core/` and `lib/hosted/` exist as sibling packages; every existing
   `depends: core` entry across the tree resolves under the new path.
2. `lib/hosted/libc.sth` exports `exit ( i32 -- )`; a program depending on
   `hosted` can call it and observe the process exit code.
3. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green with
   no other behavior change.
