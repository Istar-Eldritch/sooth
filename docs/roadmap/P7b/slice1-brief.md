# P7b.S1 brief — kinds and type-level application (recon round 260830)

Scope input for the S1 spec. Produced by three recon workers against the clean tree
(worktree `incas`, HEAD `790b81c`): a read-only extension-point map, 15 live compile/run
probes under `/tmp/p7bs1-probes/` (verbatim log:
[slice1-probes.md](./slice1-probes.md)), and a witness/golden paper design with every
today-error verified against a warm build. Repo untouched throughout; probe fixtures are
disposable. Diagnostic texts below are drafts — they freeze at implementation, pinned by
the goldens (S12 precedent).

S1's exit criteria (from the [phase doc](../P7b-higher-kinded-types.md)): a type variable
may carry a higher kind; `'F['T]` type-checks and monomorphizes to a concrete type;
kind-incorrect application is a located error; a signature may mention `'F['T]` where
`'F` has kind `* -> *`.

## What the probes established

| # | Finding | Probes |
| --- | --- | --- |
| F1 | There is **no type-application grammar anywhere**. In an effect, `[` after a type variable is a quotation-effect opener ("must be written in full as `[ inputs -- outputs ]`"); in `type:` field positions, `expected a word, found LBracket`. Even `i64['T]` (concrete head) dies the same way. | k4, k4b, k5, k5b, k7 |
| F2 | The kind-annotation position **is the bound/capability position**: `: Len` works (S6a), `: *` → `unknown capability`*``. Any Arrow kind syntax must be carved out of `parse_capabilities`' site, shared with bounds. | k3, k3b |
| F3 | **No kind inference exists.** An unannotated `'N` appearing in `array['T 'N]` is rejected ("never appears in the effect") — `: Len` is mandatory. S1's inference-from-usage is new machinery, not an extension. | k8, k1 |
| F4 | **No dedicated kind-mismatch diagnostic.** A `: Len` variable used as a value type reports "never appears in the effect" — a mislabel. The only kind error today is the Star/Len dual-use conflict. | k3d, k3c |
| F5 | **Applied-target impls and `(TraitId, concrete Type)` dispatch already work**: `trait: Functor['F] : size ( 'F -- i64 ) ;; impl: Functor for Option[i64] ... ;` with a `Functor`-bounded word dispatching at `Option[i64]` compiles and runs. S2's gap is narrower than the phase doc implies: it is the *constructor-abstract* target (`for Option`), currently behind the generic-arity gate. | k6b, k6 |
| F6 | The trait member effect-parse fires **before** the single-variable gate, so S1's parse work is prerequisite to any HKT trait regardless; the single-var gate itself is S2's to lift. | k5, k5b |

Positive controls (smoke, S6a `: Len` end-to-end, generic enum + `Option[i64]` + eliminator)
all pass, so every rejection above is attributable to the missing HKT surface. Note: the
`error: error:` doubling observed in the S7c probe round is fixed on this tree —
diagnostics print with a single prefix; the probe log is the baseline for new messages.

## Machinery map (verified anchors)

Structural facts every S1 change must respect:

- **`enum Kind { Star, Len }` is parser-private** (`src/parser.rs:1346`) and **dies in the
  parser**: kinds never reach the checker/IR. A variable's kind is encoded structurally —
  which name table its id indexes (`PolySig::ty_var_names` vs `len_var_names`,
  `src/ast.rs:2244`). `parse_trait_decl` discards the parsed kind outright
  (`src/parser.rs:2716-2719`). An `Arrow`-kinded variable is still a *type* variable, so
  this encoding breaks: kinds must start travelling (R1/R2).
