# Non-regression baseline: shared `terms.rs` fall-through diagnostics

Captured 2026-09-07, HEAD `7404a71` (main, no `src/` changes). Byte-exact
stderr for the two probes that exercise the same fall-through path S9/S10's
`foreign_single_candidate_grounding` and the ordinary env-miss unknown-word
route share (`src/check/terms.rs`, the `Call` arm's `env.get`-miss branch
around line 900). An implementation pass on the S2-9-follow-up spec must
diff against these and show zero collateral change to any S9/S10 golden.

## dp_c — minimal repro (`1 Ok drop`, no other `Res` mention in the module)

Command: `cargo run -q -- run probes/dp_c.sth --manifest tests/fixtures/sooth.pkg`
Exit: `1`

```
error: unknown word `Ok` in `main` (line 6)
```

## dp_d — nested case (`1 mkok Ok drop`, one `Res[i64 i64]` monomorph already minted by `mkok`)

Exit: `1`

```
error: type mismatch in `main` (line 8)
  `Ok` expected `i64`, found `Res[i64 i64]`
  note: declared ( -- )
```

Note: dp_d does **not** reproduce the same diagnostic class as dp_c (see
`dp_findings.md` Q1) — it is a different failure mode (silent single-mint
resolution to the wrong monomorph), not the "zero candidates" case dp_c hits.
Both are recorded here since both are candidate fall-through shapes a fix
could disturb.

## dp_e / dp_e2 — explicit type-args on a bare generic ctor (both variants, consumed or not)

Exit: `1` (both, byte-identical modulo line number)

```
error: `Ok` (line N) takes no type arguments; only a call to a polymorphic word may be explicitly instantiated
```

This is a **different gate** (`poly_call_takes_type_args` /
`no_type_arguments_error`, `src/check/terms.rs:215`, `src/check.rs:1445`),
upstream of the grounding fall-through entirely. Any B-shaped fix that wants
to admit `Ok[i64 i64]` as a call form has to widen this gate too, not just
the grounding fall-through `dp_c`/`dp_d` exercise.
