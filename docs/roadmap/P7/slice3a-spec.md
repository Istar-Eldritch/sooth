# Phase 7 Slice 3a: generic instantiation over a poly word's own type variable (spec)

## Goal

Let a polymorphic word name a generic type applied to *its own* type variables
(`Result['T 'E]`, `Box['T]`, `Option['T]`) in its signature, and let its body
*construct* such a value, by adding a deferred `PolyType::Generic` application
to the type language and keeping the generic instantiator (`GenericTypes`)
alive and mutable through check and lowering so a monomorph can be minted on
demand at the point a substitution grounds it.

Trait bounds are P7.S3b's concern and appear nowhere here.

## User story

`lib/result.sth`'s `Result 'T 'E` is declared generically but can only be
*named* by monomorphic words today. The program below is the shape this slice
buys: one poly word consuming a generic over its own variables, one poly word
producing one, both instantiated at two **asymmetric** concrete pairs
(`[i64 str]` and its swap `[str i64]`), each printing a value that is only
correct if the two monomorphs are tracked independently and positionally.

```sooth
type: Result 'T 'E | Ok 'T | Err 'E ;

: reorder ( 'T Result['T 'E] -- Result['T 'E] 'T ) swap ;
: wrap    ( 'T -- Result['T i64] ) Ok ;

: show-is ( Result[i64 str] -- ) | Ok |v| v . | Err |e| e . ;
: show-si ( Result[str i64] -- ) | Ok |v| v . | Err |e| e . ;

: main ( -- )
  7 wrap                     -- Result[i64 i64] … see note below
  "tag" swap reorder swap drop drop
  1 "boom" Err reorder . show-is
  "one" 2 Err reorder . show-si ;
```

Before this slice, the *first* line of `reorder` never gets that far: the
signature itself is rejected at parse time.

```text
-- error: unknown type `'T` at line 3, col 23
```

(Verified against `HEAD` on 2026-08-17 with the two-line program
`type: Result 'T 'E | Ok 'T | Err 'E ;` plus `reorder`.) Once the signature
parses, the residual failure moves to the body of `wrap`, where the brief's
probe recorded `` error: unknown word `Ok` ``: the constructor's target type
mentions `'T`, so there is no already-materialized `Ok[…]` env candidate for
it. Note that the *concrete* case already works today and must keep working:
`: wrap ( 'T -- Result[i64 i64] ) drop 1 Ok ;` called from `main` builds and
runs at `HEAD` (probed), because `Result[i64 i64]` is materialized at parse
time by its own appearance in the signature.

**Flagged as under-specified by the brief:** the exact `main` above is
illustrative. The brief's probe program is the authority on shape (`reorder`
over `Result['T 'E]` at `[i64 str]` and `[str i64]`, `nm`-verified to mint
`sooth_mono_reorder__m0__t0_i64_t1_str` and `..._t0_str_t1_i64`); the golden
test (T1) must be written to that shape and to whatever spelling actually
type-checks, not to this sketch.

## Recon

Restated from `docs/roadmap/P7/slice3a-brief.md` (its "Recon" and "Resolved
recon" sections), with the file:line anchors re-verified against `HEAD`.

- **The gap is parse-time monomorphization.** `resolve_type_or_apply`
  (`src/parser.rs:3129-3172`) is the only path a generic name takes. It calls
  `parse_type_arguments` (`src/parser.rs:3204`), which resolves each argument
  through `parse_type_expr` — a path that knows registered concrete names only
  — and then immediately calls `GenericTypes::instantiate_struct` /
  `instantiate_enum` (`src/ast.rs:560`, `src/ast.rs:603`) to produce a
  concrete `Type`. A bare `'T` is not a registered name, hence `unknown type
  'T`. Nothing represents "a generic applied to an argument that is still
  abstract".
- **Poly signature slots have their own parse path.** `parse_poly_slot`
  (`src/parser.rs:1898-1961`) already intercepts arrays, `~[` quotations,
  bare `'T`, and Slice 13's `&`-led shapes *before* falling through to
  `parse_type_expr` at `src/parser.rs:1960`. A generic application needs the
  same treatment: its own arm ahead of that fallthrough, producing a `RawTy`
  case folded by `raw_to_poly_type` (`src/parser.rs:2184`) exactly as
  `RawTy::Array` folds — concrete-in-full collapses to `Concrete`, anything
  variable-bearing stays symbolic.
- **A new `PolyType` variant is required and is not cheap** (brief OQ1). The
  enum lives at `src/ast.rs:1054-1083`. Exhaustive matches over it, verified
  by grepping for the `PolyType::Ref` arm, number **14 across 6 files**; the
  brief said "roughly 13", which is right to the arm. Each needs a deliberate
  decision, not a stub:
  - *Copy-ness / linearity decisions:* `poly_is_copy`
    (`src/check/poly.rs:14`, arm at `:34`), `poly_copy_gate`
    (`:1029`, arm at `:1065`), `is_reference_slot` (`:113`, arm at `:115`),
    `receiver_is_aggregate_projection` (`:1911`, arm at `:1916`).
  - *Unification and substitution:* `unify_poly_input`
    (`src/check/poly.rs:1542`, arm at `:1651`), `apply_subst` (`:1717`, arm
    at `:1770`), and lowering's `subst_polytype` (`src/ir/driver.rs:617`, arm
    at `:654`).
  - *Audits:* `contains_poly_reference` (`src/check/audits.rs:352`),
    `audit_poly_input_quotation` (`:371`), `reject_poly_quotation_anywhere`
    (`:411`).
  - *Export privacy:* `collect_poly_concrete`
    (`src/check/declarations.rs:346`).
  - *Diagnostics:* `poly_op_on_variable_error`
    (`src/check/poly.rs:1838`, arm at `:1852`) and `poly_type_str`
    (`:2201`, arm at `:2235`).
  - *REPL:* `remap_poly_type` (`src/repl.rs:228`, arm at `:260`).
