> Probe round for P7b.S1 scoping, run 260830 against the current tree (worktree `incas`, HEAD `790b81c`) by the live-probe subagent: 15 compile/run probes under `/tmp/p7bs1-probes/`. Repo untouched (`git status --porcelain` empty at finish). Findings feed the P7b.S1 brief; this file is the verbatim log. Two paper recon workers (extension-point map; witness/golden design) ran alongside; their condensed findings live in [slice1-brief.md](./slice1-brief.md).

# P7b.S1 compile-time probe results

Probes for S1 (kinds + type-level application; see
`docs/roadmap/P7b-higher-kinded-types.md`). All probe files live under
`/tmp/p7bs1-probes/`; nothing in the repo was modified (`git status
--porcelain` empty at finish). Commands were run from the repo root:
`cargo run -- build|run /tmp/p7bs1-probes/<file>`. Fixture sources are
inline in each capture below.

Positive controls: the S6a `: Len` annotation (k1) and generic enum
instantiation (k2) work end to end, so every rejection below is attributable
to the missing HKT surface, not to baseline breakage.

## Summary table

| Probe | File | Outcome |
| --- | --- | --- |
| k0 smoke | k0_smoke.sth | compiles+runs, prints `42` |
| k1 S6a Len control | k1_len_control.sth | compiles+runs, prints `4` |
| k2 generic enum control | k2_enum_control.sth | compiles+runs, prints `42` / `0` |
| k3 higher-kinded annotation, spaced | k3_hk_annotation_spaced.sth | parse-stage rejection: kind position is parsed as a *bound/capability* position (exit 1) |
| k3b higher-kinded annotation, glued | k3b_hk_annotation_glued.sth | same, glued spelling is one token (exit 1) |
| k3c `: Len` var unused | k3c_len_var_unused.sth | "length variable ... never appears in the effect" (exit 1) |
| k3d `: Len` var used as a type | k3d_len_var_as_type.sth | same message — kind misuse has no dedicated diagnostic today (exit 1) |
| k4 `'F['T]` in a word signature | k4_type_application.sth | parse error: `[` after a type variable is read as a quotation-effect opener (exit 1) |
| k4b `'F['T]` in a `type:` header field | k4b_type_header_application.sth | parse error: `expected a word, found LBracket` — type positions have no application grammar at all (exit 1) |
| k5 HKT trait member | k5_trait_hk.sth | same quotation-effect parse error; fires BEFORE the single-var gate (exit 1) |
| k5b minimal HKT trait member | k5b_trait_hk_minimal.sth | same, at the application bracket (exit 1) |
| k6 `impl: Functor for Option` (bare constructor) | k6_impl_constructor.sth | check error: bare generic type must be applied — the arity gate, not an unknown-trait error (exit 1) |
| k6b `impl: Functor for Option[i64]` + bound dispatch | k6b_impl_applied_target.sth | **compiles+runs, prints `1`** — applied-target impls and (TraitId, concrete Type) dispatch already work (exit 0) |
| k7 `i64['T]` in a signature | k7_kind_error_control.sth | same quotation-effect parse error — no kind-checking can exist before the parse admits applications (exit 1) |
| k8 unannotated length var in signature | k8_len_abstract_signature.sth | "type variable `'N` ... never appears in the effect" — Len inference from array usage does not happen; `: Len` is mandatory (exit 1) |

Note: every compiler diagnostic prints as `error: error: ...` was the S7c
observation; on the current tree (HEAD `790b81c`, the one-error-prefix fix)
diagnostics print with a single `error:` prefix. Quoted verbatim below.

## Verbatim captures

### k0 — `cargo run -- run /tmp/p7bs1-probes/k0_smoke.sth` (exit 0)

```text
42
```

Source: `import: intrinsics * ;\n: main ( -- ) 42 . ;`.

### k1 — `cargo run -- run /tmp/p7bs1-probes/k1_len_control.sth` (exit 0)

```text
4
```

Source:

