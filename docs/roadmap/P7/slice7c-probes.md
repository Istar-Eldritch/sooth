> Probe round for P7.S7c, run 260829 against the current tree (worktree `mongols`) by two subagents: a read-only paper recon over parser/checker/dispatch paths, and 23 live compile/run probes under `/tmp/s7c-probes/`. Repo untouched. Findings are condensed into [slice7c-show-brief.md](./slice7c-show-brief.md); this file is the verbatim log.

# P7.S7c compile-time probe results

Probes for the S7c `Show`/`Write` sink-generic printing pair (see
`docs/roadmap/P7/slice7c-show-brief.md`) plus the S7d `.`-retirement context.
All probe files live under `/tmp/s7c-probes/`; nothing in the repo was
modified (`git status --porcelain` empty at finish). Commands were run from
the repo root: `cargo run -- build|run /tmp/s7c-probes/<file>` (the `run`
subcommand exists — `src/main.rs`). The compiler is a lib+bin; `build` writes
the binary beside the source, the `.ssa`/`.s` intermediates go to a deleted
temp dir (`src/driver/toolchain.rs`), so ABI inspection was done by
disassembling the built binary.

## Summary table

| Probe | File | Outcome |
|---|---|---|
| smoke | smoke.sth | compiles+runs, prints `42` |
| P1a two-var trait header | p1a_two_var_trait.sth | parse error (exit 1) |
| P1b bound on 2nd header var | p1b_bound_second_var.sth | parse error (exit 1) |
| P1c 2nd var in member sig (control) | p1c_member_second_var.sth | parse error (exit 1) |
| P2 bound inside member effect, glued `'S:W` | p2_bound_in_effect.sth | parse error (multi-var; NOT the bound error — glued spelling interns a var *named* `'S:W`) |
| P2b bound inside effect, `'S: W` spaced | p2b_bound_in_effect_spaced.sth | parse error — exact bound-in-effect text |
| P2c bound inside ordinary word effect | p2c_word_bound_in_effect.sth | parse error — same text |
| P3a `&!` receiver, last position | p3a_ref_bang_receiver.sth | compiles+runs, prints `p3a hello` |
| P3b `&!` receiver, non-last position | p3b_receiver_nonlast.sth | compiles+runs, prints `p3b hello` |
| P4 (adapted) per-site sink grounding, 2 sinks | p4_sink_grounding_two_sinks.sth | compiles+runs: `one:first` / `two:second` |
| P5a impl of imported trait for local type | p5a_orphan_local_type.sth + p5lib.sth | compiles+runs, prints `mine:7` (legal) |
| P5b impl of imported trait for `i64` | p5b_orphan_i64.sth | orphan error (exit 1) |
| P6a `str` extern input | p6a_extern_str_param.sth | check error (exit 1) |
| P6a2 `str` extern output | p6a2_extern_str_output.sth | check error (exit 1) |
| P6b slice view over filled buffer | p6b_slice_view.sth | compiles+runs, `len` = 16 (whole array) |
| P6b2 Slice[u8] as str | p6b2_slice_as_str.sth | check error (exit 1) |
| P6b3 slice via `cstr` | p6b3_slice_cstr.sth | check error (exit 1) |
| P6c extern write sigs vs real write(2) | p6c_extern_write_sigs.sth | compiles+runs: `OK`/`cstr-ok`/`3`/`8` |
| P6c2 bare `&u8`/`&!u8` extern params | p6c2_extern_ref_u8_decl.sth | compiles (decl-level accept) |
| P6d literal to cstr-taking extern, bare | p6d_literal_bare.sth | check error (exit 1) |
| P6d2 literal via `cstr` | p6d2_literal_cstr.sth | compiles+runs, prints `hi` |
| P7 fallback end-to-end | p7_fallback.sth | compiles+runs, prints `88` twice |

Note: every compiler diagnostic prints as `error: error: ...` under
`sooth build` — `src/main.rs` wraps the message with `error: {e}` while the
stage messages already carry their own `error:` prefix. (Diagnostics text
below is quoted verbatim including that doubling.)

## Verbatim captures

### P1a — `cargo run -- build /tmp/s7c-probes/p1a_two_var_trait.sth` (exit 1)

```
error: error: trait `W2` names more than one type variable at line 7, col 14 (only single-type-variable traits are supported)
```

