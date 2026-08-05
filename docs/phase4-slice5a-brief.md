# Phase 4 Slice 5a: native modules, imports, and encapsulation (brief)

Slice 4 landed quotations and `times`. Slice 6 wants the combinator library written as
ordinary Sooth words, which is the first artifact in this project meant to be consumed by
more than one program, and that is what finally puts pressure on modules. The scope grew
once during planning, for a reason worth recording: word-only imports would serve the
combinator library and almost nothing else, because a reusable component is usually a
*type* plus its operations, not a bag of functions. So this slice carries multi-file
compilation, type imports, and encapsulation together, on the native path only. REPL
imports are 5b.

Two recon findings below change the shape of the work rather than merely sizing it. The
parser resolves type names in a **pre-pass over raw tokens before any body is parsed**, so
imports cannot be a post-parse merge: the importing file's parse already needs the imported
file's type names. And **a type name and its constructor word are one identifier**, which
makes `export: Queue` ambiguous in a way the ROADMAP entry did not anticipate and which
Elm answers directly.

## Recon: what already exists (measured, not assumed)

**1. The driver is single-file and straight-line.** `driver::build` (`src/driver.rs:18`) is
`read_to_string` -> `lexer::lex` -> `parser::parse` -> `check::check` -> `ir::lower` ->
`backend::qbe::emit`, over one path. `check::check(&mut module)` (`src/check.rs:1028`)
takes exactly one `Module`. There is no include, no prelude injection, no search path, and
no notion of a compilation unit larger than a file (`grep` for `include_str`/`prelude`
across `src/` is empty). So the entry point is genuinely restructured by this slice: the
ROADMAP's earlier claim that the change is "additive at the front of the pipeline" is
wrong and this brief retires it.

**2. Type names are resolved by a pre-pass over tokens, before bodies parse.** `parse`
(`src/parser.rs:240`) runs `prepass_type_decls` (`:59`) which scans the token stream for
`type:` and collects declared names, then `build_registries` (`:203`) allocates the struct
and enum registries, and only then are word bodies parsed. The reason is ordinary: a `:`
word's effect can mention a type declared later in the file, so names must exist before
effects are read.

   **Consequence, and this is the structural finding of the brief:** an import cannot be
   resolved after parsing, because the importing file's own pre-pass needs the imported
   file's type names to already be in the registry. The pipeline must resolve the import
   graph, then run **one pre-pass across the whole import closure**, then parse bodies. The
   attractive alternative (parse each file independently, merge afterwards) requires
   remapping every `StructId`/`EnumId`/`ArrayId` in the second file's already-parsed AST,
   because those ids are positional, which is strictly more work and more places to get it
   wrong.

**3. Registries are positional; type lookup is first-match-by-name.** `build_registries`
pushes into `Vec<StructDecl>`/`Vec<EnumDecl>`, so `StructId` is an index, and the declared
name is stored twice (`name: String` and `name_static: Box::leak(...)`, `src/parser.rs:211`)
because `Type::Struct(StructId, name_static)` renders itself without a registry. Resolution
is `structs.iter().position(|d| d.name == struct_name)` (`src/check.rs:6201`, `:6257`), a
linear first-match on the bare name.

**4. Duplicate type names are already a hard error, which a naive merge turns into a
module-collision bug.** `check_types` calls `check_duplicate_type_names`
(`src/check.rs:1420`). Measured:

```
type: Q a i64 ;
type: Q b i64 ;
-> error: duplicate type `Q` (line 2, col 7)
```

So if two files each declare `Point` and their registries merge into one flat namespace,
the program is rejected. That is safe rather than silent, but it is still wrong: two
modules must be able to declare same-named types. Either the qualifier becomes part of the
stored name, or the duplicate check becomes per-module. See D2.

