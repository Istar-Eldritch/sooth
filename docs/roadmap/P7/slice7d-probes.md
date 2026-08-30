> Probe round for P7.S7d, run 260830 against the current tree (worktree
> `p7-s7d`, branch `vietnamese`) by one disposable compile-probe subagent:
> the `.` intrinsic retired in-place (Phase A deletions), then a candidate
> `hosted::show`/`hosted::libc` shape iterated under `/tmp/s7d-probes/`.
> All tracked-file edits were reverted at the end; this file is the only
> repo artifact. Baselines and every verbatim capture were taken before any
> revert. Companion to the census in the S7d brief update.

# P7.S7d compile-time probe results

Probes for retiring the compiler-intrinsic `.` onto `core::show`'s
`Show`/`Write` traits (brief R1-R3). Commands ran from the repo root as
`cargo run -- run --manifest examples/sooth.pkg /tmp/s7d-probes/<file>`
(a program outside a package can only import `intrinsics`, so every probe
program resolves `core`/`hosted` through `examples/sooth.pkg`).

**Phase A deletions applied for all probes** (all reverted after):
`src/check/builtins.rs` printable-rows loop; `src/check/operators.rs` `.`
entries in `is_operator`/`is_unary` and the `"."` arm; `src/ast.rs`
`is_name_dispatched_builtin` excludes `"."` like the six comparisons;
**`src/resolve.rs` `is_operator_dispatch_name` drops `"."`** (a site the
brief's R2 delete list did not name — see P3f: without it, a selectively
imported `.` stays unrewritten expecting builtin dispatch that no longer
exists, and every bare call is `unknown word '.'`). `.` stays in
`BUILTIN_WORDS`; `Instr::Print`/`$fmt` data left in place (dead).

## Summary table

