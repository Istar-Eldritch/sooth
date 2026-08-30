# Phase 7 Slice 6b: explicit length arguments at word call sites (brief)

**Sequence.** After S6a (length parameters in `type:`/`trait:` headers, done — merged
`108630e`). Independent in principle of S7a-d, S8 (both landed on `main` since this
branch last synced): S6b touches `src/ast.rs` (`TermKind::Call`), `src/parser.rs`
(word-call type-argument parsing), `src/check/poly.rs` (`check_poly_call`,
`unify_poly_input`'s `Len::Var` arms). No shared files with the testing/hosted-layer
work.

**Branch hygiene note.** This worktree (`p7-s6b`) was 181 commits behind `main`
(still pre-S6/S6a) when this brief was drafted; it has been rebased onto `main`
(`git rebase main`, clean, `cargo build` green) before any of the discovery below.
All line numbers and claims here are against the rebased tree.

**Motivation.** A user-defined `type:` header can now bind a length variable and be
applied with an explicit length (`Buffer[u8 256]`, S6a). A **word** signature can
also declare a length variable (`capacity['T 'N: Len]`, already parseable per S6a),
but a caller has no syntax to *explicitly* bind it: `sum[i64 4]` does not parse —
`type_args` only. The type-variable case already has this (`sum[i64]`); this slice
is the length-variable parallel, narrowly scoped to call sites.

## What already works (verified by reading, not by trusting the roadmap prose)

- Inferred length binding (no explicit args) already works: `unify_poly_input`
  binds `'N` from a concrete array's count when `array[i64 4]` fills a
  `array['T 'N]` parameter (`src/check/poly.rs:8068-8079`, the `Len::Var` arm under
  `PolyType::Array`).
- `Subst` already carries `len: Vec<(u32, u32)>` (`src/ast.rs:2261`).
- `PolySig` already carries `len_var_names: Vec<String>` alongside `ty_var_names`
  (`src/ast.rs:2235-2244`).
- The **type header** application path (`Buffer[u8 256]`) already parses a mixed
  type/length bracket: `parse_type_arguments` (`src/parser.rs:5584-5629`) returns
  `(Vec<Type>, Vec<Len>)`, parsing `ty_arity` type expressions then `len_arity`
  length literals via `self.parse_array_count(name)`, with arity known statically
  from `self.generics.structs[idx].{ty,len}_var_names.len()`. This is a **different
  parser from a different call site** than the one this slice must change — it
  never touches a word call.

## What's actually missing (found by reading, corrects the roadmap's framing)

The roadmap prose says `parse_type_arguments` "currently parses only types inside
`[...]`" and needs widening. **That function already handles the mix (post-S6a);
it is the wrong function.** Word calls go through a separate, simpler parser that
has no length-argument path at all:

- **`parse_explicit_type_args`** (`src/parser.rs:6315-6349`), called from
  `parse_term` (`src/parser.rs:6418-6424`) for every `Token::Word` term, returns
  bare `Vec<Type>` and has no arity check at parse time at all — it just consumes
  type expressions until `]`. It has no access to the callee's `PolySig`
  (word signatures are registered and resolved after parsing, at check time), so
  it cannot split `ty_arity`/`len_arity` the way the header-application path does
  (which knows arity immediately from `self.generics`). **This means the
  type/length split for a word call cannot happen at parse time the way S6a's did
  for type headers — it must stay untyped at parse time and be validated at check
  time**, same as today's bare `type_args.len() != sig.ty_var_names.len()` check
  in `check_poly_call`.
- **`TermKind::Call(String, Vec<Type>)`** (`src/ast.rs:2948`) has no length-argument
  field. Every non-`check/poly.rs` consumer (`src/ast.rs:3056` rename, `src/parser.rs:656`
  member-call rewrite, `src/resolve.rs:842,1735`, `src/ir/driver.rs:664`) only
  reads the callee name, never the type-argument list, so widening this tuple's
  shape is check/poly.rs-local in effect — confirmed by grep, not assumed.
  **This AST change is not mentioned in the roadmap prose at all** (it only
  discusses `check_poly_call`'s seeding loop) but is required: there is currently
  no field to carry an explicit length literal from parser to checker.
- **`check_poly_call`** (`src/check/poly.rs:5600-5789`) takes `type_args: &[Type]`
  and seeds `subst.ty`/`seeded` from it (lines 5655-5665, quoted below) but has no
  parameter or seeding step for lengths at all.

  ```rust
  let mut seeded: Vec<u32> = Vec::new();
  if !type_args.is_empty() {
      if type_args.len() != sig.ty_var_names.len() {
          return Err(instantiation_arity_error(span, name, &sig, type_args.len()));
      }
      for (v, ty) in type_args.iter().enumerate() {
          subst.ty.push((v as u32, *ty));
          seeded.push(v as u32);
      }
  }
  ```

- **The `Len::Var` conflict-check gap (the real correction to the roadmap).** The
  roadmap claims "the conflict-check logic in `unify_poly_input`'s `Len::Var` arm
  is already identical in shape to the `Var` arm's conflict check." **It is not.**
  Both `Len::Var` arms (`src/check/poly.rs:8068-8079` under `PolyType::Array`, and
  `:8247-8263` under `PolyType::Generic`) always raise the generic
  `poly_len_conflict_error` on a mismatch:

  ```rust
  Len::Var(ln) => {
      if let Some(prev) = subst.len_of(*ln) {
          if prev != count {
              return Err(poly_len_conflict_error(ctx, span, name,
                  &sig.len_var_names[*ln as usize], prev, count));
          }
      } else {
          subst.len.push((*ln, count));
      }
  }
  ```

  The `PolyType::Var` arm (`:7987-7999`), by contrast, routes the message through
  `seeded.contains(v)`:

  ```rust
  PolyType::Var(v) => {
      if let Some(prev) = subst.ty_of(*v) {
          if prev != slot_ty {
              let var = &sig.ty_var_names[*v as usize];
              return Err(match seeded.contains(v) {
                  true => explicit_instantiation_conflict_error(ctx, span, name, var, prev, slot_ty),
                  false => poly_var_conflict_error(ctx, span, name, var, prev, slot_ty),
              });
          }
      } else { /* insertion */ }
  }
  ```

  There is no `seeded_len`-equivalent today. Until this slice adds explicit length
  seeding, this asymmetry is latent (nothing can seed `subst.len` explicitly, so
  the `explicit_instantiation_conflict_error` branch is simply unreachable for
  lengths). Once seeding lands, the gap becomes live: without fixing the routing,
  an explicit `sum[i64 4]` whose operand is actually length 8 would report the
  generic `poly_len_conflict_error` (talking about "conflicting bindings") instead
  of the caller-context-carrying `explicit_instantiation_conflict_error` the
  roadmap's exit criterion explicitly asks for ("the same 'explicit instantiation
  conflict' diagnostic a conflicting type argument already produces").

## What changes

1. **`TermKind::Call`** widens from `Call(String, Vec<Type>)` to carry a length-argument
   list alongside the type-argument list (e.g. `Call(String, Vec<Type>, Vec<Len>)`,
   or a small struct if a third positional field reads poorly — an implementation
   call, not a design one). All non-poly consumers ignore the new field (confirmed:
   they pattern-match `_` on the existing `Vec<Type>` already).
2. **`parse_explicit_type_args`** (or its call site in `parse_term`) extends to
   optionally parse trailing length literals after type expressions, using
   `parse_array_count`'s existing `1..=u32::MAX` check (`src/parser.rs:4762-4788`).
   Because the callee's arity is not known at parse time (no `PolySig` available
   here, per "what's missing" above), the parser cannot split type-vs-length by
   position the way the header path does. **Open implementation question below.**
3. **`check_poly_call`** gains a `len_args: &[u32]` parameter (or extracts it from
   the widened `TermKind::Call`/caller), extends the arity check
   (`instantiation_arity_error` or a length-specific sibling) and the seeding loop
   to push into `subst.len`, and extends `seeded` (or a parallel `seeded_len`) to
   record which length variable ids were explicitly bound.
4. **Both `Len::Var` arms in `unify_poly_input`** route their conflict error through
   the seeded-length set, mirroring the `PolyType::Var` arm exactly: seeded →
   `explicit_instantiation_conflict_error`, unseeded → `poly_len_conflict_error`
   (unchanged for the inferred case).
5. **`explicit_instantiation_conflict_error`** (`src/check/poly.rs:9373-9384`)
   already takes `(ctx, span, callee, var, instantiated: Type, operand: Type)` —
   it is `Type`-typed, not generic over `Type`/length. It needs either a sibling
   for lengths (comparing two `u32`s) or a small generalization. Check both call
   sites' message wording stays sensible for a length ("was instantiated at `'N`
   = `4` but its operand is `8`" reads fine using the same template with `u32`
   formatted the same as `Type`'s `Display`, so a thin sibling function is likely
   simpler than generalizing the existing one over a trait).

## Implementation question this brief surfaces (not present in the roadmap prose)

**How does a word call's parser distinguish a trailing length literal from a
trailing type argument when it has no `PolySig` to consult?** Two candidate
answers, to be settled at implementation time (paper-test both against
`sum[i64 4]` and a pathological case like a type named the same as a small
integer — not currently possible, since types are capitalized/reserved words and
length literals are decimal integers):

- **(a) Lexical disambiguation**: a length literal is a bare decimal integer
    token; a type expression is never a bare integer (types are word-shaped:
    `i64`, `Box[...]`, etc.). So the parser can greedily parse type expressions
    until it sees an integer token, then switch to parsing integers as lengths,
    with no arity split needed at parse time at all — this mirrors how
    `parse_type_arguments`'s over-application arm already falls back to
    permissive parsing. This looks like the simpler answer and matches the
    "context disambiguates and no annotation is needed" principle the roadmap
    states for `array['T 'N]` at use sites.
- **(b) Defer the split entirely to check time**: parse a flat mixed sequence
    of "type expr or integer literal" tokens into two parallel `Vec<Type>`/
    `Vec<u32>` in encounter order (order matters: `sum[i64 4]` must record `i64`
    then `4`, but a length can only follow a type positionally per the
    declaration order `ty_arity..ty_arity+len_arity`), then have
    `check_poly_call` validate against `sig.ty_var_names.len()` +
    `sig.len_var_names.len()` — this is really the same as (a), just described
    from the checker's side. These two aren't actually competing designs; (a) is
    the parsing tactic, (b) is the arity-validation site. Flagging this as a
    question mostly to make sure the phase-1 implementer doesn't invent a
    position-based split that silently assumes `PolySig` is available at parse
    time (it is not, confirmed above).

## Out of scope

- Type headers' length arguments (`Buffer[u8 256]`) — already done, S6a.
- Generic-length array indexing in a non-inline body
  (`poly_generic_length_index_error`, `src/check/poly.rs:7646`-adjacent, unchanged
  checker limitation).
- Any lib word migrating to explicit `['T 'N: Len]` bracket syntax — none exist
  yet (`lib/core/combinators.sth`'s `each`/`map`/`fold`/`filter` all use array
  parameters with an *implicit* `'N`, not a bracket-declared one); this slice
  needs at least one golden word with an explicit length variable to exercise the
  call-site syntax against, most naturally a small new test fixture (following
  `tests/phase7_slice6a.rs:77-89`'s `Buffer['T 'N: Len]` / `capacity['T 'N: Len]`
  pattern) rather than a lib retrofit.

## Exit criteria (carried from the roadmap, corrected per the seeded-routing gap above)

A caller can write `sum[i64 4]` to explicitly bind both `'T = i64` and `'N = 4`
against a word declared `sum['T 'N: Len] ( array['T 'N] -- 'T )` (or equivalent),
and a conflicting operand — either type or length — produces the routed
`explicit_instantiation_conflict_error`, not the generic `poly_var_conflict_error`/
`poly_len_conflict_error`. `cargo fmt --check && cargo clippy -- -D warnings &&
cargo test` is green.

## Open questions

None blocking spec-write. The one design question above (parser disambiguation
strategy for a length literal vs. a type expression at a word call site) has a
clear leading answer (lexical: integer token switches mode) verified against the
existing over-application fallback in `parse_type_arguments`; it should be stated
as a decision in the spec rather than re-opened, but is called out here since it
is a small design choice the roadmap prose glossed over as "extends to accept a
mix."
