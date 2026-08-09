# Phase 4 Slice 8a: static overloading, the mechanism (brief)

Sooth already dispatches one name over several operand types. It does it in hand-written
Rust match arms, not in any table, which is why `builtin_table` (`src/check.rs:224`) is empty
with no entries and no users. This slice retires the arms into a real table.

The reason that is worth doing is not tidiness. Operators are already just words, so a user
overload needs no new syntax and no new parse form: `: + ( Vec2 Vec2 -- Vec2 ) ;` is the
ordinary definition form and it already parses and checks. What is missing is dispatch, and
its absence is silent. Verified by compiling, both halves:

```
type: Vec2 x i64 y i64 ;
: + ( Vec2 Vec2 -- Vec2 ) | a b | a Vec2>x b Vec2>x - a Vec2>y b Vec2>y - Vec2 ;
: main ( -- ) ;
=> compiles clean, links, and adds exactly one symbol to the binary

: main ( -- ) 1 2 Vec2 3 4 Vec2 + Vec2>x . ;
=> error: type mismatch in `main` (line 7)
     `+` requires two operands of the same numeric type, found `Vec2` and `Vec2`
```

The call site dies in `check_operator`'s `is_numeric` gate (`src/check.rs:6695`) before any
environment lookup happens, naming the exact two types the author just defined an overload
for and refusing anyway. So Sooth today accepts a word, lowers it, links it, and can never
reach it. That is the Forth silent-failure class this language exists to convert into a
compile error, sitting in the compiler itself.

## Recon: measured against the built compiler, read against the current checker

**1. The dispatch is structural, not gated by a predicate.** `check_term` tries
`check_shuffle`, `check_operator`, `check_str_word`, and `check_array_word` in order, each
returning `Ok(None)` to fall through, and only then consults the word env. There is no
"is this a builtin" branch to flip at the call-site level. `is_builtin_word_name`
(`src/check.rs:1585`) exists but has exactly one caller, `src/check.rs:1519`, the `extern:`
redeclaration check. So `extern:` redeclaring a builtin is *already* a located error
(`check_extern_redeclaring_a_builtin_is_error`), while a `:` word redeclaring one is
accepted silently. The asymmetry is the bug, and it is one-sided in the direction that hurts.

**2. `builtin_table`'s entry type cannot hold overloads at all.** It is
`HashMap<String, Sig>`, one effect per *name*, and `Sig` is
`{ inputs: Vec<Type>, outputs: Vec<Type> }` (`src/check.rs:23`). Several candidates per name
is not an extension of that shape, it is a different shape. This is the slice's first design
question, not an implementation detail.

**3. An entry has to carry a lowering, not just an effect, and `len` proves it.** `len` has
two *shipped* candidates that differ in consumption and in codegen, dispatched by fall-through
across two functions. On `str` (`check_str_word`, `src/check.rs:7652`) it pops its operand and
emits a runtime `Instr::StrLen` load (`src/ir.rs:3052`), because R8 carries a string's length
in the value. On an array (`check_array_word`, `src/check.rs:7730`) it leaves the array on the
stack and folds to the constant `N` read off the type, emitting no instruction at all. Same
name, different arity of effect, different lowering. A table of signatures cannot express
that; a table of `(signature, lowering)` can.

