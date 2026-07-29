# Phase 3 Slice 8a — Typed foreign calls + string slices (implemented)

Base `main` @ `a957489`. Adds one typed foreign-call declaration (`extern:`) and two string types (`str`, `cstr`) with literals. No resource types, no destructor bodies, no buffer slicing, no allocator involvement, no new runtime symbol. Slice 8b is separate.

## Decisions

**R1 — `extern:` declaration.** Top level only, alongside `type:` and `:`:

```
extern: <word-name> ( <effect> ) "<c-symbol>" ;
```

Registers `<word-name>` in the ordinary word environment with the declared effect, so existing arity/type checks apply to call sites unchanged. The C symbol is an explicit string, not the word name reused (Sooth names admit characters C does not; binding `openat` as `open` must be possible). The parser validates the symbol is a legal C identifier and rejects an empty or illegal one, so the QBE `call $<symbol>` site can trust its shape. Redeclaring an existing name (builtin, user word, or another `extern:`) is a located error.

**R2 — What may cross.** The numeric tower (`i64`/`u8`/`usize`/`isize`/`f64`/`f32`/`bool`), `&T` and `&!T`, and `cstr`: types whose machine representation is a scalar or a single opaque `Ptr`. *Amended during implementation:* an earlier draft also listed `str`, which contradicts the list's own criterion — R4 makes a `str` two machine words, matching no C prototype without an invented ABI. Nothing is lost, since R7's `cstr` conversion is total.

**R3 — What may not, rejected at the declaration, not the call.** Owned aggregates (struct/enum/array/`^T`) in any position: ownership across the boundary has no answer and no client. `str` in any position: input names the `cstr` conversion in its message; output because C supplies no length and R11 forbids a non-literal `str`. `^T` in output: would forge ownership of memory the allocator never handed out. A reference in output: reuses the existing no-declared-output-reference message rather than duplicating it. Variadics are unrepresentable by construction (no syntax), so no check, only a test pinning that `printf` cannot be usefully declared.

**R4 — `str` is pointer + length, and the length is the only thing it promises.** One word: an opaque pointer to a static two-word `{bytes_ptr, len}` descriptor the backend writes (`emit_str_literal`); a dedicated `IrType::Str`, not struct-layout reuse (`IrType::Cstr` mirrors it as a bare pointer). `Copy`; length is authoritative Sooth-side, never scanned. `len`/`cstr` read the descriptor through dedicated `StrLen`/`StrPtr` IR instructions, not a hand-written byte offset, so the descriptor's layout stays a backend-only concern. *Narrowed after implementation:* the type-wide-sentinel story this rule used to argue now lives in R11. Rejected at an `extern:` boundary (R3) because a `str` is a descriptor handle matching no C prototype, not merely a width mismatch.

**R5 — `cstr` is pointer-only.** One opaque `Ptr`, NUL-terminated, Zig's `[*:0]const u8`. Serves the C boundary in both directions. `Copy`.

**R6 — String literals.** `Token::Str(String)` delimited by `"`, escapes `\n \t \\ \" \0`; unterminated literals and unknown escapes are located lex errors. A literal has type `str` with compile-time length; the backend emits static data with a trailing NUL the length does **not** count (same shape as `$sfmt`/`$oobfmt`/`$boolstrs`). This lowering, not R4, is what puts a terminator behind every `str` that exists in this slice.

**R7 — `str -> cstr` is explicit, never implicit.** `cstr ( str -- cstr )` discards the length. Its soundness rests on **R11 alone**: a literal is the only constructor of a `str`, so the pointer is static and the byte at `len` is the NUL R6's lowering emitted. It does not rest on a type-wide invariant: lifting R11 breaks this one word's precondition, in one place. The borrowed-buffer form is then a *different* word whose precondition is that the caller already wrote a terminator into storage it owns, `core` having no allocator to copy with. Passing a `str` where `cstr` is declared is a located type error naming the conversion. `cstr -> str` needs a scan and is out of scope: no client until a foreign call returns `char*`.

**R8 — `len` extends to `str`.** Reads the second word; emits no call, and a golden asserts no `Instr::Call` on that path.

