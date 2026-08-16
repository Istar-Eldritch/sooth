# Spec: Phase 6 Slice 2 — variant types and accessors

**Status:** Draft
**Created:** 2026-08-16
**Timestamp:** 2608161200

Pairs with [slice2-brief.md](./slice2-brief.md) (the discovery document; recon measured
against `main` at `42f0764`). This spec resolves the brief's three open questions, adds
numbered traceable requirements, a verified codebase map, and a phased plan sized for
Sooth's craft conventions.

## Problem Statement

- **Business context:** Phase 6 replaces clause-style enum dispatch (`WordBody::Clauses`)
  with an explicit eliminator. Slice 3 builds the eliminator; this slice builds the type
  it binds (`Type::Variant`) and the per-variant field accessors (`Variant>field`,
  `Variant>`) that mirror the struct-generated words already in the tree, so Slice 3 has
  nothing left to add on the accessor side.
- **Current state:** `Type` has no `Variant` case (`src/ast.rs:1186`). Variants get only a
  generated *constructor* `Sig` (`enum_generated_sigs`, `src/check/declarations.rs:1268`),
  whose own doc comment (`src/check/declarations.rs:1263-1264`) records the **opposite** of
  this slice as settled design: "Unlike a struct, a variant has no destructure/getter/setter
  (D2: not a standalone type; elimination is clause-style, Phase 4)." This slice
  **deliberately reverses that D2 decision** (the same way R1 reverses brief Decision 1 on the
  leaked name): it adds exactly the destructure/getter the comment denies. Phase 3 updates the
  now-false comment beside the new `variant_generated_sigs`.
  Clause bodies already project variant fields positionally (`check_clause_body`,
  `src/check/word_entry.rs:411`), and the runtime layout already carries per-variant field
  offsets (`VariantLayout`, `src/ir/layout.rs:228`), so no IR/backend/layout change is
  implied.
- **Key issues:** A variant is not a standalone first-class type; it is legal only as an
  arm's declared input and the value inside that arm. Nothing mints a `Type::Variant`
  value until Slice 3's eliminator, so this slice's own exit witness cannot be an `.sth`
  golden and must be a unit test against a hand-built checker state.

## Resolved Open Questions

- **OQ1 — is the shared projection helper (brief Decision 4) a pure extraction, or does it
  change clause-body behaviour? → Pure extraction.** The only logic the accessor path and
  `check_clause_body` share is the field-type computation loop at
  `src/check/word_entry.rs:440-447`: for each variant field in declared order, produce the
  field's exposed type, ref-mode via `intern_ref_type`, value-mode as the plain field type.
  Everything after that loop in the clause path (positional local binding, `check_terms`,
  `check_outputs`, `leave_block`) is clause-specific and untouched. Verified against the
  three dimensions the brief flagged: field order (declared, first field deepest), ref-mode
  projection (`intern_ref_type(refs, ty, mutable)`), and exhaustive field list — all
  identical between the two call sites. The extraction is therefore behaviour-preserving:
  every clause-style golden and dogfood stays green with identical generated code (proven by
  regression + a mutation check in Phase 2). No clause-body code path changes semantics.

- **OQ2 — register the accessor sigs globally now, or gate them behind the not-yet-built
  eliminator? → Register now.** A new `variant_generated_sigs` (beside
  `enum_generated_sigs`) emits the getter/destructure `Sig`s into the env alongside struct-
  generated words, so Slice 3 adds only the eliminator itself. This is sound with no
  eliminator present (brief recon 6): no surface syntax can produce a `Type::Variant`-typed
  operand yet, so no legal call site can satisfy an accessor's declared input; a call with
  any other operand takes the ordinary `type_mismatch_error` path (identical to
  `check_struct_get_word`, `src/check/word_families.rs:751`), and an unregistered name falls
  through to the ordinary unknown-word diagnostic. The only site that supplies a
  `Type::Variant` operand this slice is its own hand-built unit tests.

- **OQ3 — are aggregate-typed (struct/array) variant fields in scope for the accessor? → In
  scope.** A variant field may be aggregate today: variant field types come from
  `parse_type_expr`, the same route struct fields take, so a struct- or array-typed payload
  is constructible in an `enum` decl now. Brief Decision 2 mandates exact struct parity, and
  `check_struct_get_word` already handles an aggregate field by pushing its *interior
  address* (aliasing the operand region) rather than copying it out
  (`src/check/word_families.rs:719-765`). Deferring would ship a `Variant>field` getter that
  is silently wrong for an aggregate field the moment Slice 3 can construct one. The accessor
  therefore mirrors `check_struct_get_word`'s aggregate-field aliasing device; the exit
  witness includes one aggregate-field unit test. `examples/vm.sth`'s `Op` variants are all
  scalar, so no dogfood forces this, but the getter is correct for both.

## Requirements

