# Sooth — roadmap

Implementation roadmap for the language in [DESIGN.md](./DESIGN.md). Milestones,
not a schedule.

## Current status / next action

Design phase complete (DESIGN.md, Decided section). Backend: **QBE**. Phases 0-6 are
complete and merged to `main` — see each phase file's own header for its exit criteria
and dogfood (Phase 6's S5, nested tag paths, is deferred by design, not owed).
**Phase 7 (language prerequisites for the stdlib) is in progress**: every slice is done
and merged except **S6c** (runtime bounds-checked indexing in a poly body), which still
needs its brief; **S7d** (retiring the intrinsic `.` onto `hosted::show`) is merged, S3u
(trait objects) is parked for want of a consumer, and S3w (the generic Show-backed dot)
is parked as a follow-up to S7d; see
[P7](./P7-language-prereqs.md) for the full slice breakdown. Named-slot-locals sugar
landed (named input slots in a word definition bind locals; see
`docs/named-slot-locals-spec.md`). Per-phase completion history
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
- **Liveness early.** Fast local iteration (`sooth run`) arrives in Phase 1, not at
  the end, for the same reason.
- **No calendar estimates** (they'd be fiction). Effort weights (S/M/L/XL) are
  relative, to show where the mass is.

## Phases

Full detail lives one file per phase, split out because this file had grown past 2500
lines. Each phase file is self-contained: exit criteria, dogfood, slice breakdown.

| Code | Phase | Weight |
| --- | --- | --- |
| **P0** | [Codegen spine](./P0-codegen-spine.md) | `[L]`  ✅ done |
| **P1** | [REPL and liveness](./P1-repl-and-liveness.md) | `[M]`  ✅ done; interactive criteria retired with the REPL |
| **P2** | [Typed core (monomorphic)](./P2-typed-core.md) | `[L]`  ✅ done |
| **P3** | [The linear spine](./P3-linear-spine.md) | `[XL]`  ✅ done — the point of the language |
| **P4** | [Minimal polymorphism + quotations](./P4-polymorphism-quotations.md) | `[L]`  ✅ done |
| **P5** | [Errors as values](./P5-errors-as-values.md) | `[S]`  ✅ done |
| **P6** | [Term-level enum elimination](./P6-enum-elimination.md) | `[L]`  ✅ done (S5 deferred by design) |
| **P7** | [Language prerequisites for the stdlib](./P7-language-prereqs.md) | `[L]` — in progress (S3u/S3w parked; S6c brief outstanding) |
| **P7b** | [Higher-kinded types](./P7b-higher-kinded-types.md) | `[L]` — type-class abstraction over type constructors |
| **P8** | [Packages and modules](./P8-packages-modules.md) | `[L]` |
| **P9** | [The stdlib layers](./P9-stdlib-layers.md) | `[L]` |
| **P10** | [Concurrency (library)](./P10-concurrency.md) | `[M]` |
| **P11** | [Bare metal](./P11-bare-metal.md) | `[M]` — the craft milestone |
| **P12** | [Self-hosting](./P12-self-hosting.md) | `[XL]` |

## Cross-cutting — Tooling and diagnostics  `[ongoing from Phase 0]`

Not a terminal phase. Good, localised compile errors start at Phase 0, for the
author's own write-run-fix loop and for legibility, not for any LLM-authorability
goal (dropped). A formatter and an auto-generated reference doc (word list + stack
effects) once the surface stabilises around Phase 4. An LSP is optional and low
priority for a craft language; add it only if you're using it enough to want it.

## Shape of the risk

- **Phase 0 is done and the go/no-go came back *go***: the virtual-stack → IR → QBE
  → native path holds. **Phase 3** (the linear memory model, the most novel work and
  the reason the language exists) is also done. **Phase 12** (self-hosting) is the
  other large lift and is well understood, but still ahead.
- Phases 4-11 are more independent than the numbering implies. Errors (5) is nearly
  free once ADTs exist (2). Concurrency (10) needs the linear model (3) but little
  else. Bare metal (11) needs the `fixed` layer (9) but not the hosted one. Reorder
  within that band by what you want to play with first, which for a craft project is
  a legitimate way to choose. Two orderings are not free: the stdlib layers (9) are
  built as packages, so they follow packages (8), and packages need Phase 7's bounds
  before an exported signature's API description can be baselined. Higher-kinded types
(7b) need P7.S4's polymorphic impl targets and are independent of P8-P12; take it up
whenever the type-system work is what you want to play with. Phase 12 depends on
  Phase 8 too, but not on 9-11: self-hosting is planned as a progressive, stage-by-stage
  takeover rather than a rewrite-and-cutover, and the FFI boundary that depends on
  (richer `extern:` payloads, unmangled exports) is P8.S4, not new content in Phase 12.
