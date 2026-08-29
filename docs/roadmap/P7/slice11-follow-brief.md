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