**5. A type name and its constructor word are the same identifier.** Measured: `type: Q a
i64 ;` then `7 Q` constructs a `Q`, and `Q>a` reads the field, in one program that
compiles. So `Q` lives in both the type namespace and the word namespace, and a `type:`
declaration generates a constructor, getter, peek, setter, and destructure word per field,
all ordinary words in the flat environment.

   **Consequence:** `export: Queue` is ambiguous. It could mean the type name (so consumers
   can write `( queue::Queue -- )`), the constructor word (so consumers can *build* one),
   or all the generated accessors as well (so consumers can reach the fields). These are
   three different encapsulation stories, and the first without the second is exactly
   Elm's opaque type: `exposing (Queue)` exports the name, `exposing (Queue(..))` exports
   the constructors, and that distinction *is* the encapsulation mechanism. This needs
   settling in syntax, not left to a default. See D3.

**6. `::` needs no lexer change.** `is_delimiter` is `; ( ) | [ ]` only
(`src/lexer.rs:24-26`); `:` is an ordinary word character, which is why `type:` and
`extern:` are plain `Token::Word`s dispatched by string comparison (`src/parser.rs:65`,
`:261`, `:267`). So `queue::push` lexes as a single `Token::Word`, and `import:`/`export:`
join the existing defining-word family with no tokenizer work.

