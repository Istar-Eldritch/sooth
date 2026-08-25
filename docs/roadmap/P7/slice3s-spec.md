# Phase 7 Slice 3s: `Ord` as a library trait, not a compiler-hardcoded bound

**Status:** specified, not implemented.
**Discovery:** `docs/roadmap/P7/slice3s-brief.md`.
**Roadmap:** `docs/roadmap/P7-language-prereqs.md:676-690`.
**Re-verified against:** `1964d5d`. Every `path:line` in the brief was re-checked against
this commit; the corrected set is in "Citation drift" below. Two of the brief's factual
claims are corrected in "Corrections to the brief", and both change the phase plan.

## Problem

`Bound` (`src/ast.rs:1696-1700`) has three variants: `Copy`, `Ord`, `User(TraitId)`. The
first two are reserved, member-less trait-table entries (`seed_predicate_traits`,
`src/ast.rs:1810-1827`, module `RESERVED_TRAIT_MODULE`) that exist only so
`parse_capabilities` can look every bound name up through one uniform mechanism.
Satisfaction is never nominal for either. `is_ord` (`src/check/poly.rs:120-122`) is
exactly `ty.is_numeric()`.

So `'T: Ord` categorically excludes a struct or enum, and a user cannot opt their own type
in: `impl: Ord for Point` is rejected by `impl_for_predicate_trait_error`
(`src/parser.rs:2440-2442`, message at `:348`) and `trait: Ord 'T` collides with the
reserved name. Both are correct, located rejections. **There is no live footgun; the gap
is a missing capability.** `examples/traits.sth` worked around it by inventing a separate
`Order` trait with its own `cmp ( &'T &'T -- Ordering )`.

The design: `Ord` becomes an ordinary library trait declared in `core::cmp` over an
`Ordering` enum, with one `impl:` per numeric width, the six surface comparisons derived
from `cmp`, and `Bound::Ord` deleted.

Two compiler gaps block that, both with fix designs validated in scratch probe worktrees
(neither landed here; **re-derive against live `main`, do not copy-paste**):

1. A trait re-exported through a hub module is invisible to a consumer importing the hub
   (`find_trait_in_module` is a raw one-hop table). Blocks the "zero new import lines"
   exit criterion.
2. A polymorphic body calling a polymorphic word carrying a `Bound::User` is rejected
   outright by `poly_cross_signature_supported` (`src/check/poly.rs:2170-2199`); with that
   rejection removed the program type-checks but the composed `CallInst` carries
   `trait_calls: HashMap::new()`, so lowering cannot find a symbol (R2). A related arm,
   `(Image::Concrete(_), Bound::User(_)) => None` (`src/check/poly.rs:1886`), looks like a
   second silent-accept gap but is not one for any reachable program — R2's own `compose`
   step already re-derives and checks this obligation once the caller is grounded, for
   every case `compose` reaches; the residual is a named, precedented limitation for
   never-instantiated code, not a second fix (R3).

## Citation drift

Every citation in the brief, re-resolved against `1964d5d`. Cite these, not the brief's.

| Brief | Live at `1964d5d` |
| --- | --- |
| `ast.rs:1679-1683` (`Bound`) | `src/ast.rs:1696-1700` |
| `ast.rs:1793-1811` (`seed_predicate_traits`) | `src/ast.rs:1810-1827` (`predicate_traits` at `:1800`) |
| `poly.rs:120-122` (`is_ord`) | unchanged, `src/check/poly.rs:120-122` |
| `declarations.rs:1360` (`poly_admits`) | `src/check/declarations.rs:1395-1401` |
| `poly.rs:1881` (concrete `Ord` discharge) | unchanged, `src/check/poly.rs:1881-1883` |
| `poly.rs:1886` (`(Concrete, User) => None`) | unchanged, `src/check/poly.rs:1886` |
| `poly.rs:1856` (symbolic forwarding arm) | `src/check/poly.rs:1857-1859` |
| `poly.rs:2188` (blanket `Bound::User` rejection) | `src/check/poly.rs:2188-2197`, in `poly_cross_signature_supported` at `:2170` |
| `poly.rs:2285` (`Bound::Ord => "Ord"`) | unchanged, `src/check/poly.rs:2285` |
| `poly.rs:4428` (`poly_sig_could_match` Ord filter) | unchanged, `src/check/poly.rs:4428`; `fn` at `:4401` |
| `poly.rs:4693` (bound-loop `Ord` arm) | unchanged, `src/check/poly.rs:4693` |
| `poly_ord_bound_error` | `src/check/poly.rs:6457` |
| `calls.rs:737` (`checked user word exists`) | unchanged, `src/ir/func_builder/calls.rs:737` (twin at `:728`) |
| `parser.rs:508-538` (`find_trait_in_module`) | `src/parser.rs:508-541` |
| `parser.rs:530-534` (selective branch) | `src/parser.rs:534-539`; qualified branch `:515-521` |
| `parser.rs:1676` (unknown-capability text) | unchanged |
| `driver.rs:352-421` (type walker) | `resolve_type_export_origins` `src/driver.rs:363`, `walk_type_export_origin` `src/driver.rs:406`; call site `:579`, threaded at `:663` |
| `declarations.rs:394-400` (`impl_target_module`) | `src/check/declarations.rs:404-411` |
| `declarations.rs:407-482` (`check_impl_decls`) | `src/check/declarations.rs:417-…` |
| `declarations.rs:432-438` (orphan rule) | `src/check/declarations.rs:444-…` |
| `declarations.rs:3488-3507` (orphan-scalar test) | `src/check/declarations.rs:3698` |
| `member_binds_trait_var` `declarations.rs` | `src/check/declarations.rs:367` |
| `poly_trait_member_call` | `src/check/poly.rs:904` |
| `combinators.rs:155-157` (`is_combinator`) | unchanged |
| `word_families.rs:1397` (scalar-borrow error) | unchanged |
| `reject_user_bound_on_combinator` (roadmap says `poly.rs:5919`) | `src/check/poly.rs:5927`; error text `:5951` |
| `CrossGround` / `compose` / `discover_transitive_instantiations` | `src/check/poly.rs:4856` / `:5015` (`trait_calls: HashMap::new()` at `:5067`) / `:4785` |
| `resolve_user_bound` / `TraitResolveCtx` | `src/check/poly.rs:5145` / `:79-84`; real construction `src/check.rs:853`, `scratch()` `src/check/poly.rs:93` |

Confirmed unchanged in substance: `lib/cmp.sth`'s six `inline ( 'T: Copy Ord 'T -- Bool )`
words; `examples/poly_if.sth:6`'s `import: core::prelude * ;`; `core::prelude` re-exports
`Bool` as a *type* name (`lib/prelude.sth`), so the type-name hub walk is landed and works.

