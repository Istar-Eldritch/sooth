# Phase 3 Slice 8a — Typed foreign calls + string slices (spec)

Base: `main` @ `a957489`, 833 tests green. Design input: [the brief](./phase3-slice8a-brief.md),
whose D1-D6 are settled and not relitigated here. Slice 8b (resources, user destructor bodies)
is a separate spec and out of scope.

## What the slice adds

One typed foreign-call declaration (`extern:`), and two string types (`str`, `cstr`) with
literals. Nothing else. No resource types, no destructor bodies, no buffer slicing, no allocator
involvement, no new runtime symbol.

## Requirements (final decisions only)

**R1 — The `extern:` declaration.** Grammar, at top level only, alongside `type:` and `:`:

```
extern: <word-name> ( <effect> ) "<c-symbol>" ;
```

It registers `<word-name>` in the ordinary word environment with the declared effect, so every
existing arity/type check applies to its call sites unchanged. The symbol is an explicit string
literal, not the word name reused: Sooth word names admit characters C does not (`&!S>fi`), and
binding `openat` as `open` must be possible. Redeclaring a name that already exists (builtin,
user word, or another `extern:`) is a located error.

**R2 — What may cross the boundary.** Exactly: the numeric tower (`i64`/`u8`/`usize`/`isize`/
`f64`/`f32`/`bool`), `&T` and `&!T`, `str`, and `cstr`. These are the types whose machine
representation is either a scalar or an opaque `Ptr` the backend already passes.

**R3 — What may not, each with its own rejection at the declaration, not at the call.** An owned
aggregate (struct/enum/array/`^T`) in any position: ownership across the boundary has no answer
and no client. A `^T` in output position specifically: it would forge ownership of memory the
allocator did not hand out. A reference in output position: already forbidden generally
(`src/check.rs:1694-1718`), and the existing message is reused rather than duplicated. Variadic
C functions: unrepresentable in a fixed effect, so out of scope, and an `extern:` cannot express
them by construction (no syntax for it) — no new check needed, but a test pins that `printf`
cannot be usefully declared.

**R4 — `str` is pointer + length with a sentinel invariant.** Two machine words: an opaque `Ptr`
and a `usize`. The invariant is `byte[len] == 0`, i.e. Zig's `[:0]const u8`: the length is
authoritative for all Sooth-side use and is never discovered by scanning, while the guaranteed
terminator one past the end lets the pointer alone go to C at zero cost. `str` is `Copy`.

**R5 — `cstr` is pointer-only, length unknown.** One opaque `Ptr`, NUL-terminated, Zig's
`[*:0]const u8`. It exists for the C boundary in both directions: what a `char*` parameter wants,
and what a `char*` return hands back. `cstr` is `Copy`.

**R6 — String literals.** A new `Token::Str(String)` in the lexer, delimited by `"`, with escapes
`\n`, `\t`, `\\`, `\"`, `\0`. An unterminated literal and an unknown escape are both located lex
errors. A literal has type `str`, with its length known at compile time. The backend emits it as
static data with a trailing NUL byte that the length **does not count**, which is what makes R4's
invariant free for every literal (the backend already emits exactly this shape for `$sfmt`,
`$oobfmt`, `$boolstrs`).

**R7 — `str` -> `cstr` is an explicit word, and there is no implicit conversion.** `cstr
( str -- cstr )` discards the length; it is sound for any `str` because R4 guarantees the
sentinel and R11 keeps every `str` static-rooted. Passing a `str` where a declaration wants a
`cstr` is a located type error naming the conversion, never a silent pointer pun. The reverse
direction (`cstr -> str`) requires a scan and is **not** in this slice: no client needs it until
a foreign call returns a `char*`, and adding it now would mean adding a `strlen` dependency to
justify no caller.

**R8 — `len` extends to `str`.** `len ( str -- usize )` reads the second word. It must emit no
call: the whole point of R4 is that length is carried, and a golden asserts the absence of any
`Instr::Call` on this path.

**R9 — `.` prints both, differently.** `str` prints via `printf("%.*s", len, ptr)`, passing the
length, because `%s` would rely on the sentinel and make the printed result depend on an
invariant rather than on the value's own length. `cstr` prints via `%s`, since a length is all it
lacks. Two new static format strings alongside the existing ones.

