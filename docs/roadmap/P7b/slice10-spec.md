# P7b.S10 spec — header-level export ambiguity for the third-module bare caller

> Delivery spec. Companion frozen docs: [slice10-brief](./slice10-brief.md)
> (rulings R1-R5, constraints, open questions), [slice10-paper-tests](./slice10-paper-tests.md)
> (complete fixtures + measured-before columns + unit sketches),
> [slice10-probes](./slice10-probes.md) (verbatim probe log + verdict, plus a dated
> corrections appendix from the pre-implementation review round). Read
> [slice9-spec](./slice9-spec.md) for the two-defect story and the
> R1.1a / R2.4 / R-NFR context this slice sits beside. Base: `a9eca84`
> (P7b.S9 merged to `main` at `cd44b1c`, plus this slice's own recon-round docs;
> suite 3175/0, unaffected by a docs-only commit).

## Why

S9 closed operand provenance (V2, R1.1a) and monomorphization identity (V3,
R2.1) and left one measured hole as its Phase-4 Residual: two modules `a` and
`b` each declare their own same-named generic header (`Widget['T]`) with their
own `impl: Sized for Widget` (constants 1 and 2); a third module `c` imports
both, declares **no** `Widget` header, and makes a **bare** `Widget` ctor call.
Today that call builds and **silently dispatches on whichever module happens to
spell the instantiation eagerly** (probes P1/P8: with `b` eager, `c` prints `2`;
with `a` eager, `c` prints `1`). The output of `c` is decided by internals of a
module `c`'s author may never have read — the silent, semantically load-bearing
dependence this roadmap exists to turn into sharp compile errors (CLAUDE.md).

S9's R1.1a own-header grounding has nothing to ground at (`c` declares no
header); the only `Widget[i64]` visible to `c`'s env lookup is the single
eagerly-minted instantiation, so the pre-existing single-candidate arm takes it
silently. S10 replaces that silent pick with a **located compile-time ambiguity
error**, at the single-candidate grounding fall-through, keyed on the generic
header registry **scoped to the caller's own import set** (its own imports and
selective imports, one hop — not the whole-program file closure; R1) — not on
the trait-impl matcher (R2/R-NFR2 forbid the dispatch-time machinery a naive
fix would reach for). Reachability-scoping is load-bearing, not a
simplification: a program-wide header count would flag a header some
unrelated, unimported module happens to declare, breaking single-lib programs
that never see it (GH, measured during the pre-implementation review round —
see below). A round-2 review measured two further gaps this revision closes:
the reachable-header count alone is not enough (a reachable header that never
mints its own instantiation must not license silently borrowing a
*different*, unreachable module's instantiation instead — GM), and the
explicit-resolution exemption must resolve through a re-exporting hub before
comparing, or it mismatches a program that resolves correctly today — GL. A
round-3 review then measured that reachability itself (not just exemption 4)
needs that same hub-awareness (GN), that reachability is name-independent at
the raw layer (GO), and that a `*` wildcard import must not satisfy
exemption 4 (GP) — see below.

## Adjudicated mechanism (probe round, P6 spike — verbatim in slice10-probes)

- The env is built **once, before any body is checked** (`src/check.rs:586`,
  `struct_generated_sigs(&module.structs)`), from parse-time **eager mints
  only**. A module's own-header grounding (R1.1a) mints mid-check inside its own
  word loop and never precedes another module's env build.
- So a headerless caller's bare ctor call sees a candidate list of exactly the
  eagerly-spelled instantiations — in the G4 shape, **one** (`b`'s
  `Widget[i64]@m8`), even though **both** headers are already visible in
  `module.generic_structs` at env-build time (`["Widget@m7","Widget@m8"]`).
- The silent pick happens at the single-candidate arm's grounding fall-through
  in `bare_generated_word_own_module_grounding` (`src/check/terms.rs:1507-1509`
  — the `find_struct(header_name, caller_module)` → `None` arm): caller has no
  own header, one env candidate exists, it is used unchanged (`Ok(None)`,
  falling through to the borrowed mint).
- The S5 tier policy cannot see the latent ambiguity: with one env candidate
  `select_overload`/`tier_pick` never error (lone-survivor ruling,
  `src/check/builtins.rs:118-127`); with two env candidates the existing
  accepted-ambiguity error already fires (P2), naming no modules.
- **Confirmed during the pre-implementation review round:** a *qualified* type
  spelling in a signature (`a::Widget[i64]`) parse-time-mints exactly the same
  way an unqualified one does — it is not a term-position resolution mechanism,
  it is a **second eager mint**. A fixture with `b` eager via its own signature
  and `c` writing `a::Widget[i64]` in *its own* signature reaches the
  **multi-candidate** arm (2 env candidates) and hits the pre-existing,
  unchanged 2-candidate `no_overload_matches_error` — the same error GC pins,
  not a new one, and not a cure. This is why R5's remedy note drops qualified
  spelling entirely rather than listing it as "not yet available" (OQ-3).

## Rulings

### R1 — the policy (accepted ambiguity at the single-candidate arm)

At the single-candidate arm of `bare_generated_word_own_module_grounding`,
once the sole env candidate has survived **both** the
instantiating-module-is-foreign check (`owning_module == caller_module` fails,
`terms.rs:1504`) **and** the candidate-identity check (`generated_word_entry`
plus the `key`/`symbol` match, `terms.rs:1531-1537` — see R2's guard-ordering
requirement), the grounding raises a **located compile-time ambiguity error**
at the call site — naming the surface name, the relevant declaring module(s),
and the call span, with remedy pointers — **unless one of the following four
exemptions holds**:

1. `m` declares its own header — S9's R1.1a grounds the caller's own mint (the
   caller-owns tier, untouched);
2. **[both halves required, over the fully-resolved reachable set — R2]** at
   most one same-named header is reachable, **and** the sole candidate's
   actual **declaring** module (`guard.structs[gi].module` — R2 clarifies why
   this, not the *instantiating* module, is the right component) is itself
   among that reachable set. The reachable set is `m`'s raw import set
   (`ModuleInfo.imports` ∪ `ModuleInfo.selective` target-module values, one
   hop, **regardless of which name each selective entry was keyed by** — GO,
   `selother`: `c` selectively imports only `Gadget` from `d`
   (`import: self::d | Gadget | ;`), a *different* name than the surface name
   (`Widget`) under check, yet `d` — where `Widget` is declared *and* minted
   — is still reachable, because `d` is a raw target of *some* selective
   entry of `c`'s, not because `c` selected `Widget` specifically; measured
   `4`, exit 0, unchanged) **plus, for every module in that raw set, whatever
   module the export-origin walk resolves the surface name to when started
   there** (the same walk exemption 4 uses, over the generic header registry
   — R2; GN, `hubq`: `c` plainly imports `h`, a re-exporting hub with no
   header of its own; walking `h` for `Widget` resolves to `a`, so `a` joins
   the reachable set even though `c` never imports `a` directly — one header
   exists program-wide, nothing to mis-dispatch to, measured `1`, exit 0,
   unchanged). A header some other, unimported (and un-walked-to) module
   declares does not count against the ≤1 threshold (GH, `overfire` — the
   reachability-scoping witness: two headers exist program-wide, but `app`
   imports only `lib`, never `z`; only `lib` is reachable, so this exemption
   applies even though a second header exists elsewhere, program-wide,
   measured `7`, exit 0, unchanged) — **and separately**, a reachable header
   that never mints its own instantiation does not entitle the caller to
   silently borrow a *different*, unreachable module's instantiation instead
   (GM, a case built during the round-2 review specifically to probe this
   gap: `app` imports only `lib`, which declares its own `Widget` header but
   never instantiates `Widget[i64]`; the only existing instantiation belongs
   to `z`, a module `app` never imports at all — today this silently prints
   `9`, `z`'s value, exit 0; the tightened exemption does not cover this, so
   the error fires);
3. the call reaches the **multi-candidate** arm (≥2 env candidates) — the
   existing S5 `select_overload` path governs there unchanged, including
   tier-2 pinning for declared type imports (GJ, `p5i2`), byte-identical;
4. `m` performs an **explicit resolution** for this surface name — a
   **named** selective import (the `| Widget |` spelling; a `*` wildcard's
   per-export desugaring never counts, however it happens to populate the
   same `ModuleInfo.selective` map — GP, below) names a target module for
   `Widget`, and that target, **resolved through any hub re-export chain
   first** (see R2), lands on the sole candidate's actual **declaring**
   module (GI, `sel1`: the sole-minter case; GL, `hub`, a case built during
   the round-2 review: `h` re-exports `a`'s `Widget` under its own
   `export: Widget ;`, and `c` selectively imports `Widget` from `h` — `c`'s
   raw `ModuleInfo.selective` value is `h`, not `a`; the match must resolve
   through `h`'s own re-export chain to `a` before comparing, or this
   fixture wrongly errors a program that resolves correctly today, `1`,
   exit 0). Without the named-vs-wildcard exclusion, a bare `*` wildcard
   import would silently satisfy this exemption for *every* name the target
   exports, reintroducing GK's defect class through the desugar (GP,
   `wild`: `a` and `b` both declare `Widget`; only `b` mints; `c` writes
   `import: self::a ; import: self::b * ;` and bare-calls `Widget` — today
   this silently prints `2`, exit 0, as if the wildcard had explicitly
   resolved to `b`, though `c` never named `Widget` anywhere; under this
   ruling it becomes the same located error as GA/GB).