- **The real cost centre is registry lifetime** (brief OQ2). Arrays and refs
  mint monomorphs downstream and on demand: `apply_subst` / `subst_polytype`
  are handed `&mut Vec<ArrayDecl>` / `&mut Vec<RefDecl>` and call
  `intern_array_type` / `intern_ref_type` at the point of use. Named generics
  have no such downstream registry: `GenericTypes` is consumed and dropped at
  `src/driver.rs:308-309` (`structs.extend(generics.inst_structs); enums.extend(generics.inst_enums);`),
  its paired construction site being `GenericTypes::with_bases(...)` at
  `src/driver.rs:242`. After that point nothing can mint a
  `Result[i64 str]` that parse time did not already materialize. The array/ref
  arms are *not* a lookup precedent for a generic: at check time they **mint**
  (`intern_array_type` `src/check/poly.rs:1740`, `intern_ref_type` `:1772`)
  into a `Module`-persisted `&mut Vec`, and only *lowering* is lookup-only. A
  named generic has no check-side mint at all until R2, so there is nothing for
  a pre-R2 lookup to hit.
- **`GenericTypes` ids are base-relative.** `struct_base`/`enum_base`
  (`src/ast.rs:394-395`, set by `with_bases`, `:519`) mean the id of
  `inst_structs[i]` is `struct_base + i`, which is only true because the
  `extend` at `src/driver.rs:308` appends the instances at exactly that
  offset. Any downstream mint must preserve that: push into the live
  `structs`/`enums` registries in the same step, or the identity silently
  rots.
- **The two gaps are entangled** (brief OQ5). A poly word can only *produce*
  a generic monomorph absent elsewhere in the program if it can construct
  one, and construction in a poly body over a variable-bearing target is the
  pre-existing `unknown word Ok` gap. So on-demand minting cannot be tested
  without construction, and construction cannot be lowered without on-demand
  minting. The brief decided **option B**: both in this slice.
- **No placebo** (brief OQ3). The probe's asymmetric two-variable run
  specialized positionally and correctly, `nm`-verified as two distinct
  symbols.

## Design decisions

- **A new `PolyType::Generic` variant, not a reuse of an existing one.**
  Nothing in the enum can carry `(header identity, argument list)` without
  erasing one of them: `Concrete` demands a real `StructId`/`EnumId` (which is
  exactly what does not exist yet), and `Array`/`Ref` are shape-specific. The
  precedent is `PolyType::Ref` (`src/ast.rs:1072-1082`), which deliberately
  carries no `RefId` for the same reason: the id is minted only when the
  referent grounds.
- **Shape:** `PolyType::Generic { is_enum: bool, idx: u32, module: u32, args: Vec<PolyType> }`.
  `idx` indexes `GenericTypes::structs` or `GenericTypes::enums` per
  `is_enum`; `module` is the *instantiating* module, the third component of
  the dedup key (`struct_keys`/`enum_keys`, `src/ast.rs:392-393`), and must be
  captured at the naming site because that is where module identity is known.
  `args` is recursive, so depth > 1 is *representable*, and is nonetheless
  rejected in v1 (below).
- **Grounding routes through the same parse-time dedup table, not through an
  independent downstream intern.** This is the brief's OQ2 caveat made an
  explicit decision: the probe preserved monomorph identity only because it
  routed grounding through `GenericTypes`' own keys. An independent
  downstream interner would mint a second `Result[i64 str]` with a different
  `StructId` for the same type, and the checker's `Type` equality would then
  quietly answer "different". One instantiator, one key table, one id space.