- **R1.** A new leaf case `Type::Variant(EnumId, usize, &'static str)` exists on the `Type`
  enum (`src/ast.rs:1186`), carrying the owning enum id, the variant's index into
  `EnumDecl.variants`, **and** a leaked `&'static str` display name spelled `Enum.Variant`
  (e.g. `Shape.Circle`), exactly as `Struct`/`Enum` each carry a leaked `name_static`. The
  leaked name is **required, not optional**: `intern_ref_type` (`src/ast.rs:797`) builds a
  reference's permanent display name as `format!("&{}{}", if mutable {"!"} else {""},
  referent.name())` and `Box::leak`s it (`src/ast.rs:804`), so a `&Variant`/`&!Variant`
  interned by the ref-mode accessor (R11) reuses `Type::Variant::name()` verbatim. Without a
  real name the ref would leak the useless spelling `&variant`/`&!variant` into every
  ref-mode mismatch diagnostic; with the `Enum.Variant` name it interns as
  `&Shape.Circle`/`&!Shape.Circle`. This reverses the brief's Decision 1 (no leaked name).

  **Single source of truth (mandatory).** `Type` derives `PartialEq/Eq` over *all* fields,
  including the `&'static str` (`src/ast.rs:1185-1186`), so two `Type::Variant`s for the same
  `(EnumId, vi)` whose leaked names differ by a byte compare **unequal**, silently
  failing every `ty != want` check and every `Sig`-overload match against that variant.
  `Struct`/`Enum` dodge this only because the name is interned **once** on the decl and reused
  (`Type::Struct(id, structs[idx].name_static)`, `src/ast.rs:139`; `struct_generated_sigs`
  reuses `decl.name_static`, `src/check/declarations.rs:1218`). But `VariantDecl.name_static`
  (`src/ast.rs:282`) is the **bare** variant name (`Circle`), and no dotted `Shape.Circle`
  string exists anywhere — so "build the name where the `Type::Variant` is constructed" would
  mean a fresh `format!` + `Box::leak` per construction site, and two sites that format
  differently compare unequal (a monomorphized generic enum makes this concrete:
  `Option[i64].Some` at one site vs `Option.Some` at another). Therefore this slice mandates
  one source, not a per-site format:
  - Add a `display_static: &'static str` field to `VariantDecl` (`src/ast.rs:280`), built
    **once** at declaration time where the owning enum's name is in hand and leaked there —
    at the concrete-enum parse (`src/parser.rs:275`), at generic-enum instantiation
    (`instantiate_enum`, `src/ast.rs:576`), at the `Bool` built-in (`src/ast.rs:651`), and at
    the two REPL builders (`src/repl.rs:1572`, `:1915`). Three further sites exist and are
    **not** covered by the uniform rule, because the owning enum's name is structurally out
    of scope at each; a new required field breaks all of them at compile time, so name them
    now: `src/ast.rs:657` (the `True` half of the `Bool` built-in, beside the cited `False`
    at `:651` — same fn, same literal name in scope, so the rule *does* apply here);
    `src/ast.rs:2011` (the `#[cfg(test)]` `fn variant(...)`, whose caller `module_with_enum`
    at `:2000` holds the enum name, not the builder); and `src/check/builtins.rs:642` (a test
    closure reused across the three enums `Plain`/`Item`/`Boxed`, `:650-687`, so it cannot
    embed any single enum name). The two test builders take a trivially leaked placeholder;
    they are diagnostic-only and equality-safe (see below). Likewise `src/repl.rs:1915`
    builds its variants *before* the enclosing `EnumDecl` computes its import-mangled name
    (`:1911-1925`), so it forwards the existing `display_static` and carries the pre-import
    enum spelling; cosmetic, not a correctness matter. Spelled `"{EnumDecl.name}.{bare
    surface variant name}"`: `Shape.Circle` for a concrete enum, and — because the enum name
    carries its instantiation suffix while the variant is its bare surface spelling —
    `Option[i64].Some` for a monomorphized generic enum (**not** `Option[i64].Some[i64]`: take
    the variant name through `generic_surface_name`).
  - Add a single constructor helper `variant_type(enums, id, vi) -> Type` that reads
    `enums[id].variants[vi].display_static` and returns `Type::Variant(id, vi, display_static)`.
    It is the **only** place a `Type::Variant` is ever built — R6's sig-input slot and R11's
    operand both go through it, and the R10 unit tests build their `Type::Variant`s through it
    too — so the leaked name has exactly one origin and every construction of the same
    `(EnumId, vi)` is byte-identical and compares equal.

    **What this mandate is and isn't.** Because `variant_type` *reads* one registry slot
    rather than formatting a fresh string, equality holds by construction no matter what any
    builder stored: `display_static`'s content affects only diagnostic spelling, never type
    equality. The soundness hazard is the *absence* of a sole constructor (per-site
    `format!` + `Box::leak`, which the paragraph above rejects), not a per-site disagreement
    about the string. So the placeholder names at the two test builders are safe, and this
    mandate is a diagnostic-quality decision resting on a soundness-shaped constraint, not a
    soundness fix in its own right.

    Both `VariantDecl.display_static` and `variant_type` are declared **`pub`**. This is not
    style: at Phase 1's close `variant_type`'s only callers are Phase 1's own `#[cfg(test)]`
    tests (its real callers arrive in Phase 3), and a non-`pub` item with no non-test caller
    is `dead_code`, i.e. fatal under this project's `clippy -- -D warnings` phase gate. `pub`
    lib items are exempt, which is how the sibling `enum_generated_sigs`/`struct_generated_sigs`
    already sit. The field and the helper both land in **Phase 1**, beside the leaf.
