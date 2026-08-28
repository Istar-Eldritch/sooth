# P7.S12 -- Eliminating an ungrounded generic enum inside a poly body (spec)

Status: **done**. Input: [slice12-brief](./slice12-brief.md). Roadmap:
[P7-language-prereqs](../P7-language-prereqs.md), P7.S12.

## Problem

Two defects, one slice, because the second is the first one's exit criterion.

**A. No ungrounded generic scrutinee.** `Option?`/`Result?` cannot be called inside a
polymorphic body while the scrutinee is still `Option['T]`, the normal shape for a library
word over an arbitrary `'T`. `eliminator_registry` (`src/check/declarations.rs:1889`) is
built from `enums: &[EnumDecl]`, so a generic header with no concrete instantiation anywhere
in the program is not a key; and `poly_eliminator_call` (`src/check/poly.rs:2973`) has no
scrutinee arm for `PolyType::Generic` even when the key exists. The rejection is rendered
through `eliminator_arm_outside_call_error` (`src/check/terms.rs:1178`), the message for a
written-adjacency mistake, which is the wrong cause.

**B. A generated enum word in a poly body resolves to the wrong monomorph.** A concrete call
site resolves a generated struct/enum word through the checker-recorded *mangled* symbol in
`builtin_overloads` (`src/check/terms.rs:851`, read at
`src/ir/func_builder/calls.rs:370-388`). A poly body records no such symbol: its walk is
abstract and its spans are shared by every instantiation, so a span-keyed global map cannot
hold the N distinct resolutions the body reaches (`CallInst::poly_calls`' own doc,
`src/ast.rs:2125-2135`). Every generated enum word in a poly body therefore falls to the bare
surface key in `ewords` (`src/ir/layout.rs:651-686`), which is **last write wins across every
monomorph of one header**. `eliminator_registry` has the identical bare-key collision one
stage up, and there -- unlike the concrete path -- the registry's id is used as *identity*,
not as a family gate.

Both halves are the same defect: a poly body's per-monomorph resolution of a generated enum
word does not exist, so it silently borrows the last-declared monomorph's.

### Repros, all built and run against `main` at `afd3d52`

Each is complete and compiles as printed (an ancestor `sooth.pkg` supplies `core`). The
shared preamble is:

```sooth
import: intrinsics * ;
import: core::prelude * ;

type: Pair['A] | Nil | One 'A ;
type: Pt x i64 y i64 ;
```

**B1 -- `Eliminate`, a false rejection, order-dependent.**

```sooth
: mk1 ( i64 -- Pair[i64] ) One ;
: mk2 ( Pt -- Pair[Pt] ) One ;

: use ( Pair[i64] 'T -- i64 'T )
  | keep |
  ~[ ( One ) drop 1 ]
  ~[ ( Nil ) drop 0 ]
  Pair?
  keep ;

: main ( -- )
  1 2 Pt mk2 drop
  7 mk1 5 use drop . ;
```

```
error: error: type mismatch in `use` (line 14)
  `Pair?` expected `Pair[Pt]`, found `Pair[i64]`
  note: declared ( -- )
```

Transcripts here are verbatim, doubled `error: error:` and all: messages bake in the
`"error: "` literal and `main.rs` re-prefixes (the known defect R7.2 matches rather than
fixes). The brief-level precedent is the same (`slice3o-brief.md:49`, `slice3q-brief.md:32`).

Swapping the `mk1`/`mk2` declaration order makes the same program build and print `1`. The
scrutinee is fully concrete; nothing here is generic-scrutinee territory. The registry's
`"Pair?"` key resolved to whichever monomorph was flushed last, and
`poly_eliminator_call` compared the scrutinee against *that* id.

**B2 -- `Destructure`, same shape.** B1's `use` with the `One` arm written `~[ ( One ) One> ]`
and the return type unchanged: same false rejection in the unlucky order, prints `7` in the
lucky one. `Destructure` shares `Eliminate`'s dispatch with no scrutinee read
(`EnumWord::Construct(..) | EnumWord::Destructure(..) => self.lower_enum_word(ew)`,
`src/ir/func_builder/calls.rs:906-909`), so once the checker stops false-rejecting, the
unlucky order reads `Pair[Pt]`'s field layout out of a `Pair[i64]`.

**B3 -- `Construct`, two runtime deaths, one per declaration order.**

```sooth
: wrap ( 'T -- Pair['T] ) One ;
: mk1 ( i64 -- Pair[i64] ) wrap ;
: mk2 ( Pt -- Pair[Pt] ) wrap ;

: main ( -- )
  1 2 Pt mk2 drop
  7 mk1 drop ;
```

Builds clean, then `Segmentation fault`, `EXIT=139`. With `mk1`/`mk2` swapped it does not
even build: `unreachable code: an aggregate field is copied by blit, not scalar-stored`
(`src/backend/qbe.rs:534`).

**Controls (all exit 0).** Two *concrete* construction sites at two monomorphs, with no poly
word anywhere. Two `Pair?` eliminations at two monomorphs inside two concrete words. A single
instantiation of any of B1-B3. The concrete path is correct throughout, because
`builtin_overloads` carries its mangled resolution.

The concrete **check** side already has the right rule: `check_eliminator_call`
(`src/check.rs:2564`, S3b R5) treats the name-keyed entry as a family gate only and reads the
operative `EnumId` off the scrutinee's own type. Neither the poly check side nor lowering has
any such rule.

## Shape of the fix

1. Per-monomorph identity for a generated enum word reached from a poly body: a family-gate
   rule on the check side, a per-instantiation record on the lowering side (`R1`).
