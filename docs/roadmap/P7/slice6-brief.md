# Phase 7 Slice 6: surface syntax unification (brief)

**Sequence.** After S4b (bounds on impl variables) and S5 (linear array elements), both of
which touch the type and signature syntax this slice reshapes. Landing the syntax refactor
*after* those means their specs and tests are written in the new syntax directly, with no
transitional churn from rewriting freshly landed tests.

**Motivation.** Three surface warts, each independently defensible, together a legibility
pass that makes the polymorphic surface read the way a reader already expects.

1. **The bare `[` is overloaded.** `parse_poly_slot` (`src/parser.rs:3090`) disambiguates a
   `[` between a quotation and an array by a forward scan, `quotation_type_ahead`
   (`src/parser.rs:4043`), which walks to the matching `]` looking for a `--` at depth 1. A
   human reader does the same scan. Naming the array type makes the bracket a quotation
   delimiter unambiguously — the only remaining bare `[`.

2. **Generic binding and application are spelled differently.** A type variable is bound by
   postfix space-separated words (`type: Box 'T`, `trait: Ord 'T`), but applied by bracketed
   arguments (`Box[i64]`). Brackets at both sites unifies the shape: `Box['T]` binds,
   `Box[i64]` applies.

3. **A word's bounds are inside its effect.** `: word ( 'T: Ord 'T -- )` interleaves the
   bound declaration with the stack effect, so a reader must pick the bound out of the slot
   list. `: word['T: Ord] ( 'T -- )` separates the two: the bracket names the variables and
   their bounds, the effect names the stack. This mirrors how `type: Box['T]` already
   separates the parameter list from the fields.

## The four changes

### 1. `array['T 'N]` — the array type gets a name

The anonymous array shape `['T 'N]` becomes `array['T 'N]`; a concrete array is
`array[i64 4]`. The `array` name is reserved for the type syntax exactly as `Slice`/`!Slice`
(`src/parser.rs:242`) and `owning` (`src/ast.rs:2438`) are already reserved, and for the same
reason: it names a structural type whose delimiter would otherwise be ambiguous. A user
cannot declare `type: array ...`, just as they cannot declare `type: Slice ...` today.

The parser win is concrete: the `quotation_type_ahead` lookahead
(`src/parser.rs:4043-4055`) is deleted. A bare `[` in `parse_poly_slot`
(`src/parser.rs:3090-3094`) unambiguously opens a quotation; `array[` (a `Word("array")`
followed by `LBracket`) opens an array. The two-token lookahead
`quotation_type_ahead` scan, which today must walk to the matching bracket and check for
`--` at depth 1, is replaced by a single-token peek: is the token before `[` the word
`array`?

`ArrayDecl::name_static` (`src/ast.rs:1264`), the leaked `&'static str` spelling every
`Type::Array` carries, changes from `[i64 4]` to `array[i64 4]`. Every diagnostic and
pretty-printer that renders an array type picks up the new spelling through `name_static`
without per-site changes, though error message *strings* that hardcode the `[T N]` shape
in prose need updating.

### 2. Bracket binding sites for `type:` and `trait:`

The postfix type-variable list after a `type:` or `trait:` name moves into brackets,
matching the application syntax:

- `type: Box 'T val 'T ;` → `type: Box['T] val 'T ;`
- `type: Result 'T 'E | Ok 'T | Err 'E ;` → `type: Result['T 'E] | Ok 'T | Err 'E ;`
- `trait: Ord 'T` → `trait: Ord['T]`

The `header_ty_var_count` function (`src/parser.rs:104`), which counts `'`-prefixed tokens
after the header name, is replaced by an explicit bracket parse: after the name, an
optional `[`-delimited list of `'`-prefixed type variables, then the body. The `'` prefix
on each variable is retained (per the design decision to keep it this slice), so the
variable/non-variable distinction inside the brackets is the same `starts_with('\'')` check
it is today.

The zero-variable case is unchanged: `type: Ordering | ...` carries no brackets, exactly
as `type: Box['T]` carries no arguments to a non-generic `Box` — a bare name with no
following `[` is a concrete (non-generic) declaration, the same rule the application site
already uses.

### 3. Word-definition bound brackets

A word's type variables and their bounds move from inside the effect to a bracket between
the word name and the effect:

