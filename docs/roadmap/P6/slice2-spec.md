# Spec: Phase 6 Slice 2 — variant types and accessors

**Status:** Implemented
**Created:** 2026-08-16
**Timestamp:** 2608161200

Pairs with [slice2-brief.md](./slice2-brief.md) (discovery). Phase 6 replaces clause-style
enum dispatch with an explicit eliminator; Slice 3 builds the eliminator, this slice built
the type it binds (`Type::Variant`) and the per-variant accessors, so Slice 3 adds nothing
on the accessor side.

## What shipped

A variant is now a standalone `Type` leaf with a stable `Enum.Variant` display name, a
shared field-projection helper used by both the clause path and the accessor path, and
generated destructure/getter sigs plus two accessor check paths. No IR, backend, or
`EnumLayout`/`VariantLayout` change: per-variant field offsets already existed.

Nothing mints a `Type::Variant` value until Slice 3's eliminator, so the exit witness is a
unit suite over hand-built checker state, not an `.sth` golden.

## Requirements (as delivered)

- **R1 — the leaf and its single name source.** `Type::Variant(EnumId, usize, &'static str)`
  carries a leaked `Enum.Variant` name. This reverses the brief's Decision 1 (no leaked
  name): `intern_ref_type` builds a reference's permanent display name from
  `referent.name()` and leaks it, so a nameless variant would leak `&variant` into every
  ref-mode diagnostic.
  `Type` derives `Eq` over all fields including the `&'static str`, so two `Type::Variant`s
  for one `(EnumId, vi)` whose names differ by a byte compare **unequal**, silently failing
  every `ty != want` check and `Sig` match. `VariantDecl.name_static` is the *bare* name
  (`Circle`) and no dotted string existed, so a per-site `format!`+`Box::leak` would have
  been the hazard (a monomorphized generic makes it concrete: `Option[i64].Some` vs
  `Option.Some`). Delivered instead:
  - `pub display_static: &'static str` on `VariantDecl` (`src/ast.rs:288`), built once where
    the owning enum's name is in hand: concrete-enum parse (`src/parser.rs:278`),
    `instantiate_enum` (`src/ast.rs:608`), both halves of the `Bool` built-in
    (`src/ast.rs:670`, `:677`), and the REPL builder (`src/repl.rs:1576`). Three sites cannot
    apply the rule and take a forwarded or placeholder name — equality-safe because
    `variant_type` *reads* the slot rather than reformatting, so content affects only
    diagnostic spelling: `src/repl.rs:1925` (variants built before the enclosing `EnumDecl`
    computes its import-mangled name, so it forwards and carries the pre-import spelling),
    the `#[cfg(test)]` `fn variant` builder (`src/ast.rs:2045`), and a test closure shared
    across three enums (`src/check/builtins.rs:648`).
  - `pub fn variant_type(enums, id, vi) -> Type` (`src/ast.rs:297`) is the **sole**
    constructor; R6, R11 and the R10 tests all build through it. `pub` is load-bearing: with
    no non-test caller at Phase 1's close, a private helper is `dead_code` under
    `clippy -- -D warnings`.
  - Spelling: `Shape.Circle`; for a monomorphized generic the enum name carries the suffix
    and the variant stays bare (`Option[i64].Some`, via `generic_surface_name`).
- **R2.** `Type::name()`/`Display` return the leaked name directly
  (`Type::Variant(_, _, name) => name`), so registry-free diagnostic sites render
  `Shape.Circle` in value mode and `&Shape.Circle`/`&!Shape.Circle` in ref mode. No
  placeholder and no ctx-aware renderer.
- **R3.** Exactly three catch-all-free `Type` matches were extended (found by grepping
  `Type::InlineQuotation`, which every such match must name to compile): `Type::name()`
  (R2), `ir_type_of` (`unreachable!`, mirroring its `InlineQuotation` arm — no variant
  reaches the backend this slice), and `type_node` (`=> None`: a variant is never a *field*,
  so it closes no by-value containment cycle). `is_aggregate` is a `matches!` over
  `Struct`/`Enum`/`Array`/`OwnedCell`, so a variant correctly reads non-aggregate with no
  edit; the `_`-catch-all matches (`is_copy`, `contains_reference`, `find_zero_unsafe_element`,
  `ref_parts`) fall through unchanged. Variant-*correct* behaviour there is Slice 3 scope.