2. A registry value that can name a generic header (`R2`).
3. A `PolyType` for a narrowed generic variant, the poly twin of `Type::Variant` (`R3`).
4. A `PolyType -> PolyType` field substitution (`R4`).
5. A generic-scrutinee branch in `poly_eliminator_call`, and a poly-aware variant destructure
   so an arm can reach the payload (`R5`, `R6`).
6. A diagnostic that distinguishes "there is no such eliminator" from "your arms are not
   adjacent to the call" (`R7`).

`R1` is the folded scope. It is not separable: this slice's exit criterion is a poly word
used at two instantiations, and every such program routes `Option?`, `Some>` and `Some`
through the colliding keys.

## Requirements

### R1 -- a generated enum word in a poly body resolves per monomorph

- **R1.1 (check side, family gate)** `poly_eliminator_call` stops using the registry's id as
  identity. The registry entry gates the *name*; the operative header is read off the
  scrutinee slot's own `PolyType` -- `Concrete(Type::Enum(id, _))` gives the monomorph
  directly, `Generic { is_enum: true, idx, module, .. }` gives the header (R5.1). This is
  `check_eliminator_call`'s S3b R5 rule, restated one path over. A scrutinee whose *family*
  differs from the gate's stays the existing `type_mismatch_error`; a scrutinee that is a
  different monomorph of the *same* family is accepted, which is what fixes B1.
- **R1.2 (lowering side, per-instantiation record)** `CallInst` gains
  `enum_words: HashMap<Span, EnumId>`: a pure function of `(callee, θ)` over the *callee
  body's* spans, so two call sites sharing a `(callee, θ)` record identical maps. The poly
  walk records each generated-enum-word call site's header and arguments as it already
  resolves them (`poly_construct_generic`, `poly_eliminator_call`, R6.1); the θ pass grounds
  each through the same `apply_subst` route that mints the monomorph, yielding the `EnumId`.
  Empty on every monomorphic word, so the existing corpus lowers unchanged. Read at
  `src/ir/driver.rs:372`, beside `&inst.trait_calls`.
- **R1.2a (the three grounding sites; where `trait_calls` is not a usable model)** The
  previous draft said "built and threaded exactly as `trait_calls` is
  (`src/check/poly.rs:5132`)". That names one of `trait_calls`' three construction sites, and
  it is the only one that works as described. Measured, the three are `check_poly_call`
  (`src/check/poly.rs:5132`), `CrossGround::impl_mono_seed` (`:5570`) and `CrossGround::compose`
  (`:5779`). Only the first holds a live `GenericTypes`: the other two build their `Ctx`
  through `word_ctx(.., None)` (`:5546`, `:5637`), and `apply_subst`'s `Generic` arm
  (`:7546`) is a `poly_generic_not_yet_groundable_error` without one.

  The two are not vacuous, so this is not dismissible as "eliminator/construct calls only
  occur where `poly_walk` runs". `poly_walk` runs over every poly body, including these two
  sites' callees; what differs is only *where θ arrives*. `compose` grounds the body of a
  cross-called generic word, and such a body holds a generic enum word today -- built and run
  against `main` at `afd3d52`, with the shared preamble above:

  ```sooth
  : sink ( Pair[i64] -- i64 ) ~[ ( One ) One> ] ~[ ( Nil ) Nil> 0 ] Pair? ;
  : inner ( 'T -- i64 ) drop 5 One sink ;
  : outer ( 'U -- i64 ) inner ;
  ```

  `7 outer .` prints `5`. `inner`'s `One` is a `poly_construct_generic` site in a body whose
  only θ arrives through `compose`. `impl_mono_seed` is the same shape one door over: a
  generic-`impl:` member word's body is an ordinary poly body. The in-tree justification for
  `compose`'s `None` -- "`compose` only ever grounds a callee's declared *output*, and phase 1
  already rejects every output shape that could mint one" -- holds for outputs (verified:
  `: outer ( 'U -- Pair['U] ) inner ;` is rejected with "returning the compound type
  `Pair['T]` ... is not yet supported from a polymorphic body") and stops being the whole
  story the moment a *body* span needs grounding. An all-`Concrete` argument list does not
  escape it either: `instantiate_enum` is find-or-mint, and there is no find-only door.

  Ruling: thread the live instantiator into `discover_transitive_instantiations` rather than
  invent a second carrier. `check_module`'s `generics_cell` (`src/check.rs:756`) is consumed
  at `:1022`, immediately *before* the call at `:1078`; move that `into_inner()` past the
  call, pass `Option<&RefCell<GenericTypes>>` as a new parameter, hang it on `CrossGround`,
  and give both `word_ctx(.., None)` sites `self.generics`. Probed on a throwaway tree: it
  compiles and `cargo test` is green (the one failure is pre-existing, from this worktree's
  in-progress `lib/option.sth`, and reproduces with the probe reverted). Two obligations ride
  with it:
  - a monomorph minted during the fixpoint is flushed into `module.enums`
    (`GenericTypes::flush_enums_into`) after the call and before layout, as
    `check_module`'s per-word loop already flushes at `:880`/`:1014`;
  - such a mint is *not* visible in the `self.enums` slice `CrossGround` borrows for the
    fixpoint's duration, so nothing inside the fixpoint may read a freshly minted decl back.

  `trait_calls` never had to care about any of this: `resolve_user_bound` is a registry
  lookup, not an interning mint.
