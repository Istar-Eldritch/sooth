# P7b.S7 paper tests — Monad.bind / Applicative.ap grounding

- Date: 2026-09-05. Measured against HEAD `a9eca84` (suite green).
- Source: P7b.S7 recon probe round (`slice7-probes.md`, verbatim log; fixtures
  preserved under `/tmp/p7bs7-probes/`). Every fixture's Before column is a
  direct probe-round measurement; the After column is the S7 exit's expected
  behaviour once the two fences are lifted and the already-shipped grounding
  mechanisms (`ground_member_poly`, `unify_member_operand`/`render_member_decl`)
  are verified against this shape — not yet measured, since the fences are
  not yet lifted.
- Convention: complete fixture text, so goldens can be written without
  deriving anything. Names follow `thing_condition_expected` (CLAUDE.md).
- Revised (post-review): the two grounding mechanisms G2/G3 depend on
  (`ground_member_poly`'s App-in-Quotation-row combination,
  `unify_member_operand`/`render_member_decl`) are **already shipped** — the
  work is verifying them against this shape, not extending them
  (`slice7-spec.md` REQ-5). G4 is now two fixtures, since the original one
  was found to be a placebo for its stated purpose (see G4 below).

## G1 — the trait declaration itself must parse and check

`monad_bind_declares_app_in_quotation_row`

```sth
// monad.sth
import: intrinsics * ;
trait: Monad['F: * -> *] :
  bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] )
;
```

Before (measured, P1): `error: trait 'Monad''s member at line 1, col 28 applies a
type variable inside a quotation row (''F[...]' may not appear inside '[ ... ]')`
— parse-time rejection, `app_in_member_quotation_row_error`
(`src/parser.rs:452`).

After (expected): parses to a registered `TraitDecl` with `bind`'s declared
effect intact, `'F` kind `* -> *`.

## G2 — Option impl dispatches and produces the expected value

`option_bind_dispatches_and_short_circuits`

```sth
// main.sth
import: intrinsics * ; import: core::option * ; import: self::monad * ;

impl: Monad for Option :
  bind ( Option['T] [ 'T -- Option['U] ] -- Option['U] )
    dup Some?
    if drop [ ~call ] over Some.val swap drop call
    else drop
  ;
;

: half ( i64 -- Option[i64] )
  dup 2 mod 0 eq
  if 2 div Some
  else drop None
;

: main ( -- )
  4 Some bind [ half ] ...
```

(exact body idiom TBD by the spec — the point is the shape: `Some 4` -> `half`
applied inside `bind` -> `Some 2`; `Some 3` -> `bind [ half ]` -> `None`.)

