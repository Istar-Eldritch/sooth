# P7.S11-follow brief — The frozen call-site env

## Problem, confirmed live against current `main`

`check::check`'s concrete `env: HashMap<String, Vec<Overload>>` (`src/check.rs:559-574`) is
built **once**, before the per-word loop (`src/check.rs:911`), from `struct_generated_sigs`/
`enum_generated_sigs`/`variant_generated_sigs` run over whatever `module.structs`/
`module.enums` already contain at that point — every *parse-time* monomorph (an explicit
`Result[i64 i64]` written somewhere in the source, or one required by another word's
signature) is in there, but nothing minted later.

A **check-time** monomorph — one `apply_subst`'s `Generic` arm mints while checking an
ordinary word's own body (`src/check.rs:990-1006`), or one `check_poly_combinator_standalone`
mints while grounding an `inline` combinator's own signature (`src/check/poly.rs:381`, the
mechanism P7.S11 just built) — is flushed into the *live* `structs`/`enums` vectors right
after that word is checked:

```rust
let mut g = generics_cell.borrow_mut();
g.flush_structs_into(structs);
g.flush_enums_into(enums);
```

(`src/check.rs:1004-1006`, ordinary-word arm). But `env` itself is never touched by this
flush — nothing re-runs `enum_generated_sigs`/`struct_generated_sigs`/
`variant_generated_sigs` over the newly-appended tail and merges the result back in. The
call-site `env` every subsequent word's `check_word` call is handed
(`&env` at `src/check.rs:915` / `:998`) is the one built before the loop started, frozen for
the rest of the module.

Consequence, pinned live by P7.S11's golden 6 (`tests/phase7_slice11.rs:198`, and struct
twin golden 10 at `:283`):

```sooth
type: Result['T 'E] | Ok 'T | Err 'E ;
: wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;
: main ( -- )
  7 ~[ 1 add ] wrap
  ~[ ( Ok ) Ok> . ]
  ~[ ( Err ) Err> . ]
  Result? ;
```

`wrap` is `inline`, so P7.S11's word-scoped-registries fix lets its own body's
`Ok` construction check cleanly (the monomorph is registered into a *word-scoped clone* of
`env` for `wrap`'s own standalone check, per S11's design — deliberately dropped when that
check returns, never touching the live `env`). But `wrap`'s splice site inside `main` is
checked separately, against the module's live `env`, which has no `Result[i64 i64]`
constructor entry unless some *other*, unrelated word in the program happens to also
instantiate it — golden 9 (`tests/phase7_slice11.rs:262`) is the fixture that shows this:
adding `: mki ( i64 -- Result[i64 i64] ) Ok ;` anywhere in the file makes the "unknown word
`Ok`" error at `main` disappear.

So a file whose *only* use of a generic type is a combinator's own construction of it (the
`map`/`and_then` shape P7.S11 exists to unblock) still does not build end to end: the
combinator's own definition checks, but every call site that uses its result fails with
"unknown word" on the generated constructor/accessor.

## Mechanism (why it's this shape, not a smaller patch)

Two independent mint sites feed the same gap, and only one of them can safely reach the
live `env`:

- **Ordinary-word path** (`src/check.rs:990-1006`): `generics_cell` is the *real*, live
  `GenericTypes` for the whole module (`std::mem::take(generics)` at `:723`). A mint here is
  flushed into the live `structs`/`enums` unconditionally — it is a real monomorph the
  program now contains, and every later word needs to see it, including through `env`. This
  is the fix's actual scope.
- **Standalone-combinator path** (`ground_into_word_scoped_registries`,
  `src/check/poly.rs:412-461`): deliberately mints into a *cloned*
  `WordScopedRegistries`, dropped when the check returns — P7.S11's explicit design, so
  nothing here should ever reach the live `env` either. Confirmed: this path's registrations
  are already correctly out of scope; the gap this brief targets is the sibling path above.

