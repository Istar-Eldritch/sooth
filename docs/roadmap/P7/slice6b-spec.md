# P7.S6b -- Explicit length arguments at word call sites

**Status:** Implemented (commits below).

## What & why

A word signature could already *declare* a length variable
(`sum['T 'N: Len] ( array['T 'N] -- 'T )`, parseable since S6a) and a caller
could bind it *by inference* (`sum[i64]` seeds `'T`; `'N` is read off a
concrete array operand's count). What was missing was syntax to bind that
length **explicitly** at the call site: `sum[i64 4]` did not parse. This slice
is the length-variable parallel of the existing explicit type-argument path,
narrowly scoped to word call sites.

Key decisions and their rationale:

- **Parser disambiguation is lexical, not arity-driven.** A word call goes
  through `parse_explicit_type_args`, which has no `PolySig` at parse time and
  cannot split type-vs-length by declaration position. Instead the grammar
  fixes its own convention: parse type expressions greedily until an integer
  token appears, then parse integers as `Len::Concrete` until `]`. A type token
  after an integer is a parse error. Position `i` in the length sublist indexes
  `len_var_names[i]` and in the type sublist indexes `ty_var_names[i]`,
  independent of how the callee ordered its own declaration bracket.

- **The call-site carrier is `Vec<Len>`, not `Vec<u32>`.** `Len` already has a
  `Var` arm; a bare `u32` would permanently foreclose ever writing a length
  *variable* as an explicit argument. `Vec<Len>` costs nothing extra to parse
  (a length position produces `Len::Concrete`) and keeps that one-way door open
  without committing to variable-forwarding syntax now.

- **`sum[i64 'N]` (forwarding a variable) is unreachable, and stays so.** The
  parser's own instantiation-list scan rejects a type variable at any position
  before the checker sees it, and a concrete body has no length variable in
  scope to name. A `Len::Var` reaching `check_poly_call` is therefore an
  internal-consistency error, guarded with `unreachable!()` (not
  `debug_assert!`, which no-ops in release) rather than a user diagnostic.

- **The two allow-guards needed real logic, not a forward.** The non-poly
  dispatch guard and the poly-body guard each read the type-arg list to decide
  whether to *reject* it; both were widened to also gate on a non-empty length
  list, so an explicit length in the wrong context is rejected rather than
  silently dropped. The `inline`/combinator exclusion is unchanged: a
  combinator never takes an explicit instantiation list, which is why the
  integration fixture is deliberately **non-`inline`** and reads `len` back
  rather than indexing.

- **Both `Len::Var` conflict arms route through a seeded-length set.**
  Previously both `Len::Var` arms in `unify_poly_input` always raised the
  generic `poly_len_conflict_error`, while the `Var` arm routed through
  `seeded`. Once `check_poly_call` can seed `subst.len`, that asymmetry became
  live: `sum[i64 4]` over a length-8 operand must report the caller-context
  explicit-instantiation message, not the generic one. Both arms now use the
  same `seeded_len` routing; the inferred (unseeded) path is unchanged.

- **Generic-length array *indexing* in a non-inline body stays out of scope.**
  Probed as a real cross-layer change (checker + lowering + QBE backend), not a
  guard removal; deferred to P7.S6c. This slice's fixture reads the length back
  (`len`) instead.

## Implementation

- **Phase 1 -- AST + parser** (`612df54`): widened `TermKind::Call` to
  `Call(String, Vec<Type>, Vec<Len>)` (`src/ast.rs`), rippled through every
  consumer (each forwards `Vec::new()` or clones), and extended
  `parse_explicit_type_args` with the integer-token mode switch, the widened
  empty-instantiation guard (`sum[4]` parses, `sum[]` still errors), and a
  call-site-shaped range message (`src/parser.rs`).
- **Phase 2 -- guard widening + checker seeding** (`02f4cff`): widened the two
  allow-guards (`src/check/terms.rs`, `src/check/poly.rs`), threaded the length
  list into `check_poly_call`, added `length_instantiation_arity_error` and the
  `subst.len`/`seeded_len` seeding, and renamed/re-scoped the stale
  `instantiation_arity_error` test and note.
- **Phase 3 -- conflict routing** (`8f03328`): added `seeded_len: &[u32]` to
  `unify_poly_input` (`src/check/poly.rs`, which also gains
  `explicit_len_instantiation_conflict_error`), routed both `Len::Var` arms
  through it, and threaded the new parameter through `src/check/combinators.rs`'s
  two call sites (each passing `&[]`, unchanged behaviour).
- **Phase 4 -- integration golden** (`6c4dcbb`): `tests/phase7_slice6b.rs`
  covering accept (`sum[i64 4]` over a length-4 array), length-conflict
  (explicit-instantiation message, not the generic one), and length-arity
  rejection, over the non-`inline` fixture
  `: sum['T 'N: Len] ( array['T 'N] -- usize ) len swap drop ;`.

## Exit criteria (met)

A caller can write `sum[i64 4]` to explicitly bind both `'T = i64` and `'N = 4`,
and a conflicting operand -- type or length -- produces the routed
`explicit_instantiation_conflict_error` / `explicit_len_instantiation_conflict_error`,
not the generic `poly_var_conflict_error`/`poly_len_conflict_error`.
`cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