- **R1.3 (the read)** `lower_call` consults `self.enum_words.get(&span)` **before** the bare
  `self.enums.words` lookup. On a hit it dispatches `lower_enum_call` with the recorded id.
  The bare-key lookups stay for monomorphic bodies, where they are unambiguous.

  As built the consult sits between the bare `self.structs.words` and `self.enums.words`
  lookups, not ahead of both as this rule first read. The difference is inert: `enum_words`
  is keyed by the span of a generated *enum*-word call, so a hit whose name also resolves in
  `structs.words` would need one name in both generated-word maps, which is a pre-existing
  collision the earlier stages already exclude.
- **R1.4 (what stays on the family id, and why)** Two reads are family-invariant and must
  **not** move:
  - the variant *count* in `lower_eliminator` (`calls.rs:920`) is read before the
    `self.stack.len() - n` split that locates the scrutinee slot, so it is structurally
    impossible to derive it from the scrutinee; it is also identical across monomorphs of one
    header.
  - the variant *index* lookup in `lower_clauses`
    (`self.enums.words[&clause.variant]`, `src/ir/func_builder/control_flow.rs:247`) is a
    position within the header's variant list, likewise identical across monomorphs.

  State this as a two-step rule in the code comment: **the family id locates and gates; the
  recorded id dispatches.** (A variant *name* shared by two unrelated enums is a separate,
  pre-existing module-blindness hazard and is out of scope.)