- **R4/R5.** `variant_field_projection(variant, ref_mutable, refs) -> Vec<Type>`
  (`src/check.rs:356`) computes a variant's field types in declared order (first field
  deepest), value-mode plain and ref-mode via `intern_ref_type`. Elevated to the `check`
  module, the lowest common ancestor of its callers (`word_entry.rs`, `word_families.rs`,
  `declarations.rs`). `check_clause_body` (`src/check/word_entry.rs:441`) calls it in place
  of its inline loop; everything after that loop in the clause path (positional binding,
  `check_terms`, `check_outputs`, `leave_block`) is untouched. Pure extraction, guarded by
  a mutation test reproducing the pre-extraction inline loop in both modes.
- **R6/R7/R8.** `variant_generated_sigs` (`src/check/declarations.rs:1305`) emits, per
  variant, `Variant> ( Variant -- T1 … Tn )` and `Variant>field ( Variant -- Tf )`, keyed on
  the mangled name with the `generic_surface_name` surface key, mirroring
  `struct_generated_sigs` minus the setter. The input slot is built through `variant_type`.
  Registered globally alongside struct/enum sigs (`src/check.rs:519`) and in the REPL
  (`src/repl.rs:1222`). A zero-field variant gets a no-op `Variant>` only. **No
  `Variant<field` setter**: a `Type::Variant` value has no legal destination outside its arm.
  Registering with no legal caller is sound — no surface syntax mints a `Type::Variant`
  operand yet, so a wrong operand takes the ordinary `type_mismatch_error` path and an
  unregistered name the ordinary unknown-word path.
- **R9 — three distinct mechanisms.** Structs have **no** whole-destructure check function;
  the "check-function sibling" in the brief was a phantom.
  1. **Scalar getter and both whole destructures — `Sig` dispatch only.** No check function
     fires: `check_struct_get_word` bails `Ok(None)` on a non-aggregate field, and
     `check_variant_get_word` does the same.
  2. **Aggregate value-mode getter — `check_variant_get_word`**
     (`src/check/word_families.rs:847`), wired at `src/check/terms.rs:490`. Pushes the
     field's *interior address* aliasing the operand region via `peek_region` rather than
     copying it out, matching `check_struct_get_word`. Aggregate variant fields were kept in
     scope (OQ3) because variant field types come from `parse_type_expr` like struct fields,
     so an aggregate payload is constructible today and deferring would ship a getter that
     goes silently wrong the moment Slice 3 can build one.
  3. **Reference mode `&Variant>field`/`&!Variant>field`** — a new arm in
     `check_reference_word`'s `_` branch (`src/check/word_families.rs:~148`), shaped like the
     `&Struct>field` arm.
- **R11 — resolve from the operand's `EnumId`, never a global scan.** The struct arm's
  `ctx.structs().iter().position(...)` scan is sound only because *type* names are globally
  deduped by `check_duplicate_type_names`, which never checks variant names. Variant names
  are **not** unique across enums (verified: two enums each with a `Circle` variant build
  clean), so a scan would pick the first match and mis-reject the second enum's variant.
  Both mechanisms 2 and 3 therefore look the variant up in the enum the operand already
  names, via the shared resolver `variant_accessor_field`
  (`src/check/word_families.rs:820`), which matches the spelled name against
  `generic_surface_name(&v.name)` — a source term can only ever spell the bare surface name,
  since `[` is a lexer delimiter (D7).
  The ref arm builds `want = intern_ref_type(refs, variant_type(...), mutable)` and rejects
  on **full-type equality** `stack[n-1].ty != want`, the `&Struct>field` idiom, **not** the
  `recv_mut != mutable`/`ref_parts` idiom of the `>` and `^` arms; that one comparison
  catches a wrong variant and a wrong mutability alike. It pushes `&field`/`&!field` with
  `prov.project(...)` provenance, and sits after the struct-name lookup and ahead of the
  bare-local prefix-borrow fallback (a name containing `>` is never a local borrow). No
  `&Variant>` whole-destructure arm, matching the struct precedent.
- **R12 — the fall-through ladder** (both accessor mechanisms, in order): operand not a
  `Variant` (resp. not a ref-to-`Variant`) ⇒ `Ok(None)`; spelled name absent from the
  operand's *own* enum ⇒ `Ok(None)` (nothing to build a `want` from, so the call degrades to
  the ordinary unknown-word error in value mode and the prefix-borrow error in ref mode);
  name resolves but `vi` differs ⇒ `type_mismatch_error` against the *spelled* variant;
  resolves and the field is scalar ⇒ `Ok(None)`, deferring to mechanism 1; resolves,
  matches, aggregate ⇒ handled. Deliberately **no** targeted "enum `Shape` has no variant
  `Rect`" diagnostic: a name reaching these arms is not yet known to be an accessor, and
  claiming it would capture unrelated unknown words.
