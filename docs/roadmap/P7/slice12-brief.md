# P7.S12 brief — Eliminating an ungrounded generic enum inside a poly body

## Problem, confirmed live against current `main`

```sooth
type: Option['T] | None | Some 'T ;

: is-some ( Option['T] -- i64 )
  ~[ ( Some ) drop 1 ]
  ~[ ( None ) drop 0 ]
  Option?
;
```

fails with:

```
error: this quotation is annotated `( Some )`, an eliminator-arm tag, but it is not consumed
  by a call to a generated eliminator in `is-some` (line 4)
  arms are written together, immediately before the call: `~[ ( A ) .. ] ~[ ( B ) .. ] Enum?`
```

This message is the same one `eliminator_arm_outside_call_error`
(`src/check/terms.rs:1180`) gives for an actual written-adjacency mistake (a typo'd call
name, an intervening term). Here there is no adjacency mistake — the arms are written
correctly, immediately before `Option?` — and the message is misleading about the real
cause.

## Root cause, isolated by instrumenting `poly_walk`

`eliminator_registry` (`src/check/declarations.rs:1889`) builds `"{Enum}?" -> EnumId` from
`enums: &[EnumDecl]`. That list is the module's already-*monomorphized* enum declarations —
a generic enum's header is staged separately, in `GenericTypes` (the P7.S3a instantiator),
and only flushed into `enums` when something in the program concretely instantiates it. In
the repro above, nothing ever instantiates `Option` concretely, so `Option` is never in
`enums`, so `"Option?"` is never a key in `eliminator_registry` — confirmed by instrumenting
`poly_walk` (`src/check/poly.rs:668`): `eliminators.keys()` is `[]` at the point `is-some`'s
body is checked. `tagged_literal_reaches_an_eliminator_call`
(`src/check/terms.rs:1159`) then correctly reports "no such eliminator", which
`poly_walk` renders through the adjacency-mistake message because that is the only message
this call site has.

