# Sooth — roadmap

Implementation roadmap for the language in [DESIGN.md](./DESIGN.md). Milestones,
not a schedule.

## Current status / next action

Design phase complete (DESIGN.md, Decided section). Backend: **QBE**. Phases 0-5 are
complete and merged to `main` — see each phase file's own header for its exit criteria
and dogfood. **Phase 6 (term-level enum elimination) is in progress**: Slices 1-2
(quotation effect annotations; variant types and accessors) are done and merged; see
[P6](./P6-enum-elimination.md) for the full slice breakdown. Per-phase completion history
(what shipped in which slice,
defects found and fixed in review) lives in each phase's own file and its
`docs/roadmap/P{N}/` briefs/specs, not here.

Host language is Rust — no longer forced by LLVM/Z3 (both dropped), so it is a free
choice in principle, but nothing motivates switching.

## Guiding principles

- **De-risk novel-before-laborious.** Prove the uncertain, novel parts (the codegen
  model, then the linear memory model, which is the whole point of the language)
  early. The larger-but-understood parts (stdlib, self-hosting) can wait.
- **Vertical slices with a dogfood program each phase.** Every phase ends with a
  language you can run a real (if small) program in, and you actually write that
  program. This is the antidote to the failure mode named in DESIGN.md: a beautiful
  half-built compiler no one writes code in. If a phase produces no runnable
  program, the phase isn't done.
- **Liveness early.** A REPL and immediate feedback arrive in Phase 1, not at the
  end, for the same reason.
- **No calendar estimates** (they'd be fiction). Effort weights (S/M/L/XL) are
  relative, to show where the mass is.

## Phases

Full detail lives one file per phase, split out because this file had grown past 2500
lines. Each phase file is self-contained: exit criteria, dogfood, slice breakdown.

| Code | Phase | Weight |
| --- | --- | --- |
| **P0** | [Codegen spine](./P0-codegen-spine.md) | `[L]`  ✅ done |
| **P1** | [REPL and liveness](./P1-repl-and-liveness.md) | `[M]`  ✅ done |
| **P2** | [Typed core (monomorphic)](./P2-typed-core.md) | `[L]`  ✅ done |
| **P3** | [The linear spine](./P3-linear-spine.md) | `[XL]`  ✅ done — the point of the language |
| **P4** | [Minimal polymorphism + quotations](./P4-polymorphism-quotations.md) | `[L]`  ✅ done |
| **P5** | [Errors as values](./P5-errors-as-values.md) | `[S]`  ✅ done |
| **P6** | [Term-level enum elimination](./P6-enum-elimination.md) | `[L]` — in progress (Slice 2) |
| **P7** | [Stdlib and `no_std` layering](./P7-stdlib-nostd.md) | `[L]` — where it becomes usable for real programs |
| **P8** | [Concurrency (library)](./P8-concurrency.md) | `[M]` |
| **P9** | [Bare metal](./P9-bare-metal.md) | `[M]` — the craft milestone |
| **P10** | [Self-hosting](./P10-self-hosting.md) | `[XL]` |

## Cross-cutting — Tooling and diagnostics  `[ongoing from Phase 0]`

Not a terminal phase. Good, localised compile errors start at Phase 0, for the
author's own write-run-fix loop and for legibility, not for any LLM-authorability
goal (dropped). A formatter and an auto-generated reference doc (word list + stack
effects) once the surface stabilises around Phase 4. An LSP is optional and low
priority for a craft language; add it only if you're using it enough to want it.

The REPL (`src/repl.rs`, `src/editor.rs`) grew a hand-rolled raw-mode line editor
(prompt, cursor movement, history, Ctrl-C/Ctrl-D handling), multi-line continuation
for an open `:`/`type:` definition or bracket, typed/rich stack rendering on the tty
path, and `:help`/`:words`/`:type`/`:stack`/`:clear` meta-commands with tab
completion. The piped (non-tty) path is unchanged byte-for-byte.

## Shape of the risk

- **Phase 0 is done and the go/no-go came back *go***: the virtual-stack → IR → QBE
  → native path holds. **Phase 3** (the linear memory model, the most novel work and
  the reason the language exists) is also done. **Phase 10** (self-hosting) is the
  other large lift and is well understood, but still ahead.
- Phases 4-9 are more independent than the numbering implies. Errors (5) is nearly
  free once ADTs exist (2). Concurrency (8) needs the linear model (3) but little
  else. Bare metal (9) needs the `fixed` layer (7) but not the hosted one. Reorder
  within that band by what you want to play with first, which for a craft project is
  a legitimate way to choose.