Source: `trait: W2['A 'B] : both ( 'A 'B -- ) ;`.

### P1b — bound on the second header variable (exit 1)

```
error: error: trait `Show` names more than one type variable at line 5, col 16 (only single-type-variable traits are supported)
```

Source: `trait: Show['T 'S:Write] : show ( &'T &!'S -- ) ;`. The bracket
parse itself tolerates a bound on a header var; the count check fires first.

### P1c — member introduces a second variable (exit 1)

```
error: error: trait `S2` names more than one type variable at line 8, col 5 (only single-type-variable traits are supported)
```

### P2 — glued spelling `'S:W` inside an effect (exit 1)

```
error: error: trait `S1` names more than one type variable at line 7, col 5 (only single-type-variable traits are supported)
```

SURPRISE: `:` is not a lexer delimiter, so `'S:W` (all glued) lexes as ONE
word; `parse_poly_ty_var` only treats a trailing glued colon (`'S:`) or a
standalone `:` token as a bound. A fully-glued `'S:W` silently interns a
type variable *named* `'S:W` — no bound error, and downstream a confusing
multi-var error.

### P2b — spaced spelling `'S: W` inside a member effect (exit 1)

```
error: error: bound on `'S` at line 7, col 17 may not be written inside a stack effect; declare it in the word's bound bracket (e.g. `: f['S: Copy] ( ... )`)
```

### P2c — same shape on an ordinary poly word (exit 1)

```
error: error: bound on `'S` at line 6, col 16 may not be written inside a stack effect; declare it in the word's bound bracket (e.g. `: f['S: Copy] ( ... )`)
```

### P3a — `&!` receiver, last position (exit 0, stdout: `p3a hello`)

Trait `W['S] : write ( &!'S str -- ) ;`, `type: Sk ;`, `impl: W for Sk`
body-form member, bounded poly word `: log['S: W] ( &!'S str -- ) write ;`,
called `&!sk "p3a hello\n" log`. Dispatch works; `&!Sk` grounds per site.

### P3b — receiver at non-last position (exit 0, stdout: `p3b hello`)

Same with `write ( str &!'S -- )`, called `"p3b hello\n" &!sk log`. Works.

### P4 (adapted) — one bounded word, two sink types (exit 0)

```
one:first
two:second
```

Two sink types `Sk1`/`Sk2` with two `W` impls; single bounded
`: log['S: W] ( &!'S str -- ) write ;` called at both. `'S` grounds per call
site; the right impl dispatches each time.

### P5a — impl for a local type, trait imported (exit 0, stdout: `mine:7`)