Live `'T: ... Ord` signature count in `.sth` sources: **8**, not 14 — `lib/cmp.sth` (6) and
`examples/poly_if.sth` (2). `docs/roadmap/P8/dogfood/core/cmp.sth`'s 6 are stale and do not
compile (its own README says so); ignore them. In Rust fixtures, `grep -rn "Copy Ord"
tests/ src/ --include=*.rs` returns **60** sites.

## Corrections to the brief

### C1. `examples/poly_if.sth`'s two `Ord`-bounded words are `inline`, so they are combinators, and the flip rejects them outright

`mymax` and `mymax3` (`examples/poly_if.sth:9,13`) are both
`: … inline ( 'T: Copy Ord … )`. `is_combinator` is exactly `word.declares_inline`
(`src/check/combinators.rs:155-157`), and `reject_user_bound_on_combinator`
(`src/check/poly.rs:5927`) refuses *any* `Bound::User` on a combinator's own type
variable. The moment `Ord` stops being a predicate, both words are a hard error.

Probed live at `1964d5d` with a stand-in user trait:

```text
$ : pick inline ( 'T: Copy MyOrd 'T -- 'T ) drop ;
error: `'T: MyOrd` on the combinator `pick` at line 6, col 3 is not supported
  note: a combinator is spliced at its call sites and records no instantiation a trait bound could resolve against
```

The brief asserts these two call sites are "covered by the existing `core::prelude *`
import" and therefore free. They are not free: import visibility was never their problem.
This is the same `inline`-vs-user-bound wall the brief already accepts for the six
comparisons, and it reaches further than the brief counted — 5 of the 60 in-tree Rust
fixture sites are `inline` too (`tests/phase4_generics.rs:1114`,
`tests/phase7_slice3b_follow.rs:667`, and doc comments at
`tests/phase4_slice11_inline.rs:224`, `src/check/word_entry.rs:74,428`).

**Resolution (R7): drop `inline` from `mymax`/`mymax3` as well.** Probed: with `inline`
removed from both, `examples/poly_if.sth` builds clean at `1964d5d` today. Under the flip
they become ordinary generic bodies forwarding `'T: Copy Ord` to a non-inline `gt` — the
exact cross-call forwarding case phase 1 closes, so this is a witness, not a workaround.
It costs the same call-frame tax the slice already accepts and it churns
`tests/qbe_baseline/poly_if.ssa`.

### C2. The REPL loses `'T: Copy Ord` entirely, and there is no cheap fix

`src/repl.rs` holds no trait registry at all (`grep -n traits src/repl.rs` is empty). Every
REPL parser context is built with `traits: crate::ast::predicate_traits()`
(`src/parser.rs:1162,1217,1298`), and the comment above the first of those says outright
"a REPL word def still needs `'T: Copy Ord` to work; a user `trait:` declaration is not yet
supported at REPL scope, so the reserved predicate-only table is all this context ever
sees." `TraitResolveCtx::scratch()` (`src/check/poly.rs:93-99`) rests on the same premise:
"a session declares no `trait:`, so its bounds are `Copy`/`Ord` only."

Working today:

```text
$ : mymax ( 'T: Copy Ord 'T -- 'T ) drop ;
defined mymax
$ 3 4 mymax
stack: 3
```

Delete `Ord` from the predicate table and that line becomes
`error: unknown capability 'Ord' …`. Restoring it needs the REPL to carry a whole-program
trait *and* `impl:` registry from its imported modules, which is a slice of its own.
Ruled in R8: accept the regression, but make it a specific located diagnostic rather than
the generic unknown-capability text, and name the follow-on.

### C3. `trait_origin` cannot be computed beside `type_origin`

`type_origin` is built at `src/driver.rs:579`, before the trait pre-pass. The trait
registry does not exist until the `prepass_trait_decls` loop finishes
(`src/driver.rs:632-649`). `resolve_trait_export_origins` must run **between
`src/driver.rs:649` and the per-module loop starting at `:652`** (the `parse_bodies` call
itself is nested inside, at `:655`), not next to `type_origin`.

### C4. Both of `find_trait_in_module`'s import branches are one-hop, and there are two callers

The brief names only the selective branch (`src/parser.rs:534-539`). The qualified branch
(`:515-521`) has the same defect, which is why the probe needed a second golden for the
bare-qualifier form `h::Greet`. Both callers must benefit: `parse_impl_decl`
(`src/parser.rs:2432`) and `bound_trait_id` (`src/parser.rs:3293`) — i.e. a hub-re-exported
trait must be usable both as a *bound* and as an `impl:` target's trait name.

### C5. The roadmap entry is stale on `cmp`'s shape

`docs/roadmap/P7-language-prereqs.md:685` still says `cmp ( &'T &'T -- Ordering )`. The
brief's probe correction is by value, `cmp ( 'T 'T -- Ordering )` (R4). Update that line in
phase 2; do not silently ship a spec that contradicts the roadmap.

## Design rulings

### R1 — Phase 0: `resolve_trait_export_origins`, mirroring the type walker exactly

Add to `src/driver.rs`, adjacent to `resolve_type_export_origins`/`walk_type_export_origin`
and shaped identically:

```rust
fn resolve_trait_export_origins(
    traits: &[TraitDecl],
    module_count: usize,
    exports: &[Vec<(String, Span)>],
    import_maps: &[HashMap<String, u32>],
    selectives: &[HashMap<String, u32>],
) -> Vec<HashMap<String, u32>>
```

with `walk_trait_export_origin` as the per-name hop walk: `visited` set for cycle
termination, stop when `declared_traits[current]` contains the name, else follow
`selectives[current][name]` or the first import target that declares it, else `None`.
`declared_traits[m]` excludes `RESERVED_TRAIT_MODULE`, so `Copy`/`Ord` never enter the walk.

The table is **advisory and tolerant**, exactly as the type one is: a name it cannot place
(it names a word, or the walk dead-ends or cycles) is left out, and the ordinary one-hop
lookup reports the real diagnostic. Placement per C3.

`find_trait_in_module` gains a sixth parameter `trait_origin: &[HashMap<String, u32>]` and
consults it as a fallback in **both** import branches, after the existing exact match
fails:

- qualified `q::Base`: `target = imports[q]`; if no trait `Base` has `module == target`,
  retry with `module == trait_origin[target][Base]`.
- selective `Name`: `target = selective[Name]`; same retry against
  `trait_origin[target][Name]`.

Own-module and `RESERVED_TRAIT_MODULE` lookups are untouched and must be re-verified
working. `trait_origin` is threaded as a `Parser` field beside `type_origin`
(`src/parser.rs:2004`), defaulting to `&[]` at each of the scratch/REPL construction sites
that already pass `type_origin: &[]` (`src/parser.rs:776,841,1020,1065,1158,1216,1297`) and
through `parse_bodies` (`:661-676`, `:699`) and the two internal re-constructions
(`:3904`, `:4108`).