**7. Symbol mangling sanitizes punctuation, which can collide across modules.**
`instantiation_symbol` (`src/ast.rs:488`) maps every non-alphanumeric character to `_`
(`:489-493`). So a qualified name reaching the mangler makes `queue::Queue` and
`queue__Queue` the same symbol. With `RTLD_GLOBAL` first-loaded-wins already a known hazard
in this codebase (slice 2's brief traced a silent wrong-body call from exactly this), the
spec should say how qualified names enter symbols rather than discover it later.

**8. Destructor discovery is whole-module.** `find_drop_overloads(&module.words,
&module.structs)` (`src/check.rs:1031`) runs over one module's words, and the result is
recorded back onto the `StructDecl` as `has_drop_overload` because "every `is_copy` call
site, `ir`'s layout fold, and the REPL's persistent registries all read the same
`StructDecl`" (`:1033-1036`). An imported module's `drop` override must therefore be
discovered in the same pass as the importer's, which the merged-registry approach gives
for free and a per-file approach would not.

**9. Today's behaviour for `import:`, measured, in both front ends.** Native:

```
error: parse error: expected `:`, found Word("import:") at line 2, col 1
```

REPL:

```
import: queue "queue.sth" ;
-> parse error: unexpected token Semicolon at line 1, col 27
```

The REPL one is the problem: it is unlocated, mentions a semicolon rather than the
construct, and will get *worse* once 5a teaches the native parser `import:` while the REPL
line path still does not know it. Pinning this is 5a's job (D7).

**10. The REPL already threads registries into parsing.** `parse_line_with_structs`
(`src/parser.rs:311`) takes the session's struct/enum/array registries so a line can name a
previously-declared type. That is the seam 5b will widen, and it is worth not breaking
here.

## The paper pre-check

Per the technique that shaped the phase plan, the dogfood was hand-written first, in the
proposed syntax, to find out what the compiler owes it.

```
\ queue.sth
type: Queue buf [i64 8] head usize count usize ;

: empty ( -- Queue ) ... ;
: push ( Queue i64 -- Queue ) ... ;
: pop ( Queue -- Queue i64 ) ... ;
: grow ( Queue -- Queue ) ... ;

export: Queue empty push pop ;
```

```
\ main.sth
import: queue "queue.sth" ;

: main ( -- ) queue::empty 5 queue::push queue::pop . drop ;
```

What it owes, beyond the obvious:

- **`grow` must be unreachable from `main.sth`, and saying so must be legible.** Today the
  failure lands as `error: unknown word`grow` in `main`(line N)` (`src/check.rs:3753`),
  which is wrong: the word exists, it is not exported. Diagnostics are behaviour in this
  project, so the message is part of the spec, not an implementation detail (D5).
- **`Queue<head` must be unreachable too**, or the invariant `push`/`pop` maintain is
  decoration. That is the whole reason encapsulation is in this slice, and it is only
  achievable because the generated accessors are ordinary words that an export list can
  decline to name.
- **`main` never writes `queue::Queue`, but a consumer that binds one does**, so the *type*
  parser must accept a qualified name, not just the word parser. Since `queue::Queue` is
  one token, this works if the registry key carries the qualifier, which is D2.
- **An exported word's effect may name an unexported type.** `pop ( Queue -- Queue i64 )`
  is fine only because `Queue` is exported. Nothing today stops a module exporting `push`
  while keeping a type in its effect private, which would hand the consumer a signature it
  cannot name. Rust has a lint for exactly this shape. Needs a rule (D4).
- **`0 8 fill` interns `[i64 8]` into an `ArrayId`.** Arrays are structurally interned
  (`src/ast.rs:21-27`), so a merged registry dedupes two files' `[i64 8]` automatically,
  but only if the merge happens before checking, which is R2 again.
- **If `Queue` ever holds a `^`, it becomes linear and the consumer must be able to dispose
  it.** Reachable two ways once D3 is transparent: a bare `drop`, which dispatches on the
  concrete type and reaches the module's destructor glue unnamed, or destructuring down to
  Copy leaves. So this is a positive golden rather than an enforcement rule (D6). It was the
  motivating case for opacity's disposal rule, and dropping opacity is what removed it.

The pre-check did **not** turn up a missing slice this time. The scope as written is
sufficient to make the dogfood compile, which is the useful negative result.

## Decisions the spec has to make

**D1. Where the import closure is resolved.** Recommendation: resolve the graph, order it
topologically, reject cycles with a located error naming both files, then run one shared
pre-pass and one shared registry set across the closure, then parse bodies per file. This
follows from R2 and avoids all id remapping. The spec should also fix whether a path is
relative to the importing file (recommended) or to an invocation-relative root, and say
what happens when the same file is imported twice by different importers (dedupe by
canonical path).

**D2. How the qualifier is stored.** *Resolved to (b) below; both candidates are recorded
because every later rule depends on the choice and review should be able to re-weigh it.*
   (a) *Qualified in the name*: the registry stores `queue::Queue`, so `check_duplicate_type_names`
   keeps working unchanged and rendering is free, but the declaring file's own unqualified
   `Queue` references need rewriting during its parse, which means the parser needs
   file context it does not have today.
   (b) *Name plus owning-module tag*: the registry stores `Queue` with an owner, and
   resolution takes `(name, current_module)`, searching own-module first, then imports by
   qualifier. Duplicate-name checking becomes per-module. This keeps parsing file-agnostic
   and is the recommendation, but it touches every `structs.iter().position(...)` site.

**D3. What `export:` means for a type.** *Resolved to transparent, with no opacity mechanism
at all, below.* Naming a type in `export:` exports the type together with its generated
words. There is no `Queue(..)` distinction, no name-versus-constructors split, and no way to
export a type abstractly in this slice.

The reasoning matters because an earlier draft of this brief said the opposite. Sooth structs
are dumb data with no logic on them, so hiding a constructor or setter buys much less here
than it does in a language where a violated invariant is undefined behaviour: there is no UB,
array indexing traps (Phase 2 slice 5), and linearity already prevents the aliasing hazards.
A consumer who builds a `Queue` with nonsense field values gets a trap or a wrong answer *in
the code that did it*. That is an ordinary bug in their program, not a hole in the library.

The resource argument for opacity also fails, and it fails on a measured fact: destructuring
a type bypasses its `drop` override entirely, today, in a single file.

```
type: R tag i64 ;
: drop ( R -- ) | r | "DROPPED " . r R>tag . ;
: main ( -- ) 7 R | r | r R>tag . ;
```

prints `7`, not `DROPPED 7`. So visibility was never what protected resource discipline, and
hiding accessors across a module boundary would not have protected it either. Rust closes
this with E0509 (cannot move out of a type implementing `Drop`); Sooth does not. That is a
checker rule with nothing to do with modules, recorded against slice 8 where the sibling
question (what discharges a linear obligation) already lives.

Honest costs of transparency, both of which this slice must therefore specify:
   (a) A qualified accessor spelling such as `queue::Queue>buf` becomes *valid* and must
   resolve, where under opacity it was always a visibility error and only needed a
   diagnostic. Since `>` is not a delimiter, that token splits on the first `::` into
   qualifier `queue` and name `Queue>buf`.
   (b) Selectively importing a type (D9) must bring its generated words in unqualified too,
   which was not a question when accessors could not cross at all.
   (c) A consumer can now destructure an imported type and so bypass its destructor. This is
   not a new hole, it is the pre-existing one above becoming reachable across a file
   boundary; opacity was accidentally papering over it, and only for types whose author chose
   opacity. The fix is the E0509-style rule, not a visibility rule.

Per-member control is dropped with opacity, deliberately, and that is consistent with the
slice's premise. Generated accessors are ordinary words, so per-member withholding is
*implementable* (a withhold marker on the export list), but it is not in 5a: `export: Queue`
is all-in, exporting the type and every generated word, and there is no syntax to export the
type while withholding its setter. Hiding accessors is the OOP ceremony this language is
stepping away from, and the export list's job in 5a is name visibility across files, not
field-level access control. If a real consumer ever wants it, a withhold marker is an
additive feature landing with that consumer.

**Framing to carry into the spec.** A `type:` declaration introduces a *name-scope* whose
generated words are its members (the spellings are literally `format!("{}>{}", type, field)`
and siblings, `src/check.rs:1788-1795`, `src/ir.rs:624-627`, i.e. an ad-hoc qualified
namespace built by string concatenation). Visibility over those members is the ordinary
export mechanism applied to that scope, not a special rule for types. Reframing them as
actual module members was considered and declined for this slice: `::` cannot replace the
`>`/`<`/`|>` separators because those encode three *different operations* on one field, so it
would force either two separators or a corpus-wide rename, and it would drag in the
hierarchical module paths this slice deliberately declines. Revisit if hierarchical modules
arrive for another reason.

**D4. Exported signatures may only mention exported types.** A checkable rule, and without
it a consumer can receive a signature naming a type it cannot write. The spec should decide
whether this is an error at the *declaration* (recommended, it is the module author's bug)
or at the use site.

**D5. Where privacy is enforced, and what it says.** Filtering unexported names at splice
time is the simplest implementation and gives the wrong diagnostic (R9, and the pre-check).
Recommendation: splice everything, mark visibility, and reject at the use site with a
message naming the module and the fact of non-export, distinct from `unknown word`. Test
both: an unexported name and a genuinely absent one must not produce the same error.

**D6. The disposal/export rule dissolves in this slice; specify a positive golden only.** The
ROADMAP frames it as "an exported opaque linear type must also export the word that discharges
its obligation, or the consumer is stuck." Two independent reasons it does not bite in 5a.
First, `drop` here is compiler-known and dispatches on the concrete type, so it always reaches
an imported type's destructor glue whether or not that glue was exported: a destructor runs
without being named. Second, D3 dropped opacity, so a consumer can also always destructure
down to Copy leaves. The premise needed an undisposable value and there is no longer a way to
hold one.

So: no export-site disposal enforcement in 5a. Specify one positive golden (an imported linear
type disposed by `drop` in the consumer, its destructor observably running) and defer
enforcement to slice 8, where a polymorphic `drop` could be structurally total and the premise
finally becomes reachable.

**D7. `import:` at the REPL is a specified, tested rejection in this slice.** Not deferred
to 5b. Slice 1 shipped without pinning REPL behaviour for polymorphic words and slice 2's
recon found the gap had produced a silent miscompile rather than an error. R9 shows today's
REPL message is already poor and will degrade once the native parser learns the form. A
located "`import:` is not supported at the REPL yet" is one diagnostic and it is the
difference between a deferral and a defect.

**D8. How a qualified name enters a symbol.** Given R7's sanitization, either qualified
names never reach the mangler (module resolution happens entirely before symbol minting)
or the mangling gains a separator that cannot collide. State which.

**D9. Selective import, additive to the qualifier.**

```
import: queue | push pop | "collections/queue.sth" ;
```

The `| ... |` clause is optional and *additive*: it binds the qualifier `queue` as always,
and additionally exposes the listed names unqualified. One form with an optional clause
rather than two competing forms, so `queue::grow` stays reachable for anything not listed
and a qualified spelling is always available as the disambiguator.

`|` is already a lexer delimiter (`src/lexer.rs:24-26`), so this needs no tokenizer work,
and it is the only bracket left that does not clash: parens are stack effects, square
brackets are quotations and array types. It also rhymes with `| a b |`, since both forms
mean "these names enter scope here." The spec should note this is the fourth use of `|`
(locals, enum variants, this), each disambiguated by its enclosing form.

Justified now rather than deferred because qualification costs more in a concatenative
language than elsewhere: word names appear inline in dense chains, so
`arr [ 1 + ] combinators::each` is bad in a way `combinators::each(arr)` would not be in
Rust, and the combinator library is one slice away. The marginal cost over qualified-only is
a binding step plus one diagnostic, not new machinery.

**The collision rule is what keeps this cheap, and it is dumb on purpose:** two selective
imports naming the same word is an error at the second import, located, naming both
modules. No precedence, no shadowing, no ambiguity resolution at the use site. A
selectively-imported name colliding with a locally-defined word is the same error. This is
precisely why *selective* unqualified import is admissible while wholesale `USING:`-style
import is not: the collision is explicit in the source and caught at the import site,
rather than arising implicitly from two modules' entire surfaces and surfacing at some
distant use.

## Scope

**In:** an `import:` form; an `export:` form; qualified reference syntax; multi-file
resolution, ordering, and cycle rejection in the driver; one shared registry set across the
import closure; visibility enforcement with its own diagnostic; the exported-signature
rule; native compilation and linking of a multi-file program; a specified REPL rejection.

Also in: selective import, per D9.

**Out:** REPL imports (5b); a serializable API description, version diffing, and anything
else `docs/dependency-management.md` needs (Phase 6); a package manifest, dependency
resolution, or registry; a `mod.sth`-style directory-mirrors-module-tree convention
(deliberately declined, it solves problems this language does not have yet, and a flat
file-is-a-module model with qualified access covers the only consumer that exists);
re-exports; aliasing an import to a different local qualifier; wholesale unqualified import
(Factor's `USING:` shape, declined per D9); generic type declarations crossing files
(Phase 6, they do not exist yet).

## Exit

Two files, one importing a type and a word from the other, compiling and linking as one
program and running. A non-exported word used from the importing file is a located error
that names the module and does not say `unknown word`. `import:` at the REPL is a located
rejection. The combinator library (slice 6) has a file to live in, and an example imports a
type from another file.

## Decisions taken on the owner's behalf

These were the brief's open questions. Each is settled with its reasoning so the spec has a
definite instruction rather than inventing an answer, and each is cheap to reverse at spec
review.

1. **D3 resolves to transparent, with no opacity mechanism in this slice.** See D3 above for
   the full reasoning and its three specified costs. Nothing in 5a's scope wants opacity: the
   dogfood is the combinator library (words, no types) plus a small `Point`/`Vec2`, which is
   dumb data. Fine-grained control survives through the export list naming individual
   generated words. Abstraction is a clean additive follow-on if a real consumer ever appears
   (a validated newtype is the plausible one), and it should arrive with that consumer rather
   than ahead of it.
2. **D2 resolves to (b), name plus owning-module tag.** Resolution takes
   `(name, current_module)`, own-module first, then imports by qualifier; duplicate-name
   checking becomes per-module. Chosen because it keeps parsing file-agnostic, at the cost of
   touching every `structs.iter().position(...)` site.
3. **The `.sth` extension is written explicitly in the path.** No implicit extension, no
   search path, no resolution rule to learn. Consistent with `extern:` naming its symbol
   verbatim.
4. **5a does not carry the slice 6 dogfood.** There is no combinator library to move yet;
   that lands in slice 6, which writes it.