- `: word ( 'T: Ord 'T -- )` → `: word['T: Ord] ( 'T -- )`
- `: max ( 'T: Copy Ord 'T 'T -- 'T )` → `: max['T: Copy Ord] ( 'T 'T -- 'T )`

The bracket sits in the same position as `type:`'s and `trait:`'s parameter list, and
after the optional `inline` keyword, so `: word inline ['T: Copy] ( 'T -- )` reads
naturally. A word with no type variables carries no bracket: `: sq ( i64 -- i64 )` is
unchanged.

Inside the effect, `'T` is still a use of the variable bound in the bracket; the
`bound_on_use_error` check (`src/parser.rs:1761`) that today rejects a bound at a
non-binding occurrence stays, but its trigger moves: a `:` after a `'T` *inside the
effect* is now always an error (bounds live in the bracket, not the effect), rather than
conditionally consumed as a bound declaration. The `parse_poly_ty_var` bound-parsing arm
(`src/parser.rs:3302-3335`) moves to the bracket parser, and `parse_poly_effect`'s slot
parser no longer needs the `forbid_bounds` distinction for the word-def path (it remains
for the `impl:` target path, where bounds on impl variables are S4b's concern).

### 4. `impl:` target uses `array[...]`

The `impl:` target's `for` pattern changes its array spelling to match:

- `impl: Show for ['T 'N]` → `impl: Show for array['T 'N]`

The `parse_impl_target` function (`src/parser.rs:2667`) already routes through
`parse_poly_slot`, so this change falls out of change 1's parser simplification for free.
The `forbid_bounds` guard on `parse_poly_target` stays, since S4b's bounds on impl variables
arrive through a `where`-clause, not through the target pattern's slots.

## What this does not touch

- **The `'` prefix on type variables is retained.** Every type variable is still `'T`,
  `'N`, `'E`, etc. — the prefix is the parser's type-variable signal at every site, and
  dropping it is a separate (larger) decision this slice does not prejudge.
- **`^'T` (owning cell) and `&'T` (reference) stay sigiled.** These are single-argument
  structural types with no delimiter ambiguity (each is a dedicated sigil that lexes as part
  of the token it prefixes), and both double as value-level accessor prefixes (`&hp`,
  `^>`, `&!>`), so a bracketed type form would break the type/accessor visual unity.
- **`Slice[T]` and `!Slice[T]`** are already named-bracket types (`src/parser.rs:242`),
  unchanged.
- **Quotation syntax** `[ 'T -- 'T ]` and `~[ ... ]` is unchanged — a bare `[` now
  unambiguously opens a quotation, which is the point of naming the array type.
- **The checker, lowering, and IR.** This is a parser-and-test change. `PolySig`,
  `PolyType`, `Type::Array`, `ArrayDecl`, and every downstream consumer are unchanged:
  the AST the parser produces is identical, only the surface spelling that produces it
  changes. `name_static` spellings change, but they are display strings, not structural
  identity (which is `ArrayId` into the interned `arrays` registry).
- **The REPL.** The REPL's `type:`-line path (`src/parser.rs:1329`) and word-def path
  (`src/parser.rs:1252`) adopt the new syntax in lockstep with the build path.

## Scope and risk

This is a large mechanical pass: ~1190 `'T`-style occurrences across 41 test files, 8
library/example files, and every diagnostic that mentions `[T N]` or spells a bound inside
an effect. The risk is low (no semantic change, no AST change), but the surface area is
wide, so the golden tests and library files are the regression surface — every test and every
`.sth` file must be updated in one pass, and `cargo test` green is the exit gate.

The `quotation_type_ahead` deletion is the one structural simplification, and the one place
where a mistake could change parsing behaviour rather than just spelling: a quotation
mistaken for an array (or vice versa) would surface as a parse error or a wrong `PolyType`,
caught by the existing quotation and array golden tests.

**Exit:** every `[` in the language that is not a quotation is preceded by a type name
(`array[`, `Box[`, `Slice[`, `Result[`); `quotation_type_ahead` is deleted; `type:`,
`trait:`, and `:` binding sites use bracketed parameter lists; word-definition bounds live
in the bracket, not the effect; `cargo fmt --check && cargo clippy -- -D warnings &&
cargo test` is green; and the P7 goldens (`gcd.sth`, `factorial.sth`, the `lib/cmp.sth`
trait/impl family, the `lib/combinators.sth` polymorphic combinators) read in the new
syntax.