- **R2.** `Type::name()`/`Display` (`src/ast.rs:1484`/`:1525`) return the leaked
  `Enum.Variant` name directly via the arm `Type::Variant(_, _, name) => name`, mirroring the
  `Struct`/`Enum`/`Array`/`Ref` arms one line above. **No** registry-free placeholder and
  **no** ctx-aware renderer are needed: the name is carried on the leaf (R1), so every
  diagnostic site — including those without the enum registry — renders `Shape.Circle` in
  value mode and `&Shape.Circle`/`&!Shape.Circle` in ref mode. This is the exact spelling the
  user sees in an accessor's `type_mismatch_error`.
- **R3.** Every exhaustive `match` on `Type` that lacks a `_` arm gains a `Type::Variant`
  arm. Verified: exactly **three** such sites (every catch-all-free `Type` match must name
  `Type::InlineQuotation`, the last leaf, to compile; grepping that spelling finds exactly
  these three with no preceding `_`):
  - `Type::name()` (`src/ast.rs:1484`, R2): `Type::Variant(_, _, name) => name`.
  - `ir_type_of` (`src/ir/types.rs:205`): `unreachable!` arm, mirroring the
    `Type::InlineQuotation` arm at `src/ir/types.rs:252` — a `Type::Variant` never reaches
    the backend this slice, since only the not-yet-built eliminator mints one.
  - `type_node` (`src/check/declarations.rs:1081`, the value-containment graph feeding the
    recursive-type check): `Type::Variant(..) => None`, with the same reasoning the file's
    own `Ref`/`Quotation`/`InlineQuotation` arms give — a variant is never a *field*, so it
    closes no by-value containment cycle (the enum it belongs to, and that enum's fields, are
    the graph nodes; a bare variant is not). Its match ends `| Type::InlineQuotation(_) =>
    None,` at `:1105`.

  Not extended this slice: `is_aggregate` (`src/ast.rs:1470`) is a **`matches!` macro** over
  `Struct`/`Enum`/`Array`/`OwnedCell` (not a `_` catch-all), so `Type::Variant` correctly
  reads as non-aggregate with no edit; the `_`-catch-all `Type` matches `is_copy`
  (`src/check/builtins.rs:233`, `_ => true`), `contains_reference`
  (`src/check/builtins.rs:279`, `_ => false`), `find_zero_unsafe_element`
  (`src/check.rs:361`, `_ => None`), and `ref_parts` (`src/check.rs:2193`, `_ => None`)
  fall through unchanged. No `Type::Variant` value flows through any of them this slice;
  giving them variant-correct *behaviour* is Slice 3 scope (borrow/return/drop guards over a
  bound variant value).
- **R4.** A single shared helper computes a variant's projected field types in declared
  order (first field deepest), value-mode as the plain field type and ref-mode via
  `intern_ref_type(refs, ty, mutable)`. It is elevated to the lowest common ancestor of its
  two callers (the clause path in `word_entry.rs` and the accessor path in
  `word_families.rs`), i.e. the `check` module. `check_clause_body` calls it in place of its
  inline loop.