`prepass_trait_decls` does **not** get the table: it parses trait *declarations*, and a
member signature's bound is resolved later, in `parse_bodies`. If a probe shows otherwise,
that is a finding to report, not to fix silently.

### R2 — Phase 1: `CrossGround` carries a `TraitResolveCtx`, and `compose` resolves the callee's obligations

Delete the `Bound::User` blanket rejection from `poly_cross_signature_supported`
(`src/check/poly.rs:2188-2197`) and its doc bullet. `src/ir/func_builder/calls.rs` gets
**no change**: it already reads `trait_calls` by span (`:278`) for every `CallInst`
regardless of which path produced it.

`discover_transitive_instantiations` (`src/check/poly.rs:4785`) gains two parameters,
`word_symbols: &[String]` and `trait_obligations: &[WordObligations]`, both live locals at
its single call site (`src/check.rs:993`; `symbols` at `:667`, `trait_obligations` at
`:786`). It builds its own `TraitResolveCtx` from those plus `module.traits`/`module.impls`
— built *inside*, because a caller-side context would freeze `module` while this function
still needs `&mut module`. `CrossGround` (`:4856`) gains a `tr: TraitResolveCtx<'a>` field.

`compose` (`:5015`), once it has grounded `subst` (which it already does to resolve
ordinary output types), runs the same `resolve_user_bound` loop `check_poly_call` runs, and
stores the result in place of the hardcoded `trait_calls: HashMap::new()` at `:5067`. The
comment there ("a user trait bound is a located rejection on the cross-call path") is
deleted, not amended; `quot_inputs: Vec::new()` stays, that half of the rejection remains.

### R3 — Phase 1: `(Image::Concrete(_), Bound::User(_))` stays `None`, but its comment is wrong and must be corrected

**Revised during spec review.** The brief and an earlier draft of this ruling both assumed
the arm at `src/check/poly.rs:1886` needs to become a live registry lookup, mirroring R2.
It does not, and building that would be redundant plumbing. Traced live: `compose`
(`:5015`) grounds *every* mapping entry — `Image::Concrete` and `Image::CallerVar` alike
— into a real `ty` in its `subst`-building loop (`:5026-5030`), uniformly, before R2's
`resolve_user_bound` loop runs over `sig.bounds` against that grounded `subst`. R2's loop
does not branch on the mapping entry's original kind — it cannot tell, by the time it
runs, whether a given `v` started as `Image::Concrete` or `Image::CallerVar`. So **R2's
loop already re-derives and checks this exact obligation for every reachable cross-call**,
using the identical diagnostic (`unsatisfied_user_bound_error`) and the identical span
(`record.span`, recorded at the original cross-call site either way — there is no
diagnostic-quality loss from deferring). No new plumbing, no mutable `arrays`/`refs`/
`impls` threaded through `poly_call_term`'s live-walk call chain, and no borrow-conflict
risk against the walk's existing immutable `arrays: &[ArrayDecl]` (the concern the doc
comment at `:1863-1865` raises for the sibling `Bound::Copy` arm does not apply here, since
nothing is being threaded through that chain at all).

**What R3 actually changes:** the arm's comment, not its code. Today it reads "A user
bound is gated out of a cross-call entirely by `poly_cross_signature_supported`, so this is
unreachable" — false once R2 lands, since `poly_cross_signature_supported` no longer gates
anything out. Correct it to say the arm is intentionally *not* the resolution point:
resolution happens later, in `compose`, once the caller is grounded, and this walk-time
site defers to it. The code (`(Image::Concrete(_), Bound::User(_)) => None`) is unchanged.

**The residual gap, named rather than silently accepted:** `compose` only runs for a
cross-call whose *enclosing* poly word is reached from `discover_transitive_instantiations`'s
fixpoint, itself seeded from real concrete instantiations (`insts`). A poly word that is
never instantiated anywhere in the whole program — genuinely dead generic code — has its
cross-call recorded but never composed, so an unsatisfiable `Bound::User` obligation inside
it is never checked, ever. This is not a new gap this ruling introduces: it is precedented
by the sibling arm two lines above it in the same match, `(Image::Concrete(ty), Bound::Copy)
if !type_is_registered(...)`, which the existing shipped code already accepts with the same
rationale ("deciding it needs a registry the walk does not hold"). No runtime unsoundness
follows — unreached code is never monomorphized or lowered — only a latent, undetected
compile-time error in code nobody calls. Accept it on the same precedent, named explicitly
in the phase report, not left implicit.

The brief separately flagged the concrete-image case as **unconfirmed** (a probe claimed it
"errors correctly" but the archived session lost the output). That probe result is now
subsumed by this ruling: re-verify by exercising R2's own golden (a reachable cross-call
with no matching `impl:`) rather than chasing the original unrecoverable claim.

R2 and R3 land in the **same phase** — R3 has no deliverable of its own beyond the comment
fix and the exit-criterion wording below, but it depends on R2 being in place to be true.

### R4 — `cmp` is by value

```sooth
type: Ordering | Less | Equal | Greater ;

trait: Ord 'T
  cmp ( 'T 'T -- Ordering )
;
```

Not `&'T &'T`. A `&i64` is unobtainable from a plain scalar local at all
(`src/check/word_families.rs:1397`: "a scalar has no address; borrow a field or an
aggregate instead"), so a borrowed `cmp` cannot be called on `i64` from an ordinary generic
body without routing every numeric comparison through a wrapper struct. It is also
unnecessary: every `Ord`-adjacent generic word already carries `'T: Copy`, so element reuse
after a comparison is `Copy`'s job.

`Ord`, `Ordering` and its three variants are declared in `core::cmp` (`lib/cmp.sth`) and
re-exported from `core::prelude` (`lib/prelude.sth`'s `import: self::cmp | … |` and
`export:` lines). `Ordering`'s type name crosses the hub on the existing
`resolve_type_export_origins` walk (as `Bool` does today); `Ord` crosses on R1's.

### R5 — The six comparisons ship non-inline, deliberately, for one slice

`lib/cmp.sth`'s `eq`/`lt`/`gt`/`lte`/`gte`/`ne` lose `inline` and are rewritten over `cmp`
rather than over the raw intrinsics. Measured cost from the brief's benchmark: **+86.6%**
on a comparison-heavy loop (28.9ms → 53.9ms, 7 runs each, non-overlapping spreads),
mechanism confirmed by disassembly (straight-line `cmp`/`setcc` versus a real call per
comparison). This is a real ~2x tax, accepted because the cross-call gap must close either
way, and because landing a correct non-inline implementation first hands P7.S3o the
differential oracle it has never had (R9).

The `impl: Ord for i64` (and one per remaining numeric width) bodies live in `core::cmp`
and are built from the raw `ult`/`ueq`/… intrinsics exactly as today's comparison bodies
are. `impl:` for a scalar target is already legal inside the trait's own declaring module
(`impl_target_module` returns `None`, the orphan rule accepts, pinned by
`check_impl_decls_orphan_scalar_target_names_only_the_trait_module`,
`src/check/declarations.rs:3698`).

Ruled during review: deriving all six comparisons from one `cmp` call needs care around a
NaN operand, since IEEE-754 treats a NaN pair as a fourth, "unordered" case none of
`Less`/`Equal`/`Greater` can represent directly. `lib/cmp.sth`'s float `impl: Ord` answers
`Greater` for a NaN pair (after ruling out true equality via `ueq`), which keeps
`eq`/`ne`/`lt`/`lte` IEEE-correct for NaN by construction -- preserving Phase 0's D4 (NaN
detected via `x = x`). `gt`/`gte` cannot read that same `Greater`/`Greater-or-Equal` arm
directly without also answering `True` for NaN; instead they compare with the operands
swapped (`a > b` iff `b < a`, `a >= b` iff `b <= a`, an IEEE-754 identity that holds for
every value including a NaN pair, where both sides are `False`), which keeps all six
comparisons IEEE-correct for NaN with the existing 3-variant `Ordering` -- no fourth
variant or `PartialOrd` split needed.