**A second, independent gap sits behind the first.** Even if the registry did carry
`Option`'s id (say, because some other word in the same module happens to instantiate
`Option[i64]` concretely first, flushing it in before this body checks — a real, order-
dependent accident, not a fix), `poly_eliminator_call`'s scrutinee match still has no arm for
`PolyType::Generic`. Its concrete arm is `PolyType::Concrete(Type::Enum(found, _)) if
*found == id`, matching an `EnumId` that names one specific monomorph. An ungrounded
`Option['T]` scrutinee's `PolyType` is `Generic { is_enum: true, idx, module, args: [Var(0)],
... }` — a different shape entirely, naming the *generic header* by `(is_enum, idx, module)`,
not a monomorph by `EnumId`. Nothing today unifies those two identities.

## Controls (confirming the scope of what's broken)

| Shape | Result |
|---|---|
| Concrete, non-generic enum (`Tri`), eliminated inside a poly word | works (gets past this check, reaches ordinary arm-exit checking) |
| Generic enum, concretely instantiated (`Option[i64]`), eliminated inside a *monomorphic* word | works |
| Generic enum, still ungrounded (`Option['T]`), eliminated inside a *poly* body | **broken**, both ways above |

So the break is specifically: ungrounded generic scrutinee × poly-body eliminator call. Not
generic enums in general, not poly bodies in general.

## Consequence

`Option`/`Result`'s eliminator (`Option?`/`Result?`) cannot be called inside *any*
polymorphic word body while the scrutinee's argument is still a type variable — which is the
normal, expected shape for a library word operating over `Option['T]` for an arbitrary `'T`.
This is independent of **P7.S11** (which blocks *constructing* `Option['U]` from inside a
combinator's standalone check): S11 blocks `map`'s output arm; this blocks `is-some`,
`unwrap_or`, `and_then`'s *input* pattern-match, with no shared code path and no shared fix —
different registry (`eliminator_registry` vs. the `GenericTypes` instantiator),
different function (`poly_eliminator_call` vs. `poly_construct_generic`/`unify_poly_input`).

## Resolved by probe (two independent read-only investigations, converging)

**Data exists, no new storage needed.** `GenericTypes` (`src/ast.rs:591-636`) already stages
a generic enum's header pre-instantiation: `pub enums: Vec<GenericEnumDecl>` (`:593`),
populated at the parse-time prepass, addressable by plain `idx` — `instantiate_enum` itself
reads `self.enums[idx]` (`:1193`) to mint a monomorph, so this is an existing, already-used
access pattern, not new plumbing. Each `GenericVariantDecl`'s fields are stored as
`Vec<(String, PolyType)>` (`:566`) — the header-local type variable is already symbolic data,
not something that needs deriving.

**Reachability is free.** `poly_eliminator_call` already takes `ctx: &Ctx`
(`src/check/poly.rs:2979`), and `Ctx::generics()` (`src/check/engine.rs:1335`) is `Some` on
the native `check::check` path that calls it. No signature threading needed to reach the live
`GenericTypes` registry from inside the function.

**The real blocker is three concrete-only sites, and there is a working precedent to mirror.**

1. Two name-keyed gates exclude a generic-header call name before `poly_eliminator_call` ever
   runs: `poly_walk`'s adjacency scan (`poly.rs:668`, over `eliminator_registry`) and
   `poly_call_term`'s own dispatch (`poly.rs:1821-1822`), both keyed on `HashMap<String,
   EnumId>` — a generic header mints no `EnumId`, so neither gate can ever admit one. The
   value type needs widening to something like `Concrete(EnumId) | Generic(usize, u32)`,
   which also touches `PolyCtx.eliminators`'s type and its concrete-path consumer
   (`src/check/terms.rs:601`).
2. Once past the gate, the arm's narrowed input type is computed via `variant_type`
   (`src/ast.rs:529`), returning an opaque `Type::Variant` marker with no generic
   counterpart — both the narrowing (`poly.rs:3132-3134`) and the escape check
   (`:3279-3298`) are hard-wired to it.
3. The field's *payload* type (what an arm binds after calling `Option>`) is fetched through
   `variant_generated_sigs`/`variant_field_projection` (`src/check/declarations.rs:1779`,
   `src/check.rs:469`), which reads a monomorphized `EnumDecl.variants[..].fields:
   Vec<(String, Type)>` — concrete-only, dispatched as an ordinary `env` call, never through
   any poly-aware arm. There is no poly-aware destructure intercept for a still-generic
   variant today.

**The construction path already solves the dual problem, in reverse.**
`poly_construct_generic` (`poly.rs:4004`, P7.S3a) reads `generics.enums[idx].variants[..].fields`
directly off the live header (`:4041`) and binds the header's type variables via
`poly_bind_construction_arg` (`:3929`) — a working, symbolic `PolyType`-aware mechanism for
*building* a `Some`/`None` value from a poly body. Elimination needs the same mechanism run
backward: substitute a `GenericVariantDecl` field's header-local `PolyType` through the
scrutinee's own already-known `Generic.args` (the scrutinee already carries the substitution;
nothing needs re-unifying, only applying) rather than through `substitute_generic_field`
(`ast.rs:815-833`), which grounds to a concrete `Type` and needs concrete arguments — the
wrong shape for a still-rigid poly body.

**So: bounded, not open-ended, but a real three-site fix, not a small patch.** Touches
`src/ast.rs` (a new `PolyType -> PolyType` substitution, no existing sibling to copy),
`src/check/poly.rs` (registry-widening at the two gates, plus a generalized or parallel
`poly_eliminator_call` that branches on concrete-vs-generic scrutinee data), and
`src/check/declarations.rs`/`src/check/terms.rs` (the registry value-type change and its
concrete-path consumer). The misleading diagnostic (a real adjacency mistake and "no such
eliminator for this scrutinee shape" currently share one message) should get distinguished
wording as part of the same slice, since the fix changes which cases are real rejections.

## Ready to spec: yes

The design direction is settled — mirror `poly_construct_generic`'s mechanism in reverse,
widen the two name-keyed gates to admit a generic header identity, and give
`poly_eliminator_call` a generic-scrutinee branch reading `GenericVariantDecl` fields through
the scrutinee's own substitution. A minimal `Option['T]`-eliminating poly word
(this brief's `is-some` repro) is the golden.
