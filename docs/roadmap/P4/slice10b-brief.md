# Phase 4 Slice 10b: `times` moves to the library (brief)

`docs/phase4-slice10-brief.md` scoped this as "the migration, separate slice, 8c-shaped
lightweight process: delete the special case, let the suite find the fallout." 10a shipped
(`e87bcae`) and proved the mechanism (a user-space `my-times` compiles, sums, runs a
million iterations at constant stack, carries an aggregate without aliasing). 10b spends
that mechanism on the real thing: `times` becomes ordinary Sooth source, and the intrinsic
(`check_abstract_quotation_times`, the `check_term` interception arm, the `ir.rs` lowering
arm) is deleted outright, not kept as a fallback.

This lands right after 6g (merged `86aee0a`), per `ROADMAP.md:608`'s own "Next action"
pointer: `each`/`map`/`fold`/`filter` all call `times` directly, so once `times` is itself a
combinator, calling any of them nests one combinator splice inside another, which is exactly
the shape 6g's R1/D1/R2 exist to make sound. 10b is the first real caller of that fix, not a
hypothetical beneficiary.

## Recon (measured against the built compiler, 2026-08-13, `main` at `86aee0a`)

1. **The intrinsic's own check-time interception**: `src/check/terms.rs:321-459` (the whole
   `if name == "times" { ... }` block), plus `check_abstract_quotation_times`
   (`src/check/terms.rs:1246-1280`) for the abstract-parameter path (a declared quotation
   parameter passed down rather than a known literal). Both are gated on the literal string
   `"times"`, unconditionally, whenever nothing else has already claimed the name (recon 2).
2. **The intrinsic's own lowering**: `src/ir/func_builder/calls.rs:321-418`, a dedicated
   `"times" =>` match arm. It shares `begin_loop`/`finalize_loop` with the generic self-tail
   combinator lowering (`lower_self_tail_combinator`, `calls.rs:155`, what `while` rides) but
   does not call it: its own back-edge push, `tail = false`, its own loop-state save/restore.
3. **A same-module user word literally named `times` already wins over the intrinsic today,
   no core-compiler change needed for dispatch priority.** Verified directly: `: times
   ( -- ) 99 . ; : main ( -- ) times ;` compiles and runs the user body (prints `99`), not
   the intrinsic (which would reject the call outright: it pops a quotation and a count that
   are not there). Consistent with `resolve::mangle` rewriting a call site to a user
   definition's mangled name before `check_term`'s literal-name dispatch ever sees the bare
   string (see the memory entry on `resolve::mangle` being unconditional per module). This
   means 10b needs no dispatch-priority change: define `times` in `lib/combinators.sth`,
   have a caller import it, and that caller's bare `times` calls resolve to the library word.
4. **This makes `times` an ordinary opt-in import, not an always-available builtin, which is
   a real regression for anything that does not already import `combinators.sth`.**
   Measured: seven files call bare `times` today with no such import: `examples/array_ctor.sth`,
   `examples/array_totals_hand.sth`, `examples/combinator_in_times_hand.sth`,
   `examples/filter_while_hand.sth`, `examples/inplace_fold.sth`, `examples/times.sth`, and
   `lib/arrays.sth`. Each stops compiling the moment the intrinsic is deleted, unless it
   gains an import (or is rewritten).
5. **The Rust test suite's blast radius is larger, and only measurable by actually deleting
   the intrinsic.** Eleven test files reference `times` in inline `.sth` source strings:
   `phase0.rs`, `phase3_refs.rs`, `phase4_combinators.rs`, `phase4_generics.rs`,
   `phase4_quotations.rs`, `phase4_slice10a_exit_witnesses.rs`,
   `phase4_slice10a_inline_quotation.rs`, `phase4_slice6f.rs`, `phase4_slice6g.rs`,
   `phase4_slice6h.rs`, `qbe_baseline.rs`. Deleting the intrinsic and swapping
   `each`/`map`/`fold`/`filter` onto a library `times`, with nothing else changed, reds
   **22 tests in `tests/phase4_combinators.rs` alone**: `unknown word \`times\`` for the
   bare-source ones, assertion mismatches for the ones pinning the intrinsic's own bespoke
   diagnostic wording. The other ten files are unmeasured; each needs the same sweep before
   a phase plan can be sized.