- **No application variant exists in `Type` or `PolyType`.** `Option[i64]` resolves
  eagerly at parse time (`resolve_type_or_apply` `src/parser.rs:5469` →
  `parse_type_arguments` `:5606` → `GenericTypes::instantiate_*`, monomorph keyed on
  `(decl idx, module, Vec<Type>, Vec<Len>)` in `struct_keys`/`enum_keys`,
  `src/ast.rs:577`). In `PolyType`, `Generic { is_enum, idx, module, args, len_args,
  name }` (`src/ast.rs:2145`) has a *registry-index* head — never a variable; `Var(u32)`
  is a bare leaf. Neither can carry `'F['T]`; a new variant is required (R3).
- **The parse hook is `parse_poly_slot`'s `'` arm** (`src/parser.rs:3497`) →
  `parse_poly_ty_var` (`:3761`), which consumes only the `'F` token and never peeks `[`;
  the `[` then re-enters the slot loop's LBracket arm and dies in
  `require_top_depth_arrow` (`:4509`). The interception template to copy is
  `parse_poly_generic_application` (`:3673`) — which today explicitly *skips*
  `'`-led words, exactly the gate S1 removes. Secondary site: generic struct fields
  (`parse_generic_field_shape` `:5793`, whose variable arm has no bracket continuation;
  the sibling `&`-glued and `^`-glued application arms do).
- **Grounding is a two-sided contract.** Check-side `apply_subst`
  (`src/check/poly.rs:8347`) is the write side: it grounds `PolyType → Type` through a
  `Subst` (`ty_of`/`len_of` maps, `src/ast.rs:2270`), minting monomorphs via the live
  instantiator. IR-side `subst_polytype` (`src/ir/driver.rs:524`) is lookup-only — every
  miss is an assertion, "check already minted this". S1's App grounding extends the write
  side and delegates to the existing `Generic` route, so lowering stays lookup-only.
- **Anchor corrections** (vs. the S12 roadmap entry / older notes):
  `eliminator_registry` is at `src/check/declarations.rs:1934`;
  `check_poly_combinator_standalone` at `src/check/poly.rs:494`;
  `lower_materialized` at `src/ir/func_builder/mod.rs:1144` (1061 is inside
  `lower_word_parts`).

## Design rulings the spec must make

**R1 — Promote `Kind` to `ast.rs`; make the Arrow n-ary.** `Kind` gains
`Arrow { domains: Vec<Kind>, result: Kind }`, not a curried binary arrow: Sooth's type
application is not curried — every application site splits type slots from length slots
(`parse_type_arguments` takes `(ty_arity, len_arity)`; `PolyType::Generic` carries
parallel `args`/`len_args`), so `array` is honestly `* -> Len -> *`. The annotation
grammar accepts Len domains from day one; S1's goldens cover Star domains only.

**R2 — Kinds travel, and inference is parser-side collection with deferred validation.**
`PolyBuilder` already interns every variable with a kind (`intern_ty_var`/`intern_len_var`,
`src/parser.rs:1470-1498`) and fires the Star/Len conflict at intern time — but drops the
map at `finish` (`:1500`). Keep collection there: each mention records a kind
*requirement* with its span (bare type slot → `Star`; application head →
`Arrow(domains, result)`; application argument → `Star`; count position → `Len`), and
kind unification runs as deferred validation at signature end — the established precedent
is `validate_pending_quotation_rows` (`src/parser.rs:1458`, invoked from
`parse_poly_effect` `:3413`). First mention binds; later mentions check; conflicts are
located errors carrying both spans. Parser-side is right because mention spans — which
every new diagnostic needs — exist only there, and X1 kind conflicts are already
parser-side. But the resolved kinds must then be **published**: `PolySig` gains
`ty_kinds: Vec<Kind>` parallel to `ty_var_names`, and `GenericStructDecl`/
`GenericEnumDecl`/`ImplTarget` their own — the checker consumes kinds at
`unify_poly_input` (`src/check/poly.rs:8008`), `apply_subst`, and the annotation path
(`attach_bracket_bounds` `src/parser.rs:2520`). No second kind mechanism (phase-doc rule).