`instantiate_struct`/`instantiate_enum` (`src/ast.rs:1139`, `:1193`) dedup by a memo key
before minting (`instantiate_struct_dedups_and_counts_from_its_base`,
`src/ast.rs:3721`), so re-grounding the same header from two different words never mints a
second decl — extending `env` incrementally, keyed off exactly the tail each word's flush
appends (`structs[prev_struct_len..]`, `enums[prev_enum_len..]`), cannot double-register the
same monomorph's constructor under two `Overload` entries, provided the extension reads the
post-flush length each time rather than the whole vector.

## What's already known to work (no design question here)

- The signature-slot grounding itself (`apply_subst`'s `Generic` arm) is correct; the flush
  into `structs`/`enums` is correct; the only missing step is projecting that flush through
  `struct_generated_sigs`/`enum_generated_sigs`/`variant_generated_sigs` into `env`, the same
  three functions the pre-loop setup already calls once (`src/check.rs:563-573`).
- Dedup safety across repeated mints of the same header is already proven
  (`src/ast.rs:3721`, `:4031`).
- `poly_env` (the polymorphic-word table) is untouched by this gap: a poly word's own
  signature never depends on a concrete monomorph existing in `env` ahead of time — only a
  *concrete* call site's constructor/accessor lookup does.

## Open questions for the spec

- **Where exactly to run the re-projection.** The natural site is right after
  `g.flush_structs_into(structs)` / `g.flush_enums_into(enums)`
  (`src/check.rs:1005-1006`), reading the pre-flush lengths (captured before `check_word`
  runs) and generating sigs only for the tail past them. Needs confirming this captures both
  a struct and an enum mint in the same word (golden 10's struct case, goland 6/9's enum
  case) without double-running the untouched half.
- ~~Whether `eliminator_registry`'s per-enum name gate has the identical gap.~~ **Resolved by
  probe (2026-08-29, against current `main`):** ruled out. Fixture: golden 6's exact source,
  plus golden 9's sibling-monomorph workaround (`: mki ( i64 -- Result[i64 i64] ) Ok ;`) to
  get *past* the constructor-lookup gate, keeping `main`'s `Result?` eliminator call
  otherwise identical. Result: clean build, runs to exit code 0 — no separate
  "registered before the mint existed" failure surfaces once the constructor gate is out of
  the way. This confirms P7.S12's own design note (the registry is a name gate only, reading
  the operative header live off the scrutinee) already covers a check-time-only monomorph;
  `eliminator_registry`'s pre-loop construction needs no change. **This is a single fix, not
  two**: extending `env`'s constructor/accessor entries after each word's flush is sufficient.
- **Whether this also needs threading through `check_extern_decls`/`check_main_effect`
  or any other pre-loop pass that reads a snapshot of `env`.** Ruled out by inspection: both
  run once, before the per-word loop even starts (`src/check.rs:576-618`), against a
  `module.externs`/`module.words` slice that is itself fixed at that point — an `extern:`
  declaration or `main`'s own signature is parsed source text, so it can only name a type
  that already exists in the source (a parse-time monomorph or a hand-written concrete type),
  never a check-time mint that by construction doesn't exist yet. No gap here; out of scope.

## Ready to spec: NO — the mechanism above is wrong (correction 2026-08-29)

**Do not spec from the "single after-flush fix" model above.** It was probed for the *bug*,
not for the *fix*. Re-probing the proposed fix against current `main` (prototypes written and
reverted) refutes the core model and two of the "resolved" conclusions. The bug description
(a frozen call-site `env`, goldens 6/9/10 as witnesses) still stands; everything downstream
about the fix shape does not.

### 1. There is no check-time mint to project (the whole model is inverted)

At the failing `Ok` lookup (`env.get(name)` miss, `src/check/terms.rs:843`), the live
`GenericTypes` cell's `inst_enums` is **empty** (confirmed by instrumenting the miss arm).
Nothing has minted `Result[i64 i64]` yet: resolving the constructor `Ok` is *itself* what
would trigger the mint (via `apply_subst`'s `Generic` arm reading the constructor's output
header). Constructor resolution fails *before* any mint, so there is nothing for an
after-flush (or any mid-word) `env` projection to pick up. The "re-run the three
generated-sig helpers over each word's post-flush tail" fix does **not** flip golden 6:
prototyped exactly that in the per-word loop (`src/check.rs`), golden 6 still fails with
`unknown word Ok in main`.

