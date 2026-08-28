# Phase 7 Slice 6a: length parameters in `type:` headers, and the `Kind` type (brief)

**Sequence.** After S6 (surface syntax unification, done), which introduced the
`type: Box['T]` bracket binding site this slice extends. Independent of S7b (the
testing vocabulary, currently mid-implementation): S6a touches only `src/ast.rs`,
`src/parser.rs`, `src/check/poly.rs`; S7b touches `src/driver.rs`, `src/main.rs`,
`lib/hosted/*`. No shared files.

**Motivation.** A user-defined generic type can bind a type variable (`type:
Box['T]`) but not a length variable: `type: Buffer['T 'N] data array['T 'N] ;`
does not parse — `parse_header_bracket` (`src/parser.rs:5297-5323`) accepts only
`'`-prefixed words with no way to mark one as a length rather than a type, and even
if it did, `substitute_generic_field`'s `Len::Var` arm (`src/ast.rs:824-827`, the
"N3" comment) is a genuine `unreachable!()`, not a stub — a generic `type:` header
today provably binds no length variable. A `Buffer`-shaped struct wrapping a
generic-length array is common stdlib material (any fixed-capacity container) and
is currently unwritable.

## What already works (verified by reading, not by trusting the roadmap prose)

A **word** signature already fully supports length variables of a distinct kind
from type variables:

- `VarKind { Ty, Len }` already exists (`src/parser.rs:1310-1315`), used by
  `PolyBuilder::intern_ty_var`/`intern_len_var` (`src/parser.rs:1432-1462`) to
  reject a name used as both kinds (X1, `var_kind_conflict_error`).
- `PolySig` already carries `ty_var_names`/`len_var_names` separately, and
  `PolyType::Array(Box<PolyType>, Len)` (`src/ast.rs:1982`) already carries
  `Len::Var` for a word's declared array-length parameter.
- Check-time machinery already resolves `Len::Var`: `unify_poly_input` binds it
  from a concrete array's count at call sites, `match_impl_target_rec` matches it,
  `apply_subst` resolves it from `Subst.len: Vec<(u32, u32)>`, and a poly body's
  `len` on a generic-length array already folds to `usize`
  (all in `src/check/poly.rs`, line numbers below are close but drift with edits —
  re-grep at phase 1 rather than trust these: `unify_poly_input` ~6560,
  `match_impl_target_rec` ~5810, `apply_subst` ~6840, `len`-folds-to-`usize` ~1260).

What's missing is entirely on the **declaration** and **use-site-instantiation**
paths for `type:`/`trait:` headers, which never touch `PolyBuilder` — they go
through a separate, simpler parser (`parse_generic_header`, `src/parser.rs:4872`)
that has no kind distinction at all:

- `GenericStructDecl`/`GenericEnumDecl` (`src/ast.rs:542-562`) carry only
  `ty_var_names: Vec<String>`, no length-variable field.
- `parse_header_bracket` (`src/parser.rs:5297-5323`) accepts only `'`-prefixed
  words, no `: Len` annotation, no length-variable case at all.
- `substitute_generic_field`'s `PolyType::Array` arm (`src/ast.rs:824-830`) only
  ever sees `Len::Concrete` — the `Len::Var` case is the N3 `unreachable!()`.
- `GenericTypes::struct_keys`/`enum_keys` (`src/ast.rs:602-603`) dedup on
  `Vec<Type>` alone — `Buffer[u8 256]` and `Buffer[u8 512]` would currently
  collide onto the same monomorph if this compiled at all.
- `type_instantiation_name` (`src/ast.rs:777-780`) renders only `Vec<Type>` args.
- The **use-site** application parser, `resolve_type_or_apply` ->
  `parse_type_arguments` (`src/parser.rs:5142`, `5270-5296`), resolves
  `Buffer[u8 256]` by parsing `arity` (from `ty_var_names.len()`) type
  expressions and immediately calling `instantiate_struct`/`instantiate_enum` —
  eager monomorphization at parse time, unlike a word call's deferred check-time
  binding. This is a second, distinct site from S6b's `check_poly_call` seeding
  (which is about a *word* call like `sum[i64 4]`); S6a needs its own length-
  literal parsing here, since a `type:` application resolves to a concrete `Type`
  immediately and has no `Subst`/check-time step at all. **Not mentioned in the
  P7-language-prereqs.md prose for this slice — found by reading, add it to the
  spec.**