**R10 — `str` and `cstr` are `Copy`, are not `Type::Ref`, and are not caught by
`contains_reference`.** Stated explicitly because it is load-bearing and would otherwise be
implied: neither participates in the escape rules or the aliasing rule, and both may be stored in
a struct field, returned from a word, and duplicated freely. This is sound **only** under R11.

**R11 — A `str` may point at static data only.** There is no way in this slice to construct a
`str` other than a literal (or `cstr` of one, R7). Consequently no `str` can dangle, which is
what makes R10 safe. Slicing a heap `^[u8 N]` or a local buffer into a `str` is deferred (see
DESIGN.md Open / deferred): it would be a borrow not spelled `&`, bypassing the escape rules,
and spelling it as a returned reference is precisely what R3's output-side rule forbids — that
rule being what stands in for lifetimes. R4's sentinel independently forbids the general case,
since an arbitrary substring's end is not a NUL.

**R12 — The `extern:` declaration site is the trust boundary; there is no `unsafe` marker.** C
may retain a pointer it was passed and Sooth cannot prevent it. One keyword granting that at a
reviewable declaration is the whole mechanism, which is what every FFI does (Rust spells the
same trust `unsafe`).

**R13 — Failure handling is library-level.** A foreign call's return value is an ordinary value
the caller checks in Sooth. Nothing traps, no `dprintf`/`exit` path, no new runtime symbol. This
supersedes an earlier draft decision that syscall failures would trap like the OOB check.

**R14 — A missing symbol is a `cc` linker error, not a Sooth diagnostic.** Accepted limitation:
checking would need a symbol table the compiler has no access to. Documented in the spec and in
the `extern:` error-message note, not worked around.

## Open questions from the brief, answered

**`str`'s representation: a built-in two-field aggregate, not a new `IrType`.** It reuses struct
layout, the existing `is_copy` fold, and field access lowering, and it keeps the `Ptr` component
opaque without any new backend case. A new `IrType` variant would touch every `match` over
`IrType` in `ir.rs` and `qbe.rs` for no gain. The one thing to verify during implementation is
that `contains_reference` (`src/check.rs:219-231`) does **not** see through it (it must not — the
`Ptr` component is not a `Type::Ref`), and that `check_no_stored_references` therefore leaves it
alone, which is R10's requirement.

**`.` on a `str` uses `%.*s`, not `%s`** (R9). Reasoning in R9: printing must depend on the
value's length, not on an invariant, or the two diverge the moment R11 is ever relaxed.

**`str`/`cstr` escape and `Copy` status: stated, not implied** (R10), and explicitly conditional
on R11.

**The dogfood stops short of file I/O.** It declares `strlen` and `puts` against a literal.
Opening a file needs the fd to be a bare `i64` with a hand-written check, which is exactly the
shape 8b replaces with a resource type — writing it here would mean writing it twice and
deleting one. See Dogfood below.

**A missing symbol stays a linker error** (R14), accepted rather than mitigated.

## Criterion → test map

| # | Criterion | Test |
|---|---|---|
| 1 | a `str` literal lexes, with each escape | `lex_string_literal_handles_every_escape` |
| 2 | an unterminated literal is a located lex error | `lex_unterminated_string_literal_is_error` |
| 3 | an unknown escape is a located lex error | `lex_unknown_string_escape_is_error` |
| 4 | `extern:` parses and registers its effect | `parse_extern_declaration_registers_its_effect` |
| 5 | an `extern:` redeclaring an existing word is an error | `check_extern_redeclaring_a_word_is_error` |
| 6 | `len` on a `str` is the carried length, no call emitted | `len_of_a_str_emits_no_call_native` |
| 7 | the literal's terminator is uncounted, so Sooth's length and C's `strlen` agree | `str_length_and_foreign_strlen_agree_native` |
| 8 | `.` on a `str` prints via `%.*s` | `emit_print_of_str_uses_precision_format` |
| 9 | `.` on a `cstr` prints via `%s` | `emit_print_of_cstr_uses_string_format` |
| 10 | a `str` passed where `cstr` is declared is a type error naming the conversion | `check_str_where_cstr_declared_is_error` |
| 11 | an `extern:` declaring an owned aggregate is rejected at the declaration | `check_extern_with_aggregate_parameter_is_error` |
| 12 | an `extern:` returning `^T` is rejected at the declaration | `check_extern_returning_owned_pointer_is_error` |
| 13 | an `extern:` returning a reference reuses the existing rejection | `check_extern_returning_a_reference_is_error` |
| 14 | a reference crosses as an input and the callee's write is observable | `foreign_call_writes_through_a_mutable_reference_native` |
| 15 | `str`/`cstr` are `Copy`: `dup` accepted, storable in a field | `str_and_cstr_are_copy_and_storable` |
| 16 | the dogfood runs with the documented output | `slice8a_dogfood_compiles_and_runs` |