### 2. The real fix site is `inline_combinator`, not the per-word loop, and it is three entangled gates

The gap is at the **real splice site**, `inline_combinator` (`src/check/combinators.rs`),
which computes the concrete θ (`poly_subst`, `'T = i64`) but splices the callee body against
the frozen module `env` without first grounding the combinator's declared *outputs*. Making
the `map`/`wrap` shape build end to end needs all three of:

- **(i) Splice-site constructor grounding.** In `inline_combinator`, after `poly_subst`,
  ground `sig.outputs` via `apply_subst` (minting into the live cell), then build a
  splice-scoped `env` by cloning-not-draining the cell into an id-correct temp
  `enums`/`structs` vector and projecting `struct_generated_sigs`/`enum_generated_sigs`/
  `variant_generated_sigs` (the S11-R4 move, at the real splice). Prototyped: this makes
  golden 6's `unknown word Ok` **disappear**. This half is genuinely fixable and understood.
- **(ii) Mid-walk-extensible `enums`/`structs` registries.** The splice body walk indexes
  the minted decl **by id** for `drop`/layout/`is_copy`/`tag`, but the live `enums`/`structs`
  slices it walks (immutable `&[..]` threaded through the whole `check_word`) do **not**
  contain the pending mint. Golden 10 (`Cell` struct + `drop`) **panics** at
  `src/check.rs:3168` (`ctx.structs()[id.index()]`, index out of bounds, len 0). Fixing this
  means the body walk must see decl registries that include the mint — i.e. mid-walk-
  extensible registries threaded through the entire recursive walk. This is the real,
  invasive design question, and it is exactly S11's P0-B hazard reappearing at the splice.
  (Golden 6b — the enum + `drop` variant, no eliminator — builds and runs, exit 0, only
  because enum `drop` doesn't index the *struct* table; that is why the panic hid.)
- **(iii) Eliminator grounding for a check-time-only monomorph.** Past gate (i), golden 6
  (enum + `Result?`) fails at a *later* gate:
  `a concrete body cannot eliminate it while it is ungrounded (Result)`. The "eliminator
  registry needs no change" conclusion below is **wrong**: its probe used golden 9's
  parse-time-sibling workaround (`: mki ( i64 -- Result[i64 i64] ) Ok ;`), which seeds a real
  `module.enums` monomorph and masks this gate. Without a parse-time sibling — the exact case
  this slice targets — the eliminator gate fires.

### Verdict (partially superseded — see gate (iv) addendum below)

This is a **multi-gate architectural slice** (splice-site constructor grounding +
mid-walk-extensible decl registries + eliminator grounding for a check-time-only monomorph),
not a single-fix follow-on. It needs its own **design discovery** on how to thread
mid-walk-extensible `enums`/`structs` through the splice body walk, before any spec.
Counter-probes: golden 6 (constructor gate, then eliminator gate), golden 6b (enum+drop
builds, masking the struct panic), golden 9 (parse-time sibling masks both later gates),
golden 10 (struct+drop panics at `check.rs:3168`).

The pre-loop-pass ruling below (extern/`main`-effect out of scope) still holds. The
"eliminator resolved" and "single fix, not two" conclusions below do **not**.

---

## Ready to spec: yes  *(SUPERSEDED — see correction above)*

Both open questions are resolved: the eliminator question by a live probe against current
`main` (above), and the pre-loop-pass question by inspection. The mechanism, the exact
broken site, the three-function fix shape (re-run `struct_generated_sigs`/
`enum_generated_sigs`/`variant_generated_sigs` over each word's post-flush tail and merge
into `env`), and the dedup-safety argument are all confirmed against current `main` with
concrete line references and existing goldens (6, 9, 10 in `tests/phase7_slice11.rs`) as the
regression fixtures — no new fixture needs writing to prove the bug exists, and this is a
single fix, not two. The only remaining decision (exactly where in the per-word loop the
re-projection runs, and how the pre-flush lengths are captured) is spec-level detail, not a
structural unknown.

## Gates (i)-(iii) validated fixed by prototype; a fourth gate found (2026-08-29)

A follow-on discovery pass built a prototype combining (i) splice-site output grounding in
`inline_combinator` plus a `struct_decl`/`struct_decl_or_generic` fallback (S11's existing
`enum_decl` shape, twinned for structs) threaded into the body-walk's id-indexed lookups --
gate (ii)'s fix. Measured against all four goldens (reverted afterward, nothing landed):

- **Golden 10** (struct + `drop`): now **passes**, exit 0. Previously panicked
  (`ctx.structs()[id.index()]`, out of bounds) -- confirms gate (ii)'s fix shape is
  sufficient on its own, no further registry plumbing needed for the struct/drop/layout
  path.
- **Golden 9** (parse-time sibling) and **golden 6b** (enum + `drop`, no eliminator): stay
  green, no regression.
- **Golden 6** (enum + `Result?` eliminator): progresses past gate (iii) -- with
  `scrutinee_enum_id_of_family` added (read the concrete `Type::Enum` id straight off the
  live stack slot when the registry only has a `Generic` entry, rather than trusting the
  frozen pre-loop registry classification) the "ungrounded scrutinee" error **disappears**,
  confirming gate (iii)'s fix shape too. But it now fails at a **new, later** error: unknown
  word `Ok>` in `main`.

### (iv) The eliminator's own arm bodies are checked outside the splice, against the enclosing word's env -- which the splice-local fix never touches

`~[ ( Ok ) Ok> . ]` is not part of `wrap`'s spliced body; it is written in `main`, directly
adjacent to the `wrap`/`Result?` call, and is checked as part of *`main`'s own* body walk
using *`main`'s own* `env`/`structs`/`enums` -- the ones `check_word`'s outer loop threads
in, not the splice-local clone `inline_combinator` built for grounding `wrap`'s callee body.
Gate (i)'s fix mints `Result[i64 i64]`'s accessor (`Ok>`) sigs only into that splice-local
clone, which is dropped the moment `inline_combinator` returns -- so by the time `main`'s
walk reaches the arm quotation a few terms later, the accessor is gone again. This is not a
fifth independent bug; it is the direct consequence of scoping gate (i)'s fix to the splice:
**the mint has to escape back into the enclosing word's own live `env`/`structs`/`enums` for
the remainder of that word's walk**, not just live inside the splice that triggered it. That
is functionally the original "extend `env` after a mint" idea from the first (retracted)
version of this brief -- but triggered by a splice-site grounding mid-word, not a per-word
end-of-loop flush, and it still needs gate (ii)'s mid-walk-extensible registries so *later*
terms in the same word (the arm bodies) can resolve both the accessor call and any
id-indexed operation (drop/layout) against the now-larger `structs`/`enums`.

### Updated verdict

Four entangled gates, not three: (i) splice-site output grounding, (ii) mid-walk-extensible
`structs`/`enums` for id-indexed body ops, (iii) eliminator scrutinee grounding from the live
stack type rather than the frozen registry, and (iv) propagating (i)'s mint out of the
splice into the enclosing word's own `env`/`structs`/`enums` so later terms in *that* word
(typically the eliminator's own arm bodies, always written adjacent to the call rather than
inside the combinator) see it too. (i)-(iii) are each independently prototyped and confirmed
to fix their own symptom with no regression on the other three goldens; (iv) is diagnosed
with a concrete line (arm body fails at the accessor lookup, not the constructor) but not
yet prototyped. This remains a design-discovery slice, not a spec-ready single fix -- the
shape of (iv)'s fix (how far the mint propagates, and whether it needs the same mid-walk-
extensible registry plumbing as (ii) or something narrower) is the open structural question,
not spec-level detail.