- **R5.** The extraction in R4 is behaviour-preserving: every existing clause-style test and
  dogfood (`examples/vm.sth`, `Bool`, Phase 5's `Result`/`Option` consumers) stays green,
  and clause bodies generate identical code. Proven by the existing suite plus a mutation
  check that the helper's output equals the pre-extraction inline result.
- **R6.** A new `variant_generated_sigs` (beside `enum_generated_sigs`,
  `src/check/declarations.rs:1268`) emits, per variant, a whole-variant destructure
  `Variant> ( Variant -- T1 … Tn )` and a per-field getter `Variant>field ( Variant -- Tf )`,
  keyed on the mangled name with the `generic_surface_name` surface key, exactly mirroring
  `struct_generated_sigs` (`src/check/declarations.rs:1215`) minus the setter (R8). The input
  slot is built through the R1 `variant_type(enums, id, vi)` helper (never a fresh
  `format!`+`Box::leak`), so its leaked name shares R1's single source of truth. Field
  order matches declared order (first field deepest). These sigs join the env globally
  (R-OQ2). Both words type purely by this registered `Sig` dispatched through the ordinary
  env-call path (the same path that types a scalar struct getter and the whole `S>`
  destructure) — **no check function is involved** for a scalar-field getter or for either
  whole destructure.
- **R7.** A zero-field variant gets no per-field getter and a no-op `Variant>` destructure
  only (nothing to project), mirroring the zero-field struct precedent.
- **R8.** No `Variant<field` setter is generated: a `Type::Variant` value has no legal
  destination outside its arm for a "same variant, one field replaced" result.
- **R9.** The three struct-accessor mechanisms are distinct, and each variant accessor maps
  to exactly one of them (structs have **no** whole-destructure check function — the earlier
  "whole-destructure check-function sibling" was a phantom):
  1. **Scalar value-mode getter `Variant>field` and the whole `Variant>` destructure — Sig
     dispatch only (R6).** `check_struct_get_word` (`src/check/word_families.rs:719`) returns
     `Ok(None)` for any non-aggregate field (its `if !field_ty.is_aggregate()` bail at
     `:738`) and for a whole destructure (empty field name matches no field), so a scalar
     getter and both whole destructures are typed *entirely* by the registered `Sig`. No
     `check_variant_get_word` clause fires for them.
  2. **Aggregate value-mode getter `Variant>field` — `check_variant_get_word`.** A new
     function beside `check_struct_get_word`, shaped like it but **resolving the variant from
     the operand's own `EnumId`** (the stack top is `Variant(EnumId, vi, _)`), **never** a
     global variant-name scan — variant names are not unique across enums (see R11). It splits
     the word on `>`, verifies the spelled variant name matches the operand's variant via
     `generic_surface_name`, and takes the field from it. It fires **only** for an aggregate
     field (an early `if !field_ty.is_aggregate() { return Ok(None); }`, deferring the scalar
     case to mechanism 1), pushes the field's *interior address* aliasing the operand region
     via the `peek_region` device (`src/check/word_families.rs:753`) rather than copying it
     out, and reports a wrong operand (its variant is not the spelled one) with
     `type_mismatch_error`, whose `want` is the spelled variant located **within that same
     enum** (still no cross-enum scan).
  3. **Reference-mode getter `&Variant>field`/`&!Variant>field` — `check_reference_word`
     (R11).** Handled by a new arm in `check_reference_word`'s `_` branch, **shaped like** the
     existing `&Struct>field` arm but resolving from the operand's `EnumId` (R11), **not** the
     struct arm's global registry scan, and **not** by `check_variant_get_word`. This is a
     real, in-use family for structs (`examples/vm.sth:105`: `&vm &Vm>mem addr &> @`).
- **R11.** Reference-mode variant field access is **in scope** (user-approved): a new
  `&Variant>field`/`&!Variant>field` arm in `check_reference_word`'s `_` branch
  (`src/check/word_families.rs:12`, struct arm at `:108-141`), **shaped like** the
  `&Struct>field` arm but **not sharing its resolution**. The struct arm's global scan
  (`ctx.structs().iter().position(|d| d.name == struct_name)`, `:109`) is sound only because
  type names are globally deduped (`check_duplicate_type_names`, `src/check/declarations.rs:666`,
  which checks **type** names, never variant names); **variant names are not unique across
  enums** (verified by compiling `type: A | Circle r i64 | Sq ;` and `type: B | Circle w i64 |
  Tri ;` in one file — builds clean, exit 0), so a global variant-name scan would pick the
  first match arbitrarily and mis-reject the second enum's variant. Instead **resolve from the
  `EnumId` the operand already carries**: the stack top is `&Variant(EnumId, vi, _)`, so look
  the variant up in **that** enum, verify the accessor's spelled variant name matches it via
  `generic_surface_name` (a source term can only ever spell the bare surface name — `[` is a
  lexer delimiter, per the D7 note on `check_struct_peek_word`,
  `src/check/word_families.rs:662-666`; state it here so Slice 3 does not inherit the
  surface-vs-mangled ambiguity), and take the field from it — **no global name scan**. It
  builds `want = intern_ref_type(refs, variant_ty, mutable)` (with `variant_ty` from the R1
  `variant_type` helper; legal because `intern_ref_type` at `src/ast.rs:797` is generic over
  any referent `Type`) and rejects a wrong operand on **full-type equality** `if
  stack[n-1].ty != want` — the exact idiom the `&Struct>field` arm uses (`:118`, `:126`),
  **not** the `recv_mut != mutable` / `ref_parts` idiom (that belongs only to the `>` and `^`
  arms, `:60`, `:97`). A mutability mismatch (a `&Variant` operand to a `&!Variant>field`
  accessor, or vice versa) is caught by this same inequality, since the two interned ref types
  differ. It then pushes `&field`/`&!field` via `intern_ref_type(refs, field_ty, mutable)`
  with `prov.project(...)` provenance. **Resolution ordering:** the variant arm sits after the
  struct-name lookup and ahead of the `_` prefix-borrow-of-a-local fallback (a bare `&x`),
  exactly as the struct-name lookup already does — a name containing `>` is an accessor, never
  a local borrow. The `&Variant`/`&!Variant` operand becomes *constructible* only once Slice
  3's eliminator binds an arm over a **borrowed** scrutinee, so this slice ships it validated
  against hand-built state only (R10). No `&Variant>` whole-destructure arm is added, matching
  the struct precedent (`&>` is the array-index reference word, not a struct destructure).
- **R12 (the absent-name branch, both accessor mechanisms).** Resolving from the operand's
  `EnumId` (R9 mechanism 2, R11) makes the arm a total function, and each fall-through must
  be stated so neither mechanism invents a diagnostic it cannot ground. In order: operand is
  not a `Variant` (resp. not a ref-to-`Variant`) ⇒ `Ok(None)`, fall through to the ordinary
  lookup chain; the spelled variant name is **absent from the operand's own enum** (`&Rect>w`
  against a `&Variant(Shape, Circle)` operand, or a `Rect` belonging to some other enum)
  ⇒ `Ok(None)`, since there is no variant in that enum to build a `want` from — the call then
  degrades to the generic unknown-word error in value mode, and in ref mode to the
  prefix-borrow fallback's error (a `>`-containing name is never a valid bare local); the
  name resolves but the operand's `vi` differs ⇒ `type_mismatch_error` against the `want`
  built from the *spelled* variant; resolves, matches, and the field is scalar ⇒ `Ok(None)`,
  deferring to `Sig` dispatch (mechanism 1); resolves, matches, aggregate ⇒ handled here.
  Deliberately **no** targeted "enum `Shape` has no variant `Rect`" diagnostic this slice: a
  name that reaches these arms is not yet known to be an accessor at all, and claiming it is
  would capture unrelated unknown words.
