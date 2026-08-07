# Sooth — design notes

A small, statically-checked concatenative language in the Forth/Factor/Kitten
lineage, compiled straight to native code with no external runtime. Working notes,
not a spec.

## What this is: a craft language

Sooth is built for the pleasure of building and writing it, not for a market. That
is a deliberate scope decision, made after arguing the alternative and rejecting
it (see "Why craft, not product" below). The consequences run through every choice
here: the language stays small enough to hold in one head, the compiler stays small and legible, and its one backend dependency
(QBE) is itself small enough to read rather than an opaque black box. Where a decision
trades reach or peak
performance for simplicity and legibility, simplicity wins.

The one intellectual bet that makes Sooth more than a tidy Forth clone: **in a
stack language the stack discipline already is move semantics.** Every word
consumes its inputs and produces its outputs, so a value is moved by default and
keeping a copy requires an explicit `dup`. Linear types therefore fall out for
free: `dup` is the explicit copy, and `drop` is the explicit, checked destructor
point. That
single idea pays off three times over (resource safety, deterministic destruction,
data-race-free concurrency) and is the reason to write programs in Sooth rather
than in Forth or Rust.

## Why craft, not product

Recorded so the scope decision isn't re-litigated by accident.

- **No market gap for a general-purpose version.** Every axis a serious version
  would compete on is already held: memory-safe no-GC systems work by Rust,
  proven-real-time by Ada/SPARK, refinement+SMT by F*/Dafny/Liquid Haskell,
  effects by Koka, data-race-free actors by Pony. A new general-purpose language
  faces a brutal adoption bar against incumbents that keep improving.
