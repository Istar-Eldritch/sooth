# REPL UX: interactive editor, continuation, typed rendering, meta-commands

**Cross-cutting tooling (not a numbered phase)** under `ROADMAP.md`'s "Cross-cutting — Tooling and diagnostics". Grew `src/repl.rs` from a pipe filter into a usable interactive REPL via four slices. Editor logic landed in a new `src/editor.rs`; meta-command goldens in `tests/repl_ux.rs`.

## Fixed constraints

- **F1** Hand-rolled termios raw-mode editor via `extern "C"` (`tcgetattr`/`tcsetattr`/`read`/`write`), mirroring the existing `dlopen` block. No new crate.
- **F2** The piped (non-tty) path stays byte-for-byte. All editor affordances gate on `stdin.is_terminal()`. The load-bearing regression witness: `tests/phase1.rs` and `tests/phase4_repl_imports.rs` pass unchanged.
- **F3** No compilation-strategy change: no lex/parse/check/lower/emit, QBE, linear-spine, or `dlopen` change. I/O, buffering, and display only.

## Locked decisions

- **D1** `run()` branches once at entry on `is_terminal()`. Both paths funnel each committed logical line through one shared dispatch helper (blank-skip, meta-command, `:quit`/EOF, `eval_line`), preserving the piped call sequence by construction.
- **D2** Rich stack rendering is a tty-only affordance. Piped path keeps calling today's `format_stack` verbatim; tty path calls a new pure rich formatter (unit-tested directly). Goldens stay byte-for-byte.
- **D3** Meta-commands (`:help`/`:words`/`:type`/`:stack`/`:clear`) dispatch in the shared helper *before* `eval_line`, so they are golden-testable piped and work interactively. No existing golden regresses (no script uses these tokens).
- **D4** `:clear` disposes before it resets: runs residual-stack destructors (`dispose_residual`, as `end_session` does) then clears env/stack/registries/generations. Reset is scope-end, not a forget.
- **D5** `RawModeGuard` saves cooked termios on construction, restores in `Drop` — so `:quit`, Ctrl-D, an unwinding eval error, and a panic all leave the terminal cooked.
- **D6** The `src/main.rs:34` `error: error:` double-prefix stays out of scope.

## Testability architecture

Pure decision logic is separated from the thin syscall shell: key-decode + edit-buffer transform is a pure function over (bytes, buffer, cursor, history); input completeness is a pure predicate over tokens; the rich formatter is a pure function; meta-commands run on the shared (piped-golden-testable) path. The termios/`read`/`write` shell is the untested-by-golden layer; the guard's restore gets a unit test through a seam.

## Requirements by slice

### Slice 1 — raw-mode line editor (tty only)
- **R1** `run()` gains the D1 branch; `driver::repl` (`src/driver.rs:302`) unchanged in shape.
- **R2** `RawModeGuard` (D5): saves termios, sets raw mode (`ISIG`/`ICANON`/`ECHO` cleared, 1-byte reads), restores on `Drop`.
- **R3** Terse primary prompt written before each line (tty only).
- **R4** UTF-8-naive byte-buffer editing: insert at cursor, left/right, home/end, backspace, delete; re-render via ANSI escapes.
- **R5** History: in-memory ring + persistent file (`SOOTH_HISTORY` override), append-on-commit, capped. Recall walks history; editing a recalled entry doesn't mutate the stored one; blanks and immediate dups not recorded.
- **R6** *(regression witness)* Arrow escape sequences are consumed by the decoder, never handed to `lexer::lex`.
- **R7** Ctrl-C aborts the line (no process exit); Ctrl-D on empty line is EOF (`end_session`→`dispose_residual`); Ctrl-D mid-line behaviour stated in a unit test.
- **R8** One committed logical line per Enter, handed to the shared dispatch helper.