### R6 — Both overload-admission sites become registry lookups

`poly_admits` (`src/check/declarations.rs:1395-1401`) and `poly_sig_could_match`
(`src/check/poly.rs:4401`, filter at `:4428`) are **not** bound-satisfaction sites. They
are slice 10c's overload-admission filter: "has an `Ord` bound && `!is_numeric` → decline",
which is what lets the library's generic `lt ( 'T: Copy Ord 'T -- Bool )` coexist with a
user's concrete `lt ( Vec2 Vec2 -- Bool )`. The brief confirmed both load-bearing by
mutation (neutering either breaks the coexistence program, one at declaration time, one at
the call site). Both go dead the instant `Bound::Ord` is deleted.

Each becomes: decline unless the operand type has an `impl: Ord` in the whole-program
`(TraitId, Type)` registry. `poly_admits` currently takes only `(&PolySig, &[Type])` and
`poly_sig_could_match` is `pub(super)`; both need the registry threaded in. Thread the
minimum — `&[ImplDecl]` plus the `TraitId` of `Ord` where it is resolvable — and do **not**
introduce a shared "bound satisfaction" abstraction spanning `declarations.rs` and
`poly.rs` for two call sites. If a helper is genuinely shared by both, it goes in
`src/check.rs`, the lowest common ancestor, and nowhere higher.

`poly_admits`'s doc comment currently justifies consulting only `Ord` ("`Copy` needs the
struct/enum registries this pass does not carry"). That justification survives the rewrite
verbatim and must be preserved, not dropped.

