# P7b.S8c probes — verbatim log and verdict (recon round)

Probe round for [slice8c-brief](./slice8c-brief.md), run against the clean tree
(worktree `p7b-s8c`, rebased onto `main` `445a74e` before probing; `git status
--porcelain` empty apart from the recon docs). Fixtures and the raw capture
live under `/tmp/s8c-probe/` (`PROBE-LOG.txt`, `FIXTURES.txt`, one `.sth` per
probe, `run-probes.sh`) — ephemeral; this doc preserves the log and every
fixture verbatim.

## Baseline

`cargo fmt --check && cargo clippy -- -D warnings && cargo test` at `445a74e`:
**green** (`GATE_EXIT=0`; 88 test binaries, 3293 tests passed, 0 failed).

## Verbatim log

```text
=== HEAD ===
445a74eab7967e1776178911604809a635931d24
=== P1 repro: bound dispatch, plain-slot free input var (Odd/Box['T]) ===

thread 'main' (1207793) panicked at src/ir/driver.rs:579:14:
checked: unification bound every input type variable
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
exit=101
=== P2 mono direct member call (no bound) ===
ran-ok
exit=0
=== P3 S6 Functor.map via mono caller (bump2) ===
2
sooth_mono_map_Functor_0_Option__T0___m0__t0_i64_t1_i64
exit=0
=== P4 quotation-row local, no explicit instantiation ===
error: `consume` in `main` (line 8) has output variable `'U` that no input binds
  note: supply it explicitly: `consume[SomeType SomeType]`
exit=1
=== P5 quotation-row local, explicit instantiation ===

thread 'main' (1208108) panicked at src/ir/driver.rs:579:14:
checked: unification bound every input type variable
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
exit=101
=== P6 HKT trait, App-arg local only (W/Box) ===
ran-ok
exit=0
=== P7 HKT trait, App-arg local + plain local ===
ran-ok
exit=0
=== P8 P7 with two differently-typed slots ===
sooth_mono_w2_W2_0_Box__T0___m0__t0_i64_t1_e0_Bool
exit=0
=== P9 plain-kind trait, bare header slot + plain local ===

thread 'main' (1208142) panicked at src/ir/driver.rs:579:14:
checked: unification bound every input type variable
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
exit=101
=== P10 P9 with bare-ctor impl target ===