**4. `len` is also the case that breaks a table of concrete rows.** Its array candidate is
non-consuming over an array of *any* length and element type. That is a `PolySig`-shaped
entry (slice 1's machinery), not a finite enumeration. Either the table carries generic
entries from day one or `len` is carved out and stays hardcoded, and the roadmap's claim that
`len` is simply absorbed is the first thing the spec should check rather than inherit.

**5. `.` dispatches on a category that exists only as Rust code.** `src/check.rs:6839`:
`is_numeric() || is_bool() || matches!(Str | Cstr)`. There is no list of printable types
anywhere to copy into a table. The fix is settled below as N concrete rows, one per `IrType`
it already handles, each tagged to the existing `Instr::Print` lowering (`backend/qbe.rs`).
No category/bound key is needed, because every row is an exact-type match like any other
overload -- which is also what lets a user's own `: . ( Vec2 -- ) ;` become reachable through
the same table, for free.

`.`'s lowering stays backend code here because moving it is strictly larger work, *not*
because it cannot move. `extern:` cannot express a variadic C call today, but QBE emits
variadic calls natively -- that is exactly what `Instr::Print` already does -- so no runtime
shim library is required. The path to `.` as ordinary library code is DESIGN.md's bounded-row
entry (`..N`), plus a way to decompose a `str` descriptor for `%.*s`, plus accepting that `.`
becomes `hosted`-layer rather than universally available. All of that is out of scope here.

**6. `unify_pair` is one cross-cutting rule, not per-operator logic.** `src/check.rs:206`
handles literal and size-type coercion for the dozen binary operators
(`+ - * mod and or xor = < > <= >= <>`), and carries X10's specific "needs an explicit
conversion to `usize`" diagnostic. Whether it runs before lookup or becomes table rows
decides how much of `check_operator` actually disappears.

**7. A latent symbol collision was on this slice's path, and turned out to be general.**
`qbe_name` (`src/backend/qbe.rs:186`) replaced every character outside `[A-Za-z0-9_.]` with
`_`, which is not injective: `+` and `-` both sanitized to the bare symbol `_`, and so did any
two ordinary word names made of the same count of symbol characters (`~` and `?`, reproduced
on `main` with no relation to overloading at all). Fixed standalone, outside this slice, since
it was reachable today independent of dispatch and fixing it (not just diagnosing it) was the
only correct answer: there is nothing wrong with a program defining both `+` and `-`, so a
located collision error would have permanently forbidden a legitimate program rather than
rejecting an actually-invalid one.

## Decisions (settled in ROADMAP.md, not reopened by the spec)

1. **No shadowing.** A user overload whose input types exactly match an existing candidate,
   builtin or imported, is a located error rather than a silent override.
2. **Exact match beats coercion.** `unify_pair`'s coercion ranks below an exact-type
   candidate, so adding an overload cannot silently steal a call site that previously
   coerced.
3. **Overloads are imported, not carried by the type.** Importing `Vec2` does not bring `+`
   for it. Absence is a resolution error naming the missing overload, never a fallback.
4. **One arity per name in scope.** Candidates must agree on input count, because the
   resolver has to know how deep to read the stack before it can match anything.
   Disagreement is a located error where the second candidate enters scope: the definition
   site when one is local, the import site when both are imported. Never a call-site
   ambiguity resolved by ranking.
5. **Overlap between a concrete and a generic candidate is rejected, not ranked.**
   `: + ( 'T 'T -- 'T )` beside `: + ( i64 i64 -- i64 )` is not identity (rule 1 doesn't
   catch it: a poly word's `effect` is empty by construction, its signature living in
   `PolySig`, so the types don't textually match), but it is still two candidates whose
   domains overlap at every concrete type. No specialization ordering: reject it the same
   shape as rule 1, at whichever site the second candidate enters scope. Nothing in the
   language today needs a generic default with a concrete override to coexist, and inventing
   ordering semantics for a consumer that doesn't exist yet is exactly the mistake the
   generic-struct-declarations item already made once. Loosen this later if a real consumer
   asks for it.
6. **`.` gets N concrete rows, not a category key.** One row per `IrType` it already
   handles (`i64`, `f64`, `Bool`, `Str`, `Cstr`, ...), each an exact-type match tagged to the
   existing `Instr::Print` lowering (finding 5). No `Bound`-style category mechanism is built
   for this slice; `.`'s printability was the only candidate for one, and it doesn't need it.

Enforcement reuses the two sites that already exist rather than adding any:
`check_duplicate_word_names` for definitions, and `check_selective_imports`'
`selective_collision_error` / `selective_collides_with_local_error` (`src/check.rs:1894`,
`:1903`) for imports, which are already the two halves rule 4 needs.

`check_duplicate_word_names`' key widens from `(module, name)` to
`(module, name, input types)`. Not an exemption class for overloadable names: that would
reproduce the bespoke `find_drop_overloads` registry (`src/check.rs:981`) this slice exists to
generalise, and two collision checks that must agree with each other is a worse failure mode
than one check with a wider key. Not a deletion either, since that hands back the bare linker
error the check was added to replace. `drop`'s exemption survives 8a untouched and dies in 8b.

## Open questions for the spec

- **The entry type.** Findings 2, 3, and 4 rule out `Sig`. An entry carries a lowering, and at
  least one shipped candidate set (`len`) is generic over length and element type. Does the
  table hold generic entries from day one, or does `len` get carved out?
- **Where `unify_pair` runs.** Before lookup, leaving the table to answer only "is this
  operator defined for these types"? Or as table rows, which multiplies entries and has to
  re-derive X10's specificity from a lookup miss?

## Out of scope

- **Everything `drop`.** Polymorphic `drop`, the structural-totality constraint, the
  disposal-scope invariant, and the destructuring-bypass hole are 8b. `drop` keeps its
  bespoke registry and its duplicate-check exemption through this slice.
- **Dispatch on outputs.** Inputs only. Return-type overloading is not on the table.
- **Traits, type classes, or any dispatch the compiler cannot resolve statically.** The
  slice's name is *static* overloading; every call site resolves at compile time or errors.
- **The deferred view type** (DESIGN.md, *Slicing a buffer into a view*), which records this
  slice as its ordering gate. It would add a third `len` candidate of the runtime kind, so the
  entry shape must not preclude one, but nothing here waits on it.
- **Moving `.` (or any builtin) onto `extern:`.** `.`'s lowering stays backend code (finding
  5); only its dispatch key changes. Giving Sooth an actual linked runtime library, so a
  print primitive could be declared via `extern:` instead of hand-lowered, is a real question
  for later, not one this slice needs an answer to.

## Exit

- `check_operator` / `check_term`'s type-directed match arms are gone and `builtin_table` is
  populated.
- The full existing corpus, goldens and examples, is unchanged byte-for-byte.
- A user-defined `: + ( Vec2 Vec2 -- Vec2 ) ;` compiles *and dispatches* at a call site with
  two `Vec2` operands.
- Rule 1's collision, rule 3's missing import, rule 4's arity clash, and rule 5's
  concrete/generic overlap are each a located error, with no definition left silently
  unreachable.
- `: + ( i64 i64 -- i64 ) ;` beside `: - ( i64 i64 -- i64 ) ;` in one file compiles and links
  (the `qbe_name` fix, merged ahead of this slice).
