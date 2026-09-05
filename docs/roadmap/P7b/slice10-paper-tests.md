# P7b.S10 paper tests — export-ambiguity / third-module shape

- Date: 2026-09-05. Measured against HEAD `cd44b1c` (suite 3175/0).
- Source: P7b.S10 recon probe round (`slice10-probes.md`, verbatim log; fixtures
  preserved under `/tmp/p7bs10-probes/fixtures/`). Every fixture below was
  built and its Before column measured directly against this HEAD; the After
  column is the S10 policy's expected behaviour, not a measurement.
- Convention: complete fixture text (every source file), so a golden can be
  written without deriving anything. Test names follow
  `thing_condition_expected` (CLAUDE.md). Diagnostics are behaviour: error
  goldens pin byte-exact text.

## The shape under policy

Two modules each declare their own same-named generic header
(`type: Widget['T] v 'T ;`) with their own `impl: Sized for Widget` (constants
1 and 2); a third module `c` imports both, declares **no** `Widget` header of
its own, and makes a **bare** `Widget` ctor call feeding a `size` member call.
Today this silently dispatches on whichever module happens to spell the
instantiation eagerly — the S10 policy replaces the silent pick with a located
compile-time ambiguity error.

## GA — the central golden

`third_module_bare_caller_with_ambiguous_headers_is_a_located_error`
(new, `tests/phase7b_slice10.rs`)

Fixture `p1-a-b` (verbatim; `f.sth` shared by GA/GC/GD/GF):

```sth
// f.sth
import: intrinsics * ;
trait: Sized['S] : size ( 'S -- i64 ) ; ;
: sized['S: Sized] ( 'S -- i64 ) size ;
export: Sized sized ;
```

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

| | measured |
| --- | --- |
| Before (cd44b1c) | builds, prints `2`, exit 0 — deterministic 3/3 rebuild+run cycles, both import orders (a,b) and (b,a) (probes P1) |
| After (S10 policy) | located ambiguity error naming `Widget`, the two declaring modules, and `c`'s call site; exit 1. Byte-exact text pinned once the spec fixes the wording |

## GB — minter-swap twin (the minter must stop deciding)

`third_module_bare_caller_error_is_independent_of_the_eager_minter`
(new; same fixture family, only `a` spells the instantiation eagerly — a gets
the `usesize ( Widget[i64] -- i64 )` word, `b` keeps only the bare `run`)

| | measured |
| --- | --- |
| Before | prints `1`, exit 0, 2/2 cycles (probes P8) — output decided by which unrelated module spells the instantiation |
| After | the same ambiguity error as GA (exit 1) — the minter is irrelevant |

## GC — both modules spell eagerly (existing 2-candidate error)

`both_modules_eager_2_candidate_ambiguity_error_unchanged` (regression pin)

Fixture `p2-both-eager`: GA's fixture with `a` also carrying
`: usesize ( Widget[i64] -- i64 ) size ;` (so both mints exist at parse time).

| | measured |
| --- | --- |
| Before | `error: no overload of \`Widget\` in \`try\` (line 3) accepts these operands` + two `candidate: \`i64\`` lines, exit 1, deterministic 2/2 (probes P2/P5j) |
| After (default ruling) | **unchanged** — the existing 2-candidate `select_overload` error is reached before the single-candidate arm and is not S10's target. (Open question for the spec: whether to upgrade this text with module names too; default is no churn — diagnostics are behaviour) |

## GD — compat: single declaring header (must survive untouched)

`single_declaring_header_bare_caller_still_resolves` (regression pin)

Fixture `p3-single-lib`: `lib.sth` declares `Widget['T]` + impl (constant 7) and
keeps its Widget-touching words **private**; `app.sth` imports lib, bare ctor:

```sth
// lib.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 7 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
export: usesize ;
```

(plus `app.sth` = c.sth shape with `import: self::lib ;`, main printing via
`app::try`).