6. **`tests/phase4_slice10a_exit_witnesses.rs` is retired by this slice, not merely edited.**
   Its whole premise, that `my-times` compiles "beside the untouched intrinsic," stops being
   true the moment the intrinsic is deleted: `my_times_compiles_beside_the_untouched_intrinsic_and_sums`
   literally asserts both `my-times` and the real `times` sum correctly in the same binary.
   Its constant-stack, aggregate-carrying, and nesting witnesses (R15-R19) must each be
   either deleted as subsumed by 10b's own goldens on the real `times`, or kept and renamed as
   coverage of the row mechanism in its own right, independent of `times`. The spec must
   decide this file test by test, not leave it half-obsolete.
7. **A library `times`, self-tail-recursive on a from/to pair, reproduces the intrinsic's
   exact signature and semantics.** Verified end to end, build and run, in a scratch tree
   with the intrinsic fully deleted:

   ```sooth
   : times-helper ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )
     | f | | to | | from |
     from to < if
       from f call
       from 1 + to f times-helper
     else
     end ;

   : times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )
     | f | | n | 0 n f times-helper ;
   ```

   `0 5 [ + ] times` sums to `10`; `0 1000000 [ drop 1 + ] times` completes (unconstrained
   stack in this probe; the constant-stack-under-`ulimit` witness itself belongs to 10a's
   `my-times` test, not yet re-run against this exact word, see exit criteria). This is
   10a's own `my-times` (`tests/phase4_slice10a_exit_witnesses.rs:63`) wrapped so its two
   visible counters (`from`/`to`) collapse into one visible count with an internally
   synthesized starting index of `0`. `times-helper` needs no export: `times` is its only
   caller.
8. **The general mechanism still rejects the intrinsic's bespoke body-safety violations, with
   reworded diagnostics, not silently.** Verified: a body that drops too much of the row
   (`[ drop drop ]` against a one-element row) fails with `` the quotation passed to `times`
   was declared `~[ i64 -- ]` but its body has effect `[ i64 -- ]` ``, a 10a-style declared-
   effect mismatch, not the retired `times_body_row_effect_error` wording. Not yet verified
   for the other two bespoke diagnostics (`times_body_consumes_local_error`,
   `times_body_borrow_across_loop_error`): both should already be subsumed by the ordinary
   per-combinator move/borrow tracking every `inline_combinator` body gets (nothing
   intrinsic-specific backed the *check*, only the message), but the spec must confirm each
   with its own probe, not assume by analogy to the one that was checked.
9. **Deleting the intrinsic leaves exactly four dead diagnostic functions**, confirmed by
   `cargo build` warnings once both interception sites are removed: `times_needs_quotation_error`
   (`terms.rs:1314`), `times_body_consumes_local_error` (`:1330`),
   `times_body_borrow_across_loop_error` (`:1341`), `times_body_row_effect_error` (`:1351`).
   All four delete cleanly; nothing else references them.
10. **`each`/`map`/`fold`/`filter`'s own splice depth genuinely grows by one**, confirmed
    structurally: all four call `times` directly (`lib/combinators.sth:22,27,32,47`). Once
    `times` is itself a combinator (`is_combinator` true), a call to `each` splices a body
    that itself contains a combinator call, which now needs its own `inline_combinator`
    splice at check time. This is exactly the nesting shape 6g's R1/D1/R2 were built to make
    sound, and it is now exercised for real on every existing `each`/`map`/`fold`/`filter`
    call site in the corpus, not just in a purpose-built test.

## Decisions

1. **`times` is a thin public wrapper over a private self-tail-recursive `times-helper`
   carrying a from/to pair, not a single word recursing on one synthesized index.** There is
   no way to carry "the next index to hand the body" as loop state while also handing the
   body a value that starts at `0` without a second counter: the from/to shape is what 10a's
   own `my-times` proved out, and `times` is that shape with `from` seeded at `0` and `to`
   the caller's count.
2. **Only `times` is exported from `lib/combinators.sth`; `times-helper` is not.** Nothing
   outside `times` itself calls it.