- **R10 — placebo-proofed exit witness.** Unit suite over hand-built state
  (`src/check/word_families.rs:1606+`), each case labelled with the mechanism it guards and
  asserting a discriminating shape. A scalar-field test calling `check_variant_get_word`
  would return `Ok(None)` and pass vacuously, so that shape is **forbidden** — scalar cases
  go through env dispatch, and only the R12 fall-through case asserts on `Ok(None)`. Cases:
  Sig-dispatch scalar getter and whole destructure (exact residual stack types, driven the
  way `infer_src` harnesses do); aggregate getter (pushed type **and** operand-region alias);
  `&Variant>field`/`&!Variant>field` (pushed type **and** provenance, read as
  `prov.deriv(pushed.deriv.unwrap()).projected == true` plus `owned_root`/`place` matching
  the operand — `Provenance::project` mints a fresh `DerivId` and is not idempotent, so
  `pushed.deriv == prov.project(...)` can never hold); mutability mismatch on the
  `want`-equality path; zero-field variant mints no per-field getter; wrong-operand
  `type_mismatch_error` asserting the `Enum.Variant` spelling, which must use an **aggregate**
  field since that rendering only fires inside `check_variant_get_word`; and one R12
  absent-name case. A separate test pins the global-scan ban by giving two enums a
  same-named variant.
- The `enum_generated_sigs` doc comment asserting "a variant has no destructure/getter/setter
  (D2)" was updated: this slice deliberately reverses D2.

## Success criteria (met)

- [x] `Type::Variant` with a leaked name sourced once via `variant_type`; the three
      catch-all-free matches extended; all `_`-catch-all / `matches!` predicates left as-is.
- [x] `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- [x] Clause goldens/dogfood unchanged after the R4 extraction, plus the mutation check.
- [x] `variant_generated_sigs` emits getter + destructure, no setter; zero-field variant
      emits no per-field getter.
- [x] R10 suite passes, mechanism-labelled, no vacuous `Ok(None)` pass.
- [x] No `src/ir/`, `src/backend/`, or `EnumLayout`/`VariantLayout` change.

## Deferred (with reasons)

- The eliminator word, arm-position effect elision (`( Circle )`), exhaustiveness and
  duplicate-arm checking: **Slice 3**.
- Migrating clause-dispatch sites; deleting `WordBody::Clauses`/`parse_clauses`: **Slice 4**.
- Variant-correct behaviour in `is_copy`/`contains_reference`/`is_aggregate`/drop/return/
  borrow guards: **Slice 3** (no variant value flows through them until the eliminator).
- **A variant sharing a struct's name.** `type: S x i64 ; type: E | S y i64 ;` builds clean:
  `check_duplicate_type_names` checks *type* names, and neither registry rejects a variant
  colliding with a struct. Both constructors are keyed `S` with `( i64 -- )`, so pre-slice
  the native env's operand match already picks the first candidate (variant constructor
  unreachable) and the REPL's single-candidate `env.insert` clobbers outright. R6 extends
  the same collision to the destructure key `S>`. Not fixed here: the root rule is the
  missing variant-vs-type name-collision check, which belongs with Slice 3, where a variant
  name first has to resolve unambiguously.

## As-built map

| Location | Symbol |
|---|---|
| `src/ast.rs:288` | `VariantDecl.display_static` — the sole name source |
| `src/ast.rs:297` | `variant_type` — sole `Type::Variant` constructor |
| `src/ast.rs` | `Type::Variant` leaf; `name()` arm (`Display` delegates) |
| `src/ir/types.rs` | `ir_type_of` — `unreachable!` arm |
| `src/check/declarations.rs:1081` | `type_node` — `Type::Variant(..) => None` |
| `src/check.rs:356` | `variant_field_projection` — shared clause/accessor projection |
| `src/check.rs:519`, `src/repl.rs:1222` | env registration of the variant sigs |
| `src/check/declarations.rs:1305` | `variant_generated_sigs` |
| `src/check/word_families.rs:820` | `variant_accessor_field` — shared `EnumId`-scoped resolver |
| `src/check/word_families.rs:847` | `check_variant_get_word` (aggregate value mode) |
| `src/check/word_families.rs` `_` branch | `&Variant>field`/`&!Variant>field` arm |
| `src/check/terms.rs:490` | dispatch into `check_variant_get_word` |
| `src/check/word_entry.rs:441` | `check_clause_body` rewired to the shared helper |

## References

- [slice2-brief.md](./slice2-brief.md) — discovery, recon, settled decisions.
- [ROADMAP.md](../ROADMAP.md) — Phase 6 plan.
