# P7b.S6 probe round — verbatim log

Recon round for P7b.S6 scoping (container traits over the real lib types, plus
linear merge and `List['T]`), against the clean tree (worktree `p7b-s6`, HEAD
`600bc1b`, P7b's S1-S5 lines landed). Read-only against the repo; probe
fixtures live under `/tmp/p7bs6-probes/`. `git status --porcelain` is empty
before, during, and after this round.

Baseline: `cargo test --no-fail-fast` at HEAD is green (see Reproduction
section for the exact run).

Fixtures need a package manifest (`hosted::show` is not reachable from an
anonymous single-file package). `/tmp/p7bs6-probes/sooth.pkg`:

```
package: p7bs6 ;
layer: hosted ;
depends: core path "<repo>/lib/core" ;
depends: hosted path "<repo>/lib/hosted" ;
```

## Summary table

| Probe | Fixture | Outcome |
| --- | --- | --- |
| p1-functor | `p1_functor_real_option.sth` | grounds: `map` over real `core::option` prints `2` |
| p1-foldable | `p1_foldable_real_option.sth` | grounds: `fold` over real `core::option` prints `13` |
| p3-forget | `p3_fold_never_drop.sth` | ordinary linear-check rejects it (arm-shape mismatch), no new machinery |
| p3-double | `p3_fold_double_use.sth` | ordinary linear-check rejects it (`call` underflow), no new machinery |
| p2-bifunctor-ok | `p2_bifunctor_result.sth` | grounds: `bimap` over real `core::result`, Ok branch, prints `4` |
| p2-bifunctor-err | `p2b_bifunctor_err_branch.sth` | grounds: `bimap` over real `core::result`, Err branch, prints `-91` |
| p4-none-ctx | `p4_none_grounds_from_context.sth` | zero-arity variant ctor already grounds bottom-up from a declared consumer; not a trait member |
| p4-monoid-empty | `p4_monoid_empty.sth`, `p4_monoid_empty2.sth` | hits a wall: `empty ( -- 'T )` has no dispatchable operand, mono dispatch rejects it even with explicit instantiation |
| p5-selfref | `p5_list_selfref.sth` | grounds: `^List['T]` self-reference builds and runs, single module |
| p5-str-payload | `p5_list_str_payload.sth` | grounds: 3-deep `List[str]` linear chain builds, runs, drops clean |
| p6-trait-decl | `p6_array_kind.sth` | grounds: `trait: F['F: * -> Len -> *]` with `'F['T 'N]` member sig parses and type-checks |
| p6-impl | `p6_array_impl.sth` | hits a wall: `impl: ... for array['T 'N]` is not a constructor target — `array` has no `CtorImage` representation |

## p1 — Functor.map over the real `core::option`

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::option * ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Option
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;
;
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;
: mkopt ( i64 -- Option[i64] ) Some ;
: main ( -- ) 3 mkopt [ 1 sub ] map[i64 i64] showopt ;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p1_functor_real_option.sth
2
```

Byte-for-byte the S4 golden's shape (`tests/phase7b_slice4.rs`'s
`mono_member_call_dispatches_over_the_real_core_option`), re-run here to
confirm it is still live at S6's HEAD (S4 already answered this question; not
a new finding, a control).

## p1 — Foldable.fold over the real `core::option`

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::option * ;
trait: Foldable['F: * -> *] :
  fold ( 'F['T] i64 [ i64 'T -- i64 ] -- i64 ) ;
;
impl: Foldable for Option
  : fold | f | | acc |
    ~[ ( Some ) Some> acc swap f call ]
    ~[ ( None ) drop acc ]
    Option? ;
;
: mkopt ( i64 -- Option[i64] ) Some ;
: main ( -- ) 3 mkopt 10 [ add ] fold . ;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p1_foldable_real_option.sth
13
```

`3 + 10 = 13`: `fold` dispatches on `Option[i64]`, the `Some` arm calls the
accumulator quotation once with `(acc, payload)`. No new dispatch machinery
needed beyond what `impl: Functor for Option` already exercises (S2/S4) — the
first attempt (a version whose `None` arm was `~[ ( None ) acc ]`, no `drop`)
was rejected first:

```text
error: an arm of `Option?` leaves `Option.None` on the stack in `fold` (member of trait `Foldable` for `Option['T0]`) (line 10)
  a variant-typed value is reachable only inside the arm that bound it; consume it there, or leave its fields instead
```

confirming the pre-existing per-arm variant-consumption check (not new for
S6) fires exactly as it would for a hand-written `fold`.

## p3 — does the checker already enforce single consumption inside `fold`'s quotation body?

Forgetting to consume the payload at all (never calling `f`, never dropping
it) — this is actually **legal**, because `drop` is DESIGN.md's explicit
destructor, not a special fold-only rule:

```sth
impl: Foldable for Option
  : fold | f | | acc |
    ~[ ( Some ) Some> drop acc ]
    ~[ ( None ) drop acc ]
    Option? ;
;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p3_fold_forget_payload.sth
10
```

Runs fine, prints `10` (the accumulator unchanged, payload explicitly
dropped). This is not what "linear consumption" rules out — DESIGN.md's rule
is *forgetting* is an error, not *discarding via `drop`*.

Never dropping and never using the payload at all (a bare `~[ ( Some ) Some> acc ]`,
leaving the extracted variant field unconsumed at the end of the arm):

```text
$ cargo run -- run /tmp/p7bs6-probes/p3_fold_never_drop.sth
error: the quotations passed to `Option?` leave different stack shapes: an earlier one leaves `'ctor0 i64`, this one leaves `i64` in `fold` (member of trait `Foldable` for `Option['T0]`) (line 10)
```

Rejected — but by the pre-existing arm-shape-parity check (`Option?`'s two
arms must leave the same stack shape), the same mechanism that would reject
this in a hand-written `Option?` use with no trait involved at all.

Calling the accumulator quotation `f` twice (double use of a linear
quotation-typed local):

```sth
~[ ( Some ) Some> acc swap f call f call ]
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p3_fold_double_use.sth
error: stack effect mismatch in `fold` (member of trait `Foldable` for `Option['T0]`) (line 9)
  `call` needs 2 values, but the stack holds 1
  note: declared ( -- )
```

Rejected — the second `call` underflows because the first `call` already
consumed `acc`/payload; the linear local `f` itself is `Copy`-quotation, so
what actually blocks the double-use is ordinary arity checking, not a
linearity violation on `f`. Anchor for the general use-after-move check this
family rests on (unrelated to Foldable specifically): `check.rs:1576`
(`error: use after move in {}...`) and `check/poly.rs:10598` (the poly-body
twin) — neither fired in these three fixtures because each was caught by an
earlier, more specific check (arm-shape parity, or plain arity underflow), so
this round did not construct a fixture that reaches the use-after-move path
through a `Foldable.fold` body specifically. **Verdict: `Foldable.fold`
consuming linearly is already enforced end to end by machinery that exists
for every quotation-eliminator body today; S6 owes it nothing new.**

## p2 — Bifunctor.bimap unifying map/map_err/swap on the real `core::result`

First attempt used qualified names throughout (`import: core::result r | Ok Err | ;`,
then `r::Ok`/`r::Result?` inside the impl body) — rejected at multiple
points:

```text
$ cargo run -- run /tmp/p7bs6-probes/p2_bifunctor_result.sth   # v1, r::Ok inside a pattern arm
error: unknown type `r::Ok` at line 11, col 10
```

Eliminator-arm tags (`( Ok )`) and the eliminator call itself (`Result?`) are
never qualified anywhere in the tree (`grep -rn "( r::Ok )\|r::Result?"` finds
zero hits outside this probe) — confirmed by grepping every existing `( Ok )`
usage (`examples/tests/result.sth`, `tests/phase7b_slice2.rs`, etc.): always
bare. Switching to `import: core::result * ;` (bare wildcard, matching the S4
golden's `core::option` import style) and bare `Ok`/`Err`/`Result?` throughout
fixed the qualifier issue; a second wall was two more rounds of ordinary
mistakes (binding order for the two quotation locals, needing explicit
instantiation for `bimap`'s otherwise-unbound output variables) — not
findings, just probe iteration. Final:

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::result * ;
trait: Bifunctor['F: * -> * -> *] :
  bimap ( 'F['A 'B] [ 'A -- 'C ] [ 'B -- 'D ] -- 'F['C 'D] ) ;
;
impl: Bifunctor for Result
  : bimap
    | h |
    | g |
    ~[ ( Ok ) Ok> g call Ok ]
    ~[ ( Err ) Err> h call Err ]
    Result? ;
;
: mkres ( i64 -- Result[i64 i64] ) Ok ;
: showres ( Result[i64 i64] -- )
  ~[ ( Ok ) Ok> . ]
  ~[ ( Err ) Err> . ]
  Result? ;
: main ( -- ) 3 mkres [ 1 add ] [ 1 sub ] bimap[i64 i64 i64 i64] showres ;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p2_bifunctor_result.sth
4
```

`3 + 1 = 4`, the `Ok` branch runs `g` (`[ 1 add ]`), confirming `Bifunctor`
over a **two-argument** constructor kind (`* -> * -> *`) type-checks and
dispatches over the real `Result`. Confirmed the `Err` branch runs the other
quotation with a second fixture (`p2b_bifunctor_err_branch.sth`, `Err(9)`
through `[ 1 add ]`/`[ 100 sub ]`):

```text
$ cargo run -- run /tmp/p7bs6-probes/p2b_bifunctor_err_branch.sth
-91
```

`9 - 100 = -91`, confirming `h` (the second quotation, bound to `Err`'s
payload transform) runs on the `Err` branch, not `g`. This grounds S2's
two-argument constructor-kind machinery over the real lib type; `map` and
`map_err` and `swap` are all expressible as `bimap` partial applications
(`bimap[.. id ..]` for `map`, `bimap[id ..]` for `map_err`, `bimap` with two
swapping ctors for `swap`) — this round did not spell those three convenience
wrappers out, only the general `bimap` dispatch they would share.

## p4 — Semigroup/Monoid: is return-type polymorphism from consuming context already grounded anywhere?

The zero-arity variant ctor case (`None`) already grounds purely from a
consuming declared type, with **no operand and no explicit instantiation**:

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::option * ;
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;
: main ( -- ) None showopt ;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p4_none_grounds_from_context.sth
(exits 0, no output -- the `None` arm's `drop` consumes it, printing nothing)
```

Grounds: `None`'s `'T` is inferred from `showopt`'s declared parameter type,
with `None` itself carrying zero operands. This is the **zero-arity variant
ctor** mechanism, not a trait member — recorded standing memory note
`project_zero_arity_variant_ctor_collides_across_monomorphs` already
documents this path exists.

Whether an ordinary **trait member** with signature `( -- 'T )` (Monoid's
`empty` shape) can dispatch the same way was tested directly:

```sth
import: intrinsics * ;
import: hosted::show | . | ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
;
impl: Monoid for i64
  : empty 0 ;
;
: main ( -- ) empty[i64] . ;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p4_monoid_empty.sth
error: `empty` in `main` (line 9, col 15) is a trait member of Monoid, but no `impl:` in this program dispatches on these operands
  the operand types here are ``; declare an impl of one of those traits for the operand's type, or import a word that claims this name
```

Rejected even with an **explicit type-argument list** (`empty[i64]`). Without
the explicit instantiation, same result:

```sth
: main ( -- ) empty . ;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p4_monoid_empty2.sth
error: `empty` in `main` (line 9, col 15) is a trait member of Monoid, but no `impl:` in this program dispatches on these operands
  the operand types here are ``; declare an impl of one of those traits for the operand's type, or import a word that claims this name
```

Traced to the mechanism: `resolve_mono_member_call` (`src/check/poly.rs:2165`)
requires a **dispatchable input operand** to find a viable impl at all —
`dispatchable_input_pos` (`src/check/poly.rs:1444-1450`) scans `sig.inputs`
for one that heads with the trait's own variable (`PolyType::Var(0)` or an
`App { head: 0, .. }`); `empty`'s signature has zero inputs, so this always
returns `None`, the member is never added to `viable` (`poly.rs:2225`), and
`viable.len() == 0` unconditionally raises `mono_member_no_dispatch_error`
(`poly.rs:2256`) — the empty-operand-list branch of the *same* error path S5's
probes already exercised for a different reason (S5's `pa2`). **Verdict:
grounding a trait member's output purely from the calling context, with zero
operands, is not merely absent from `Monoid` — it is structurally
unreachable through the current mono-dispatch entry point, which requires an
operand to find a candidate at all.** This is new machinery S6 needs to spec,
not library work riding an existing mechanism (`empty` is out of pocket
differently than `Option`'s `None`, which sidesteps trait dispatch entirely
by being a variant ctor).

## p5 — `List['T]` self-reference and the fused destructor

```sth
import: intrinsics * ;
import: hosted::show | . | ;
type: List['T] | Nil | Cons 'T rest ^List['T] ;
: mklist ( i64 -- List[i64] ) Nil ^ Cons ;
: main ( -- ) 5 mklist drop ;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p5_list_selfref.sth
(exits 0, no output)
```

Builds and runs clean — `^List['T]` self-reference grounds under a generic
declaration, single module, same shape as the S4 golden's `L['T]`
(`tests/phase7b_slice4.rs`'s
`recursive_generic_header_named_across_modules_builds_and_runs`) but with
`List`, not `L`, and no cross-module naming.

A 3-deep chain with a **linear payload** (`str`, which needs disposal, unlike
`i64`):

```sth
import: intrinsics * ;
import: hosted::show | . | ;
type: List['T] | Nil | Cons 'T rest ^List['T] ;
: mkempty ( -- List[str] ) Nil ;
: main ( -- )
  "c" mkempty ^ Cons
  "b" swap ^ Cons
  "a" swap ^ Cons
  drop ;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p5_list_str_payload.sth
(exits 0, no output)
```

Builds and runs clean — the trailing `drop` disposes a 3-element
`List[str]` with no crash. This exercises the **fused iterative destructor**
concept: `synthesize_struct_destructor`/`synthesize_enum_destructor`
(`src/ir/destructors.rs`; the enum recursive-cycle case is
`ir/destructors.rs:409`, "If the enum is on a disposal cycle, the whole
destructor becomes one fused..."; the struct twin is
`ir/destructors.rs:298-299`, "disposed by one fused loop that walks the whole
route"). This machinery operates on already-monomorphized `IrType::Struct`/
`IrType::Enum` ids (`ir/destructors.rs:332` builds `self_ty =
IrType::Enum(id)` or `Struct(id)` from a concrete `id`), so "per-instantiation
payload drop" falls out for free from the existing per-concrete-type
destructor synthesis — each `List[i64]`/`List[str]`/etc. mints its own
`EnumId`, hence its own synthesized destructor, with no new plumbing needed
for the *drop* side. **Verdict: both S6 probe questions for `List['T]`
ground at HEAD; no wall found.**

## p6 — array-as-`'F: * -> Len -> *`

The `Kind` enum already models an n-ary arrow with `Len` as a domain
(`src/ast.rs:552-556`, `Kind::Arrow { domains: Vec<Kind>, result: Box<Kind> }`,
comment explicitly: "so `array` is honestly `* -> Len -> *`"), and the parser
already accepts this exact spelling in a bound bracket
(`src/parser.rs:2424-2429`, `parse_kind_expr`; a unit test already exercises
it, `src/parser.rs:13863-13870`,
`header_bracket_vars("['F: * -> Len -> * 'T]")`).

A user-declared trait with this kind and a member applying it with both a
type and a length argument parses and type-checks clean:

```sth
import: intrinsics * ;
import: hosted::show | . | ;
trait: ArrFunctor['F: * -> Len -> *] :
  amap ( 'F['T 'N] [ 'T -- 'T ] -- 'F['T 'N] ) ;
;
: main ( -- ) 0 . ;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p6_array_kind.sth
0
```

The wall is at `impl:`, not the trait declaration. `impl: ArrFunctor for array`
(bare) fails at parse (array's own reserved-name rule, unrelated to HKT):

```text
error: `array` must be followed by `[T N]` to form an array type at line 6, col 22
```

`impl: ArrFunctor for array['T 'N]` (the named-array reader,
`parse_impl_target_named_array_parses`'s exact shape,
`src/parser.rs:13796-13812`, already a passing unit test for a different
trait, `Show`) parses — `ImplTarget.pattern` becomes a `Type::Array`-headed
pattern, not a `Type::CtorImage` — but fails at grounding:

```sth
impl: ArrFunctor for array['T 'N]
  : amap map ;
;
```

```text
$ cargo run -- run /tmp/p7bs6-probes/p6_array_impl.sth
error: trait member `amap` of `ArrFunctor` (line 7, col 5) applies the trait variable `'F`, but the impl target `array['T 'N]` is not a constructor
  a fully-abstract target names no constructor for the application to dissolve into; implement the trait for a constructor application (e.g. `impl: ... for Option['Opt]`)
```

Traced: the *sole* representation of "a constructor" the HKT member-grounding
path (`ast.rs`'s `ground_member_poly`, the App arm around `ast.rs:2352`) can
dissolve an application into is `Type::CtorImage(GenericId, &'static str)`
(`src/ast.rs:3115`), minted only from a user `type:`/`variant:` generic header
(`GenericId` indexes `generics.structs`/`generics.enums`). `array` is a
built-in `Type::Array(ArrayId, name)` (`src/ast.rs`, distinct variant, no
`CtorImage` wrapping anywhere in the tree — `grep -n "Type::Array" src/ast.rs`
finds only the ordinary array-type paths, never a `CtorImage` construction
site for it). Confirmed the impl-target parser also discards whatever kind
information a named-array target might carry: `parse_impl_target`
(`src/parser.rs:4071-4103`) unconditionally sets
`ty_kinds: vec![Kind::Star; builder.ty_names.len()]` (`parser.rs:4097`) for
**every** impl target, regardless of what the target's actual variable roles
are — so even a hypothetical `CtorImage`-wrapped array would arrive at the
grounding step with its `'N` mis-kinded as `Star`. **Verdict: this is not a
narrow kind-arity gap to size — the array-as-constructor path is
structurally absent** (no representation for "the array constructor" as a
`CtorImage`, and the one impl-target parse path that gets close to naming it
throws its kind information away regardless). A ruling is needed on whether
`Type::CtorImage` grows an array-shaped variant, or whether array Functor
instances are declared through some other mechanism entirely (e.g. a
built-in `impl:` desugar that never goes through `CtorImage` at all) — this
round found no partial machinery to extend, only the trait-declaration half
already working.

## Reproduction

```sh
cd /root/code/ordfruma/sooth-worktrees/p7b-s6 && cargo build
cargo run -- run /tmp/p7bs6-probes/p1_functor_real_option.sth       # 2
cargo run -- run /tmp/p7bs6-probes/p1_foldable_real_option.sth      # 13
cargo run -- run /tmp/p7bs6-probes/p3_fold_forget_payload.sth       # 10 (legal drop)
cargo run -- run /tmp/p7bs6-probes/p3_fold_never_drop.sth           # arm-shape-parity error
cargo run -- run /tmp/p7bs6-probes/p3_fold_double_use.sth           # call underflow error
cargo run -- run /tmp/p7bs6-probes/p2_bifunctor_result.sth          # 4
cargo run -- run /tmp/p7bs6-probes/p2b_bifunctor_err_branch.sth     # -91
cargo run -- run /tmp/p7bs6-probes/p4_none_grounds_from_context.sth # exits 0
cargo run -- run /tmp/p7bs6-probes/p4_monoid_empty.sth              # mono_member_no_dispatch_error
cargo run -- run /tmp/p7bs6-probes/p4_monoid_empty2.sth             # mono_member_no_dispatch_error
cargo run -- run /tmp/p7bs6-probes/p5_list_selfref.sth              # exits 0
cargo run -- run /tmp/p7bs6-probes/p5_list_str_payload.sth          # exits 0
cargo run -- run /tmp/p7bs6-probes/p6_array_kind.sth                # 0
cargo run -- run /tmp/p7bs6-probes/p6_array_impl.sth                # "is not a constructor" error
```

No mutation experiments were run against `src/`: every probe was a source
fixture read-only against the built binary; the two "wall" findings (Monoid's
`empty`, array-as-constructor) were traced to their exact structural cause by
reading the relevant functions, not by spiking a fix.