### Slice 2 — multi-line continuation (tty only)
- **R9** Pure `input_is_complete(&[Token]) -> Completeness`: `NeedMore` for an open `:`/`type:` (closed by `Semicolon`) or unbalanced `[`/`]`; `Complete` otherwise. A balance count, never a rejection.
- **R10** On `NeedMore`, write a distinct continuation prompt and append to a pending buffer; dispatch the joined text once `Complete`.
- **R11** Ctrl-C with a non-empty pending buffer discards the buffer, not the process.
- **R12** Continuation is tty-only; the piped path never buffers across lines (F2).

### Slice 3 — typed stack rendering (tty only)
- **R13** Rich formatter (new fn, D2), tty-only; piped keeps `format_stack` (`src/repl.rs:314`). Signature mirrors `format_stack` (buf + types + four layout tables), pure/unit-testable.
- **R14** Aggregates render contents: struct field values, enum active variant+payload, array elements — reusing `format_stack`'s offset/cell arithmetic.
- **R15** Ambiguous scalar widths made visually distinct (a `u8` `1` vs an `i64` `1`); rendered type must be recoverable from the line.
- **R16** *(load-bearing)* Reading an owning cell (`^T`) payload for display is a read: leaves the value live, stack unchanged, consumes no linearity, runs no destructor.

### Slice 4 — meta-commands (both paths; D3)
- **R17** Dispatched in the shared helper before `eval_line` on a leading `:`-word.
- **R18** `:help` — lists meta-commands, one line each.
- **R19** `:words` — defined word names with declared signatures, under user-facing names at current generation (not mangled symbols).
- **R20** `:type <line>` — lex→parse→check against current stack types, prints the stack effect, executes/mutates nothing. *(hazard)* must not grow the array/owned-cell/ref registries: snapshot and restore lengths (`parse_line_with_structs` takes `&mut` to them).
- **R21** `:stack` — prints the residual stack via the active formatter, no push/consume.
- **R22** `:clear` — disposes residual (D4) then resets env/stack/registries/generations.
- **R23** Tab completes the current word against the `:words` name list (shared listing logic); rides slice-1 keybindings, tty-only.

## Load-bearing invariants preserved

QBE backend, `Ptr[T]` opaque, no LLVM/native/JIT/comptime. Linear spine untouched (R16 read-without-consume; D4/R22 dispose-before-reset). `core` stays `no_std`. No new crate (F1), no `Instr`/`Terminator`/`qbe.rs` change (F3). Every piped golden passes byte-for-byte (F2/D1/D2).

## Exit criteria and tests