Every row is a runnable golden; rows 2, 3, 5, 10, 11, 12, 13 assert the *specific* message, not
merely that compilation failed.

## Stage unit-test obligations

- **lexer**: string tokenisation, each escape, unterminated, unknown escape, a `"` inside a word
  position.
- **parser**: `extern:` happy path; missing symbol string; malformed effect; `extern:` nested
  inside a word body rejected.
- **check**: R2's accepted type set; each R3 rejection; R7's missing-conversion error; `len` and
  `.` typing for both string types; `is_copy` for both.
- **ir**: literal lowering to a static-data reference; `len` lowering to a field read with no
  call; the foreign call lowering to `Instr::Call` with the declared symbol.
- **backend**: static data emission with the uncounted terminator; `%.*s` vs `%s` selection; the
  emitted `call $<symbol>` argument classes for a `Ptr` and a scalar.

## Dogfood

`examples/strings.sth`:

```
extern: strlen ( cstr -- usize ) "strlen" ;
extern: puts   ( cstr -- i64 )   "puts" ;

: main ( -- )
  "hello, sooth" | s |
  s len .
  s cstr strlen .
  s cstr puts drop ;
```

Expected output `12`, `12`, `hello, sooth`. The two `12`s agreeing is criterion 7: the first is
Sooth's carried length, the second is C scanning to the terminator the backend emitted without
counting it.

## Out of scope

Slicing a buffer into a `str` (DESIGN.md Open / deferred). `cstr -> str` (R7). Growable `String`,
concatenation, formatting, and anything allocator-touching (Phase 6's `alloc` layer). Variadic
foreign calls (R3). Resource types and user destructor bodies (Slice 8b). A symbol-existence
check (R14).

## Carried into phase 3

After phase 2, `module.externs` is read only by the parser, `ast`, and `check`; nothing lowers it.
So a program the checker *accepts* still panics in lowering: `strlen` declared and called reaches
`src/ir.rs`'s `self.env.get(name).expect("checked user word exists")`, because an `extern:` never
enters the lowering env. This is phase 3's wiring, and phase 3's goldens (criteria 7, 14, 16)
force it. It was deliberately not patched in phase 2 with a stopgap guard: the fix is to lower a
foreign call, not to reject one later. Note for whoever picks it up that the QBE `call $<symbol>`
site can trust the symbol's shape, since the parser validates it at the declaration (R12).

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "String literals and the str/cstr types: Token::Str with escapes in the lexer, both types through check with their Copy status, literal lowering to static data whose trailing NUL is not counted in the length, len on str emitting no call, and . printing via %.*s and %s respectively. Covers R4, R5, R6, R8, R9, R10, R11 and criteria 1, 2, 3, 6, 8, 9, 15.",
      "difficulty": "normal"
    },
    {
      "phase": 2,
      "focus": "The extern: declaration: top-level grammar with an explicit C symbol string, registration into the word environment so existing arity and type checks apply unchanged, the accepted boundary type set, and a distinct located rejection for each forbidden position (owned aggregate anywhere, ^T or a reference in output position), reusing the existing no-declared-output-reference message rather than duplicating it. Covers R1, R2, R3, R7, R12, R13, R14 and criteria 4, 5, 10, 11, 12, 13.",
      "difficulty": "normal"
    },
    {
      "phase": 3,
      "focus": "Wire the two halves together and prove them empirically: examples/strings.sth as the dogfood, the str-length-agrees-with-foreign-strlen golden that pins the uncounted terminator, and the mutable-reference-crossing golden showing a callee's write observable in Sooth. Covers criteria 7, 14, 16.",
      "difficulty": "normal"
    }
  ]
}
```