- **Keep `GenericTypes` alive and mutable through check and lowering rather
  than pre-materializing every reachable instantiation at parse time.**
  Pre-materialization would need a whole-program pass over every poly word
  crossed with every call site's substitution before checking has produced
  those substitutions: the information does not exist yet at parse time. The
  arrays/refs pattern already in the codebase is the working precedent.
- **Depth 1 only in v1** (brief OQ4). `Box[Box['T]]` is representable in the
  variant but was never grounded by the probe, and the on-demand mint would
  have to fire for the inner and outer monomorph in the right order. No
  consumer forces it. It is a **located rejection**, not a silent
  mis-compile.
- **A generic over variables is conservatively linear (never `Copy`).** The
  probe took this and a real spec should state it deliberately: `Copy`-ness of
  `Result['T 'E]` depends on the args' bounds, and a per-argument `Copy`
  derivation is a new rule with its own drop-obligation consequences.
  Rejecting `dup` on a variable-bearing generic slot is the conservative
  answer, consistent with the linear spine (forgetting is an error), and it
  can be relaxed later without invalidating any program this slice accepts.
- **Construction is admitted as a narrow arm, not as a general lifting of the
  poly-body restrictions.** Quotation literals (`src/check/poly.rs:466`) and
  array constructors (`:487`) stay rejected in poly bodies; only a generic
  variant constructor gains an arm, and only where its target type is
  determined (R3).

## Requirements

### R1 — `PolyType::Generic`, plus a deliberate arm at all 14 match sites

Add the variant at `src/ast.rs:1054-1083` with the shape above, and the parse
route: a `RawTy::Generic` case beside `RawTy::Array` (`src/parser.rs:861-870`),
produced by a new arm in `parse_poly_slot` ahead of the `parse_type_expr`
fallthrough at `src/parser.rs:1960`, folded in `raw_to_poly_type`
(`src/parser.rs:2184`). The fold mirrors the array fold exactly: if every
argument is `PolyType::Concrete`, call `instantiate_struct`/`instantiate_enum`
and yield `PolyType::Concrete`, so **the concrete path is byte-for-byte
unchanged**; otherwise keep `PolyType::Generic`. The new arm reuses the
existing header lookup and privacy gate (`bare_generic_owner`,
`generic_is_declared`, `type_is_exported`, `src/parser.rs:3130-3143`), so a
qualified `r::Result['T 'E]` and a private-header rejection behave as they do
concretely. Arity mismatch keeps `generic_arity_error`.

Each site gets the arm named here, and none gets a `_ =>` catch-all:

| Site | Arm |
| --- | --- |
| `poly_is_copy` (`check/poly.rs:14`) | `false` (conservative linearity, D5) |
| `poly_copy_gate` (`:1029`) | the located "cannot copy" family, rendering the type via `poly_type_str` |
| `is_reference_slot` (`:113`) | `false` |
| `receiver_is_aggregate_projection` (`:1911`) | `true` for a struct header, `true` for an enum header (a variant projection receiver), matching what the concrete `Type::Struct`/`Type::Enum` answer |
| `unify_poly_input` (`:1542`) | positional recursion: same `is_enum`/`idx`/`module` and equal arity, then unify argument-wise; a concrete `Type::Struct`/`Type::Enum` on the stack matches when it is the instantiation of that header (looked up through the dedup key), else a rendered mismatch |
| `apply_subst` (`:1717`) | substitute each argument, then mint-or-find through the live instantiator (R2) and return `Concrete` |
| `subst_polytype` (`ir/driver.rs:617`) | the same, through the same instantiator, so the lowering side never invents a second id |
| `contains_poly_reference` (`check/audits.rs:352`) | recurse into `args`: a generic carrying `&'T` must not escape the Copy-containment audit |
| `audit_poly_input_quotation` (`:371`) | recurse into `args` |
| `reject_poly_quotation_anywhere` (`:411`) | recurse into `args`, so a quotation smuggled in as a generic argument is still rejected |
| `collect_poly_concrete` (`check/declarations.rs:346`) | recurse into `args` only (contributing any concrete `Type`s found inside); it collects into `Vec<Type>` and a variable-bearing generic has no concrete `Type` of its own to contribute (see the export-privacy note below the table) |
| `poly_op_on_variable_error` (`:1838`) | `"a generic type"`, rendered with the application |
| `poly_type_str` (`:2201`) | `Name['A 'B]` in the signature's own variable spellings |
| `remap_poly_type` (`repl.rs:228`) | remap each argument; the header `idx`/`module` pass through unchanged |