```sth
import: intrinsics * ;
: sum['T 'N: Len] ( array['T 'N] -- usize ) len swap drop ;
: main ( -- ) 0 4 fill sum[i64 4] . ;
```

The S6a shape works end to end: header `: Len` annotation, `array['T 'N]`
in the effect, explicit `sum[i64 4]` at the call site.

### k2 — `cargo run -- run /tmp/p7bs1-probes/k2_enum_control.sth` (exit 0)

```text
42
0
```

Source:

```sth
import: intrinsics * ;
type: Option['T] | None | Some val 'T ;
: to-int ( Option[i64] -- i64 ) ~[ ( Some ) Some> ] ~[ ( None ) None> 0 ] Option? ;
: main ( -- ) 42 Some to-int . None to-int . ;
```

Generic enum declaration, `Option[i64]` in a concrete signature, and the
eliminator all work (baseline for S1's `'F['T]` to monomorphize *into*).

### k3 — `cargo run -- build /tmp/p7bs1-probes/k3_hk_annotation_spaced.sth` (exit 1)

```text
error: unknown capability `*` at line 2, col 9 (a bound names `Copy` or a trait in scope)
```

Source: `import: intrinsics * ;\n: f['F: * -> *] ( -- ) ;\n: main ( -- ) f ;`.

LOAD-BEARING: the annotation position after a header variable is parsed as
the **bound/capability** position (`parse_capabilities`). `Len` is accepted
there (k1) but there is no kind grammar — `*` is read as a capability name
and rejected as unknown. S1 must carve the kind syntax out of (or alongside)
the bound grammar at the same site; `: Len` proves the position is shared.

### k3b — `cargo run -- build /tmp/p7bs1-probes/k3b_hk_annotation_glued.sth` (exit 1)

```text
error: unknown capability `*->*` at line 2, col 9 (a bound names `Copy` or a trait in scope)
```

Source: `import: intrinsics * ;\n: f['F: *->*] ( -- ) ;\n: main ( -- ) f ;`.

The glued spelling is one word token (`*->*`), same capability rejection. No
lexer surprise: any Arrow kind syntax needs the kind grammar, not a lexing
fix. (Also confirms `:` is not a delimiter issue here — the annotation
parses; only the kind *name* fails.)

### k3c — `cargo run -- build /tmp/p7bs1-probes/k3c_len_var_unused.sth` (exit 1)

```text
error: length variable `'N` declared in the bound bracket of `f` at line 2, col 8 never appears in the effect
```

Source: `import: intrinsics * ;\n: f['T 'N: Len] ( 'T -- ) drop ;\n: main ( -- ) ;`.

### k3d — `cargo run -- build /tmp/p7bs1-probes/k3d_len_var_as_type.sth` (exit 1)

```text
error: length variable `'N` declared in the bound bracket of `f` at line 2, col 8 never appears in the effect
```

Source: `import: intrinsics * ;\n: f['T 'N: Len] ( 'T 'N -- ) drop drop ;\n: main ( -- ) 1 2 f ;`.

Kind misuse today has NO dedicated diagnostic: k3d's `'N` *does* appear in
the effect (as a value type — a genuine kind error), but the checker reports
"never appears in the effect", a mislabel. S1's "kind-incorrect application
is a located error" exit needs a real kind-mismatch diagnostic; today's
closest message is this heuristic, and it fires for both "unused" and
"used at the wrong kind".

### k4 — `cargo run -- build /tmp/p7bs1-probes/k4_type_application.sth` (exit 1)

```text
error: parse error: a quotation effect at line 2, col 30 must be written in full as `[ inputs -- outputs ]`, found no top-depth `--` (for an array type write `array[T N]`)
```

Source (first attempt used `inline` after the bracket — rejected at col 18
with `expected LParen, found Word("inline")`; the working word shape is
`inline` before the header bracket, as in `filter`):

```sth
import: intrinsics * ;
: pass inline ['F 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) call ;
: main ( -- ) ;
```

LOAD-BEARING: col 30 is the `[` of `'F['T]` — inside an effect, `[` after
*anything* is parsed as a quotation-effect opener. The effect-type grammar
has no type-application production; the error's own hint ("for an array type
write `array[T N]`") shows the only `[`-disambiguation that exists is
`array[...]`. S1's `'F['T]` in signatures is exactly a new production at
this site: after a type variable (or constructor word) in a type position,
`[` opens type arguments, not a quotation effect. Note the ambiguity is real:
`[ 'T -- 'U ]` IS a quotation type in the same grammar — the disambiguator
will be "did the bracket follow a type, or start a type position".

### k4b — `cargo run -- build /tmp/p7bs1-probes/k4b_type_header_application.sth` (exit 1)

```text
error: parse error: expected a word, found LBracket at line 2, col 23
```

Source:

```sth
import: intrinsics * ;
type: Wrap['F 'T] v 'F['T] ;
: main ( -- ) ;
```

In `type:` header field positions there is not even an effect-parse fallback:
`[` after a word is simply unexpected ("expected a word"). So S1 needs the
application production in BOTH type grammars: effect positions (k4, where
quotation effects collide) and type-header/field positions (k4b, where the
type-argument grammar of `L[i64]` exists for names but not for variables).
Col 23 is the `[` after `'F` in `v 'F['T]`.

### k5 — `cargo run -- build /tmp/p7bs1-probes/k5_trait_hk.sth` (exit 1)

```text
error: parse error: a quotation effect at line 2, col 30 must be written in full as `[ inputs -- outputs ]`, found no top-depth `--` (for an array type write `array[T N]`)
```

Source:

```sth
import: intrinsics * ;
trait: Functor['F] : map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ; ;
: main ( -- ) ;
```

The member's effect parse (same k4 site) fires BEFORE the single-var gate —
the S7c "names more than one type variable" gate never runs. So the trait
surface's first blocker is the same application parse as k4, not the
var-count gate. S2 (not S1) must lift the single-var gate, but S1's parse
change is what unlocks the trait surface.

### k5b — `cargo run -- build /tmp/p7bs1-probes/k5b_trait_hk_minimal.sth` (exit 1)

```text
error: parse error: a quotation effect at line 2, col 31 must be written in full as `[ inputs -- outputs ]`, found no top-depth `--` (for an array type write `array[T N]`)
```

Source:

```sth
import: intrinsics * ;
trait: Functor['F] : size ( 'F['T] -- i64 ) ; ;
: main ( -- ) ;
```

Even the minimal two-variable member (`'F['T]` only) dies at the application
bracket. The single-var gate would ALSO reject this (2 vars) — the gate and
the parse both need work before any HKT trait compiles; parse fires first.

### k6 — `cargo run -- build /tmp/p7bs1-probes/k6_impl_constructor.sth` (exit 1)

```text
error: generic type `Option` declares 1 type variable, but none were supplied at line 3, col 19 (apply it as `Option[T]`, one type argument per declared variable)
```

Source:

```sth
import: intrinsics * ;
type: Option['T] | None | Some val 'T ;
impl: Functor for Option
  : map ( Option['T] [ 'T -- 'U ] -- Option['U] ) call ;
;
: main ( -- ) ;
```

`impl:` targets go through the generic-instantiation arity check: a bare
constructor name is rejected as an uninstantiated generic. Note the unknown
trait `Functor` never gets reported — the target-arity gate fires first
(col 19 = `Option` in `for Option`). S2's constructor-keyed target
(`impl: Functor for Option`) must relax exactly this gate (or special-case
HKT traits), and the impl-target parser's ordering hides trait-existence
errors behind it.

### k6b — `cargo run -- run /tmp/p7bs1-probes/k6b_impl_applied_target.sth` (exit 0)

```text
1
```

Source:

```sth
import: intrinsics * ;
type: Option['T] | None | Some val 'T ;
trait: Functor['F] : size ( 'F -- i64 ) ; ;
impl: Functor for Option[i64]
  : size drop 1 ;
;
: sized ['F: Functor] ( 'F -- i64 ) size ;
: main ( -- ) 42 Some sized . ;
```

POSITIVE SURPRISE, LOAD-BEARING: a trait over an ordinary `*`-kind variable,
an impl whose target is an APPLIED generic type (`Option[i64]`), and
bound-dispatch where `'F` grounds to the concrete `Option[i64]` at the call
site — all work today. The (TraitId, concrete Type) registry already keys on
applied types; intermediate probe states confirmed the path: a member that
restates its signature is rejected (`impl member`size` must not restate its
signature at line 5, col 10 (it is inherited from trait `Functor`'s`size`
with the `for`type)`), a value operand against a `&'F` receiver is rejected
with the expected mismatch (`sized` expected `&'F`, found `Option[i64]`).
What S2 adds is keying on the bare constructor + re-instantiation; the
applied-target path it generalizes is already live.

### k7 — `cargo run -- build /tmp/p7bs1-probes/k7_kind_error_control.sth` (exit 1)

```text
error: parse error: a quotation effect at line 2, col 10 must be written in full as `[ inputs -- outputs ]`, found no top-depth `--` (for an array type write `array[T N]`)
```

Source: `import: intrinsics * ;\n: f ( i64['T] -- ) ;\n: main ( -- ) ;`.

Same k4 site for a CONCRETE type: `i64['T]` is also a quotation-effect parse
error today. Consequence for S1's exit criterion "kind-incorrect application
is a located error": no kind checker can fire before the parse admits
applications at all; the located kind error is a check-stage diagnostic that
only becomes reachable once k4's parse production exists.

### k8 — `cargo run -- build /tmp/p7bs1-probes/k8_len_abstract_signature.sth` (exit 1)

```text
error: type variable `'N` declared in the bound bracket of `sum2` at line 2, col 11 never appears in the effect
```

Source:

```sth
import: intrinsics * ;
: sum2['T 'N] ( array['T 'N] -- usize ) len swap drop ;
: main ( -- ) 0 4 fill sum2[i64 4] . ;
```

An UNANNOTATED `'N` in the array length slot is rejected even though `'N`
literally appears in `array['T 'N]`: without `: Len` the variable cannot
bind to the length position, so the appearance does not count. Compare k3c's
message: same family, but "type variable" (unannotated) vs "length variable"
(annotated). Inference from usage does not exist for Len today — `: Len` is
mandatory — so S1's "kind inference from usage context" (`'F` appearing only
as `'F['T]` ⇒ kind `* -> *`) is NEW machinery, not an extension of an
existing inference pass. (k1, with `: Len`, is the working control.)

## Cross-cutting observations for the S1 brief

1. **The kind position IS the bound position.** `: Len` (k1) and the
   capability rejection for `*` (k3) come from the same annotation site
   (`parse_capabilities` region). S1's Arrow kind syntax and S1's kind/bound
   coexistence (`'T: Copy` + `'F: * -> *` in one header) must be designed
   against that single parser.
2. **`[` is overloaded three ways**: quotation effect, `array[...]` type,
   and generic-name type arguments (`L[i64]`). The k4/k7 site is where a
   fourth reading (application of a variable/constructor) must disambiguate;
   the parser's own error text already advertises the `array[T N]` escape
   hatch, so the disambiguator should key on what precedes the bracket.
3. **Kind enforcement exists today only at the array length slot** (k1/k8),
   with no dedicated kind-mismatch diagnostic (k3d mislabels kind misuse as
   "never appears in the effect"). S1 should introduce the real
   kind-mismatch diagnostic and consider re-pointing k3d's case at it.
4. **Applied-target impl dispatch already works** (k6b) — the concrete-type
   half of S2's registry is live on this tree; only constructor-keyed
   targets are missing, behind the k6 arity gate.
5. **Trait single-var gate ordering**: the member effect-parse fires before
   the var-count gate (k5/k5b), so S1's parse work lands first regardless;
   the gate itself is S2's to lift.