- **R10.** The exit witness is a unit test suite against a hand-built checker state, not an
  `.sth` golden. **Placebo-proofing (this repo has shipped placebo tests five times):** every
  case names the mechanism it guards and asserts a *discriminating* shape, never mere
  non-failure. A scalar-field test calling `check_variant_get_word` directly would return
  `Ok(None)` and pass vacuously — **that shape is forbidden**; a scalar getter is guarded
  only through env dispatch of its `Sig`.
  - *Sig dispatch (mechanism 1):* scalar-field getter and whole `Variant>` destructure —
    drive them through the ordinary env-call path with the `Sig` registered
    (`variant_generated_sigs`) and a hand-built stack carrying a `Type::Variant` slot (built
    via `variant_type`, R1), and **assert the exact resulting stack types** (e.g.
    `Variant>field` leaves `[Tf]`; `Variant>` leaves `[T1 … Tn]`). Build the env the way
    `infer_line`'s own test harnesses do (`infer_src` in `src/check/word_families.rs:1199` and
    `src/check.rs:2515`, both seeding an env from `*_generated_sigs` and calling `infer_line`,
    `src/check.rs:857`).
  - *`check_variant_get_word` (mechanism 2):* aggregate-field getter (R-OQ3) — call the
    function directly with a hand-built `Ctx`/stack and **assert both** the pushed slot's type
    (`field_ty`) **and** that it aliases the operand region (the `peek_region` alias/provenance
    is set, not a fresh unaliased slot).
  - *`check_reference_word` arm (mechanism 3, R11):* `&Variant>field` and `&!Variant>field`
    — this mechanism **cannot** use `infer_src` (that harness drives `infer_line`, and no
    source line can produce a `&Variant` operand this slice), so it is a **raw direct call to
    the 11-arg `check_reference_word`** (`src/check/word_families.rs:12`) with fully hand-built
    state, unlike mechanism 1, which correctly uses `infer_src`. Build the stack with a
    `&Variant`/`&!Variant` slot and **assert both** the pushed slot's type (`&field`/`&!field`)
    **and** its provenance. The provenance assertion must be literally expressible:
    `Provenance::project` (`src/check/engine.rs:362`) clones the parent `Deriv`, sets
    `projected: true`, and mints a **fresh** `DerivId`, so it is **not** idempotent and
    `pushed.deriv == prov.project(operand.deriv)` can never hold; instead read
    `prov.deriv(pushed.deriv.unwrap())` (`src/check/engine.rs:263`) and assert `.projected ==
    true` plus `.owned_root`/`.place` matching the operand's `deriv` — which discriminates a
    real `project()` from a placebo bare-forward of the parent. Plus a mutability-mismatch case
    (a `&Variant` operand to a `&!Variant>field` accessor, or vice versa) asserting
    `type_mismatch_error` on the **`want`-equality path** (`stack[n-1].ty != want`), the idiom
    the `&Struct>field` arm uses — **not** `recv_mut != mutable` (the `>`/`^` arms' idiom).
  - Also: a zero-field variant (R7, no per-field getter minted) and a value-mode wrong-operand
    `type_mismatch_error` asserting the `Enum.Variant` spelling from R2 — this case **must use
    an aggregate field**, since that rendering only fires inside `check_variant_get_word`,
    which bails `Ok(None)` for a scalar field (a scalar wrong-operand goes through `Sig`
    dispatch, a different error path and wording).

## Success Criteria

- [ ] `Type::Variant(EnumId, usize, &'static str)` exists and compiles with a leaked
      `Enum.Variant` name sourced once from `VariantDecl.display_static` via the sole
      `variant_type` constructor (R1); the **three** catch-all-free matches (R3: `name`,
      `ir_type_of`, `type_node`) are extended, all `_`-catch-all / `matches!` predicates left
      as-is.
- [ ] `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
- [ ] Clause-style goldens/dogfood unchanged after the R4 extraction (R5), with a mutation
      check proving the shared helper reproduces the inline result.
- [ ] `variant_generated_sigs` emits getter + destructure sigs (no setter) matching the
      struct template; zero-field variant emits no per-field getter (R6, R7, R8).
- [ ] The R10 unit suite passes, each case mechanism-labelled and asserting a discriminating
      shape (no `Ok(None)` vacuous pass): Sig-dispatch scalar getter + whole destructure
      (exact stack types), `check_variant_get_word` aggregate getter (type + alias),
      `check_reference_word` `&Variant>field`/`&!Variant>field` (type + provenance +
      mutability mismatch, R11), zero-field, and value-mode wrong-operand mismatch.
- [ ] No `src/ir/`, `src/backend/`, or `EnumLayout`/`VariantLayout` change (brief recon 4).

## Scope & Boundaries

**In scope:**

- The `Type::Variant` leaf (with leaked `Enum.Variant` name) and its **three** forced match
  arms (R1-R3).
- The shared field-projection helper and clause-path rewire (R4-R5).
- Generated getter/destructure sigs (R6-R8) and the value-mode aggregate-getter check
  function `check_variant_get_word` (R9 mechanism 2), scalar getter + whole destructure being
  Sig-dispatch only (R9 mechanism 1).
- The reference-mode `&Variant>field`/`&!Variant>field` arm in `check_reference_word` (R9
  mechanism 3 / R11), shipped validated against hand-built state.
- Unit-test exit witness against hand-built checker state (R10).

**Out of scope:**

- The eliminator word, arm-position effect elision (`( Circle )`), exhaustiveness/duplicate-
  arm checking: Slice 3.
- Migrating any clause-dispatch site to the eliminator; deleting `WordBody::Clauses`/
  `parse_clauses`: Slice 4.
- Any `EnumLayout`/`VariantLayout` or backend-emission change (recon 4 found no lowering gap).
- A `Variant<field` setter (R8).
- Variant-correct behaviour for `is_copy`/`contains_reference`/`is_aggregate`/drop/return/
  borrow guards: Slice 3 (no variant value flows through them until the eliminator exists).

## Codebase Map

Anchors verified against `main` at `42f0764` on 2026-08-16.

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/ast.rs:1186` | `enum Type` | Add the `Variant(EnumId, usize, &'static str)` leaf (R1). |
| `src/ast.rs:280` | `VariantDecl` | Add the `display_static: &'static str` field, built once at each construction site (R1, BLOCKER 1). |
| `src/ast.rs:576` | `instantiate_enum` | A `VariantDecl` construction site; builds `display_static` as `Option[i64].Some` for a monomorphized generic enum (R1). |
| `src/ast.rs:1484` | `Type::name()` | Exhaustive, no catch-all: add `Variant(_, _, name) => name` arm (R2, R3). `Display` delegates to `name()` (`src/ast.rs:1525`). |
| `src/ir/types.rs:205` | `ir_type_of` | Exhaustive, no catch-all: add `unreachable!` arm mirroring `InlineQuotation` at `:252` (R3). |
| `src/check/declarations.rs:1081` | `type_node` | Third catch-all-free `Type` match (value-containment graph): add `Type::Variant(..) => None` (R3). |
| `src/check/word_entry.rs:411` | `check_clause_body` | Owns the inline field-projection loop at `:440-447` (`intern_ref_type` call at `:443`); rewire to call the R4 helper (R4-R5). |
| `src/check/word_entry.rs:288` | `check_clause_word` | Supplies `ref_mutable` (value/`&`/`&!` mode); the mode the helper threads (R4). |
| `src/check/declarations.rs:1215` | `struct_generated_sigs` | Template for R6 (getter/destructure/setter shape, `generic_surface_name` keying); variant drops the setter (R8). |
| `src/check/declarations.rs:1268` | `enum_generated_sigs` | Constructor-only today; add sibling `variant_generated_sigs` (R6). |
| `src/check/word_families.rs:719` | `check_struct_get_word` | Template for R9 mechanism 2 (**aggregate value-mode getter only**): `Ok(None)` scalar bail at `:738`, `peek_region` interior-address aliasing at `:753`, `type_mismatch_error` for wrong operand. A scalar getter / whole `S>` destructure never enter here. |
| `src/check/word_families.rs:12` | `check_reference_word` | R9 mechanism 3 / R11: the `_` branch's `&Struct>field` arm (`:108-141`) shapes the new `&Variant>field`/`&!Variant>field` arm (operand-check + `intern_ref_type` + `prov.project`), but the arm resolves from the operand's `EnumId`, **not** the struct arm's global registry scan (`:109`) — variant names aren't cross-enum unique. |
| `src/check/word_families.rs:650` | `check_struct_peek_word` | Reference of the `peek_region` aliasing device mechanism 2 reuses (`:695`). |
| `src/ir/layout.rs:228` | `VariantLayout` | Confirms per-variant field offsets already exist; **not modified** (recon 4). |
| `src/ast.rs:797` | `intern_ref_type` | Ref-mode field interning; called by `check_clause_body` at `word_entry.rs:443` and by the R11 arm. Leaks `format!("&{}{}", mut?, referent.name())` at `:804` (drives R1's leaked-name decision). |

## Open Questions

- [x] ~~OQ1 — shared helper a pure extraction? → Yes, pure extraction (see Resolved).~~
- [x] ~~OQ2 — register sigs globally now? → Yes (see Resolved).~~
- [x] ~~OQ3 — aggregate variant fields in scope? → Yes (see Resolved).~~

## Implementation Plan

### Phase 1: `Type::Variant` leaf and forced match arms

Add `Type::Variant(EnumId, usize, &'static str)` carrying a leaked `Enum.Variant` name (R1),
and extend the **three** exhaustive matches that lack a catch-all (R3). No placeholder and no
ctx-aware renderer: `name()` returns the leaked name directly, so diagnostics render the full
spelling at every site (R2).

**Scope:**

- Modify: `src/ast.rs:1186` (`enum Type`) — new leaf with leaked name; `src/ast.rs:1484`
  (`Type::name()`) — `Type::Variant(_, _, name) => name` arm (`Display` delegates, `:1525`).
- Add: `display_static: &'static str` on `VariantDecl` (`src/ast.rs:280`), **`pub`** (R1).
  A required field breaks *every* construction site at compile time; all eight are listed so
  none is a surprise. Enum name in hand, uniform `Shape.Circle` / `Option[i64].Some` rule:
  the concrete-enum parse (`src/parser.rs:275`), `instantiate_enum` (`src/ast.rs:576`, the
  literal at `:590`), and both halves of the `Bool` built-in (`src/ast.rs:651` **and `:657`**,
  same fn). Enum name *not* in scope, so a forwarded or placeholder name (equality-safe,
  diagnostic-only, per R1): `src/repl.rs:1572` (name still live, uniform rule applies),
  `src/repl.rs:1915` (built before the import-mangled enum name exists — forward the existing
  `display_static`), the `#[cfg(test)]` `fn variant` (`src/ast.rs:2011`), and the test closure
  shared across three enums (`src/check/builtins.rs:642`).
- Add: `variant_type(enums, id, vi) -> Type`, **`pub`** — the **sole** constructor of a
  `Type::Variant` (reads `display_static`); every later site (R6, R11, R10 tests) builds
  through it (R1). `pub` is load-bearing: with no non-test caller until Phase 3, a non-`pub`
  helper is `dead_code` and fails this phase's own `clippy -- -D warnings` gate.
- Modify: `src/ir/types.rs:205` (`ir_type_of`) — `unreachable!` arm mirroring `:252`.
- Modify: `src/check/declarations.rs:1081` (`type_node`) — `Type::Variant(..) => None`.
- Unit tests beside each: `name()`/`Display` returns `Enum.Variant`, `ir_type_of` arm is
  unreachable, `type_node` returns `None` for a variant (closes no containment cycle);
  `variant_type` returns the `display_static` spelling for a concrete and a monomorphized
  generic enum, and two calls for one `(EnumId, vi)` compare equal (R1).
- Out of bounds: any `_`-catch-all / `matches!` predicate (`is_copy`, `contains_reference`,
  `is_aggregate`, `find_zero_unsafe_element`, `ref_parts`); any `src/ir/layout.rs` change.

### Phase 2: extract the shared field-projection helper

Lift the field-type projection loop out of `check_clause_body` into a helper at the lowest
common ancestor of its two callers, and rewire the clause path to call it. Behaviour-
preserving.

**Scope:**

- Add: `variant_field_projection(variant, ref_mutable, refs) -> Vec<Type>` in the `check`
  module (lowest common ancestor of `word_entry.rs` and `word_families.rs`).
- Modify: `src/check/word_entry.rs:440-447` (`check_clause_body`) — call the helper.
- Tests: helper unit tests (value + ref mode, declared order, zero-field); a mutation check
  that the helper output equals the pre-extraction inline result; existing clause goldens/
  dogfood stay green.
- Out of bounds: any change to clause local-binding, `check_terms`, or `check_outputs`.

### Phase 3: generated sigs and the three accessor mechanisms

Add `variant_generated_sigs` (R6) and map each accessor to its correct mechanism (R9): scalar
getter + whole destructure by Sig dispatch, aggregate getter by a new `check_variant_get_word`,
reference mode by a new `check_reference_word` arm (R11). Deliver the placebo-proofed R10
exit witness. This stays one phase (the three mechanisms are one cohesive accessor family and
do not warrant a 3a/3b split), but its scope is wider than the prior draft by the ref-mode arm.

**Scope:**

- Add: `variant_generated_sigs` beside `enum_generated_sigs`
  (`src/check/declarations.rs:1268`), mirroring `struct_generated_sigs` minus the setter;
  wire into the same env-registration path struct-generated sigs use (mechanism 1).
- Add: `check_variant_get_word` beside `check_struct_get_word`
  (`src/check/word_families.rs:719`) — **aggregate field only** (early `Ok(None)` scalar
  bail), resolving the variant from the operand's `EnumId` (not a global scan, R11) and
  reusing the `peek_region` device for interior-address aliasing (mechanism 2). No
  whole-destructure check function (structs have none).
- Add: a `&Variant>field`/`&!Variant>field` arm in `check_reference_word`'s `_` branch
  (`src/check/word_families.rs:12`), shaped like the `&Struct>field` arm but resolving from
  the operand's `EnumId` (not the struct arm's global scan), with variant-name resolution
  ahead of the bare-local fallback (mechanism 3 / R11).
- Modify: the `enum_generated_sigs` doc comment (`src/check/declarations.rs:1263-1264`) — it
  asserts a variant has "no destructure/getter/setter"; `variant_generated_sigs` landing
  beside it makes that false, so update it (the FIX-NOW-4 reversal of D2).
- Both accessor mechanisms implement R12's fall-through ladder (not-a-variant operand,
  name absent from the operand's own enum, scalar field ⇒ `Ok(None)`; `vi` mismatch ⇒
  `type_mismatch_error`), so neither invents a diagnostic for a name it has not established
  is an accessor.