- **Therefore, for production, use a mainstream language** (Rust when its
  guarantees are needed, Go/typed-Python when they aren't). Sooth is the hobby,
  kept honestly separate.

Nothing below is justified by market need. It's justified by being interesting to
build and to write in.

## Surface language

Concatenative, Forth-flavoured, with two non-negotiable ergonomics that Forth
lacks: statically-checked stack effects and named locals.

`gcd`, to fix the shape. `| a b |` binds the top items left-to-right in the same
order as the effect comment, and a word calls itself directly (no `recurse`). A
binding is a term, not just an entry declaration: `| names |` is legal at any point
in a body, popping that many values off the stack where it appears, with its extent
running to the end of the enclosing block (a word body, a clause body, or an
`if`/`else` arm) rather than the whole word:

```forth
: gcd ( int int -- int )
  | a b |
  b 0 = if
    a
  else
    b  a b mod  gcd
  end ;
```

Locals are opt-in, not the default. Prefer the stack: with `dup`/`swap`/`drop` most
one- or two-value words stay point-free (`square`, below, is just `dup *`). Reach for
`| … |` only when shuffling would read worse than names, typically three-plus live
values reused out of order, like a formula:

```forth
: lerp ( int int int -- int )   \ a + (b - a) * t
  | a b t | b a - t * a + ;
```

`gcd` above sits on the line: two values, each reused and reordered in the recursive
call. It is shown with names here, but `swap`/`over` write it just as legibly (that
version is in the README), which is the point: two values is where the judgment call
lives, and `lerp`'s three is where names clearly win.

Checked stack effects are the cheap, high-value feature: Forth's signature failure
mode (a silent underflow producing a wrong number at runtime) becomes a compile
error.

```forth
: oops ( int -- int )
  | a | a a + + ;
```

```
error: stack effect mismatch in `oops`
  declared ( int -- int ), but body has net effect ( int -- ⊥ )
  a a + +
        ^ `+` needs 2 values, stack holds 1 here (one `+` too many)
```

Types and names live in different places, never both at once: the effect comment
carries the boundary **types** (`( int int -- int )`), and `| … |` introduces the
**names** the body uses. Naming a slot in the effect comment (`( a:int … )`) is an
alternative to binding it, useful as caller-facing documentation for a word that
juggles on the stack instead of naming; a slot that is bound by `| … |` stays a bare
type, so a name is never written twice.

## The linear spine

Plain data is `Copy`: reuse is free and `dup` is ordinary.

```forth
: square ( int -- int )
  dup * ;                \ int is Copy, so `dup` just copies the bits
```

A value is *moved* by default, and a resource is *linear*, not `Copy`. `dup` on
something that owns a resource is a type error. This is the whole point, and it is
where Sooth diverges from both Forth and Rust:

```forth
: leak ( File -- File File )
  dup ;
```

```
error: cannot `dup` a value of type File
  dup
  ^^^ File is linear: it owns an OS handle and has no Copy instance
  note: `dup` on plain data copies bits; there are no bits to copy here.
        thread the File through, or open a second handle explicitly.
```

There is no lifetime-tracking borrow checker. Operations that don't consume a resource
take it and hand it back, which in a stack language is just normal data flow
(`size-of ( File -- File int )` returns the File):

```forth
: report ( str -- )
  | path |
  path open-read         \ ( -- File )         acquire ownership
  size-of                \ ( File -- File int ) hands the File back
  print                  \ ( File int -- File ) print consumes the int
  close ;                \ ( File -- )          destructor runs HERE
```

Disposal is explicit and deterministic: a linear value is used *exactly once*. You
end its life by consuming it, either with a word that takes it and never hands it
back (`close` above), or with a bare `drop` when you hold a live value and have no
further use for it. `drop` is the single disposal primitive; `close`/`free` are
library words layered on top. Forgetting to dispose is **not** silently patched up,
it is a compile error:

```forth
: leak-file ( str -- )
  | path |
  path open-read
  size-of
  print ;                \ error: `print` leaves a File on the stack, but ( str -- )
                         \ declares no output. Close it, `drop` it, or return it.
```

A forgotten resource surfaces through the same stack-effect check that catches a
forgotten `int`: every value is accounted for by the signature or explicitly dropped.
Nothing runs behind your back, so the destructor fires exactly where you wrote the
disposal. That is the deterministic-drop property with no hidden control flow, and it
is a **linear** discipline (use exactly once), stricter than the affine (use at most
once, auto-drop) style of Rust or Hylo.

## Type system: deliberately small

The value is in a few sharp, cheap features, not in a research-grade type theory.

**In:**

- Checked stack effects (the compile-time virtual-stack pass, needed for codegen
  anyway).
- Concrete monomorphic types: the numeric tower, `bool`, fixed arrays, slices,
  string slices (`str`/`cstr`, see Memory model), records/structs, enums/ADTs.
- Enough parametric polymorphism to give `dup`/`swap`/`max` honest signatures:
  type variables (`'T`) and a row variable (`..s`, the rest of the stack). This is
  Kitten-style row polymorphism, kept minimal.
- The `Copy` marker distinction (copyable vs linear). This is the load-bearing
  bit for the memory model and must exist early.

**Out, and why:**

- **Full HM inference**: not required for a craft language. Annotate stack effects
  explicitly (they double as documentation and as a legibility win: nothing
  left implicit for a reader to infer). Keeping each stack slot as `(value, type)` from day one leaves
  the door open to add inference later without a data-structure rewrite, but it is
  not a goal.
- **Refinement types + SMT (Z3)**: dropped. Great for a safety product, far too
  much machinery (and a heavyweight solver dependency) for a hold-in-head craft
  language. Division-by-zero and bounds become ordinary runtime checks or `Result`
  returns.
- **Effect rows**: dropped as a core feature. Effects are not tracked in the type
  system. (Concurrency safety is recovered structurally instead, see below.)
- **Dependent types**: never. Research-level over a concatenative calculus, and
  the payoff doesn't fit a craft language.

## Memory model

Ownership + linear types, deterministic explicit drop, **no tracing GC**. Reference counting
is opt-in only (`Rc`/`Arc`-equivalent), reached for knowingly when shared ownership
is genuinely needed, because dropping the last ref cascades frees synchronously.

References are **second-class**, in the Hylo (mutable value semantics) mould, not
Rust's full borrow checker: refs can be passed into a word but cannot be stored and
cannot escape their scope. Because they can't escape, no *lifetime* system is
needed: no lifetime variables, no region annotations, nothing that binds a
reference's validity to a named scope. Lifetimes attach to named bindings; stack
values are anonymous and shuffled by `swap`/`rot`, so a lifetime system is the worst
possible fit here and stays deliberately avoided. Phase 3 Slice 6 shipped a narrower
rule instead: per-place exclusivity (at most one live mutable reference to a place),
checked at the point each place is consumed rather than by a liveness pass. That is
not a lifetime system — it never asks how long a reference is allowed to live, only
whether two live ones alias — and it works with none of the lifetime apparatus
because a reference already can't escape its creating scope. Affine values plus
non-escaping, exclusivity-checked refs give most of the safety with none of the
lifetime apparatus.

A `&!T` is consequently a third disposal category, neither `Copy` (so it cannot be
`dup`ed) nor linear (so it carries no `drop` obligation): it owns nothing, so a
reference-typed local simply expires, while a reference left on the stack is still a
surplus value. Exclusivity is also keyed per place rather than per path, so two
references into disjoint fields of one local conflict while both are live; sequencing
them is the workaround.

The rule that specifying references actually forced into the open is not about references
at all: **naming an aggregate does not copy it**, so two names can denote one region of
memory. That was invisible while nothing could mutate in place, and `!`/`+!` make it
observable. So taking a mutable reference to a place another live name denotes is an error,
and the remedy is `dup`. Two things follow, and both are deliberate. The error fires at the
*borrow*, never at the naming: two names for a value nothing mutates read identically, so
rejecting the naming would refuse correct programs. And no copy is ever inserted for the
programmer, even though that would be the friendlier fix, because `dup` is *the* explicit
copy in a language whose whole point is that copying and destruction are visible, and
because hard real-time here is carried by the programmer's own worst-case reasoning, which
requires instruction counts to be readable off the source. A compiler-inserted copy is the
same category of invisible behaviour as an auto-drop.

The rule is `Copy`-only by construction, which bounds how bad it ever was: every route to a
linear aggregate is closed independently, by move tracking, by the non-consuming peek's
refusal of a linear field, and by the standing ban on linear array elements. So the failure mode
was a wrong *value*, never a double free or a use-after-free, and the linear spine was never
at risk. It is still exactly the class of silent failure this language exists to turn into a
compile error, which is why it is closed rather than documented.

Because a branch merge can denote either arm's place, a value carries a *set* of regions
rather than one. The merge unions both arms, so no aliasing rejection ever happens at a
join: selecting one of two owned records takes no borrow and compiles, and the error waits
for the borrow, where the diagnostic can name both ends.

Pointers (`^T`) are non-null by default: there is no compiler-known optional/nullable
pointer type, now or planned before Phase 4's generics. Nullability, when a program
wants it, is an ordinary user-defined two-variant enum (a `Some`/`None`-shaped ADT falls
out of Slice 4's enums for free); a compiler-synthesized `Option` would be exactly the
throwaway machinery generics exist to replace, so it is never built. The return stack is
hidden or balance-checked; raw return addresses are never exposed.
FFI is the explicit unsafe hole, wrapped in safe words that establish invariants
(same discipline as Rust std over libc), and only exists at the hosted layer.

## Control flow and iteration

Boolean branching is `if ... else ... end` (which later becomes a combinator, see
below); structural dispatch is `match`. There are deliberately **no loop keywords** (no `begin/until`, `do/loop`);
dropping them keeps the surface small and matches the Factor/Kitten lineage, where
iteration is expressed with combinators rather than syntax.

The iteration story, top to bottom:

- **Quotations are the only iteration primitive.** A quotation `[ ... ]` is a
  first-class code value; `call` invokes it. This is the one piece that must be built
  into the language (new syntax + a value kind); it cannot be a library.
- **An internal loop primitive gives constant-stack iteration.** The IR has a
  loop/back-edge construct (not surface syntax, not user-facing). It is what makes
  iterating a large collection not overflow, independently of whether the TCO pass
  exists.
- **Combinators are library words, not keywords.** `each`, `map`, `filter`, `fold`,
  `while`, `times` are ordinary higher-order words that take quotations, written in
  Sooth on top of the loop primitive. A thin floor (one or two intrinsic combinators)
  bottoms out on the loop primitive; the rest are pure library. Reserving them as
  keywords would bloat the core for no reason.
- **Combinators are inlined.** The compiler inlines the common combinators and their
  quotation arguments at the call site, so `[ ... ] each` lowers to a tight loop with
  the body inlined, not a higher-order `call` per element. This is what makes "loops
  are a library" perform as well as loop syntax would have.
- **Raw recursion is legal but not the idiom.** A word may call itself; it is just a
  word. But threading the stack across a self-call by hand is fiddly, so combinators
  are the normal tool. Tail-call handling is therefore a convenience for hand-written
  recursion, not the lifeline for iteration (combinators over the loop primitive are
  that). Where it does apply it is a **guarantee, not a best-effort optimisation**: a
  self-call in tail position is compiled to a jump, so self-tail-recursion runs in
  constant stack (Scheme-style), and code may rely on it not overflowing.

Quotations and the combinator library land in Phase 4; but the internal loop/back-edge
primitive and the self-tail-call transform that sits on it are **brought forward to
Phase 2**, because the bytecode-VM dogfood (the Phase 2 exit) needs an overflow-free
dispatch loop and pulling quotations forward would be the larger change. Phases 0-1 and
the scalar/aggregate slices need only shallow recursion; the VM is the first golden that
needs real iteration.

**Phase 4 Slice 4 shipped the quotation literal and the `times` floor**, landing the first
bullet above narrower than the design implies and the second bullet exactly as designed. A
quotation stays a **compile-time-only marker** rather than a first-class runtime value
(D1): it carries the identity of its literal body, rides the checker's `Slot`/`Binding` and
lowering's phantom `Value` verbatim through shuffles and binds (D2), and is consumed only by
fusion — `call` splices a literal's body at the consumption site, type-checking identically
to writing the body inline, because there is no standalone effect to infer for a bare body
(D3). `call` therefore accepts only a statically-known literal; every position that would
need a real runtime value instead (a branch join, an array element, a user or polymorphic
word argument, an operator operand, a REPL residual stack) is a located rejection, not a
panic (D4). This defers the `Type`/`PolyType`/`IrType`/unification/mangling change a real
runtime quotation type implies to the slice that gives escaping quotations a consumer for
it (Phase 4 Slice 6). The floor is one intrinsic, `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )`,
passing the iteration index and requiring the body return the row it received, so effect
realization only ever checks an inner row against itself (D6); `while` was weighed as a
second floor member and declined, since its condition quotation returns a `bool` on a
passthrough row, strictly harder than `times` needs. `times` drives the existing internal
loop primitive (`begin_loop`/`finalize_loop`) unchanged, with a synthesized index; a `times`
nested inside another loop is rejected in the checker rather than taught to the lowering
primitive. The quotation-literal fusion this slice owns (splicing a literal's body at its
`call` or `times`) never crosses a `:` word boundary (D5); the interprocedural user-word
inliner that lowers the combinator library itself is Slice 5's.

**Phase 4 Slice 6a made a quotation nameable and shipped the inliner.** `Type`/`PolyType`
gain a `Quotation` variant carrying an interned declared effect (`[ 'T -- ]`), with
unification and `apply_subst` following, so a word may declare a quotation parameter and be
checked standalone against it — `IrType` gains no variant and there is still no "statically
known" bit on the type (D6): knownness stays a predicate on the value (`Slot.quot`), which is
what lets slice 7 later admit a genuine runtime closure without unpicking unification or the
monomorphization walk. `call`/`times` accept an *abstract* quotation typed only by a declared
parameter, beside the literal they already accepted; a literal passed to a declared
parameter is checked directionally against it, enforcing a `Copy`-only capture restriction on
what it may read from its defining scope. Combinators are now ordinary Sooth library words
(`lib/combinators.sth`'s `each`/`map`/`fold`), and every call to one is inlined by
term-splicing the callee's AST body against the caller's live stack — the compiler's only
inliner, generalizing slice 4's `call`/`times` fusion across a `:` boundary — transitively (a
combinator forwarding its own quotation parameter to a nested combinator splices through both
frames) and **totally**: with a quotation type but no runtime representation there is no
fallback, so anything un-inlinable, starting with recursion among quotation-taking words, is a
located error rather than a silent real call (D5). `each`/`map`/`fold` are leaf combinators
rather than `map` and `fold` being written over `each`, but that is a cost preference, not
an impossibility: `fold` and `map` over `each` are both expressible (the accumulator rides a
captured one-element array reached by balanced borrows, which D3 accepts). Because inlining
is total, library composition depth is code size at every call site, so building `map` on
`each` would make every `map` call site depth 2 plus an extra array copy and a counter cell,
where a leaf keeps the library flat at depth 1. "When to inline" becomes a real question
only at slice 7, when a runtime representation first makes a genuine choice possible; until
then "always" is the only implementable answer. The REPL
stays a located rejection, both at a session line defining a quotation-taking word and at an
imported closure exporting one (D7); 6c lifts it.

**Conditionals and dispatch.** Boolean branching is `if ... else ... end`. Structural
dispatch on ADTs is `match`, exhaustiveness-checked (a missing case is a compile
error). Multi-way branching is a **`cond` combinator** (a library word taking
`[ pred ] [ body ]` pairs), not syntax, so nested `if`s aren't the only option.
Haskell-style clause-based definitions with guards were considered and rejected: they
fit a stack language badly (Haskell matches named positional arguments, while Sooth's
inputs are anonymous stack values, the same named-vs-position tension that rules out
dependent types), and they replace the tiny `if` construct with a larger machine
(literal patterns + guards + clause sugar) without shrinking the language, since the
condition still has to be written somewhere.

**`if` becomes a combinator once quotations exist (Phase 4).** Phases 0 through
Slice 3 keep `if/else/then` as syntax (renamed `if/else/end` from Slice 4 onward)
because they predate quotations. Once quotations land, `if`
is redefined as an ordinary combinator (`cond [ then ] [ else ] if`, Factor-style) and
stops being a keyword. This shrinks the core the honest way, by making `if` a word
rather than by replacing it with a bigger feature.

## Modules and encapsulation

A file is a compilation unit (Phase 4 Slice 5a, native only; REPL imports are 5b).
`import: q "path.sth" ;` binds a qualifier to another file, resolved relative to the
*importing* file, with an explicit `.sth` and no search path (consistent with
`extern:` naming its C symbol verbatim: no implicit extension is one fewer resolution
rule to learn). `export: name... ;` (lines accumulate) is the only way a name leaves
its file; a module with none exports nothing, so every pre-5a example is unaffected
— it exports nothing and stays a program, not a library.

**Why resolution has to happen as one merged pass, not a parse-then-merge.** The
parser resolves every type name in a pre-pass over raw tokens *before any word body
parses* (`prepass_type_decls`, then `build_registries`, both inside `parse`). An
importing file's own pre-pass needs the imported file's type names present before its
bodies can parse at all, so parsing each file independently and merging the two ASTs
afterward would mean remapping every positional `StructId`/`EnumId`/`ArrayId` in the
second file's already-parsed tree — strictly more work, and more places to get it
wrong, than doing it once. The model instead: resolve the import graph from the entry
file, canonicalize and dedupe by path (a diamond import is parsed once), order it
topologically and reject a cycle or self-import with a located error naming both
files, then run **one shared pre-pass** across the whole closure's tokens into **one
shared registry set**, and only then parse bodies per file against that shared set.
The closure still assembles into one `Module`, so `check::check` keeps its
single-module signature; module identity rides on a per-decl owning-module tag, not on
threading multiple `Module`s through the pipeline.

**Name resolution is "own module first, then qualifier," not filtering at merge
time.** The registry stores a bare name plus its owning module (rather than, say, a
fully-qualified stored name), so an unqualified reference resolves in its own module
first and a `q::base` splits on the qualifier, maps `q` through the current module's
import table, and resolves in the target module subject to its export list. Every
module's names are spliced into one shared environment and *marked* with their module
and export status; rejecting an unqualified-but-private reference happens at the use
site, never by hiding the name at merge time — filtering there would collapse two
distinct failure modes (a name that exists but is private, vs. a name that is simply
absent) into one `unknown word`, which is a worse diagnostic for a language that
otherwise turns Forth's silent failures into sharp errors. Two modules may each
declare `Point`; the duplicate-type-name check is per-module, not global. Same-named
words in two modules mint distinct emitted symbols via a module-disambiguating
component, added to `instantiation_symbol` the same way its `generation` suffix
already is, so `::` never has to survive to the symbol sanitizer and a single-module
closure (every pre-5a program, every REPL session) is byte-for-byte unchanged.

**A `type:` declaration is a name-scope, and visibility is the ordinary export
mechanism applied to it, not a special rule for types.** Its generated words
(constructor, getter, peek, setter, destructure) are literally named by string
concatenation (`Type>field` and siblings) — an ad-hoc qualified namespace the compiler
already builds for every struct. Exporting `Type` therefore exports that whole
name-scope as one unit: naming a type in `export:` is **transparent**, with no opacity
mechanism and no per-member withholding in this slice. A consumer may name `q::Type`
in an effect, construct one, and reach every field through `q::Type>field` /
`q::Type<field` / `q::Type|>field` (each resolved by splitting on the *first* `::`,
since `>` is not a lexer delimiter and the whole qualified accessor is one token).

This was a reversal mid-design, not the obvious choice: the first draft made export
opaque by default, Elm-style, distinguishing "export the type" from "export its
constructor." It didn't survive contact with what Sooth actually is. Structs are dumb
data; a violated field invariant is a bug in the *consumer's* program, not unsoundness,
because there is no UB, indexing traps at the bound, and linearity already prevents
aliasing a value into two invariant-breaking places at once. And the resource argument
for opacity fails on a fact measurable in the single-file compiler today: destructuring
a type with a `drop` override already skips that override (`type: R tag i64 ;` with a
`drop` override, then `r R>tag .`, prints the tag and never runs the destructor) — so
visibility was never actually protecting resource discipline, opaque or not. Hiding an
accessor behind a visibility rule is the OOP ceremony this language is declining to
need; a withhold marker on `export:` is an additive feature for a real consumer that
wants it, not a default this slice should guess at.

That destructure-bypasses-`drop` gap is real and becomes newly *reachable* across a
file boundary once types are transparent — a library consumer can now destructure an
imported linear type down to `Copy` leaves without running its destructor, the same way
a single-file program already could. It is not a new hole class, and this slice does
not half-fix it with a partial guard: the honest fix is a Rust-E0509-style rule
(reject destructuring a type that has a `drop` override) belonging to the ownership
checker, independent of modules, and it is recorded against Phase 4 Slice 8 rather than
grown here.

**Disposal crosses the export boundary for free, so this slice adds no new disposal
rule.** `drop` is compiler-known and dispatches on the concrete type (Slice 3/8b), so a
consumer disposes an imported linear value with a bare `drop` whichever destructor glue
runs whether or not that glue was named in `export:` — "a destructor runs without being
named." Combined with transparent types' second route (destructuring to `Copy` leaves),
no case in this slice lets a consumer hold an undisposable imported value, so the
ROADMAP's hypothesized "an exported linear type must also export its discharging word"
rule has nothing to fire on yet. It only becomes a live question once a polymorphic
`drop ( 'T -- )` could be structurally total — exactly what Slice 8's own constraint
forbids — so enforcement is deferred there, not decided here.

**Declaration-site and selective-import rules round out encapsulation.** An exported
word whose stack effect names a private, non-primitive type of its own module is
rejected at the `export:` declaration itself (the module author's bug, not the
consumer's), naming the word and the private type; exporting the type satisfies it.
Selective import, `import: q | a b | "path.sth" ;`, is additive to the qualifier: `q`
is always bound, and the listed names are *additionally* exposed unqualified (a
selectively-imported type brings its generated words unqualified too, one unit as
ever). The collision rule is deliberately dumb, with no precedence and no use-site
disambiguation: two selective imports exposing the same unqualified name is an error at
the second, naming both modules, and a selectively-exposed name colliding with a local
definition is the same error.

**REPL imports (Slice 5b) answer what an import means in a live session by treating it
as an ordinary redefinition event applied to a batch of names, not a new rule.** The
REPL's own top-level `import:` path resolves relative to the *process current working
directory* (every transitive import inside the discovered closure keeps 5a's
importer-relative rule unchanged, so exactly one frame of reference is new). Each
`import:` line mints one fresh, session-wide import epoch and recompiles every word in
the closure under it, exactly the way redefining an ordinary word mints a fresh
generation every time: a caller already compiled against the old epoch stays frozen
under `RTLD_GLOBAL`, while a fresh reference resolves against the new one. Splicing the
closure into the session's flat, positionally-indexed registries (`StructId = index` and
siblings) remaps every type id it carries from closure-local to session indices — the
remap a fresh native compile never needs, since it never has an already-populated
registry to append onto. Transitive re-export stays closed at the REPL exactly as
natively: a third file imported by the imported file contributes no session-visible
name. Registry growth on re-import is accepted, not deduplicated or capped, matching a
redefined word's fresh generation every time. An imported file declaring `main` is a
located rejection naming the file and the word, at import time — the native path's own
exposure to the same collision (recon #4, ROADMAP) stays unfixed. `export:` as a REPL
line is its own located rejection: a live session has no export boundary to cross.

Out of scope for this slice, all deferred to Phase 6's eventual package/versioning
layer or later: a serializable API description and semver enforcement (which will
consume this slice's export list, not redefine it), package manifests and a registry,
re-exports or aliasing an import to a different local qualifier, a `mod.sth`-style
directory-mirrors-module-tree convention (declined: flat file-is-a-module plus
qualified access covers the only consumer that exists), and generic type declarations
crossing files (they don't exist yet).

## Codegen and backend

Codegen model (unchanged from first principles, it's the good part): don't model
the data stack at runtime. Simulate it at compile time as an array of typed slots;
push/pop manipulate the array, and when IR is emitted the slots become ordinary
SSA/register values. Each word compiles to a function taking N stack args and
returning M results. `if`/`end` become basic blocks and branches; there are no loop
keywords (see Control flow and iteration), and iteration lowers to an internal loop
primitive with a back-edge. Branch and loop join points unify the virtual-stack state
(depth and type) across predecessors; mismatched depth or type across arms is a
compile error.

**No LLVM, and not a hand-written backend either. Decided: QBE.** The joy in this
project is the language and writing programs in it, not emitting machine code, so
codegen is offloaded to the smallest backend that stays legible. QBE (~15k lines you
could actually read) gives arm64/x86_64/riscv64 plus C-ABI struct classification for
free, and can carry essentially the entire design: everything interesting (linear
analysis, monomorphisation of the small polymorphic core, deterministic drop) is
frontend/runtime work QBE is agnostic to. A hand-written native backend (own the
vertical, direct syscalls) was the craft-purist alternative, set aside for
now, and reconsidered after self-hosting, because it optimises for building the
compiler, which isn't the point at this stage. LLVM was
rejected outright: too large and opaque for a hold-in-head project, a perpetual
dependency tax, and product-grade output the language doesn't need. Wanting LLVM's
full-service codegen is a tell that the project has drifted back to product-think,
where the honest answer was "use Rust."

**RISC-V 32 is a committed eventual target; the backend to reach it is deferred to
post-bootstrap.** QBE gives arm64/x86_64/riscv64, but has no rv32 target (and assumes a
64-bit machine word in places), so emitting rv32 will mean either patching an rv32 target
into QBE or the hand-written backend, a call taken after the language self-hosts, consistent
with the "reconsidered after self-hosting" stance above. The commitment is recorded now not
to build anything, but so the frontend stops accruing 64-bit assumptions before then (see
next).

QBE's costs, accepted: it emits assembly text, so you depend on the system assembler

- linker (a cross-toolchain + sysroot when cross-compiling the hosted layer);
atomics lower via FFI to C11 atomics rather than a QBE primitive. Its modest optimiser is a feature, not a
bug: more predictable than LLVM's aggressive passes and friendlier to any later WCET
work.

**WASM is a sibling lowering, not routed through QBE.** Sooth's IR is already
stack-shaped with structured control flow, exactly what WASM wants, so WASM hangs off
the neutral IR in parallel to QBE (emit WASM, hand to binaryen for optimisation),
never downstream of it (going through QBE would flatten the stack/structured-control
shape only to rebuild it with a relooper). The "uxn that grew up" target: portable,
AOT-to-native via wasm2c when a native binary is wanted.

**Enabling decision, load-bearing from Phase 2:** keep pointer size and memory model
abstract in the IR (`Ptr[T]` is an opaque handle, not a native `u64`), so the QBE
(native pointers) and WASM (linear-memory offsets) lowerings each concretise it. A
native-pointer assumption leaking into shared IR is the one thing that makes WASM
chafe later.

The same rule extends to integer width: **the IR never assumes a 64-bit machine word.**
Word, pointer, and `usize`/`isize` width are a target parameter, not a constant, exactly as
`Ptr[T]` is opaque. This is not abstract tidiness: the committed rv32 target (above) has
32-bit pointers and makes `i64`/`u64` double-word there (synthesised as register pairs in
the frontend), so `usize` is genuinely 32-bit there. `usize` is a target-width type
introduced with fixed-size arrays (Phase 2, Slice 5), where indexing is its first real
consumer; `isize` mirrors it but waited for Phase 3 Slice 3, since it had no consumer
until recursive/heap data existed. Both resolve to 64-bit on current targets but must
never be *assumed* 64-bit in shared IR. A corollary worth revisiting under a 32-bit target: the
current "integer literals default to `i64`" stance is 64-bit-centric, since on rv32 the
natural machine word is 32-bit, not `i64`.

**Dropping LLVM means no in-process JIT, and it turns out to cost nothing.** LLVM's
ORC would have let the REPL and a compile-time evaluator share one native engine. Two
decisions remove the need for it outright:

- **No compile-time execution.** There is no immediate-word / macro facility (see
  Declined), so nothing runs Sooth at compile time and there is no comptime
  interpreter to build.
- **The REPL runs on the backend, not an interpreter.** Each new word is compiled
  through the normal pipeline to a shared object and `dlopen`'d into the session, so
  the process holds live, natively-compiled code it can call at once; redefinition
  loads a new object and swaps the name→symbol entry. Whole-program `run`/watch takes
  the simpler compile-to-binary + subprocess path. One execution semantics either way,
  so the live loop exercises the real backend with nothing to keep in sync. This is
  Factor's in-image model minus the sub-millisecond per-word compile: an assembler +
  linker + load round-trip per definition, higher latency than a JIT, acceptable for
  craft. Sub-millisecond would require owning a backend (see Open / deferred); not now.

**The REPL's stack buffer is a driver artifact, not a runtime-stack feature.** Word
bodies still compute entirely in SSA/registers; the compile-time-virtual-stack
invariant is untouched. The buffer exists only to bridge separately-compiled `.so`
units: each bare expression line is a wrapper that loads the whole carried stack from
a `Vec<i64>`, runs the line's body in registers exactly like a word, and stores the
result back. This is also a deliberate preview of the **uniform runtime stack**
reserved for escaping quotations (Phase 4): the same "marshal to/from a byte buffer
at a compiled boundary" shape, reused there for closures that must cross into `alloc`
rather than for a REPL line. Neither case puts a runtime stack inside a word body.

## Concurrency: a library, not a core feature

Only two things must be core intrinsics; everything else is a library.

**Core intrinsics (cannot be synthesised from below):**

- **Atomics + memory ordering** (compare-and-swap, acquire/release). Codegen must
  respect them as barriers. On the from-scratch backend, emit LL/SC (arm64) or
  `LOCK`/CAS (x86) directly; on QBE, lean on FFI to C11 atomics or hand-written asm
  (QBE's atomics story is thin).
- **A spawn primitive.** Can be a thin FFI to `pthread_create` at the hosted layer
  rather than a language feature.

**Free from the linear spine:** data-race freedom. Sending a message *moves* the
payload (the same `dup`-is-the-copy rule across a thread boundary), so there is no
shared mutable aliasing by construction. Second-class refs can't cross a thread
boundary because they can't escape a scope at all, so the dangerous case is already
forbidden by machinery built for resources. No separate `Send`/`Sync` apparatus
needed.

**Library (the whole model as the user sees it):** channels, mutexes, condvars,
pools, futures, and actors (a mailbox + a loop + move-only messages). Split-endpoint
channels so nothing needs to be `dup`ed:

```forth
\ intrinsics: spawn ( q -- Thread ), cas ( p a b -- bool ). the rest is library.

: worker ( Recv[Job] -- )
  | ch |
  begin
    ch recv            \ ( -- ch Job )  ownership of the Job MOVES to us
    handle
  again ;

: main ( -- )
  chan                          \ ( -- Send[Job] Recv[Job] )  two linear ends
  swap [ worker ] spawn drop    \ Recv end moves into the spawned quotation
  ... ;                         \ Send end stays here, still owned, race-free
```

Note `spawn [ ... ]` uses an escaping quotation, so the convenient concurrency
library is an alloc/hosted citizen. A `no_std`/real-time concurrency library is a
distinct, more constrained one (static topology, fixed mailboxes, no escaping
captures). Concurrency-as-a-library is two libraries, not one.

## Real-time: capable, not guaranteed

The core is a strong real-time foundation, better than most, because the two hardest
RT properties come for free: no GC (no stop-the-world pauses) and deterministic drop
(destruction at a statically-known time). Errors-as-values means no unwind-path
jitter either.

What was *not* built is the RT *guarantee* machinery:

- **Soft real-time** (audio, games, robotics tolerant of occasional misses) works
  essentially out of the box.
- **Hard real-time** (deadline miss = failure) is achievable by discipline, not
  turnkey. You add a heap-free layer, a bounded-mailbox/static-topology concurrency
  library, and carry the WCET reasoning yourself. The enemies to keep off hot paths
  are dynamic dispatch through escaping quotations and spawning during the loop;
  the fix is static topology (spawn at init). Nothing in the design fights this; it
  just isn't enforced by a type/effect system the way a product version would.

The RT-safe subset is exactly: core + the fixed (no-alloc) layer + the no-escape
concurrency library.

## `no_std` core and layering

The core is `no_std` and everything else layers on top. This is the honest shape of
the language, not a concession: a tiny `no_std` core is both the hold-in-head craft
object and the thing that runs on a microcontroller with no OS. The hosted layer
(files, threads) is the optional convenience, not the foundation.

```
core       stack semantics, numeric tower, bool, fixed arrays + slices,
           string slices (rodata), linear/move/drop, checked stack effects,
           control flow, modules, non-escaping quotations, atomics,
           second-class refs, and the allocator *interface* (not an impl).
           assumes: a few compiler intrinsics, no allocator, no OS.

fixed      fixed-capacity vec/map/string/ringbuffer, bounded mailboxes.
(no-alloc) allocation-free, real-time-safe. the embedded sweet spot.

alloc      growable Vec/Map/String, Box, opt-in Rc/Arc, escaping closures,
           bignum. needs an allocator satisfying core's interface.

hosted     files, stdio, time, net, OS thread spawning, FFI-to-libc,
           blocking channels. needs an OS.
```

Placements worth noting: atomics are core but spawning threads is hosted; string
slice is core but growable `String` is `alloc`; a non-escaping quotation is core but
an escaping one is `alloc`; the allocator *interface* is core, its *implementations*
(arena, pool, malloc) are not.

Discipline: fix the layer boundaries and the allocator interface on day one and tag
every stdlib word with the layer it needs, even though the hosted layer is built
first (that's where dogfooding happens). Carving `no_std` out later is the retrofit
tax Rust paid early; avoid it. And `no_std` core is not "runs on nothing": it still
assumes a handful of intrinsics (memcpy/memset for moves, integer-divide and
soft-float helpers where there's no hardware, the atomics) plus a per-target linker
script and entry point.

## Liveness and the craft discipline

The failure mode with this project's name on it is a beautiful half-built compiler
that no one ever writes a program in, because building the compiler is more
immediately rewarding than using the language. The antidote is a hard requirement:

- A REPL / immediate feedback from day one.
- Write real small programs in Sooth (a raymarcher, a text adventure, the classic
  demos, an LED blink on bare metal) *before* the compiler is "done."

If it's fun to play with early, it gets finished. If it's only fun to build, it
won't.

## Bootstrap and implementation

Self-hosting is a milestone, not a day-one constraint (rustc/Go/TypeScript
precedent): thick host-language compiler first, rewrite in a defined Sooth subset
later, verify by fixpoint.

Dropping LLVM and Z3 removes the constraints that previously forced the host
language, so the choice is now preference, not necessity. **Rust remains the
sensible default** (ADT + pattern-matching-heavy compiler workload, and `no_std`
for the runtime/intrinsics library in the same language), but nothing now *requires*
it. Keep the bootstrap clean and un-clever so it translates to the self-hosting
subset rather than idiomatic host-language code with no analog.

The self-hosting subset is smaller than before precisely because the language is
smaller: concrete monomorphic types + ADTs + pattern matching, growable collections

- strings, words + modules, errors as values, and a modest C FFI (now only for the
OS/hosted layer, not for a solver or LLVM). No inference, no refinements, no effect
rows, no borrow analysis needed to write the compiler in it.

## Decided

- Scope: craft language, not a product. Optimise for legibility, hold-in-head size,
  and the joy of building and writing it.
- Signature idea: linear (use exactly once) by default, `dup` is the explicit copy,
  `drop` is the explicit destructor point the checker enforces.
- Surface: concatenative, Forth-lineage, checked stack effects, `| named locals |`.
- Control flow: `if/else/end` for boolean branching (becomes an ordinary combinator,
  `cond [ then ] [ else ] if`, once quotations land in Phase 4); `match` for exhaustive
  structural dispatch on ADTs; a `cond` combinator (library word) for multi-way
  branching. No clause-based definitions (bad fit for a stack language). No loop
  keywords.
- Iteration: quotations (`[ ]` + `call`) are the sole primitive; lowers to an internal
  loop primitive for constant stack; combinators (`each`/`while`/`fold`/`times`/`map`)
  are library words built on quotations and inlined at call sites. Raw recursion is
  legal but not the idiom. Self-tail-recursion is a guaranteed constant-stack transform
  (tail self-call → jump), implemented in Phase 2; mutual TCO is deferred (SCC
  contraction, not a trampoline). A loop-carried aggregate gets an entry-hoisted stable
  slot with a read-before-write staged move-blit on the back-edge, no header phi.
- Type system: small. Concrete types + ADTs + minimal row polymorphism + a `Copy`
  marker. No full HM inference, no refinement/SMT, no effect rows, no dependent
  types.
- Memory: ownership + linear types, deterministic explicit drop, no GC, RC opt-in (deferred to
  the `alloc` layer, Phase 6); second-class refs (Hylo-style), no lifetime-tracking borrow
  checker; non-null pointers; hidden/checked return stack.
- Strings: two types, taking Zig's *split* (a length-carrying view plus a bare C pointer) but not
  its sentinel-in-the-type. `str` is pointer + length and promises nothing about `byte[len]`, so
  Sooth code always reads the length and never scans. `cstr` is pointer-only with an unknown
  length, which is what C hands back. `str` -> `cstr` is free *for a literal*, whose lowering
  emits an uncounted NUL, and is not free in general: `core` has no allocator to copy with, so a
  caller that owns a buffer writes the terminator itself. `cstr` -> `str` costs an explicit scan.
  A whole-type NUL guarantee was rejected because a view over part of a buffer could never uphold
  it, and an invariant a later slice must revoke is worse than one never claimed. Slicing a buffer
  into a view is deferred, see Open / deferred.
- A user-supplied destructor is an overload of `drop` for a concrete type, not a new
  declaration form, and defining one *forces* that type linear regardless of what its fields
  would otherwise imply. `Copy` and a user destructor are mutually exclusive, for the reason
  Rust makes them so (E0184): a `Copy` type could be duplicated and each copy discarded, so a
  destructive body would run more than once for one logical resource. The body runs *instead
  of* the synthesized field glue, never before or alongside it, because "nothing auto-drops"
  already makes the body answerable for its own fields via the ordinary must-consume rule.
- Foreign calls: one typed declaration form (`extern:`, a symbol plus a stack effect) rather
  than per-call compiler builtins or an untyped generic syscall word. A raw syscall word is
  ruled out: it would force `Ptr[T]` to become an integer, breaking the backend-neutral
  invariant the WASM lowering depends on, and syscall numbers are neither OS- nor
  arch-portable. Scalars and references may cross; owned aggregates and `^` returns may not.
  The declaration site is itself the trust boundary, so there is no separate `unsafe` marker.
- Codegen: compile-time virtual stack to native; words as functions.
- Backend: QBE (small, legible, multi-arch native + C ABI for free); no LLVM. Owning a
  hand-written native backend is deferred, not ruled out (the joy is the language, not
  codegen; reconsider after self-hosting, see Open / deferred). WASM is a sibling
  lowering off the neutral IR via binaryen, not routed through QBE. IR keeps `Ptr[T]`
  abstract so both lowerings concretise it. No in-process JIT and no comptime
  interpreter: the REPL loads freshly compiled words in-process via `dlopen`;
  whole-program run uses compile-to-binary + subprocess.
- Errors as values, no THROW/CATCH, no unwinding.
- Concurrency: library, not core. Only atomics + spawn are intrinsics; data-race
  freedom is free from linear types + non-escaping refs.
- Real-time: soft-RT out of the box; hard-RT by discipline (fixed layer + static
  topology), not by enforced guarantee.
- `no_std` core with core / fixed / alloc / hosted layering; allocator interface in
  core; seams fixed day one.
- Bootstrap: host-language compiler then self-host a small subset, fixpoint-verify.
  Host language now free choice; Rust the sensible default.

## Open / deferred

- Exact surface syntax for quotations/closures and their captures (illustrative
  above, not settled).
- Whether to add optional HM inference later (kept possible by the `(value, type)`
  slot representation, not planned).
- **Mutual tail-call elimination (tier 2).** Self-tail-call → loop lands in Phase 2
  (see Control flow and iteration); *mutual* tail recursion (a tail-call cycle A→B→A)
  stays deferred. When taken, the mechanism is **not a trampoline** (a real trampoline
  needs first-class function values / quotations, which are Phase 4) and **not** QBE
  backend tail calls (QBE has none, and adding them forks the backend we chose not to
  fork). It is **strongly-connected-component contraction**: detect the SCC of the
  tail-call graph, merge its members into one function carrying a state tag (which
  member we are in, an ordinary enum discriminant) plus the union of their live values,
  lower every intra-SCC tail call as a back-edge jump, and keep thin public wrappers so
  members stay callable from outside. Constrained to SCCs whose members share a return
  signature (a divergent return type would want a result union, i.e. generics, Phase 4).
  Until then, mutual tail recursion is a located compile error, not a silent overflow.
- **Drop at the back-edge (co-design with deterministic drop).** The self-tail-call
  transform is the point where the outgoing iteration's linear values that are *not*
  forwarded must be dropped before the jump. In Phase 2 every type is `Copy`, so that
  drop set is empty and the concern is vacuous; the back-edge is the **defined disposal
  point**, so it has a home when a later Phase 3 slice lets a linear value ride a loop
  (Phase 3 Slice 1 defers loop-carried linear values).
- **Slicing a buffer into a view.** A literal-rooted `str` points at static data and cannot
  dangle, so it is unrestricted. A view over a heap `^[u8 N]` or a local buffer *is* a borrow,
  and unrestricted it would bypass the escape rules entirely because it is not spelled `&`. The
  objection that killed the earlier sketch was that restricting by *provenance* would leave a
  `( str -- )` signature no longer saying which kind it holds, and honest signatures are what the
  no-lifetimes bet trades on. A separate problem the type-predicate story above says nothing
  about: a `str`'s `{bytes_ptr, len}` descriptor is static data `emit_str_literal`
  (`src/backend/qbe.rs`) emits per literal, so a borrowed or sliced view cannot reuse that
  representation without materializing a descriptor at runtime (e.g. onto a stack slot), a
  representation question on top of the predicate one. That objection is answered by putting
  the rooting in the **spelling**: `str` is the static view, `&str` / `&[u8]` / `&![u8]` are
  borrowed ones, a leading `&` meaning exactly what `contains_reference` already reports, and a
  static view coercing one-way into a borrowed position (`'static: 'a` collapsed to two points, which would be the only
  subtyping in the language). For `contains_reference` itself, no new checks fall out: "is
  borrowed" as that answer routes a borrowed view through every existing no-stored-reference
  position phrased over it, and "is shared" as
  the `is_copy` answer is the rule `Type::Ref` already uses. `cstr` needs the same bit, or it
  becomes a side door that launders a borrow into an escapable pointer. But `is_copy` is not the
  last knob: `is_linear` is `!ty.is_ref() && !is_copy(...)` (`check.rs`), and `Type::is_ref` is
  `matches!(self, Type::Ref(..))` (`ast.rs`), so a borrowed view spelled as a new variant answering
  only `is_copy = false` would be classified linear and acquire a drop obligation, exactly the
  third disposal category this entry says a borrow must not have. `is_ref` is a third required
  answer, alongside `contains_reference` and `is_copy`. What stays true is that a borrowed view
  **cannot be returned**: the no-declared-output-reference rule is precisely what keeps a
  two-point lattice from having to grow into lifetime variables, so a word handing a region back
  returns indices instead. Still deferred until a real client pushes on it. The likely client is
  Phase 9's self-hosted lexer, which wants byte offsets for diagnostics anyway, and is also where
  the evidence would exist to justify whatever it costs.
- **`.` appending no separator, for every type (decided, not yet implemented).** Today `.` appends
  a trailing newline for every type except `str`/`cstr` (slice 8a's R9). The decision is to make it
  uniform the other way: `.` writes exactly the value and nothing else, a newline spelled
  explicitly by the caller, e.g. `: println ( i64 -- ) . "\n" . ;`. Consequence: `1 . 2 .` then prints
  `12`, callers supplying every separator, not just newlines. Amends Phase 0's definition of `.`
  (`docs/phase0-spec.md` defines it as `printf("%ld\n", …)` in three places) and touches
  ~130 stdout assertions across five test files (`assert_eq!(stdout` count: 91 in
  `tests/phase0.rs`, 24 in `tests/phase3_refs.rs`, 9 in `tests/phase3_strings.rs`, 6 in
  `tests/phase3_locals.rs`, plus roughly 22 REPL-session assertions in `tests/phase1.rs`), plus
  the backend's own format-string unit tests (`src/backend/qbe.rs`), so it lands as its own
  scoped change, not folded into slice 8a. A single `print` covering every type needs Phase 4's
  static overloading; until then it is one wrapper word per type, e.g. `: println ( i64 -- ) .
  "\n" . ;`, or an explicit `"\n" .` at each call site — and the wrapper is only expressible
  from slice 8a onward, since it needs a string literal.
- Owning a native backend (a hand-written machine-code emitter replacing QBE's
  text-assembly path). Not now: the joy is the language, not codegen, and QBE plus
  `dlopen` cover native output and a live REPL without it. Reconsider after
  self-hosting, and only if the pull to own the vertical is genuine or a
  sub-millisecond in-image REPL is something you actually want; if taken, it is its
  own phase, never welded to the self-hosting rewrite.
- **REPL late binding for redefinition.** Every REPL word today is frozen at whichever
  generation of its callees existed when it was compiled: redefining `f` after `g`
  already calls it leaves `g` calling the old `f` forever, verified even across a
  signature-incompatible redefinition (`f` going from `( -- i64 )` to `( -- bool )`
  does not perturb an already-compiled `g`). This is not a chosen UX principle, it
  falls straight out of the architecture: each line compiles once to native code via
  `dlopen` (no in-process JIT, see Decided), calls are direct and baked at the calling
  line's compile time, and nothing ever recompiles an earlier line. It also matches
  Forth's own long-standing convention (a colon-definition compiles its calls to fixed
  execution tokens; redefining a word a later definition already calls does not
  retroactively change that definition), which is Sooth's actual reference class, not
  the late-bound convention of Python/Lisp/JS REPLs, where redefining a helper
  immediately updates every existing caller. Phase 4 Slice 2 (REPL monomorphization)
  surfaced the question by needing to decide which env a polymorphic word's
  instantiation binds its callees against, and kept the existing frozen rule there for
  consistency with ordinary words rather than deciding the bigger question in passing.
  Genuine late binding needs every call to go through a mutable dispatch slot that
  redefinition updates, rather than a direct symbol reference, for every word, not just
  polymorphic ones (doing it only for generics would make a caller's behavior on
  redefinition depend on whether the callee happens to be generic, a worse
  inconsistency than either uniform choice), and it is a breaking change to already-
  shipped, already-tested ordinary-word REPL semantics. Revisit only if live-patching is
  something actually wanted, as its own design track with its own brief, not as a side
  effect of a generics or monomorphization slice. Import reload (Slice 5b) rides this
  same frozen-generation rule rather than reopening it: a re-run `import:` line mints a
  fresh epoch and recompiles every word in the closure under it, but an already-compiled
  caller stays exactly as frozen as it would after any other word's redefinition.

- **Accessors as lenses: separate the location from the operation.** Today a struct field
  access bakes the type, the field, and the ownership semantics into one generated word
  name, built by string concatenation: `format!("{}>{}")` for get/destructure,
  `"{}<{}"` for drop-on-overwrite, `"{}|>{}"` for the non-consuming `Copy` peek. Arrays
  already work the other way: `arr_ref idx &>` is receiver, selector, operation, with the
  selector an ordinary runtime value. The direction is to make structs match, so
  `q buf &>` reads like `l 0 &>`.

  **The strongest argument is the factoring, not the syntax.** `>` / `<` / `|>` conflate
  *which field* with *what ownership transfer happens*. Lenses put ownership in the
  operation and location in the selector, which for a language whose point is an explicit
  linear spine is the more principled decomposition. It also collapses generated words from
  O(fields x operations) to O(fields + operations), which is the same problem the module
  export list runs into from a different direction (listing three words per field to control
  visibility), and two independent routes to one root cause is a signal it is real. A
  further benefit: with few explicit operations, the destructure-bypasses-`drop` hole
  (recorded on ROADMAP Phase 4 slice 8) becomes one rule about one operation rather than a
  property spread across three generated words per field.

  **The hard problem is heterogeneity.** An array is homogeneous, so its index does not
  affect the result type; a struct's fields have different types, so the selector
  *determines* it. Two coherent designs follow. A **compile-time selector marker** (no
  runtime representation, identity on a side channel, consumed only at projection sites,
  default-denied elsewhere) is exactly the machinery Phase 4 slice 4 built for quotations,
  so its cost is known, but it buys no composition and is close to alternative syntax for
  what exists. A **first-class lens** (`buf : Lens['S 'A]`, with
  `&> ( &'S Lens['S 'A] -- &'A )`) is expressible once type variables exist, is cheap at
  runtime since a lens is a field offset, and buys real composition of paths. The tension to
  resolve: composition needs first-class selectors, first-class selectors need unambiguous
  names, unambiguous names need qualification, and qualification undoes the terseness that
  motivated the change. Pick two.

  **Placement.** Needs one `&>` to accept both arrays and structs, i.e. static overloading
  (Phase 4 slice 8), so it cannot precede that. It belongs *before* the stdlib is written,
  for the same reason modules did: writing `Vec`/`Map`/`String` against the old accessors
  and migrating them afterwards is the waste. Filed as a Phase 6 prerequisite item rather
  than its own phase, since it is a surface-syntax direction plus a corpus migration rather
  than a theme on the scale of the other phases. Not settled; the selector-representation
  question above is genuinely open.

## Declined

Recorded so it isn't relitigated. Revisit only against a concrete program that can't
be written without it, never on principle.

- **Immediate-word / defining-word / macro facility** (would have replaced Forth's
  `CREATE`/`DOES>`). Declined. Sooth already sent the non-metaprogramming uses of
  Forth immediate words elsewhere (`if/else/end` is a keyword, then a combinator;
  iteration is quotations + combinators; comments/strings are lexer-level), leaving
  only metaprogramming, which splits into two capabilities both covered without a
  comptime facility: defining new words is a plain nullary word (`: answer 42 ;` is
  already a constant) plus generics, or an external build-time `.sth` generator for
  large families; baking a computed value into the binary is external codegen or
  runtime init. Declining it also keeps one "when does code run" story, with no
  compile-time phase in the user's model. **Revisit if** a bare-metal / no-allocator
  target (Phase 8) needs precomputed tables it genuinely can't build at startup; the
  answer then is a minimal comptime const-eval (a foldable-pure-word evaluator or a
  build-emitted data section), not a macro system and not an interpreter.
- **Open multimethods** (`generic:`/`method:` on a sum). Declined. The expression
  problem — extending both types and operations without modifying existing code — is
  real, but open multimethods solve it at a cost that cuts against the language's core
  property: dispatch resolution that depends on which modules are loaded is behaviour
  coming from "who knows where," exactly the kind of implicit magic the linear spine
  exists to eliminate. The exhaustiveness checking that closed match gives you is a
  feature, not a tax: the compiler tells you every place a new variant is unhandled,
  and that safety is worth more than the convenience of adding arms without touching
  existing code. With Phase 4 Slice 6 (functions as values) landing first, the
  handler-struct convention — a struct of quotations, one instance per variant,\   selected by a closed match — covers the extensible-types dimension using existing
  language features, with exhaustiveness intact and no runtime dispatch table. The
  expression problem's second dimension (adding new operations without touching
  existing constructors) stays closed under that convention, but for a single-author
  codebase updating constructors is a safety check, not a tax, because the compiler
  names every site that needs it. Dropping open multimethods also keeps Elm-style
  enforced semver fully sound: without orphan arms there is no scenario where a MINOR
  addition silently shifts dispatch resolution for existing callers. **Revisit if** a
  concrete program needs a third party to extend operations on a type they don't own
  without modifying the type's defining module, and the handler-struct convention is
  genuinely insufficient — but that is a plugin-system requirement, not a personal-product
  one.
