# Phase 3 Slice 8a — Typed foreign calls + string slices (brief)

ROADMAP's Slice 8 was one entry ("resources as linear values (fds, hosted) + user-definable
destructor bodies") and is now two. This is 8a: the ability to call a C function with a declared
stack effect, and enough of a string type to name a file path. 8b is the destructor mechanism
(see `phase3-slice8b-brief.md`), which wants `close` to already exist so it has a second real
client to be designed against besides slice 2's `free`.

Slice 7's opt-in RC is deferred to Phase 6.

Prerequisite state: Phase 3 Slices 1-6 merged, plus the `get`/`set` retirement and the R21
third-aliasing-route fixes. 833 tests green at `eaea0cd`.

## Recon: what already exists (measured, not assumed)

**1. There are no strings at all.** `Token` (`src/lexer.rs:9-19`) is
`Semicolon`/`LParen`/`RParen`/`Pipe`/`LBracket`/`RBracket`/`Int`/`Float`/`Word` — no string
token, no escape handling, no `Str` type. `: main ( -- ) "hi" drop ;` fails with
``unknown word `"hi"` ``, because `"hi"` lexes as an ordinary `Word` and dies at name
resolution. **Consequence: Phase 3's stated exit criterion ("open/read/close files") was
unreachable as written**, since `open(2)` needs a path. Nobody caught this when the criterion
was set.

**2. Calling a C function by name is already the established mechanism, used six times, with no
FFI feature as such.** The backend emits `call $malloc` / `call $free`
(`src/backend/qbe.rs:634,652`), `call $printf` for `.` (`:899-936`), `call $dprintf(w 2, ...)` +
`call $exit(w 1)` for the OOB trap (`:607-608`), and `call $getenv` for trace gating (`:674`).
QBE IL just names the symbol; the driver links with plain `cc` (`src/driver.rs:9`, a shim of
`extern void sooth_main(void); int main(void) { sooth_main(); return 0; }`), so libc symbols
resolve for free. `extern:` is therefore **user-facing access to a path that already works**,
not new machinery.

**3. The backend already emits static NUL-terminated byte data and passes pointers to it.**
`data $sfmt = { b "%s", b 0 }` (`src/backend/qbe.rs:45`), plus `$oobfmt`, `$boolstrs`,
`$allocfmt`, `$freefmt`. So a string *literal* is likewise mostly exposing existing capability:
static data plus a pointer. Note `$sfmt` is currently only used to print the `bool` name table,
not user data.

**4. A reference is legal as an input but forbidden as a declared output.**
`check_effect_signature`-side rule (`src/check.rs:1694-1718`): ``a reference cannot be stored:
`{w}` declares the output `{ty}` `` … *"take the reference as an input instead"*, and the
input-side variant allows a bare `&T`/`&!T` but not one nested inside an aggregate. This is what
lets 8a pass `&![u8 N]` into `read` with no new rule, and it is also why a buffer-slicing word
returning a borrow is unspellable (see D3).

**5. `Ptr[T]` opacity is a load-bearing invariant, not a preference.** CLAUDE.md: "IR stays
backend-neutral: `Ptr[T]` is an opaque handle, never assumed to be a `u64` (a future WASM
lowering depends on this)." This is what rules out the generic-syscall design (D1).

## Decided (locked, one at a time)

**D1. One typed `extern:` declaration form, not per-call builtins and not a generic syscall
word.** Shape: a Sooth word name, a stack effect, and the C symbol as an explicit string:

```
extern: close ( i64 -- i64 ) "close" ;
```

Rejected alternative A, **`open`/`read`/`close` as compiler builtins**: every future hosted
call would be a compiler change, and they would all be deleted again when Phase 6's hosted
layer lands. Rejected alternative B, **an untyped generic `syscall ( n a b c -- r )` word**:
it is the smaller compiler surface, but it forces a buffer pointer to be passed as an integer,
which breaks recon 5's invariant; syscall numbers are per-arch *and* per-OS (`open` is 2 on
x86_64 Linux, absent on aarch64 which has only `openat`, and WASM has no syscalls at all,
against a roadmap committing to arm64/riscv64/rv32/WASM); and QBE has no inline asm, so a
"raw" syscall would route through libc's own `syscall()` wrapper anyway, buying nothing over
naming `close` while losing portability and type checking. The symbol is an explicit string
rather than reusing the word name because Sooth word names admit characters C does not, and
because binding `openat` as `open` is worth allowing.