Depth > 1 (an argument that is itself `PolyType::Generic`) is rejected at the
parse fold with one located error naming the outer and inner headers.

**Export-privacy gap (known, must not be silently dropped).** Because
`collect_poly_concrete` can only carry concrete `Type`s, it cannot carry an
ungrounded generic *header* named in an exported poly word's signature, and the
parse-time gate `type_is_exported` (`src/parser.rs:3138`) only fires for
*qualified* names. So an exported poly word can name a bare, module-private
generic header with no privacy check anywhere. Closing this needs a dedicated
generic-header check in `check_exported_signatures` (a separate
`Vec<(usize, u32)>` header-privacy channel, not the `Vec<Type>` one); fully
designing that is out of scope for this slice, but the gap is recorded here as a
required implementation follow-up, not an accident.

### R2 — `GenericTypes` lives through check and lowering

`src/driver.rs:308-309` stops consuming `generics` (its paired construction
site is `GenericTypes::with_bases(...)` at `src/driver.rs:242`). Instead:

- `GenericTypes` is threaded as `&mut` into check and into lowering, beside
  the `&mut Vec<ArrayDecl>` / `&mut Vec<RefDecl>` registries those paths
  already carry. Concretely this exposes a
  `(is_enum, idx, module, args: Vec<Type>) -> {Struct,Enum}Id` resolver
  reachable from `apply_subst`, `unify_poly_input`, and `subst_polytype` — the
  `struct_keys`/`enum_keys` dedup table (`src/ast.rs:392-393`) kept alive
  rather than dropped.
- A single mint entry point (`instantiate_struct`/`instantiate_enum`) is used
  from *all* callers, parse-time and downstream, so the dedup keys stay the
  one identity source (D3).
- **The id invariant, stated precisely:** at every mint (parse-time or
  downstream) the returned `StructId`/`EnumId` index equals the position the
  `StructDecl`/`EnumDecl` occupies in the *final merged* `structs`/`enums`
  registry, and `instantiate_struct`/`instantiate_enum` is the **sole** writer
  of `structs`/`enums` beyond `struct_base`/`enum_base`. The naive
  implementation — keep the one-shot `structs.extend(generics.inst_structs)`
  at `src/driver.rs:308` *and also* mint downstream — is wrong: `extend`
  drains `inst_structs` to empty, so a later `instantiate_struct` computes
  `struct_base + inst_structs.len() = struct_base + 0` and **collides** with
  the first parse-time instance already sitting at that index, giving two
  distinct `Type::Struct` values one id with different field layouts (a
  layout-level miscompile). Once downstream minting exists there is no separate
  `extend`: the live registry grows only through the mint.
- **Implementation note (flag, not solved here):** the mint that pushes into
  the live `structs`/`enums` needs `&mut`, but `instantiate_struct` currently
  reads `regs.structs` immutably while naming the instantiation
  (`src/parser.rs:3145`); the same `Vec` cannot be borrowed both ways, so the
  instantiation name must be computed before the push. Two other consume-drop
  sites must also be checked at implementation time: `src/parser.rs:544-545`
  (the single-file `parser::parse` path drops its own `GenericTypes`, so D3's
  "one id space" is not globally true until it is addressed too) and the REPL
  (reuses `assemble_module` so inherits the driver fix, but re-bases ids across
  import epochs at `src/repl.rs:204`/`:1828`; confirm it does not separately
  offset `generic_structs`, or `remap_poly_type`'s pass-through of
  `idx`/`module` is unsound there).
- A unit test pins the invariant: mint a generic monomorph **downstream, after**
  at least one parse-time instance of the *same header* already exists, and
  assert the two ids differ and each resolves to its own field layout. A
  single-mint-in-isolation test passes under both the correct and the colliding
  implementation, so the guard must be an *interleaved* mint.
- Nothing else about parse-time instantiation changes; a program with no
  variable-bearing generic mints exactly the same set of monomorphs as before.

### R3 — generic construction in a poly body

`poly_call_term` (`src/check/poly.rs:499-791`) gains one arm before its
`unknown_word_error` fallthrough at `:791`, in the same family as `len`
(`:661`) and the comparison words (`:687`): a call naming a **variant of a
generic enum header** (or a generic struct's constructor) is legal in a
polymorphic body.

- Target resolution: the header comes from the variant's base name. The
  arguments come from unifying the operand slots against the header's declared
  payload `PolyType`s. Arguments that the operands do not determine are taken
  from the enclosing word's declared output slot at that stack position when
  that slot is a `PolyType::Generic` over the same header.
- If any argument remains undetermined, this is a **located error** naming the
  constructor and the undetermined variable, not a latent failure at
  monomorphization.
- Operand/payload type mismatch is reported at the constructor call, during
  body check, through the existing `poly_rendered_type_mismatch_error` family
  — never deferred into synthesis.
- The pushed slot is `PolyType::Generic` for the resolved header and
  arguments; when every argument is concrete it folds to `Concrete`, which is
  exactly today's behaviour for the already-working concrete case (probed at
  `HEAD`: `: wrap ( 'T -- Result[i64 i64] ) drop 1 Ok ;` builds and runs).
