# CLAUDE.md

Guidance for agents working on Sooth. Keep it short. This is a **craft** project
(see [DESIGN.md](./DESIGN.md) for why), so resist importing production-scale process
and structure. If a change starts to look like a big SaaS codebase, stop.

Sooth is a small, statically-checked concatenative language compiled via QBE. See
[ROADMAP.md](./ROADMAP.md) for the phased plan.

## Build / test / run

```sh
cargo build
cargo test
cargo run -- build examples/gcd.sth
```

"Green" means: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Implementation lifecycle

Handled by **.pi-spec-pipeline** (spec → phased implementation → review → commit).
Do not hand-roll a parallel workflow; specs and phases arrive through the pipeline.
This file defines the *conventions the work must satisfy*, not the process that
produces it.

## Test coverage (a convention, not a percentage gate)

- Every stage function (lex / parse / check / lower / emit) gets unit tests beside it
  (`#[cfg(test)] mod tests`): the happy path plus at least one error/edge case.
- Every roadmap phase exit criterion is a **golden test** (source in → expected output,
  or source in → expected diagnostic out). `gcd.sth` and `factorial.sth` are the
  Phase 0 goldens.
- Diagnostics are behaviour: test that bad input produces the *right error*, not just
  that it fails. Turning Forth's silent failures into sharp compile errors is the point,
  so the errors are part of the spec.
- Naming: `thing_condition_expected` (e.g. `check_branch_depth_mismatch_is_error`).
- A phase is not done until its goldens pass and its new stage code has unit coverage.

## Growth structure (start small, let it split under pressure)

- Start in one file. Split a module out only when complexity actually emerges, never
  preemptively.
- Group by responsibility (a compiler stage), not by technical layer.
- **Elevate shared code to the lowest common ancestor**: when 2+ modules need it, move
  it up to the nearest shared parent, and no higher.
- Keep code that changes together in the same place.
- Refactor (split/elevate) when **2+ of these signals** appear together: import
  divergence (different sections of a file pull different dependencies); a module doing
  X and Y and Z; high- and low-level code mixed in one file; functions in a file that
  never call each other; a split forced by a would-be circular dependency.

## Load-bearing invariants (do not break silently)

- Backend is **QBE**; no LLVM. A hand-written native backend is deferred, not ruled
  out (reconsider after self-hosting); do not start one without that decision.
- IR stays **backend-neutral**: `Ptr[T]` is an opaque handle, never assumed to be a
  `u64` (a future WASM lowering depends on this).
- The **affine spine** is the point: `dup` is the explicit copy, drop is a
  statically-known destructor point.
- `core` is **`no_std`**; layers are `core` / `fixed` / `alloc` / `hosted`.
- **No in-process JIT** and no comptime interpreter; the REPL loads freshly compiled
  words in-process via `dlopen` (there are no immediate words).