Exemption 4's match requirement is load-bearing, not decorative:
`ModuleInfo.selective` records what the caller *asked for* (after resolving
any hub re-export), not what the caller *gets* — the two agree only when the
module the caller ultimately selected happens to be the one that eagerly
minted. When they disagree (the caller selectively imports module `X`'s
`Widget`, but the sole existing instantiation belongs to a different
reachable module `Y`), the exemption must **not** apply: silently handing the
caller `Y`'s value while they asked for `X`'s is the exact V2-class defect
S10 exists to close, only relabeled behind a selective import the caller
believes already disambiguated it. Measured today (GK, a case built during
the pre-implementation review round specifically to probe this gap): `c`
selectively imports `a`'s `Widget` (`import: self::a | Widget | ;`) while
only `b` ever eagerly mints — today this silently prints `2` (`b`'s value,
contradicting the caller's own selection), exit 0. Under this ruling it
becomes the **same** located error as GA/GB, naming both reachable modules
(GK is curable — see R5's two-part remedy 2). **A blanket "selective import
is present ⇒ exempt" reading, without the match-against-the-declaring-module
test and without resolving hub re-exports first, is unsound and must not be
implemented** — it would leave exactly the silent mis-dispatch this slice
exists to close, merely hidden behind a selective import that looks like it
resolved something.

### R2 — where the check lives, guard ordering, and what data it reads

**Guard ordering (critical, not merely a location anchor).** The new check
must run only after the sole env candidate has survived, in order: (a) the
`owning_module == caller_module` check (`terms.rs:1504` — the candidate is
genuinely foreign), and (b) the candidate-identity check
(`generated_word_entry(ctx, id, destructure)` plus the `key != name ||
symbol != only.symbol` comparison, `terms.rs:1531-1537` — confirms the sole
candidate really *is* this header's own generated ctor/destructure word, not
an ordinary user word whose output merely happens to be another module's
instantiation). Today's code returns `Ok(None)` immediately at the
`find_struct(header_name, caller_module)` → `None` fall-through
(`terms.rs:1507-1509`, when the caller has no own header) — *before* it ever
reaches the candidate-identity check at 1531-1537, since that check currently
sits inside the same block, gated behind `own_idx` being found. **The
implementing phase restructures this control flow** so a headerless caller's
call still passes through the candidate-identity check before the new
ambiguity check can run; `1507-1509` remains the right *semantic* anchor
(headerless caller, foreign single candidate) but is not, as written today,
the code location an emitted error can sit at without first surviving
1531-1537 — emitting there directly would newly reject the "ordinary user
word whose output happens to be another module's instantiation" shape
1531-1537 exists to protect. `terms.rs:1492-1497`'s "the order is free"
comment describes a premise (every early exit here is an interchangeable
`Ok(None)`) that stops holding the moment one of these arms can instead
return `Err(...)`; the implementing phase updates that comment.

**Declaring module vs. instantiating module (critical, not interchangeable).**
`struct_instantiation_of(id)` returns `(gi, owning_module, args, lens)` where
`owning_module` is the candidate's *instantiating* module — "the third
component of `struct_keys`, captured at the naming site" (`ast.rs:2598`'s
`Generic::module` doc comment) — the module whose file wrote the concrete
`Widget[i64]` application, which can differ from the module that declared
`type: Widget['T]` in the first place (a qualified spelling in a *foreign*
module's own signature instantiates a *foreign* header — see the
qualified-spelling finding in the probes corrections appendix). The header's
true **declaring** module is `guard.structs[gi].module`
(`GenericStructDecl.module`). R1's exemption 2 and exemption 4 both compare
against the **declaring** module, not `owning_module`. This distinction
changes no *current* golden's outcome — **not** because declaring and
instantiating always coincide (they do **not**: in `p5g2` (GE) `c` mints
`a`'s header via its own `: mk ( i64 -- Widget[i64] ) Widget ;`, and in
`p7c3` (GF) `c` mints it via its own `: try ( a::Widget[i64] -- i64 )
size ;` — both declaring `a`, instantiating `c`) — but because wherever
exemption 2/4's logic actually *runs* (i.e. wherever the candidate survives
the `:1504` foreign check at all), the two happen to coincide across every
current golden. GE and GF are exactly the fixtures where they diverge, and
there the divergence is moot: `c` is the *instantiating* module for its own
mint, so `owning_module == caller_module` and the call exits at `:1504`
before ever reaching exemption 2 or 4 — see GE/GF's corrected mechanism in
the goldens table, below. The distinction is a correctness requirement for a
shape no golden happens to exercise the *foreign*-instantiation side of
(one module instantiating a *different* module's header, from outside
either of them), and it matters directly for GL/GN below, where the raw
selective target (or raw reachable module) is neither the instantiating nor
(yet) the declaring module.