- Lowering the constructor at an instantiation goes through R2's mint, so the
  monomorph exists even when no other site in the program materialized it.

Why pulling an undetermined argument from the declared output slot is sound: the
undetermined argument is *phantom* for the constructed variant (constructing
`Ok` leaves `E` with no runtime representation — `substitute_generic_field`,
`src/ast.rs:505-511`, substitutes only fields that exist), so adopting any
concrete `E` from the output slot cannot create a runtime/static mismatch. The
real backstop for the *determined* arguments is that `unify_poly_input`'s new
`Generic` arm unifies the produced value against the enclosing word's declared
output at word exit: a wrong inferred position surfaces there as a located type
mismatch, not a silent miscompile. This is what makes inferring from the output
slot safe rather than a type confusion — and why T-nontail (below) must build a
body where the constructed value is *not* in 1:1 tail position, so the exit-time
unification is exercised rather than assumed.

### R4 — one independent monomorph per instantiation

Two distinct substitutions over the same poly word naming a generic yield two
distinct monomorph symbols carrying positionally-correct types. Proven by
`nm` in a test (the pattern of `tests/symbol_hijack.rs`) over an
**asymmetric** pair (`[i64 str]` and `[str i64]`), per the project's
symmetric-instantiation-placebo precedent, plus runtime output that is only
correct if each monomorph carries its own argument order.

### R5 — soundness rejections

Located errors, each asserted on message text:

1. A generic applied to a type variable at nesting depth > 1 (`Box[Box['T]]`)
   — the out-of-scope rejection (D5), so v1's boundary is enforced, not
   assumed.
2. A constructor call in a poly body whose generic arguments are not fully
   determined by its operands or by the declared output slot (R3).
3. A constructor call whose operand types do not match the header's declared
   payload, caught at body check.
4. `dup`/`over` on a variable-bearing generic slot (D5's conservative
   linearity), naming the type.
5. Arity mismatch on a generic applied to variables, reusing
   `generic_arity_error`.

## Implementation

Two phases. Phase 1 introduces the `PolyType::Generic` variant and the parse
route and makes a variable-bearing generic *nameable* and *renderable*: it adds
the arms that need no id resolution and the parse-fold rejections. It does
**not** ground a generic to a concrete id. There is no check-side mint for a
named generic until R2 (the array/ref arms mint at check into a persisted
registry; a generic has no such registry until Phase 2), so Phase 1 does no
grounding and ships no build+run consumption golden — it would be un-passable in
isolation. Phase 2 keeps `GenericTypes` alive, adds the grounding arms and
on-demand minting, admits construction, and carries the build+run goldens.
The grounding arms and R2 are one unit: none of `apply_subst`,
`unify_poly_input`, `subst_polytype` can resolve a generic without the live
table, so they cannot be split from it.

### Phase 1 — the variant, the parse route, and the non-grounding arms

Files: `src/ast.rs`, `src/parser.rs`, `src/check/poly.rs`,
`src/check/audits.rs`, `src/check/declarations.rs`, `src/ir/driver.rs`,
`src/repl.rs`.

- `src/ast.rs:1054-1083`: add `PolyType::Generic { is_enum, idx, module, args }`.
- `src/parser.rs:861-870`: add `RawTy::Generic`.
- `src/parser.rs:1898-1961`: the new `parse_poly_slot` arm ahead of the
  `parse_type_expr` fallthrough at `:1960`, reusing the header lookup and
  privacy gate from `resolve_type_or_apply` (`:3130-3143`) and *only the arity
  check* of `parse_type_arguments` (`:3204-3229`) — not its argument parser,
  which resolves concrete names only; the generic arm parses its arguments as
  poly slots.
- `src/parser.rs:2184-2199`: the `raw_to_poly_type` fold, mirroring the array
  fold; the depth-2 rejection lands here.
- The **non-grounding** arms of R1's table (no id resolution): `poly_is_copy`,
  `is_reference_slot`, `poly_copy_gate`, `poly_op_on_variable_error`,
  `poly_type_str`, `receiver_is_aggregate_projection`, the three audit walks
  (`contains_poly_reference`, `audit_poly_input_quotation`,
  `reject_poly_quotation_anywhere`), `collect_poly_concrete`, and
  `remap_poly_type`.
