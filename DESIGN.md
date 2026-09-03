# Sooth — design notes

A small, statically-checked concatenative language compiled straight to native
code with no external runtime. Working notes,not a spec.

## What this is: a craft language

Sooth is built for the pleasure of building and writing it, not for a market. The
consequences run through every choice here: the language stays small enough to hold
in one head, the compiler stays small and legible, and its one backend dependency
(QBE) is itself small enough to read rather than an opaque black box. Where a
decision trades reach or peak performance for simplicity and legibility,
simplicity wins.

The one intellectual bet that makes Sooth more than a tidy Forth clone: **in a
stack language the stack discipline already is move semantics.** Every word
consumes its inputs and produces its outputs, so a value is moved by default and
keeping a copy requires an explicit `dup`. Linear types therefore fall out for
free: `dup` is the explicit copy, and `drop` is the explicit, checked destructor
point. That single idea pays off three times over (resource safety, deterministic
destruction, data-race-free concurrency) and is the reason to write programs in
Sooth rather than in Forth or Rust.

But it is not a toy. Embedded and real-time is a first-class target, and Sooth is
expected to be *used* there. That narrows to a domain where the linear spine adds value. The craft
constraints are unchanged (small enough to hold in one head, legible compiler, simplicity over reach).

## Surface language

Concatenative, Forth-flavoured, with two non-negotiable ergonomics that Forth
lacks: statically-checked stack effects and named locals.

`gcd`, to fix the shape. The inputs get their names inline, in the effect comment
itself — `( a: int b: int -- int )` binds `a` to the deeper slot and `b` to the
top before the body runs — and a word calls itself directly (no `recurse`). The
`| … |` block is the other spelling and the more general one: a binding is a
term, not just an entry declaration, legal at any point in a body, popping that
many values off the stack where it appears, with its extent running to the end
of the enclosing block (a word body or a quotation body) rather than the whole
word:

```forth
: gcd ( a: int b: int -- int )
  b 0 eq 
  ~[ a ] 
  ~[ b  a b mod  gcd ] 
  if
;
```

Locals are opt-in, not the default. If you prefer the stack, `dup`/`swap`/`drop` most
one- or two-value words stay point-free (`square`, below, is just `dup mul`). You reach for
names when shuffling would read worse than names, typically three-plus live
values reused out of order, like a formula:

```forth
: lerp ( a: int b: int t: int -- int )   \ a + (b - a) * t
  b a sub t mul a add ;
```

`gcd` above sits on the line: two values, each reused and reordered in the recursive
call. It is shown with names here, but `swap`/`over` write it just as legibly,
which is the point: two values is where the judgment call lives, and `lerp`'s three is where names clearly win.

```forth
: gcd ( int int -- int )        \ ( a b )
  dup 0 eq                      \ test b (the top) directly
  ~[ drop ]                     \ ( a b ) → a
  ~[ tuck mod gcd ]             \ ( a b ) → ( b, a mod b )
  if
;
```

Checked stack effects are the cheap, high-value feature: Forth's signature failure
mode (a silent underflow producing a wrong number at runtime) becomes a compile
error.

```forth
: oops ( a: int -- int )
  a a add add ;
```

```text
error: stack effect mismatch in `oops` (line 2)
  `add` needs 2 values, but the stack holds 1
  note: declared ( int -- int )
```

The effect comment carries the boundary **types** (`( int int -- int )`); a word's
inputs get their **names** either there, inline (`( a : int int -- int )`), or from
a body-level `| … |` block — and the two compose, including out of order, so an
inline name and a block can bind different slots of the same word. An inline input
name is not documentation: it binds a local for that slot before the body runs.
Output slots and `extern:` slots are the exception — a name there stays doc-only,
since neither has a body to bind into. Inline naming has no reach into a
polymorphic effect: a slot name on either side of a `'T`-bearing effect is a hard
reject, not sugar (see `docs/named-slot-locals-spec.md`).

## The linear spine

Plain data is `Copy`: reuse is free and `dup` is ordinary.

```forth
: square ( int -- int )
  dup mul ;               \ int is Copy, so `dup` just copies the bits
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
: report ( path: str -- )
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
: leak-file ( path: str -- )
  path open-read
  size-of
  print ;
```