- **R1.5 (the splice path)** A record keyed by `Span` alone is wrong inside an `inline`
  combinator body, which is spliced at N sites (`splice_trait_calls` is keyed `(uid, span)`
  for exactly this reason, `src/check/poly.rs:1157`). This slice does **not** widen the key.
  Instead: a generated enum word reached through a combinator splice at a *generic* enum
  gets a located rejection naming the restriction.

  Measured after the fact: this is a message upgrade, not a safety gate. With the R1.5 check
  stubbed out its own fixture is still rejected, by the pre-existing
  variable-bearing-application error ("grounding a generic over its own type variable is not
  yet implemented"). Keep the sharper message; do not credit it with closing a hole. That
  also bounds the residual gap noted at `src/check.rs`'s pre-pass skip: a combinator without
  a `Bound::User` never reaches this check at all, and nothing miscompiles through the gap
  today only because the same older gate catches that shape first.
- **R1.6** No `expect` on the recorded id at lowering: a miss falls through to the bare key
  (the monomorphic path), it does not panic.

### R2 -- the registry admits a generic header, and never decides identity

- **R2.1** `eliminator_registry` returns `HashMap<String, EliminatorTarget>` where
  `EliminatorTarget` is `Concrete(EnumId) | Generic { idx: u32, module: u32 }` (`u32`
  throughout, matching `PolyType::Generic`'s own field types, `src/ast.rs:2029-2035`), built
  from `enums` **and** from `ctx.generics()`'s `enums: Vec<GenericEnumDecl>`.

  **Amended in phase 3, and the rule is corrected:** the `Generic` arm is
  `Generic { idx: u32 }` -- no `module`. Carrying it was shape parity with `PolyType::Generic`
  and nothing more: no consumer reads it (`poly_eliminator_call`'s `Generic` scrutinee arm
  needs only the header's variant list, and declaring-module identity is the wrong notion at
  an instantiation site -- the family is compared through the carried header *spelling*, as
  R5.1's own comment says). Its only assertion read the field back from
  `generics.enums[bare].module`, i.e. the value the code had just written there, so it proved
  storage and not use. A phase that needs to disambiguate two same-named headers from
  different modules adds the field back *with* a multi-module fixture; adding it ahead of that
  consumer buys a placebo.
- **R2.2 (threading, and the one site that cannot be threaded)** There are six call sites:
  `check_module` (`src/check.rs:692`), `check_def_collecting_drop_sites` (`:1284`),
  `infer_line` (`:1378`), `check_poly_combinator_repl` (`src/check/poly.rs:489`), `poly_walk`
  (`:668`) and `poly_call_term` (`:1821`). The first five can reach a live `GenericTypes`.
  `check_poly_combinator_standalone` (`src/check/poly.rs:362`) deliberately builds its `Ctx`
  with `generics: None` and then runs the body through the **concrete** `check_word` path, so
  its body's eliminator calls hit the concrete consumer at `src/check/terms.rs:601` with a
  registry that may now classify a name as `Generic`. That consumer must not fall through
  silently -- see R2.4. The claim that P7.S11's combinator path "shares no code with this" is
  withdrawn: the name gate is shared code.

  A seventh caller is a unit test, `eliminator_registry_keys_the_bare_surface_name`
  (`src/check/declarations.rs:2066-2077`), which calls `eliminator_registry(&module.enums)`
  with one argument and asserts `registry.get("Shape?") == Some(&id)`. Both halves die under
  R2.1 -- arity and value type. It updates as part of this slice: the second argument is the
  (empty) generic-enum decl list, and the assertion becomes
  `Some(&EliminatorTarget::Concrete(id))`. Same treatment as R5.3's `:10216`; called out here
  so it is not discovered mid-phase.
- **R2.3** A header that also has monomorphs registers **once**, as `Concrete`: that is what
  today's concrete path expects, and R1.1/R5.1 make the choice non-load-bearing. A header
  with no monomorph registers as `Generic`.
- **R2.4 (the concrete consumer)** `src/check/terms.rs:601` matches `Concrete(id)` and
  reaches `check_eliminator_call` unchanged. On `Generic` it produces a located rejection: a
  concrete body (including `check_poly_combinator_standalone`'s i64 stand-in) cannot hold an
  ungrounded scrutinee, and the stand-in has no instantiator to ground one with. This is a
  *stated* restriction with its own message, not a fallthrough into the unknown-word path and
  not the adjacency message.
- **R2.5** Both poly name gates widen to the same value type: `poly_walk`'s adjacency scan
  (`src/check/poly.rs:668`, via `tagged_literal_reaches_an_eliminator_call`,
  `src/check/terms.rs:1156`) and `poly_call_term`'s dispatch (`:1821`). `PolyCtx.eliminators`
  changes type with them.

### R3 -- `PolyType::GenericVariant`, with a deliberate arm at every match site

- **R3.1** Shape: `GenericVariant { idx: u32, module: u32, vi: usize, args: Vec<PolyType>,
  name: &'static str }`, mirroring `PolyType::Generic` (`src/ast.rs:2029`). Identity is
  `(idx, module, vi)`; `args` is the scrutinee's own argument list, carried forward
  unchanged; `name` is the leaked `Enum.Variant` display spelling, diagnostics only. A
  separate variant, not a flag on `Generic`: every predicate that must reject a *variant*
  (escape, `Copy`, projection) has to see it without reasoning about a boolean.
- **R3.2** Constructed in exactly one place, a `generic_variant_type` helper beside
  `variant_type` (`src/ast.rs:529`) and for the same reason: one origin for the leaked
  display string, so two constructions of one `(idx, module, vi)` compare equal.
- **R3.3 (the forced inventory -- 22 sites, measured)** Adding the variant to `PolyType` and
  building yields exactly 22 `E0004` non-exhaustive-pattern errors. Each gets a deliberate
  arm; none gets a `_ =>`.

  | File | Function |
  |---|---|
  | `src/ast.rs:1826` | `ground_member_poly` |
  | `src/check/audits.rs:361` | `contains_poly_reference` |
  | `src/check/audits.rs:386` | `audit_poly_input_quotation` |
  | `src/check/audits.rs:444` | `reject_poly_quotation_anywhere` |
  | `src/check/declarations.rs:632` | `collect_poly_concrete` |
  | `src/check/poly.rs:137` | `poly_is_copy` |
  | `src/check/poly.rs:330` | `is_reference_slot` |
  | `src/check/poly.rs:2528` | `poly_type_mentions_caller_var` |
  | `src/check/poly.rs:2543` | `poly_mentions_len_var` |
  | `src/check/poly.rs:4448` | `poly_copy_gate` |
  | `src/check/poly.rs:6384` | `match_impl_target_rec` |
  | `src/check/poly.rs:6548` | `collect_positions` |
  | `src/check/poly.rs:7162` | `unify_poly_input` |
  | `src/check/poly.rs:7463` | `apply_subst` |
  | `src/check/poly.rs:7690` | `poly_op_on_variable_error` |
  | `src/check/poly.rs:8461` | `poly_type_str` |
  | `src/ir/driver.rs:783` | `subst_polytype` |
  | `src/parser.rs:353` | `member_shape_is_supported` |
  | `src/parser.rs:436` | `poly_type_shape_str` |
  | `src/parser.rs:2125` | `generic_field_type_str` |
  | `src/parser.rs:2194` | `reject_growing_generic_argument` |
  | `src/repl.rs:317` | `remap_poly_type` |

  Four of these are in `src/parser.rs`. A `GenericVariant` is still unspellable and
  unparseable (R3.5); the parser arms exist because these functions match exhaustively over
  the shape, and each rejects or renders it rather than admitting it.

  Defaults: never `Copy` (it wraps a possibly-linear payload); `apply_subst` grounds it to
  the concrete `Type::Variant` of the minted monomorph, through the same instantiator (S3a
  D3, one id space); `poly_type_str` renders `Enum.Variant`.
- **R3.4 (silent wildcards that must become explicit)** These sites compile unchanged and
  would therefore mis-handle a `GenericVariant` in silence. Each is inspected, and converted
  to an explicit arm where a narrowed variant can reach it:
  - the eliminator arm-exit **escape check** (`src/check/poly.rs:3283-3290`), whose
    `_ => None` currently means "not a variant, let it escape". This is the load-bearing one:
    an escaped `GenericVariant` reads as trivially `Copy` outside the call and a later `dup`
    double-drops a linear payload (R5.5).
  - `poly_eliminator_call`'s own scrutinee `_ =>`
    (`poly_abstract_enum_scrutinee_error`, `src/check/poly.rs:3066`).
  - `poly_construction_fallback` (`:3921`), `poly_cross_match` (`:2432`), `poly_cross_output`
    (`:2469`), `receiver_is_aggregate_projection` (`:7844`), and `concrete_ty`
    (`src/ast.rs:1921`).

  The rest of the `_ =>` sites over `PolyType` are unreachable for a value that only exists
  between an eliminator call and its arm exits; the phase report lists which and why.
- **R3.5** `GenericVariant` is unconstructible outside an eliminator arm's own input row: no
  parse route, no signature spelling, no constructor. R3.3's parser arms are exhaustiveness
  and rendering, not admission.

### R4 -- a symbolic field substitution

- **R4.1** New function, `PolyType` field × `&[PolyType]` arguments -> `PolyType`. The brief
  calls this "a new `PolyType -> PolyType` substitution, no existing sibling to copy"; that
  is wrong. `ground_member_poly` (`src/ast.rs:1825-1862`) is the
  same `PolyType -> PolyType` walk, substituting `Var` against a single target rather than an
  indexed argument list, rebuilding `Array`/`Ref`/`OwnedCell` without interning, recursing
  structurally into `Generic`, and giving `QuotLit` a truthful `unreachable!`. This is an
  adaptation of that function, not new machinery, and should be sized accordingly.
- **R4.2 (arm set, matched to what is actually constructible)** A generic **enum variant**
  field can only be `Var` or `Concrete` at HEAD. Measured: `| B array['A 2]`,
  `| B Inner['A]` and `| B Cell2['A]` are rejected by the parser
  ("a quotation field naming `Box`'s type variable `'A` ... is not supported"), and
  `| B &'A` / `| B ^'A` are parse errors ("field `&'A` has no type before `;`"). Generic
  *struct* fields do admit `array['A 2]`, which is why `substitute_generic_field`
  (`src/ast.rs:815`) carries the wider arm set; this function is called only on enum variant
  fields.

  So the arms are `Var(v) -> args[v].clone()`, `Concrete(t) -> Concrete(t)`, and one truthful
  `unreachable!` naming the parser rejections above. This is the identical arm set to
  `poly_bind_construction_arg` (`src/check/poly.rs:3929-3969`), which is required: that
  function is the dual, and a field shape one accepts and the other rejects is a defect.
  R3.2/R5.2 of the previous draft contradicted each other on exactly this point; the narrow
  set is the resolution.
- **R4.3** It interns nothing and mints nothing, so it takes no `MutRegistries` and cannot
  recurse into the instantiator. An all-`Concrete` result does **not** fold: folding is
  `apply_subst`'s job at grounding time, and a fold here would need the instantiator this
  function deliberately does not hold.

### R5 -- the generic-scrutinee branch of `poly_eliminator_call`

- **R5.1** The branch is selected by the scrutinee slot's `PolyType`, per R1.1. The header is
  `Generic { is_enum: true, idx, module, args, .. }`; `is_enum: false` is the existing
  non-enum-scrutinee `type_mismatch_error`.
- **R5.2** Arm collection, written-order normalization, duplicate-arm, unknown-variant and
  exhaustiveness checks are the **same code** as the concrete branch, reading
  `generics.enums[idx].variants` instead of `enums[id].variants`. Variant names come through
  `generic_surface_name` on both sides, so the arm-tag matching rule is one rule. If sharing
  forces a borrow of the `RefCell` across the arm walk, the variant name/arity list is copied
  out and the borrow dropped first (`poly_construct_generic`'s `drop(generics)`,
  `src/check/poly.rs:4058`, is the precedent).
- **R5.3 (`PolyArm.declared_inputs`)** "The same code" is not free: `PolyArm.declared_inputs`
  is `Vec<Type>` (`src/check/poly.rs:3193`), built at `:3138` from `variant_type` and at
  `:3742` from a combinator's grounded `ins: Vec<Type>` (`:3458`), and consumed at `:3253` to
  build `inline_quotation_type` for `ordinary_literal_at_inline_param_error`
  (`src/check.rs:3083`, which takes a `Type`). A narrowed generic variant has no `Type`.
  Ruling: widen the field to `Vec<PolyType>`; the two existing producers wrap in
  `PolyType::Concrete`; the consumer at `:3253` keeps today's message for the all-`Concrete`
  case and gets a sibling that takes a `poly_type_str`-rendered parameter for the
  `GenericVariant` case. The existing unit test at `:10216`
  (`declared_inputs: vec![Type::I64]`) updates with it. This is visible work, not a footnote.
- **R5.4** Each arm's narrowed input is `generic_variant_type(idx, module, vi, args)` where
  `args` is the scrutinee's own `Generic.args`, unchanged. Nothing re-unifies: the scrutinee
  already carries the substitution.

  **Amended in phase 3:** built, but witnessed for *arity and family only* --
  `poly_eliminator_narrows_a_two_parameter_header_at_swapped_monomorphs` says so in its own
  comment. Nothing before phase 4's R6 destructure *reads* a narrowed variant's fields, so
  swapping the carried `args` is unobservable from either the unit test or its
  swapped-monomorph golden twin. The positional half of this rule has no witness until phase 4 lands R8.3's
  asymmetric-payload destructure, which is what R8.9's mutation 3 (reverse the `args`) kills.
  **Recommendation for phase 4:** mutation 3 is R5.4's only positional evidence -- it must not
  be retired or classified as inert without one.

  **Amended in phase 4:** paid off. `Option['T]` has one type parameter, so even R8.3's
  destructure could not have witnessed mutation 3 -- reversing a 1-element list is the
  identity. `a_two_field_variant_destructures_fields_in_declared_order_and_type` (a
  two-parameter, two-field header, `Two['A 'B] | Both fst 'A snd 'B`, each field a different
  type) is what mutation 3 actually needs, and it also witnesses R6.1's field-push-order
  claim below in the same fixture: both mutations turn it into `` `f` leaves `'B`, but the
  declared outputs are `'A` `` at check time.
- **R5.5** The escape check (R3.4) rejects a `GenericVariant` leaving an arm on exactly the
  grounds it rejects `Type::Variant`. Same message, the variant rendered through
  `poly_type_str`. A `Ref` wrapping a `GenericVariant` is caught too, matching the concrete
  arm's `Ref` case.

  **Amended in phase 3:** the bare `GenericVariant` half is witnessed
  (`a_generic_variant_escaping_its_arm_is_rejected`, and mutation 4 kills exactly it). The
  `Ref` half is **unreachable today** and therefore unwitnessed: producing a `Ref` to a
  narrowed variant needs the variant parked in a local first, and binding one is rejected
  earlier -- ``cannot borrow the local `v` of type `Option.Some` `` (measured on
  `~[ ( Some ) | v | &v .. ]`). It stays in, at exactly the depth the concrete arm has always
  looked to, because the peel is what makes the classification exhaustive; it is a
  guard-in-advance, not a closed hazard.
- **R5.6** `drop` of a narrowed `GenericVariant` inside its arm is accepted (the concrete
  twin accepts `drop` of a `Type::Variant`) and lowers per instantiation.
- **R5.7** Scrutinee modes: this slice admits the **owning** mode only. `&`/`&!` narrowing
  needs `intern_ref_type` over a shape that has no `Type` yet, which is R4.3's explicit
  non-goal. A `( &Some )`-tagged arm over a generic scrutinee is a located rejection naming
  the restriction, not silence and not a fallthrough into the concrete branch.

### R6 -- reaching the payload

- **R6.1** A poly-aware destructure intercept in `poly_call_term`, ahead of the ordinary
  `env` dispatch and next to `poly_construct_generic`'s interception for the same reason (a
  single registered concrete candidate under the bare name would otherwise commit): a call to
  `{Variant}>` whose operand is a `GenericVariant` pushes that variant's fields, in declared
  order (first field deepest), each substituted through R4.1 with the variant's own `args`.
  It records its header and variant index for R1.2.

  **Amended in phase 4:** "records its header and variant index" overstates the implementation
  by one field. `poly_destructure_generic` (`src/check/poly.rs:4425`) records only the header
  (`tctx.enum_sites.push` carries a `PolyType::Generic` with no variant slot); the variant
  index is not pushed here. This is not a gap: `apply_subst`'s `Generic` arm grounds the
  header to a concrete `Type::Enum`, and `lower_clauses` reads the variant index off the
  eliminator's own family id at lowering time (R1.4), not off `enum_sites`. Read "records its
  header" and drop "and variant index" as this rule's actual scope.
- **R6.2** It is the exact dual of `poly_bind_construction_arg` (`src/check/poly.rs:3929`):
  construction binds header variables from operands, destructure applies them to fields. The
  two read the same `GenericVariantDecl.fields` and share R4.2's arm set.
- **R6.3** A zero-field variant destructures to nothing (the concrete rule, Phase 6 slice 2
  R7), which is what makes `~[ ( None ) None> .. ]` work. R8.5 witnesses it.
- **R6.4** Field *projection* (`&field` into a generic variant) is out of scope and stays
  rejected: `no field projection in a poly body` is a standing, separately-tracked gap.

### R7 -- the diagnostic split

- **R7.1** `tagged_literal_reaches_an_eliminator_call` returns a three-state result, not a
  `bool`: reached; a following call that is not an eliminator name (an adjacency mistake, in
  the sense the current message describes); no following call at all. Both gates report the
  existing `eliminator_arm_outside_call_error` for the adjacency states, unchanged text.
- **R7.2** A new message covers the case this slice makes reachable and does not fix: the
  arms are adjacent to a call that names no eliminator at all, i.e. neither a monomorph nor a
  generic header of that name is in scope. It names the call, says no such eliminator is in
  scope for it, and does not talk about adjacency. House style is
  `eliminator_arm_outside_call_error`, including the trailing `(line {})` and the `in_word`
  clause, and it keeps the `"error: "` literal its siblings carry (the doubled prefix is a
  known, separately-tracked defect; match the siblings, do not fix it here).

  **Amended in phase 4:** the implementation narrows this rule further than written --
  `tagged_literal_reaches_an_eliminator_call` only classifies a call as `NamesNoEliminator`
  when the name both fails the registry lookup **and** ends in `?`; a non-`?` call falls to
  `NotAdjacent` (the adjacency message) instead. This resolves a real R7.1/R7.2 tension
  rather than being an oversight: R7.1's "adjacency states" is plural and R7.3's own
  `drop`/`swap` witnesses (`tests/phase6_slice3b.rs:105`, `tests/phase7_slice3b.rs:135,170`,
  `src/check.rs:5437`) all pin the adjacency text for a non-eliminator-shaped call sitting
  right there. `?` is the generated-eliminator naming convention
  (`eliminator_registry`'s own key shape), so a `?`-suffixed miss reads as a typo'd
  eliminator name specifically, while any other call reads as a genuine written-adjacency
  problem. Both call sites carry this narrowing (`src/check/poly.rs:746` for a poly body,
  `src/check/terms.rs:1077` for a concrete one; the latter was previously unwitnessed, closed
  by `a_typod_eliminator_call_in_a_concrete_body_names_no_eliminator_rather_than_an_adjacency_mistake`).
- **R7.3** After this slice, the brief's repro is accepted, so its message is not merely
  reworded. A witness for R7.2 has to be a genuinely absent eliminator (a typo'd `Optionn?`).
- **R7.4** R1.5's splice rejection and R2.4's stand-in rejection are each their own located
  message. Neither reuses the adjacency text.

### R8 -- tests

All goldens assert `status.code() == Some(0)` alongside stdout, so a signal death or a
backend panic is a failure rather than a missing line.

- **R8.1 (golden, R1, live-today witnesses)** `tests/phase7_slice12.rs`: B1, B2 and B3 above,
  each in **both** declaration orders (four programs for B1/B2, two for B3). At `afd3d52`
  B1/B2 false-reject in one order, B3 segfaults in one order and fails to build in the other.
  All six must build, run and print. Both orders is the point: the defect *is* order
  sensitivity, and a single order cannot witness a last-write-wins key.
- **R8.2 (golden, exit criterion)** The brief's `is-some` over `Option['T]`, built and run at
  **two instantiations with different payload layouts** (`i64` and a struct), asserting both
  stdout values. One instantiation cannot witness R1 (S3a R4's rule: an all-`i64` pair is a
  layout placebo, and a symmetric pair cannot tell positional routing from its swap).

  **Amended in phase 3:** witnessed over a **locally declared** `Option['T]`, not over
  `lib/option.sth`'s. Two walls stand between the two, both pre-existing and neither in this
  slice's scope:
  1. a poly word over an *imported* generic enum is rejected at its call site --
     `: idop ( o::Option['T] -- o::Option['T] ) ;` gives ``expected `o::Option['T]`, found
     `Option[i64]` ``, with no eliminator anywhere in the program;
  2. behind it, `poly_eliminator_call`'s family gate compares surface *spellings*, so an
     imported scrutinee renders `o::Option['T]` against family name `Option` and falls into
     the rendered-mismatch arm (``expected `Option`, found `o::Option['T]` ``).

  The second is a distinct fix from the first, so clearing the call-site mismatch alone will
  not reach `lib/option.sth`. **Recommendation for whichever slice takes cross-module generic
  instantiation** (see the standing `export:`-cannot-carry-`Result[i64 i64]` limit): budget
  both, and re-key the family gate off header identity rather than its rendered spelling.
- **R8.3 (payload golden)** `unwrap_or`-shaped: a poly word whose `Some` arm destructures the
  payload and returns it, run at two instantiations. This is R6's only end-to-end witness.
- **R8.4 (construction inside the poly body)** A poly word that both constructs and
  eliminates `Option['T]`, at two instantiations. Unlike the previous draft, this is *in*
  scope: R1.2 is what makes it lowerable.
- **R8.5 (zero-field destructure)** `~[ ( None ) None> .. ]` over a generic scrutinee, at two
  instantiations (R6.3). Previously unwitnessed.
- **R8.6 (rejections, each asserted on message text)** an ungrounded scrutinee with a missing
  arm; a duplicate arm; an unknown variant tag; a `GenericVariant` escaping its arm; a
  `&`-mode tag over a generic scrutinee (R5.7); a generic eliminator call inside an `inline`
  combinator body (R1.5); the same inside a standalone-checked combinator (R2.4); R7.2's
  absent-eliminator message. Each names the enum and the call.

  **Amended in phase 3:** the R1.5 case is not witnessed on R1.5's own message, matching R1.5's
  own construction-side finding from phase 1. `a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire`
  is the fixture; it asserts the standing variable-bearing-application rejection, because a
  combinator carrying a generic-enum slot is rejected during its standalone check before
  `poly_eliminator_call`'s R1.5 arm can run. R1.5's gate is a sharper message waiting on a
  restriction this slice does not lift, not a safety net this slice closes -- see R1.5's own
  note. R2.4's stand-in case is unaffected and is witnessed as specified, on its own text.
- **R8.7 (no false rejection)** These existing files pass unmodified: `tests/phase5_slice1.rs`,
  `tests/phase5_slice2.rs`, `tests/phase6_slice1.rs`, `tests/phase6_slice3.rs`,
  `tests/phase6_slice3b.rs`, `tests/phase7_slice3a.rs`, `tests/phase7_slice3b.rs`,
  `tests/phase7_slice3b_follow.rs`, `tests/phase7_slice3c.rs`. (The previous draft cited
  `tests/phase5_generic_enum_elimination.rs`, which does not exist.) R2.3's registration
  choice is only sound if the concrete path is byte-identical, and these are what say so.
- **R8.8 (unit tests beside the stage)**
  - `src/ast.rs`: R4.1 over a `Var` field and a `Concrete` field leaves the result symbolic.
    The draft also asked to assert `refs`/`arrays` are untouched; **amended in phase 2**:
    R4.3 gives the function no registry parameters at all, so an interning slip is not
    representable and the assertion could only pass. It was written, measured as a placebo,
    and deleted; no-interning is carried by the signature and documented there.
  - `src/check/declarations.rs`: a header with monomorphs registers `Concrete`, one without
    registers `Generic` (R2.3).
  - `src/check/poly.rs`: R1.1's branch selection in both directions (a `Concrete` gate with a
    `Generic` scrutinee and the reverse); R5.4's narrowing carries the scrutinee's args
    positionally; R6.1's destructure pushes fields first-deepest.
  - `src/check/poly.rs` (**added in phase 2**): `apply_subst`'s `GenericVariant` arm, the one
    arm of R3.3's 22 that computes rather than rejects, over a hand-built `GenericVariant` --
    both sides of its display lookup (an unflushed mint read through `GenericTypes::enum_decl`,
    and the same mint read off `ctx.enums()` after a flush and rebase). Its id-range guard is
    *not* witnessable: reverting it to the previous `unwrap_or_else` fallback leaves the test
    green, because the two paths can only disagree when `enum_base` is stale relative to
    `ctx.enums().len()`, which `check_poly_body`'s rebase-before-walk ordering excludes. It is
    a diagnostic tightening (a truthful panic in place of an out-of-bounds index), not a
    behaviour fix. If phase 3 ever grounds a `GenericVariant` from a context that does not
    rebase first, that ordering becomes load-bearing and needs its own witness.
  - `src/ir/func_builder/calls.rs`: R1.3 dispatches on the recorded id when the bare-key
    `EnumWord` names a *different* monomorph; R1.6's miss falls through rather than panicking;
    R1.4's invariance (two monomorphs of one header have equal variant counts and equal
    per-name variant indices), which is that requirement's only evidence -- see R8.9's
    retired mutation 6.
- **R8.9 (mutation recipe, run against a committed tree)** Each must make a **named** test
  fail, classified on `test result: FAILED`, and the runner must confirm the mutated binary
  actually rebuilt. Five live mutations; 6 is retired and says why:
  1. make R1.3 skip the recorded id and use the bare key -- R8.1's unlucky orders and R8.2
     must fail.
  2. make R1.1 compare against the registry id again -- R8.1's B1 must fail.
  3. reverse the `args` passed in R5.4 -- R8.3 must fail at the asymmetric instantiation,
     which is why it uses two distinct payload types.
  4. delete R3.4's escape arm -- an R8.6 rejection must fail.
  5. make R2.3 always register `Generic` -- R8.7 must fail. If it stays green, R1.1 has made
     the registry value genuinely inert and R2.3 is a comment, not a requirement: say so in
     the phase report and drop it rather than keep an unwitnessed rule.
  6. **retired.** The previous draft's "move R1.4's variant-count read onto the recorded id
     -- must **fail to compile**" is false, measured at `afd3d52`: stubbing R1.3's map into
     `lower_eliminator` and routing the count through
     `enum_words.get(&span).copied().unwrap_or(id)` builds clean. `span` is already a
     parameter of `lower_eliminator`, so the recorded id is in scope there; the read's
     position relative to the `self.stack.len() - n` split blocks deriving `n` from the
     *scrutinee*, which is what R1.4 actually says, and blocks nothing else. The edit is a
     semantic no-op besides: `substituted_enum_variants` (`src/ast.rs:1072`) emits one
     `VariantDecl` per header variant in header order, so both the count and `lower_clauses`'
     variant *index* are equal across every monomorph of one header by construction.

     R1.4's evidence class is therefore a **passing invariance test**, not a mutation: R8.8
     gains one that mints two monomorphs of one generic enum header and asserts equal variant
     counts and equal per-name variant indices. That is the property "the family id locates
     and gates" rests on, and it is the thing that would have to break for R1.4 to be unsound.
     No mutation can witness it, because no mutation of a family-invariant read changes
     behaviour.

## Out of scope

`&`/`&!` narrowing of a generic scrutinee (R5.7); field projection into a generic variant
(R6.4); widening the `Span`-keyed record to `(uid, span)` for the combinator splice path
(R1.5 rejects instead); nesting depth beyond S3a's D5; relaxing S3a's D6 `Copy` rule for a
variable-bearing generic; the doubled `"error: "` prefix; the module-blindness of a variant
name shared by two unrelated enums (R1.4); P7.S11's construction-inside-a-combinator gap
itself -- but note R2.2: the *name gate* is shared code, and R1.5/R2.4 are this slice's
obligations at that boundary.

## Phasing

**Phase 1 -- R1 (L).** Per-monomorph identity, check side and lowering side together. They
must land together: R1.1 alone turns B1/B2 from a false rejection into a miscompile, and R1.2
alone leaves B1/B2 unreachable. R1.2a's instantiator threading lands here too, ahead of the
`enum_words` grounding that depends on it. Exit: R8.1's six programs, R8.8's `calls.rs` unit
tests (including R1.4's invariance test), and mutations 1 and 2. Standalone, no dependency on
anything below, and it closes three live runtime/build failures.

**Phase 2 -- R3 + R4 (M).** The `PolyType` variant, its 22 forced arms, R3.4's wildcard
conversions, and the symbolic substitution. No behaviour change: nothing constructs a
`GenericVariant` yet, so the arms are exhaustiveness-only. R8.8's `src/ast.rs` tests land
here, plus the `apply_subst` witness R8.8 gained.

The escape check (R3.4's load-bearing site) is one exhaustive `match` over `PolyType` rather
than two matches ending in `_ => None`, so the next `PolyType` variant cannot fall through it
in silence -- which is the whole reason R3.4 named it. `poly_eliminator_call`'s scrutinee
`_ =>` is deliberately *not* converted here: it is R5.1's own branch site and phase 3 replaces
it. The remaining R3.4 wildcards are justified in place, at each site, rather than in a
separate report.

**Phase 3 -- R2 + R5 (hard, L).** The registry widening (including R2.4's stand-in
rejection), `PolyArm.declared_inputs`' widening, and the generic branch up to and including
the escape check. Exit: R8.2 builds and runs; R8.6 minus the payload and R7.2 cases;
mutations 4 and 5.

**Phase 4 -- R6 + R7 (M).** The destructure intercept and the diagnostic split. Exit: R8.3,
R8.4, R8.5, the remaining R8.6 cases, mutation 3.

**Phase 5 -- bookkeeping (S).** P7.S12 to `[ done ]` with the shipped messages; the growth-
signal re-run over `src/check/poly.rs` (already at 3/5 signals per `poly_rs_split_deferred`,
and this slice adds to it) and over `src/ir/func_builder/calls.rs`.

## Exit criteria

- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- B1, B2 and B3 build, run and print correctly in **both** declaration orders.
- The brief's `is-some` over `Option['T]` builds, runs, and prints correctly at two
  instantiations with different payload layouts.
- A poly word's arm reads a generic variant's payload, including a zero-field variant.
- A poly word may construct *and* eliminate a generic enum at two instantiations.
- Every R8.6 rejection is a located message naming the enum and the call; no eliminator
  rejection reaches `eliminator_arm_outside_call_error` unless the arms really are
  non-adjacent.
- No `_ =>` catch-all for `PolyType::GenericVariant` anywhere, and every wildcard in R3.4's
  list is either converted or justified in the phase report.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "per-monomorph identity, check and lowering sides", "effort": "L", "difficulty": "standard" },
    { "phase": 2, "focus": "PolyType GenericVariant and field substitution", "effort": "M", "difficulty": "standard" },
    { "phase": 3, "focus": "registry widening and generic eliminator branch", "effort": "L", "difficulty": "hard" },
    { "phase": 4, "focus": "destructure intercept and diagnostic split", "effort": "M", "difficulty": "standard" },
    { "phase": 5, "focus": "bookkeeping and growth-signal re-run", "effort": "S", "difficulty": "standard" }
  ]
}
```