**R9 — `.` prints both, differently.** `str` via `printf("%.*s", len, ptr)`: under R4 there is no sentinel to read against, so the carried length is what bounds the print. `cstr` via `%s`, a length being all it lacks — and so it, like C's `strlen`, stops at the first NUL: a literal with an embedded `\0` diverges between Sooth's counted `len` and either's terminator-bound read, which is reachable (`\0` is a supported escape) and pinned by a golden rather than left a surprise. Two new static format strings. *Amended after review:* neither format appends a newline, unlike every other type's `$fmt` (`"%ld\n"`) — deliberate, not an oversight. For a number `.`'s newline is a convenience: no other useful printed form exists, so the compiler supplying one costs nothing. A string's printed form is exactly what the programmer wrote; appending a byte the source didn't contain would be the compiler emitting output nobody asked for, the same "nothing happens behind your back" rule `dup`/`drop`'s explicitness rests on. A trailing newline is spelled with the `\n` escape, which R6 resolves at lex time into a real byte in the literal's static data — no runtime formatting involved.

**R10 — Both are `Copy`, neither is `Type::Ref` nor caught by `contains_reference`.** They sit outside the escape and aliasing rules, may be stored in a struct field, returned, and duplicated. `check_no_stored_references` leaves them alone. Sound under R11's static-rooting for `str` always, and for `cstr` everywhere except a declared `extern:` boundary, where R12's trust governs instead: a `cstr` returned by a foreign call may point at whatever C chose, not static data. `cstr` is the easier one to forget: a bare pointer with no length, so one derived from a local buffer (not behind a declared boundary) would be an escapable, `dup`-able value aimed at a dying frame, laundering a borrow past the escape rules. Whatever distinguishes a static/trusted view from a borrowed one later must apply to `cstr` as much as to `str`.