3. **Every corpus file identified in recon 4 gets an explicit import added, not a rewrite to
   avoid `times`.** This matches the existing precedent: `each`/`map`/`fold`/`filter` already
   require `import: c "lib/combinators.sth" ;` (or the module's own equivalent) to use, and
   `times` becoming an ordinary export of the same file is one more name on that same import.
4. **`tests/phase4_slice10a_exit_witnesses.rs` is explicitly resolved as part of 10b, test by
   test.** Left alone, several of its assertions become false the day the intrinsic is
   deleted; "let the suite find the fallout" does not excuse leaving a file whose own stated
   premise is retired.
5. **The four dead diagnostic functions are deleted outright, not repurposed.** The original
   10-brief's decision 7 already calls for the intrinsic's bespoke messages to become the
   general combinator ones; recon 8 confirms the general path already produces a rejection
   for at least one of the three bug classes, with a different, not worse, message.
6. **Corpus program output (stdout, exit code) stays byte-identical everywhere `times` is
   used.** Source text may change (an added import); a runtime value may not.

## Open questions for the spec

- Whether the two unprobed bespoke diagnostics (`times_body_consumes_local_error`,
  `times_body_borrow_across_loop_error`) are genuinely still caught, with some rejection, by
  the general per-combinator move/borrow tracking, the way the row-mismatch case already was
  (recon 8). Construct both probes against the real `times-helper` shape before the spec
  locks a phase plan around "diagnostics re-point for free."
- The binary-size delta measurement: which files under `examples/`, what counts as an
  acceptable delta versus a red flag. The original 10-brief says this must be "measured and
  recorded"; it does not say how.
- Full enumeration of the fallout in the other ten touched test files (recon 5 only measured
  `phase4_combinators.rs`). Sweeping all eleven is necessary before the spec can size an
  effort estimate or a phase split.
- Whether the REPL needs any accommodation. Three of the 22 measured failures were
  REPL-specific (`repl_two_output_combinator_define_and_call`, `repl_imported_filter_runs`,
  `repl_combinators_dogfood_matches_native`). `src/repl.rs` was explicitly out of scope for
  6g's D5 fix; determine whether it is similarly out of scope here, or whether an
  interactive session needs some ergonomic path to `times` that a `.sth` file does not.
- Whether `lib/arrays.sth`'s bare `times` calls should get an import of `combinators.sth`, or
  whether `times` should be defined directly in `arrays.sth` too. Check whether a
  library-to-library import (as opposed to an example importing a library) is even
  precedented in the current corpus before assuming the former.
- Whether `times-helper`'s own row-preservation and move/borrow checks, once it is an
  ordinary self-tail combinator rather than a compiler-blessed one, actually exercise 10a's
  decision 3 and decision 5 (the back-edge arm's ground-declared-outputs rewrite, and its
  obligation to forward a slot's `surviving` set rather than manufacture a fresh one). `times`
  is state-threading in exactly the shape decision 5 was written about; confirm directly
  against this word, not by inference from `while`'s or `my-times`'s own coverage.

## Out of scope

- Rewriting `while`, `each`, `map`, `fold`, or `filter`'s own bodies beyond the required
  `times` import; their signatures and semantics are unchanged. (Already out of scope per
  the original 10-brief; still true.)
- Any change to `lower_self_tail_combinator` or the generic loop-lowering machinery; the
  migrated `times-helper` becomes another customer of it, unmodified.
- The `~` type's call-versus-bare-mention semantics question 10c left open. `times-helper`
  only ever invokes its quotation parameter via `f call`, the same pattern `while`'s `p call`
  already uses; nothing here needs that question settled.
- Any change to the loop-counter's type (`Bound::Int`, the original 10-brief's decision 6).
  The migrated `times` keeps `i64`, exactly like the intrinsic.

## Sequencing

After 6g (merged `86aee0a`), per `ROADMAP.md:608`. Independent of 10c, which gates on 10a
phases 1 through 4 only, not on 10b; the two may land in either order.

## Exit

- `times` compiles as ordinary Sooth source in `lib/combinators.sth`; `check_abstract_quotation_times`,
  the `check_term` interception arm, the four dead diagnostic functions, and the `ir.rs`
  lowering arm do not exist.
- Every corpus `.sth` file that used bare `times` compiles again (with an added import) and
  produces byte-identical stdout and exit code to `main` at `86aee0a`.
- The full test suite, all eleven touched files, is green; `tests/phase4_slice10a_exit_witnesses.rs`
  is explicitly resolved (deleted or kept, with a stated rationale), not left broken.
- The 1M-iteration constant-stack witness is re-run against the real `times` word (not just
  `my-times`) at `ulimit -s 1024`.
- An aggregate-carrying-through-the-row witness is re-run against the real `times` word to
  confirm decisions 3 and 5's guarantees (ground declared outputs, forwarded `surviving` set)
  hold for this exact word, not only for `my-times`'s isomorphic but distinct shape.
- A `times` call nested inside `each`/`map`/`fold`/`filter` (the newly doubled splice depth),
  and a `times`-driven combinator nested inside an outer loop (`while`, or another `times`),
  both produce correct values under 6g's fix. This is the single most novel risk this
  migration introduces beyond what 10a proved, and needs its own explicit golden, not just
  reliance on the existing library test suite passing once the missing imports are added.
- The binary-size delta across `examples/` is measured and recorded, not skipped.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` all green.