**The export-origin walk: resolving a hub re-export, for exemption 4 (GL)
and for reachability (GN) alike.** A caller's raw `ModuleInfo.selective`
value for a name is the import statement's own *target* module — which may
itself be a re-exporting hub with no `type:` header of its own (GL, `hub`:
`h.sth` re-exports `a`'s `Widget` via `import: self::a | Widget | ; export:
Widget ;`, and `c` writes `import: self::h | Widget | ;` — `c`'s raw
selective value for `Widget` is `h`, not `a`). Comparing this raw value
directly against the sole candidate's declaring module (`a`) would wrongly
mismatch and error a program that resolves correctly today.

Generic headers are **not** in the concrete type-export registries
`resolve_type_export_origins`/`walk_type_export_origin` (`src/driver.rs:364`/
`:407`) walk: `parser.rs:81-83` deliberately excludes a generic `type:`
header from the concrete struct/enum scan (`if header_is_generic(...) {
continue; }` — it lives in `module.generic_structs`/`generic_enums`
instead), and `resolve_type_export_origins`'s own `declared_types` builder
(`driver.rs:373-384`) iterates only `StructDecl`/`EnumDecl`. So
`walk_type_export_origin(h, "Widget", ...)` returns `None` for *every*
module in a fixture like GL or GN — confirmed by measurement (`hubtype`, a
case built during the round-3 review specifically to probe this: `d.sth`
writes `idw ( Widget[i64] -- Widget[i64] )`, a *type-position* reference to
`Widget` reached only through `h`'s re-export, exactly the shape the
existing walk is supposed to cover for a concrete type — it errors
`unknown type \`Widget\`` today, because the existing walk cannot see a
generic header either). The existing walk is **not** reusable as-is, and
the earlier claim that S10 "reuses the same hub-hop the checker already
performs for a type reference in an effect signature" is corrected: the
implementing phase builds a walk that is **structurally the same walk the
checker runs for concrete type names, but over the generic header
registry** — same chase logic (follow `selective`/`imports` until landing on
a module that declares the name, or `None` for a cycle or dead end), applied
to `ctx.generics().structs`' name/module pairs instead of `StructDecl`/
`EnumDecl`. Its ingredients (`ctx.generics().structs` and `ctx.modules`'s
`selective`/`imports` maps) are already reachable from `Ctx`, so this is
still no new registry, just a new (small) function — analogous to, not a
call into, `walk_type_export_origin`. **The already-computed `type_origin`
table (`driver.rs:651`) is not a usable alternative and must not be
threaded through instead** — it is built from the same concrete-only
`declared_types`, so it never has an entry for a generic header reached
through a hub (confirmed by `hubtype`'s measured failure); an implementer
choosing it would find no resolution for GL and fail that golden.

**The walk's `None` arm.** When the walk cannot resolve the name from a
given starting module (a cycle, or a dead end — the loop body at
`driver.rs:411-427`), that starting module contributes nothing further:
exemption 4 does not apply on its account (the general rule decides,
possibly via a different exemption or a different reachable module), and it
adds no module to reachability's walk-extension (below) either.

**Reachability's walk-extension (GN, R1 exemption 2).** The *same*
generic-header walk, run from every module in `m`'s raw reachable set (see
"Everything else", below) for the surface name under check, extends that
set: whatever module the walk resolves to (if any) joins it too. GN,
`hubq`: `c` plainly imports `h` (no selective clause at all —
`ModuleInfo.selective` is empty for `c`); `h` has no header of its own, but
walking `h` for `Widget` resolves to `a` (`h`'s own selective map, populated
by `h`'s `import: self::a | Widget | ;`, names `a`, and `a` declares
`Widget` as a generic header) — so `a` joins `c`'s reachable set even
though `c` never imports `a` directly. Only one header exists in this
fixture program-wide (`a`'s), so there was never anything to mis-dispatch
to; the walk-extension is what lets exemption 2 see that, rather than
wrongly treating `a` (the sole candidate's declaring module) as unreachable
and erroring — measured `1`, exit 0, unchanged, both before and after.

**Wildcard-desugared entries do not satisfy exemption 4 (GP).**
`ModuleInfo.selective: HashMap<String, u32>` is populated identically by a
NAMED selective import (`import: self::a | Widget | ;`) and by a `*`
wildcard's per-export desugar (`driver.rs:568-600`: a real-target wildcard
synthesizes a `selective_map` entry for every one of the target's exported
names) — the raw map cannot tell the two apart. The precedent this slice
already follows for module naming (R4) already makes this distinction
elsewhere: `declarations.rs:972-976`/`:989` render a NAMED selective import
as `` selective import of `{name}` from module `{q}` `` and a wildcard
desugar as `` wildcard import of `{name}` ``, keyed on `SelectiveName`'s own
`qualifier: Option<String>` field (`Some` for named, `None` for wildcard,
`driver.rs`'s import-assembly pass) — computed once, at assembly time, but
**not** threaded into `ModuleInfo`/`Ctx`; only the flattened, qualifier-blind
`HashMap<String, u32>` survives to check time. Exemption 4 requires a
**named** selective import specifically; the implementing phase threads the
already-computed `SelectiveName.qualifier` distinction (or an equivalent
per-entry flag) through to `Ctx`, since it cannot be recovered from
`ModuleInfo.selective` alone. Without this, a bare `*` wildcard import would
silently satisfy exemption 4 for every name the target module exports,
reintroducing GK's defect class through the desugar (GP, `wild`: `a` and
`b` both declare `Widget`; only `b` mints; `c` writes `import: self::a ;
import: self::b * ;` and bare-calls `Widget` — today this silently prints
`2`, exit 0, as if the wildcard had explicitly resolved to `b`, though `c`
never named `Widget` anywhere; under this ruling it becomes the same
located error as GA/GB). This exclusion is scoped to exemption 4 only — a
wildcard-reached module still counts toward **reachability** (exemption 2;
`b` is one of GP's two reachable headers either way, which is why GP
reaches the general rule and errors rather than falling through exemption
2's ≤1-reachable-header case).

**Naming a module the caller has no qualifier for (GM).** When the sole
candidate's declaring module is not in `m`'s own `ModuleInfo.imports` at all
(not even via a bound qualifier — GM's shape: `app` never imports `z` in any
form), there is no qualifier to render it by. This is not a new problem S10
invents: the existing `drop`-visibility diagnostic
(`word_families.rs:1319-1322`'s qualifier lookup, `.find(|(_, &target)|
target == decl.module)`) hits exactly this gap already and falls back to a
structural phrasing that names no module at all ("a module this module never
imports directly", `word_families.rs`'s `None` arm, ~line 1334). S10's
message follows the same precedent: when the sole candidate's declaring
module has no caller-side qualifier, the message describes it structurally
rather than fabricating a name (R5).

**Everything else.** Reading: the caller's span and module (already in
scope); the foreign candidate's `owning_module` (already computed by
`struct_instantiation_of`, line 1501); header provenance from
`ctx.generics().structs` (`GenericStructDecl.module`, `.name`) — the full,
whole-program-declared header list; the caller's own import data from
`ctx.modules` (`Option<&[ModuleInfo]>`, `engine.rs:1133`; `Some` on the
whole-program build path, `None` off it — e.g. a retained poly word), indexed
by `caller_module`. **Reachability (exemption 2) reads both**
`ModuleInfo.imports` (qualifier → target module id, `ast.rs:176`) **and**
`ModuleInfo.selective` (bare name → target module id, `ast.rs:185`) **target
values, unioned, regardless of which name each selective entry was keyed
by** (GO), plus the walk-extension above (GN) — a single, authoritative
definition; R1/brief/paper-tests all read the same way. **Exemption 4's
match test** reads `ModuleInfo.selective` too, but filtered to named entries
only (a wildcard's per-export desugar populates the same map — excluded,
above), then hub-walked (also above).

When `ctx.modules` is `None`, the check does not fire — the same "reads it and
never fires when it is absent" discipline the existing D1 drop gate follows
(`engine.rs:1133`'s doc comment: "the gate reads it and never fires when it is
absent"), since there is no import set to test reachability against.

**NOT** at `select_overload`/`tier_pick` (cannot see 1-candidate shapes; the
lone-survivor ruling deliberately never errors on one candidate). **NOT** at
env build (`src/check.rs:586`; no call site to locate an error at). **NOT** in
the matcher (`find_bound_impl` / `match_impl_target` — S9's R-NFR2 carried
over: the dispatched `size` member call keeps routing through
`resolve_mono_member_call` → `find_bound_impl` exactly as today whenever
grounding succeeds).

### R3 — diagnostics

A **NEW** located message in house style (`error: \`Widget\` in \`try\` (line N,
col M) is ambiguous: ...` + a `note:` remedy line). It names the surface name,
the call site, and the relevant declaring module(s) using the caller's own
import qualifier for each — except when the sole candidate's declaring module
has no caller-side qualifier at all (GM), where it is named structurally
instead (R2's `drop`-diagnostic precedent) — and points at the remedies that
actually cure the shape (R5). The existing 2-candidate `no_overload_matches_error`
text is **not churned** (diagnostics are behaviour; GC pins it byte-unchanged).

### R4 — module naming and deterministic ordering

No canonical, caller-independent module-name registry exists anywhere in the
checker (`ModuleInfo` carries no `name` field, `ast.rs:175-189`) — every
existing diagnostic that names a foreign module renders the **caller's own
qualifier** for it (`declarations.rs:974`, `:988`, `word_families.rs:1336` are
the precedent: `` module `{qualifier}` `` from the caller's own `import:`
binding). S10 follows the same precedent: for each reachable declaring module,
print the qualifier `m`'s own `ModuleInfo.imports` binds to it. A bare
`import: self::a ;` binds qualifier `a` by default; no renaming/aliasing
syntax exists for `self::` imports (confirmed: no `as`-style grammar in the
parser). If `m` reaches a declaring module only through a wildcard import (no
bound qualifier — `ModuleInfo.imports` has no entry for it, only
`ModuleInfo.selective` does, per-export), fall back to the existing
wildcard-import phrasing precedent (`declarations.rs:989`, "its
wildcard-imported module"). No fixture in this spec exercises the
both-declaring-modules-wildcard-only sub-case, so a distinguishable phrasing
for 2+ unqualified modules is left to the implementing phase — flagged, not
blocking; no golden requires it. A *third* case — `m` has no qualifier for a
module at all, not even a wildcard-bound one, because `m` never imports it in
any form (GM) — is not this same fallback; R2/R3/R5 specify it separately,
following the `drop`-diagnostic's own precedent for exactly this gap
(`word_families.rs:1319-1340`).

Determinism (REQ-5) follows directly: the message **sorts the collected
qualifier strings lexicographically** before joining them, rather than using
registry order (import-order-dependent — `p1-a-b` vs `p1-b-a` reach the
fall-through with the identical reachable set `{a, b}` regardless of which
import statement came first, so a lexicographic sort names them `a` then `b`
in both orders) or mint order (minter-placement-dependent — the exact
dependence GB exists to falsify).

### R5 — the exact wording (contract; the golden pins the measured bytes)

The new message follows the measure-then-pin discipline: this spec fixes the
wording **contract** below; the implementing phase measures the real rendered
output and pins whatever the formatter actually emits (spacing, the
parenthetical `(line N, col M)` shape, and how module names are joined all
follow the existing `terms.rs` diagnostic helpers). Draft contract (GA/GB/GK's
shape — ≥2 reachable declaring headers, none resolved):

```
error: `Widget` in `try` (line 3, col 22) is ambiguous: declared in modules `a` and `b`, and `try`'s module declares no `Widget`
  note: declare your own `Widget` header and impl, or selectively import the module whose `Widget` you want -- if that module does not itself instantiate `Widget[i64]`, also spell the type in your own word's signature (`import: self::a | Widget | ;` then `: mk ( i64 -- Widget[i64] ) Widget ;`)
```

Draft contract (GM's shape — a reach failure, not an ambiguity: one
reachable header, one candidate, but they are different modules and the
candidate's declaring module is unreachable, no caller-side qualifier —
R2/R3):

```
error: `Widget` in `try` (line 3, col N) is unresolved: the only `Widget[i64]` instantiation in scope is declared in a module `try`'s module does not import
  note: import the module that declares the instantiation you want, or declare and instantiate your own `Widget` header
```

Contract, byte-exact wording deferred to the golden:

- lead line: surface name in backticks, the enclosing word in backticks, the
  `(line N, col M)` call site, the word "ambiguous", and the declaring module
  names, lexicographically sorted (R4) — or, when the sole candidate's
  declaring module has no caller-side qualifier at all (GM), the structural
  phrasing R2/R3 describe instead of a fabricated name;
- note line: the **two-part** remedy 2 is the round-2 correction — (1)
  declare your own `Widget` header and `impl` (this alone cures); (2)
  selectively import the module whose instantiation you want, which cures
  **alone** only when the named module either is the sole eager minter (GI,
  `sel1`) or both modules mint and tier-2 pins your selection at the
  multi-candidate arm (GJ, `p5i2`, exemption 3, untouched). When neither
  holds — the selected module is reachable but has never itself minted the
  concrete instantiation (GK's shape) — selective import alone does **not**
  cure (GK still errors); the caller must **additionally** write an
  intermediate word whose own signature spells the concrete type (e.g.
  `: mk ( i64 -- Widget[i64] ) Widget ;`) and call that instead of
  constructing bare inline. This makes the caller's own module the
  *instantiating* module (`owning_module == caller_module` at
  `terms.rs:1504`), sidestepping the shared-env borrow entirely rather than
  merely disambiguating within it — measured to cure GK's exact shape
  (`rem2sig`, a case built during the round-2 review: `c` selectively
  imports `a`'s `Widget` and writes `mk`'s signature explicitly; prints `1`,
  exit 0). Qualified type spelling (`a::Widget[i64]`) is **dropped** from the
  note entirely (OQ-3, revised from "not yet available" to simply omitted):
  it is a type-position tool only — no term-position ctor syntax exists
  (`a::Widget` is an unknown word, P7b2) — and using it in a signature to try
  to pin an operand's type just parse-time-mints a *second* candidate,
  routing into the pre-existing, unchanged 2-candidate error (GC's shape)
  rather than resolving anything (confirmed by measurement, not a cure).

## Requirements

- **REQ-1 (policy, R1).** At the single-candidate grounding arm, once the
  sole foreign candidate has survived the ordering in REQ-2, a bare
  ctor/destructure call in module `m` fires the located ambiguity error
  **unless** one of the four exemptions holds (own header; ≤1 reachable
  header **and** the sole candidate's declaring module itself reachable,
  over the fully-resolved reachable set (raw imports ∪ selective targets,
  name-independent — GO, **plus** the export-origin walk-extension — GN);
  multi-candidate arm; matching explicit resolution — a **named** selective
  import only, never a wildcard desugar (GP) — resolved through any hub
  chain) — exemption 4 compares against the sole candidate's **declaring**
  module (R2), never its instantiating module, and never the raw,
  un-walked selective-import target. Traces to GA, GB, GD, GH, GC, GI, GJ,
  GK, GL, GM, GN, GO, GP; units
  `ambiguous_foreign_headers_grounding_is_located_error`,
  `single_foreign_header_grounding_still_borrows`, `own_header_still_grounded_first`,
  `reachable_header_count_excludes_unimported_declaring_modules`,
  `matching_selective_import_exempts_the_ambiguity_check`,
  `mismatched_selective_import_does_not_exempt_the_ambiguity_check`,
  `hub_reexported_selective_import_resolves_through_origin_walk`,
  `unreachable_declaring_module_of_the_sole_candidate_is_still_an_error`,
  `reachable_set_includes_selective_targets_regardless_of_selected_name`,
  `reachable_set_extends_through_reexport_origin_walk`,
  `wildcard_desugar_is_not_explicit_resolution`.
- **REQ-2 (layer and ordering, R2).** The check lives at the
  `terms.rs:1507-1509` grounding fall-through (semantic anchor), but only
  fires once the candidate has survived, in order, the
  `owning_module == caller_module` check (`:1504`) and the candidate-identity
  check (`generated_word_entry` + `key`/`symbol` match, `:1531-1537`) — the
  implementing phase restructures the function's control flow so a
  headerless caller reaches the candidate-identity check before this new
  arm, rather than exiting at `:1507-1509` before ever reaching it; the
  `:1492-1497` "the order is free" comment is updated to match. Reads header
  provenance from `ctx.generics().structs` (using `GenericStructDecl.module`
  as the **declaring** module, never `struct_instantiation_of`'s
  *instantiating*-module component) and the caller's import data from
  `ctx.modules` (`ModuleInfo.imports` ∪ `.selective` target values, unioned
  and name-independent, for reachability; `.selective` filtered to **named**
  entries only, excluding wildcard desugars, for exemption 4). Both
  exemption 4's match and reachability's walk-extension resolve through a
  hub-chain walk built **over the generic header registry**
  (`ctx.generics().structs`) — structurally the same walk the checker runs
  for concrete type names (`walk_type_export_origin`, `driver.rs:407`), but
  **not** that function itself (generic headers are absent from its
  `declared_types`, so it always returns `None` for them — confirmed by
  `hubtype`'s measured failure; the already-computed `type_origin` table,
  `driver.rs:651`, is for the same reason **not** a usable alternative and
  must not be threaded through instead). When the walk cannot resolve
  (cycle/dead end, `driver.rs:411-427`'s loop shape), exemption 4 does not
  apply and the walk-extension adds nothing — the general rule decides.
  Excluding wildcard desugars from exemption 4 requires threading the
  already-computed `SelectiveName.qualifier` distinction (`Some`/`None`,
  computed at assembly time but not currently threaded past
  `check_selective_imports`) through to `Ctx`, since `ModuleInfo.selective`'s
  flattened map cannot recover it. No edit to `select_overload`,
  `tier_pick`, env build (`check.rs:586`), or the matcher
  (`find_bound_impl`/`match_impl_target`). When `ctx.modules` is `None`, the
  check never fires. Traces to REQ-7 guardrails; units
  `own_header_still_grounded_first`,
  `hub_reexported_selective_import_resolves_through_origin_walk`,
  `reachable_set_extends_through_reexport_origin_walk`,
  `wildcard_desugar_is_not_explicit_resolution`.
- **REQ-3 (diagnostic, R3/R5).** A new located message naming surface name,
  the relevant declaring module(s), and call site (or, when the sole
  candidate's declaring module has no caller-side qualifier at all, the
  structural phrasing R2 describes — GM), with the two-part remedy note; the
  existing 2-candidate `no_overload_matches_error` text is byte-unchanged.
  Traces to GA, GB, GC, GK, GM; unit
  `ambiguous_header_error_names_declaring_modules`.
- **REQ-4 (compat pins).** GD/GE/GF/GG/GH/GI/GJ/GL/GN/GO byte-identical to
  today; GC byte-identical; P5i2 tier-2 selective-import pinning untouched
  (not re-errored); GE/GF pin the `:1504` own-instantiating-module arm, not
  exemption 2 (R2, corrected mechanism). Traces to
  GC/GD/GE/GF/GG/GH/GI/GJ/GL/GN/GO.
- **REQ-5 (determinism and naming, R4).** GA/GB/GK error **identically**
  (byte-exact) regardless of which module is the eager minter and regardless
  of import order — declaring modules are named by the caller's own import
  qualifier (R4) and joined in lexicographic order, not registry/import/mint
  order. No run-count ratio asserted (R-NFR3). Traces to GA, GB, GK.
- **REQ-6 (S9 goldens: one inverts, the rest untouched).** S9's
  `third_module_bare_caller_dispatches_the_single_shared_env_instantiation`
  (`tests/phase7b_slice9.rs`) is the **exact inverse** of GA/GB — it currently
  asserts the silent `2`/`1` outputs this slice replaces with a located error.
  This golden is **retired or rewritten** in Phase 1 (not merely "untouched");
  every other S9 golden — G1/G1a–G1f, G2, G2r, G3, plus #10 and S5 tier-1 —
  stays green and byte-unchanged, since the new check sits beside R1.1a's
  grounding and only fires where R1.1a falls through today, on the specific
  shape S9 documented as its Residual.
- **REQ-7 (guardrails).** R-NFR1 (no IR/lowering edits — and the S9
  `ir/layout.rs` exception does **not** carry into S10; S10 is check-stage
  only), R-NFR2 (matcher untouched), R-NFR3 (no ratio assertions on the new
  goldens). Diagnostics are behaviour: only the NEW message is new; no
  existing diagnostic text churns.
- **REQ-8 (roadmap + growth + gate).** The roadmap's S9 entry has **two**
  stale sentences that both go false after S10 lands —
  (`docs/roadmap/P7b-higher-kinded-types.md:241-244`) the dispatch-mechanism
  sentence ("A third-module bare caller ... dispatches deterministically on
  the single instantiation minted into the shared whole-program env") and
  (`:244-247`) the trailing "Residual: ... future work" sentence — **both**
  are updated (not merely the Residual sentence) to state the shape is
  closed by P7b.S10; a new S10 entry in current-design prose (no history
  narration), recording that S10 is implemented **in parallel with P7b.S6**
  (maintainer ruling, 2026-09-05; both branch from base `a9eca84`) rather
  than strictly before or after it, with the landing rule: if S6's own
  probes demand a check-stage grounding change in `terms.rs`, or the two
  slices' changes interact at merge time, S10's checker change lands first
  and S6 rebases onto it. Growth signals (R6) re-run on every touched file
  at phase exit. Final full gate ×2.

## Goldens

All new goldens in `tests/phase7b_slice10.rs`. Complete fixture text lives in
[slice10-paper-tests](./slice10-paper-tests.md); the error goldens pin
byte-exact text once the implementing phase measures the rendered output
(measure-then-pin).

| Golden | Test name | Behaviour | Fixture |
| --- | --- | --- | --- |
| GA | `third_module_bare_caller_with_ambiguous_headers_is_a_located_error` | before: builds, prints `2`, exit 0 (both import orders — this is S9's own `third_module_bare_caller_dispatches_the_single_shared_env_instantiation`, retired by REQ-6) → after: located ambiguity error naming `Widget`, modules `a`/`b` (lexicographic), `c`'s call site; exit 1; **identical text for both `p1-a-b` and `p1-b-a`** (import-order independence, R4) | `p1-a-b`, `p1-b-a` |
| GB | `third_module_bare_caller_error_is_independent_of_the_eager_minter` | before: prints `1`, exit 0 (`a` is the sole eager minter — the other half of S9's G4) → after: the **same** error text as GA (modules named `a`/`b`, lexicographic — independent of which module minted), exit 1 | `p8-a-eager` |
| GC | `both_modules_eager_2_candidate_ambiguity_error_unchanged` | `error: no overload of \`Widget\` in \`try\` (line 3) accepts these operands` + two `candidate: \`i64\`` lines, exit 1 — **byte-identical** | `p2-both-eager` |
| GD | `single_declaring_header_bare_caller_still_resolves` | prints `7`, exit 0 — **unchanged** (one header program-wide, no export-gate issue — `lib.sth`'s `usesize` is private) | `p3a-single-lib-private` |
| GE | `single_reachable_header_with_selective_import_still_resolves` | prints `1`, exit 0 — **unchanged**. `c`'s own `: mk ( i64 -- Widget[i64] ) Widget ;` mints `Widget[i64]` *in `c` itself* — `c` is the instantiating module, so the call exits at the `:1504` `owning_module == caller_module` check, before exemption 2 (or any exemption) is ever consulted; this golden pins the `:1504` arm, not the reachability logic (corrected from an earlier, wrong attribution to "exemption 2, ≤1 reachable header" — the selective import is present but plays no role in *this* mechanism either; renamed from the original `selective_type_import_bare_ctor_pins_exporters_impl`) | `p5g2-selective-type` |
| GF | `single_reachable_header_with_qualified_signature_still_resolves` | prints `1`, exit 0 — **unchanged**, the same `:1504` mechanism as GE: `c`'s own `: try ( a::Widget[i64] -- i64 ) size ;` mints `Widget[i64]` *in `c` itself* (declaring module `a`, instantiating module `c`), so `owning_module == caller_module` and the call exits at `:1504` before any exemption runs (corrected from an earlier, wrong attribution to "no second header exists"; renamed from `qualified_type_spelling_bare_ctor_pins_exporters_impl`) | `p7c3-qualified-type` |
| GG | `unimported_foreign_type_annotation_is_still_an_error` | `error: unknown type \`Widget\` at line 3, col 9`, exit 1 — **unchanged** (type-position rule; S10 governs term-position only) | `p4-c-annotates` |
| GH | `unimported_declaring_module_does_not_count_toward_ambiguity` | prints `7`, exit 0 — **unchanged, both before and after**. Two headers exist program-wide (`lib`, `z`); `app` imports only `lib`; only one header is reachable from `app` (and the walk-extension adds nothing — `lib` declares `Widget` directly, no re-export chain to follow), so exemption 2 fires even though a second header exists elsewhere, program-wide — the fixture that justifies reachability-scoping the count (R1) rather than a program-wide one | `overfire` |
| GN | `hub_reexport_reachable_through_plain_import_still_resolves` | prints `1`, exit 0 — **unchanged, both before and after**. `a` declares, mints, and exports `Widget`; `h` re-exports it with no header of its own; `c` writes a *plain* `import: self::h ;` (no selective clause at all) and bare-calls `Widget`. `c`'s raw reachable set is `{h}`; walking `h` for `Widget` resolves to `a` (`h`'s own selective map), so `a` joins the reachable set even though `c` never imports it directly — one header exists program-wide, nothing to mis-dispatch to. Built during the round-3 review to probe whether reachability's walk-extension (not just exemption 4's) needs the hub chain | `hubq` (new fixture, see paper-tests) |
| GO | `selective_import_of_different_name_still_grants_reachability` | prints `4`, exit 0 — **unchanged, both before and after**. `d` declares and mints `Widget`, and separately declares `Gadget`; `c` selectively imports only `Gadget` from `d` (`import: self::d | Gadget | ;` — a name *other than* the surface name under check) and bare-calls `Widget`. `d` is still reachable: the raw reachable set is `imports ∪ selective` *target-module* values, name-independent — `d` is a value in `c`'s selective map regardless of which name selected it. Built during the round-3 review to probe (and reject) an imports-only reading of reachability that a drafting fork in R2/brief had introduced | `selother` (new fixture, see paper-tests) |
| GI | `matching_selective_import_of_the_sole_minter_still_resolves` | prints `1`, exit 0 — **unchanged, both before and after**. Two reachable headers (`a`, `b`); only `a` mints; `c` selectively imports `a`'s `Widget`, matching the sole candidate's declaring module — exemption 4 fires | `sel1` |
| GJ | `selective_import_pins_the_named_exporter_when_both_mint` | prints `1`, exit 0 — **unchanged, both before and after**. Both `a` and `b` mint (multi-candidate arm); existing S5 tier-2 pinning selects `a` — exemption 3, untouched by S10 | `p5i2-selective-one-of-two` |
| GK | `mismatched_selective_import_is_still_a_located_error` | before: silently prints `2`, exit 0 (`c` selectively imports `a`'s `Widget`, but `b` is the sole eager minter — the caller's own selection is silently overridden) → after: the **same** located error as GA (naming modules `a`/`b`), exit 1 — exemption 4 does not apply since the selected module does not match the sole actual candidate's declaring module. Built during the pre-implementation review round specifically to probe exemption 4's soundness; without the match requirement this case would stay silently wrong. Curable (unlike a naive reading might suggest) — see R5's two-part remedy 2 and `rem2sig` | `p9-mismatched-selective` (new fixture, see paper-tests) |
| GL | `hub_reexported_selective_import_still_resolves` | prints `1`, exit 0 — **unchanged, both before and after**. `h` re-exports `a`'s `Widget` (`import: self::a | Widget | ; export: Widget ;`, no header of its own); `c` selectively imports `Widget` from `h`, plus plain imports of `a` and `b`; only `a` mints. `c`'s raw `ModuleInfo.selective` value for `Widget` is `h`, not `a` — exemption 4 fires only because the match is resolved through `h`'s re-export chain to `a` first (R2); built during the round-2 review specifically to probe this gap | `hub` (new fixture, see paper-tests) |
| GM | `unreachable_minter_bare_call_is_a_located_error` | before: silently prints `9` (`z`'s impl), exit 0 (`app` imports only `lib`, which declares its own `Widget` header but never instantiates `Widget[i64]`; the sole existing instantiation belongs to `z`, a module `app` never imports at all) → after: a located, un-ambiguous *reach-failure* error (R5's reworded contract — one candidate exists, but its declaring module is unreachable, not "multiple candidates competing") — exemption 2's tightened form requires the sole candidate's declaring module to itself be reachable, which fails here (`z` is unreachable, and the walk-extension adds nothing: `lib` declares `Widget` directly, no chain leads to `z`) even though only one header (`lib`'s) is reachable; `app` has no qualifier for `z` at all, so the message names it structurally rather than fabricating a name (R2/R3, the `drop`-diagnostic precedent). Built during the round-2 review specifically to probe this gap | `under` (new fixture, see paper-tests) |
| GP | `wildcard_import_does_not_exempt_the_ambiguity_check` | before: silently prints `2`, exit 0 (`a` and `b` both declare `Widget`; only `b` mints; `c` writes `import: self::a ; import: self::b * ;` and bare-calls `Widget` — the wildcard's per-export desugar puts `Widget` in `c`'s `ModuleInfo.selective` as if `c` had named it, though `c` never wrote `Widget` anywhere) → after: the same located error as GA/GB (naming modules `a`/`b`) — exemption 4 requires a **named** selective import; a wildcard-desugared entry never counts, however identically it populates the same map (R2). Built during the round-3 review specifically to probe whether the round-2 predicate's data source (raw `ModuleInfo.selective`) could distinguish the two — it cannot, without additional plumbing | `wild` (new fixture, see paper-tests) |

## Units (beside the changed code)

- `ambiguous_foreign_headers_grounding_is_located_error` — headerless caller,
  ≥2 same-named headers from reachable distinct modules, single env candidate
  ⇒ the grounding path errors (never falls through to the borrowed mint).
- `single_foreign_header_grounding_still_borrows` — same shape but at most one
  same-named header *reachable* ⇒ current borrow behaviour, covering both "no
  second header exists anywhere" (GD) and "a second header exists but is not
  reachable" (GH's mechanism at unit level).
- `own_header_still_grounded_first` — caller declares its own header ⇒ R1.1a
  grounding, no ambiguity (the caller-owns exemption).
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
  unit level — the soundness-critical case).
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
- `reachable_set_includes_selective_targets_regardless_of_selected_name` —
  a module that is the target of a selective import for a name *other than*
  the surface name under check is still part of the reachable set ⇒
  reachability is name-independent at the raw layer (GO's mechanism at unit
  level).
- `reachable_set_extends_through_reexport_origin_walk` — a raw-reachable
  module with no header of its own, whose own selective/import maps chase
  through to a module that does declare the surface name, extends the
  reachable set to include that declaring module too ⇒ exemption 2 sees a
  header behind a hub the raw layer alone would miss (GN's mechanism at
  unit level).
- `wildcard_desugar_is_not_explicit_resolution` — a `selective` entry
  populated by a `*` wildcard's per-export desugar does not satisfy
  exemption 4, even though it is indistinguishable from a named selective
  import in the raw `ModuleInfo.selective` map ⇒ the match test requires the
  richer, assembly-time `SelectiveName.qualifier` signal (GP's mechanism at
  unit level — the soundness-critical case for this round).

## Guardrails (carried from S9, verbatim in intent)

- **R-NFR1** — no IR/lowering edits. S10 is **check-stage only**; S9's sole
  sanctioned `ir/layout.rs` exception (its duplicated-name layout keys) does
  **not** carry into S10. Any candidate fix needing an IR/lowering edit stops and
  escalates.
- **R-NFR2** — `match_impl_target`/`..._rec` and `find_bound_impl`'s scan have
  zero behavioural diff; the dispatched `size` member call routes exactly as
  today whenever grounding succeeds.
- **R-NFR3** — no run-count / ratio assertions on the new goldens; determinism is
  pinned by identical byte-exact text across import orders and minter placements
  (REQ-5), not by a cycle ratio.
- Diagnostics are behaviour: only the NEW message (R3/R5) is new. No existing
  diagnostic text churns (GC pins the 2-candidate text unchanged).

## Out of scope (pre-existing warts, recorded not fixed)

`export: Widget[i64]` parse error and the R18 gate's unsatisfiable instantiation
remedy (P5a/P5f); qualified ctor **term** `a::Widget` → unknown word (P7b2); the
concrete-type same-name collision dimension; a distinguishable naming phrasing
for 2+ declaring modules reached *only* through wildcard imports (R4; GP
exercises the mixed one-named-one-wildcard shape, but the both-wildcard
sub-case remains unexercised). None may be assumed as a workaround; none
are fixed by S10 (the policy may not assume "export the word over the type").

## Open questions (resolved before /implement)

- **OQ-1 (error wording).** New located message vs extending
  `no_overload_matches_error` for both shapes. **Resolved: new message; no
  churn to the existing 2-candidate text** (R3; GC pins it).
- **OQ-2 (sequencing).** Implement S10 before or after the pending ladder
  slices S6–S8? **Resolved by maintainer ruling (interview, 2026-09-05): S10
  is implemented in parallel with P7b.S6**, both branching from base `a9eca84`
  — not strictly before or after. Landing rule: if S6's own probes demand a
  check-stage grounding change in `terms.rs`, or the two slices' changes
  interact at merge time, S10's checker change lands first and S6 rebases onto
  it. The roadmap records "S10 ∥ S6", not a strict ladder position (REQ-8).
- **OQ-3 (remedy note).** Should the error's remedy note mention the future
  qualified-ctor-term syntax (P7b2), or qualified type spelling generally?
  **Resolved: neither.** Qualified spelling is dropped from the note entirely,
  not phrased as "not yet available" — measurement during the
  pre-implementation review round showed it never cures the single-candidate
  shape (it just mints a second candidate, routing into the pre-existing
  2-candidate error). See R5.

## Baseline (measured at cd44b1c / a9eca84)

- Suite: **3175 passing / 0 failed** at `cd44b1c` (the slice4 flake is gone
  since S9 Phase 3; expect zero failures); `a9eca84` adds only this slice's own
  docs, no code, so the baseline is unaffected. Re-measure with
  `--no-fail-fast`.
- Test binaries: **80 integration files** (`tests/*.rs`) + lib unit-tests + bin
  unit-tests = **82 test binaries**. S10 adds `tests/phase7b_slice10.rs` → **83**.
  (Slice9-spec's JSON baseline counted 82 at the earlier base `600bc1b`; numbers
  drift with the base, so the implementing phase re-measures from `a9eca84`.)
- Green = `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
  (baseline `clippy --all-targets` separately; it is red at HEAD independent of
  this slice — stash-baseline before trusting it).

## Delivery plan

### Phase 1 — policy + goldens (difficulty: standard)

**Changes.**

- Restructure `bare_generated_word_own_module_grounding` (`src/check/terms.rs`)
  so a headerless caller survives the candidate-identity check
  (`generated_word_entry` + `key`/`symbol` match, currently `:1531-1537`)
  before the new check runs — not the `:1507-1509` fall-through directly,
  which today exits before ever reaching that check (REQ-2). Update the
  `:1492-1497` "the order is free" comment to match.
- Implement R1/R3/R4/R5 at that (restructured) single-candidate arm: once the
  candidate has survived `:1504` (foreign) and the candidate-identity check,
  emit the new located ambiguity error unless one of the four exemptions
  holds — in particular, exemption 2 requires *both* ≤1 reachable header
  *and* the sole candidate's declaring module (`guard.structs[gi].module`,
  never `struct_instantiation_of`'s instantiating-module component) itself
  reachable, where the reachable set is the raw `imports ∪ selective` target
  values (name-independent) plus the export-origin walk-extension; exemption
  4 requires a **named** selective import (never a wildcard desugar) whose
  raw target is resolved through any hub re-export chain before comparing
  against that same declaring module. Build the export-origin walk **over
  the generic header registry** (`ctx.generics().structs`) — not a call into
  `walk_type_export_origin`/`type_origin` (both are concrete-type-only and
  cannot see a generic header at all). Thread the assembly-time
  `SelectiveName.qualifier` distinction (or an equivalent flag) through to
  `Ctx` so exemption 4 can exclude wildcard-desugared entries. No edit to
  `select_overload`, `tier_pick`, env build, or the matcher.
- Retire or rewrite S9's `third_module_bare_caller_dispatches_the_single_shared_env_instantiation`
  golden (`tests/phase7b_slice9.rs`) — REQ-6 is explicit that this one S9
  golden inverts (it currently pins the exact silent outputs GA/GB replace
  with an error); every other S9 golden stays green and byte-unchanged.

**Goldens.** GA, GB, GK, GM (new located error, byte-exact, deterministic
across import orders + minter placement + the mismatched-selective-import
and unreachable-minter cases), GP (new located error, the wildcard case) +
regression pins GC, GD, GE, GF, GG, GH, GI, GJ, GL, GN, GO — all in
`tests/phase7b_slice10.rs`. GE/GF pin the `:1504` own-instantiating-module
arm specifically, not exemption 2 (corrected mechanism attribution).

**Units.** `ambiguous_foreign_headers_grounding_is_located_error`,
`single_foreign_header_grounding_still_borrows`, `own_header_still_grounded_first`,
`ambiguous_header_error_names_declaring_modules`,
`reachable_header_count_excludes_unimported_declaring_modules`,
`matching_selective_import_exempts_the_ambiguity_check`,
`mismatched_selective_import_does_not_exempt_the_ambiguity_check`,
`hub_reexported_selective_import_resolves_through_origin_walk`,
`unreachable_declaring_module_of_the_sole_candidate_is_still_an_error`,
`reachable_set_includes_selective_targets_regardless_of_selected_name`,
`reachable_set_extends_through_reexport_origin_walk`,
`wildcard_desugar_is_not_explicit_resolution` (beside the changed `terms.rs`
code).

**Exit.** GA/GB/GK/GM/GP error byte-exact and deterministic where applicable
(both import orders, both minter placements, the mismatched-selective-import
and unreachable-minter cases); GC/GD/GE/GF/GG/GH/GI/GJ/GL/GN/GO
byte-identical to today; S9's G4 golden retired/rewritten, every other S9
golden + #10 + S5 tier-1 green; new error text pinned; full gate green
(3175+N / 0, less S9's one retired test if replaced rather than rewritten).
Growth signals noted on `terms.rs` (deferred to Phase 2's formal re-check).

**Notes.** Measure-then-pin: pin whatever the formatter renders for the new
message; the R5 draft is the contract, the golden is the bytes. R-NFR1: no IR
edit — if the check needs one, stop and escalate. Exemption 4's match test
(R1) is the soundness-critical piece of this phase — do not implement it as a
bare "selective import present" check (GK), do not compare against the raw,
un-walked selective target (GL), and do not let a wildcard desugar satisfy it
(GP — requires the extra `SelectiveName.qualifier` plumbing, not just a read
of `ModuleInfo.selective`); exemption 2's declaring-module-reachable half is
equally soundness-critical (GM), and its reachable-set definition must be
the union (imports ∪ selective, name-independent — GO) plus the
walk-extension (GN), not imports alone. Guard ordering (REQ-2) is not
optional either — emitting at the `:1507-1509` fall-through without first
restructuring past the candidate-identity check would newly reject the
"ordinary user word" shape `:1531-1537` protects. Do not build the
export-origin walk as a call into the existing `walk_type_export_origin`/
`type_origin` — both operate over concrete `StructDecl`/`EnumDecl` only and
never resolve a generic header (GL/GN both fail if built this way).

### Phase 2 — roadmap + growth re-check + final gate (difficulty: standard)

**Changes.** Update **both** stale sentences in the roadmap's S9 entry
(`docs/roadmap/P7b-higher-kinded-types.md`): the dispatch-mechanism sentence
(`:241-244`, "A third-module bare caller ... dispatches deterministically on
the single instantiation minted into the shared whole-program env") and the
trailing "Residual: ... future work" sentence (`:244-247`) — both now false,
both updated to state the shape is **closed by P7b.S10**; add a new
**P7b.S10** entry in current-design prose (no history narration, per
feedback), pointing at this spec and recording the S10 ∥ S6 parallel
sequencing and landing rule (REQ-8). No code change.

**Goldens.** None new (documentation phase).

**Units.** None.

**Exit.** Roadmap S9 Exit clause no longer describes an open hole; S10 entry
present, recording the parallel-with-S6 sequencing; growth signals (R6)
re-run on every file S10 touched (`src/check/terms.rs`,
`tests/phase7b_slice10.rs`, `tests/phase7b_slice9.rs`, roadmap docs) with the
outcome recorded; final full gate ×2 green.

**Notes.** ROADMAP/DESIGN carry current design only, never history narration.

## Phases (JSON)

```json
{
  "phases": [
    {
      "id": 1,
      "name": "policy + goldens",
      "difficulty": "standard",
      "requirements": ["REQ-1", "REQ-2", "REQ-3", "REQ-4", "REQ-5", "REQ-6", "REQ-7"],
      "changes": [
        "Restructure bare_generated_word_own_module_grounding (src/check/terms.rs) so a headerless caller survives the candidate-identity check (generated_word_entry + key/symbol match, currently 1531-1537) before the new check runs, not the 1507-1509 fall-through directly; update the 1492-1497 'the order is free' comment to match",
        "Implement R1/R3/R4/R5 at that restructured arm: headerless caller + one foreign env candidate (post 1504 + candidate-identity survival) => new located ambiguity error unless one of four exemptions holds; exemption 2 requires both <=1 reachable header AND the sole candidate's declaring module (GenericStructDecl.module, not struct_instantiation_of's instantiating-module component) itself reachable, where the reachable set is imports UNION selective target values (name-independent) PLUS the export-origin walk-extension; exemption 4 requires a NAMED selective import (never a wildcard desugar) whose raw target is resolved through any hub re-export chain before comparing against that same declaring module",
        "Build the export-origin walk over the generic header registry (ctx.generics().structs) -- a new, small function structurally identical to walk_type_export_origin's chase logic but NOT a call into that function or into the precomputed type_origin table, since both are concrete-type-only (StructDecl/EnumDecl) and never resolve a generic header",
        "Thread the assembly-time SelectiveName.qualifier distinction (Some=named, None=wildcard-desugared) through to Ctx, since ModuleInfo.selective's flattened HashMap<String,u32> cannot recover it -- required for exemption 4 to exclude wildcard-desugared entries",
        "Read header provenance from ctx.generics().structs; read the caller's import set from ctx.modules (ModuleInfo.imports/.selective); no edit to select_overload, tier_pick, env build (check.rs:586), or the matcher (find_bound_impl / match_impl_target)",
        "Retire or rewrite S9's third_module_bare_caller_dispatches_the_single_shared_env_instantiation golden (tests/phase7b_slice9.rs), which currently pins the exact silent outputs GA/GB replace with an error"
      ],
      "goldens": [
        "third_module_bare_caller_with_ambiguous_headers_is_a_located_error",
        "third_module_bare_caller_error_is_independent_of_the_eager_minter",
        "both_modules_eager_2_candidate_ambiguity_error_unchanged",
        "single_declaring_header_bare_caller_still_resolves",
        "single_reachable_header_with_selective_import_still_resolves",
        "single_reachable_header_with_qualified_signature_still_resolves",
        "unimported_foreign_type_annotation_is_still_an_error",
        "unimported_declaring_module_does_not_count_toward_ambiguity",
        "matching_selective_import_of_the_sole_minter_still_resolves",
        "selective_import_pins_the_named_exporter_when_both_mint",
        "mismatched_selective_import_is_still_a_located_error",
        "hub_reexported_selective_import_still_resolves",
        "unreachable_minter_bare_call_is_a_located_error",
        "hub_reexport_reachable_through_plain_import_still_resolves",
        "selective_import_of_different_name_still_grants_reachability",
        "wildcard_import_does_not_exempt_the_ambiguity_check"
      ],
      "units": [
        "ambiguous_foreign_headers_grounding_is_located_error",
        "single_foreign_header_grounding_still_borrows",
        "own_header_still_grounded_first",
        "ambiguous_header_error_names_declaring_modules",
        "reachable_header_count_excludes_unimported_declaring_modules",
        "matching_selective_import_exempts_the_ambiguity_check",
        "mismatched_selective_import_does_not_exempt_the_ambiguity_check",
        "hub_reexported_selective_import_resolves_through_origin_walk",
        "unreachable_declaring_module_of_the_sole_candidate_is_still_an_error",
        "reachable_set_includes_selective_targets_regardless_of_selected_name",
        "reachable_set_extends_through_reexport_origin_walk",
        "wildcard_desugar_is_not_explicit_resolution"
      ],
      "exit": "GA/GB/GK/GM/GP error byte-exact and deterministic where applicable (both import orders, both minter placements, the mismatched-selective-import and unreachable-minter cases); GC/GD/GE/GF/GG/GH/GI/GJ/GL/GN/GO byte-identical to today (GE/GF pin the :1504 arm, not exemption 2); S9's G4 golden retired/rewritten, every other S9 golden (G1/G1a-f/G2/G2r/G3) + #10 + S5 tier-1 green; new error text pinned (measure-then-pin); full gate green."
    },
    {
      "id": 2,
      "name": "roadmap correction + growth re-check + final gate",
      "difficulty": "standard",
      "requirements": ["REQ-8"],
      "changes": [
        "Update BOTH stale sentences in the roadmap S9 entry in docs/roadmap/P7b-higher-kinded-types.md: the dispatch-mechanism sentence (:241-244, 'A third-module bare caller ... dispatches deterministically on the single instantiation minted into the shared whole-program env') and the trailing Residual sentence (:244-247) -- both to state the shape is closed by P7b.S10",
        "Add a new P7b.S10 roadmap entry in current-design prose (no history narration), pointing at this spec and recording the S10-parallel-with-S6 sequencing and landing rule"
      ],
      "goldens": [],
      "units": [],
      "exit": "Roadmap S9 Exit clause no longer describes an open hole; S10 entry present recording the parallel sequencing; growth signals (R6) re-run on every file S10 touched with outcome recorded; final full gate x2 green."
    }
  ]
}
```

## Requirement → phase map

| REQ | Phase | Goldens / units |
| --- | --- | --- |
| REQ-1 policy | 1 | GA, GB, GD, GH, GC, GI, GJ, GK, GL, GM, GN, GO, GP; ambiguous/single/own/reachable/matching/mismatched/hub/unreachable/reachable-set/wildcard units |
| REQ-2 layer + ordering | 1 | own_header_still_grounded_first, hub_reexported_selective_import_resolves_through_origin_walk, reachable_set_extends_through_reexport_origin_walk, wildcard_desugar_is_not_explicit_resolution |
| REQ-3 diagnostic | 1 | GA, GB, GC, GK, GM; ambiguous_header_error_names_declaring_modules |
| REQ-4 compat pins | 1 | GC, GD, GE, GF, GG, GH, GI, GJ, GL, GN, GO |
| REQ-5 determinism + naming | 1 | GA, GB, GK |
| REQ-6 S9 goldens (one inverts) | 1 | S9's G4 retired/rewritten; G1/G1a-f, G2, G2r, G3, #10, S5 tier-1 untouched |
| REQ-7 guardrails | 1 | (R-NFR1/2/3 across all Phase-1 goldens) |
| REQ-8 roadmap + growth + gate | 2 | (docs; final gate x2) |