**R3 — New `PolyType` variant `App { head: u32 /* var */, args: Vec<PolyType> }`** (or the
`Generic`-head-enum reshaping; the dedicated variant keeps `Generic`'s registry-index
invariant intact and is the lower-risk shape). With it comes the S12 R3.3 forced-arm
inventory (~22 sites): the grounding pair (`apply_subst` / `subst_polytype`), the
unification collectors (`collect_positions` `:7349`, `collect_concrete_positions` `:7459`,
`generic_args_of` `:7655`), copy/escape predicates, `poly_type_str` (`:9586`), and the ~9
`unreachable!` variant guards (`src/parser.rs:384/490/2077/2135`, `src/ir/driver.rs:651`,
`src/check/declarations.rs:681`, `src/check/audits.rs:376/432/477`, `src/ast.rs:1964`,
`src/repl.rs:317`). No `_ =>` arms (S12's R3.3 discipline).

**R4 — The `[` router: top-depth `--` lookahead decides quotation vs application.** In
`parse_poly_slot`'s `'` arm, on a following `[`, run the `require_top_depth_arrow` bracket
scan (`src/parser.rs:4509`) as a *router*: top-depth `--` present → quotation slot
(today's behavior, byte-unchanged); absent → type application. The route is total because
an application's arguments are type expressions and never contain a top-depth `--` (a
quotation *argument* `'F[[ i64 -- i64 ]]` keeps its `--` at depth 2). Bare-`[` type
positions that start a slot (no preceding type) are untouched — P7.S6 R4's
quotation-unconditional rule survives. A golden pins the Functor-map shape
`'F['T] [ 'T -- 'U ] -- 'F['U]` where both readings coexist in one effect.

**R5 — Constructor images in `Subst` vs ground-before-symbol-derivation.** `Subst`
(`src/ast.rs:2270`) holds only `ty` and `len` maps; binding `'F` to a constructor needs a
third map or an early grounding. **Symbol hazard**: the mangled callee symbol derives from
`(callee, θ)` in vector order (`src/check/poly.rs:8001`), so two call sites binding `'F`
to *different* constructors must mint different symbols — S12's last-write-wins defect
class one abstraction level up. Either the ctor image joins the symbol derivation, or the
App grounds to the concrete `Generic` monomorph *before* symbol derivation so the mangler
never sees an App. Must be ruled explicitly and unit-tested with a two-constructor dedup
case.

**R6 — Annotation grammar and the glued-colon hazard.** `:` is not a lexer delimiter;
`'T:Len` fully-glued lexes as one word (S7c probe P2 surprise), and k3b shows
`'F:*->*`-style glue reads as one capability token. The kind expression parser (`*`,
`Len`, `* -> *`, `* -> Len -> *`) extends `parse_optional_bound_bracket`
(`src/parser.rs:2443`, where `is_len_kind` already special-cases `Len` beside
capabilities) and `parse_header_bracket` (`src/parser.rs:5666`, where
`header_bracket_unknown_kind_error` `:1727` says "the only spellable kind is `Len`").
Kind/bound coexistence in one bracket (`'T: Copy 'F: * -> *`) must be designed against
that single site; `Len` is reserved (`reject_reserved_name` `:278`), so kinds can never
be shadowed by trait names — `*` and `->` need the same protection or a grammar shape
that cannot collide.

**R7 — Diagnostics family** (drafts; single `error:` prefix, `(line, col)`, parenthetical
advice — house style per `header_bracket_unknown_kind_error`, `var_kind_conflict_error`
`:1704`, `type_mismatch_error` `src/check.rs:1457`):

- star-kind variable applied like a constructor (W3):
  ``error: type variable `'F` at line 5, col 21 is applied like a type constructor but has kind `*` (bound bare at line 5, col 17); only a higher-kinded variable can head `'F['T]` ``
- arrow-kind variable used bare:
  ``error: type variable `'F` at line 5, col 26 is used as a plain type but has kind `* -> *` (from the application `'F['T]` at line 5, col 17); a higher-kinded variable never appears bare``
- annotation conflicting with usage (`'F: * -> *` used bare):
  ``error: type variable `'F` at line 5, col 24 is used as a plain type but is annotated `* -> *` at line 5, col 9``
- application arity conflicting with inferred kind (`'F['T] 'F['T 'U]`):
  ``error: `'F['T 'U]` at line 5, col 24 applies `'F` to 2 arguments but its kind is `* -> *` (from `'F['T]` at line 5, col 17)``
- header-field twin of the first
- use-site constructor argument of the wrong kind (`Wrap[Nat i64]` where `'F: * -> *` and
  `Nat` is `*`) — one more message in the same family.

Optionally re-point k3d's mislabeled case (`: Len` var used as a type) at the real
kind-mismatch diagnostic.

**R8 — Scope fences.** (i) A poly *cross-call* with App slots
(`poly_cross_match`, `src/check/poly.rs:2500`) stays a located "unsupported" rejection
(precedent: `poly_cross_call_unsupported_error`) — S2's constructor-keyed dispatch owns
it. (ii) `'F['U]` inside a *body* quotation literal stays unexercised (body-side
variable-bearing-quotation rejections fire first); a *declared* quotation effect
containing `'F['U]` is expected to ground for free once App has an `apply_subst` arm
(`Quotation` rows are `PolyType`s, substituted pointwise) — S2's open question, flagged
not designed. (iii) S3t's explicit-instantiation spelling must not regress: `Nat['T]`
after a non-generic type name in a signature is today's `instantiation_ty_var_error`
(`src/parser.rs:1985`) and keeps its message; whether `Name[args]` on a non-generic type
gains a dedicated "applied like a constructor" message is an optional upgrade riding the
same lookahead. (iv) Word-header spelling: `inline` goes *before* the bracket
(`: pass inline ['F 'T] ( ... ) ;` — k4's first attempt failed on this).

## Witness programs

W1 — generic struct whose field is `'F['T]` (exit: application monomorphizes; the field
must lay out as the `Box[i64]` monomorph):

```sth
import: intrinsics * ;

