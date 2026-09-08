# P7b.S8b probe log — round P8b + the two dig rounds (260906–260907)

Consolidated verbatim evidence for the S8b spec. Three rounds, all on base
`c406149` (+ `/tmp/p8b-arm.patch`, the measured wall arm, where noted):

1. **Round P8b** (the brief's PB-1..PB-6 battery) — fixtures under
   `/tmp/p8b-probes/`, full 26-fixture rerun captured below (Part 1); Part 1a inlines
   the golden-model fixture sources these goldens cite, verbatim, for durability.
2. **Segfault root-cause** (worker run d9e51aa6) — why `empty[List[i64]]`
   crashed after any prior List construction: a pre-existing S6-era two-defect
   pair (θ seeding + variant-map clobber). Verbatim log (Part 2).
3. **Spellings** (worker run 0a9a5d78, salvaged and completed on the main
   line) — the member-surface facts: distinct-'U fences, working substitutes,
   combine-through-bound golden. Verbatim log (Part 3).

The consolidated verdict ledger closes the file (Part 4). Line numbers in the
round logs were taken on `c406149 + arm` and differ from the clean tree;
re-locate by symbol name.

# Part 1 — P8b battery, 26-fixture rerun (verbatim build/run results)

```text
===== p8b-2b-prepend-list-cons.sth =====
build: OK
run exit: 0
===== p8b-2d2-plain-generic-cons-helper.sth =====
build: OK
run exit: 0
===== p8b-2d2-plain-generic-cons.sth =====
build FAILED (exit 1)
error: unknown word `Nil` in `main` (line 4)
===== p8b-append-listspecific-ctor-target.sth =====
build FAILED (exit 1)
error: trait member `append` of `Appender` (line 4, col 3) has no input for a call to dispatch on (expected the trait's variable `'E` bare or heading an application like `'E['T]`)
  note: a variable nested inside a composite input (an array element, say) does not count
===== p8b-append-listspecific-mono-trait.sth =====
build FAILED (exit 1)
error: parse error: expected a type variable or bracketed header (`trait: ListI64Ops['T]`) after `trait: ListI64Ops`, found Word(":") at line 3, col 19
===== p8b-append-listspecific-trait.sth =====
build FAILED (exit 1)
error: trait member `append` of `Appender` (line 7, col 5) declares the ctor-headed application `List['E]`, but the impl target `i64` is concrete
  a ctor-headed member row has no monomorphic representation here (`ground_member_type` grounds concrete/array/reference/quotation shapes only); implement the trait for a constructor target with a type variable instead
===== p8b-concrete-tail-operand.sth =====
build FAILED (exit 1)
error: type mismatch in `snoci64` (line 3)
  `Cons` expected `List['T]`, found `List[i64]`
  note: declared ( -- )
===== p8b-cons-tail-mismatch-clean.sth =====
build FAILED (exit 1)
error: type mismatch in `badcons` (line 4)
  `Cons` expected `List['T]`, found `Option['T]`
  note: declared ( -- )
===== p8b-cons-tail-mismatch-differently-headed.sth =====
build FAILED (exit 1)
error: type mismatch in `badcons` (line 4)
  `Cons` expected `List['T]`, found `^Option['T]`
  note: declared ( -- )
===== p8b-dup-list-operand-fenced.sth =====
build FAILED (exit 1)
error: cannot `dup` a generic type applied to a variable in `duplist` (line 3)
  `List['T]` is conservatively linear: it may carry a linear argument at some instantiation, so it cannot be duplicated
===== p8b-lenvar-selfref-decl.sth =====
build FAILED (exit 1)
error: type mismatch in `mkring` (line 3)
  `Ring` expected `^Ring['T 'N]`, found `'T`
  note: declared ( -- )
===== p8b-lenvar-selfref-fence.sth =====
build FAILED (exit 1)
error: type mismatch in `mkring` (line 3)
  `Ring` expected `Ring['T 'N]`, found `Ring['T 'N]`
  note: declared ( -- )
===== p8b-map-distinct-u-explicit-list-head.sth =====
build FAILED (exit 1)
error: generic type `List` declares 1 type variable, but none were supplied at line 14, col 22 (apply it as `List[T]`, one type argument per declared variable)
  note: a glued bracket is an explicit type instantiation; insert a space for a quotation or array literal
===== p8b-map-distinct-u-unbound.sth =====
build FAILED (exit 1)
error: `mapadd` in `main` (line 15) has output variable `'U` that no input binds
  note: supply it explicitly: `mapadd[SomeType SomeType SomeType]`
===== p8b-map-list-end-to-end.sth =====
build: OK
run exit: 0
stdout:
2
3
4
===== p8b-map-option-payload-drop.sth =====
build: OK
run exit: 0
stdout:
ok===== p8b-map-shared-bound-twice.sth =====
build: OK
run exit: 0
stdout:
3
4
5
===== p8b-monoid-list-append-show.sth =====
build: OK
run exit: 0
stdout:
1
2
3
5
3
===== p8b-monoid-list-append-visible.sth =====
build FAILED (exit 1)
error: `empty` in `main` (line 29, col 3) is a trait member of Monoid, but no `impl:` in this program dispatches on these operands
  the operand types here are ``; declare an impl of one of those traits for the operand's type, or import a word that claims this name
===== p8b-monoid-list-mconcat.sth =====
build FAILED (exit 1)
error: no overload of `Nil` in `mkempty` (line 26) accepts these operands
  candidate: no operands
  candidate: no operands
===== p8b-monoid-list-witness.sth =====
build: OK
run exit: 0
===== p8b-side-empty-instantiation-only.sth =====
build: OK
run exit: 0
stdout:
ok===== p8b-side-empty-instantiation-spelling.sth =====
build: OK
run exit: 1
===== p8b-side-nested-nil-explicit.sth =====
build FAILED (exit 1)
error: `Nil` (line 3) takes no type arguments; only a call to a polymorphic word may be explicitly instantiated
===== p8b-side-nested-nil-output.sth =====
build FAILED (exit 1)
error: no overload of `Nil` in `mkouter` (line 3) accepts these operands
  candidate: no operands
  candidate: no operands
===== p8b-side-nested-nil-poly.sth =====
build FAILED (exit 1)
error: `List[...]` at line 3, col 22 names `List[...]` as a type argument, but a generic applied to another generic (nesting depth > 1) is not yet supported
===== p8b-undropped-append-result.sth =====
build FAILED (exit 1)
error: linear value left on the stack in `main` (line 16)
  body leaves a `List[i64]` beyond the 0 declared output(s): a linear value must be consumed exactly once, so `drop` it or return it
  note: declared ( -- )
===== p8b-undropped-map-result.sth =====
build FAILED (exit 1)
error: linear value left on the stack in `main` (line 14)
  body leaves a `List[i64]` beyond the 0 declared output(s): a linear value must be consumed exactly once, so `drop` it or return it
  note: declared ( -- )
```

# Part 1a — golden-model fixture sources, inlined 260907 for durability

Phase 3's map/append goldens and Phase 2's P8-2d2 exit criterion name `/tmp`-resident
fixtures by filename. Inlined here verbatim (byte-identical to `/tmp/p8b-probes/<name>`
at probe time) so the spec's golden-model descriptions do not depend on a volatile path.
Each source is followed by a one-line restatement of its Part 1 (or, where noted, Part 2/3)
verdict.

### `p8b-2d2-plain-generic-cons-helper.sth`

```sth
import: intrinsics * ;
import: core::list * ;
: cons2['T] ( 'T List['T] -- List['T] ) ^ Cons ;
: mknil ( -- List[i64] ) Nil ;
: main ( -- ) 5 mknil cons2 drop ;
```

Verdict (Part 1): build OK, run exit 0 — the P8-2d2 shape (a plain generic word
constructing `Cons`) grounds via the helper-function spelling.

### `p8b-map-list-end-to-end.sth`

```sth
import: intrinsics * ;
import: core::list * ;
import: hosted::show | . | ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for List
  : map
    swap
    ~[ ( Nil ) drop drop Nil ]
    ~[ ( Cons ) Cons> | v rest | dup v swap call rest ^> rot map ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  [ 1 add ] map[i64 i64]
  showlist ;
```

Verdict (Part 1): build OK, run exit 0, stdout `2\n3\n4` — `map` through the `Functor`
bound over a real `List[i64]` grounds end-to-end.

### `p8b-map-shared-bound-twice.sth`

```sth
import: intrinsics * ;
import: core::list * ;
import: hosted::show | . | ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for List
  : map
    swap
    ~[ ( Nil ) drop drop Nil ]
    ~[ ( Cons ) Cons> | v rest | dup v swap call rest ^> rot map ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: twice['F: Functor 'T] ( 'F['T] [ 'T -- 'T ] -- 'F['T] )
  | q |
  q map
  q map ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  [ 1 add ] twice
  showlist ;
```

Verdict (Part 1): build OK, run exit 0, stdout `3\n4\n5` — `map` dispatched twice through
a *shared* `Functor` bound (the `'U := 'T` specialization) grounds.

### `p8b-sp-4-combine-through-bound.sth`

```sth
import: intrinsics * ;
import: core::list * ;
import: hosted::show | . | ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for List
  : empty Nil ;
  : combine
    swap
    ~[ ( Nil ) drop ]
    ~[ ( Cons ) Cons> | v rest | rest ^> swap combine v swap ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: merge['T: Monoid] ( 'T 'T -- 'T ) combine ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  3 mkempty ^ Cons
  5 swap ^ Cons
  merge
  showlist ;
```

Verdict (Part 3, item 4): build OK, run exit 0, stdout `1 2 3 5 3` (green twice) —
`combine` through a shared `Monoid` bound appends two `List[i64]` spines correctly.

### `p8b-bisect-v1-drop-then-empty.sth`

```sth
import: intrinsics * ;
import: core::list * ;
import: hosted::show | . | ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for List
  : empty Nil ;
  : combine
    swap
    ~[ ( Nil ) drop ]
    ~[ ( Cons ) Cons> | v rest | rest ^> swap combine v swap ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
: main ( -- )
  1 mkempty ^ Cons drop
  empty[List[i64]] drop "ok" . ;
```

Verdict (Part 2): build OK; `sooth run` exits 1 with no output; the direct binary
SIGSEGVs (exit 139), reproducibly, twice — this is the segfault-round's minimal repro
for the pre-existing θ-seeding + variant-word-clobber pair (Part 2, Step 1/2).

# Part 2 — Segfault root-cause log (worker d9e51aa6, verbatim)

# P8b follow-up probe — root-causing the `empty[List[i64]]` runtime crash

Date: 2026-09-07 · Tree: branch `pi-parallel-d9e51aa6…` (p7b-s8b line) at `c406149`, plus
`/tmp/p8b-arm.patch` applied (the P8b probe arm: a `PolyType::Generic` field arm in
`poly_bind_construction_arg`, `src/check/poly.rs`, + flipped witness test). All source
changes disposable; patch stashed/popped during attribution and reverted at round end.

All fixtures live in `/tmp/p8b-probes/`. `sooth` = `target/debug/sooth` of the worktree.
An ancestor manifest `/tmp/p8b-probes/sooth.pkg` (added this round, resolves `core`/`hosted`
against the worktree's `lib/`) makes every fixture build reproducible from that directory.

Verbatim conventions: every step below records fixture path, exact command, exact captured
output, and a per-step verdict.

---

## Step 0 — setup

```
$ cd /tmp/pi-worktree-d9e51aa6-668f-4001-a843-9ac1bac25c01-s0-0
$ git status --short && git log --oneline -1 && git branch --show-current

c406149 docs(roadmap): P7b.S8b brief — carve-out record, wall delta, probe questions PB-1..PB-6
pi-parallel-d9e51aa6-668f-4001-a843-9ac1bac25c01-s0-0
```

(worktree clean; commit matches the assigned c406149)

```
$ git apply /tmp/p8b-arm.patch && git status --short
 M src/check/poly.rs
 M tests/phase7b_slice6.rs
$ cargo build
   Compiling sooth v0.0.0 (…)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.29s
```

Verdict: probe arm applied, build clean. (Canonical gate with the patch was verified green
in the P8b round: fmt clean, clippy clean, 3293 tests / 0 failed — not re-run here.)

Note: the stale pre-built binaries in /tmp/p8b-probes initially ran without a manifest; a
first `sooth run`/`sooth build` attempt against them recorded the manifest error, after
which `/tmp/p8b-probes/sooth.pkg` was written and every binary below was rebuilt fresh with
the patched compiler:

```
$ target/debug/sooth run /tmp/p8b-probes/p8b-side-empty-instantiation-only.sth
error: import `core::list` at line 2, col 1 in /tmp/p8b-probes/p8b-side-empty-instantiation-only.sth:
  /tmp/p8b-probes/p8b-side-empty-instantiation-only.sth has no ancestor `sooth.pkg` and no user-level manifest, so it is an implicit anonymous package that can only import `intrinsics` and its own quoted-path siblings
  `core` cannot be resolved; write $XDG_CONFIG_HOME/sooth/global_sooth.pkg with a `depends:` entry, add an ancestor `sooth.pkg`, or pass `--manifest <path>`]
(build exit=1 — fixture unchanged; recorded result, then manifest added)
```

---

## Step 1 — reproduction matrix (sooth run + direct binary, crash cases twice)

Command pattern (verbatim per fixture):

```
target/debug/sooth run   <fixture.sth>     # exit + stdout + stderr
target/debug/sooth build <fixture.sth>
<fixture-without-.sth>                     # direct binary, twice
```

| fixture | prior List work in main | `sooth run` | direct binary (x2) |
|---|---|---|---|
| `p8b-side-empty-instantiation-only.sth` | none (empty call only) | exit 0, `ok` | exit 0, `ok` / exit 0, `ok` |
| `p8b-bisect-v1-drop-then-empty.sth` | `1 mkempty ^ Cons drop` | exit 1, silent | exit 139, empty / exit 139, empty |
| `p8bf-m3-showlist-then-empty.sth` (new) | `1 mkempty ^ Cons` + showlist | exit 1, silent | exit 139, empty / exit 139, empty |
| `p8b-side-empty-instantiation-spelling.sth` | construct×5 + combine + showlist | exit 1, silent | exit 139, empty / exit 139, empty |
| `p8bf-m5-print-before-drop.sth` (new) | `1 mkempty ^ Cons drop`; then `empty[List[i64]] "ok" . drop` | exit 1, silent | exit 139, empty / exit 139, empty |
| `p8bf-m6-bare-nil-then-empty.sth` (new) | `mkempty drop` (bare Nil, no Cons ever) | exit 0, `ok` | exit 0, `ok` / exit 0, `ok` |
| `p8bf-m7-nil-print-before-drop.sth` (new) | `Nil drop`; then `empty[List[i64]] "ok" . drop` | exit 0, `ok` | exit 0, `ok` / exit 0, `ok` |

Verbatim samples (all crash cases produced byte-identical captures across attempts):

```
##### CASE p8b-bisect-v1-drop-then-empty
--- sooth run (attempt 1)
exit=1
stdout: []
stderr: []
--- build
build exit=0
build stderr: []
--- direct binary (attempt 1)
exit=139
stdout: []
stderr: []
--- direct binary (attempt 2)
exit=139
stdout: []
stderr: []

##### CASE p8bf-m5-print-before-drop
--- sooth run (attempt 1)
exit=1
stdout: []
stderr: []
--- direct binary (attempt 1)
exit=139
stdout: []
stderr: []
```

`RUST_BACKTRACE=1` on the direct binary (C-compiled, no Rust runtime — no effect):

```
$ RUST_BACKTRACE=1 /tmp/p8b-probes/p8b-bisect-v1-drop-then-empty
Segmentation fault
exit=139
```

Verified load-bearing facts:

- Buffering is NOT hiding output: `lib/hosted/show` writes via raw `write(2)` ("Everything
  is `write(2)`, so program order is transcript order", `lib/hosted/show.sth` header).
- The print-before-drop variant (m5) segfaults with **no `ok`** — the crash precedes the
  first print; and m3/m4 (showlist before empty) print **no list elements at all**.
  Both facts point *earlier* than the empty call.

Verdict: crash reproduced; localization must move *before* the empty call.

---

## Step 2 — localization (gdb)

```
$ gdb -batch -ex run -ex bt --args /tmp/p8b-probes/p8b-bisect-v1-drop-then-empty
Program received signal SIGSEGV, Segmentation fault.
0x0000555555555329 in sooth_main ()
#0  0x0000555555555329 in sooth_main ()
#1  0x0000555555556769 in main ()
```

```
$ gdb -batch -ex run -ex "info registers rip rsp rbp rax rbx rcx rdx rsi rdi" \
      -ex "x/8i \$rip" /tmp/p8b-probes/p8b-bisect-v1-drop-then-empty
rip            0x555555555329      0x555555555329 <sooth_main+73>
rcx            0x1                 1
=> 0x555555555329 <sooth_main+73>: mov    (%rcx),%rcx
   0x55555555532c <sooth_main+76>: mov    %rcx,-0x20(%rbp)
```

`sooth_main` disassembly (excerpt, addresses relative to function start):

```
  +12:  lea -0x40(%rbp),%rdi ; call mkempty__m0     ; mkempty (sret) -> Nil
  +25:  mov $0x18,%edi ; call sooth_alloc           ; 24-byte cell
  +35:  mov (%rbx),%rcx …                           ; copy 3 words Nil -> cell
  +57:  movl $0x1,-0x28(%rbp)                       ; tag 1
  +64:  mov $0x1,%ecx ; add $0x0,%rcx
  +73:  mov (%rcx),%rcx                             ; FAULT: deref address 1
 +162:  call sooth_enum_drop_3
 +175:  call sooth_mono_empty_Monoid_0_List__T0___m0__t0_e2_List_i64_
```

Verdict: the SIGSEGV is at `sooth_main+73`, inside the **first `^ Cons` construction in
main** — code that runs *before* the `empty[List[i64]]` call ever executes. The faulting
instruction dereferences the i64 literal `1` as a pointer (`mov $1,%ecx; mov (%rcx),%rcx`).

---

## Step 3 — narrowing the trigger

New fixtures (all built + run on the patched tree, method as in step 1):

| fixture | body | result |
|---|---|---|
| `p8bf-n1-cons-drop-no-empty.sth` (new) | `1 mkempty ^ Cons drop` then `"ok" .` — **no empty call** | exit 0, `ok` |
| `p8b-monoid-list-append-show.sth` (reused, rebuilt fresh) | Cons×5 + recursive combine + showlist, **no empty call** | exit 0, prints `1 2 3 5 3` |
| `p8bf-m6` / `p8bf-m7` (step 1) | bare `Nil` then empty | exit 0, `ok` |

```
$ target/debug/sooth build /tmp/p8b-probes/p8bf-n1-cons-drop-no-empty.sth && /tmp/p8b-probes/p8bf-n1-cons-drop-no-empty
ok
exit=0
$ target/debug/sooth build /tmp/p8b-probes/p8b-monoid-list-append-show.sth && /tmp/p8b-probes/p8b-monoid-list-append-show
1
2
3
5
3
exit=0
```

Verdict: Cons construction alone is fine (even with the same `impl: Monoid for List`
present); empty alone is fine; a bare prior `Nil` does NOT suffice. The crash requires
the `empty[List[i64]]` mono call to exist in the program — yet fires in code that runs
before it. Conclusion: a **compile-time** interaction, not a runtime-order one.

---

## Step 4 — compiler-side root cause (SSA diff)

SSA dumped via a throwaway scratch crate (`/tmp/ssa-dump`, path-dependency on the worktree
crate, calls `sooth::driver::emit_ssa`) — the CLI has no IR dump flag; this mirrors the
`tests/phase7b_slice8.rs` IR-pin pattern without touching the worktree.

```
/tmp/ssa-dump/target/debug/ssa-dump /tmp/p8b-probes/p8b-bisect-v1-drop-then-empty.sth  > m2.ssa
/tmp/ssa-dump/target/debug/ssa-dump /tmp/p8b-probes/p8b-side-empty-instantiation-only.sth > m1.ssa
/tmp/ssa-dump/target/debug/ssa-dump /tmp/p8b-probes/p8bf-n1-cons-drop-no-empty.sth > n1.ssa
/tmp/ssa-dump/target/debug/ssa-dump /tmp/p8b-probes/p8bf-m6-bare-nil-then-empty.sth > m6.ssa
```

n1 (green, no empty call) — main's Cons, correct 24-byte `List[i64]`:

```
 %v0 =l copy 1
 %v1 =:List.5b.i64.5d. call $mkempty__m0()
 %v3 =l call $sooth_alloc(l %v2)      ; 24
 %v4 =l alloc8 24
 storew %v5, %v6                      ; tag 1
 storel %v0, %v7                      ; element = i64, 8 bytes at +8
 storel %v3, %v8                      ; rest ptr at +16
 call $sooth_enum_drop_2(:List.5b.i64.5d. %v4)
```

m2 (crash) — main's Cons, WRONG: 40-byte `List[List[i64]]`, element blitted from the
i64 operand's raw value:

```
 %v0 =l copy 1
 %v1 =:List.5b.i64.5d. call $mkempty__m0()
 %v3 =l call $sooth_alloc(l %v2)      ; 24
 blit %v1, %v3, 24
 %v4 =l alloc8 40                     ; <- 40-byte "Cons"
 storew %v5, %v6                      ; tag 1
 blit %v0, %v7, 24                    ; <- element: blit 24 bytes FROM %v0 (=1) — the SIGSEGV
 storel %v3, %v8                      ; rest at +32
 call $sooth_enum_drop_3(:List.5b.List.5b.i64.5d..5d. %v4)
```

The empty mono itself, in BOTH m1 (green) and m2 (crash), is declared one level too deep:

```
export function :List.5b.List.5b.i64.5d..5d. $sooth_mono_empty_Monoid_0_List__T0___m0__t0_e2_List_i64_() {
@start
 %v0 =l alloc8 40                     ; constructs a 40-byte Nil = List[List[i64]]
 %v1 =w copy 0
 storew %v1, %v2
 ret %v0
}
```

(`empty[List[i64]]` over `Monoid for List` must return `List[i64]` = a 24-byte Nil.)

m6 (green, `mkempty drop` + empty): mkempty's Nil **also** allocates 40 bytes while its
declared type and its drop (`sooth_enum_drop_2(:List.5b.i64.5d.)`) still say `List[i64]`:

```
export function :List.5b.i64.5d. $mkempty__m0() {
@start
 %v0 =l alloc8 40                     ; <- clobbered layout, harmless for a unit Nil
 %v1 =w copy 0
 storew %v1, %v2
 ret %v0
}
```

Verdict: `empty[List[i64]]` (a) mints the `empty` monomorph with the WRONG θ — one level
too deep (`List[List[i64]]` instead of `List[i64]`) — and (b) that wrong instantiation's
variant words clobber the lowering-side variant map, re-typing **every** concrete-body
`Cons`/`Nil` construction program-wide. A 40-byte `Nil` is harmless (tag 0 → drop returns;
why m1/m6 stay green); a clobbered `Cons` blits its 24-byte element field from the i64
operand's value → dereference of address 1 → SIGSEGV.

Compiler-side attribution (function:line on the patched tree):

1. **θ seeding defect (primary)** — `resolve_mono_member_call`'s S6-R4 nullary branch
   (`src/check/poly.rs:2363-2391`) finds the impl via `find_bound_impl(zero_tid, List[i64])`
   (correct: target `List['E]` unified with `List[i64]` gives `'E := i64`) but then falls
   into the generic branch and hands the **call-site `type_args`** to `check_poly_call`,
   which seeds θ **positionally** ("position `i` binds variable `i`",
   `src/check/poly.rs:7426-7452`, loop at 7447-7452). For a generic-target impl the member
   word's variable #0 is the **impl header's own element var** (`List`'s `'E`; the trait
   var `'T` was already substituted away by the impl desugar), so `empty[List[i64]]`
   seeds `'E := List[i64]` instead of `'E := i64` → the mono's body `Nil` (i.e.
   `List['E]`) becomes `List[List[i64]]`. The S6 golden never hit this because
   `empty[i64]` over a **concrete** target (`Monoid for i64`) takes the mono branch
   (`ground_member_type`), never the seeding path.
2. **Bare-name collision amplifier (lowering)** — grounding that output mints the enum
   instantiation `List[List[i64]]`, whose generated variant words register under **bare
   names** (`"Cons"`, `"Nil"`) in the lowering-side word map, which is
   **last-write-wins across every monomorph of one header**
   (`src/ir/func_builder/calls.rs:896-921`, bare-key miss fallback at :931; the hazard is
   pre-documented at `src/ast.rs:1421-1424`, `instantiate_enum`'s doc: "two instantiations
   sharing a bare `Ok` would silently clobber each other there"). Concrete-body
   construction sites have **no** span-keyed `enum_words` record (that map exists only for
   poly-callee body sites — `src/check/poly.rs:7695-7703`; the fall-through is pinned by
   `enum_words_miss_falls_through_to_bare_key_lookup`, `src/ir/func_builder/calls.rs:2542-2547`),
   so main's `Cons` lowers from the clobbered entry. The CHECKER saw correct types (the
   fixture builds clean); only the lowering re-derives shapes from the clobbered map.

---

## Step 5 — attribution: pre-existing vs introduced

Method: `git stash push -m "p8b-probe-arm…"` (recorded: stash@{0}, later popped; the prior
round's stash@{1} untouched), rebuild at base c406149, re-run. Control shape: a generic
enum with **no self-reference field**, so the impl compiles at base without ever reaching
the patched arm — `type: Opt['T] | None | Some 'T` (the P8-2a lesson: only self-referential
fields hit the wall).

Fixture `p8bf-opt-crash.sth` (new):

```sth
import: intrinsics * ;
import: hosted::show | . | ;
type: Opt['T] | None | Some 'T ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for Opt
  : empty None ;
  : combine drop ;
;
: mkopt ( i64 -- Opt[i64] ) Some ;
: main ( -- )
  1 mkopt drop
  empty[Opt[i64]] drop "ok" . ;
```

```
# PATCHED tree (arm applied)
$ target/debug/sooth build /tmp/p8b-probes/p8bf-opt-crash.sth      -> build exit=0
$ /tmp/p8b-probes/p8bf-opt-crash                                  -> exit=139, output: []

# BASE (arm stashed, c406149 rebuilt)
$ target/debug/sooth build /tmp/p8b-probes/p8bf-opt-crash.sth      -> build exit=0
$ /tmp/p8b-probes/p8bf-opt-crash                                  -> exit=139, output: []
```

Fixture `p8bf-opt-empty-only.sth` (empty call only — the θ defect without a Cons):

```
# PATCHED: exit=0, [ok]; mono declared  :Opt.5b.Opt.5b.i64.5d..5d.  = Opt[Opt[i64]]  (WRONG)
# BASE:    exit=0, [ok]; mono declared  :Opt.5b.Opt.5b.i64.5d..5d.  = Opt[Opt[i64]]  (WRONG)
$ /tmp/ssa-dump/target/debug/ssa-dump /tmp/p8b-probes/p8bf-opt-empty-only.sth | grep sooth_mono_empty
 %v0 =:Opt.5b.Opt.5b.i64.5d..5d. call $sooth_mono_empty_Monoid_0_Opt__T0___m0__t0_e2_Opt_i64_()
export function :Opt.5b.Opt.5b.i64.5d..5d. $sooth_mono_empty_Monoid_0_Opt__T0___m0__t0_e2_Opt_i64_()
```

(identical on base and patched tree)

Concrete-target control `p8bf-concrete-i64-control.sth` (`impl: Monoid for i64`,
`: empty 0 ;`, `empty[i64]` after i64 work):

```
target/debug/sooth build /tmp/p8b-probes/p8bf-concrete-i64-control.sth -> build exit=0
/tmp/p8b-probes/p8bf-concrete-i64-control -> ok / exit=0      (green at base AND patched)
```

List shape at base (the known wall, re-confirmed verbatim):

```
$ target/debug/sooth build /tmp/p8b-probes/p8b-bisect-v1-drop-then-empty.sth   # at BASE

thread 'main' (1506535) panicked at src/check/poly.rs:6193:18:
internal error: entered unreachable code: a generic `type:` field is never Generic { is_enum: true, idx: 0, module: 1, args: [Var(0)], len_args: [], name: "List" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

Unstash recorded verbatim:

```
$ git stash pop
On branch pi-parallel-d9e51aa6-668f-4001-a843-9ac1bac25c01-s0-0
Changes not staged for commit:
 modified:   src/check/poly.rs
 modified:   tests/phase7b_slice6.rs
Dropped refs/stash@{0} (e8cb269fbd53cfaa4d7824d331372f2c331bb87f)
```

Patch identity re-verified: `git diff > /tmp/p8bf-restored.diff; git apply --check --reverse
/tmp/p8b-arm.patch` → clean (the working-tree diff is exactly the probe arm). Crash and
green controls re-run on the restored tree: List 139, Opt 139, empty-only ok, m6 ok,
concrete-i64 ok.

Verdict: **PRE-EXISTING.** Both defects (wrong θ seeding for a nullary member over a
generic impl target; last-write-wins bare-name variant-word clobbering in lowering) are
reachable at base c406149 through `impl: Monoid for Opt` — no patched code involved. The
S8b arm neither causes nor fixes them; it only makes `impl: Monoid for List` compilable
(base panics at the wall, `src/check/poly.rs:6193`), which makes the *List spelling* of the
pre-existing crash newly expressible. The concrete-target nullary path (S6's own golden) is
confirmed unaffected.

---

## Step 6 — verdict and scope recommendation

**Root cause (two coupled defects, both pre-existing at c406149):**

1. `src/check/poly.rs:2363-2391` (S6 R4 nullary branch) + `src/check/poly.rs:7426-7452`
   (`check_poly_call`'s positional θ seeding): for a nullary trait member called with an
   explicit type argument over a **generic impl target**, the call-site type argument
   (meant for the trait's type variable) is bound positionally to the member word's own
   variable #0 — which for a generic-target impl is the **impl header's element
   variable**, not the trait variable. `empty[List[i64]]` therefore instantiates the
   member at `'E := List[i64]` instead of `'E := i64`, minting a monomorph typed/returning
   `List[List[i64]]` (one level too deep). Observable statically in the SSA of the
   green cases too (m1/m6/opt-empty-only).
2. `src/ir/func_builder/calls.rs:896-921` (+ `:931`, + the pre-documented hazard at
   `src/ast.rs:1421-1424`): grounding that wrong output mints the `List[List[i64]]`
   instantiation, whose generated variant words overwrite the **bare-name-keyed,
   last-write-wins** lowering-side variant map. Every concrete-body construction site
   resolves exclusively through that map (no span-keyed record exists for concrete
   bodies), so all `Cons`/`Nil` constructions program-wide lower with the wrong
   instantiation's fields: `Nil` becomes 40 bytes (harmless — why the green cases are
   green), and `1 mkempty ^ Cons` blits its 24-byte element field from the i64 operand's
   raw value → `mov (%rcx),%rcx` with `rcx=1` → SIGSEGV at `sooth_main+73`.

**Minimal repro (patched tree; Opt variant reproduces at base):** prior generic-enum
construction + an `empty[<instantiation>]` call anywhere later in the program:

```
1 mkempty ^ Cons drop
empty[List[i64]] drop "ok" .        # p8b-bisect-v1-drop-then-empty.sth → SIGSEGV
```

**Attribution:** pre-existing (S6-era). S8b's arm exposes the List spelling by lifting the
impl-declaration wall; an in-slice S8b fix is not strictly required for *correctness of the
arm*, but the wall-lift's whole purpose (generic-target Monoid impls over self-referential
enums) is unusable until the pair is fixed, and the silent 40-byte layout corruption can
miscompile *any* program that merely contains a wrong-typed `empty[…]` call.

**Scope recommendation (evidence-based, for the spec author to decide):**

- **Recommendation: fold the fix into S8b as a required, explicitly-scoped phase** rather
  than carving out a separate slice — with the two defects fixed as two named requirements:
  (a) the nullary generic-target path must seed θ through the impl-target equation
  (`'E := i64`), not positionally; (b) the lowering-side variant-word map must be keyed
  per-instantiation (id- or spelling-keyed), retiring the documented last-write-wins
  hazard. Rationale: S8b's exit criteria (List Monoid end-to-end) cannot pass with this
  bug present — `empty[List[i64]]`-involving programs are either statically mistyped
  (green cases) or SIGSEGV (any Cons in program) — and the fix surface is exactly the two
  functions S8b's slice already touches (`poly_bind_construction_arg` neighbourhood +
  lowering enum-word resolution), so a carve-out slice would re-derive this same probe
  context. If the S8b budget cannot absorb it, the honest fallback is to fence: keep the
  wall (revert the arm) and record the pair as the pre-existing blocker, since shipping the
  arm alone turns a compile-time panic into a latent runtime SIGSEGV for List-shaped
  programs. Either way, add the Opt-shaped base repro as a golden (source in → expected
  diagnostic or clean run) so the fence/fix is pinned independent of the List wall.
- Not recommended: recording as a known limitation without a fence — the corruption is
  silent (green exit 0 on the mistyped path) and program-global, which is exactly the class
  of failure Sooth's diagnostics-as-behavior convention exists to prevent.

---

## Step 7 — cleanup

```
$ git checkout -- src/ tests/
$ git status --short
(clean)
```

(Performed at round end; the stash pop was already recorded in step 5. No commit was made
at any point. The prior round's stash@{0} ("p8b probe arm + witness flip (also at
/tmp/p8b-arm.patch)") was deliberately left in place.)

# Part 3 — Spellings log (worker 0a9a5d78 salvage + main-line completion, verbatim)

# P8b spellings probe log — member-surface facts for the S8b spec

Completed by the main line (260907) after the dispatched worker (run 0a9a5d78)
timed out at 45 min mid-item-1. Its partial findings are preserved below with
attribution; the orphaned fixtures were re-run to pin their verdicts, and the
one missing probe (item 4) was written and run here. Environment: worktree at
`c406149` + `/tmp/p8b-arm.patch` (the P8b probe arm); canonical gate green with
the patch (fmt clean, `cargo clippy -- -D warnings` clean, 3293/0). Pre-existing
clippy drift noted: 16 `needless_borrow` hits under `--tests` only, all in the
untouched `tests/phase4_quotations.rs`.

Invocation: `./target/debug/sooth build <fixture> --manifest tests/fixtures/sooth.pkg`,
then run the binary at the fixture path minus `.sth`. All verdicts below were
re-verified on the patched tree; run-twice where behavioral.

## Item 1 — distinct-'U through a shared bound (the worker's item, completed)

Baseline pins re-confirmed byte-exact first:

- `p8b-map-distinct-u-unbound.sth`: `has output variable 'U that no input binds`
- `p8b-map-distinct-u-explicit-list-head.sth`: `generic type List declares 1 type
  variable, but none were supplied`

Verdicts (all /tmp/p8b-probes/p8b-sp-*.sth):

- **1(a) quotation type in an explicit instantiation list — REJECTED at check.**
  The grammar technically parses a bare `[` in type position
  (`parse_type_expr` → `parse_quotation_type_expr`), but the check rejects it
  against a plain-var slot — byte-exact, three spellings:
  `a quotation cannot be passed to`qid`; only`call`accepts one`
  (p8b-sp-1a-qtype-element, -control, -owning). The `owning [ i64 -- i64 ]`
  form parses as a type but then fails at the operand:
  `` `qid` ... was instantiated at `'T` = `owning [ i64 -- i64 ]` but its
  operand is `[ i64 -- i64 ]` `` (p8b-sp-1a-qtype-owning-middleman).
- **1(a/d) applied-head / spaced-head instantiation — fenced at dispatch.**
  `mapadd[List[i64] i64 i64]` (and the spaced variant) passes parsing and
  var-binding but fails member dispatch — byte-exact:
  `` `mapadd` in `main` ... was instantiated at `'F` = `List[i64]` but its
  operand is `List` `` (p8b-sp-1a-mapadd-list-args, -quot-types, -quot-u,
  -1d-mapadd-space-head).
- **1(b) two-hop (poly→poly passing a quotation) — LOCATED FENCE, byte-exact:**
  `` `m2` cannot call the polymorphic word `m1` ... passing a quotation to a
  polymorphic word is not yet supported from a polymorphic body; call `m1`
  from a monomorphic word instead `` (p8b-sp-1b-two-hop, -two-hop-param).
- **1(b-alt) mono middleman with an explicit quotation type — GROUNDS.**
  p8b-sp-1a-qtype-mono-middleman: build OK, run exit 0, stdout `4`. The one
  route where a quotation flows through a type position: a monomorphic
  middleman whose concrete quotation row carries it.
- **1(c) 'U := 'T specialization — GROUNDS.** p8b-sp-1c-uspecial: exit 0, `4`.
- **1(c) composition (map ∘ map via two same-type wrappers) — GROUNDS.**
  p8b-sp-1c-compose: exit 0, `7`.

**One-line statement:** a map producing a distinct `'U` through a shared bound
is NOT spellable today — inference does not bind output-only variables, explicit
instantiation cannot carry a quotation type against a plain-var slot, and
poly→poly quotation passing is fenced. Working substitutes: `'U := 'T`
specialization, composition of two same-type-bound maps, or a monomorphic
middleman. (Worker's load-bearing mechanical finding, preserved: quotation
literals only ever unify against quotation-typed ROWS — which is also *why*
the output-only `'U` is never bound: the row check compares against
already-bound variables, it does not bind.)

## Item 2 — bounded mconcat at depth 1 — VERIFIED (by suite)

The S6 golden `mconcat_over_list_dispatches` (tests/phase7b_slice6.rs:359,
expects `6`) passes within the verified full-suite run on the patched tree
(3293/0). Grounds and runs; candidate S8b golden as-is.

## Item 3 — `empty` routes for the List impl — VERIFIED (both routes)

- Explicit in a mono main: `empty[List[i64]] drop "ok" .` alone grounds and
  runs green (P8b round fixture p8b-side-empty-instantiation-only.sth) — with
  the now-root-caused caveat that any prior List construction in the same
  program crashes (pre-existing two-defect bug; see the segfault ledger entry).
- Bound-directed inside a poly body: exercised by the S6 mconcat golden
  (`empty swap fold`), passing in the same suite run.

## Item 4 — `combine` through a Monoid bound — GROUNDS (new probe here)

Fixture p8b-sp-4-combine-through-bound.sth: `merge['T: Monoid] ( 'T 'T -- 'T )
combine` called on two `List[i64]` values. Build OK; run exit 0, stdout
`1 2 3 5 3` (correct spine order), green twice. Candidate S8b golden.

## Item 5 — witness sweep — VERIFIED (by suite)

`cargo test --test phase7b_slice6` green within the 3293/0 patched-tree run;
the flipped witness `monoid_for_list_append_construction_grounds_after_s8b`
passes; everything else unchanged.

## Worker-run provenance

The dispatched worker (0a9a5d78, syn-large-text, 45-min budget) applied the arm
patch, re-confirmed baselines, ran the item-1 battery through the two-hop and
middleman probes, and was killed mid-item-1(a) tail. Its worktree was cleaned
by the runner; its patch file was byte-identical to /tmp/p8b-arm.patch (no new
source edits). Its transcript conclusions are incorporated above and marked.
No log was written by the worker; this file is the round's log.

# Part 4 — Consolidated verdict ledger

## salvage note — segfault-probe dispatch (p7b-s8b)

- dispatch: segfault probe, grounded brief (no code changes; note to /tmp)
- salvage: probe arm diff + witness artifacts present on disk; verdict pending — child was killed mid-flight, no output returned
- ledger: salvage noted; no verdict filed. awaiting re-dispatch before any conclusion is drawn.

## verdict — segfault root-cause (d9e51aa6, completed 260907)

- Verdict: PRE-EXISTING (S6-era), not introduced by the S8b arm. Two coupled defects:
  1. θ seeding: resolve_mono_member_call's nullary branch (src/check/poly.rs:2363-2391) hands call-site type_args to check_poly_call's positional seeding (poly.rs:7447-7452); for a generic impl target (Monoid for List), member var #0 is the impl header's element var, so empty[List[i64]] seeds 'E := List[i64] instead of i64 -> mono returns List[List[i64]] (one level too deep, visible in SSA even on green runs).
  2. Variant-map clobber: the wrong mint's variant words overwrite the lowering-side bare-name-keyed last-write-wins map (src/ir/func_builder/calls.rs:896-921; hazard pre-documented at src/ast.rs:1421-1424) -> ALL Cons/Nil constructions program-wide lower with wrong fields; Nil 40 bytes (harmless), Cons blits 24-byte element from i64 operand -> SIGSEGV at sooth_main+73 BEFORE the empty call executes.
- Attribution: base repro without the arm (type Opt['T] + impl: Monoid for Opt, no self-reference field) SIGSEGVs identically; empty-only variant mists Opt[Opt[i64]]; List shape at base hits the impl-declaration wall, so the arm's role is EXPOSURE not causation. Concrete-target control (Monoid for i64, empty[i64]) green.
- Scope recommendation: fold fix into S8b as a required two-requirement phase (theta seeding via impl-target equation; per-instantiation variant-word keys) + Opt-shaped base repro as a golden; fallback = keep the wall with a fence (arm alone turns compile-time panic into silent program-global miscompile).
- Log: /tmp/sooth_p8b_segfault.md (443 lines, verbatim).
- Ledger status: this closes the segfault question; remaining open item: spellings worker (0a9a5d78).

## verdict — spellings round (0a9a5d78 salvaged + main-line completion, 260907)

- Distinct-'U through a shared bound: NOT spellable by any route (inference doesn't bind output-only vars; quotation types rejected at check vs plain-var slots — "only call accepts one"; poly->poly quotation passing fenced). Working substitutes all ground: 'U:='T specialization, composition of two same-type maps, mono middleman with explicit quotation type.
- Mechanical root: quotation literals only unify against quotation-typed ROWS; the row check compares against already-bound vars, never binds output-only ones.
- Bounded mconcat at depth 1: VERIFIED by suite (S6 golden mconcat_over_list_dispatches green on patched tree) — candidate golden.
- empty routes: both verified (explicit mono main with the now-root-caused segfault caveat; bound-directed in poly body via S6 golden).
- combine through a Monoid bound: GROUNDS (1 2 3 5 3, green twice) — candidate golden. New fixture p8b-sp-4-combine-through-bound.sth.
- Witness sweep: green in suite (flipped witness passes, nothing else changed).
- Log: /tmp/sooth_p8b_spellings.md. Worktree reverted clean at c406149; probe-arm stash preserved separately.
- Ledger status: ALL P8b questions closed. Spec-writer unblocked.