| Question | Outcome |
|---|---|
| P1: can user land print a str byte-exactly through write(2)? | **Yes.** `extern: sys-write-str ( i32 cstr usize -- isize ) "write"` accepts a `cstr` param (explicit `cstr` word conversion); `s len` then `s cstr` then write(2) reproduces the str row exactly: no newline, exact bytes. The P6d lesson (bare literal rejected) does not apply to the explicit conversion. |
| P2: does a poly `: . ['T: Show] ( 'T -- )` work? | **Parses and registers, but its body is unwriteable today.** Two independent blockers: (a) a local-borrowed `&!StrBuf` does not unify with a poly callee's declared `&!StrBuf` param — both *render* identically, so the diagnostic is a look-alike mismatch; root cause: parser.rs:4117 (Slice 13 R-A4) folds a fully-concrete `&!T` slot to `PolyType::Concrete(Type::Ref(rid))` while `poly_reference_word`'s local-borrow arm produces native `PolyType::Ref` — `poly_cross_match`'s Ref arm needs both sides native. (b) Field accessors (`&!len`, `&!data`) are flat-out unsupported in generic bodies (`poly_unsupported_accessor_error`: "monomorphize this word (or write a concrete wrapper)"), so even with (a) fixed the in-buffer newline append cannot be spelled; `+!` is likewise a located error in generic bodies (R-B6). A poly word with the buffer as a *declared input* (`: t ['T: Show] ( 'T &!StrBuf -- ) render ;`) **works** — so the checker gap is narrow, but R1's `. ( 'T:Show -- )` shape needs a checker fix (unfold declared refs, or normalize at the match sites). `inline` does not escape it: the abstract declaration check fails first. |
| P3: where can the two dots live? | One module cannot mix a generic and a concrete candidate (`overlaps a concrete overload`); two modules' selective `.` imports collide for the importer; **two same-arity CONCRETE candidates in one module are legal**, but intra-module bare calls across them don't resolve ("unknown word `.`") — bodies must go through distinctly-named private helpers. Callers import ONE module's `.` and dispatch per-site: **works** (P3f). So the working layout today: all concrete dots in one module (hosted::show), private helpers for internal delegation. The generic dot cannot join them without the P2 checker fix. |
| P4: core::bool overload fate + Bool delta | **Deletion forced by layering**: post-retirement its body's inner str `.` cannot resolve from the core layer (`unknown word '.' in '.'`), since printing is now hosted-only. With it deleted and `Show for Bool` present, `True .` prints `true\n` — **lowercase, vs today's `True\n`** — an observable behavior change the spec must rule on consciously (the book's lowercase examples then become right). Also drop `.` from core/bool.sth's `import: intrinsics` and `export:` lines. |
| P5: stale `import: intrinsics \| . \| ;` | **Silently ignored** at the import (no validation error); the failure surfaces later as a plain `unknown word '.'` at the call site with no hint toward the new import. Migration must touch those lines; the diagnostic gap is a candidate for the diagnostics track. |
| P6: missing-type diagnostics | `no overload of '.' in 'main' (line N) accepts these operands` + the candidate list (Bool/i64/isize/str/usize). Correct but names no fix; once the generic dot lands the message becomes the Show-bound one. |
| P7: narrow-int Show impls | **Work**: `impl: Show for u8/u64/i8` in core::show via widening to the u64 append path (`n >u64 append-digits`; i8 sign test must widen first: `n >i64 0 lt` — a bare `n 0 lt` resolves the literal ambiguously and errors). Mono dots over them print `255\n`, `-5\n`, `1844674407370955161\n` byte-exact. |
| P8: floats via snprintf extern | **INCONCLUSIVE — blocked on the user-extern f64 ABI.** The shape compiles and runs: `extern: g-fmt ( &!array[u8 64] usize cstr f64 -- i32 ) "snprintf"`; the returned count and buffer prove the fmt string arrived and C ran — snprintf returned 2 and wrote `"0\n"`, i.e. **the f64 arg arrived as 0**. A control through libm (`llround`/`fmax` externs) failed to **link**: libm is not linked for user programs. So the snprintf path needs (i) the f64-extern-arg ABI fixed or re-diagnosed and (ii) a libm link story; the alternative is pure-Sooth `%g` in `core::show` (its own slice-sized job). |
| P9: output ordering | Covered by P3f's run: str/Show/Bool/usize prints interleave strictly in call order (`42\n-7\nhitrue\n100000\n`) — everything is write(2), so the P6c interleaving hazard is gone by construction. |
| P10: spot migration | Not run (census covers the shape). Byte-exact baselines captured for the implementer: gcd `5\n`; mean `2.5\n`; shapes `12.5664\n12\n5\n7\n`; strings `12\n12\nhello, sooth\n`; `sooth test examples/tests/bool.sth` driver output `ok   examples/tests/bool.sth\n1 entries, 0 failed (4 ok, 0 not ok assertions)\n` (the driver parses `ok --`/`not ok --` lines from captured stdout — expect's format is load-bearing and is str-path-only, so it survives retirement unchanged). |
| Bonus: the real UX shape | `import: hosted::show \| . \| ;` + bare `42 .` / `"hi" .` / `True .` resolves per-site across the concrete candidates — byte-identical to P3f — **after** the resolve.rs `is_operator_dispatch_name` deletion. This is R1's migration shape, working. |

## Verbatim captures

### Baseline (pre-deletion), byte-exact

`cargo run -- run examples/gcd.sth` → exit 0, stdout `5\n` (od: `5 \n`).
`cargo run -- run examples/mean.sth` → `2.5\n`. shapes → `12.5664\n12\n5\n7\n`.
strings → `12\n12\nhello, sooth\n` (final line has NO trailing newline).
`cargo run -- test examples/tests/bool.sth` → exit 0,
`ok   examples/tests/bool.sth\n1 entries, 0 failed (4 ok, 0 not ok assertions)\n`.

All-paths baseline (`/tmp/s7d-probes/base_all.sth`:
`42 . / -7 . / 255 >u8 . / -5 >i8 . / 100000 >u64 . / 3.5 . / 2.5 >f32 . /
True . / False . / "a\tb" . / "hi" cstr . / "one\ntwo\n" .`), od -c:

```
0000000   4   2  \n   -   7  \n   2   5  5  \n   -   5  \n   1   0   0
0000020   0   0   0  \n   3   .   5  \n   2   .   5  \n   T   r   u   e
0000040  \n   F   a   l   s   e  \n   a  \t   b   h   i   o   n   e  \n
0000060   t   w   o  \n
```

Note `True .`/`False .` — capitalized today (core::bool's overload), and the
str row's no-newline vs the numeric rows' baked `\n` are both visible.

### P1 — cstr-taking extern, explicit conversion (exit 0)

`/tmp/s7d-probes/p1a_extern_cstr.sth`:
`extern: sys-write-str ( i32 cstr usize -- isize ) "write" ;` +
`"hi" | s | s len | n | 1 >i32 s cstr n sys-write-str drop ;`
→ exit 0, od: `h i` — two bytes, no newline. The `cstr` param is legal at an
extern with the explicit conversion word (contrast slice7c P6d's bare literal).

### P2 — the poly dot's two blockers

`(a) look-alike ref mismatch` — `/tmp/s7d-probes/p2_poly_pieces.sth`,
`: t3 ['T: Show] ( 'T -- ) | v | fresh-buf | b | v &!b render b drop ;`:

```
error: type mismatch in `t3` (line 10)
  `render` expected `&!StrBuf`, found `&!StrBuf`
  note: declared ( -- )
```

(`note: declared ( -- )` is cosmetic — a poly word's concrete effect is
empty by design.) The declared-input control PASSES —
`/tmp/s7d-probes/p2c_poly_ref_param.sth`,
`: t3c ['T: Show] ( 'T &!StrBuf -- ) render ;` called from mono — exit 0.

`(b) field accessors` — `/tmp/s7d-probes/p2d_poly_flush.sth`,
`b &!len @` inside a poly body:

```
error: `&!len` is not yet supported in a generic body, in `t6` (line 10)
  monomorphize this word (or write a concrete wrapper) to use `&!len` today
```

`(c) flush fails identically` — `/tmp/s7d-probes/p2f_poly_flush_only.sth`,
`&!s &!b flush` inside a poly body:

```
error: type mismatch in `t7` (line 11)
  `flush` expected `&!StrBuf`, found `&!StrBuf`
  note: declared ( -- )
```

`(d) inline does not help` — `/tmp/s7d-probes/p2e_inline_poly.sth`,
`: dot-inline inline ['T: Show] ( 'T -- ) ...` — same render mismatch at the
declaration-time abstract check (line 12), before any splice.

`(e) mono-body borrow friction (in-buffer append)` — the exact
render/`&!data`/`&!len` append sequence in a mono `main`:
`/tmp/s7d-probes/p2g_mono_append.sth`:

```
error: cannot borrow `b` mutably in `main` (line 14, col 7): it is aliased by a value on the stack (pushed at line 9, col 3)
  both denote one region of memory, so a mutation through `b` would be silently visible through that value
  `dup` that value for an independent copy, or consume it before taking the borrow
```

and the shared-read variant `/tmp/s7d-probes/p2h_shared_reads.sth`
(`&b &len @` reads, `b &!data`/`b &!len` writes):

```
error: cannot borrow `b` mutably in `mainA` (line 15, col 7): it is aliased by a value on the stack (pushed at line 11, col 3)
```

So even in mono bodies, the read-modify-write append sequence over one
locally-built StrBuf hits alias records that block the later flush borrow.
The two-syscall shape (render+flush, then `"\n"` through the str path) is the
one that checks clean today. The append-shape question (or a `+!`-style
helper in core::show) is implementer work, not settled by this probe.

### P3 — module layout

`(a) generic + concrete in one module` — `/tmp/s7d-probes/p3a_two_dots_one_module.sth`:

```
error: generic overload `: . ( 'T -- )` (line 19, col 3) overlaps a concrete overload of `.`; a name cannot mix a generic and a concrete candidate
```

`(b) two modules, both exporting`.`, selective imports` —
`/tmp/s7d-probes/p3b_two_modules.sth` imports `.` from both hlibc and hshow:

```
error: selective import of `.` from module `hshow` (line 4, col 30) collides with the selective import of `.` from module `hlibc`
```

`(c) qualified calls work` — `/tmp/s7d-probes/p3c_qualified.sth`:
`42 hshow::.` / `"hi" hlibc::.` → compiles and runs (byte output in P3f's run).

`(d) two CONCRETE candidates, one module` — legal (no duplicate/overlap
error; distinct same-arity input types). See the final candidate source below.

`(e) intra-module bare calls across same-name overloads` — an i64 dot whose
body ends `"\n" .` (the str sibling):

```
error: unknown word `.` in `.` (line 22)
```

Bodies must call a distinctly-named private helper (`print-str`) instead.

`(f) the working caller shape` — `/tmp/s7d-probes/p3f_bare_dot_import.sth`:
`import: hosted::show | . | ;` + bare `42 . / -7 . / "hi" . / True . / 100000 >usize .`
→ exit 0, od:

```
0000000   4   2  \n   -   7  \n   h   i   t   r   u   e  \n   1   0   0
0000020   0   0   0  \n
```

This run REQUIRES the resolve.rs `is_operator_dispatch_name` deletion: with
`.` still listed, the selective import stays unrewritten (expecting builtin
dispatch), and every bare call is `error: unknown word '.' in 'main'`.
(Also observed en route: `import: core::show | Show | ;` collides with
`hosted::show`'s default qualifier — spell one explicitly, e.g.
`import: core::show cshow | Show | ;`.)

### P4 — core::bool overload

`(a) kept` — with the overload restored, any build importing `core::show`
(which imports `self::bool`) fails in bool.sth's own body:

```
error: unknown word `.` in `.` (line 53)
```

(the inner `"False\n" .` / `"True\n" .` can no longer resolve from the core
layer — the str row is gone and hosted is unreachable from core).

`(b) deleted` — overload removed, `import: intrinsics i | branch tag drop | ;`
and `export: Bool False True if unless ;` trimmed (original body preserved as
a comment in the probe's tree edit): `True hshow::.` → `true\n` — lowercase,
vs the baseline's `True\n`.

### P5 — stale intrinsics import

`lib/hosted/testing.sth` unchanged (`import: intrinsics | . | ;`):
`cargo run -- test examples/tests/bool.sth` →

```
error: unknown word `.` in `expect` (line 15)
```

The import line itself is silently tolerated; only the call site fails, with
no hint that printing moved to `hosted::show`.

### P6 — missing-type diagnostics

`/tmp/s7d-probes/p6_missing_types.sth` (`255 >u8 .`) and
`/tmp/s7d-probes/p6b_float.sth` (`3.14 .`), identical shape:

```
error: no overload of `.` in `main` (line 6) accepts these operands
  candidate: `Bool`
  candidate: `i64`
  candidate: `isize`
  candidate: `str`
  candidate: `usize`
```

### P7 — narrow-int Show impls

core/show.sth additions (pattern shown; u64 identical minus widening):

```
impl: Show for u8
  : show | n buf | buf n >u64 append-digits ;
;

impl: Show for i8
  : show
    | n buf |
    n >i64 0 lt ~[
      buf 45 >u8 append-byte
      n >i64 >u64 not 1 >u64 add
    ] ~[
      n >i64 >u64
    ] if
    | mag |
    buf mag append-digits ;
;
```

(A first cut used `n 0 lt` and failed with
`` error: `lt` in `show` (member of trait `Show` for `i8`) (line 140) resolved `'T` to both `i8` and `i64` `` — the sign test must widen first.)
`/tmp/s7d-probes/p7_narrow_ints.sth`: `255 >u8 . / -5 >i8 . / 1844674407370955161 >u64 .` → exit 0, od:

```
0000000   2   5   5  \n   -   5  \n   1   8   4   4   6   7   4   4   0
0000020   7   3   7   0   9   5   5   1   6   1  \n
```

(An intermediate state without the u64 dot gave the P6-style
`no overload of '.'` listing with `u8`/`i8` present — the per-type dot must
exist for each Show impl until the generic dot lands.)

### P8 — floats via snprintf: INCONCLUSIVE

Shape (libc + show additions below) compiles and runs —
`/tmp/s7d-probes/p8_floats.sth` (`3.14 . / 2.5 >f32 . / 1.0 >f64 .`) → exit 0
but od: `0 \n 0 \n 0 \n`. Debug capture `/tmp/s7d-probes/p8b_debug.sth`
prints the returned count and the first five buffer bytes:

```
2
48
10
0
0
0
```

i.e. snprintf returned 2 and wrote `'0' '\n' NUL...` — the fmt cstr arrived
and C ran, but **the f64 argument reached snprintf as 0**. Control attempt
`/tmp/s7d-probes/p8c_f64_extern.sth` — `extern: llround ( f64 -- i64 ) "llround" ;`
and `extern: fabs-arg ( f64 f64 -- f64 ) "fmax" ;` — both fail at LINK:

```
error: "cc" failed: /usr/bin/ld: /tmp/ccBc0gAJ.o: in function `sooth_main':
(.text+0x11): undefined reference to `llround'
collect2: error: ld returned 1 exit status
```

(libm is not linked for user programs; `fmax` ditto.) Conclusion for the
brief: the snprintf float path is blocked on (i) the user-extern f64-arg ABI
(args arrive as 0) and (ii) a libm link story; the compiler's own
`Instr::Print` float arm (which passes `d` args to `printf` correctly)
shows the ABI is expressible in QBE — this is a diagnosable compiler gap,
not a language one. The pure-Sooth `%g` alternative in `core::show` avoids
both but is slice-sized work (float→decimal rendering with no integer div).

### Final candidate source (scratch, reverted; kept verbatim here)

`lib/hosted/show.sth` (new module; register `module: show ;` in
`lib/hosted/sooth.pkg`):

```sth
\ hosted::show -- the print vocabulary over core::show's Show/Write traits.
\ Concrete dots only today (P2's checker gap blocks the generic form);
\ same-arity concrete overloads in one module are legal (P3d); bodies go
\ through distinctly named private helpers (P3e). Newline rides the str
\ path as a second write(2) (P2h borrow friction blocks in-buffer appends).
import: intrinsics * ;
import: core::bool | Bool | ;
import: core::show | StrBuf Show Write render flush | ;
import: self::libc | Stdout g-fmt sys-write | ;

extern: sys-write-str ( i32 cstr usize -- isize ) "write" ;

export: . ;

: print-str ( str -- )
  | s | s len | n | 1 >i32 s cstr n sys-write-str drop ;

: . ( str -- ) print-str ;

: . ( i64 -- )
  | v | 0 >u8 64 fill 0 >usize StrBuf | b |
  v &!b render
  Stdout | s | &!s &!b flush
  s drop b drop
  "\n" print-str ;

: . ( usize -- )
  | v | 0 >u8 64 fill 0 >usize StrBuf | b |
  v &!b render
  Stdout | s | &!s &!b flush
  s drop b drop
  "\n" print-str ;

: . ( isize -- )
  | v | 0 >u8 64 fill 0 >usize StrBuf | b |
  v &!b render
  Stdout | s | &!s &!b flush
  s drop b drop
  "\n" print-str ;

: . ( Bool -- )
  | v | 0 >u8 64 fill 0 >usize StrBuf | b |
  v &!b render
  Stdout | s | &!s &!b flush
  s drop b drop
  "\n" print-str ;

: . ( u8 -- )
  | v | 0 >u8 64 fill 0 >usize StrBuf | b |
  v &!b render
  Stdout | s | &!s &!b flush
  s drop b drop
  "\n" print-str ;

: . ( i8 -- )
  | v | 0 >u8 64 fill 0 >usize StrBuf | b |
  v &!b render
  Stdout | s | &!s &!b flush
  s drop b drop
  "\n" print-str ;

: . ( u64 -- )
  | v | 0 >u8 64 fill 0 >usize StrBuf | b |
  v &!b render
  Stdout | s | &!s &!b flush
  s drop b drop
  "\n" print-str ;

\ P8, inconclusive: snprintf f64 arg arrives as 0 (see capture).
: g-write ( f64 -- )
  | v |
  0 >u8 64 fill 0 >usize StrBuf | b |
  b &!data | d | | scratch |
  d 64 >usize
  "%g\n" cstr
  v
  g-fmt >usize | n |
  1 >i32 d n sys-write drop
  scratch drop ;

: . ( f64 -- ) g-write ;

: . ( f32 -- ) >f64 g-write ;
```

`lib/hosted/libc.sth` additions:

```sth
extern: sys-write-str ( i32 cstr usize -- isize ) "write" ;
\ P8: %g float formatting via libc snprintf, fixed arity.
extern: g-fmt ( &!array[u8 64] usize cstr f64 -- i32 ) "snprintf" ;

export: exit Stdout . g-fmt sys-write ;

\ P3b: the str form of `.` -- exact bytes, no newline, via write(2) over
\ the explicit `cstr` conversion.
: . ( str -- )
  | s |
  s len | n |
  1 >i32 s cstr n sys-write-str drop ;
```

(Note the str dot appears in BOTH modules across the probe history; the
final working shape keeps it only in hosted::show via `print-str`, with
libc exporting only the externs. Either home works for the word itself —
P3b's collision is about a *program* importing two `.` exports, not about
where one lives.)

## Revert verification

`git checkout -- src/ lib/` + `rm lib/hosted/show.sth` (scratch), then
`git status --short` shows only this file, and `cargo build` is green
(verification quoted in the closing acceptance report).
