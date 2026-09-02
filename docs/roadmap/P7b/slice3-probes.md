# P7b.S3 probe round — verbatim log (run 260901)

Recon round for P7b.S3 scoping, run 260901 against the clean tree (worktree `p7b-s3`,
HEAD `403618f`, P7b.S2 landed). One worker: a read-only extension-point mapper plus a
live-probe runner. Probes are compile/run fixtures under `/tmp/p7bs3-probes/` with
per-probe captures under `/tmp/p7bs3-probes/logs/`; the repo was untouched throughout
(`git status --porcelain` empty at finish, re-verified after every mutation restore).

Baseline: `cargo test --no-fail-fast` at HEAD is **green — 3044 passed, 0 failed**, so
every rejection below is attributable to a missing S3 surface, not baseline breakage.

Fixtures carry a local `sooth.pkg` (`package: p7bs3 ; layer: hosted ;` with core and
hosted path deps) and the two-line prelude (`import: intrinsics * ;` /
`import: hosted::show | . | ;`), matching `tests/phase7b_slice2.rs`'s
`single_file_hosted` helper exactly, so probe sources are golden-ready as written.

Every HKT probe below is paired with a **non-inline twin** differing only in the
`inline` keyword. That pairing is the whole attribution method: it separates "S3's
surface is missing" from "this shape never worked".

## Summary table

| Probe | Fixture | Outcome |
| --- | --- | --- |
| p0 W4 control | p0_w4_control | compiles+runs, prints `-1` / `3` — S2's shared-bound golden re-proven on this tree |
| p1 ordinary trait, **concrete** target, `inline` member | p1_ordinary_inline_bound | **compiles+runs**, prints `7` — MEMORY's "inline member panics at lowering" is **stale**; the fix landed |
| p2 HKT `map inline`, mono call | p2_hkt_inline_mono | **rejected** at check: R3 "grounding a generic over its own type variable is not yet implemented" |
| p2b twin, no `inline` | p2b_hkt_noninline_mono | compiles+runs, prints `2` — clean attribution: the `inline` keyword alone flips it |
| p3 ordinary Star trait, **generic** target, `inline` member | p3_ordinary_inline_generic_target | **rejected**, identical R3 error — the blocker is **not HKT-specific** |
| p3b twin, no `inline` | p3b_ordinary_noninline_generic_target | compiles+runs, prints `1` |
| p4 HKT `map inline` through shared bound (the S3 exit shape) | p4_hkt_inline_shared_bound | under m1: **second** blocker, `binds no word for member 'map'` |
| p5 HKT Functor over a generic **struct** (no enum elim), `inline` | p5_hkt_inline_struct | rejected: `Box>` not permitted on a generic type |
| p5b twin, no `inline` | p5b_hkt_noninline_struct | **identically rejected** — pre-existing, not S3's; the struct route to `Functor` is closed either way |
| p6 real `core::option` ctor impl | p6_real_option_ctor_impl | `no impl: in this program dispatches on these operands` — S4's gap is live; S3 must use fixture twins |
| p7 ordinary trait, concrete target, `inline`, symbol check | p7_ordinary_trait_inline_concrete | member **spliced**: no `size;Sized;0;Boxi` symbol minted |
| p8 `core::cmp`'s `lt` (poly + `inline` + bound) | p8_cmp_precedent | **spliced**, no symbol — the precedent S3's dispatch should follow |
| m1 R3 gate neutralized | mutation, check/poly.rs:566 | p2/p3 compile and run, **same output as their non-inline twins** |
| m2 tagged-diagnostic build | mutation, check/poly.rs | localized p4's failure to the `word_sig_of` miss (`[TAG7981]`) |
| m3 member combinators recorded in the poly pre-pass | mutation, check.rs:837 | p4 advances past the miss; exposes R1.5's gate — the gap check.rs's own comment predicted |
| m4 m1+m3 + both R1.5 gates stubbed | mutation, check/poly.rs:4151/5611 | p2/p4 build and print **correct** values — but `nm`/`objdump` show **no splice happened** (see the reading below) |

