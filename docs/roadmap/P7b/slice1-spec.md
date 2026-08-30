# P7b.S1 spec — kinds and type-level application

Technical specification for compiler slice P7b.S1. Scope input is the recon
[brief](./slice1-brief.md) (findings F1-F6, machinery map, rulings R1-R8, witnesses
W1-W3, golden list) and the verbatim [probe log](./slice1-probes.md); exit criteria are
from the [phase doc](../P7b-higher-kinded-types.md). All anchors below were re-verified
against HEAD `790b81c` this session (see [Anchor status](#anchor-status)).

Diagnostic texts here pin **shape**, not wording (located `(line, col)`, single `error:`
prefix, names the offending position and the kind's origin, parenthetical advice). Exact
strings freeze when the goldens are written, per the S12 precedent (brief R7).

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

- **There is no type-application grammar anywhere** (F1; k4/k4b/k5/k7). In an effect, `[`
  after any type reads as a quotation-effect opener (`require_top_depth_arrow`); in `type:`
  field positions it is `expected a word, found LBracket`. Even a concrete head `i64['T]`
  dies the same way. So S1's parse production is prerequisite to the whole trait surface
  (F6).
- **The kind-annotation position is the bound/capability position** (F2; k3/k3b). `: Len`
  works (S6a); `: *` is rejected as `unknown capability`. Arrow-kind syntax must be carved
  out beside bounds at that one site.
- **No kind inference exists** (F3; k8/k1). An unannotated length var in `array['T 'N]` is
  rejected; `: Len` is mandatory. Inference-from-usage is new machinery.
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
    the argument slots (each `Star` in S1's set; `Len` where a count slot appears);
- application argument → `Star`;
- count position → `Len`.

  First mention **binds**; later mentions **check**. This is why collection is parser-side:
  mention spans, which every new diagnostic needs, exist only there, and the X1 Star/Len
  conflict is already parser-side (no second kind mechanism, R2/phase-doc rule).

**S1-4 (R2).** Kind **unification** runs as **deferred validation at signature end**,
following the `validate_pending_quotation_rows` precedent (`parser.rs:1458`, invoked from
`parse_poly_effect`). Conflicting requirements for one variable are located errors carrying
**both** spans (the binding mention and the conflicting mention). This is where S1-16's
arity-conflict and annotation-conflict diagnostics fire.

**S1-5 (R2).** Publish the resolved kinds. `PolySig` (`ast.rs:2244`) gains
`ty_kinds: Vec<Kind>` parallel to `ty_var_names`. `GenericStructDecl` and
`GenericEnumDecl` (`ast.rs:547`/`562`) and `ImplTarget` gain their own parallel kind
vectors so the checker can consume kinds at `unify_poly_input`, `apply_subst`, and the
annotation path (`attach_bracket_bounds`, `parser.rs:2520`). No default-to-`Star` shortcut
that would drop an `Arrow` on the floor: every published vector is length-matched to its
name table.

### Application parsing (Phase 2)

**S1-6 (R4).** The `[`-router. In `parse_poly_slot`'s `'` arm (`parser.rs:3472`, the arm at
~`:3517` that calls `parse_poly_ty_var` `:3761`), when a `[` follows the type variable,
run the `require_top_depth_arrow` bracket scan (`parser.rs:4509`) as a **router**:

- a top-depth `--` present → **quotation slot** (today's behaviour, byte-for-byte
    unchanged — the LBracket re-enters the slot loop as it does now);
- absent → **type application**.

  The route is total: an application's arguments are type expressions and never contain a
  top-depth `--` (a quotation *argument* `'F[[ i64 -- i64 ]]` keeps its `--` at depth 2).
  Bare-`[` positions that start a slot with no preceding type are untouched — P7.S6 R4's
  quotation-unconditional rule survives (F1; k4). The interception template is the existing
  `parse_poly_generic_application` (`:3673`), which today explicitly skips `'`-led heads;
  S1 removes exactly that gate for the variable head.

**S1-7 (R3).** Add `PolyType::App { head: u32 /* index into ty_var_names */, args:
Vec<PolyType> }` to `ast.rs:2078`. The dedicated variant is chosen over reshaping
`Generic`'s head to an enum: it keeps `Generic`'s registry-index invariant intact and is
the lower-risk shape (R3). `head` names a variable; `args` are the applied type
expressions. (Length arguments in an application are out of S1's golden scope; the variant
carries only type args, matching the Star-domain golden set. A `Len`-domain application is a
parse-representable future extension, not exercised here.)

**S1-8 (R4/F1).** Add the application production to the **`type:` header/field** grammar
too (k4b): `parse_generic_field_shape` (`parser.rs:5793`) — its variable arm has no bracket
continuation, unlike the sibling `&`-glued and `^`-glued application arms. A field
`f 'F['T]` must parse to a field whose type is `PolyType::App`. Use-site constructor type
arguments (a bare constructor `Box` supplied as a type argument in `parse_type_arguments`'
argument loop, W1's `Wrap[Box i64]`) parse as a constructor image, gated on the header
variable's kind (S1-15's use-site diagnostic covers the mismatched case).

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
  single site. `Len` is reserved (`reject_reserved_name`, `parser.rs:278`), so kinds can
  never be shadowed by trait names; `*` and `->` must get the same protection **or** a
  grammar shape that cannot collide with a capability token. Rule: the kind expression is
  only recognised in the annotation position after `:`, and `*`/`->` are never valid
  capability names, so a bare `*` outside an annotation stays the existing "unknown
  capability" error (k3), not a silent accept.

**S1-10 (R2/R3).** `unify_poly_input` (`check/poly.rs:8008`) gains an `App` arm that
decomposes an application against a concrete generic type: matching `'F['T]` against
`Box[i64]` binds `'F := Box` (the constructor) and `'T := i64`. The arm records the
constructor binding through S1-12's ruling and the argument bindings through the existing
`ty`/`len` maps.

**S1-11 (R2/R3).** `apply_subst` (`check/poly.rs:8347`, the **write** side) gains an `App`
arm that grounds `PolyType::App → Type` by resolving `head`'s constructor binding and the
argument substitutions, then **delegating to the existing `Generic` route** to mint the
concrete monomorph via the live instantiator. Lowering stays lookup-only: `subst_polytype`
(`ir/driver.rs:524`) never mints (S1-13).

**S1-12 (R5 — RESOLVED).** Constructor images and the symbol hazard. The mangled callee
symbol derives from `(callee, θ)` in vector order (`check/poly.rs:8001`), so two call sites
binding `'F` to **different** constructors must mint different symbols, or S12's
last-write-wins defect recurs one abstraction level up.

  **Ruling:** ground the `App` to its concrete `Generic` monomorph **before** mangled-symbol
  derivation, so the mangler **never sees an `App`**. The constructor choice is already
  baked into the grounded `Generic` (its `idx`/`module`/`args`), which the mangler folds
  into the symbol as it does for any monomorph today. No third map on `Subst`.

  **Rejected alternative:** a constructor-image map on `Subst` (`ast.rs:2270`, third map
  beside `ty`/`len`) joined into the symbol derivation. Rejected because it duplicates the
  monomorph-identity information the grounded `Generic` already carries and re-opens the
  vector-order last-write-wins hazard at the symbol boundary — the exact class S12 closed.
  Grounding-first keeps `Subst` two-map and the mangler App-free.

  **Required unit test:** two call sites binding `'F` to distinct constructors (e.g.
  `Box[i64]` and a second single-field generic) mint **distinct** mangled symbols; a
  duplicate call to the same constructor dedups to one symbol
  (`hkt_two_constructor_call_sites_mint_distinct_symbols`).

**S1-13 (machinery map).** `subst_polytype` (`ir/driver.rs:524`) gains an `App` **lookup**
arm consistent with the lookup-only contract: a hit resolves; a miss is an assertion
("check already minted this"). It never instantiates. Because S1-11/S1-12 ground to
`Generic` before symbol derivation, the IR side in practice resolves through the `Generic`
arm; the `App` arm exists so the exhaustive `match` compiles and asserts on the unreachable
miss (no `_ =>`, S1-17).

### Forced arms, audits, rendering, IR (Phase 3-4)

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

  Every text carries **both** spans where a conflict has an origin (the binding mention and
  the offending mention). Optionally re-point k3d's mislabelled case (`: Len` var used as a
  type) at (a)'s real kind-mismatch diagnostic (nice-to-have, not gated).

**S1-16 (R3 forced-arm inventory).** Add `App` arms to every `PolyType` matcher — no
`_ =>` wildcard (S12 R3.3 discipline). The forced set (~22 sites), from the brief:

- grounding pair: `apply_subst` (S1-11), `subst_polytype` (S1-13);
- unification collectors: `collect_positions` (`check/poly.rs:7349`),
    `collect_concrete_positions` (`:7459`), `generic_args_of` (`:7655`);
- copy/escape predicates over `PolyType`;
- `poly_type_str` (S1-14);
- the `unreachable!` variant guards: `parser.rs:384`, `:490`, `:2077`, `:2135`;
    `ir/driver.rs:651`; `check/declarations.rs:681`; `check/audits.rs:376`, `:432`, `:477`;
    `ast.rs:1964`; `repl.rs:317`.

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

- **Constructor-image map on `Subst`** (S1-12) — rejected; grounding-first keeps `Subst`
  two-map and the mangler App-free. See S1-12 for the full reasoning.
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

Requirements S1-1..S1-5. Promote `Kind` to `ast.rs` with n-ary `Arrow`; kind-expression
grammar beside bounds (S1-2, wired but not yet reached by an application head until Phase 2);
`PolyBuilder` kind-requirement collection retained past `finish` with deferred validation at
signature end; publish `PolySig::ty_kinds` and the generic-decl/impl-target kind vectors.

- **Unit tests:** `PolyBuilder` kind-requirement conflict (first-binds/later-checks, both
    spans); `parse_header_bracket` kind-expression annotations incl. `* -> Len -> *`;
    `parse_optional_bound_bracket` kind + bound coexistence in one bracket.
- **Verifiable:** the S6a `: Len` control (k1) and generic-enum control (k2) stay green;
    a `'F: * -> *` annotation on an *otherwise-unused* header var no longer reports
    `unknown capability` (parses; usage-conflict enforcement lands in Phase 3). Golden #4's
    *annotation half* becomes reachable at Phase 3, not here.

### Phase 2 — Application parsing

Requirements S1-6, S1-7, S1-8. The R4 `[`-router in `parse_poly_slot`'s `'` arm;
`PolyType::App`; header/field application continuation in `parse_generic_field_shape`;
use-site constructor type arguments in `parse_type_arguments`.

- **Unit tests:** the R4 lookahead router **both orders** (application `'F['T]` vs
    quotation `[ 'T -- 'U ]` after a variable); a field `f 'F['T]` parses to `App`; a
    use-site constructor argument (`Wrap[Box i64]`) parses.
- **Golden (non-regression #1):** `hkt_var_before_quotation_parameter_still_parses` —
    `: fmap['F 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) call ;` builds (both readings in one
    effect; today the F1 error at the first `[`). Pins the router.
- **Golden (non-regression #2):**
    `hkt_concrete_generic_effect_and_explicit_instantiation_unchanged` — a declared
    quotation parameter (`: q['T 'U] ( [ 'T -- 'U ] 'T -- 'U ) call ;`) and an S3t-style
    explicit instantiation still parse; `tests/phase7_slice6a.rs`'s Buffer goldens stay
    green.

### Phase 3 — Checking and grounding

Requirements S1-9..S1-13, S1-15, S1-16 (forced arms), S1-17 (fences enforced). `App`
decomposition in `unify_poly_input`; `App` grounding in `apply_subst` via the `Generic`
mint route; the S1-12 grounding-before-symbol ruling; `subst_polytype` App lookup arm; the
R7 diagnostic family; the ~22 forced arms across audits/collectors/guards.

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
- **Goldens (kind errors #1-#5):** one per R7 diagnostic, distinguishing-fragment
    `stderr.contains`:
  - `hkt_star_var_applied_like_constructor_is_located_error` — W3.
  - `hkt_arrow_var_used_bare_is_located_error` — `: bad['F 'T] ( 'F['T] 'F -- ) ;`.
  - `hkt_annotation_conflicting_with_usage_is_error` —
      `: bad['F: * -> * 'T] ( 'F 'T -- ) ;`.
  - `hkt_application_arity_conflicts_with_inferred_kind_is_error` —
      `: bad['F 'T 'U] ( 'F['T] 'F['T 'U] -- ) ;`.
  - `hkt_header_field_applies_star_var_is_located_error` —
      `type: Bad['F 'T] g 'F f 'F['T] ;`.

### Phase 4 — IR + goldens (end-to-end)

The write→lookup grounding closed by Phase 3 means IR needs only its lookup arm (S1-13,
already landed structurally in Phase 3 for compilation); Phase 4 proves the two runtime
witnesses end-to-end and closes `poly_type_str` rendering (S1-14).

- **Unit test:** `poly_type_str` renders `'F['T]` (S1-14).
- **Golden (positive #1):** `hkt_signature_application_passes_through_at_concrete_call_site`
    — W2, `build_and_run` exit 0, stdout `5`.
- **Golden (positive #2):** `hkt_struct_field_monomorphizes_to_the_applied_constructor` —
    W1, exit 0, stdout `5` (the `'F['T]` field lays out as the `Box[i64]` monomorph).

Golden file `tests/phase7b_slice1.rs`, style from `tests/phase7_slice6a.rs` (`single_file`
- `build_and_run` / `build_error`). W1-W3 need only `import: intrinsics * ;`.

## Anchor status

Re-verified against HEAD `790b81c` this session; all load-bearing anchors accurate:

| Anchor | Brief | Verified |
| --- | --- | --- |
| `enum Kind` | `parser.rs:1346` | `parser.rs:1346` ✓ |
| `parse_poly_slot` | ~`:3497` (the `'` arm) | `fn` at `:3472`, `'` arm ~`:3517` ✓ |
| `parse_poly_generic_application` | `:3673` | `:3673` ✓ |
| `parse_poly_ty_var` | `:3761` | `:3761` ✓ |
| `parse_optional_bound_bracket` | `:2443` | `:2443` ✓ |
| `attach_bracket_bounds` | `:2523` | `:2520` (off by 3) |
| `parse_header_bracket` | `:5666` | `:5666` ✓ |
| `header_bracket_unknown_kind_error` | `:1727` | `:1727` ✓ |
| `var_kind_conflict_error` | `:1704` | `:1704` ✓ |
| `validate_pending_quotation_rows` | `:1437` | `:1458` (off by 21) |
| `instantiation_ty_var_error` | `:1979` | `:1985` (off by 6) |
| `unify_poly_input` | `check/poly.rs:8008` | `:8008` ✓ |
| `apply_subst` | `:8347` | `:8347` ✓ |
| `poly_cross_match` | `:2528` | `:2500` (off by 28) |
| `collect_positions` / `collect_concrete_positions` / `generic_args_of` | `:7349`/`:7459`/`:7655` | all ✓ |
| `poly_type_str` | `:9640` | `:9586` (off by 54) |
| `subst_polytype` | `ir/driver.rs:524` | `:524` ✓ |
| `Subst` / `PolyType` / `PolySig` | `ast.rs:2266`/`2145`/`2244` | `:2270`/`:2078`/`:2244` (Subst +4; PolyType is the `enum` at `:2078`, `Generic` arm `:2140`) |

The drifts are cosmetic (≤54 lines, same function bodies); no requirement changes.

## Phases (JSON)

```json
[
  { "phase": 1, "focus": "Kinds: promote Kind to ast.rs with n-ary Arrow; kind-expression grammar beside bounds; PolyBuilder kind-requirement collection with deferred validation; publish PolySig.ty_kinds and generic-decl/impl-target kind vectors", "effort": "M", "difficulty": "M" },
  { "phase": 2, "focus": "Application parsing: the R4 top-depth-arrow router in parse_poly_slot's ' arm; PolyType::App; header/field application continuation; use-site constructor type arguments", "effort": "M", "difficulty": "H" },
  { "phase": 3, "focus": "Checking and grounding: unify_poly_input App decomposition; apply_subst App grounding via the Generic mint route; ground-before-symbol ruling with two-constructor dedup; subst_polytype App lookup arm; R7 diagnostic family; ~22 forced arms; scope fences enforced", "effort": "L", "difficulty": "H" },
  { "phase": 4, "focus": "IR + goldens: poly_type_str App rendering; W1/W2 end-to-end run goldens; full golden suite (4 positive, 5 kind-error, 2 non-regression) and unit coverage green", "effort": "M", "difficulty": "M" }
]
```