- `apply_subst` (`check/poly.rs:1770`), `subst_polytype` (`ir/driver.rs:654`),
  and `unify_poly_input`'s `Generic` arm (`check/poly.rs:1651`) still need an
  arm here so the match stays exhaustive, but in Phase 1 that arm is an
  explicit **not-yet-groundable** case: it cannot resolve a variable-bearing
  generic to a concrete id (the dedup key table is dropped before check runs),
  so it errors deliberately rather than pretending to look up a registry that
  does not exist. Real grounding is Phase 2, together with R2.

Tests: parser unit tests for the accepted signature, the concrete-fold
no-change guard, the depth-2 rejection, the arity error; the `poly_type_str`
render test; `poly_generic_slot_is_not_copy`; the two audit-arm rejection tests
(quotation-in-generic-arg; `&'T`-bearing generic in a Copy position); the
`receiver_is_aggregate_projection` arm test. All green in isolation without any
id resolution.

Requirements covered: R1 (parse + non-grounding arms), R5 items 1, 4, 5.

### Phase 2 — registry lifetime, grounding, and construction

Files: `src/driver.rs`, `src/ast.rs`, `src/check/poly.rs`, `src/ir/driver.rs`
(and the check/lower call-site plumbing the `&mut` thread touches:
`src/check.rs:1462`/`:1466`, `src/check/combinators.rs:663`,
`src/check/poly.rs:168`/`:173`/`:1500`, whose `apply_subst`/`unify_poly_input`
signatures gain the generic resolver).

- `src/driver.rs:308-309`: stop moving `generics.inst_structs`/`inst_enums`
  into `structs`/`enums` and dropping `generics` (accounting for the paired
  `with_bases` at `:242`); keep the instantiator alive and thread it `&mut`
  into check and lowering.
- `src/ast.rs:560-641`: the mint entry points keep the live `structs`/`enums`
  registries in sync so the id-index invariant (R2) holds for a downstream
  mint, `instantiate_*` being the sole writer beyond the base.
- `src/check/poly.rs:1651` (`unify_poly_input`), `:1770` (`apply_subst`), and
  `src/ir/driver.rs:654` (`subst_polytype`): the real grounding arms —
  mint-or-find through the live instantiator, on both the consumption and the
  output side.
- `src/check/poly.rs:499-791`: R3's constructor arm ahead of the
  `unknown_word_error` fallthrough at `:791`.