- Tests (R10, hand-built `Ctx`/env/stack with a `Type::Variant` or `&Variant` slot):
  Sig-dispatch scalar getter + whole destructure (assert exact stack types), aggregate getter
  (assert type + alias), `&Variant>field`/`&!Variant>field` (assert type + provenance +
  mutability mismatch), zero-field variant, value-mode wrong-operand mismatch, plus one
  R12 absent-name case asserting the `Ok(None)` fall-through reaches the ordinary unknown-word
  error rather than a variant-specific one. Except that case, no test may pass on `Ok(None)`.
- Out of bounds: `Variant<field` setter; `&Variant>` whole-destructure arm; any
  eliminator/arm logic.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| The R4 extraction silently changes clause-body codegen. | Mutation check that the helper reproduces the inline result; full clause golden/dogfood suite green (R5). |
| A hidden exhaustive `Type` match without a catch-all is missed and fails to compile. | `cargo build` is authoritative; the `Type::InlineQuotation` grep (every catch-all-free `Type` match must name it) found exactly three (`name`, `ir_type_of`, `type_node`). Any further one gets a reject/`None`/`unreachable!` arm per R3. |
| Registering accessor sigs with no legal caller masks a future unsoundness. | Recon 6: no surface syntax mints a `Type::Variant` operand this slice, so no illegal call is reachable; wrong operands hit the ordinary mismatch path (R-OQ2). |
| Aggregate-field aliasing diverges from the struct device and mis-aliases. | Reuse `peek_region` unchanged; unit-test the aggregate-field getter against a hand-built state (R10). |