```text
error: stack effect mismatch in `leak-file` (line 4)
  body leaves 1 values, but ( … ) declares 0 outputs
  note: declared ( str -- )
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
  string slices (`str`/`cstr`, see [memory model](./docs/design/memory-model.md)), records/structs, enums/ADTs.
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

Ownership + linear types, deterministic explicit drop, **no tracing GC**. RC is
opt-in (`alloc` layer). References are second-class (Hylo-style, not Rust's borrow
checker): they can't escape their scope, so no lifetime system is needed — no
lifetime variables, no region annotations. Per-place exclusivity (at most one live
mutable reference to a place) replaces lifetimes, checked at consumption point.
`&!T` is a third disposal category: neither `Copy` nor linear, it owns nothing.
Pointers (`^T`) are non-null; `Option['T]` is an ordinary generic enum, not a
compiler primitive. Full detail: [memory model](./docs/design/memory-model.md).

## Control flow and iteration

No loop keywords. Quotations (`~[ ... ]`) and `call` are the sole iteration
primitive; an internal loop/back-edge construct gives constant-stack iteration.
Combinators (`each`/`map`/`fold`/`filter`/`while`/`times`) are library words
written in Sooth, inlined at call sites by term-splicing — no per-element call
overhead. Self-tail-recursion is a guaranteed constant-stack transform. `if` is a
`core::bool` word over the `branch` and `tag` primitives; the generated eliminator
word (`Shape?`) is the sole enum eliminator. No combinator is compiler-known, not
even `if`. Full detail: [control flow and iteration](./docs/design/control-flow.md).

## The irreducible core

The part of the surface no Sooth library can express. Everything else — `if`, `cond`,
the stack shuffles, `times` and the combinators, `close` and `free` — is an ordinary
word over this floor, demoted one slice at a time as the machinery beneath it lands.

The grammar that makes anything else definable: word and type declarations
(`: ... ;` with its effect comment, `type:`, `extern:`), the module declarations
(`import:`/`export:`), and locals — the inline effect spelling (`a: T`, which
desugars to the block) and the block itself (`| names |`, with block extent to
the end of the enclosing body).

Structural dispatch: the generated eliminator word, the sole eliminator for enums. An
arm is checked against the variant its tag names, coverage is exhaustive, and there is
no inline `match` — dispatch is a term, not a definition form.

Quotations: the literal `[ ... ]` and `call`. A quotation's body is checked where it is
spliced, so a library word cannot defer code the way `call` does; no word below them
can express either.

The operator words: arithmetic (`add sub mul div mod`), bitwise (`and or xor not shl shr`),
the comparison primitives (`ueq ult ugt ulte ugte une`, plus `max`/`max-total`), the two
control primitives (`branch`, `tag`), and the `>T` conversions.
Each bottoms out in a machine operation or a type-directed conversion; there is nothing
in the language to compose them from. Printing (`.`) is not among them: it is an
ordinary `hosted::show` word layered like any other hosted capability -- most
dots over `core::show`'s `Show`/`Write` traits, but the `str`, `cstr`, and float
dots ride direct externs.

`drop`: the sole way the stack shrinks by fiat. Every word body must net to its
declared effect, so any would-be library definition is circular (`: drop ( 'T -- ) ;`
fails its own effect check). Disposal stays explicit by design for every type, `Copy`
included; what varies is only what runs — a user `drop` overload for a linear leaf,
structural glue for a composite, nothing at runtime for `Copy`.

The owning-cell words (`@`, `!`, `+!`) and the borrow sigils (`&`, `&!`): raw
load/store on `^T` and reference creation with its exclusivity rules. Names under `^`
and `&` are reserved to the language, and no word body can reach the same primitive
access they couple to.

The shuffles sit just above this floor: `swap`/`over`/`rot` move and `dup` copies
today as compiler-known names, but each is an ordinary combinator's job once generics
land — `dup['a: Copy] ( ..s 'a -- ..s 'a 'a )` needs only the `Copy` bound that already
parses, and combinator splicing is what lets a quotation literal ride through them
unchanged. `times` has already demoted this way (slice 10b): the loop underneath is
carried by quotations, `call` and the self-tail-call transform alone.

## Modules and encapsulation

A file is a compilation unit; a directory tree under a `sooth.pkg` manifest is a
package. `import:`/`export:` with qualified access, selective imports, and
transitive dependency resolution. Name resolution is "own module first, then
qualifier"; a `type:` declaration is a name-scope, and exporting a type is
transparent (no opacity mechanism). Full detail: [modules and
encapsulation](./docs/design/modules.md).

## Codegen and backend

Compile-time virtual stack to native; words as functions. Backend is QBE (small,
legible, multi-arch) — no LLVM, no hand-written backend (deferred, reconsider after
self-hosting). WASM is a sibling lowering off the neutral IR, not routed through
QBE. The IR keeps `Ptr[T]` abstract and never assumes a 64-bit machine word, so
both lowerings concretise it. No in-process JIT; `driver::Library` can load a
compiled `.so` in-process via `dlopen`. Full detail: [codegen and backend](./docs/design/codegen.md).

## Concurrency: a library, not a core feature

Only atomics + memory ordering and a spawn primitive are core intrinsics;
everything else (channels, mutexes, actors) is a library. Data-race freedom is free
from the linear spine: sending a message moves the payload, and second-class refs
can't cross a thread boundary. Atomics have no single codegen strategy across
targets (LL/SC, interrupt masking, hardware spinlock). Full detail:
[concurrency and real-time](./docs/design/concurrency.md).

## Real-time: capable, not guaranteed

No GC and deterministic drop give the two hardest RT properties for free.
Soft-RT works out of the box; hard-RT by discipline (fixed layer + static
topology), not by enforced guarantee. Unsynchronised ISR/mainline sharing is a
checked error. Ravenscar is the reference for the RT concurrency library. Full
detail: [concurrency and real-time](./docs/design/concurrency.md).

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

alloc      growable Vec/Map/String, Box, opt-in Rc/Arc, heap-env closures,
           bignum. needs an allocator satisfying core's interface.

hosted     files, stdio, time, net, OS thread spawning, FFI-to-libc,
           blocking channels. needs an OS.
```

Placements worth noting: atomics are core but spawning threads is hosted; string
slice is core but growable `String` is `alloc`; a non-escaping quotation is core, and an
escaping closure's layer follows its *env*, not its escape: a single scalar capture rides in
the closure value itself and stays core, while an env owning a linear capture needs storage
outliving the frame and is `alloc` when that storage is the heap; the allocator *interface*
is core, its *implementations* (arena, pool, malloc) are not.

Discipline: fix the layer boundaries and the allocator interface on day one and tag
every stdlib word with the layer it needs, even though the hosted layer is built
first (that's where dogfooding happens). Carving `no_std` out later is the retrofit
tax Rust paid early; avoid it. And `no_std` core is not "runs on nothing": it still
assumes a handful of intrinsics (memcpy/memset for moves, integer-divide and
soft-float helpers where there's no hardware, the atomics) plus a per-target linker
script and entry point.

## Embedded: statics, MMIO, and interrupts

Static storage is a third category beside linear and `Copy` — a *place*, never
owned, moved, or dropped, reached only by second-class ref, constant-initialised,
declared at module level. MMIO is a typed fixed-address overlay with a volatile
aspect (QBE patched with a `vol` flag, not routed through opaque calls). An ISR is
a word exported under a fixed symbol and section. Shared state between an ISR and
mainline gets a global-set analysis: the hazard is a set intersection, computed
bottom-up over the call graph, inferred within a module and declared at boundaries.
Full detail: [embedded: statics, MMIO, and
interrupts](./docs/design/embedded.md).

## Liveness and the craft discipline

The failure mode with this project's name on it is a beautiful half-built compiler
that no one ever writes a program in, because building the compiler is more
immediately rewarding than using the language. The antidote is a hard requirement:

- Fast local iteration (`sooth run`) / immediate feedback from day one.
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

- **Scope**: craft language, not a product. Legibility, hold-in-head size, joy of building.
- **Signature idea**: linear (use exactly once) by default, `dup` is the explicit copy, `drop` is the checked destructor.
- **Surface**: concatenative, Forth-lineage, checked stack effects, named locals — inline in the effect (`a: T`, the default spelling for entry slots) or a `| names |` block (mid-body binding, quotations, impl members; the only spelling in polymorphic effects, where inline names are a reject).
- **Control flow**: `if`/`unless` are `core::bool` words over `branch`/`tag` primitives; generated eliminator word (`Shape?`) for enums; `cond` combinator for multi-way. No loop keywords. Detail: [control flow](./docs/design/control-flow.md).
- **Iteration**: quotations + `call` are the sole primitive; combinators are library words, inlined at call sites; self-tail-recursion is a guaranteed constant-stack transform (tail self-call → jump). Detail: [control flow](./docs/design/control-flow.md).
- **Type system**: small. Concrete types + ADTs + minimal row polymorphism + `Copy` marker. No full HM, no refinement/SMT, no effect rows, no dependent types.
- **Memory**: ownership + linear types, deterministic explicit drop, no GC, RC opt-in; second-class refs (Hylo-style), no borrow checker; non-null pointers; hidden/checked return stack. Detail: [memory model](./docs/design/memory-model.md).
- **Strings**: `str` (pointer + length) and `cstr` (pointer-only), following Zig's split without the sentinel-in-the-type. `Slice[T]`/`!Slice[T]` for array views (Phase 7 Slice 3c). Storage and view are two types: length lives in storage (`[T N]`), carried at runtime by the view (`Slice[T]`).
- **Destructors**: `drop` overload for a concrete type, not a new declaration form. Forces linearity. `Copy` and a user destructor are mutually exclusive. The body runs instead of synthesized field glue.
- **Closure captures**: a closure owning a linear capture says so in its *type* (the obligation is visible to whoever must discharge it, since the env itself is erased so combinators can exist), and says nothing about *where* the env lives (so an inline, static, or heap env are all one type). Such a closure is linear, with two consuming uses that run different code: `call` runs the body, which disposes what it captured exactly as a word body disposes a linear argument, and `drop` runs only the disposer, discarding the closure unexecuted. It may be a struct field, an enum variant field, or an owned-cell payload, where the container's synthesized destructor disposes it; an array or slice element waits on linear elements generally (Phase 7 Slice 5). Storing one in an aggregate, or discarding one unexecuted, needs a disposer synthesized glue can invoke without running the body: the closure value carries a per-construction-site disposer symbol alongside its code and env pointers, minted where the capture's concrete type is known. Not trait objects and not runtime type info: one statically-known function pointer, one indirect call on the disposal path.
- **Foreign calls**: `extern:` (symbol + stack effect). Scalars and refs cross; owned aggregates and `^` returns may not. Declaration site is the trust boundary; no separate `unsafe` marker.
- **Codegen**: compile-time virtual stack to native; words as functions. Detail: [codegen](./docs/design/codegen.md).
- **Backend**: QBE (small, legible, multi-arch). No LLVM. Hand-written backend deferred (reconsider after self-hosting). WASM is a sibling lowering off the neutral IR via binaryen. IR keeps `Ptr[T]` abstract. No JIT; `driver::Library` can load a compiled `.so` in-process via `dlopen`. Detail: [codegen](./docs/design/codegen.md).
- **Errors**: values, no THROW/CATCH, no unwinding.
- **Concurrency**: library, not core. Atomics + spawn are intrinsics; data-race freedom from linear types + non-escaping refs. Detail: [concurrency](./docs/design/concurrency.md).
- **Real-time**: soft-RT out of the box; hard-RT by discipline (fixed layer + static topology). ISR/mainline sharing is a checked error. Ravenscar is the reference. Detail: [concurrency](./docs/design/concurrency.md).
- **Layering**: `no_std` core with core / fixed / alloc / hosted layers; allocator interface in core; seams fixed day one.
- **Embedded/RT**: first-class target. Statics as third category (a place); MMIO as typed overlay with volatile aspect; ISR as exported symbol; global-set analysis for ISR/mainline sharing. Detail: [embedded](./docs/design/embedded.md).
- **Atomics and volatile**: QBE patches, not FFI. Per-target strategy (LL/SC, interrupt masking, hardware spinlock). Forking QBE for this does not pre-decide a Thumb/ARMv6-M backend. Detail: [embedded](./docs/design/embedded.md), [concurrency](./docs/design/concurrency.md).
- **Modules**: file is a compilation unit; directory tree under `sooth.pkg` is a package. `import:`/`export:` with qualified access and selective imports. Detail: [modules](./docs/design/modules.md).
- **Bootstrap**: host-language compiler then self-host a small subset, fixpoint-verify. Rust the sensible default.

## Tie-breakers

How to choose when two designs both work. `Decided` records what was chosen and `Declined`
what was chosen against; these are the rules that settle the next call, extracted from ones
that were argued out rather than assumed.

- **Never charge a semantic price for a performance property.** If a program needs an
  in-place update, the answer is not "make your type linear". Linearity is a semantic
  commitment — a destructor obligation, no free `dup`, move-on-every-use — and spending it to
  buy codegen is a bad trade that also lies about the type. Phase 4 Slice 6f is the worked
  example: a `Copy` aggregate accumulator could only be copied, at a full memcpy per loop
  iteration, because an aliasing rule was keyed on names merely in lexical scope, and the
  workaround on offer was to change the type's linearity. Fix the rule; do not bill the
  programmer. This is the same instinct as the [memory model's](./docs/design/memory-model.md) "a compiler-inserted copy is
  the same category of invisible behaviour as an auto-drop", pointed the other way: an
  invisible cost is a defect whether the compiler inserts it or the language extracts it.
- **One diagnostic severity.** Everything the compiler says is an error. "Unused" is an
  error exactly when a *linear obligation* is unmet, which the leak check already enforces;
  hygiene with no obligation behind it (a reference bound and never named, a dead local) is a
  linter's job. A warning tier would be a second, weaker answer to a question the linear
  spine already answers, and every diagnostic that is merely advisory is one the reader
  learns to skip.
- **A named thing behaves like the anonymous one.** Locals are the only readability tool a
  concatenative language has, so a rule that holds for a value on the stack must hold for the
  same value under a name. Where the two diverge, treat the named case as the bug: 6f exists
  because a borrow left on the stack ended at its last use while the identical borrow bound
  to a local lived to the end of its block, which made the legible spelling the rejected one
  and taught the workaround "don't name things".

## Open / deferred

- **Surface syntax for statics, the global clause, and register layout.** Semantics
  settled, spellings not. Settle in one brief, since they all land in the same
  declaration. Detail: [embedded](./docs/design/embedded.md).
- **Whether the ISR/mainline wrapper is a library type or a language construct.**
  Decide against a real driver, not in the abstract. Detail:
  [embedded](./docs/design/embedded.md).
- **Whether an ISR's global set can be checked under separate compilation.**
  Likely a link-time check, not per-module. Detail: [embedded](./docs/design/embedded.md).
- **Exact surface syntax for quotations/closures and their captures.** Illustrative
  above, not fully settled. Detail: [control flow](./docs/design/control-flow.md).
- **Optional HM inference later.** Kept possible by the `(value, type)` slot
  representation, not planned.
- **Mutual tail-call elimination (tier 2).** SCC contraction, not a trampoline and
  not QBE tail calls. Until then, a located compile error. Detail: [control
  flow](./docs/design/control-flow.md).
- **Drop at the back-edge.** The defined disposal point for loop-carried linear
  values not forwarded. Vacuous while all types are `Copy`; has a home once a linear
  value rides a loop. Detail: [control flow](./docs/design/control-flow.md).
- **Slicing `str` into a substring view.** Blocked by a prerequisite gap: `str`
  locals can't be borrowed today, and `str` descriptors are built statically per
  literal. Revisit once a `str` local exists as a sliceable source. Detail: [memory
  model](./docs/design/memory-model.md).
- **`.` appending no separator, for every type.** Decided, not yet implemented.
  Today `.` appends a trailing newline for every type except `str`/`cstr`. The
  decision is to make it uniform: `.` writes exactly the value, newline spelled by
  the caller. Touches ~130 stdout assertions across test files; lands as its own
  scoped change.
- **Bounded rows (`..N`).** A bounded row sibling to `..s`, with variadic FFI as a
  consumer rather than the justification. N is a compile-time literal on top of the
  row. Open: count spelling, `..N`/`'N` relationship, diagnostic on disagreement.
- **Owning a native backend.** Deferred. Reconsider after self-hosting, and only if
  the pull is genuine. Detail: [codegen](./docs/design/codegen.md).
- **Late binding for redefinition.** No redefinition path exists today: there is
  no interactive execution path, and nothing currently calls `driver::Library`'s
  `dlopen`/`dlsym` primitive to reload a word. Revisit only if a future consumer
  of that primitive (a hot-reload host, incremental compilation) introduces
  redefinition, as its own design track.

## Declined

Recorded so it isn't relitigated. Revisit only against a concrete program that can't
be written without it, never on principle.

- **Immediate-word / defining-word / macro facility** (would have replaced Forth's
  `CREATE`/`DOES>`). Declined. Sooth already sent the non-metaprogramming uses of
  Forth immediate words elsewhere (`if` is a library combinator over `branch`;
  iteration is quotations + combinators; comments/strings are lexer-level), leaving
  only metaprogramming, which splits into two capabilities both covered without a
  comptime facility: defining new words is a plain nullary word (`: answer 42 ;` is
  already a constant) plus generics, or an external build-time `.sth` generator for
  large families; baking a computed value into the binary is external codegen or
  runtime init. Declining it also keeps one "when does code run" story, with no
  compile-time phase in the user's model. **Revisit if** a bare-metal / no-allocator
  target (Phase 11) needs precomputed tables it genuinely can't build at startup; the
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