Tests: the R2 interleaved id-invariant unit test;
`unify_poly_generic_binds_arguments_positionally`; T1 (build+run consumption at
two asymmetric instantiations); T2 (`nm`, distinct symbols); T3 (a monomorph
materialized *only* by the poly word's own construction); T-nontail
(construction off tail position, exercising the exit-time unification backstop);
R5 items 2 and 3.

Requirements covered: R2, R3, R4 (consumption *and* production), R5 items 2, 3.

### Phases (machine-readable)

**Note:** no existing P7 spec in this repo carries a machine-readable
`phases` block (`slice1-spec.md` and `slice2-spec.md` both describe phases in
prose only), so the schema below is this document's own, following the field
names requested rather than a repo precedent.

```json
[
  {
    "name": "Phase 1 - PolyType::Generic, the parse route, and the non-grounding arms",
    "requirements": ["R1-parse", "R1-nongrounding-arms", "R5.1", "R5.4", "R5.5"],
    "files": [
      "src/ast.rs",
      "src/parser.rs",
      "src/check/poly.rs",
      "src/check/audits.rs",
      "src/check/declarations.rs",
      "src/ir/driver.rs",
      "src/repl.rs"
    ],
    "changes": [
      "src/ast.rs:L1054-L1083 - add PolyType::Generic { is_enum: bool, idx: u32, module: u32, args: Vec<PolyType> }",
      "src/parser.rs:L861-L870 - add RawTy::Generic beside RawTy::Array",
      "src/parser.rs:L1898-L1961 - parse_poly_slot arm for a generic application, ahead of the parse_type_expr fallthrough at L1960, reusing the header lookup/privacy gate of resolve_type_or_apply (L3130-L3143) and only the arity check of parse_type_arguments (L3204-L3229), not its concrete-only argument parser; arguments parsed as poly slots",
      "src/parser.rs:L2184-L2199 - raw_to_poly_type fold: all-concrete args instantiate to PolyType::Concrete, otherwise stay PolyType::Generic; reject nesting depth > 1 with a located error",
      "src/check/poly.rs:L34 poly_is_copy - false (conservative linearity)",
      "src/check/poly.rs:L115 is_reference_slot - false",
      "src/check/poly.rs:L1065 poly_copy_gate - located cannot-copy diagnostic",
      "src/check/poly.rs:L1651 unify_poly_input - Generic arm present for exhaustiveness but not-yet-groundable (errors; real grounding is Phase 2 with R2)",
      "src/check/poly.rs:L1770 apply_subst - Generic arm present for exhaustiveness but not-yet-groundable (errors; real grounding is Phase 2 with R2)",
      "src/check/poly.rs:L1852 poly_op_on_variable_error - render as a generic type",
      "src/check/poly.rs:L1916 receiver_is_aggregate_projection - true, matching the concrete struct/enum answer",
      "src/check/poly.rs:L2235 poly_type_str - render Name['A 'B] in the signature's spellings",
      "src/check/audits.rs:L359 contains_poly_reference - recurse into args",
      "src/check/audits.rs:L397 audit_poly_input_quotation - recurse into args",
      "src/check/audits.rs:L420 reject_poly_quotation_anywhere - recurse into args",
      "src/check/declarations.rs:L353 collect_poly_concrete - recurse into args only (Vec<Type>; a variable-bearing generic contributes no concrete Type of its own)",
      "src/ir/driver.rs:L654 subst_polytype - Generic arm present for exhaustiveness but not-yet-groundable (errors; real grounding is Phase 2 with R2)",
      "src/repl.rs:L260 remap_poly_type - remap args, pass header identity through"
    ],
    "tests": [
      "src/parser.rs: parse_poly_generic_over_own_type_variable_ok",
      "src/parser.rs: parse_poly_generic_all_concrete_args_folds_to_concrete",
      "src/parser.rs: parse_poly_generic_nested_depth_two_is_error",
      "src/parser.rs: parse_poly_generic_arity_mismatch_is_error",
      "src/parser.rs: parse_poly_generic_private_header_is_not_exported_error",
      "src/check/poly.rs: poly_generic_slot_is_not_copy",
      "src/check/poly.rs: poly_type_str_renders_a_generic_application",
      "src/check/poly.rs: poly_generic_receiver_is_aggregate_projection",
      "src/check/audits.rs: quotation_smuggled_as_generic_arg_is_rejected",
      "src/check/audits.rs: ref_bearing_generic_in_copy_position_is_rejected"
    ]
  },
  {
    "name": "Phase 2 - registry lifetime, grounding, and generic construction in poly bodies",
    "requirements": ["R2", "R3", "R4", "R5.2", "R5.3"],
    "files": [
      "src/driver.rs",
      "src/ast.rs",
      "src/check.rs",
      "src/check/poly.rs",
      "src/check/combinators.rs",
      "src/ir/driver.rs"
    ],
    "changes": [
      "src/driver.rs:L308-L309 - stop consuming GenericTypes into structs/enums (and account for the paired with_bases at L242); keep it alive and thread it &mut into check and lowering beside the array/ref registries",
      "src/ast.rs:L560-L641 - instantiate_struct/instantiate_enum are the sole writer of structs/enums beyond struct_base/enum_base; the returned id index == the decl's position in the final merged registry (no separate one-shot extend once downstream minting exists)",
      "src/check.rs:L1462,L1466 + src/check/combinators.rs:L663 + src/check/poly.rs:L168,L173,L1500 - thread the (is_enum,idx,module,args)->Id resolver through the apply_subst/unify_poly_input call sites",
      "src/check/poly.rs:L1651 unify_poly_input - real grounding: positional recursion over args; a concrete instantiation of the same header matches via mint-or-find",
      "src/check/poly.rs:L1770 apply_subst - mint-or-find through the live instantiator instead of not-yet-groundable",
      "src/ir/driver.rs:L654 subst_polytype - mint-or-find through the same instantiator, never a second id space",
      "src/check/poly.rs:L499-L791 - poly_call_term arm for a generic variant/struct constructor, ahead of the unknown_word_error fallthrough at L791: resolve the header from the base name, determine arguments from the operands and, where undetermined, from the enclosing word's declared output slot; located error when still undetermined; operand/payload mismatch reported at the call site"
    ],
    "tests": [
      "src/ast.rs: interleaved_downstream_mint_id_differs_from_parsetime_instance",
      "src/check/poly.rs: unify_poly_generic_binds_arguments_positionally",
      "src/check/poly.rs: poly_body_constructor_resolves_arguments_from_the_declared_output",
      "src/check/poly.rs: poly_body_constructor_undetermined_argument_is_error",
      "src/check/poly.rs: poly_body_constructor_operand_mismatch_is_error",
      "tests/phase7_slice3a.rs: poly_word_consuming_result_over_its_own_vars_runs_at_two_asymmetric_instantiations",
      "tests/phase7_slice3a.rs: two_asymmetric_instantiations_mint_distinct_symbols_nm",
      "tests/phase7_slice3a.rs: poly_word_constructs_a_monomorph_no_other_site_materializes",
      "tests/phase7_slice3a.rs: poly_body_constructor_off_tail_position_unifies_at_exit"
    ]
  }
]
```

## Testing

New golden file `tests/phase7_slice3a.rs`:

- **T1 `poly_word_consuming_result_over_its_own_vars_runs_at_two_asymmetric_instantiations`**
  (Phase 2 — build+run, needs grounding) — the brief's probe program:
  `: reorder ( 'T Result['T 'E] -- Result['T 'E] 'T )` instantiated at
  `[i64 str]` and `[str i64]`, printing values that are only correct if the two
  monomorphs are independent and positional. Asymmetric on purpose:
  `Result[i64 i64]` cannot tell `Ok 'T | Err 'E` from its swap.
- **T2 `two_asymmetric_instantiations_mint_distinct_symbols_nm`** — `nm` over
  the built object, asserting both
  `sooth_mono_reorder__m0__t0_i64_t1_str` and `..._t0_str_t1_i64` exist
  (spellings to be confirmed against the mangler at implementation time).
- **T3 `poly_word_constructs_a_monomorph_no_other_site_materializes`** — the
  load-bearing test for R2/R3 together: a poly word constructs
  `Result['T i64]` at an instantiation no other signature in the program
  names, so it can only come from a downstream mint. This is the test that
  does not exist without option B.
- **T-nontail `poly_body_constructor_off_tail_position_unifies_at_exit`** — a
  poly body that constructs a generic value and then moves a *different* output
  into tail position (so the constructed value is not in 1:1 tail position),
  proving the exit-time `unify_poly_input` backstop actually fires rather than
  being assumed (R3's soundness argument).
- **T4-T7, the R5 rejections** — depth-2 nesting; an undetermined constructor
  argument; a constructor operand/payload mismatch; `dup` on a
  variable-bearing generic slot. Each asserts exact message text, never
  `is_err()`.

Unit tests as listed per phase in the `phases` block, beside their stage
(`src/parser.rs`, `src/check/poly.rs`, `src/check/audits.rs`, `src/ast.rs`).

**Mutation-test the guards.** Per the project's standing rule, each of the
following must be proven to fail when the arm it guards is deleted (a suite that
stays green after the deletion means the test is a placebo — this repo has
shipped five): T4-T7; the R2 interleaved id-invariant test
(`interleaved_downstream_mint_id_differs_from_parsetime_instance`); the two
audit-arm rejection tests (`quotation_smuggled_as_generic_arg_is_rejected`
guarding `reject_poly_quotation_anywhere`, and
`ref_bearing_generic_in_copy_position_is_rejected` guarding
`contains_poly_reference`); and
`unify_poly_generic_binds_arguments_positionally` — the anti-placebo arm for
asymmetric instantiation, whose deletion mutation is "collapse positional order
/ ignore the argument index", which T1/T2 must then catch.

Regression, must stay green untouched:

- `tests/phase5_slice2.rs` — every concrete generic application
  (`Result[i64 i64]`, `Result[i64 str]`, `Option[i64]`, the qualified
  `r::Result[...]` and privacy cases). This is the fold-to-`Concrete`
  guarantee of R1.
- `tests/phase5_generic_enum_elimination.rs` — clause elimination over
  concrete instantiations, and the `Result[i64 i64]` / `Result[bool bool]`
  non-collision test that pins the dedup keys R2 now keeps alive longer.
- `tests/qbe_baseline.rs` (`corpus_qbe_stays_byte_identical_to_baseline`) — no
  program without a variable-bearing generic may change codegen.
- `examples/poly_borrow_setat.sth`'s coverage and the Slice 13 poly-reference
  suite — `parse_poly_slot` is being edited directly above the `&`-led arms.
- `tests/phase7_slice1.rs`, `tests/phase7_slice2.rs` — the neighbouring P7
  slices.

## Out of scope

- Trait bounds (P7.S3b entirely).
- Nesting depth beyond 1 (brief OQ4): representable, rejected, unblocked
  later if a real consumer forces it.
- Any change to how a *concrete* generic argument resolves.
- Relaxing the conservative "a variable-bearing generic is never `Copy`" rule
  (D5) into a per-argument derivation.
- Quotation literals and array constructors in poly bodies: still rejected
  (`src/check/poly.rs:466`, `:487`).