## Verbatim captures

### p0 — W4 control (S2's shared-bound golden, re-proven)

```sth
type: Opt['T 'E] | None | Some 'T 'E ;
type: Res['T 'E] | Ok 'T | Err 'E ;
trait: Functor['F: * -> * -> *] :
  map ( 'F['T 'E] [ 'T -- 'U ] -- 'F['U 'E] ) ;
;
impl: Functor for Opt
  : map swap ~[ ( Some ) Some> swap rot call swap Some ] ~[ ( None ) drop drop None ] Opt? ;
;
impl: Functor for Res
  : map swap ~[ ( Ok ) Ok> swap call Ok ] ~[ ( Err ) Err> swap drop Err ] Res? ;
;
: twice['F: Functor 'T 'E] ( 'F['T 'E] [ 'T -- 'T ] -- 'F['T 'E] )
  | q |
  q map
  q map ;
: showopt ( Opt[i64 i64] -- ) ~[ ( Some ) Some> drop . ] ~[ ( None ) drop ] Opt? ;
: showres ( Res[i64 i64] -- ) ~[ ( Ok ) Ok> . ] ~[ ( Err ) Err> . ] Res? ;
: mkopt ( i64 -- Opt[i64 i64] ) dup Some ;
: mkres ( i64 -- Res[i64 i64] ) Ok ;
: main ( -- ) 1 mkopt [ 1 sub ] twice showopt
  5 mkres [ 1 sub ] twice showres ;
```

```text
--- build exit: 0 ---
-1
3
```

### p1 — the MEMORY control: an `inline` member on a concrete target builds today

```sth
trait: Sized['S] :
  size inline ( 'S -- i64 ) ;
;
type: Boxi v i64 ;
impl: Sized for Boxi
  : size drop 7 ;
;
: usesize['S: Sized] ( 'S -- i64 ) size ;
: main ( -- ) 3 Boxi usesize . ;
```

```text
--- build exit: 0 ---
7
```

The prior-session note "an `inline` trait member is checker-ready but panics at lowering
with `checked resolved call exists`" is **stale**. The combinator splice path it
prescribes is present and documented at `src/ir/func_builder/calls.rs:170-228`
(`P7.S3s-follow Phase 4`, uid rule `P7.S8 R1`), and it is more careful than the note:
the uid comes from `member_uid_seeds` first, falling back to
`splice_uid_stack.last()` only on a lookup miss.

### p2 / p2b — the `inline` keyword alone flips a working HKT program

p2 (with `map inline`):

```sth
type: Opt['T] | None | Some 'T ;
trait: Functor['F: * -> *] :
  map inline ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Opt
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Opt? ;
;
: showopt ( Opt[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Opt? ;
: mkopt ( i64 -- Opt[i64] ) Some ;
: main ( -- ) 3 mkopt [ 1 sub ] map[i64 i64] showopt ;
```

