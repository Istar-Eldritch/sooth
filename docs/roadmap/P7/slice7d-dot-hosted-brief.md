# Phase 7 Slice 7d: retire the compiler-intrinsic `.`, in favor of a `hosted` word (brief)

`.` is a compiler-injected, name-dispatched builtin (`is_name_dispatched_builtin`,
`resolve.rs`) that lowers straight to libc `printf`/`dprintf` regardless of which
package calls it — an OS dependency baked into the compiler itself, invisible to
the layer system every other hosted capability goes through `depends:` to reach.
S7c gives `Show`/`Write` a real, layered home for printing; this slice moves `.`
onto that path and deletes the special case.

Ordered last among the S7 subslices because it's the only one that touches the
compiler rather than the standard library, and every other S7 deliverable
(`exit`, `expect`/`expect-eq`, `Show`/`Write`) works whether or not this one ships
— nothing here blocks S7a–S7c, and if this slice turns out larger than expected it
can slip without stalling the testing vocabulary that motivated the whole split.

## Design rulings

### R1 — `.` becomes `hosted::show`'s word, not a new compiler primitive

`. ( 'T:Show -- )` is an ordinary `hosted` word: `&Stdout show` under the hood, `'T`
resolved through `Show` (S7c) the same way any other trait member call resolves.
Every existing call site (`42 .`, `core::bool`'s `.` overload, the QBE emitter's own
diagnostic/trace paths at `qbe.rs:892-1283` that currently call `printf` directly)
either goes through the new word or, for the compiler's own trap/OOM/bounds
diagnostics, stays a direct `printf`/`dprintf` call — those are backend-internal
panic messages, not user-facing `.`, and are explicitly not in scope for
rerouting through a user-level trait.

### R2 — What actually gets deleted

`is_name_dispatched_builtin`'s `.` arm, the `resolve.rs` special case that lets
`.` bypass ordinary env lookup, and whatever check.rs/lowering path currently
special-cases `.`'s effect. `core::bool`'s existing `.` overload note in
`prelude.sth` ("One of `core::bool`'s names does not appear here... an operator
overload's candidate lookup considers the calling module and the module it
selectively imported the name from") needs re-verification once `.` is an
ordinary word going through the same operator-dispatch machinery as everything
else it documents — confirm the one-hop rule still produces the same outcome
before assuming the note's reasoning survives unchanged.

### R3 — Migration is program-wide, not opt-in

Once this slice lands, every program that prints must `depends: hosted` and
`import: hosted::show ...`. No compatibility shim keeps `.` working without the
import — CLAUDE.md's magicless-over-convenience rule applies directly: a program
that types a value into existence without importing anything is exactly the
implicit behavior the rest of Sooth refuses. `examples/*.sth` and every golden
that currently prints without an explicit import migrates in this slice, not
gradually.

## Out of scope

- Any sink beyond `Stdout` (S7c's scope, unchanged here).
- The backend's own internal `printf`/`dprintf` diagnostics (trap messages, OOM,
  bounds) — R1.

## Exit

1. `is_name_dispatched_builtin` and the `resolve.rs`/check.rs special cases for
   `.` are deleted; `.` resolves as an ordinary `hosted::show` word.
2. Every example, golden, and test that prints imports `hosted::show` explicitly;
   none compile via an implicit intrinsic.
3. `core::bool`'s `.` overload note in `prelude.sth` is re-verified or corrected
   (R2).
4. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