## Gate (iv) prototyped end-to-end: all four goldens pass, two open items found (2026-08-29)

A second discovery pass, in a fresh isolated worktree, rebuilt (i)-(iii) and added a
broader gate (iv): the `enum_decl_or_generic`/`struct_decl_or_generic` fallback (a live-cell
read when the id falls past the flushed `structs`/`enums` slice) was wired not just into the
id-indexed body-walk ops but into every remaining id-indexed lookup that touches a
check-time-only monomorph outside the splice -- `variant_type`'s call in the per-arm
expected-input-type computation (`src/check.rs:2454`) and `check_eliminator_call`'s own
`gate_decl`/`enum_decl` reads (`src/check.rs:2285`, `:2353`, `:2363`). A real (unrelated)
Rust borrow-check obstacle surfaced and was resolved along the way: `chosen` (an `&Overload`
borrowed from `env`) had to be cloned to an owned value before a later mutable borrow of
`poly` in the same expression, since `Overload` already derives `Clone`.

**Result: all four goldens (6, 6b, 9, 10) pass, exit 0, no reverts needed to get there.**
This is strictly stronger than the first gate-(iv) probe (which only got golden 6 to progress
past the constructor gate before failing at the accessor) -- the broader fallback wiring
closes gate (iv) for the combinator/`inline`-splice shape this slice targets.