| | measured |
| --- | --- |
| Before | prints `7`, exit 0, 2/2 cycles (probes P3a) |
| After | **unchanged** — one same-named header program-wide is not ambiguity; the rule keys on ≥2 same-named headers from distinct modules with the caller declaring none |

## GE — remedy channel 1: selective type import (must survive untouched)

`selective_type_import_bare_ctor_pins_exporters_impl` (regression pin)

Fixture `p5g2-selective-type`: `a.sth` = GA's a.sth plus `export: Widget ;`;
`c.sth` = `import: self::a | Widget | ;` with
`: mk ( i64 -- Widget[i64] ) Widget ;` and `: try ( i64 -- i64 ) mk size ;`.

| | measured |
| --- | --- |
| Before | prints `1`, exit 0, 2/2 cycles — tier-2 ctor pinning to a's mint (probes P5/P5g2) |
| After | **unchanged** — a declared type-name import is an explicit resolution; the policy must not re-error it (P5i2's tier-2 pinning likewise) |

## GF — remedy channel 2: qualified type spelling (must survive untouched)

`qualified_type_spelling_bare_ctor_pins_exporters_impl` (regression pin)

Fixture `p7c3-qualified-type`: `a.sth` as in GE; `c.sth` = plain
`import: self::a ;` with `: try ( a::Widget[i64] -- i64 ) size ;`; main passes
a bare `Widget` (a's mint) to `c::try`.

| | measured |
| --- | --- |
| Before | prints `1`, exit 0, 2/2 cycles (probes P7/P7c3) |
| After | **unchanged** — qualified spelling is an explicit resolution |

## GG — annotated signature without import stays sharp (already an error)

`unimported_foreign_type_annotation_is_still_an_error` (regression pin)

Fixture `p4-c-annotates`: c's signature spells `( Widget[i64] -- i64 )` with no
own header and no type import.

| | measured |
| --- | --- |
| Before | `error: unknown type \`Widget\` at line 3, col 9`, exit 1 (probes P4 — the parser's type-name visibility rule, the standing S4-era cross-module generic-instantiation limit) |
| After | **unchanged** — type-position naming already has its rule; S10 governs term-position bare ctor calls only |

## Unit sketches (beside the changed code)

- `ambiguous_foreign_headers_grounding_is_located_error` — headerless caller,
  ≥2 same-named headers from distinct modules in the generic registry, single
  env candidate ⇒ the grounding path errors (never falls through to the
  borrowed mint).
- `single_foreign_header_grounding_still_borrows` — same shape but exactly one
  same-named header ⇒ current borrow behaviour (GD's mechanism at unit level).
- `own_header_still_grounded_first` — caller declares its own header ⇒ S9's
  R1.1a grounding, no ambiguity (the policy's caller-owns exemption).
- `ambiguous_header_error_names_declaring_modules` — the rendered message
  contains the surface name, both declaring modules, and the call site.

## Boundary notes for the spec (from the probes)

- The policy lives at the single-candidate arm's grounding check
  (`src/check/terms.rs:1516-1523` region), reading header provenance from the
  generic registry (`ctx.generics()` / `module.generic_structs`), which is
  complete at env-build time (probes P6). NOT at `select_overload`/`tier_pick`
  (1 candidate by construction; `tier_pick`'s lone-survivor ruling,
  `src/check/builtins.rs:118-127`, deliberately never errors on one candidate).
- NOT at env build (`src/check.rs:586`) — no call site exists there to locate
  an error at.
- Matcher untouched (S9's R-NFR2 carried over): no
  `find_bound_impl`/`match_impl_target` changes. No IR/lowering edits
  (R-NFR1). The existing 2-candidate error path and S5's tier policy are
  upstream of the new check and stay byte-identical.
- `export: Widget[i64]` is a parse error and exporting a word whose effect
  names `Widget[i64]` trips the R18 gate with an unsatisfiable remedy —
  pre-existing warts, explicitly out of S10 scope; the policy may not assume
  "export the word" as a workaround.