thread 'main' (1208143) panicked at src/ir/driver.rs:579:14:
checked: unification bound every input type variable
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
exit=101
=== P11 concrete target, member local, matching site types ===
ran-ok
exit=0
=== P12 concrete target, mismatched site types (List in 'U slot) ===
1
7
00000000000011c0 T odd.3b.Odd.3b.0.3b.i64__m0
00000000000022a0 T sooth_mono_consume__m0__t0_i64_t1_e2_List_i64_
exit=0
=== P13 P1 with backtrace ===

thread 'main' (1208167) panicked at src/ir/driver.rs:579:14:
checked: unification bound every input type variable
stack backtrace:
   5: sooth::ir::driver::subst_polytype
             at src/ir/driver.rs:579:14
   6: sooth::ir::driver::concrete_effect::{{closure}}
             at src/ir/driver.rs:555:13
  18: sooth::ir::driver::concrete_effect
             at src/ir/driver.rs:558:46
  19: sooth::ir::driver::lower
             at src/ir/driver.rs:327:22
  20: sooth::driver::emit_ssa_with_manifest
(extracted to the sooth frames; full capture in /tmp/s8c-probe/PROBE-LOG.txt)
=== P14 output-position local (concrete-bodied impl) ===
error: stack effect mismatch in `gen;Gen;0;Box['T0]`
  body leaves `i64`, but the declared outputs are `'U`
exit=1
```

(The P13 excerpt above trims the Rust-standard-library frames; frames 0-4 and
17 are std panic/plumbing. The full backtrace is in the preserved log.)

## Fixtures (verbatim)

```sth
\ ----- plainslot.sth  (P1, P13 — the S8-review repro, restated exactly)
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: Odd['T] : odd ( 'U &'T -- ) ; ;
impl: Odd for Box['T] : odd drop drop ; ;
: consume ['T: Odd] ( 'U &'T -- ) odd ;
: main ( -- ) 7 mkbox | b | 7 &b consume ;

\ ----- mono.sth  (P2 — P1 minus the bound consumer)
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: Odd['T] : odd ( 'U &'T -- ) ; ;
impl: Odd for Box['T] : odd drop drop ; ;
: main ( -- ) 7 mkbox | b | 7 &b odd ;

\ ----- s6mono.sth  (P3 — S6's Functor.map called from a mono word)
import: intrinsics * ;
import: core::option * ;
import: hosted::show | . | ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Option
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;
;
: bump2 ( Option[i64] [ i64 -- i64 ] -- Option[i64] ) map ;
: main ( -- ) 3 Some [ 1 sub ] bump2 ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;

\ ----- qrow.sth  (P4 — local in a quotation-row input)
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: Odd['T] : odd ( &'T [ 'U -- ] -- ) ; ;
impl: Odd for Box['T] : odd drop drop ; ;
: consume ['T: Odd] ( &'T [ 'U -- ] -- ) odd ;
: main ( -- ) 7 mkbox | b | b &b [ drop ] consume ;

\ ----- qrow3.sth  (P5 — P4 plus an explicit instantiation)
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: Odd['T] : odd ( &'T [ 'U -- ] -- ) ; ;
impl: Odd for Box['T] : odd drop drop ; ;
: consume ['T: Odd] ( &'T [ 'U -- ] -- ) odd ;
: main ( -- ) 7 mkbox | b | b &b [ drop ] consume[Box[i64] i64] drop ;

\ ----- apparg.sth  (P6 — header var only heads an App; local under it)
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: W['F: * -> *] : w ( 'F['U] -- ) ; ;
impl: W for Box : w drop ; ;
: consume3 ['F: W] ( 'F['U] -- ) w ;
: main ( -- ) 7 mkbox | b | b consume3 ;

\ ----- mixed.sth  (P7 — App-arg local + plain-slot local)
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: W2['F: * -> *] : w2 ( 'F['U] 'V -- ) ; ;
impl: W2 for Box : w2 drop drop ; ;
: consume4 ['F: W2] ( 'F['U] 'V -- ) w2 ;
: main ( -- ) 7 mkbox | b | b 7 consume4 ;

\ ----- mixed3.sth  (P8 — P7 with differently-typed slots)
import: intrinsics * ;
import: core::bool | Bool True | ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: W2['F: * -> *] : w2 ( 'F['U] 'V -- ) ; ;
impl: W2 for Box : w2 drop drop ; ;
: consume4 ['F: W2] ( 'F['U] 'V -- ) w2 ;
: main ( -- ) 7 mkbox | b | b True consume4 ;

\ ----- noref.sth  (P9 — header var bare in a plain slot + local)
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: Odd['T] : odd ( 'T 'U -- ) ; ;
impl: Odd for Box['T] : odd drop drop ; ;
: consume ['T: Odd] ( 'T 'U -- ) odd ;
: main ( -- ) 7 mkbox | b | b 7 consume ;

\ ----- baretarget.sth  (P10 — P9 with a bare-ctor impl target)
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: Odd['T] : odd ( 'T 'U -- ) ; ;
impl: Odd for Box : odd drop drop ; ;
: consume ['T: Odd] ( 'T 'U -- ) odd ;
: main ( -- ) 7 mkbox | b | b 7 consume ;

\ ----- concrete.sth  (P11 — concrete impl target, member local)
import: intrinsics * ;
import: hosted::show | . | ;
trait: Odd['T] : odd ( 'T 'U -- ) ; ;
impl: Odd for i64 : odd drop drop ; ;
: consume ['T: Odd] ( 'T 'U -- ) odd ;
: main ( -- ) 7 5 consume ;

\ ----- concrete4.sth  (P12 — concrete target, List[i64] in the 'U slot)
import: intrinsics * ;
import: core::list | List Nil Cons | ;
import: hosted::show | . | ;
trait: Odd['T] : odd ( 'T 'U -- ) ; ;
impl: Odd for i64 : odd . . ;
;
: consume ['T: Odd] ( 'T 'U -- ) odd ;
: push ( List[i64] i64 -- List[i64] ) swap ^ Cons ;
: main ( -- ) Nil 3 push 2 push 1 push | l | 7 l consume ;

\ ----- output.sth  (P14 — local in an output position)
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['T] v 'T ;
: mkbox ( i64 -- Box[i64] ) Box ;
trait: Gen['T] : gen ( &'T -- 'U ) ; ;
impl: Gen for Box['T] : gen drop 7 ;
;
: produce ['T: Gen] ( &'T -- 'U ) gen ;
: main ( -- ) 7 mkbox | b | b &b produce . ;
```

All builds ran under the harness equivalent of `tests/phase7b_slice8.rs`'s
`single_file_hosted` (`package: s8cprobe ; layer: hosted ;` with `depends:` on
the worktree's `lib/core` and `lib/hosted`).

## Verdicts

**P1 (repro, restated):** the S8-review repro reproduces at the rebased base
`445a74e` byte-identically to the roadmap's recorded shape: `panicked at
src/ir/driver.rs:579:14` — "checked: unification bound every input type
variable" — `sooth build` exits 101 and produces no binary. **Correction to
the roadmap's wording:** the roadmap says "the build succeeds and the PANIC
fires at run/IR time"; measured, the *check* succeeds and the panic fires
during **lowering, at build time** — there is no built binary to run. (P13:
`subst_polytype` ← `concrete_effect` ← `lower`'s R9 instantiation loop at
`src/ir/driver.rs:327` — the checker-recorded `(member word, θ)` instantiation
is what lowering chokes on.)

**P2:** the same trait/impl/member called **directly** (mono call site, no
bound) builds and runs clean. The member word's free local `'U` is grounded by
the call site's ordinary unification. The mono route is healthy today; the
S8c golden for it is a positive pin (it must keep working).

**P3:** S6's `Functor.map` (member locals `'T`/`'U` under an App arg and in
quotation rows) called from a **mono** word builds, runs, prints `2`. The
monomorph symbol `sooth_mono_map_Functor_0_Option__T0___m0__t0_i64_t1_i64`
shows θ binding **two** variables — the member locals ground fine on this
route. The shipped S6/S7/S8 dogfoods all ride it.

**P4:** a member local in a quotation-row input, dispatched through the bound
from a caller that under-specifies, is fenced **located at the call site** by
the pre-existing caller-side rule ("output variable `'U` that no input binds /
supply it explicitly"). No panic. This is the caller rule, not a member-signature
fence — see P5.

**P5:** the same shape **with** the explicit instantiation the P4 note asks
for (`consume[Box[i64] i64]`) passes the caller rule and then panics at
`driver.rs:579`. So quotation-row-position member locals are *not* fenced by
P4's rule; they reach the same unbindable-instantiation hole. The P4 fence is
accidental coverage, not a member-signature fence.

**P6/P7/P8:** an HKT trait (`* -> *` header) whose member row mentions the
header variable **only as an App head** — locals under the App (`'F['U]`) and
in plain slots (`'V`) — builds and runs through the bound from a mono caller,
with differently-typed slots grounding correctly and independently
(`sooth_mono_w2_W2_0_Box__T0___m0__t0_i64_t1_e0_Bool`: `'U` := i64, `'V` :=
Bool). This is the S2-3 dispatchable shape riding the CtorImage per-site
composition (`resolve_user_bound`'s S2-8/S2-9 arm, which re-grounds the
obligation's slot record through the caller θ and unifies the member word's
signature per site, `src/check/poly.rs`'s CtorImage arm). Member locals are
**not** inherently unbindable — the route decides.

**P9/P10:** a plain-kind trait whose member row mentions the header variable
**bare** (`'T` as a whole slot) or **under a Ref** (`&'T`), plus any further
member local — panics at `driver.rs:579`, with either impl-target spelling
(`for Box['T]` and bare `for Box`). This is Route B (verdict "Mechanism"
below): the mint records the member instantiation under the impl-target match
substitution alone.

**P11/P12:** a **concrete** impl target (`for i64`) with a member local builds
and runs — and is a **silent type hole**: `ground_member_type`'s
`PolyType::Var(_) => target` arm (`src/ast.rs:2239`) collapses *every* member
local to the target type, so the member word emits as one mono function typed
`(i64, i64)` (`odd.::Odd.::0.::i64` in `nm`), while the dispatch performs **no
site-slot compatibility check** (the non-CtorImage concrete-winner arm keeps
the bare symbol without unifying `ob.slots`). Measured: a `List[i64]` flowed
through the `'U` slot into that `(i64, i64)`-typed member (P12: builds, runs,
prints; the monomorph symbols prove the (i64, List[i64]) caller into the
`Odd.0.i64` callee). Adjacent to the ICE, same obligation loop, but a
different failure class — silent wrong typing, not a panic. The S8c scope
decision (fold in or carve out) is the spec's; the probe record is the
evidence.

**P14:** a member local in an **output** position is unimplementable — every
impl body fails located at the member body check ("body leaves `i64`, but the
declared outputs are `'U`") because no body can produce a value of an
unbound variable. Output-position locals never reach dispatch; already fenced.

## Mechanism (adjudicated)

The dispatch route is decided in `resolve_user_bound` (`src/check/poly.rs`)
by the obligation's `ty`:

- **Route A — CtorImage (App-headed operand).** The member call's operand is
  an application headed by the bound variable (`'F['U]`, the S2-3 shape); the
  App unification binds the head at the consumer's declaration, the args live
  in the obligation's slot record, and the CtorImage arm composes **per site
  at instantiation**: re-ground `ob.slots` through `caller_subst`, unify each
  candidate's member word signature (`unify_poly_input`), mint that site's
  θ_call. Every member variable grounds. (S2-8/S2-9 machinery; P3/P6/P7/P8.)
- **Route B — non-CtorImage, generic winner.** The operand is the bound
  variable itself in a non-App position (plain slot, `&'T`, quotation row) —
  the only shape a plain-kind trait's operand can take. The P7.S4 mint arm
  (`poly.rs:9065-9071`) records `(member_word, subst)` where `subst` is
  `find_bound_impl`'s impl-target match substitution **alone** — the member
  row's own variables never enter θ. Lowering's `concrete_effect`
  (`driver.rs:555`) substitutes the whole member signature through that θ;
  any member variable outside the target pattern is unbound → the `expect`
  at `driver.rs:579` fires. (P1/P5/P9/P10.)
- **Route C — non-CtorImage, concrete winner.** Bare symbol path, no mint, and
  **no site-slot check**: the member signature was fully grounded at
  registration (locals collapsed to the target), so a caller can flow any
  type through a member-local slot unchecked. (P11/P12.)
- **Route D — mono direct member call.** Full call-site unification binds
  everything. (P2.)
- **Route E — lifted mono ctor-app target** (`for Range[i64]` with a mono
  member word): bare symbol, no signature to substitute. (S8's shipped
  golden; untouched.)

The checker/IR contract mismatch, stated once: the IR `expect`'s invariant
("unification bound every input type variable") holds for instantiations
minted by call-site unification (Routes A/D) and is violated **by
construction** for Route B's match-only mint. The checker never enforced,
at declaration or impl registration, that a member row's variables are all
determined by the impl-target pattern — `member_shape_is_supported`'s
`PolyType::Var(_) => true` arm (`src/parser.rs:391`) admits member locals in
plain slots unconditionally (only App heads are restricted to the header
variable, `*head == 0`).

## Route-taxonomy summary table

| Member row (header `'T`/`'F`, local `'U`) | Impl target | Dispatch context | Outcome |
|---|---|---|---|
| `'U &'T --` (plain local + Ref header) | `Box['T]` | bound, mono caller | PANIC `driver.rs:579` (P1) |
| `'T 'U --` (bare header + local) | `Box['T]` / `Box` | bound, mono caller | PANIC (P9/P10) |
| `&'T [ 'U -- ] --` (quotation-row local) | `Box['T]` | bound, mono caller, explicit instantiation | PANIC (P5); without it, located at the call site (P4) |
| `'F['U] --` (App-arg local) | `Box` | bound, mono caller | works (P6) |
| `'F['U] 'V --` (two slots, two types) | `Box` | bound, mono caller | works, both bound (P7/P8) |
| `'F['T] [ 'T -- 'U ] -- 'F['U]` (S6 `map`) | `Option` | bound, mono caller | works (P3) |
| `'T 'U --` | `i64` concrete | bound, mono caller | builds+runs, **no site check** — silent type hole (P11/P12) |
| same traits, member called directly | — | mono direct | works (P2) |
| `&'T -- 'U` (output-position local) | `Box['T]` | impl body check | located at the impl (P14) |

## Corrigenda for the roadmap entry

1. "the build succeeds and the PANIC fires at run/IR time" → the check
   succeeds; the panic fires during **lowering at build time** (`sooth build`
   exits 101, no binary).
2. The repro shape is one cell of a five-route taxonomy, not the whole
   surface: the same hole also swallows quotation-row locals behind P4's
   caller rule (P5), and the concrete-target arm has the adjacent silent
   no-site-check hole (P11/P12).