Naming `thing_condition_expected`. Editor/completeness/formatter units beside their code; meta-command goldens in `tests/repl_ux.rs` (piped harness, `tests/phase1.rs::run_session` shape).

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | left/right/home/end move cursor | `editor_arrow_keys_move_cursor_expected` | unit (editor) | 1 |
| 2 | backspace deletes char before cursor | `editor_backspace_deletes_char_before_cursor` | unit (editor) | 1 |
| 3 | up-arrow recalls previous entry | `editor_up_arrow_recalls_previous_entry` | unit (editor) | 1 |
| 4 | escape sequence never reaches lexer | `editor_escape_sequence_not_forwarded_to_lexer` | unit (editor) | 1 |
| 5 | Ctrl-C aborts line without exiting | `editor_ctrl_c_abandons_line_not_process` | unit (editor) | 1 |
| 6 | Ctrl-D on empty line is EOF | `editor_ctrl_d_empty_line_is_eof` | unit (editor) | 1 |
| 7 | history file round-trips, capped, no dup/blank | `editor_history_file_roundtrips_capped` | unit (editor) | 1 |
| 8 | guard restores saved termios on drop | `raw_mode_guard_restores_saved_termios_on_drop` | unit (editor) | 1 |
| 9 | existing piped goldens pass unchanged | `tests/phase1.rs`, `tests/phase4_repl_imports.rs` | golden (regression) | 1 |
| 10 | unclosed `:` def is `NeedMore` | `continuation_unclosed_def_needs_more` | unit (repl) | 2 |
| 11 | unclosed `type:` decl is `NeedMore` | `continuation_unclosed_typedef_needs_more` | unit (repl) | 2 |
| 12 | unbalanced `[` is `NeedMore` | `continuation_unbalanced_bracket_needs_more` | unit (repl) | 2 |
| 13 | balanced line is `Complete` | `continuation_balanced_line_is_complete` | unit (repl) | 2 |
| 14 | multi-line def buffers then submits as one | `editor_multiline_def_submits_as_one_line` | unit (editor) | 2 |
| 15 | Ctrl-C discards pending buffer, not process | `continuation_ctrl_c_discards_pending_buffer` | unit (editor) | 2 |
| 16 | struct slot renders field values | `format_rich_struct_shows_field_values` | unit (repl) | 3 |
| 17 | enum slot renders variant + payload | `format_rich_enum_shows_variant_and_payload` | unit (repl) | 3 |
| 18 | array slot renders elements | `format_rich_array_shows_elements` | unit (repl) | 3 |
| 19 | `u8` `1` distinct from `i64` `1` | `format_rich_u8_distinguished_from_i64` | unit (repl) | 3 |
| 20 | reading owning cell for display consumes no linearity | `format_rich_owned_cell_read_does_not_consume` | unit (repl) | 3 |
| 21 | piped path still uses plain formatter | `format_stack` units + #9 | golden/unit | 3 |
| 22 | `:help` lists meta-commands | `repl_help_lists_meta_commands` | golden | 4 |
| 23 | `:words` lists words with signatures | `repl_words_lists_words_with_signatures` | golden | 4 |
| 24 | `:type` prints effect, executes nothing | `repl_type_prints_effect_without_executing` | golden | 4 |
| 25 | `:type` doesn't grow registries | `repl_type_does_not_grow_registries` | unit (repl) | 4 |
| 26 | `:stack` prints without mutating | `repl_stack_prints_without_mutating` | golden | 4 |
| 27 | `:clear` disposes then resets | `repl_clear_disposes_then_resets` | golden | 4 |
| 28 | tab completes against word list | `editor_tab_completes_against_word_list` | unit (editor) | 4 |

Mutation-test the guards: #4, #5/#15, #8, #20, #25. #9 and #21 are the F2 regression witnesses.

## Out of scope

REPL source spans/carets (future `check.rs` work). Any change to piped byte-for-byte output (F2). The `error: error:` double-prefix (D6). Native compilation, new crate, in-process JIT, IR/backend change.

## Implementation

- **Phase 1** (`c29c8ae`) — raw-mode editor, termios guard, prompt, cursor, history, tty gating: `src/editor.rs`, `src/lib.rs`, `src/repl.rs`.
- **Phase 2** (`aadb246`, review fix `95effe7`) — multi-line continuation via completeness predicate + pending buffer: `src/editor.rs`, `src/repl.rs`, `tests/repl_ux.rs`.
- **Phase 3** (`b98c167`) — typed stack rendering (aggregate contents, width-distinct scalars, read-without-consume): `src/repl.rs`.
- **Phase 4** (`54ee17c`) — meta-commands + tab completion; `ROADMAP.md` note: `ROADMAP.md`, `src/editor.rs`, `src/repl.rs`, `tests/repl_ux.rs`.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "raw-mode line editor: termios guard, prompt, cursor, history, tty gating", "difficulty": "hard" },
    { "phase": 2, "focus": "multi-line continuation via a pure completeness predicate and pending buffer", "difficulty": "standard" },
    { "phase": 3, "focus": "typed stack rendering: aggregate contents and width-distinct scalars, read without consuming", "difficulty": "standard" },
    { "phase": 4, "focus": "meta-commands help words type stack clear and tab completion", "difficulty": "standard" }
  ]
}
```
