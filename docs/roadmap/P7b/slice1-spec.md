# P7b.S1 spec — kinds and type-level application

Technical specification for compiler slice P7b.S1. Scope input is the recon
[brief](./slice1-brief.md) (findings F1-F6, machinery map, rulings R1-R8, witnesses
W1-W3, golden list) and the verbatim [probe log](./slice1-probes.md); exit criteria are
from the [phase doc](../P7b-higher-kinded-types.md). All anchors below were re-verified
against HEAD `790b81c` this session (see [Anchor status](#anchor-status)).

Diagnostic texts here pin **shape**, not wording (located `(line, col)`, single `error:`
prefix, names the offending position and the kind's origin, parenthetical advice). Exact
strings freeze when the goldens are written, per the S12 precedent (brief R7).

**Revision 260830 (after a three-reviewer round — soundness, implementability,
consistency):** the constructor-representation ruling was approved by the user:
`Type::CtorImage(GenericId)` (see S1-12). S1-10/S1-11/S1-12, S1-16, and the phase cut were
rewritten around it; the forced-arm census was corrected (the phantom `repl.rs:317` site
removed; five missed matchers and seven wildcard-armed sites added, and
`generic_args_of` re-bucketed as wildcard-armed); the kind-collection
paths now cover the header/field and annotation paths; S1-3's count-slot parenthetical
was corrected against S1-7.

## Exit criteria (from the phase doc)

1. A type variable may carry a higher kind (`* -> *`, `* -> Len -> *`, …).
2. `'F['T]` type-checks and monomorphizes to a concrete type.
3. Kind-incorrect application is a **located** error.
4. A signature may mention `'F['T]` where `'F` is a type variable of kind `* -> *`.

Kinds are inferred from usage context (`'F` appearing as `'F['T]` ⇒ `* -> *`), with an
explicit annotation (`'F: * -> *`) as fallback. S1 plants `Arrow`; it does not re-introduce
a separate kind mechanism beside S6a's `Star`/`Len` (phase-doc rule; brief R2).

## Background the requirements depend on

Established by the probes; cited here so each requirement is traceable.

- **There is no variable-headed type-application grammar anywhere** (F1; k4/k4b/k5/k7). In
  an effect, `[` after a *variable-headed* type expression reads as a quotation-effect
  opener (`require_top_depth_arrow`); in `type:` field positions it is `expected a word,
  found LBracket`. Even a concrete head `i64['T]` dies the same way. Generic *headers*
  (`Box['T]`, `Result['T 'E]`) are the one exception — they already parse as applications
  via `parse_poly_generic_application` (`parser.rs:3637-3652`) — but a variable head
  (`'F['T]`) has no production. So S1's parse production is prerequisite to the whole
  trait surface (F6).
- **The kind-annotation position is the bound/capability position** (F2; k3/k3b). `: Len`
  works (S6a); `: *` is rejected as `unknown capability`. Arrow-kind syntax must be carved
  out beside bounds at that one site.
- **No kind inference exists** (F3; k8/k1). An unannotated length var in `array['T 'N]` is
  rejected; `: Len` is mandatory. Inference-from-usage is new machinery. **S1 changes
  k8's outcome on purpose**: under first-mention-binds (S1-3) the program is *accepted*
  with `Star`/`Len` inferred from the count slot — annotations become
  optional-but-available, and S6a's "never appears in the effect" error remains for
  never-mentioned vars. A golden pins the flip.
- **No dedicated kind-mismatch diagnostic** (F4; k3d/k3c). A `: Len` var used as a value
  type reports "never appears in the effect" — a mislabel.
- **Applied-target impls already work** (F5; k6b): `impl: Functor for Option[i64]` with a
  `Functor`-bounded word dispatching at `Option[i64]` compiles and runs. S2 owns the
  constructor-abstract target (`for Option`); S1 does not touch it.
- **`Kind` is parser-private and dies in the parser** (machinery map): `enum Kind { Star,
  Len }` at `parser.rs:1346`; `parse_trait_decl` discards the parsed kind; a variable's kind
  is encoded structurally by which name table indexes its id. An `Arrow`-kinded variable is
  still a *type* variable, so that encoding breaks — kinds must start travelling.

## Requirements

Each requirement cites its brief ruling and supporting probes. `R#` = brief ruling; `F#`
= brief finding; `k#` = probe.

### Kinds (Phase 1)

**S1-1 (R1).** Promote `Kind` from `parser.rs:1346` to `ast.rs` and add
`Arrow { domains: Vec<Kind>, result: Box<Kind> }`. The arrow is **n-ary, not curried**:
Sooth application is not curried — every application site splits type slots from length
slots (`parse_type_arguments` takes `(ty_arity, len_arity)`; `PolyType::Generic` carries
parallel `args`/`len_args`), so `array` is honestly `* -> Len -> *`. Retain `Star` and
`Len` unchanged. `Kind` gains `Debug`, `Clone`, `PartialEq`, `Eq` (needed by S1-4's
unification and S1-14's rendering).

**S1-2 (R1).** The kind-expression grammar accepts `*`, `Len`, and n-ary arrows
(`* -> *`, `* -> Len -> *`). Len domains parse from day one; **S1's goldens exercise Star
domains only** (arrow results are always `*` in the golden set). The grammar shape must not
collide with trait/capability names sharing the bracket (S1-9).

**S1-3 (R2).** Kind **collection** stays parser-side in `PolyBuilder`. It already interns
every variable with a kind (`intern_ty_var`/`intern_len_var`, `parser.rs:1470-1498`) and
fires the Star/Len conflict at intern time, but drops the map at `finish` (`:1500`). Change
`finish` to retain it. Each mention records a kind **requirement** with its span:

- bare type slot → `Star`;
- application head (`'F` in `'F['T]`) → `Arrow { domains, result: Star }`, domains from
    the argument slots (each `Star` in S1's set — an `App` carries type args only, S1-7;
    `Len`-domain applications are fenced to S2+);
- application argument → `Star`;
- count position → `Len`.

  First mention **binds**; later mentions **check**. This is why collection is parser-side:
  mention spans, which every new diagnostic needs, exist only there, and the X1 Star/Len
  conflict is already parser-side (no second kind mechanism, R2/phase-doc rule).

**S1-4 (R2).** Kind **unification** runs as **deferred validation on two clocks**,
following the `validate_pending_quotation_rows` precedent (`parser.rs:1458`, invoked from
`parse_poly_effect` `:3423`):

- **usage-vs-usage conflicts** (two mentions of one variable demanding different kinds)
  validate at **signature end** inside `parse_poly_effect`, where all mention spans are in
  hand — located errors carrying **both** spans (the binding mention and the conflicting
  mention). This is where S1-15.d's arity-conflict diagnostic fires.
- **annotation-vs-usage conflicts** validate at **attach time**, in
  `attach_bracket_bounds` (`parser.rs:2520`): bound brackets parse into a side table that
  is attached to effect-derived ids only *after* `parse_poly_effect` returns
  (`parser.rs:2384-2392`, never pre-interned), so a signature-end pass cannot see them.
  The annotation's kind is recorded there as a requirement with its own span, and a
  conflict with the collected usage requirements is **S1-15.c's firing point** (both
  spans: usage mention + annotation).

**S1-5 (R2).** Publish the resolved kinds. `PolySig` (`ast.rs:2244`) gains
`ty_kinds: Vec<Kind>` parallel to `ty_var_names`. `GenericStructDecl` and
`GenericEnumDecl` (`ast.rs:542`/`560`) and `ImplTarget` gain their own parallel kind
vectors so the checker can consume kinds at `unify_poly_input`, `apply_subst`, and the
annotation path (`attach_bracket_bounds`, `parser.rs:2520`). No default-to-`Star` shortcut
that would drop an `Arrow` on the floor: every published vector is length-matched to its
name table.

  The **header/field path has no `PolyBuilder`**: generic `type:` fields are parsed by
  `parse_generic_field_shape` (`parser.rs:5766`, variable arm ~`5795`) against plain
  `ty_var`/`len_var` tables, returning `PolyType` directly. Collection there mirrors S1-3
  into a per-decl kind-requirement side table (field mentions record requirements with
  their spans), validated at **decl end** — when `GenericStructDecl`/`GenericEnumDecl` is
  constructed — and published as the decl's kind vectors. W1's fields and kind-error
  golden #5 live entirely on this path.

### Application parsing (Phase 2)

**S1-6 (R4).** The `[`-router. In `parse_poly_slot`'s `'` arm (`parser.rs:3472`; the `'`
arm itself sits at `parser.rs:3497-3500` and calls `parse_poly_ty_var` `:3761`), when a
`[` follows the type variable, run a **boolean top-depth-`--` scan** as a **router**:

- a top-depth `--` present → **quotation slot** (today's behaviour, byte-for-byte
    unchanged — the LBracket re-enters the slot loop as it does now);
- absent → **type application**.

  The scan is a **new boolean helper** — a refactor extracting the depth-counting scan
  from `require_top_depth_arrow` (`parser.rs:4509`), which returns `Result<(), String>`
  and *errors* on absence, so it cannot be reused as a router predicate directly. The
  route is total: an application's arguments are type expressions and never contain a
  top-depth `--`. A quotation-*shaped* argument inside an application (`'F[[ i64 -- i64
  ]]`) is therefore not an application argument: inside an application's argument list a
  `[` opens a parse error ("expected a type, found `[`", S1-15 family, unit-tested) —
  S1 application arguments are type expressions only. Bare-`[` positions that start a
  slot with no preceding type are untouched — P7.S6 R4's quotation-unconditional rule
  survives (F1; k4). The interception template is the existing
  `parse_poly_generic_application` (`:3673`), which today explicitly skips `'`-led heads;
  S1 removes exactly that gate for the variable head.

**S1-7 (R3).** Add `PolyType::App { head: u32 /* index into ty_var_names */, args:
Vec<PolyType> }` to `ast.rs:2078`. The dedicated variant is chosen over reshaping
`Generic`'s head to an enum: it keeps `Generic`'s registry-index invariant intact and is
the lower-risk shape (R3). `head` names a variable; `args` are the applied type
expressions. The variant carries **type args only**: a `Len`-domain application is fenced
to S2+ (an annotated `* -> Len -> *` variable is legal grammar per S1-2 but unsatisfiable
under S1 — any application attempt raises S1-15.h's arity/kind-mismatch diagnostic, and
using such a variable in a plain type slot raises the kind-conflict diagnostic). The
parse-level twin needs the same variant: `RawTy` (`parser.rs:1286`) gains an `App`
variant and `raw_to_poly_type` (`parser.rs:4066`) gains the fold arm (including its
all-concrete fold logic). An empty application `'F[]` is a pinned error (the arity
diagnostic; unit-tested).

**S1-8 (R4/F1).** Add the application production to the **`type:` header/field** grammar
too (k4b): `parse_generic_field_shape` (`parser.rs:5766`, variable arm ~`5795`) — its
variable arm has no bracket continuation, unlike the sibling `&`-glued and `^`-glued
application arms. A field `f 'F['T]` must parse to a field whose type is `PolyType::App`.
Use-site constructor type arguments (a bare constructor `Box` supplied as a type argument
in `parse_type_arguments`' argument loop, W1's `Wrap[Box i64]`) parse to
`Type::CtorImage(box_generic_id)` (S1-12's representation), gated on the header variable's
kind (S1-15's use-site diagnostic covers the mismatched case).

  **Instantiation semantics for `App` fields** (W1's monomorphization path, previously
  undersigned): `substitute_generic_field` (`ast.rs:837`, wildcard `unreachable!` at
  `:910`) gains the `App` arm — an `App` field with a `CtorImage` head binding
  substitutes the application's arguments through the constructor's declared parameters
  and lays the field out as the concrete monomorph (`Box[i64]`); the same for
  `substitute_generic_variant_field` (`ast.rs:2225`, wildcard at `:2229` — enum variant
  fields parse through the same shape parser, `parser.rs:5222`/`5248`) and
  `poly_bind_construction_arg` (`check/poly.rs:4507`, wildcard at `:4546`, whose doc
  invariant "a generic `type:` field is always exactly one of these two shapes" S1-8
  falsifies — the doc must be updated). `struct_keys` (`ast.rs:609-610`) args carry the
  `CtorImage`, so `Wrap[Box i64]` and `Wrap[Opt2 i64]` instantiate to distinct monomorphs.

### Checking and grounding (Phase 3)

**S1-9 (R6).** Annotation grammar and the glued-colon hazard. `:` is not a lexer
delimiter — `'T:Len` fully-glued lexes as one word (S7c probe P2), and k3b shows
`'F:*->*`-style glue reads as one capability token. Extend the kind-expression parser
into:

- `parse_optional_bound_bracket` (`parser.rs:2443`), where `is_len_kind` already
    special-cases `Len` beside capabilities;
- `parse_header_bracket` (`parser.rs:5666`), where `header_bracket_unknown_kind_error`
    (`parser.rs:1727`) currently says "the only spellable kind is `Len`".

  Kind/bound coexistence in one bracket (`'T: Copy 'F: * -> *`) is designed against that
  single site. `Len` is reserved (`reject_reserved_name`, `parser.rs:244`), so kinds can
  never be shadowed by trait names; `*` and `->` must get the same protection **or** a
  grammar shape that cannot collide with a capability token. Rule: the kind expression is
  only recognised in the annotation position after `:`, and `*`/`->` are never valid
  capability names, so a bare `*` outside an annotation stays the existing "unknown
  capability" error (k3), not a silent accept.

**S1-10 (R2/R3).** `unify_poly_input` (`check/poly.rs:8008`) gains an `App` arm that
decomposes an application against a concrete generic type: matching `'F['T]` against
`Box[i64]` binds `'F := Type::CtorImage(box_generic_id)` (S1-12's representation) and
`'T := i64`. The head binding is recorded in the **existing** `ty` map — a `CtorImage`
is a grounded `Type`, so no third map — and the argument bindings go through the same
maps as today.

**S1-11 (R2/R3).** `apply_subst` (`check/poly.rs:8347`, the **write** side) gains an `App`
arm that grounds `PolyType::App → Type`: it resolves `head`'s binding from the `ty` map
(a `Type::CtorImage`, S1-10), substitutes the application's arguments through the
constructor's declared parameters, and **delegates to the existing `Generic` route** to
mint the concrete monomorph (`Box[i64]`) via the live instantiator. A bare `CtorImage`
reaching a value-type position outside `App`-head resolution is a located error —
**S1-15.g** ("constructor used as a type", both spans: the binding and the misuse), with
its own golden. Lowering stays lookup-only: `subst_polytype` (`ir/driver.rs:524`) never
mints (S1-13).

**S1-12 (R5 — RESOLVED; representation approved by the user 260830).** Constructor images
and the symbol hazard. The mangled callee symbol derives from `(callee, θ)` in vector
order (`check/poly.rs:8001`), so two call sites binding `'F` to **different** constructors
must mint different symbols, or S12's last-write-wins defect recurs one abstraction level
up. A bare constructor has no `Type` today (`Type::Struct` is a monomorph id,
`ast.rs:2483`), so the binding needs a representation before any ordering question arises.

  **Ruling:** the grounded `Type` enum gains **`Type::CtorImage(GenericId)`** — wrapping
  the constructor's generic id, not a monomorph. Unification binds
  `'F := Type::CtorImage(g)` **in the existing `ty` map**; no third map on `Subst`
  (`ast.rs:2270` stays two-map). The mangler never sees a `PolyType::App` — it folds
  `subst.ty` exactly as today, where the constructor binding appears as a grounded
  `CtorImage` whose rendered name is the constructor's name, so distinct constructor
  bindings mint distinct symbols. Grounding (S1-11) substitutes the application's
  arguments through the constructor's declared parameters and mints the concrete
  `Generic` monomorph **before** symbol derivation, so the symbol never encodes an
  unresolved application. `CtorImage` is ground-flowing but **not a concrete value
type**: any `Type` matcher that would treat it as one routes to S1-15.g. It flows into
  `struct_keys` (`ast.rs:609-610`) as an ordinary argument element, so `Wrap[Box i64]`
  and `Wrap[Opt2 i64]` get distinct instantiation keys.

  **Pre-existing limit (report-only):** the mangler renders `ty.name()` only
  (`ast.rs:2391-2395`), and the rendered name is documented non-injective across modules
  (`ast.rs:608-610`) — two *same-named* constructors in different modules bound to `'F`
  at two call sites would still collide. That hazard afflicts plain variable bindings
  today; `CtorImage` neither worsens nor fixes it.

  **Rejected alternative:** a third constructor-image map on `Subst` joined into the
  symbol derivation. Rejected on two grounds: it duplicates identity information the
  `ty` map now carries (the binding *is* a grounded `Type`), and a `Subst`-side map
  alone still leaves the use-site argument (`Wrap[Box i64]`'s `Box`) and the
  `struct_keys` instantiation key without a representation — the image must exist at the
  `Type` level regardless.

  **Required unit test:** two call sites binding `'F` to distinct constructors (e.g.
  `Box[i64]` and a second single-field generic) mint **distinct** mangled symbols; a
  duplicate call to the same constructor dedups to one symbol
  (`hkt_two_constructor_call_sites_mint_distinct_symbols`).

**S1-13 (machinery map).** `subst_polytype` (`ir/driver.rs:524`) gains an `App` **lookup**
arm consistent with the lookup-only contract: a hit resolves; a miss is an assertion
("check already minted this"). It never instantiates. Because S1-11/S1-12 ground to
`Generic` before symbol derivation, the IR side in practice resolves through the `Generic`
arm; the `App` arm exists so the exhaustive `match` compiles and asserts on the unreachable
miss (no `_ =>`, S1-16).

### Forced arms, audits, rendering, IR (Phases 2-4)

**S1-14 (R3).** `poly_type_str` (`check/poly.rs:9586`) renders `App` as `'F['T]` (head
name from `sig.ty_var_names`, args recursively). Unit-tested beside its siblings
(`poly_type_str_renders_a_generic_application`).

**S1-15 (R7).** Diagnostics family (draft shapes; single `error:` prefix, `(line, col)`,
parenthetical advice — house style per `header_bracket_unknown_kind_error`,
`var_kind_conflict_error` `parser.rs:1704`, `type_mismatch_error` `check.rs`):

  a. **star-kind variable applied like a constructor** (W3):
     `` error: type variable `'F` at line L, col C is applied like a type constructor but
     has kind `*` (bound bare at line L, col C2); only a higher-kinded variable can head
     `'F['T]` ``
  b. **arrow-kind variable used bare**:
     `` error: type variable `'F` at line L, col C is used as a plain type but has kind
     `* -> *` (from the application `'F['T]` at line L, col C2); a higher-kinded variable
     never appears bare ``
  c. **annotation conflicting with usage** (`'F: * -> *` used bare):
     `` error: type variable `'F` at line L, col C is used as a plain type but is annotated
     `* -> *` at line L, col C2 ``
  d. **application arity conflicting with inferred kind** (`'F['T] 'F['T 'U]`):
     `` error: `'F['T 'U]` at line L, col C applies `'F` to 2 arguments but its kind is
     `* -> *` (from `'F['T]` at line L, col C2) ``
  e. **header-field twin of (a)** — same shape, blamed at a `type:` field site.
  f. **use-site constructor argument of the wrong kind** (`Wrap[Nat i64]` where
     `'F: * -> *` and `Nat` is `*`) — one more message in the same family.
  g. **constructor used as a type** — a bare `Type::CtorImage` reaching a value-type
     position outside `App`-head resolution (S1-11/S1-12): both spans, the binding site
     and the misuse site. The binding span travels through `unify_poly_input`'s existing
     provenance passing (`check/poly.rs:8004-8006` — provenance is passed, never
     recorded on `Subst`, which stays span-free); when no binding span exists (an
     internally-created binding), the diagnostic degrades to the misuse span.
  h. **application unsatisfiable against an explicit annotation** (`'F: * -> Len -> *`
     applied as `'F['T]`, or any application the annotation's domains cannot accept):
     the annotation's span and the application's span.

  Every text carries **both** spans where a conflict has an origin (the binding mention and
  the offending mention). Optionally re-point k3d's mislabelled case (`: Len` var used as a
  type) at (a)'s real kind-mismatch diagnostic (nice-to-have, not gated).

**S1-16 (R3 forced-arm inventory; census corrected by the round-1 review).** Add `App`
arms to every `PolyType` matcher — no `_ =>` wildcard (S12 R3.3 discipline) — **and** an
explicit arm at every wildcard-armed site `App` can reach (the compiler cannot force
those). The census, re-derived against the tree:

- *compiler-forced* (exhaustive `match`es; rustc breaks the moment the variant lands):
  grounding pair `apply_subst` (S1-11), `subst_polytype` (S1-13); collectors
  `collect_positions` (`check/poly.rs:7349`), `poly_type_mentions_caller_var`
  (`:2759`), `poly_mentions_len_var` (`:2779`);
  predicates `match_impl_target_rec` (`:7165`), the escaping check in `poly_walk_arms`
  (`:3851`); the "not permitted on {what}" renderer `poly_op_on_variable_error`
  (`:8680` — needs an `App` wording decision); `poly_type_str` (S1-14); the
  `unreachable!` variant guards `parser.rs:384`, `:490`, `:2077`, `:2135`;
  `ir/driver.rs:651`; `check/declarations.rs:681`; `check/audits.rs:376`, `:432`,
  `:477`; `ast.rs:1964`. (`collect_concrete_positions` `:7459` matches on `Type`, not
  `PolyType` — no `App` arm; it lands in the Type-wave survey below. The former
  `repl.rs:317` entry was phantom: `src/repl.rs` was deleted in `5b8e68c`.)
- *wildcard-armed — compiler cannot force these; each gets an explicit `App` arm; silent
  pass-through is forbidden*: `generic_args_of` (`check/poly.rs:7655`, whose
  `_ => unreachable!` at `:7662` rustc will not break), its length twin
  `generic_len_args_of` (`:7667`),
  `quotation_parts` (`:7675`), `collect_paired_positions` (`:7693`),
  `substitute_member_var` (`:1084`, whose `other => other.clone()` would pass an `App`
  through unsubstituted), `poly_bind_construction_arg` (`:4546`),
  `substitute_generic_field` (`ast.rs:844`/`:910`), `substitute_generic_variant_field`
  (`ast.rs:2229`) — the last three carrying S1-8's instantiation semantics.
- *the Type wave*: `Type::CtorImage` (S1-12) adds a variant to the grounded `Type` enum
  (`ast.rs:2483`), so every `Type` matcher crate-wide — exhaustive **and** wildcard —
  must be surveyed (grep-driven, not compiler-driven: wildcards pass silently). Pin:
  a `CtorImage` is ground-flowing but not a concrete value type; a matcher that would
  treat it as one routes to S1-15.g.

  Each guard's `App` arm asserts or handles per that site's contract (an `unreachable!` guard
  that App genuinely cannot reach keeps asserting; a collector visits `head`/`args`).

**S1-17 (R8 scope fences).** Explicitly out of bounds for S1, each a located rejection kept
as-is or a deliberate non-change:

  i. **Cross-call App unification** — a poly *cross-call* with App slots
     (`poly_cross_match`, `check/poly.rs:2500`) stays a located "unsupported" rejection
     (precedent `poly_cross_call_unsupported_error`); S2 owns constructor-keyed dispatch.
  ii. **Body-quotation `'F['U]`** — `'F['U]` inside a *body* quotation literal stays
      unexercised (body-side variable-bearing-quotation rejections fire first). A *declared*
      quotation effect containing `'F['U]` is expected to ground for free once App has an
      `apply_subst` arm (`Quotation` rows are `PolyType`s substituted pointwise); flagged
      not designed (S2 open question).
  iii. **S3t explicit-instantiation** must not regress: `Nat['T]` after a non-generic type
       name in a signature keeps `instantiation_ty_var_error` (`parser.rs:1985`) and its
       message. Whether `Name[args]` on a non-generic type gains a dedicated "applied like a
       constructor" message is an optional upgrade riding the same lookahead — not required.
  iv. **Trait single-variable gate** (`multi_variable_trait_error`) is **not** lifted (S2's);
      S1's parse work merely lands before it (F6; k5/k5b).
  v. **Word-header spelling**: `inline` goes **before** the bracket
     (`: pass inline ['F 'T] ( … ) ;`); k4's first attempt failed on `inline` after the
     bracket.

## Considered and rejected

- **Constructor-image map on `Subst`** (S1-12) — rejected; the binding lives in the
  existing `ty` map as `Type::CtorImage`, which keeps `Subst` two-map, and a `Subst`-side
  map alone would still leave the use-site argument and the `struct_keys` key without a
  representation. See S1-12 for the full reasoning.
- **Reshaping `Generic`'s head to an enum** (over a dedicated `App` variant, S1-7) —
  rejected; would put a variable where `Generic` guarantees a registry index, threatening
  every `Generic` consumer. `App` isolates the new shape (R3).
- **Curried binary arrow kind** (over n-ary, S1-1) — rejected; Sooth application is not
  curried (type slots and length slots are separate arities), so a curried kind would
  misrepresent `array` as `* -> (Len -> *)` and complicate arity checking (R1).
- **Checker-side kind inference** (over parser-side collection, S1-3/S1-4) — rejected;
  mention spans exist only in the parser and every diagnostic needs them, and the X1
  Star/Len conflict already lives there (R2).

## Phased delivery plan

Each phase is independently verifiable: its goldens pass and its new stage code carries
unit coverage before it is done (CLAUDE.md). Green = `cargo fmt --check && cargo clippy --
-D warnings && cargo test`.

### Phase 1 — Kinds

Requirements S1-1..S1-5 and S1-9's grammar+collection. Promote `Kind` to `ast.rs` with
n-ary `Arrow`; kind-expression grammar beside bounds (S1-2, wired but not yet reached by
an application head until Phase 2); `PolyBuilder` kind-requirement collection retained
past `finish` with deferred validation — **usage-vs-usage at signature end**
(`parse_poly_effect`) and **annotation-vs-usage at attach time**
(`attach_bracket_bounds`, S1-15.c's firing point, since annotations attach after
`parse_poly_effect` returns); the per-decl header-field kind side table validated at decl
end (S1-5); publish `PolySig::ty_kinds` and the generic-decl/impl-target kind vectors.

- **Unit tests:** mention-vs-mention Star/Len conflict (single span, the existing
    `var_kind_conflict_error` `:1704`); annotation-vs-mention conflict at attach with
    **both spans** (S1-15.c — the only both-span pair reachable in Phase 1, since there
    is no application grammar yet; application-position conflicts arrive with Phase
    2/3); `parse_header_bracket` kind-expression annotations incl. `* -> Len -> *`;
    `parse_optional_bound_bracket` kind + bound coexistence in one bracket; header-field
    kind collection validated at decl end (the per-decl side table).
- **Golden (positive #5, the k8 flip):**
    `hkt_len_var_inferred_from_count_position_is_accepted` — k8's unannotated
    `array['T 'N]` fixture now builds with `Star`/`Len` inferred from the count slot.
    Pins the deliberate behavior change vs S6a: annotations become
    optional-but-available; "never appears in the effect" remains for never-mentioned
    vars.
- **Verifiable:** the S6a `: Len` control (k1) and generic-enum control (k2) stay green;
    a `'F: * -> *` annotation on an *otherwise-unused* header var no longer reports
    `unknown capability` (parses; usage-conflict enforcement lands in Phase 3). Golden #4's
    *annotation half* becomes reachable at Phase 3, not here.

### Phase 2 — Application parsing + compile-forcing

Requirements S1-6, S1-7, S1-8, S1-14 (the `App` arm), S1-16 (the compiler-forced arms),
and the parse-anchored members of S1-15 (a-f, h). The R4 `[`-router in `parse_poly_slot`'s
`'` arm; `PolyType::App` **and** the `RawTy` twin with its `raw_to_poly_type` arm;
header/field application continuation in `parse_generic_field_shape`; use-site constructor
type arguments in `parse_type_arguments` (→ `Type::CtorImage`).

**Green-with-a-new-variant rule.** Adding `PolyType::App` compiler-forces every exhaustive
`PolyType` matcher the moment it lands, so Phase 2 also lands the *mechanical* arms from
S1-16's census — parse-side guards get their real behavior; check/IR-side matchers that
cannot yet be correct get located "not yet supported in this slice stage" error arms
(never an `unreachable!` on an App-reachable path); `poly_type_str`'s `App` arm is real
(rendering is needed by any diagnostic that prints the type). The Type wave starts here
for `CtorImage` construction from use-site args, under the same rule. Any `App` reachable
through Phase-2 paths but not yet semantically served (e.g. instantiating
`Wrap[Box i64]`) yields a located unsupported error — a pinned intermediate state, never
a panic. Semantic completion (unification, grounding, wildcard-site arms, the symbol
ruling) is Phase 3.

- **Unit tests:** the R4 lookahead router **both orders** (application `'F['T]` vs
    quotation `[ 'T -- 'U ]` after a variable); a field `f 'F['T]` parses to `App`; a
    use-site constructor argument (`Wrap[Box i64]`) parses to `CtorImage`; the
    quotation-argument fence (`'F[[ i64 -- i64 ]]` is a parse error per S1-6's fence, the
    S1-15 family) and the
    empty application (`'F[]` is an error); the `RawTy`/`raw_to_poly_type` App fold.
- **Golden (non-regression #1):** `hkt_var_before_quotation_parameter_still_parses` —
    `: fmap['F 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) call ;` builds (both readings in one
    effect; today the F1 error at the first `[`). Pins the router.
- **Golden (non-regression #2):**
    `hkt_concrete_generic_effect_and_explicit_instantiation_unchanged` — a declared
    quotation parameter (`: q['T 'U] ( [ 'T -- 'U ] 'T -- 'U ) call ;`) and an S3t-style
    explicit instantiation still parse; `tests/phase7_slice6a.rs`'s Buffer goldens stay
    green.

### Phase 3 — Checking and grounding

Requirements S1-9's *consumers* (the grammar and collection landed in Phase 1; the
published kinds are consumed here), S1-10..S1-13, S1-15.g, S1-16 (wildcard-site arms and
the Type-wave survey; Phase-2 stubs swapped for real semantics), S1-17 (fences
enforced). `App` decomposition in `unify_poly_input` binding `Type::CtorImage`; `App`
grounding in `apply_subst` via the `Generic` mint route; the S1-12 symbol-distinctness
ruling; `substitute_generic_field`/`substitute_generic_variant_field`/
`poly_bind_construction_arg` real `App` arms with `struct_keys` carrying `CtorImage`;
`subst_polytype` App lookup arm; the check-side R7 diagnostic (S1-15.g; 15.h is
attach-anchored in Phase 2, its golden riding this phase's full suite).

- **Unit tests:** `unify_poly_input` App decomposition binds ctor + args; `apply_subst`
    App grounding via the `Generic` mint route; `subst_polytype` App lookup **and** miss →
    assertion (lookup-only contract); the **R5 two-constructor symbol dedup**
    (`hkt_two_constructor_call_sites_mint_distinct_symbols`, S1-12); a fence probe that
    `poly_cross_match` with an App slot still rejects (S1-17.i).
- **Golden (positive #3):** `hkt_var_kind_inferred_from_application_head_alone` —
    `: pass['F 'T] ( 'F['T] -- 'F['T] ) ;` with `: main ( -- ) ;`, no bare `'F` and no
    annotation, builds (inference criterion; today F1's quotation error).
- **Golden (positive #4):** `hkt_explicit_annotation_confirms_inferred_kind` — the same
    signature written `: pass['F: * -> * 'T] ( … ) ;` builds (annotation-fallback criterion;
    today k3's `unknown capability`).
- **Goldens (kind errors #1-#8):** one per S1-15 diagnostic, distinguishing-fragment
    `stderr.contains`:
  - `hkt_star_var_applied_like_constructor_is_located_error` — W3. (15.a)
  - `hkt_arrow_var_used_bare_is_located_error` — `: bad['F 'T] ( 'F['T] 'F -- ) ;`. (15.b)
  - `hkt_annotation_conflicting_with_usage_is_error` —
      `: bad['F: * -> * 'T] ( 'F 'T -- ) ;`. (15.c)
  - `hkt_application_arity_conflicts_with_inferred_kind_is_error` —
      `: bad['F 'T 'U] ( 'F['T] 'F['T 'U] -- ) ;`. (15.d)
  - `hkt_header_field_applies_star_var_is_located_error` —
      `type: Bad['F 'T] g 'F f 'F['T] ;`. (15.e)
  - `hkt_use_site_ctor_arg_of_wrong_kind_is_error` — `Wrap[Nat i64]` where `'F: * -> *`
      and `Nat` is `*`. (15.f)
  - `hkt_ctor_image_used_as_a_type_is_error` — a `CtorImage` in a plain type slot. (15.g)
  - `hkt_annotation_arity_unsatisfiable_by_application_is_error` —
      `: bad['F: * -> Len -> * 'T] ( 'F['T] -- ) ;`. (15.h)

### Phase 4 — IR + goldens (end-to-end)

The write→lookup grounding closed by Phase 3 means IR needs only its lookup arm (S1-13,
already landed structurally in Phase 3 for compilation); Phase 4 proves the two runtime
witnesses end-to-end. `poly_type_str`'s `App` arm landed in Phase 2 (rendering); its unit
test rides here.

- **Unit test:** `poly_type_str` renders `'F['T]` (S1-14).
- **Golden (positive #1):** `hkt_signature_application_passes_through_at_concrete_call_site`
    — W2, `build_and_run` exit 0, stdout `5`.
- **Golden (positive #2):** `hkt_struct_field_monomorphizes_to_the_applied_constructor` —
    W1, exit 0, stdout `5` (the `'F['T]` field lays out as the `Box[i64]` monomorph).

Golden file `tests/phase7b_slice1.rs`, style from `tests/phase7_slice6a.rs` (`single_file`, `build_and_run` / `build_error`). W1-W3 need only `import: intrinsics * ;`.

## Anchor status

Re-verified against HEAD `790b81c` this session; all load-bearing anchors accurate:

| Anchor | Brief | Verified |
| --- | --- | --- |
| `enum Kind` | `parser.rs:1346` | `parser.rs:1346` ✓ |
| `parse_poly_slot` | `:3497` (the `'` arm) | `fn` at `:3472`, `'` arm `:3497-3500` ✓ |
| `parse_poly_generic_application` | `:3673` | `:3673` ✓ |
| `parse_poly_ty_var` | `:3761` | `:3761` ✓ |
| `parse_optional_bound_bracket` | `:2443` | `:2443` ✓ |
| `attach_bracket_bounds` | `:2520` | `:2520` ✓ |
| `parse_header_bracket` | `:5666` | `:5666` ✓ |
| `header_bracket_unknown_kind_error` | `:1727` | `:1727` ✓ |
| `var_kind_conflict_error` | `:1704` | `:1704` ✓ |
| `validate_pending_quotation_rows` | `:1458` | `:1458` ✓ |
| `instantiation_ty_var_error` | `:1985` | `:1985` ✓ |
| `unify_poly_input` | `check/poly.rs:8008` | `:8008` ✓ |
| `apply_subst` | `:8347` | `:8347` ✓ |
| `poly_cross_match` | `:2500` | `:2500` ✓ |
| `collect_positions` / `collect_concrete_positions` / `generic_args_of` | `:7349`/`:7459`/`:7655` | all ✓ |
| `poly_type_str` | `:9586` | `:9586` ✓ |
| `subst_polytype` | `ir/driver.rs:524` | `:524` ✓ |
| `Subst` / `PolyType` / `PolySig` | `ast.rs:2270`/`2145`/`2244` | `:2270` / `enum` at `:2078` (`Generic` arm `:2140`) / `:2244` ✓ |

The Brief column reflects the corrected brief (the round-1 review found this table stale
against it); all rows verify against the tree. Inline anchors outside this table were
also corrected: `reject_reserved_name` `:244`, `parse_generic_field_shape` fn `:5766`,
`GenericStructDecl`/`GenericEnumDecl` `ast.rs:542`/`560`.

## Phases (JSON)

```json
[
  { "phase": 1, "focus": "Kinds: promote Kind to ast.rs with n-ary Arrow; kind-expression grammar beside bounds; PolyBuilder kind-requirement collection with deferred validation (usage-vs-usage at signature end, annotation-vs-usage at attach time) plus the per-decl header-field side table; publish PolySig.ty_kinds and generic-decl/impl-target kind vectors; k8-flip golden", "effort": "M", "difficulty": "M" },
  { "phase": 2, "focus": "Application parsing + compile-forcing: the R4 boolean top-depth-arrow router; PolyType::App and the RawTy twin with the raw_to_poly_type arm; header/field application continuation; use-site ctor args parse to Type::CtorImage; all compiler-forced PolyType arms land here (real parse-side behavior, located not-yet-supported stubs elsewhere, no reachable panic); poly_type_str App arm; parse-anchored R7 diagnostics", "effort": "M", "difficulty": "H" },
  { "phase": 3, "focus": "Checking and grounding: unify_poly_input App decomposition; CtorImage binding in the existing ty map; apply_subst App grounding via the Generic mint route; substitute_generic_field/variant_field/poly_bind_construction_arg App arms; struct_keys carry; subst_polytype App lookup arm; wildcard-site arms + Type-wave survey; S1-15.g; symbol-distinctness unit test; scope fences enforced", "effort": "L", "difficulty": "H" },
  { "phase": 4, "focus": "IR + goldens: W1/W2 end-to-end run goldens; full golden suite (5 positive, 8 kind-error, 2 non-regression) and unit coverage green", "effort": "M", "difficulty": "M" }
]
```