**R11 — `str` is static-rooted; `cstr` is static-rooted except across a declared foreign boundary.** A `str`'s only constructor is a literal, so it can never dangle. A `cstr`'s constructors are a literal's `cstr` conversion (also static) and a foreign call's return value, where R12's declared-boundary trust governs instead of static-rooting: `extern: getenv ( cstr -- cstr ) "getenv" ;` compiles, runs, and lets a `cstr` holding whatever pointer C chose escape a word. Slicing a heap `^[u8 N]` or local buffer into either type is deferred (DESIGN.md Open/deferred): it is a borrow not spelled `&`, and returning it as a reference is exactly what R3 forbids, that rule standing in for lifetimes. *Narrowed after implementation:* an earlier draft made `byte[len] == 0` a type-wide `str` invariant; R2's own amendment (nothing crosses `str` raw, only R7's `cstr` conversion does) undercut that justification, and a type-wide sentinel cannot survive a `str` viewing part of a buffer, since an arbitrary substring's end is not a NUL. The terminator is now a property of R6's literal lowering and a precondition of R7 alone: revocable at that one site rather than an invariant the whole type was documented to carry. A string type's reference-carrying answer is decided by the `_ =>` fall-through arm of `contains_reference` (src/check.rs:247) — and, for `Copy`, of `is_copy` (src/check.rs:202) — so a borrowed view later must give both predicates an explicit arm rather than being hard-coded anywhere else.

**R12 — The declaration site is the trust boundary; no `unsafe` marker.** C may retain a passed pointer and Sooth cannot prevent it; one reviewable keyword grants that trust.

**R13 — Failure handling is library-level.** A foreign return value is an ordinary value the caller checks. Nothing traps, no new runtime symbol.

**R14 — A missing symbol is a `cc` linker error**, accepted rather than mitigated: checking needs a symbol table the compiler has no access to. *Accepted limitation found in review:* the driver passes no `-l` flags beyond libc's defaults, so `extern: sqrt ( f64 -- f64 ) "sqrt" ;` fails the same way, at link time, for a symbol libm rather than libc owns. Consistent with R14's own reasoning, just a wider blast radius than "missing" implied. Phase 6's hosted layer, which owns library linking generally, is where this gets a real answer; not a slice 8a fix.

## Criterion → test map

| # | Criterion | Test |
|---|---|---|
| 1 | a `str` literal lexes, with each escape | `lex_string_literal_handles_every_escape` |
| 2 | unterminated literal is a located lex error | `lex_unterminated_string_literal_is_error` |
| 3 | unknown escape is a located lex error | `lex_unknown_string_escape_is_error` |
| 4 | `extern:` parses and registers its effect | `parse_extern_declaration_records_its_effect` + `check_extern_registers_its_effect_at_call_sites` |
| 5 | redeclaring an existing word is an error | `check_extern_redeclaring_a_word_is_error`, `check_extern_redeclaring_a_builtin_is_error` |
| 6 | `len` on `str` is carried, no call emitted | `len_of_a_str_emits_no_call_native` |
| 7 | uncounted terminator: Sooth length and C `strlen` agree on NUL-free content | `str_length_and_foreign_strlen_agree_native` |
| 8 | `.` on `str` uses `%.*s` | `emit_print_of_str_uses_precision_format` |
| 9 | `.` on `cstr` uses `%s` | `emit_print_of_cstr_uses_string_format` |
| 10 | `str` where `cstr` is declared names the conversion | `check_str_where_cstr_declared_is_error` |
| 11 | owned aggregate in an `extern:` rejected at the declaration | `check_extern_with_aggregate_parameter_is_error` |
| 12 | `extern:` returning `^T` rejected | `check_extern_returning_owned_pointer_is_error` |
| 13 | `extern:` returning a reference reuses the existing rejection | `check_extern_returning_a_reference_is_error` |
| 14 | a `&!T` crosses as input and the callee's write is observable | `foreign_call_writes_through_a_mutable_reference_native` |
| 15 | both types are `Copy` and storable in a field | `str_and_cstr_are_copy_and_storable` |
| 16 | dogfood runs with the documented output | `slice8a_dogfood_compiles_and_runs` |

Rows 2, 3, 5, 10, 11, 12, 13 assert the specific message. Stage units cover: lexer escapes/errors; parser happy path, missing symbol string, invalid symbol, malformed effect, `extern:` nested in a body; check's accepted set, each rejection, `len`/`.` typing, `is_copy`; ir literal-to-static-data, `len` lowering to `StrLen` with no `Instr::Call`, `cstr` lowering to `StrPtr`, foreign call to `Instr::Call` with the declared symbol; backend static data with uncounted terminator, format selection, argument classes. The interior-NUL divergence R9 documents is additionally pinned, past a numbered criterion, by `interior_nul_diverges_sooth_len_from_c_strlen_native`.

## Dogfood

`examples/strings.sth` declares `strlen` and `puts` against a literal and prints `12`, `12`, `hello, sooth`. The two `12`s agreeing is criterion 7: Sooth's carried length versus C scanning to a terminator the backend emitted without counting. It stops short of file I/O deliberately — an fd would be a bare `i64` with a hand-written check, the shape 8b replaces with a resource type.

## Delivery

1. `c0b6a35` — literals and both string types through lexer, check, ir, backend (R4-R6, R8-R11; criteria 1, 2, 3, 6, 8, 9, 15).
2. `44920f7`, `10305dc`, `5630943` — `extern:` grammar, registration, boundary-type rejections, C-symbol validation, builtin redeclaration (R1-R3, R7, R12-R14; criteria 4, 5, 10, 11, 12, 13).
3. `8e5a583` — `extern:` wired into the lowering env via a name→symbol map, dogfood and native goldens (criteria 7, 14, 16).

## Out of scope

`str` at an `extern:` boundary (R2 amendment; admitting it later means pinning an ABI, presumably `ptr` + `len` as two C arguments, and an output would additionally need R11 relaxed). Slicing a buffer into a `str`. `cstr -> str` (R7). Growable `String`, concatenation, formatting, anything allocator-touching (Phase 6). Variadic foreign calls. Resource types and user destructor bodies (Slice 8b). A symbol-existence check (R14). *Known gap found in review:* `extern:` has no REPL eval path (`type:` does); the parser's `unexpected token LParen` doesn't say so. The spec is silent on whether this is intended; left undecided rather than implemented here.