Before: cannot exist — `Monad` trait itself is unparseable (G1's fence).

After (expected): `4 Some [ half ] bind` produces `Option[i64]::Some(2)`;
`3 Some [ half ] bind` produces `Option[i64]::None`. IR: no call frame beyond
what a hand-written `and_then` would produce (matches roadmap exit clause).

## G3 — Result impl dispatches per constructor, distinct from Option's

`result_bind_dispatches_and_short_circuits_on_err`

```sth
impl: Monad for Result :
  bind ( Result['T 'E] [ 'T -- Result['U 'E] ] -- Result['U 'E] )
    dup Ok?
    if drop [ ~call ] over Ok.val swap drop call
    else drop
  ;
;
```

Before: cannot exist (same fence as G1/G2).

After (expected): `Ok 4 [ half_result ] bind` where `half_result` returns
`Err "odd"` on an odd input produces `Err "odd"`; the same call on `Ok 4`
(even) produces `Ok 2`. Distinct `impl:` from Option's — proves constructor-
keyed dispatch, not a single fallthrough.

## G4 — the row-fence lift is narrow, not a blanket admit (two fixtures)

**G4a (regression pin, reclassified) —**
`kind_incorrect_app_in_quotation_row_is_error`

```sth
trait: Bad['F: * -> *] :
  m ( 'F['T] [ 'T -- 'F ] -- 'F['T] )
;
```

(`'F` used bare in the quotation's output row, after an earlier App-head
mention establishes its kind as `* -> *` — not a valid standalone type.)
Before (measured, this pass, verbatim): `error: type variable \`'F\` at line 2,
col 22 is used as a plain type but has kind \`* -> *\` (from an application of
\`'F\` at line 1, col 12); a higher-kinded variable never appears bare` — this is
`arrow_var_used_bare_error` (`src/parser.rs:2658`), raised from
`mark_ty_star` (`src/parser.rs:1950`) while `raw_to_poly_type` builds the
signature, **strictly before** `member_shape_is_supported`'s row-shape gate
ever runs. **Reclassified: this fixture is a placebo for the row-fence's
narrowness** — it is rejected today for a reason **unrelated** to the row
fence (a pre-existing, independent kind check), and would be rejected
byte-identically even under a hypothetical *blanket* admit of the row fence.
Kept as a regression pin that this independent kind check still fires, not
as evidence the row-fence lift is narrow.

**G4b (the real narrowness witness, new) —**
`member_local_headed_app_in_quotation_row_is_still_unsupported`

```sth
trait: Bad2['F: * -> *] :
  m ( 'F['T] [ 'T -- 'G['U] ] -- 'F['T] )
;
```

(`'G` is a member *local*, not the trait's own header variable, heading an
application inside the quotation's output row.) Before (measured, this pass,
verbatim): `error: trait \`Bad2\`'s member at line 2, col 3 applies a type
variable inside a quotation row (\`'F[...]\` may not appear inside \`[ ... ]\`)\n  note: a type application is supported only in a plain signature slot; keep
quotation rows App-free` — confirms this is `app_in_member_quotation_row_error`,
not a kind error; the same fence G1 hits, before REQ-1 narrows it. After (expected, once
REQ-1 lifts the fence for the **trait-var-headed** case only): still
rejected, via the same `app_in_member_quotation_row_error` dispatch
(`member_quotation_row_mentions_app(t) && poly_type_app_head(t).is_none()`,
`src/parser.rs:3965`) — `member_shape_is_supported`'s own top-level `App`
arm (`head == 0` only, `src/parser.rs:386`) still rejects `'G`'s
application when the Quotation arm recurses into it, so the row still
admits *only* an App headed by the trait's own variable. The error message
is corrected (REQ-3) to say so, rather than the pre-S7 "keep quotation rows
App-free" (no longer true in general). This is the actual witness that the
fence lift is narrowed to "a trait-var-headed App inside a row", not
"anything inside a row."

## G5 — Applicative.ap, if it grounds for free

`applicative_ap_declares_app_of_quotation_argument`

```sth
trait: Applicative['F: * -> *] :
  ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] )
;
```

Before (measured, P3): `error: expected a type, found '[' at line 1, col 43 (a
type application's arguments are types, not quotations)` —
`app_arg_quotation_error` (`src/parser.rs:2630`, raised at `:5213` from
`parse_poly_app_arg`), a distinct, earlier (S1-era) fence from G1's.

After: **not a required exit criterion.** If lifting fence #2 for the
trait-member-declaration path (brief scope item 2) happens to admit this
shape too, this golden is a bonus; if it requires separate new grounding
machinery beyond what G1-G3 need, it is recorded as future work rather than
blocking the slice (roadmap: "if shape (b) grounds for free in the same
extension").

## Regression pin — named-constructor-of-quotation is unaffected

`named_ctor_of_quotation_argument_unchanged`

```sth
import: intrinsics | drop | ;
type: Box['T] v 'T ;
: h ( Box[ [ i64 -- i64 ] ] -- ) drop ;
```

Before (measured, P5): parses and checks cleanly (fails only at link time with
no `main` defined — expected for an incomplete scratch fixture, not an error
this golden should reproduce; the real golden pins a `main` that exercises
`h`). After: byte-identical — this path never touched fence #2's variable-
headed grammar, so it must not regress.