`p5lib.sth` declares `trait: Greet['T]` + `type: LibType`; entry file
declares `type: MyType` and `impl: Greet for MyType` — LEGAL (orphan rule
satisfied: the target names a struct of the impl's own module). Dispatch
through a bounded word `: dispatch['T: Greet] ( &'T -- ) greet ;` works
cross-module.

Intermediary failure (recorded, then fixed in the probe): a BARE member call
(`&m greet`, or `greet` on the concrete `LibType` in an unbounded word) is
`error: error: unknown word`greet` in `main`(line 15)`. Trait member names
resolve only (a) through the enclosing word's own bound bracket, or (b) as
ordinary words inside an impl member body. This is the same rule
`lib/core/cmp.sth` documents for `cmp`.

### P5b — impl for `i64` outside the trait's module (exit 1)

```
error: error: `impl: Greet for i64` at line 8, col 1 must live in the module declaring `Greet` (`i64` declares no module of its own)
```

### P6a — `extern: emit ( str -- ) "puts" ;` (exit 1)

```
error: error: `extern: emit__m0` declares the input `str` (line 4, col 1)
  a `str` is a pointer and a length, which matches no C parameter; declare `cstr` and convert with `cstr` at the call site
```

(Wart: the message shows the mangled name `emit__m0`, not the source name.)

### P6a2 — `extern: mk ( -- str ) "gets" ;` (exit 1)

```
error: error: `extern: mk__m0` cannot return a `str` (line 4, col 1)
  a `str` may point at static data only, and C supplies no length; declare `cstr`
```

### P6b — slice view over a filled byte buffer (exit 0, stdout: `16`)

`0 >u8 16 fill | buf |` + byte stores via `&!buf i >usize &!> v !` +
`&buf slice` compiles and runs; `len` is 16 — the FULL array length. A
`Slice[u8]` view carries no partial-fill notion; there is no way to view
"first 3 bytes written" as a shorter slice (`subslice` takes indices, so a
0..3 subslice would, but nothing tracks how many bytes were written).

### P6b2 — slice where `str` wanted (exit 1)

```
error: error: type mismatch in `main` (line 10)
  `take` expected `str`, found `Slice[u8]`
  note: declared ( -- )
```

### P6b3 — slice into `cstr` (exit 1)

```
error: error: type mismatch in `main` (line 7)
  `cstr` converts a `str`, found `Slice[u8]`
  note: declared ( -- )
```

A `str` cannot be built from non-static memory: the only `str` constructor
is a string literal (statically rooted). Runtime bytes are reachable as
`Slice[u8]` or as `&array[u8 N]` across FFI, never as `str`.

### P6c — extern write signatures against real write(2) (exit 0)

```
OK
cstr-ok
3
8
```

Accepted + working shapes:

- `extern: write-str ( i32 &array[u8 16] usize -- isize ) "write" ;` —
  shared ref over array; called `1 >i32 &buf 3 >usize write-str .`
- `extern: write-cstr ( i32 cstr usize -- isize ) "write" ;` — called
  `1 >i32 "cstr-ok\n" cstr 8 >usize write-cstr .`
- Rejected: `i32 str usize` (see P6a text — `str` matches no C parameter).
- Decl-level accept (not exercised at a call site): bare `&u8` and `&!u8`
  params (p6c2, exit 0), matching `is_extern_boundary_scalar` which admits
  `Type::Ref(..)` in any mode.
- Precedent in-tree: `examples/resources.sth` uses
  `extern: read ( i64 &!array[u8 64] usize -- isize ) "read" ;`.

Output ordering note: `OK`/`cstr-ok` (direct write(2) syscalls) appear
BEFORE `3`/`8` because `.` lowers to stdio `printf`, which buffers until
exit; mixing write(2) and `.` interleaves badly.

ABI (objdump -d on the built binary, SysV):

- `&array[u8 16]` param: `lea -0x10(%rbp),%rsi` — the array's address is
  passed directly, one slot; `mov $0x3,%edx` (len), `mov $0x1,%edi` (fd).
- `cstr` param: `mov 0x2f3d(%rip),%rsi # 4148 <strd0>` — the literal's
  static root pointer, one slot. Pointer only; there is no ptr+len pair
  across the boundary because `str` never crosses.

### P6d — bare literal to cstr-taking extern (exit 1)

```
error: error: type mismatch in `main` (line 7)
  `puts` wants `cstr`, found `str`: convert it explicitly with `cstr` first (there is no implicit `str` -> `cstr` conversion)
  note: declared ( -- )
```

P6d2 (`"hi\n" cstr puts`) compiles and prints `hi`.

### P7 — fallback end-to-end (exit 0, stdout: `88\n88\n`)

Full source in the final report. Key paths learned while landing it:

- `&'T` receiver CANNOT take a scalar: `error: error: cannot borrow the scalar local`n` of type `i64` in `main`(line 64, col 3)\n  a scalar has no address; borrow a field or an aggregate instead`. The Show member receiver must be BY VALUE (`'T`), the `core::cmp` `Ord` precedent. (This hits the brief's R1 `show ( &'T &!'S:Write -- )` shape for every scalar target: `Show for i64` could never be called on a bare `42` through a reference receiver.)
- A bare member call on a concrete value is `unknown word`; dispatch goes through a bound (`: render['T: Show] ( 'T &!SBuf -- ) show ;`).
- `&>` (shared index-read) requires a SHARED ref; reading through `&!array` uses `&!>` + `@`.
- Computed array indices need explicit `>usize` (bare literals coerce).
- `times` is a FOLD: it returns the accumulated value; `inplace_fold.sth` ends with `drop` after `times`.
- A `&!` borrow captured by a loop-body quotation is consumed by the capture: any use of the borrow AFTER `] times` fails with `body leaves 1 values` (reproduce: t_capture.sth). Reorder so post-loop uses happen before the capture.
- `render` wanting `&!SBuf` rejects a `&SBuf` (`render` expected `&!SBuf`, found `&SBuf`) — modes are strict at call sites.
