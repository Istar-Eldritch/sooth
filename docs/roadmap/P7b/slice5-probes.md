# P7b.S5 probe round — verbatim log

Recon round for P7b.S5 scoping, against the clean tree (worktree `p7b-s5`, HEAD
`4cfb887`, P7b's S2/S3/S4 lines landed). Read-only against the repo; probe fixtures
live under `/tmp/p7bs5-probes/`. `git status --porcelain` is empty before, during
(after every mutation restore), and after this round.

Baseline: `cargo test --no-fail-fast` at HEAD is **green** (every test binary printed
`test result: ok`, 0 failed across the full suite).

## Summary table

| Probe | Fixture | Outcome |
| --- | --- | --- |
| pc | `pc_nested_receiver_wrong_var.sth` | confirms (c): trait header var `'G`, error text still says `'F` |
| pa3 | `pa3_singlefile.sth` | single-module generic-target inline-free member: builds+runs, prints `7` — control |
| pa2 | `pa2/main.sth` | two-module mono caller, cross-module generic impl target: **`mono_member_no_dispatch_error`**, not `mono_member_unroutable_error` |
| pa4 | `pa4/main.sth` | variant of pa2 with export-list adjustments: blocked earlier, at export-visibility of the concrete instantiation |
| pb | `pb/main.sth` | two modules, identical-shaped ctor `Widget['T] v 'T`, same field type: **type mismatch**, not a silent cross-pick |

## pc — the hardcoded `'F` (finding c)

```sth
import: intrinsics * ;
trait: Sized['G] :
  size ( i64 -- i64 ) ;
;
: main ( -- ) 3 . ;
```

```text
$ cargo run -- build pc_nested_receiver_wrong_var.sth
error: trait member `size` (line 3, col 3) has no input for a call to dispatch on
  (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
```

The trait header declares `'G`, not `'F`; the diagnostic names `'F` regardless.
Confirms the task brief's (c) as-is. Anchor: `nested_receiver_member_error`
(`src/check/declarations.rs:444`), literal `'F` baked into the format string.

## pa3 — single-module control (generic impl target, non-inline member)

```sth
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
trait: Sized['S] : size ( 'S -- i64 ) ; ;
impl: Sized for Box['T]
  : size drop 7 ;
;
: usesize ( Box[i64] -- i64 ) size ;
: main ( -- ) 3 Box usesize . ;
```

```text
$ cargo run -- run pa3_singlefile.sth
7
```

A mono caller (`usesize` has no bound, no `'F: Sized` parameter) dispatching a
generic-target member within one module works fine. This is the baseline
`resolve_mono_member_call`'s generic-impl branch (`src/check/poly.rs:2383-2398`)
exercises cleanly when `poly.env.contains_key(&word_sym)` is true.

## pa2 — two-module mono caller, attempting to reach `mono_member_unroutable_error`

`pa2/f.sth` (shared trait module):

```sth
import: intrinsics * ;
trait: Sized['S] : size ( 'S -- i64 ) ; ;
export: Sized ;
```

`pa2/a.sth` (impl module):

```sth
import: intrinsics * ;
import: self::f * ;
type: Box['T] v 'T ;
impl: Sized for Box['T]
  : size drop 7 ;
;
export: Box ;
```

`pa2/main.sth` (mono caller, no bound):

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: self::f * ;
import: self::a * ;
: usesize ( Box[i64] -- i64 ) size ;
: main ( -- ) 3 Box usesize . ;
```

```text
$ cargo run -- run pa2/main.sth
error: `size` in `usesize` (line 5, col 31) is a trait member of Sized, but no `impl:` in this program dispatches on these operands
  the operand types here are `Box[i64]`; declare an impl of one of those traits for the operand's type, or import a word that claims this name
```

This is `mono_member_no_dispatch_error` (`src/check/poly.rs:2233-2246`), the *zero
viable candidates* branch — `find_bound_impl` (`src/check/poly.rs:8110`) never
matched `Box['T]`'s target pattern against the `Box[i64]` operand at all, so the
call never reached the `mono_member_unroutable_error` guard three branches later.
This is the pre-existing cross-module generic-instantiation gap the standing memory
note `project_generic_instantiation_cannot_cross_modules` names (confirmed live, not
stale): `export:` cannot name a generic's concrete instantiation across a module
boundary, and the caller's `Box[i64]` (built from `main`'s own import of `a::Box`)
and the impl's target pattern do not unify as the same registry identity.

## pa4 — same shape, exporting the instantiation explicitly

```sth
export: mkbox Box Box[i64] ;
```

```text
error: parse error: expected `;` terminating `export:`, found LBracket at line 8, col 22
```

`export:` cannot even spell a bracketed instantiation — confirms there is no
available workaround through the export list; the S4-era gap is a hard wall here,
not a spelling problem.

## Reading: is `mono_member_unroutable_error` reachable at all?

Every attempt to reach the generic-impl branch of `resolve_mono_member_call`
(`src/check/poly.rs:2383-2398`) cross-module was intercepted first by
`mono_member_no_dispatch_error`, because `find_bound_impl` is whole-program
(`src/check/poly.rs:8110`, iterates `tr.impls` with no module filter) but a
*successful* match still requires the operand's concrete `Box[i64]` and the impl's
target pattern to resolve to the same registry `StructId` — which the
generic-instantiation-across-modules gap prevents. No unit test in the tree
exercises `mono_member_unroutable_error` directly (`grep -rn
"mono_member_unroutable" tests/` finds only a comment in `phase7b_slice2.rs:633`
describing golden #10's workaround, no assertion). Given `check.rs`'s `poly_env` is
built once, whole-program, from the fully `assemble_module`-flattened `Module`
(`src/driver.rs:512`, `src/check.rs:684-698`) — every word in the closure,
regardless of which file declared it, ends up in the one `poly_env` the checker
sees — the "not visible from this module" framing in the error's own doc comment
(`src/check/poly.rs:2422-2426`) describes a per-module `poly_env` split that does
not exist in the current architecture. This needs to be a design question for the
spec (see brief Q1), not asserted as a live routing bug: the round could not
construct a case where the impl is found (`viable.len() == 1`) *and* the resulting
`word_sym`/`symbol` is absent from `poly.env`.

## pb — module-blind ctor collision, live check

`pb/f.sth`:

```sth
import: intrinsics * ;
trait: Functor['F] : size ( 'F -- i64 ) ; ;
: sized['F: Functor] ( 'F -- i64 ) size ;
export: Functor sized ;
```

`pb/a.sth`:

```sth
import: intrinsics * ;
import: self::f * ;
type: Widget['T] v 'T ;
impl: Functor for Widget
  : size drop 1 ;
;
: mk ( i64 -- Widget[i64] ) Widget ;
: run ( i64 -- i64 ) mk sized ;
export: run ;
```

`pb/b.sth` — byte-identical shape, `size drop 2`, same `Widget['T] v 'T`, same `mk`.

`pb/main.sth`:

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: self::f * ;
import: self::a ;
import: self::b ;
: main ( -- ) 5 a::run . 5 b::run . ;
```

```text
$ cargo run -- run pb/main.sth
error: type mismatch in `mk` (line 7)
  body leaves `Widget[i64]` where the declaration requires `Widget[i64]`
  note: declared ( i64 -- Widget[i64] )
```

Deterministic across repeated runs (checked twice, exit code 1 both times). This
answers the task's "determine which" question directly: two identically-shaped,
same-named ctors across modules do **not** silently cross-pick and do **not**
compute a bogus result — they are rejected by the checker's own type-identity
comparison (`match_slot`'s `Exact` match, keyed on `Type` equality, which is
`StructId`-keyed, not name-keyed). What breaks is the **diagnostic surface**: the
error prints the identical rendered name (`Widget[i64]`) on both sides, because the
message's rendering path is `Type::name()`, which is module-blind (per the standing
notes at `ast.rs:766`, `ast.rs:5229`, `driver.rs:1171`) — so a user reading this
error cannot tell that two distinct `Widget[i64]`s are involved, only that
*something* doesn't match. Golden #10 (`same_named_ctors_in_two_modules_dispatch_distinct_impls`,
`tests/phase7b_slice2.rs:664`) avoids this entirely by giving `b`'s `Widget` a `str`
payload instead of `i64` — the documented workaround the task brief names.

### Root of the `pb` mismatch (traced, not fixed)

- Struct decl names are mangled per module before check runs
  (`src/resolve.rs:799`, `mangle(&s.name, s.module)`), so module `a`'s `Widget`
  header and module `b`'s `Widget` header get distinct `decl.name`s
  (`Widget__m<N>`), hence distinct registry `StructId`s once each is instantiated
  at `[i64]`.
- The **env key** the ctor call `Widget` resolves through is
  `generic_surface_name(&decl.name)` (`src/check/declarations.rs:1815-1839`,
  `src/ast.rs:848`), which strips only the `[...]` instantiation suffix, not the
  `__m<N>` module tag — so the two modules' `Widget` ctors register under
  *different* bare env keys (`Widget__m1`, `Widget__m2`) after module mangling, and
  each module's own body resolves `Widget` to its own module's mangled name via the
  ordinary import-scoped name rewrite the resolve pass already does for words.
- So the ctor *lookup* itself is not the module-blind step in this fixture; the
  module-blind step is purely in **rendering**: `Type::name()` on the resulting
  `Type::Struct(StructId, name_static)` prints `decl.name`'s surface spelling
  (post `generic_surface_name`, i.e. with the module tag *also* stripped for
  display) for both, even though the two `StructId`s differ. That is consistent
  with every other "module-blind" note in the tree: the *identity* is
  `StructId`-correct, the *display* collapses two distinct identities to one
  string.

## Reproduction

```sh
cd /root/code/ordfruma/sooth-worktrees/p7b-s5 && cargo build
cargo run -- run /tmp/p7bs5-probes/pc_nested_receiver_wrong_var.sth   # build-only, expect reject
cargo run -- run /tmp/p7bs5-probes/pa3_singlefile.sth                 # prints 7
cargo run -- run /tmp/p7bs5-probes/pa2/main.sth                       # mono_member_no_dispatch_error
cargo run -- run /tmp/p7bs5-probes/pb/main.sth                        # type mismatch, Widget[i64]/Widget[i64]
```

No mutation experiments were run against `src/`: the round could not construct a
live case that reaches the `mono_member_unroutable_error` guard at all (every
attempt was intercepted upstream by the cross-module generic-instantiation gap), so
there was no reachable code path to spike a routing fix against. Spiking a fix for
an error path with no known live trigger would not have produced attributable
findings; this is itself the round's headline result for (a) — see brief Q1.

## Round 2 (2026-09-03) — correcting the (b) fails-closed claim

Follow-up recon round after a spec review reproduced a silent cross-pick the
Round 1 `pb` fixture did not exhibit. Fixtures under `/tmp/p7bs5-probes2/`. Repo
untouched except one immediately-reverted mutation spike (recorded below, restore
confirmed both before and after). `git status --porcelain` empty throughout.

Baseline re-confirmed: `cargo test --no-fail-fast` at HEAD `4cfb887` is green (1858
lib tests + every integration binary `ok`, 0 failed).

### pb2 — the reviewer's exact shape: live silent cross-pick

The difference from Round 1's `pb`: `a`'s ctor consumer (`run`) never spells the
full instantiation `Widget[i64]`; `b`'s does (`usesize`'s declared parameter type).

`pb2/f.sth`:

```sth
import: intrinsics * ;
trait: Sized['S] : size ( 'S -- i64 ) ; ;
: sized['S: Sized] ( 'S -- i64 ) size ;
export: Sized sized ;
```

`pb2/a.sth`:

```sth
import: intrinsics * ;
import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget
  : size drop 1 ;
;
: run ( i64 -- i64 ) Widget sized ;
export: run ;
```

`pb2/b.sth`:

```sth
import: intrinsics * ;
import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget
  : size drop 2 ;
;
: usesize ( Widget[i64] -- i64 ) size ;
: run ( i64 -- i64 ) Widget usesize ;
export: run ;
```

`pb2/main.sth`:

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: self::f * ;
import: self::a ;
import: self::b ;
: main ( -- ) 5 a::run . 5 b::run . ;
```

```text
$ cargo run -- run pb2/main.sth
2
2
```

Both `a::run` and `b::run` print `2` (b's impl). `a::run` should print `1` — it
never sees `b`'s impl in source, its own module declares its own `Widget` with
`size drop 1`. **Silent wrong answer, no diagnostic, exit 0.** Reproduced
deterministically (repeat runs, same result). This directly falsifies Round 1's F2
("does not silently cross-pick") and the spec's R3/R4 built on it: the collision
*is* live, and it is a soundness bug, not only a diagnostic-legibility gap. The
determining factor Round 1 missed: whether the *consumer* word's signature spells
the instantiation out loud (`Widget[i64]` in a declared type) rather than only
the checker inferring it from the ctor's own body.

### Root cause, traced end to end

**1. `generic_structs`/`generic_enums` are never mangled.** `resolve.rs`'s
module-scoping pass (`~resolve.rs:798-803`) mangles `module.structs` and
`module.enums`:

```rust
for s in &mut module.structs {
    s.name = mangle(&s.name, s.module);
}
for e in &mut module.enums {
    e.name = mangle(&e.name, e.module);
}
```

`grep -n "generic_structs\|generic_enums" src/resolve.rs` shows no analogous loop —
the only other touches are a module-filtered iteration at `resolve.rs:586,593` and
two `Vec::new()` initializers. So `module.generic_structs[i].name` stays the bare
declared spelling (`"Widget"`) in *both* `a` and `b`, unlike concrete `structs`.
This directly **falsifies Round 1's "Root of the `pb` mismatch" trace**, which
claimed the two modules' `Widget` ctors register under different mangled env keys
(`Widget__m1`, `Widget__m2`) — that claim assumed `generic_structs` gets the same
mangle pass `structs` does; it does not, so the env keys are identical bare
`"Widget"` strings, not merely identically-rendered ones.

**2. `instantiate_struct` mints the instantiation name from the unmangled header.**
`ast.rs:1310-1359`'s `instantiate_struct` builds `name =
type_instantiation_name(&self.structs[idx].name, args, lens, ...)` where
`self.structs[idx]` is the `GenericStructDecl` — the unmangled header from step 1.
So `a`'s `Widget[i64]` and `b`'s `Widget[i64]` both mint the literal string
`"Widget[i64]"`.

**3. The dedup key (`struct_keys`) does include `module`, so this alone would not
collapse the two `StructId`s.** `instantiate_struct`'s memo key is `(idx, module,
args, lens)` (`ast.rs:1334-1335`), and `idx` here indexes into `generics.structs`,
which is the *generic-header* registry — `a`'s `Widget` header and `b`'s `Widget`
header are separate `GenericStructDecl` entries (assembled per-module, `driver.rs
~800`), so they get separate indices, and `instantiate_struct` mints two distinct
`StructId`s, one per module. Confirmed indirectly: `driver.rs:1114`'s existing test
`instantiate_struct_distinct_across_modules_same_bare_name` proves cross-module
same-bare-name generics mint distinct `StructId`s and distinct rendered names *in
that fixture's shape* (a header `Box` local to one module, instantiated over two
modules' `P`). That fixture is not this bug's shape: there the *header* `Box` is
single-module; here the *header* `Widget` itself is redeclared per module, giving
two `GenericStructDecl` entries whose names collide only in surface spelling, not
registry identity.

**4. The actual collision is at env-key overload resolution, module-blind and
output-blind.** `struct_generated_sigs` (`declarations.rs:1815-1839`) registers each
minted ctor under `env[generic_surface_name(decl.name)]` — `"Widget"` for both
`a`'s and `b`'s instantiation, *appended* (not inserted) specifically to avoid
clobbering (comment at `check.rs:582-587`), so `env["Widget"]` legitimately holds
two `Overload`s, one per module, each with input `[i64]` but a different output
`Type::Struct(StructId, "Widget[i64]")`. The call-site resolution
(`check/terms.rs:915-928` single-candidate arm is skipped since there are 2; the
multi-candidate arm at `check/terms.rs:955-963`) is:

```rust
let hit = candidates.iter().find(|o| {
    operands.len() >= o.sig.inputs.len()
        && operands[operands.len() - o.sig.inputs.len()..] == o.sig.inputs[..]
});
```

This matches on **input signature only** — `sig.outputs` never enters the
comparison, and neither does the caller's own module. Both `a`'s and `b`'s
`Widget` overloads have input `[i64]`, so `.find` returns whichever is **first in
the `Vec`**, deterministically, by declaration/module-assembly order — not by
which module the call site lives in. This is the true collision site: not identity
(the `StructId`s are already distinct per step 3), not rendering (Round 1's F3 was
half right, half a distraction), but **overload disambiguation blind to both output
type and module**.

### Fix-shape spike (immediately reverted)

**Attempt: mangle `module.generic_structs`/`module.generic_enums` names like
`structs`/`enums`** (Task A's Option 1). Patch to `src/resolve.rs`:

```rust
for s in &mut module.structs {
    s.name = mangle(&s.name, s.module);
}
for e in &mut module.enums {
    e.name = mangle(&e.name, e.module);
}
for s in &mut module.generic_structs {
    s.name = mangle(&s.name, s.module);
}
for e in &mut module.generic_enums {
    e.name = mangle(&e.name, e.module);
}
```

```text
$ cargo run -- run /tmp/p7bs5-probes2/pb2/main2.sth
2
2
```

**Does not fix the bug.** Still `2 2` — confirms the collision is not primarily a
naming/mangling problem; per the trace above, the `StructId`s were already
distinct, so mangling the header name changes what string gets minted
(`"Widget__m1[i64]"` vs `"Widget__m2[i64]"`) but the overload-resolution bug at
`check/terms.rs:955` still picks the first `Vec` entry by input-signature match
alone, regardless of which name each entry carries.

**Collateral**: `cargo test --no-fail-fast --lib` — 1 failure:
`driver::tests::whole_closure_generic_pre_pass_registers_each_header_once` panics
at `driver.rs:1426`, `` assertion `left == right` failed: `Box` is registered once,
not per pass `` (`left: 0, right: 1`) — a pre-pass dedup keyed on the header's bare
name now sees two distinct mangled names and stops deduping.
`cargo test --no-fail-fast --test phase5_slice1` — 2 failures:
`generic_enum_header_colliding_with_a_concrete_type_is_a_duplicate` and
`generic_header_colliding_with_a_concrete_type_is_a_duplicate`, both because
`check_duplicate_type_names` (`declarations.rs:1231`) keys its collision check on
`(decl.module, decl.name.as_str())` for `generic_structs`/`generic_enums` directly
against the *concrete* `structs`/`enums` bare names — mangling only the generic
side breaks that comparison's premise that both sides use the same bare spelling.

Reverted: `git checkout -- src/resolve.rs`; confirmed `git status --porcelain`
empty; `cargo build` succeeds; `cargo test --no-fail-fast` green (1858 lib tests,
every integration binary `ok`, 0 failed) both before and after this spike.

**What a real fix needs, traced but not built**: the mangle-copy alone is
necessary-but-not-sufficient and, done naively, actively regresses the two sites
above. A real fix needs at minimum: (a) the mangle (or an equivalent module tag)
on the generic header, sized against `whole_closure_generic_pre_pass_registers_
each_header_once` and `check_duplicate_type_names`'s premises, and (b) widening
the overload-candidate match at `check/terms.rs:955` (and its `poly.rs:3202/3258`
analogues, unaudited this round) to break ties on the caller's own module identity
or the candidate's output type, not input signature alone. Sizing (b) is a real
design question — it changes general overload resolution, not just this
ctor-collision shape — and is out of this round's scope to spec.

### B — blast radius: golden #10 and the wider suite

No spike above touched `tests/`, and every spike was reverted before running the
full suite. `cargo test --no-fail-fast` is green at HEAD both before and after
every mutation in this round (measured 3 times total: pre-spike, post-mangle-spike-
pre-revert failure enumeration above, post-revert). Golden #10
(`same_named_ctors_in_two_modules_dispatch_distinct_impls`,
`tests/phase7b_slice2.rs:664`) was not independently re-run in isolation beyond
being part of the full-suite green count above; its `i64`/`str` payload split
means its two ctors never collide on `sig.inputs`, so it is not expected to
exercise the `check/terms.rs:955` collision site at all — consistent with it
passing unmodified through every spike.

### pb3/pb4 — R4's scope-fence claim (per-call-shape diagnostic identity)

`pb3` reproduces Round 1's exact `pb` fixture verbatim (both `mk` bodies leave the
instantiation unspelled): confirms the same `check.rs:1506`-style
`type_mismatch_error`... actually the `check_outputs`/`SlotMatch::Mismatch` shape:

```text
$ cargo run -- run pb3/main.sth
error: type mismatch in `mk` (line 7)
  body leaves `Widget[i64]` where the declaration requires `Widget[i64]`
  note: declared ( i64 -- Widget[i64] )
```

`pb4` — both modules' consumer word **spells** the full instantiation as a declared
parameter type (`usesize ( Widget[i64] -- i64 )`), both calling `Widget usesize`
directly rather than through an intermediate `mk`:

```text
$ cargo run -- run pb4/main.sth
error: type mismatch in `run` (line 8)
  `usesize` expected `Widget[i64]`, found `Widget[i64]`
  note: declared ( i64 -- i64 )
```

**This is a different message, from `type_mismatch_error` (`src/check.rs:1506`,
`` `{op}` expected `{expected}`, found `{found}` ``), not `check_outputs`'s
`SlotMatch::Mismatch` arm (`src/check.rs:1361`, `` body leaves `{X}` where the
declaration requires `{Y}` ``).** Confirms the review's finding 3: the same
underlying collision reaches at least two distinct diagnostic-rendering call
sites depending on the call shape (ctor-body-vs-declared-output vs
operand-vs-parameter). R4's fence to "the ctor-generated type-mismatch message
only" (`check_outputs`'s arm) misses this second site entirely — a Phase 2 fix
scoped only to `check_outputs` would leave `pb4`'s shape rendering the
undisambiguated message unchanged. (Both `pb3` and `pb4` are still, per this
round's headline finding, cases the checker *rejects* — the disambiguation gap is
real in both, but `pb2` shows the same underlying collision has a third,
*unrejected*, silently-wrong shape that neither R3 nor R4 anticipated.)

### C — Phase 3 per-site verdict: poly.rs:1915 vs poly.rs:2390

Current line numbers (`mono_member_unroutable_error` call sites; text unchanged
since Round 1, minor line drift):

- **`poly.rs:2390`** (mono generic-target branch, `resolve_mono_member_call`'s
  non-inline path): unreachability re-confirmed this round on the same grounds as
  Round 1 (F4/F5) — `poly_env` is whole-program (`check.rs:684-698`), and no
  cross-module fixture in either round reached this branch without first hitting
  `mono_member_no_dispatch_error`. No new evidence this round changes that
  verdict.
- **`poly.rs:1915`** (inline re-entry path, `!tr.impls[imp_idx].target.is_concrete()`
  guarded by `poly.env.contains_key(&symbol)`): attempted a live trigger via direct
  self-recursion in an inline generic-target trait member:

  ```sth
  trait: Sized['S] :
    size inline ( 'S -- i64 ) ;
  ;
  type: Box['T] v 'T ;
  impl: Sized for Box['T]
    : size drop 1 1 Box size drop ;
  ;
  : usesize ( Box[i64] -- i64 ) size ;
  : main ( -- ) 3 Box usesize . ;
  ```

  ```text
  $ cargo run -- run pa5_inline_reentry/main.sth
  error: an always-spliced word cannot be recursive (the inliner would splice it
    forever): `size` (member of trait `Sized` for `Box['T0]`) -> `size` (member of
    trait `Sized` for `Box['T0]`) (line 8, col 5)
  ```

  Direct self-recursion is intercepted by a **different, dedicated guard**
  upstream of `poly.rs:1915` (a distinct "always-spliced word cannot be recursive"
  check), not the guard itself. This round did not find a live trigger for
  `poly.rs:1915` specifically — mutual recursion between two inline generic-target
  members was attempted but blocked on trait-member declaration syntax within this
  round's time budget (leading-colon requirements for a second member proved
  inconsistent with the single-member form and were not resolved). **Inconclusive**
  for `poly.rs:1915`: neither a live trigger nor a positive unreachability argument
  was established this round; the "always-spliced word cannot be recursive" guard
  is a plausible but unconfirmed candidate for making this site permanently
  unreachable via any recursive shape (it would need to be shown that *every* path
  to `poly.rs:1915`'s branch goes through that recursion check first, which this
  round did not attempt).

### D — smaller P1s

- **Third test site for the `'F` diagnostic text**: `grep -rn "expected the
  trait's variable" .` finds exactly the two `tests/` sites the spec's R2 already
  enumerates (`tests/phase7_slice3t.rs:505`,
  `tests/phase7b_slice2.rs:158`/`hkt_member_without_dispatchable_input_is_located_
  error`) plus the source site itself (`src/check/declarations.rs:447`) and the
  docs referencing it. **No third test site** — `declarations.rs:3875` was
  suggested as a location to check specifically; the file's current line 3875 is
  unrelated content, and no test named
  `check_trait_decls_rejects_a_receiver_nested_in_an_array_input` (or similar)
  asserts this literal text. R2's "exactly two" claim holds.
- **`TraitDecl` has no header-variable-name field**: confirmed by direct read
  (`src/ast.rs:1952-1972`) — fields are `name: String`, `kind: TraitKind`,
  `var_kind: Kind`, `var_span: Span`, `members: Vec<TraitMember>`, `module: u32`,
  `span: Span`. No `var_name` or equivalent. The header variable's *name* (as
  opposed to its `Kind`/`Span`) is reachable only via `member.sig.ty_var_names[0]`
  on one of `members`, confirming R1's fix plan is sound on this point.

### Reproduction

```sh
cd /root/code/ordfruma/sooth-worktrees/p7b-s5 && cargo build
cargo run -- run /tmp/p7bs5-probes2/pb2/main2.sth   # 2, 2 — silent cross-pick
cargo run -- run /tmp/p7bs5-probes2/pb3/main.sth    # check_outputs mismatch shape
cargo run -- run /tmp/p7bs5-probes2/pb4/main.sth    # type_mismatch_error shape
cargo run -- run /tmp/p7bs5-probes2/pa5_inline_reentry/main.sth  # recursion guard, not :1915
```