type: Box['T] v 'T ;
type: Wrap['F 'T] f 'F['T] t 'T ;

: mk ( i64 -- Wrap[Box i64] ) Box Wrap ;
: main ( -- ) 5 mk Wrap> drop Box> . ;
```

(`'T` needs a field of its own — a header variable in no field is a phantom error today.
Bare constructor `Box` as a use-site type argument is new grammar in
`parse_type_arguments`' argument loop, gated on the header variable's kind — R7's
use-site diagnostic covers the mismatched case.)

W2 — generic word with `'F['T]` in input and output positions (exit: signature
mention + call-site unification binds `'F := Box`, `'T := i64`; the Star-kind twin
builds and runs today, so the application is the only blocker):

```sth
import: intrinsics * ;

type: Box['T] v 'T ;

: pass['F 'T] ( 'F['T] -- 'F['T] ) ;
: mk ( i64 -- Box[i64] ) Box ;
: main ( -- ) 5 mk pass Box> . ;
```

W3 — kind-error fixture (kind-`*` variable applied like a constructor; today it dies
with W2's wrong-cause quotation error, under S1 it is R7's first diagnostic):

```sth
import: intrinsics * ;

type: Box['T] v 'T ;

: bad['F 'T] ( 'F 'F['T] -- 'F ) ;
: main ( -- ) ;
```

## Golden list — `tests/phase7b_slice1.rs`

Style to copy: `tests/phase7_slice6a.rs` (`single_file` + `build_and_run` asserting
exit 0 and exact stdout; `build_error` asserting `stderr.contains(...)` on distinguishing
fragments). W1-W3 need only `intrinsics`.

Positive (the four exit criteria):