**Ruling: a new same-module overlap becomes reachable, and it is correct behaviour, not a
bug to route around.** `check_generic_concrete_overlap` (`src/check/declarations.rs:1285`)
is keyed by `(word.module, name)` — it only fires when *one module* declares both a
concrete and a poly candidate of the same name itself, which is exactly the shape the
coexistence golden below already uses (not a general cross-module import concern). Once
`Vec2` has `impl: Ord`, a module declaring both `impl: Ord for Vec2` and its own concrete
`lt ( Vec2 Vec2 -- Bool )` now has two candidates that genuinely both admit a `Vec2 Vec2`
call — real ambiguity, not a false positive, and `generic_concrete_overlap_error`
(`:1405`) is the correct diagnostic for it. A carve-out would require ranking between two
equally-admissible candidates, i.e. real overload resolution, which this language has
deliberately never had (slice 10c's design is admission-only). Building that now is scope
creep this slice should not absorb. The way out costs nothing new: drop the standalone
concrete `lt` (the generic one now dispatches via the type's own `impl: Ord`), or keep the
concrete `lt` and skip `impl: Ord` for that type.

### R7 — `examples/poly_if.sth` drops `inline` too

Per C1. `mymax` and `mymax3` become ordinary generic bodies. Both then exercise R2's
forwarding path against the library `gt`. `tests/qbe_baseline/poly_if.ssa` is regenerated
deliberately (`REGEN_QBE_BASELINE=1 cargo test --test qbe_baseline`) and the diff reviewed
for the expected shape: calls where straight-line comparison code used to be, and nothing
else. `tests/corpus_stdout/poly_if.txt` must be **byte-identical** — behaviour does not
change. The same applies to every other baseline the flip churns; regenerate deliberately,
never to make red go green, and state in the phase report how many `.ssa` files changed and
that every `.txt` was untouched.

**Revised count and finding, from live verification during spec review.** The 5 sites are
**3 real fixtures, 2 doc comments**, not 2-and-3 as C1 originally counted:
`tests/phase4_slice11_inline.rs:224` and `src/check/word_entry.rs:74` are doc comments;
`tests/phase4_generics.rs:1114`, `tests/phase7_slice3b_follow.rs:667`, and
`src/check/word_entry.rs:428` are live fixtures.

The two test fixtures are **not** a mechanical "drop `inline`" migration like
`poly_if.sth`'s. Live-checked: `poly_mymax_runs_at_i64_and_f64`
(`tests/phase4_generics.rs:1106`) and `inline_generic_body_still_splices_a_row_combinator`
(`tests/phase7_slice3b_follow.rs:660`) both name, in their own comments, the exact property
they exist to pin — that an `inline` generic body calling `if` (a row combinator) splices
correctly, the S3o wall this slice's own R5 already accepts for `lib/cmp.sth`. Once `Ord`
is nominal, `inline ( 'T: Copy Ord 'T -- 'T )` is illegal to declare at all
(`reject_user_bound_on_combinator`) — dropping `inline` from these two, the way `poly_if.sth`
drops it, would silently delete the inline-splice property their names say they pin, not
migrate it.

**Verified live** (not assumed): a non-`inline` `mymax` with the identical `if`-based body
compiles and runs correctly today at `1964d5d` (`3 7 mymax . 3.0 7.0 mymax .` produces
`7\n7\n`), confirming C1's claim for `poly_if.sth` and ruling out that these two tests'
own "a non-spliced polymorphic body rejects a quotation outright" framing still holds —
that framing predates S3k (generic-calls-generic), which is exactly what makes a
non-inline body calling `if` legal today. The comments are stale, not the mechanism.

**Ruling:** rewrite both fixtures' bound list from `'T: Copy Ord` to `'T: Copy` alone,
replacing the library `gt` call with the raw intrinsic **and the `Bool` construction it
wraps** — `ugt [ True ] [ False ] branch`, not bare `ugt` — so the body stays genuinely
`Bound::User`-free and legal to declare `inline` while still producing the `Bool` its own
`if` consumes. **Verified live, both ways during spec review**: bare `ugt` in place of `gt`
fails — `error: type mismatch in mymax ... 'if' expected 'Bool', found 'u32'` — because
`if` needs `Bool` and `ugt` is the raw flag `gt` itself wraps in exactly this `branch`
construction (`lib/cmp.sth:24`: `gt inline (...) ugt [ True ] [ False ] branch`); with the
full `ugt [ True ] [ False ] branch` in place of `gt`, the equivalent fixture builds and
runs, producing the expected output. This preserves the actual property under test (an
`inline` generic body splices `if` correctly) without needing a bound this slice retires
from the `inline` surface, and the fixture still needs `Bool`/`True`/`False`/`if` from
`core::prelude` as it already does today. Update each test's comment to note `Ord` was
dropped and `gt` replaced with its own wrapped intrinsic for exactly this reason, with a
forward reference to R5/S3o for why. Do **not** drop `inline` from these two — that would
be silent scope loss, not a fix.

`word_entry.rs:428`'s `check_inline_polymorphic_signature_is_accepted` has a narrower
issue: its primary assertion (`: id inline ( 'T -- 'T )`) is unaffected, but its `EQ`
witness constant incidentally uses `'T: Copy Ord` where the test's own comment says its
real purpose is only to hit the builtin-name overlap gate for a word named `eq` — `Ord`'s
presence there is incidental, not load-bearing. Drop `Ord` from the witness's bound list
(`'T: Copy` alone), preserving the test's actual purpose.

### R8 — The REPL regression is ruled, diagnosed, and handed off

Per C2. A REPL word definition carrying `'T: Ord` stops working.

**Discriminator, concrete.** `Parser` (`src/parser.rs:2004`-adjacent) has no field
distinguishing REPL scope today — `traits: crate::ast::predicate_traits()` is shared by
the three genuine REPL construction sites (`parse_line_with_structs:1162`,
`parse_typedef_line:1217`, `parse_enum_typedef_line:1298`) **and** four non-REPL
file-parsing prepass/scratch sites (`:777,846,1021,1066` — `prepass_generic_typedefs`,
`prepass_trait_decls`, `scan_imports`, `scan_exports`), so "only sees `predicate_traits()`"
cannot be the signal. Add a new `Parser` field, `is_repl: bool`, `false` everywhere except
the three REPL construction sites above, which set it `true`.

**Call site, exact.** `parse_capabilities` (`src/parser.rs:3243`) is where
`unknown_capability_error` is raised (`:3261,3266`, via `predicate_bound`/`bound_trait_id`
failing). Branch there on `self.is_repl`: the existing message for a file, a new
REPL-specific one otherwise.

**Wording:** `error: unknown capability '{name}' at line {line}, col {col} ('{name}' is a
core::cmp trait; the REPL carries no trait or impl: registry to resolve it against --
define a word needing it in a file and load that instead)`. Pinned by a `repl_ux.rs` test
asserting this exact text for `'T: Ord`. `TraitResolveCtx::scratch()`'s doc comment
(`src/check/poly.rs:86-92`) and the comment at `src/parser.rs:1159-1162` are both updated
— they currently assert the opposite, and leaving them is how a future reader reintroduces
the bug.

Named follow-on: a REPL session carrying its imported modules' trait/`impl:` registries.
Out of scope here.

**Scope correction, found during review: the REPL loses every comparison, not only a
session's own `'T: Copy Ord` declaration.** `repl.rs`'s `splice_import` binds an imported
module-0 word into `self.env` only if `w.poly.is_none()` (an ordinary concrete word), and
retains it in the combinator store only if `check::is_combinator(w)` (`declares_inline`).
Before R5, every polymorphic library word was `inline`, so this pair of cases was exhaustive
for everything the REPL could import; R5 is what creates the first *non-inline* polymorphic
library word (all six comparisons), and it falls into neither case, so it is never bound at
all. Importing `eq`/`lt`/`gt`/`lte`/`gte`/`ne` and then using one, at a session line or inside
a spliced combinator's arm, is `error: unknown word`. This is strictly wider than R8's own
framing above (which only anticipated a session's own generic *declaration* losing `Ord`):
importing and calling the library's already-compiled comparisons is broken too, with no bound
resolution involved at all -- the imported closure's `impl:` bindings are already resolved by
`assemble_module` before the REPL ever sees them. Closing it needs the REPL to support
calling/monomorphizing a non-inline generic word for the first time (a new binding case in
`splice_import`, and a call-site instantiation mechanism the REPL's `dlopen`-per-word model has
never needed before), which is a separate slice, not this one's. Left `#[ignore]`d with this
note: `sign_definable_and_callable_in_repl`,
`self_tail_recursive_word_completes_in_constant_stack_in_repl`, `vm_dogfood_runs_in_repl`
(`tests/phase1.rs`), `usize_comparison_across_a_repl_line_matches_same_line_semantics`
(`tests/phase3_strings.rs`), `repl_while_define_runs_to_fixpoint`,
`repl_two_output_combinator_define_and_call`, `repl_imported_while_runs_to_fixpoint`,
`repl_imported_filter_runs`, `repl_combinators_dogfood_matches_native`
(`tests/phase4_combinators.rs`), and `repl_defined_spliced_self_tail_loops_in_constant_stack`
(`tests/phase4_slice10c_tail_splice.rs`).

### R9 — What this hands P7.S3o

S3o's brief parks it on "revisit only if a concrete program actually needs bound dispatch
on a combinator's own type variable". After this slice, `lib/cmp.sth`'s six comparisons and
`examples/poly_if.sth`'s two words are exactly that program, plus a **correct non-inline
implementation to differential-test against**: flip `inline` back on the same source, diff
program output and the resolved `impl:` symbols (`nm`) at two splices, at three, and inside
a materialized quotation literal. That converts S3o's untestable soundness property into a
mechanical diff. Phase 3 lands the harness skeleton and the roadmap note; it does **not**
attempt S3o.

### R10 — Out of scope, named

- **A borrowed impl for a linear element.** `impl: Ord for &Point` is an independent
  `(TraitId, Type)` entry, not a variant reading of `impl: Ord for Point`, and the brief
  confirmed by probe that there is no autoref: a generic word whose `'T` infers to the
  owned type fails outright against a registry holding only `(Ord, &i64)`. A by-value `Ord`
  therefore excludes *linear* elements from being sorted or searched, since a linear value
  can never carry `Copy`. `examples/experiments/binary_search.sth` already assumes the
  borrowed shape and does not compile as-is; it stays that way. **The exit criteria below
  commit only to `'T: Copy Ord`**, the numeric/`Copy`-struct case that exists today.
- **P7.S3o** (re-`inline`ing the comparisons). R9.
- **A REPL trait registry.** R8.
- **Explicit call-site instantiation.** That is P7.S3t.

## Phases

Each phase must end green: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
(assess with `--no-fail-fast`; `cargo test` stops at the first failing binary).

### Phase 0 — Trait-through-hub re-export

Scope: R1 only. Nothing `Ord`-shaped touches this phase.

Deliverables:

- `resolve_trait_export_origins` + `walk_trait_export_origin` in `src/driver.rs`, placed
  per C3.
- `find_trait_in_module`'s sixth parameter and both fallback branches (C4).
- `trait_origin` threaded as a `Parser` field through `parse_bodies` and every construction
  site.

Tests:

- Unit, beside `find_trait_in_module` in `src/parser.rs`: the existing
  `find_trait_in_module_resolves_own_module_then_qualified` (`:8473`) extended, or a
  sibling, covering own-module / reserved / one-hop selective / one-hop qualified — all
  four still resolve with an empty `trait_origin`.
- Unit, beside the walker in `src/driver.rs`: multi-hop resolution, a cycle returning
  `None`, and a name that is a word rather than a trait returning `None`.
- Golden (`tests/phase7_slice3s.rs`), both **built and run**, asserting real stdout:
  1. A trait declared in module A, re-exported by hub B, consumed by C via
     `import: B | Name | ;` used as a bound.
  2. The same reached by bare qualifier `b::Name`.
- Golden: an `impl:` in C naming a hub-re-exported trait resolves (C4's second caller).
- Mutation check: revert the selective fallback alone, then the qualified fallback alone,
  and confirm one golden fails each time. A twinned guard tested in one half only is a
  known repeat failure in this repo.

Exit: the two goldens pass; the one-hop and `Copy`/`Ord` reserved lookups are re-verified
working; suite green.

### Phase 1 — Cross-call user-bound resolution, and the concrete-image hole

Scope: R2, plus R3's comment fix (no code change — see R3's revision). No `.sth` library
change.

First act: verify R2's own golden (below) actually produces a located error for a
reachable concrete-image cross-call with no matching `impl:`, live, before writing
anything else — this supersedes the brief's original unrecoverable probe claim, which is
no longer the thing being confirmed.

Deliverables: the `poly_cross_signature_supported` rejection deleted;
`discover_transitive_instantiations`'s two new parameters; `CrossGround.tr`; `compose`'s
`resolve_user_bound` loop. The `(Image::Concrete(_), Bound::User(_))` arm's **code is
unchanged** (still `None`); only its comment is corrected, per R3, to state it defers to
`compose` rather than claiming unreachability. `src/ir/func_builder/calls.rs` unchanged —
if it needs a change, the design is wrong.

Tests:

- `check_generic_cross_call_discharges_a_forwarded_user_bound` (the probe's own name),
  asserting the composed `CallInst.trait_calls` resolves to `impl: Show for Point`'s
  symbol — not merely that the program checks.
- The existing test pinning the old blanket rejection has its `Bound::User` case replaced;
  its row/quotation/length-variable cases stay.
- A golden that **runs** the forwarding program and asserts stdout, so a wrong-symbol link
  is caught, not just a resolvable one.
- A located-error golden exercising R2's `compose` loop on a concrete-image cross-call: a
  generic word, reachable from `main`, whose body cross-calls a `Bound::User`-bounded
  callee where the mapped operand is already `Image::Concrete` at walk time (not a
  forwarded caller variable), against a type with no matching `impl:`. Asserts the exact
  `unsatisfied_user_bound_error` text. This is what used to be called "R3's registry
  lookup"; it is now a regular exercise of R2's own loop, not a separate code path.
- A **documented, deliberately unenforced** residual case, not a golden: a poly word with
  an internal `Bound::User`-unsatisfiable cross-call that is never instantiated anywhere in
  the program compiles clean today (dead code, unreached by `compose`'s fixpoint). Record
  this in the phase report as a named, precedented limitation (same class as the sibling
  `Bound::Copy`-on-body-local-instantiation arm), not a bug to chase in this phase.
- Mutation: restore `trait_calls: HashMap::new()` in `compose` and confirm the run golden
  fails; break `compose`'s `resolve_user_bound` call (e.g. skip the loop) and confirm the
  concrete-image error golden fails. Commit first — a mutation run in this repo has wiped
  an uncommitted phase before.

Exit: both mutations kill a test; the dead-code residual is named in the phase report;
suite green.

### Phase 2 — The flip

**Revised framing.** "Atomic and irreducible" overstated the phase: `trait: Ord`'s own
declaration cannot coexist with the reserved seed entry (step 3), so declaring the library
trait, deleting the variant, and rewriting the six comparisons over `cmp` are genuinely
inseparable. But R7's `poly_if.sth`/fixture migration (step 6) and C5's roadmap-prose
correction (step 7) are independently green *before* the flip — both were probed standalone
and compile clean today — and R6's *threading* (adding registry parameters to
`poly_admits`/`poly_sig_could_match` without yet changing their `Ord`-vs-registry decision)
is separable from R6's *behavioural* change. The irreducible core is steps 1, 3, and the
behavioural half of step 5; everything else can land in the same commit for review
convenience but does not have to. Still the largest phase; the point is precision about
why, not a smaller phase.

Sub-steps, in order, for a reviewer to check:

1. `lib/cmp.sth`: `type: Ordering`, `trait: Ord 'T cmp ( 'T 'T -- Ordering ) ;`, **twelve**
   `impl: Ord for <width>` blocks over the raw intrinsics — the eight fixed-width ints
   (`i8 i16 i32 i64 u8 u16 u32 u64`, `INT_TYPES` at `src/ast.rs:2413-2422`), `usize`,
   `isize`, and the two floats (`f32 f64`, `FLOAT_TYPES` at `:2441`) — and the six
   comparisons rewritten over `cmp`, **non-inline** (R4, R5). Add `Ord Ordering Less Equal
   Greater` to `lib/cmp.sth`'s own `export:` line (currently `export: eq lt gt lte gte ne
   ;`, `:20`) — without this the `cmp -> prelude -> consumer` hub chain in step 2 has
   nothing to walk.
2. `lib/prelude.sth`: `Ord`, `Ordering` and its variants added to the `self::cmp` import
   and the `export:` list.
3. Delete `Bound::Ord` from `src/ast.rs:1698`, `Ord` from `seed_predicate_traits`, and
   `is_ord` from `src/check/poly.rs:120-122`. The remaining `Bound` arms at
   `src/check/poly.rs:1881,2285,4428,4693` and `src/check/declarations.rs:1398` go with
   them. Enumerate every wildcard arm over `Bound` before starting: a phase's own list of
   unarmed arms has missed cases in this repo before.
4. **No source change** for the reserved-predicate guard or the reserved-name collision —
   verified live, correcting an earlier draft of this spec. `impl_for_predicate_trait_error`'s
   guard (`src/parser.rs:2440`, `if let TraitKind::Predicate(_) = ...`) and
   `colliding_name_kind`'s reserved-module check (`src/check/declarations.rs:305-307`, in
   `src/check/declarations.rs`, **not** the parser) both match on whatever is actually
   seeded in `RESERVED_TRAIT_MODULE` — once step 3 deletes the `Ord` seed entry, both stop
   firing for `Ord` and keep firing for `Copy` with zero code change, because the real
   `trait: Ord` declared in step 1 is an ordinary `TraitKind::Nominal` trait like any other.
   What *does* need work is the tests: delete
   `parse_impl_decl_for_reserved_ord_is_error` (`src/parser.rs:8333-8337`, nothing left to
   reject) and `parse_trait_ord_collides_with_the_reserved_predicate_entry`
   (`src/parser.rs:8289-8299`, nothing left to collide with); keep
   `parse_impl_decl_for_a_reserved_predicate_trait_is_error` (`:8323-8331`, `Copy`) and
   `parse_trait_copy_collides_with_the_reserved_predicate_entry` (`:8275-8287`, `Copy`)
   unchanged, and add an assertion to each confirming it still fires. Separately,
   `parse_capabilities_still_folds_copy_ord_byte_for_byte` (`src/parser.rs:8687`) asserts
   `sig.bounds == vec![(0, Bound::Copy), (0, Bound::Ord)]` — `Bound::Ord` will not exist to
   construct; rewrite the assertion to fold `'T: Copy` alone, plus a companion case
   confirming `'T: Copy SomeUserTrait` still folds to `[Bound::Copy, Bound::User(id)]`
   (`tests/phase7_slice3e.rs:144-150`'s `user_trait_named_copy_collides_with_the_reserved_entry`
   was checked as a candidate site and is a false lead — it tests a user `trait: Copy`
   collision, untouched by `Ord`'s removal; do not edit it).
5. R6: both admission sites as registry lookups, plus the overlap-check ruling above (a
   golden in Tests, below).
6. R7: `examples/poly_if.sth`'s two words drop `inline`; the two load-bearing Rust
   fixtures (`tests/phase4_generics.rs:1106`, `tests/phase7_slice3b_follow.rs:660`) have
   their bound rewritten to drop `Ord` rather than dropping `inline`, per R7's revision;
   `src/check/word_entry.rs:428`'s `EQ` witness drops `Ord` from its bound list; baseline
   regeneration.
7. C5: the roadmap's `&'T &'T` line corrected.
8. Diagnostics: `poly_ord_bound_error` deleted; an unsatisfied `Ord` becomes an ordinary
   user-trait failure naming the missing `impl:`. `unknown_capability_error`
   (`src/parser.rs:1674-1679`) still reads "a bound names `Copy`, `Ord`, or a trait in
   scope" after the flip — user-visible and wrong once `Ord` is an ordinary trait, not a
   reserved name. Reword to "a bound names `Copy` or a trait in scope" (the R8 REPL variant
   is separate wording, not this string).

Tests:

- Golden: a `'T: Copy Ord`-bounded generic word instantiated over a **user struct** with
  `impl: Ord for Point`, built and run. This is the slice's headline exit criterion.
- Golden: the `Vec2 lt` coexistence program from the brief — a concrete
  `lt ( Vec2 Vec2 -- Bool )` and the library generic `lt` in one module, dispatching
  correctly, `Vec2` carrying no `impl: Ord`. Mutation-tested at **both** R6 sites
  independently, per the brief's own finding that each fails differently (declaration time
  vs call site).
- Golden: the R6 overlap ruling — `impl: Ord for Vec2` plus a standalone concrete
  `lt ( Vec2 Vec2 -- Bool )` in **one module** (matching `check_generic_concrete_overlap`'s
  same-module scope) produces `generic_concrete_overlap_error` at declaration time. Assert
  the diagnostic fires; cite its fixed wording (`src/check/declarations.rs:1405-1413`) if
  convenient, not required.
- Golden: an unsatisfied `Ord` on a struct with no `impl:` produces a located error naming
  the missing `impl:`, asserted by exact text.
- Classification pass over the ~60 Rust-fixture `'T: ... Copy Ord` sites
  (`grep -rn "Copy Ord" tests/ src/ --include=*.rs`): confirm each migrates mechanically
  (bound stays, behaviour unchanged) versus needs the non-mechanical treatment this phase
  already found twice (the two `inline` fixtures above, the `EQ` witness above). Do not
  assume a raw grep count is a migration plan — this phase already falsified that assumption
  for 3 of the 5 `inline` sites found by the same method; treat the full 60 as unverified
  until each is checked. Record the count that needed non-mechanical treatment in the phase
  report.
- Every `tests/corpus_stdout/*.txt` unchanged. Every `'T: Copy Ord` fixture still compiles
  with **no new import line**.

Exit: all of the above; the classification pass is complete and its non-mechanical count is
stated; suite green; the phase report states the `.ssa` churn count and confirms zero `.txt`
churn.

### Phase 3 — REPL ruling, and the S3o handoff

Scope: R8 + R9. No checker behaviour change beyond the diagnostic.

Deliverables: the REPL's located `Ord`-at-REPL-scope diagnostic; the two stale comments
corrected; the differential-oracle harness skeleton and the roadmap note pointing S3o at
it; `docs/roadmap/P7-language-prereqs.md`'s S3s entry marked `[ done ]`.

Tests: a `tests/repl_ux.rs` case asserting the exact new REPL text; the harness runs and
reports a clean diff against itself (it has nothing to compare to until S3o flips `inline`
back, which is the point).

Exit: suite green.

## Exit criteria

| # | Criterion | Phase | Evidence |
| --- | --- | --- | --- |
| 1 | `Ord` bounds a struct or enum, satisfied nominally by `impl: Ord for Point`; a comparison-bounded generic word instantiates over a user type | 2 | run golden |
| 2 | A polymorphic body may call a polymorphic word carrying a `Bound::User` on a forwarded variable, through lowering, without ICE | 1 | run golden + `trait_calls` assertion |
| 3 | A reachable generic word's concrete-image cross-call on a type with no matching `impl:` is a located checker error, covered by R2's `compose` loop — not conditional on any code change to the `(Image::Concrete(_), Bound::User(_))` arm itself | 1 | error golden, mutation-tested |
| 4 | The numeric tower satisfies `'T: Ord` through ordinary `impl:` blocks in `core`, none written by the user | 2 | `lib/cmp.sth` + existing corpus |
| 5 | `Bound::Ord` and `is_ord` no longer exist | 2 | `grep -rn "Bound::Ord\|fn is_ord" src/` empty |
| 6 | The generic `lt` still does not swallow a user's concrete `Vec2 lt` | 2 | coexistence golden, mutation-tested at both sites |
| 7 | Every existing `'T: Copy Ord` **file** program still compiles with **no new import line** | 0 + 2 | corpus builds; `git diff` shows no added `import:` |
| 8 | Every existing `'T: Copy Ord` program produces the same results | 2 | `tests/corpus_stdout/*.txt` byte-identical |
| 9 | The built-in-predicate `impl:`/collision guards still fire for `Copy` | 2 | surviving `Copy` assertions |

Criterion 3 is conditional on reachability, not unconditional: a poly word whose
unsatisfiable cross-call obligation is never instantiated anywhere in the program is a
named, precedented residual gap (R3), not covered by any test in this slice — mirroring
the sibling `Bound::Copy`-on-body-local-instantiation arm's existing, shipped limitation.

Criterion 7 is scoped to **file** programs. The REPL is a known, ruled regression (R8) and
is deliberately not covered by it; the brief's unqualified wording did not account for C2.

Codegen regression is expected and accepted (criterion 8 covers behaviour, not IL).

## Residual risks

- **Phase 2's size.** Only steps 1, 3, and the behavioural half of step 5 are genuinely
  irreducible (see the phase's revised framing); the rest lands with them for review
  convenience. Still the phase most likely to hide a partial migration. The sub-step list
  exists so a reviewer can check each independently.
- **R3's residual gap is permanent, not a probe artifact to chase.** A poly word with an
  unsatisfiable cross-call obligation that is never instantiated anywhere in the program
  stays unchecked, precedented by the sibling `Bound::Copy` arm. Named explicitly in the
  phase report each time this slice's tests run, not something a future probe is expected
  to close.
- **Baseline regeneration is the classic place a real regression hides.** Mitigated by
  requiring zero `.txt` churn and a stated `.ssa` count.
- **The ~60 `Copy Ord` fixture sites are assigned to Phase 2's Tests as a classification
  pass, not left as an unassigned risk.** A raw grep count has overstated a migration in
  this repo before, and this spec's own review already found 3 real counter-examples (two
  `inline` fixtures, one witness constant) among just the 5 `inline`-tagged sites checked by
  the same method — the remaining ~55 are unverified, not presumed mechanical.

  **Classification pass, completed (review cycle 2).** The raw grep over `src/`+`tests/`
  `*.rs` finds 65 lines at the parent-plus-flip tree, but that is not 65 migration sites:
  2 match `Copy Order`, an unrelated *user* trait, and 28 are prose in comments. The real
  input was **35 fixture/code sites**, of which **4 needed non-mechanical treatment** and
  31 migrated mechanically (bound unchanged, behaviour unchanged): `src/check/word_entry.rs`'s
  `EQ` witness and `tests/phase4_generics.rs` + `tests/phase7_slice3b_follow.rs`'s two
  `mymax` fixtures all had `Ord` dropped from the bound rather than `inline` dropped (R7's
  revision), and `src/parser.rs`'s `parse_capabilities_still_folds_copy_ord_byte_for_byte`
  had its assertion rewritten because `Bound::Ord` no longer exists to construct.

  A **fifth** non-mechanical site exists that this grep cannot find, and it is the reason
  the grep-count plan was the wrong instrument: `tests/phase7_slice3e.rs`'s `sort3` fixture
  declared its own `type: Ordering | Less | Equal | Greater ;`, which the flip's new
  `core::cmp` variant names capture. It was migrated to `Rank | Under | Same | Over` and is
  invisible to a `Copy Ord` grep, since its bound is `'T: Copy Order` on a user trait. Its
  necessity is verified by reverting it (the build then fails inside `lib/cmp.sth`'s own
  `impl: Ord for i8`), not assumed. See the variant-name reservation risk below.
- **The flip reserves `Less`, `Equal` and `Greater` as variant names program-wide, found in
  review cycle 2.** A variant constructor's env key is the bare surface name with no module
  in it (`enum_generated_sigs`, `src/check/declarations.rs`), so a user enum with a variant
  named `Less`, `Equal`, or `Greater` — any one of the three, individually — captures the
  constructor `lib/cmp.sth`'s `impl: Ord` bodies use, and the build fails inside `cmp` with
  a confusing "body leaves `Ordering` where the declaration requires `Ordering`". The type
  name `Ordering` itself is fine; only the variants collide.

  The module-blind key is **pre-existing, not caused by this slice**: verified at the parent
  commit, where two *user* modules each declaring a variant `Less` collide identically, with
  `core::cmp` carrying no `Ordering` at all. What the flip changes is that one colliding
  party is now a module every program reaches through the prelude, so three previously-safe
  names became unusable. Fixing it means keying variant constructors by module — a real
  design change to already-shipped machinery, and not this slice's scope. Named here rather
  than absorbed silently, and deliberately not pinned by a test, since the current behaviour
  is the bug rather than the contract.
- **The `+86.6%` comparison tax** is real and user-visible until S3o lands. It is accepted
  for one slice; if S3o stalls again, this becomes a standing cost worth re-litigating.
- **A generic cross-call inside a spliced combinator's own body is invisible to lowering,
  found during review.** `lib/combinators.sth`'s `times-helper` calls the library `lt`
  internally (`from to lt`); now that `lt` is a real generic call (R5) rather than spliced,
  that cross-call needs an instantiation lowering can look up. It has none: combinators are
  excluded from cross-call discovery entirely (P7.S3e's documented "R9 scope cut" --
  `check_poly_combinator_standalone` records no `PolyCrossCall`, `src/check.rs:813`), a
  pre-existing limitation from an earlier, already-shipped phase that this slice's
  comparisons flip newly exposes rather than causes: `lt` was always spliced inline before,
  so no instantiation lookup was ever needed for a call reached this way. Reproduces as an
  `Option::expect` panic at lowering (`checked user word exists`,
  `src/ir/func_builder/calls.rs:737`) for *any* non-inline generic body that uses `times`,
  `each`, `map`, `fold`, or `filter` (all route through `times-helper`); `while` is
  unaffected (no internal comparison). **Not a corpus regression**: every shipped example
  and golden calls comparisons from a body's own top level or from `if`/`unless`'s arms,
  never from inside a *combinator's own declared body* — `examples/poly_if.sth`'s `gt` sits
  at `mymax`'s top level and still builds. The three `tests/phase7_slice3b_follow.rs` goldens
  that do exercise this combination (`times_in_a_non_inline_generic_body_compiles_and_runs`,
  `clampsum_golden_behavioural_matrix`,
  `clampsum_structural_characterization_one_definition_per_instantiation`) are marked
  `#[ignore]` with this finding cited, rather than fixed here: closing it means widening
  `check_poly_combinator_standalone`/P7.S3k's cross-call discovery to walk into a
  combinator's own body too, a change to already-shipped P7.S3e/P7.S3k machinery and a
  separate slice's scope, not this one's.