### Two items surfaced by running the full existing suite against the prototype (not just the four target goldens)

**1. Three `phase7_slice11.rs` goldens flip from reject to accept -- expected, not a
regression.** `a_check_time_monomorphs_constructors_are_absent_from_the_call_site_env`
(golden 6's own test), its struct twin `a_check_time_struct_monomorphs_constructor_is_
absent_from_the_call_site_env` (golden 10), and `a_standalone_mint_after_an_earlier_
check_time_mint_lands_at_the_right_id` (golden 9) all assert `unknown word .. in main` and
say so explicitly in their own doc comments: golden 6's reads "pins the out-of-scope
frozen-env gap so it cannot regress silently or be quietly claimed as fixed." These three
fixtures exist specifically to pin *this* bug as current behavior pending this slice's fix --
once gates (i)-(iv) land, all three build and run to exit 0 instead of erroring, which is
the fix working, not a regression. The spec must include migrating these three fixtures
(flip the assertion from `build_error` to `build_and_run`/exit-0, update their doc comments
to describe the now-fixed behavior) alongside the implementation -- the same test-migration
discipline as any other bug-pinning golden being retired by the fix it pins.

**2. `phase7_slice12.rs`'s `concrete_body_generic_eliminator_message_does_not_fabricate_an_
instantiation` regresses for real -- gate (iii)'s fallback is too permissive.** This fixture
is a genuinely-ungrounded case with a different shape from golden 6: `wrap` is an ordinary
*non-inline* poly word (`: wrap ( 'T -- Pair['T] ) One ;`), not a combinator, so no splice
ever runs and gate (i)'s output-grounding fix never fires -- `Pair[f64]` is never minted or
registered anywhere in this program. Before the prototype, this correctly produced the
honest "`Pair?` names the generic enum `Pair`, but a concrete body cannot eliminate it
while it is ungrounded" diagnostic. With gate (iii)'s `scrutinee_enum_id_of_family`
fallback wired in, it instead produces `unknown word Ok>`/`One>` in `main` -- a confusing
later failure, not the honest one.

Mechanism: `scrutinee_enum_id_of_family` only checks that the scrutinee's *static* type is
already a concrete `Type::Enum(id, name)` matching the right family -- it does not check
that any monomorph was actually *minted* (registered into `structs`/`enums`/`env`) anywhere.
A poly call's own unification can leave a concrete-looking `Type::Enum` on the stack purely
from substituting `'T = f64` into the *type*, independent of whether anything ever grounded
that instantiation's constructors/accessors. Gate (iii) as prototyped conflates "the
scrutinee's type looks concrete" with "this instantiation was actually grounded somewhere" --
only gate (i)'s splice-site grounding guarantees the latter, and it never runs for this
fixture's shape (no combinator, no splice). The fallback needs an actual-mint check (e.g.
confirm `id` resolves to a real decl via `enum_decl_or_generic`, not just that the stack
type's tag matches), not just a type-shape match, so a genuinely-ungrounded non-combinator
call keeps getting the honest ungrounded diagnostic instead of falling through to a
confusing accessor-not-found error two terms later.

