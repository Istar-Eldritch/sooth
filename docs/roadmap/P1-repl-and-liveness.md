[← ROADMAP](./ROADMAP.md)

### Phase 1 — REPL and liveness  `[M]`  ✅ **done**

No in-process JIT (that left with LLVM), and no comptime interpreter (there are no
immediate words; see DESIGN Declined). The REPL runs on the **backend** via `dlopen`:
each new word is compiled to a shared object and loaded into the live session, so the
process holds natively-compiled code it can call at once; redefinition loads a new
object and swaps the name→symbol entry. Whole-program `run` uses compile-to-binary +
subprocess. Factor's in-image model minus the sub-millisecond compile, without owning
a backend.
**Exit (retired):** the interactive criteria (define and test words
interactively, redefinition, a live throwaway-but-real session) have no form
without the REPL. Phase 1's surviving language facts are `sooth run` goldens:
word definition and call (`tests/phase0.rs`, `gcd_compiles_and_runs`,
`factorial_compiles_and_runs`), `| a b |` locals (`tests/phase3_locals.rs`,
`mid_body_binding_consumes_from_the_stack`,
`mid_body_binding_leftmost_name_takes_deepest_value`), and the linear
discipline (`tests/phase0.rs`, `explicit_drop_runs_destructor_once`,
`surplus_linear_on_stack_is_error`,
`drop_of_linear_struct_runs_field_glue_in_declaration_order`).
**Dogfood (retired):** the interactive calculator session has no whole-program
form. Its language facts, a `| n |`-bound local, `sq`/`neg` definition and
call, `mul`/`sub`/`add` and `.`, are covered by the goldens above and by
`tests/phase0.rs`'s `countdown_dogfood_runs_in_constant_stack`.
