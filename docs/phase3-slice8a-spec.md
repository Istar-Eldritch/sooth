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

**R4 — `str` is pointer + length with a sentinel.** Two machine words (opaque `Ptr` + `usize`), invariant `byte[len] == 0`, i.e. Zig's `[:0]const u8`. Length is authoritative Sooth-side and never scanned; the guaranteed terminator lets the pointer alone go to C at zero cost. `Copy`. Represented as a built-in two-field aggregate reusing struct layout, the `is_copy` fold, and field-access lowering, not a new `IrType`.

**R5 — `cstr` is pointer-only.** One opaque `Ptr`, NUL-terminated, Zig's `[*:0]const u8`. Serves the C boundary in both directions. `Copy`.

**R6 — String literals.** `Token::Str(String)` delimited by `"`, escapes `\n \t \\ \" \0`; unterminated literals and unknown escapes are located lex errors. A literal has type `str` with compile-time length; the backend emits static data with a trailing NUL the length does **not** count (same shape as `$sfmt`/`$oobfmt`/`$boolstrs`), which makes R4's invariant free.

**R7 — `str -> cstr` is explicit, never implicit.** `cstr ( str -- cstr )` discards the length; sound for any `str` under R4 + R11. Passing a `str` where `cstr` is declared is a located type error naming the conversion. `cstr -> str` needs a scan and is out of scope: no client until a foreign call returns `char*`.

**R8 — `len` extends to `str`.** Reads the second word; emits no call, and a golden asserts no `Instr::Call` on that path.

**R9 — `.` prints both, differently.** `str` via `printf("%.*s", len, ptr)` so printing depends on the value's length rather than the invariant; `cstr` via `%s`. Two new static format strings.

**R10 — Both are `Copy`, neither is `Type::Ref` nor caught by `contains_reference`.** They sit outside the escape and aliasing rules, may be stored in a struct field, returned, and duplicated. `check_no_stored_references` leaves them alone. Sound only under R11.

**R11 — A `str` may point at static data only.** The only constructor is a literal (or `cstr` of one), so no `str` can dangle. Slicing a heap `^[u8 N]` or local buffer is deferred: it is a borrow not spelled `&`, and spelling it as a returned reference is exactly what R3 forbids. R4's sentinel independently forbids the general case.

**R12 — The declaration site is the trust boundary; no `unsafe` marker.** C may retain a passed pointer and Sooth cannot prevent it; one reviewable keyword grants that trust.

**R13 — Failure handling is library-level.** A foreign return value is an ordinary value the caller checks. Nothing traps, no new runtime symbol.

**R14 — A missing symbol is a `cc` linker error**, accepted rather than mitigated: checking needs a symbol table the compiler has no access to.

## Criterion → test map

| # | Criterion | Test |
|---|---|---|
| 1 | a `str` literal lexes, with each escape | `lex_string_literal_handles_every_escape` |
| 2 | unterminated literal is a located lex error | `lex_unterminated_string_literal_is_error` |
| 3 | unknown escape is a located lex error | `lex_unknown_string_escape_is_error` |
| 4 | `extern:` parses and registers its effect | `parse_extern_declaration_records_its_effect` + `check_extern_registers_its_effect_at_call_sites` |
| 5 | redeclaring an existing word is an error | `check_extern_redeclaring_a_word_is_error`, `check_extern_redeclaring_a_builtin_is_error` |
| 6 | `len` on `str` is carried, no call emitted | `len_of_a_str_emits_no_call_native` |
| 7 | uncounted terminator: Sooth length and C `strlen` agree | `str_length_and_foreign_strlen_agree_native` |
| 8 | `.` on `str` uses `%.*s` | `emit_print_of_str_uses_precision_format` |
| 9 | `.` on `cstr` uses `%s` | `emit_print_of_cstr_uses_string_format` |
| 10 | `str` where `cstr` is declared names the conversion | `check_str_where_cstr_declared_is_error` |
| 11 | owned aggregate in an `extern:` rejected at the declaration | `check_extern_with_aggregate_parameter_is_error` |
| 12 | `extern:` returning `^T` rejected | `check_extern_returning_owned_pointer_is_error` |
| 13 | `extern:` returning a reference reuses the existing rejection | `check_extern_returning_a_reference_is_error` |
| 14 | a `&!T` crosses as input and the callee's write is observable | `foreign_call_writes_through_a_mutable_reference_native` |
| 15 | both types are `Copy` and storable in a field | `str_and_cstr_are_copy_and_storable` |
| 16 | dogfood runs with the documented output | `slice8a_dogfood_compiles_and_runs` |

Rows 2, 3, 5, 10, 11, 12, 13 assert the specific message. Stage units cover: lexer escapes/errors; parser happy path, missing symbol string, invalid symbol, malformed effect, `extern:` nested in a body; check's accepted set, each rejection, `len`/`.` typing, `is_copy`; ir literal-to-static-data, `len` as a field read, foreign call to `Instr::Call` with the declared symbol; backend static data with uncounted terminator, format selection, argument classes.

## Dogfood

`examples/strings.sth` declares `strlen` and `puts` against a literal and prints `12`, `12`, `hello, sooth`. The two `12`s agreeing is criterion 7: Sooth's carried length versus C scanning to a terminator the backend emitted without counting. It stops short of file I/O deliberately — an fd would be a bare `i64` with a hand-written check, the shape 8b replaces with a resource type.

## Delivery

1. `c0b6a35` — literals and both string types through lexer, check, ir, backend (R4-R6, R8-R11; criteria 1, 2, 3, 6, 8, 9, 15).
2. `44920f7`, `10305dc`, `5630943` — `extern:` grammar, registration, boundary-type rejections, C-symbol validation, builtin redeclaration (R1-R3, R7, R12-R14; criteria 4, 5, 10, 11, 12, 13).
3. `8e5a583` — `extern:` wired into the lowering env via a name→symbol map, dogfood and native goldens (criteria 7, 14, 16).

## Out of scope

`str` at an `extern:` boundary (R2 amendment; admitting it later means pinning an ABI, presumably `ptr` + `len` as two C arguments, and an output would additionally need R11 relaxed). Slicing a buffer into a `str`. `cstr -> str` (R7). Growable `String`, concatenation, formatting, anything allocator-touching (Phase 6). Variadic foreign calls. Resource types and user destructor bodies (Slice 8b). A symbol-existence check (R14).