## What changes

1. **`Kind` enum.** `VarKind { Ty, Len }` (word-signature-local, parser-private)
   generalizes to a `Kind` enum with `Star` (replacing the implicit "no kind" of
   `ty_var_names`) and `Len`, used by both the word-signature path and the new
   `type:`/`trait:` header path. Whether this literally renames `VarKind` in place
   or introduces a new shared `Kind` that `VarKind` becomes an alias/wrapper for
   is an implementation-phase call, not a design one — the roadmap's framing
   ("`VarKind { Ty, Len }` becomes a `Kind` enum") suggests the former.

2. **Header bracket syntax.** `parse_header_bracket` gains a `: Len` annotation
   path: `'N: Len` interns a length variable, a bare `'T` interns a type variable
   (kind `Star`, the default — unannotated stays the common case). `type: Buffer['T
   'N: Len]` declares one of each.

3. **AST fields.** `GenericStructDecl`/`GenericEnumDecl` gain `len_var_names:
   Vec<String>`, parallel to `ty_var_names`.

4. **Field substitution.** `substitute_generic_field`'s N3 arm becomes real: given
   a length-argument list (parallel to the existing `args: &[Type]`), it looks up
   the concrete length exactly as `PolyType::Var` looks up a concrete type from
   `args`.

5. **Instantiation plumbing.** `instantiate_struct`/`instantiate_enum` accept a
   length-argument list alongside the type-argument list. `struct_keys`/
   `enum_keys` dedup keys widen to include it, so `Buffer[u8 256]` and `Buffer[u8
   512]` mint distinct monomorphs. `type_instantiation_name` renders length args
   in the mangled symbol (`Buffer[u8 256]`).

6. **Use-site parsing.** `parse_type_arguments` (or a sibling for the mixed case)
   splits a header application's bracket contents into `0..ty_arity` type
   expressions and `ty_arity..ty_arity+len_arity` length literals (`u32`, same
   `1..=u32::MAX` range check `parse_array_count` uses, `src/parser.rs:4295`),
   using `ty_var_names.len()`/`len_var_names.len()` to know the split — the arity
   is known statically from the header the moment it's found, same shape as
   S6b's plan for word calls.

## Out of scope (mirrors the roadmap's framing for this slice)

- Generic-length array indexing in a non-inline body (`poly_generic_length_index_error`,
  `src/check/poly.rs:7646`, stays as-is — a checker limitation this slice does not
  touch).
- Non-length const kinds (booleans, strings as phantom parameters) — `Kind` stays
  `{ Star, Len }`; no `Const` generalization (DESIGN.md's "dependent types: never").
- `Arrow` kind (higher-kinded types) — named as a later P7b addition in the
  roadmap prose, not this slice's concern.
- S6b (explicit length arguments at a **word** call site, `sum[i64 4]`) is the
  next slice, sequenced after this one but independent in principle — this brief
  covers only the `type:`/`trait:` header and application side.

## Exit criteria (carried from the roadmap, unchanged by this brief)

`type: Buffer['T 'N: Len] data array['T 'N] ;` parses; `Buffer[u8 256]`
instantiates as a distinct monomorph from `Buffer[u8 512]`; a word declaring
`Buffer['T 'N]` in its signature unifies correctly against a concrete caller
(exercising the check-time machinery in "what already works" above, which needs
no change). `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
The `Kind` enum has `Star` and `Len` variants; the `: Len` annotation syntax is
live at `type:`, `trait:`, and `:` (word) binding sites.

## Open questions

None outstanding. The infrastructure split (check-time machinery already generic
over `Len::Var`; only declaration/instantiation is missing) was confirmed by
reading `src/ast.rs` and `src/parser.rs` directly, not inferred from roadmap
prose. No probe/paper-test subagents were needed — every claim above was checked
against current source line-by-line.
