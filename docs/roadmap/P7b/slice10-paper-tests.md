# P7b.S10 paper tests — export-ambiguity / third-module shape

- Date: 2026-09-05. Measured against HEAD `a9eca84` (`cd44b1c` P7b.S9 merged;
  suite 3175/0, unaffected by this slice's docs-only commit).
- Source: P7b.S10 recon probe round (`slice10-probes.md`, verbatim log;
  fixtures preserved under `/tmp/p7bs10-probes/fixtures/`), plus GH/GI/GJ/GK
  and the qualified-spelling-mints-a-second-candidate finding, all measured
  during the pre-implementation review round, plus GL/GM and the
  own-signature-spelling remedy (`rem2sig`), measured during the round-2
  review (both dated corrections appendices in `slice10-probes.md`). Every
  fixture below was built and its Before column measured directly; the After
  column is the S10 policy's expected behaviour, not a measurement.
- Convention: complete fixture text (every source file, including the
  manifest), so a golden can be written without deriving anything. Test names
  follow `thing_condition_expected` (CLAUDE.md). Diagnostics are behaviour:
  error goldens pin byte-exact text.
- Manifest convention: every fixture's `sooth.pkg` follows
  `tests/phase7b_slice9.rs`'s `write_manifest` shape (`depends: core path
  "{CARGO_MANIFEST_DIR}/lib/core" ;`, same for `hosted`) — reproduced below
  once, not repeated per fixture.

```sth
// sooth.pkg (every fixture; {root} = env!("CARGO_MANIFEST_DIR") in the harness)
package: p7bs10 ;
layer: hosted ;
depends: core path "{root}/lib/core" ;
depends: hosted path "{root}/lib/hosted" ;
```

## The shape under policy

Two modules each declare their own same-named generic header
(`type: Widget['T] v 'T ;`) with their own `impl: Sized for Widget` (constants
1 and 2); a third module `c` imports both, declares **no** `Widget` header of
its own, and makes a **bare** `Widget` ctor call feeding a `size` member call.
Today this silently dispatches on whichever module happens to spell the
instantiation eagerly — the S10 policy replaces the silent pick with a located
compile-time ambiguity error, scoped to headers reachable through the
caller's own imports (R1), with a matching-explicit-resolution exemption
(GI/GJ) that must not degrade into a blanket "a selective import exists"
exemption (GK) and must resolve through a re-exporting hub before comparing
(GL); the reachable-header count itself additionally requires the sole
candidate's own declaring module to be reachable, not just that few enough
headers are reachable overall (GM).

`f.sth` is shared by every fixture below:

```sth
// f.sth
import: intrinsics * ;
trait: Sized['S] : size ( 'S -- i64 ) ; ;
: sized['S: Sized] ( 'S -- i64 ) size ;
export: Sized sized ;
```

## GA — the central golden

`third_module_bare_caller_with_ambiguous_headers_is_a_located_error`
(new, `tests/phase7b_slice10.rs`) — **this is S9's own golden
`third_module_bare_caller_dispatches_the_single_shared_env_instantiation`**
(`tests/phase7b_slice9.rs`), inverted (REQ-6). Test both import orders in one
function (style: S9's own G4 test, `for (first, second) in [("a","b"),("b","a")]`).

Fixture `p1-a-b` (verbatim; import order `a` then `b`):

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: run ( i64 -- i64 ) Widget sized ;
export: run ;
```

```sth
// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
: run ( i64 -- i64 ) Widget usesize ;
export: run ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a ; import: self::b ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ;
: main ( -- ) 5 c::try . ;
```

Fixture `p1-b-a`: identical except `c.sth`'s import line reads
`import: self::b ; import: self::a ;` (order swapped). Only `b` is the sole
eager minter in both; the import-order swap alone must not change the
rendered module ordering in the error text (R4 — lexicographic, not
import-order).

| | measured |
| --- | --- |
| Before (a9eca84) | builds, prints `2`, exit 0 — deterministic, both import orders `p1-a-b`/`p1-b-a` (probes P1; re-confirmed during the pre-implementation review round) |
| After (S10 policy) | located ambiguity error naming `Widget`, modules `a` and `b` (lexicographic order — identical text for both `p1-a-b` and `p1-b-a`), and `c`'s call site (line 3); exit 1. Byte-exact text pinned once the implementing phase measures the rendered output |

## GB — minter-swap twin (the minter must stop deciding)

`third_module_bare_caller_error_is_independent_of_the_eager_minter`
(new) — the other half of S9's own G4 (which asserts `1` here).

Fixture `p8-a-eager` (only `a` spells the instantiation eagerly now):

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
: run ( i64 -- i64 ) Widget usesize ;
export: run ;
```

```sth
// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
: run ( i64 -- i64 ) Widget sized ;
export: run ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a ; import: self::b ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ;
: main ( -- ) 5 c::try . ;
```

| | measured |
| --- | --- |
| Before | prints `1`, exit 0 (probes P8; re-confirmed) — output decided by which unrelated module spells the instantiation |
| After | the **same** ambiguity error text as GA (modules `a`/`b`, lexicographic — identical to GA's text even though `a`, not `b`, is now the minter), exit 1 |

## GC — both modules spell eagerly (existing 2-candidate error, unchanged)

`both_modules_eager_2_candidate_ambiguity_error_unchanged` (regression pin)

Fixture `p2-both-eager`: GA's fixture with `a` also carrying
`: usesize ( Widget[i64] -- i64 ) size ;` (so both mints exist at parse time,
reaching the multi-candidate arm, not the single-candidate one S10 changes):

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
: run ( i64 -- i64 ) Widget usesize ;
export: run ;
```

```sth
// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
: run ( i64 -- i64 ) Widget usesize ;
export: run ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a ; import: self::b ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ;
: main ( -- ) 5 c::try . ;
```

| | measured |
| --- | --- |
| Before | `error: no overload of \`Widget\` in \`try\` (line 3) accepts these operands` + two `candidate: \`i64\`` lines, exit 1 (probes P2/P5j; re-confirmed) |
| After | **byte-identical** — the existing 2-candidate `select_overload` error is reached before the single-candidate arm and is not S10's target; no module names added (diagnostics are behaviour, no churn) |

## GD — compat: single declaring header (must survive untouched)

`single_declaring_header_bare_caller_still_resolves` (regression pin)

Fixture `p3a-single-lib-private` (**not** `p3-single-lib`, which trips the R18
export gate before ever reaching the ctor call — `lib.sth`'s `usesize` here
stays private, exporting nothing):

```sth
// lib.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 7 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
```

```sth
// app.sth
import: intrinsics * ; import: self::f * ;
import: self::lib ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::app ;
: main ( -- ) 5 app::try . ;
```

| | measured |
| --- | --- |
| Before | prints `7`, exit 0 (probes P3a; re-confirmed) |
| After | **unchanged** — one same-named header program-wide (and reachable) is not ambiguity; exemption 2 fires |

## GE — single reachable header, selective import present but inert (must survive untouched)

`single_reachable_header_with_selective_import_still_resolves` (regression
pin; renamed from `selective_type_import_bare_ctor_pins_exporters_impl` —
that name overclaimed a pinning mechanism this fixture never exercises: only
module `a` exists here, so there is no second header to pin against.
Exemption 2 (≤1 reachable header) fires, not exemption 4)

Fixture `p5g2-selective-type` (no `b.sth` at all):

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
export: Widget ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a | Widget | ;
: mk ( i64 -- Widget[i64] ) Widget ;
: try ( i64 -- i64 ) mk size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ;
: main ( -- ) 5 c::try . ;
```

| | measured |
| --- | --- |
| Before | prints `1`, exit 0 (probes P5/P5g2; re-confirmed) |
| After | **unchanged** — only one header exists in this fixture; the selective import is present but inert |

## GF — single reachable header, qualified signature present but inert (must survive untouched)

`single_reachable_header_with_qualified_signature_still_resolves` (regression
pin; renamed from `qualified_type_spelling_bare_ctor_pins_exporters_impl` —
same reasoning as GE)

Fixture `p7c3-qualified-type` (no `b.sth` at all):

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
export: Widget ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a ;
: try ( a::Widget[i64] -- i64 ) size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ; import: self::a ;
: main ( -- ) 5 Widget c::try . ;
```

| | measured |
| --- | --- |
| Before | prints `1`, exit 0 (probes P7/P7c3; re-confirmed) |
| After | **unchanged** — only one header exists in this fixture; the qualified signature is inert |

## GG — annotated signature without import stays sharp (already an error, unchanged)

`unimported_foreign_type_annotation_is_still_an_error` (regression pin)

Fixture `p4-c-annotates`: `a.sth` exports only `run` (never `Widget`); `c.sth`
plainly imports `a` and annotates `Widget[i64]` in its own signature with no
type import bringing the name into scope:

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
: run ( i64 -- i64 ) Widget usesize ;
export: run ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a ;
: try ( Widget[i64] -- i64 ) size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ; import: self::a ;
: main ( -- ) 5 a::run drop 5 c::try . ;
```

| | measured |
| --- | --- |
| Before | `error: unknown type \`Widget\` at line 3, col 9`, exit 1 (probes P4) |
| After | **unchanged** — type-position naming already has its own rule; S10 governs term-position bare ctor calls only |

## GH — unimported declaring module does not count toward ambiguity (new; justifies reachability-scoping)

`unimported_declaring_module_does_not_count_toward_ambiguity` (new)

Fixture `overfire` (built during the pre-implementation review round
specifically to falsify a program-wide header count): two headers exist in
the whole program closure (`lib`, `z`), but `app` — the module whose call is
actually checked — imports only `lib`, never `z`. `z` is reachable from
`main` (which imports both `app` and `z`), but **not** from `app` itself,
whose own `ModuleInfo.imports`/`.selective` is what R1's reachability count
reads.

```sth
// lib.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 7 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
```

```sth
// z.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 9 ; ;
: run ( i64 -- i64 ) Widget sized ;
export: run ;
```

```sth
// app.sth
import: intrinsics * ; import: self::f * ;
import: self::lib ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::app ; import: self::z ;
: main ( -- ) 5 app::try . ;
```

| | measured |
| --- | --- |
| Before | prints `7`, exit 0 |
| After | **unchanged, both before and after** — `z`'s header is not reachable from `app`'s own imports, so it does not count toward the ≥2 threshold even though it exists in the program closure; this is the fixture that falsifies a naive program-wide header count (a program-wide count would wrongly flag this as ambiguous) |

## GI — matching selective import stays legal (new; exemption 4's positive case)

`matching_selective_import_of_the_sole_minter_still_resolves` (new)

Fixture `sel1`: two reachable headers (`a`, `b`); only `a` ever eagerly mints;
`c` selectively imports `a`'s `Widget` — the selected module matches the sole
candidate's declaring module.

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
export: Widget ;
```

```sth
// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
export: Widget ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a | Widget | ;
import: self::b ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ;
: main ( -- ) 5 c::try . ;
```

| | measured |
| --- | --- |
| Before | prints `1`, exit 0 |
| After | **unchanged, both before and after** — `c`'s selective import names `a`, and `a` is the sole eager minter (`b` never mints); exemption 4's match test succeeds |

## GJ — selective import pins the named exporter when both mint (multi-candidate arm, unchanged)

`selective_import_pins_the_named_exporter_when_both_mint` (new; makes
explicit what the old GE description assumed — this is the fixture where
selective import genuinely disambiguates, because it reaches the
**multi-candidate** arm)

Fixture `p5i2-selective-one-of-two`: both `a` and `b` eagerly mint; `c`
selectively imports `a`'s `Widget`.

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
export: Widget ;
```

```sth
// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
export: Widget ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a | Widget | ;
import: self::b ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ;
: main ( -- ) 5 c::try . ;
```

| | measured |
| --- | --- |
| Before | prints `1`, exit 0 |
| After | **unchanged, both before and after** — 2 env candidates (multi-candidate arm), existing S5 tier-2 pinning selects `a`; exemption 3, untouched by S10 |

## GK — mismatched selective import is still ambiguous (new; the soundness-critical case)

`mismatched_selective_import_is_still_a_located_error` (new; built during the
pre-implementation review round specifically to probe whether "a selective
import exists" alone is a sound exemption — it is not)

Fixture `p9-mismatched-selective`: two reachable headers (`a`, `b`); only `b`
ever eagerly mints; `c` selectively imports **`a`'s** `Widget` (the module
that does *not* mint) and also plainly imports `b`:

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: run ( i64 -- i64 ) Widget sized ;
export: run Widget ;
```

```sth
// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
: run ( i64 -- i64 ) Widget usesize ;
export: run ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a | Widget | ;
import: self::b ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ;
: main ( -- ) 5 c::try . ;
```

| | measured |
| --- | --- |
| Before (a9eca84) | silently prints `2` (`b`'s impl), exit 0 — `c` selectively imported `a`'s `Widget`, but the sole existing instantiation belongs to `b`; the caller's own selection is silently overridden, undetected |
| After (S10 policy) | the **same** located ambiguity error as GA (naming modules `a` and `b`), exit 1 — exemption 4 requires the selected module to equal the sole candidate's **declaring** module; here they disagree (`a` selected, `b` is the sole candidate), so the exemption does not apply and the general rule fires |

**Why this fixture exists:** a naive reading of "explicit resolution ⇒
exempt" (checking only whether *some* selective import of this name exists,
without comparing it against the actual candidate) would exempt this case too
— reproducing exactly the silent, caller-can't-see-it mis-dispatch S10 exists
to close, merely hidden behind a selective import that looks like it should
have worked. GK is the regression pin against that specific implementation
mistake. **GK is curable** (round-2 finding, R5's two-part remedy 2): see
`rem2sig` under "measured but not adopted" below — additionally spelling the
concrete type in an own-signature intermediate word sidesteps the borrow
entirely; GK itself does not do this, so it still errors.

## GL — a re-exporting hub does not defeat the explicit-resolution exemption (new; round-2)

`hub_reexported_selective_import_still_resolves` (new; built during the
round-2 review specifically to probe whether the exemption-4 match test
compares against the raw, one-hop selective-import target or the true,
possibly-hub-mediated declaring module)

Fixture `hub`: module `h` declares **no** `Widget` header of its own — it
only re-exports `a`'s, selectively importing it and re-exporting the bare
name. `c` selectively imports `Widget` from `h` (not from `a` directly),
plus plain imports of `a` and `b`. Only `a` ever eagerly mints.

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
export: Widget ;
```

```sth
// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
export: Widget ;
```

```sth
// h.sth
import: intrinsics * ; import: self::a | Widget | ;
export: Widget ;
```

```sth
// c.sth
import: intrinsics * ; import: self::f * ;
import: self::h | Widget | ;
import: self::a ; import: self::b ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ;
: main ( -- ) 5 c::try . ;
```

| | measured |
| --- | --- |
| Before (a9eca84) | builds, prints `1`, exit 0 |
| After (S10 policy) | **unchanged, both before and after** — `c`'s raw `ModuleInfo.selective` value for `Widget` is `h` (the import target), not `a`; `h` declares no header of its own, so a naive comparison against the raw value would mismatch (`h` ≠ `a`, the sole candidate's declaring module) and wrongly error. Exemption 4 fires only because the match resolves `h`'s own re-export chain down to `a` first (R2) |

**Why this fixture exists:** without the hub-resolution step, the
exemption-4 match test as originally drafted (compare the raw selective
target directly) would break exactly this legitimate, correctly-resolving
program — a regression the round-1 spec did not anticipate. GL is the
regression pin against that specific implementation mistake.

## GM — an unreachable minter is still an error, even with one reachable header (new; round-2, the soundness-critical case)

`unreachable_minter_bare_call_is_a_located_error` (new; built during the
round-2 review specifically to probe whether exemption 2's original
"≤1 reachable header" half, alone, is a sound single-lib compat test — it is
not)

Fixture `under`: `app` imports only `lib`, which declares its own `Widget`
header but never spells `Widget[i64]` in any of its own signatures (so it
never eagerly mints). `z` also declares its own `Widget` header and **does**
eagerly mint — but `app` never imports `z` in any form; only `main` does.

```sth
// lib.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 7 ; ;
: run ( i64 -- i64 ) Widget sized ;
export: run ;
```

```sth
// z.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 9 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
: run ( i64 -- i64 ) Widget usesize ;
export: run ;
```

```sth
// app.sth
import: intrinsics * ; import: self::f * ;
import: self::lib ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
```

```sth
// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::app ; import: self::z ;
: main ( -- ) 5 app::try . ;
```

| | measured |
| --- | --- |
| Before (a9eca84) | silently prints `9` (`z`'s impl), exit 0 — `app` never imports `z` at all, yet its bare `Widget` ctor call resolves to `z`'s instantiation, since `z`'s is the only one that was ever eagerly minted program-wide |
| After (S10 policy) | a located ambiguity error — exemption 2's original "≤1 reachable header" half alone is satisfied (only `lib`'s header is reachable from `app`), but the *second*, tightened half fails: the sole candidate's declaring module (`z`) is not itself reachable from `app`. Since `app` has no qualifier for `z` at all (never imported, not even via a wildcard), the message names it structurally rather than fabricating a name (R2/R3, the `drop`-diagnostic precedent) |

**Why this fixture exists:** exemption 2 as originally drafted (≤1 reachable
header, full stop) would silently exempt this case — `app` sees only one
reachable header (`lib`'s), so the original rule would let the bare ctor call
borrow whichever module happens to have minted, even though that module
(`z`) is one `app`'s author has never heard of. This is the exact
silent-mis-dispatch defect class S10 exists to close, reappearing through a
gap the reachable-*header*-count check alone does not cover (the count was
about how many *headers* compete, not whether the *actual* candidate's
module is one the caller can even see). GM is the regression pin against
that specific gap.

## Measured but not adopted as a golden

- **`qual2`** (two reachable headers `a`/`b`; only `a` mints via both its own
  signature and `c`'s qualified-type signature reference; `main` performs a
  **separate**, fully bare `Widget` ctor call feeding `c::try`): measured
  prints `1`, exit 0 today. Under R1 this call (main's own, not c's) would
  newly error (2 reachable headers from `main`'s own imports, no own header,
  no matching selective import) — but the fixture entangles two different
  call sites (main's bare ctor, and c's qualified-signature type reference)
  in one program, so it does not cleanly witness any single exemption. Not
  adopted; `p7c3-qualified-type` (GF) remains the qualified-spelling
  regression pin, correctly re-scoped as a single-header compat case.
- **The qualified-spelling-mints-a-second-candidate fixture** (`a.sth` bare,
  `b.sth` eager via its own signature, `c.sth` writing `a::Widget[i64]` in its
  own signature): measured
  `error: no overload of \`Widget\` in \`main\` (line 3) accepts these
  operands` + two `candidate: \`i64\`` lines — the pre-existing 2-candidate
  error, because the qualified signature reference is itself a second
  parse-time eager mint (of `a`), not a term-position resolution. This is the
  direct evidence backing R5's decision to drop qualified spelling from the
  remedy note entirely (it never cures; it just converts the shape into GC's).
- **Own header with its own impl** (`c` declares `type: Widget['T]` and
  `impl: Sized for Widget` locally, in addition to `a`/`b`'s headers):
  measured prints `9` (c's own impl) — confirms exemption 1 (own header
  grounds first, S9's R1.1a, untouched) continues to hold regardless of how
  many foreign headers exist. Not a new golden; S9's own goldens already
  cover this mechanism.
- **`rem2sig`** (GK's exact shape — `c` selectively imports `a`'s `Widget`
  while `b` is the sole eager minter — but `c` additionally writes
  `: mk ( i64 -- Widget[i64] ) Widget ;` and calls `mk` instead of
  constructing `Widget` bare inline): measured builds, prints `1`, exit 0.
  `mk`'s own signature spells `Widget[i64]` explicitly, so `c` itself becomes
  the *instantiating* module for that specific construction —
  `owning_module == caller_module` at `terms.rs:1504` short-circuits before
  the ambiguity check (or the borrow) ever runs; `c`'s selective import of
  `a`'s `Widget` is what lets `c`'s own bare, unqualified signature reference
  resolve to `a`'s header in the first place. Not a golden of its own (its
  behaviour is a corollary of the existing own-instantiating-module
  short-circuit at `:1504`, not a new mechanism S10 adds) — the direct
  evidence backing R5's two-part remedy 2 (GK's row cross-references it).

## Unit sketches (beside the changed code)

- `ambiguous_foreign_headers_grounding_is_located_error` — headerless caller,
  ≥2 same-named headers from reachable distinct modules in the generic
  registry, single env candidate ⇒ the grounding path errors (never falls
  through to the borrowed mint).
- `single_foreign_header_grounding_still_borrows` — same shape but at most one
  same-named header *reachable* ⇒ current borrow behaviour, covering both
  "no second header exists anywhere" (GD) and "a second header exists but is
  not reachable" (GH's mechanism at unit level).
- `own_header_still_grounded_first` — caller declares its own header ⇒ S9's
  R1.1a grounding, no ambiguity (the policy's caller-owns exemption).
- `ambiguous_header_error_names_declaring_modules` — the rendered message
  contains the surface name, both declaring modules (lexicographically
  ordered), and the call site.
- `reachable_header_count_excludes_unimported_declaring_modules` — a header
  declared by a module absent from the caller's own `imports`/`selective` maps
  does not count toward the ≥2 threshold (GH's mechanism at unit level).
- `matching_selective_import_exempts_the_ambiguity_check` — the caller's
  `selective` map, resolved through any hub chain, names exactly the sole
  candidate's **declaring** module ⇒ exemption 4 fires, no error (GI's
  mechanism at unit level).
- `mismatched_selective_import_does_not_exempt_the_ambiguity_check` — the
  caller's `selective` map, resolved through any hub chain, names a
  *different* reachable module than the sole candidate's declaring module ⇒
  exemption 4 does **not** fire, the error still raises (GK's mechanism at
  unit level — the soundness-critical case). A unit harness exercising this
  needs `ctx.modules = Some(&[...])` explicitly — the default harness passes
  `None` (`terms.rs`'s existing unit tests), which never reaches this check
  at all (R2's "never fires when absent" discipline).
- `hub_reexported_selective_import_resolves_through_origin_walk` — the
  caller's raw `selective` value names a re-exporting hub with no header of
  its own; the match test resolves through the hub's own re-export chain to
  the true declaring module before comparing ⇒ exemption 4 still fires, no
  error (GL's mechanism at unit level).
- `unreachable_declaring_module_of_the_sole_candidate_is_still_an_error` —
  at most one same-named header is reachable, but the sole env candidate's
  declaring module is a *different*, unreachable module ⇒ exemption 2 does
  **not** fire (both halves are required), the error still raises, named
  structurally when the caller has no qualifier for that module at all
  (GM's mechanism at unit level).

## Boundary notes for the spec (from the probes, the pre-implementation review round, and round 2)

- The policy lives at the single-candidate arm's grounding check
  (`bare_generated_word_own_module_grounding`, `src/check/terms.rs:1507-1509`
  — the `find_struct(header_name, caller_module)` → `None` arm, not the
  `1516-1523` success-path region an earlier draft cited), reading header
  provenance from the generic registry (`ctx.generics().structs`), complete
  at env-build time (probes P6), and the caller's own import set from
  `ctx.modules` (`ModuleInfo.imports`/`.selective`, `ast.rs:175-189`).
- **Guard ordering (round 2):** the new arm must run only after the
  candidate has survived `owning_module == caller_module` (`:1504`) *and* the
  candidate-identity check (`generated_word_entry` + `key`/`symbol` match,
  `:1531-1537`). Today's `:1507-1509` fall-through returns `Ok(None)`
  immediately, before ever reaching `:1531-1537` — the implementing phase
  restructures the control flow so a headerless caller still passes through
  the identity check first; the `:1492-1497` "the order is free" comment is
  updated to match once one arm can return `Err(...)`.
- **Declaring vs. instantiating module (round 2):** `struct_instantiation_of`
  returns the *instantiating* module (`ast.rs:2598`'s `Generic::module` doc
  comment); the true *declaring* module is `guard.structs[gi].module`. R1's
  exemption 2 and exemption 4 compare against the declaring module — they
  coincide in every existing golden, but not in GL's hub shape.
- NOT at `select_overload`/`tier_pick` (1 candidate by construction;
  `tier_pick`'s lone-survivor ruling, `src/check/builtins.rs:118-127`,
  deliberately never errors on one candidate).
- NOT at env build (`src/check.rs:586`) — no call site exists there to locate
  an error at.
- Matcher untouched (S9's R-NFR2 carried over): no
  `find_bound_impl`/`match_impl_target` changes. No IR/lowering edits
  (R-NFR1). The existing 2-candidate error path and S5's tier policy are
  upstream of the new check and stay byte-identical.
- Module naming has no canonical, caller-independent source (`ModuleInfo` has
  no `name` field) — every existing diagnostic that names a foreign module
  renders the caller's own import qualifier for it (`declarations.rs:974`,
  `:988`, `word_families.rs:1336`). S10 follows the same precedent and sorts
  the collected qualifiers lexicographically for determinism (R4) — not
  registry, import, or mint order. When the sole candidate's declaring
  module has no caller-side qualifier at all (GM), the message names it
  structurally instead, following the existing `drop`-visibility
  diagnostic's own fallback for exactly this gap
  (`word_families.rs:1319-1340`).
- **Hub resolution (round 2):** a caller's raw `ModuleInfo.selective` value
  may name a re-exporting hub, not the true declaring module (GL). The
  implementing phase resolves it through the same hop-walk the checker
  already runs for a type reference in an effect signature
  (`resolve_type_export_origins`/`walk_type_export_origin`, `driver.rs:364`/
  `:407`) before comparing.
- `export: Widget[i64]` is a parse error and exporting a word whose effect
  names `Widget[i64]` trips the R18 gate with an unsatisfiable remedy —
  pre-existing warts, explicitly out of S10 scope; the policy may not assume
  "export the word" as a workaround.
