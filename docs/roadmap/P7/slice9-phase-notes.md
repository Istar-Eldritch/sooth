# P7.S9 — phase notes

Where this slice's phases record what their focus text says to "say in the phase report":
classifications, rulings, and findings a later phase must not rediscover. One section per
phase, added as each lands.

## Phase 1 (R1) — relocate `Library` into `src/driver.rs`

- **`compile_so` was not moved, and no motion was manufactured to satisfy prose.** It was
  already in `driver.rs` (`src/driver.rs:948` at HEAD), so the roadmap's "`compile_so`
  moves into `driver.rs`" is satisfied by construction.
- Relocated from `repl.rs` to `src/driver.rs:971-1032`, beside `compile_so`: the
  `RTLD_NOW`/`RTLD_GLOBAL` constants (both `cfg` arms), the `dlopen`/`dlsym`/`dlerror`
  extern block, `Library`/`open`/`symbol`, and `last_dlerror`. `fflush` stayed behind: it
  flushes the interactive prompt and dies with `repl.rs`.
- **Two doc comments were de-sessioned, not one.** The spec granted the rewrite for
  `Library`'s own doc ("The session keeps every handle resident … callable by later
  ones"). `open`'s doc said "objects loaded by later lines" — the same REPL-line prose, in
  a phrasing E2's grep cannot see, since it matches `session`/`repl` and not `line`. It now
  reads "objects loaded later". Leaving it would have handed phase 10 an unlisted edit.
- `library_opens_and_resolves_a_compiled_symbol` (`src/driver.rs:2494`) is the surviving
  witness. It also carries `repl.rs:3804`'s transmute-and-call (`sq(5) == 25`), so the
  "the resolved symbol is a usable fn pointer" fact does not die with `repl.rs` in phase
  5a; a non-null assertion alone would not have carried it. Mutation-proved in an isolated
  copy: deleting `symbol`'s null check fails the test on "a bad symbol name should error";
  returning `self.handle` in place of the resolved export segfaults.
- Behaviour-neutrality, proved while the REPL is still alive to prove it against:
  `--test repl_ux` 16/16, `--test symbol_hijack` 3/3, full suite green.

### Carried forward, for later phases

- **E2 is already unsatisfiable as worded; phase 11 should amend the criterion, not the
  code.** E2 expects the grep to return "only `src/driver.rs`'s relocated
  `dlopen`/`dlsym` extern declarations". Post-move, `driver.rs` legitimately says `dlopen`
  in five more places: `open`'s error message (`:1002`), `symbol`'s SAFETY comment
  (`:1010`), the pre-existing `TempDir` scratch-dir doc (`:1036`) and the round-trip test
  (`:2506`) — plus the call itself (`:999`). Only the `dlopen` line of the extern block
  matches at all: `dlsym` is not one of E2's alternatives, so the criterion's own prose
  names a string it never greps for. The residual set to expect at slice end is
  `driver.rs`'s `Library` plumbing plus `TempDir`'s doc, six hits; `driver.rs`'s other
  current hits (`:510`, `:890`, `:1264`, `:2021` prose, `:940`/`:943` `driver::repl`) die
  in phases 5a and 10 as scheduled.
- **`cargo clippy --all-targets -- -D warnings` is already red at HEAD**, in three places
  this slice does not own: `tests/phase4_combinators.rs:2499` (`507e0b7`) and `:2561`
  (`e4d43a2`), both `needless_borrow`, and `src/parser.rs:10351` (`1ccf370`),
  `bool_comparison`. CLAUDE.md's green is `clippy -- -D warnings`, which is clean, so this
  is not a regression and not this slice's to fix — but phases 2 through 4 edit tests
  heavily and will hit it if they widen the command.