### Updated verdict, second pass

Gates (i)-(iv), taken together with the broader fallback wiring, make all four target
goldens build and run correctly with no revert needed -- confirming the four-gate model
is complete for the shape P7.S11-follow targets (a combinator/`inline`-splice constructing
and/or eliminating a generic type with no parse-time sibling monomorph). Two items remain
for the spec, both tractable and now precisely characterized rather than open-ended:
(a) migrate three `phase7_slice11.rs` goldens whose whole purpose was pinning this bug, and
(b) tighten gate (iii)'s scrutinee fallback to require an actual mint, not just a matching
static type, so the pre-existing `phase7_slice12.rs` honest-ungrounded diagnostic does not
regress for the non-combinator case. Neither is a structural unknown; both are spec-level
requirements. **Ready to spec: yes.**

## Correction: item 2 above misdiagnosed gate (iii) as permissive; it is gate (v) (2026-08-29)

Re-checked against current baseline `main` (debug-printed and reverted, nothing landed):
`instantiate_enum` for `Pair[f64]` genuinely fires when `phase7_slice12.rs`'s
`concrete_body_generic_eliminator_message_does_not_fabricate_an_instantiation` fixture's
`main` calls `wrap` -- a real monomorph is minted **mid-word**, through the *ordinary*
poly-call path (`apply_subst`'s `Generic` arm at `src/check/poly.rs:8260`), not through
`inline_combinator` at all (`wrap` here is a plain, non-`inline` poly word). Yet the
eliminator still rejects the scrutinee as "ungrounded," because `eliminator_registry`'s
classification (`Concrete`/`Generic`) is frozen at pre-loop build time (`src/check.rs:661`)
and never re-consulted once a mint happens later.

This means the "item 2" diagnosis above is **wrong**: gate (iii)'s `scrutinee_enum_id_of_
family` fallback correctly recognized this scrutinee as grounded -- it is a real,
already-minted `Type::Enum` -- so it is not being "too permissive." What actually fails is
gate (iv), and it fails because its fix was scoped to only one of *two* originating sites
for a mid-word mint:

- **(iv-a)**, already covered: a mint inside `inline_combinator`'s splice-local clone,
  escaping outward to the enclosing word once the splice returns.
- **(iv-b)**, not covered, the actual cause here: a mint from an **ordinary poly call**
  (`main` calling `wrap` directly, no splice, no combinator) inside the enclosing word's
  own body walk. `apply_subst`'s `Generic` arm mints into the live `generics_cell`
  regardless of call shape, but that mint is only flushed into `structs`/`enums`/`env` at
  **end-of-word** (`src/check.rs:1004-1006`) -- too late for sibling terms later in the
  *same* word (here, the eliminator's own arm bodies, called right after `wrap` returns).

**Gate (v): propagation must happen at both originating sites**, not just the splice.
Whatever mechanism escapes a mint out to the enclosing word's live state for (iv-a) needs
to run for any mid-word `apply_subst` mint, splice or not -- likely the same code path,
triggered from the ordinary per-term dispatch rather than only from `inline_combinator`.

**Open scope question for the spec, not a decision I can make in discovery:**
`phase7_slice12.rs`'s fixture is very likely a **fourth instance of the same bug class**
goldens 6/9/10 pin (a real check-time mint invisible to later same-word lookups), not a
legitimately-different "must stay rejected" case -- its own doc comment's claim that "a
concrete body cannot eliminate the header while it is ungrounded" is "always true" appears
to be false once a real mid-word mint like this one exists. Whether this slice's scope
extends to fixing gate (v) too (flip this fourth fixture, same as the other three) or
deliberately draws the line at combinator/`inline`-splice sites only and leaves ordinary
poly-call mid-word mints as a separate follow-on is a real design choice for the spec to
make explicitly, not an implementation detail.

**Ready to spec: yes, with this scope question named as an explicit open decision** rather
than silently resolved either way.