1. `hkt_signature_application_passes_through_at_concrete_call_site` — W2, stdout `5`.
2. `hkt_struct_field_monomorphizes_to_the_applied_constructor` — W1, stdout `5`.
3. `hkt_var_kind_inferred_from_application_head_alone` — W2's signature with no bare
   `'F` mention and no annotation, uncalled: `: pass['F 'T] ( 'F['T] -- 'F['T] ) ;`
   with `: main ( -- ) ;` builds (inference criterion; today the F1 quotation error).
4. `hkt_explicit_annotation_confirms_inferred_kind` — same signature written
   `: pass['F: * -> * 'T] ( 'F['T] -- 'F['T] ) ;` builds (annotation-fallback criterion;
   today k3's `unknown capability` error).

Kind errors (one per R7 diagnostic, draft texts as in R7):

1. `hkt_star_var_applied_like_constructor_is_located_error` — W3.
2. `hkt_arrow_var_used_bare_is_located_error` — `: bad['F 'T] ( 'F['T] 'F -- ) ;`.
3. `hkt_annotation_conflicting_with_usage_is_error` — `: bad['F: * -> * 'T] ( 'F 'T -- ) ;`.
4. `hkt_application_arity_conflicts_with_inferred_kind_is_error` —
   `: bad['F 'T 'U] ( 'F['T] 'F['T 'U] -- ) ;`.
5. `hkt_header_field_applies_star_var_is_located_error` —
   `type: Bad['F 'T] g 'F f 'F['T] ;`.

Non-regression:

1. `hkt_var_before_quotation_parameter_still_parses` — the exact shape S2 needs:
    `: fmap['F 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) call ;` with `: main ( -- ) ;`
    builds — an application head and a quotation parameter in one effect; pins R4's
    router (today the F1 error at the first `[`).
2. `hkt_concrete_generic_effect_and_explicit_instantiation_unchanged` — a declared
    quotation parameter (`: q['T 'U] ( [ 'T -- 'U ] 'T -- 'U ) call ;`) and an
    S3t-style explicit instantiation still parse; `tests/phase7_slice6a.rs`'s Buffer
    goldens stay green.

Unit tests beside the stages (CLAUDE.md): the R4 lookahead router (both orders,
application vs quotation), `PolyBuilder` kind-requirement conflict, `parse_header_bracket`
kind-expression annotations (incl. `* -> Len -> *`), `unify_poly_input` App decomposition
binding ctor + args, `apply_subst` App grounding via the `Generic` mint route,
`subst_polytype` App lookup (and miss → assertion per the lookup-only contract),
`poly_type_str` renders `'F['T]`, and the R5 two-constructor symbol dedup.

## What S1 leaves for S2 (updated by k6b)

The phase doc's S2 gap was "key trait lookup on `(TraitId, ConstructorId)`". k6b shows the
concrete half is already live: `(TraitId, concrete Type)` keys on *applied* types and
dispatches through bounds. S2's remaining work: (a) constructor-abstract impl targets
(`for Option`), currently behind the generic-arity gate in `parse_impl_target`
(`src/parser.rs:2969`) — note that gate also hides unknown-trait errors behind it (k6);
(b) re-instantiating the constructor's variables per call site; (c) lifting the trait
single-variable gate (`multi_variable_trait_error`); (d) the cross-call/App unification
S1 fences off (R8i); (e) deciding the body-quotation `'F['U]` question (R8ii).

## Weight

M (parser-heavy but checker/IR work rides existing grounding rails). Files touched:
`src/parser.rs` (R4 router, R6 annotations, field-shape continuation, use-site ctor args),
`src/ast.rs` (Kind promotion + Arrow, `PolyType::App`, `PolySig::ty_kinds`, Subst ruling),
`src/check/poly.rs` (`apply_subst`/`unify_poly_input` App arms, kind publication
consumers), `src/check/audits.rs` (forced arms), `src/ir/driver.rs` (`subst_polytype`
lookup arm), `src/repl.rs` (remap arm). The checker split signals do not fire: the App
arms land inside existing responsibilities (grounding, unification), not on a new axis.