## References

- [slice2-brief.md](./slice2-brief.md) — discovery, recon, settled decisions.
- [ROADMAP.md](../ROADMAP.md) — Phase 6 plan.
- Phase 6 Slice 1 spec/brief — precedent for a checker-only slice unit-tested without a golden.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Add Type::Variant(EnumId, usize, &'static str) leaf carrying a leaked Enum.Variant name, and extend the three catch-all-free Type matches: name/Display return the leaked name, ir_type_of unreachable arm, type_node returns None. No placeholder or ctx-aware renderer.", "effort": "S", "difficulty": "standard" },
    { "phase": 2, "focus": "Extract the shared variant field-projection helper to the check module and rewire check_clause_body to it, behaviour-preserving with a mutation check", "effort": "M", "difficulty": "standard" },
    { "phase": 3, "focus": "Add variant_generated_sigs (getter + destructure, no setter) plus check_variant_get_word for aggregate value-mode getters and a &Variant>field/&!Variant>field arm in check_reference_word for reference mode, both resolving the variant from the operand's EnumId; scalar getter and whole destructure are Sig-dispatch only; update the now-false enum_generated_sigs doc comment. Placebo-proofed hand-built-state exit witness asserting stack types, alias, and provenance per mechanism.", "effort": "L", "difficulty": "hard" }
  ]
}
```