**D2. Two string types, following Zig.** `str` is pointer + length **and** guarantees
`byte[len] == 0` (Zig's `[:0]const u8`): Sooth code always reads the length and never scans,
while C receives just the pointer at zero cost. `cstr` is pointer-only with unknown length
(Zig's `[*:0]const u8`), which is what C hands *back*. `str` -> `cstr` drops the length for
free; `cstr` -> `str` costs an explicit scan and is a word, never implicit. A literal satisfies
`str`'s invariant natively, since the backend controls its static bytes and can emit the
terminator without counting it. NUL-termination alone (C's model) was rejected: length would be
the only quantity in the language discovered by scanning rather than carried (`[u8 N]` has its
length in the type, indexing is bounds-checked, `usize` arrived with arrays), its failure mode
is an unbounded read, and it cannot express a substring without writing a NUL into the parent.

**D3. A `str` points at static data only in this slice.** A literal-rooted `str` cannot dangle,
so it needs no restriction at all. Slicing a heap `^[u8 N]` or a local buffer would make the
`str` a borrow that bypasses every escape rule, because it is not spelled `&`; spelling it
`( ^[u8 N] -- &str )` is rejected outright by recon 4, and that rule is precisely what stands
in for lifetimes. Restricting a buffer-derived `str` the way `&T` is restricted forbids
returning it, which unspells the slicing word; making the restriction provenance-dependent
means `( str -- )` no longer says which kind it holds. Deferred to a real client, likely Phase
9's lexer. Recorded in DESIGN.md's Open / deferred. Note D2 independently forbids the general
case anyway: an arbitrary substring's end is not a NUL.

**D4. Scalars, references, `str` and `cstr` may cross the boundary; owned aggregates and `^`
returns may not.** Passing an owned aggregate by value raises "who frees it now" with no good
answer and no client. A foreign call returning `^T` would forge ownership of memory Sooth's
allocator did not hand out; `^` allocation already exists in-language for that. A reference
crossing as an *input* needs no new rule (recon 4).

**D5. The `extern:` declaration site is itself the trust boundary; there is no `unsafe`
marker.** C can stash a pointer it was passed, and Sooth cannot prevent it — this is the same
trust every language's FFI takes (Rust spells it `unsafe`). One keyword granting it at the
declaration, where it is reviewable in one place, is enough for a language this size.

**D6. Failure handling is library-level, not a compiler-inserted trap.** An earlier draft had
syscall failures trap via `dprintf`+`exit(1)`, mirroring the OOB trap. `extern:` obsoletes
that: `open` returns a raw `i64` and the caller checks it in ordinary Sooth. Nothing traps, no
new runtime symbol. (Recorded because this decision was silently invalidated by D1 and should
not evaporate unnoticed.)

## Open questions the spec should answer

- **`str`'s representation.** A built-in two-field aggregate (opaque `Ptr` + `usize`, reusing
  struct layout and the existing `is_copy` fold) or a new `IrType` variant. The aggregate route
  looks strictly cheaper; confirm it does not collide with `contains_reference`,
  `check_no_stored_references`, or the `Ptr`-opacity invariant.
- **Whether `.` prints a `str`.** It cannot reuse `$sfmt`/`%s` naively, since a `str` is not
  guaranteed NUL-terminated *at the pointer C would scan from* if it is ever a substring —
  though under D3 it always is. `%.*s` with the length is the honest form. Decide, and note
  that `cstr` printing is the `%s` case.
- **Escape/`Copy` status of `str` and `cstr` written down explicitly.** Both are `Copy` and
  neither is a `Type::Ref`, so neither is caught by `contains_reference`; under D3 that is
  sound, but the spec should state it rather than leave it implied.
- **The dogfood's exact shape** (see below): whether 8a stops at `strlen`/`puts` on a literal,
  or already opens a file and leaves only the resource typing to 8b.
- **A missing symbol is a `cc` linker error, not a Sooth diagnostic.** Recommend accepting
  this and noting it; a symbol table to check against is out of scope.

## Dogfood

Both halves in one program, with no resource typing yet (that is 8b's job), so the fd stays a
bare `i64`:

```
extern: strlen ( cstr -- usize ) "strlen" ;
extern: puts   ( cstr -- i64 )   "puts" ;

: main ( -- )
  "hello, sooth" | s |
  s len .                  \ 12, from str's own length, no scan
  s cstr strlen .          \ 12 again, this time C counted it
  s cstr puts drop ;
```

Exit criteria should include: `str`'s length is known without a call (no `strlen` in the emitted
body of the `s len` path); the literal's static data carries a terminator that the length does
not count, demonstrated by the two `12`s agreeing; a `str` used where the declaration wants a
`cstr` without the conversion is a compile error rather than a silent pointer pun; and an
`extern:` declaring an owned aggregate or a `^` return is rejected at the declaration (D4).