```text
error: `map;Functor;0;Opt['T0]` in `map` (member of trait `Functor` for `Opt['T0]`) (line 8) names the generic type `Opt['ctor0]`, which cannot yet be instantiated at a variable-bearing application
  grounding a generic over its own type variable is not yet implemented
```

p2b, byte-identical but for the dropped `inline`, builds and prints `2`.

### p3 / p3b — the blocker is not HKT-specific

An ordinary Star-kind trait, a generic impl target, an `inline` member, no application
anywhere:

```sth
type: Box['T] v 'T ;
trait: Sized['S] :
  size inline ( 'S -- i64 ) ;
;
impl: Sized for Box['T]
  : size drop 1 ;
;
: usesize['S: Sized] ( 'S -- i64 ) size ;
: mkbox ( i64 -- Box[i64] ) Box ;
: main ( -- ) 3 mkbox usesize . ;
```

```text
error: `size;Sized;0;Box['T0]` in `size` (member of trait `Sized` for `Box['T0]`) (line 8) names the generic type `Box['T]`, which cannot yet be instantiated at a variable-bearing application
  grounding a generic over its own type variable is not yet implemented
```

p3b (no `inline`) prints `1`. So S3's first gate is "`inline` member on a *generic*
impl target", of which HKT is one instance: S2's bare-ctor desugar (`for Opt` ≡
`for Opt['ctor0]`) always produces a generic target, so every HKT member hits it.

### p5 / p5b — the struct route to `Functor` is closed independently of S3

```sth
type: Box['T] v 'T ;
trait: Functor['F: * -> *] :
  map inline ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Box
  : map swap Box> swap call Box ;
;
: mk ( i64 -- Box[i64] ) Box ;
: main ( -- ) 5 mk [ 1 sub ] map[i64 i64] Box> . ;
```

```text
error: `Box>` is not permitted on a generic type `Box['ctor0]` in `map` (member of trait `Functor` for `Box['T0]`) (line 8)
```

p5b (no `inline`) is rejected **identically**. This is pre-existing, not S3's to fix,
but it is load-bearing for scope: it means the only writable `Functor` bodies are
enum-eliminating ones, which is exactly the shape R1.5 fences (below).

### p6 — S4's gap is live; S3 goldens must use fixture twins

```sth
import: core::option * ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Option
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;
;
```

```text
error: `map` in `main` (line 12, col 33) is a trait member of Functor, but no `impl:` in this program dispatches on these operands
  the operand types here are `Option[i64] cstr`; declare an impl of one of those traits for the operand's type, or import a word that claims this name
```

### p7 / p8 — what a *working* splice looks like (the two positive controls)

p7, the concrete-target inline member of p1, symbol-checked:

```text
$ nm p7.../main | grep -E "size.3b.Sized"
(empty => member SPLICED, no symbol minted)

$ nm p7.../main | grep -E "show.3b.Show" | head -3
0000000000001e80 T show.3b.Show.3b.3.3b.Bool__m3
0000000000002030 T show.3b.Show.3b.3.3b.i16__m3
00000000000020c0 T show.3b.Show.3b.3.3b.i32__m3
```

`.3b.` is the mangling of `;`, so a *non*-inline member (`core`'s `Show` impls) mints
`show;Show;3;Bool`; the `inline` member `size;Sized;0;Boxi` mints nothing. The splice
machinery works — for a **mono** member word.

p8, the precedent for a *polymorphic* inline word being spliced: `core::cmp`'s
`: lt inline ['T: Ord] ( 'T 'T -- Bool )` (`lib/core/cmp.sth:145`) minted no symbol in a
program calling it twice. Poly + `inline` + a user bound **can** splice — via the
`poly.combinators` interception, which records no instantiation.

## Mutation experiments

All mutations were applied to a `cp`-backed copy of the original file
(`/tmp/p7bs3-probes/poly.rs.ORIG`, `check.rs.ORIG`, sha-verified identical before the
first edit) and restored by `cp` back, with `git status --porcelain` empty afterwards.

### m1 — neutralize R3 (`check/poly.rs:566`)

```rust
// before
for pty in &sig.inputs {
    if matches!(pty, PolyType::Generic { .. } | PolyType::GenericVariant { .. }) {
// after (probe only)
for pty in &sig.inputs {
    if matches!(pty, PolyType::GenericVariant { .. }) { // m1
```

Result: p3 builds and prints `1`; p2 builds and prints `2` — each matching its
non-inline twin exactly. The gate is the whole of the first blocker, and grounding the
input slot at the standalone stand-in types (`Subst` seeds every ty var at `i64`,
`check/poly.rs:552`) is evidently sufficient for these shapes.

### m2 — tagged diagnostics, to localize p4's failure

With m1 applied, p4 fails with:

```text
error: `impl: Functor for Opt` binds no word for member `map`, dispatched at line 16, col 5 in the body of `twice` (instantiated at line 22, col 33 in `main`)
```

Three call sites produce that text (`check/poly.rs:7938`, `:7969`, `:7981`). Tagging
each showed:

```text
error: [TAG7981] error: `impl: Functor for Opt` binds no word for member `map`, ...
```

`:7981` is the `tr.word_sig_of(word_sym)` miss. `word_sig_of` (`check/poly.rs:181`)
reads `PolyCtx::recorded`, the `WordObligations` table built by the poly pre-pass.

### m3 — record member combinators in the poly pre-pass (`check.rs:837`)

```rust
// before
if is_combinator(word) && !sig.bounds.iter().any(|(_, b)| matches!(b, Bound::User(_))) {
    continue;
}
// after (probe only)
let is_member_word = word.name.contains(';'); // m3
if is_combinator(word)
    && !is_member_word
    && !sig.bounds.iter().any(|(_, b)| matches!(b, Bound::User(_)))
{
    continue;
}
```

A synthesized member word carries no `Bound::User` (S2-6 grounds the trait variable into
the target), so as a combinator it took the skip, was never recorded, and `word_sig_of`
could not find it. With m3 it is recorded — and p2/p4 then hit a **third** gate:

```text
error: `Opt?` in `map` (member of trait `Functor` for `Opt['T0]`) (line 8) eliminates `Opt` at a type this combinator's own splice determines
  a generic enum eliminated inside a combinator body is not yet supported: each splice would need its own resolution, and none is recorded
```

This is R1.5 (`check/poly.rs:4151`, text at `:10637`). The coupling was **predicted in
the tree**: `check.rs:822-835` says the skip is safe only because "that body shape is
rejected first by the pre-existing variable-bearing-application gate ... **Revisit the
skip if that restriction is lifted.**" m1 lifts exactly that restriction.

### m4 — m1 + m3 + both R1.5 gates stubbed (`check/poly.rs:4151`, `:5611`)

Both gate bodies commented out. Result:

```text
--- BUILD p2_hkt_inline_mono ---   exit 0, prints 2
--- BUILD p4_hkt_inline_shared_bound --- exit 0, prints -1 / 3
```

Correct values, for two different impls dispatched from the same `twice` body spans.
**This is not evidence that R1.5 is merely conservative.** The symbol check shows why:

```text
=== INLINE (m1+m3+m4) p4 symbols ===
0000000000002350 T sooth_mono_map_Functor_0_Opt__T0__T1___m0__t0_i64_t1_i64_t2_i64
00000000000023d0 T sooth_mono_map_Functor_0_Res__T0__T1___m0__t0_i64_t1_i64_t2_i64

=== NON-INLINE control p0 symbols ===
0000000000002350 T sooth_mono_map_Functor_0_Opt__T0__T1___m0__t0_i64_t1_i64_t2_i64
00000000000023d0 T sooth_mono_map_Functor_0_Res__T0__T1___m0__t0_i64_t1_i64_t2_i64
```

```text
$ objdump -d p4.../main | grep -cE 'call.*map_Functor'
4
$ objdump -d p0.../main | grep -cE 'call.*map_Functor'
4
```

Identical symbols, identical call counts. **The `inline` keyword produced no lowering
difference at all**: the member was monomorphized into a real function and called four
times, exactly as the non-inline control. m4's green run therefore says only that the
three checker gates guard a path lowering never splices; the per-splice resolution
hazard R1.5 names has not been tested, because no splice occurred.

## Reproduction

```sh
# harness (fixture writer + runner) lives at /tmp/p7bs3-probes/{mkfixture.sh,run.sh}
cd /root/code/ordfruma/sooth-worktrees/p7b-s3 && cargo build
/tmp/p7bs3-probes/run.sh p2_hkt_inline_mono
```
