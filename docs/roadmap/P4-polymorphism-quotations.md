[← ROADMAP](./ROADMAP.md)

### Phase 4 — Minimal polymorphism + quotations  `[L]`

Not full HM inference. Type variables (`'T`) and a row variable (`..s`) so the
monomorphic Phase 0 shuffles (`dup`/`swap`/`over`/`rot`/`drop`), plus `max` and user
words, gain honest polymorphic signatures; monomorphise
per concrete stack shape, force-inline the small core words. Required operations
(e.g. `>` for `max`) resolved at the concrete instantiation, Kitten-style, no formal
trait system. When such a required operation is a total order over **floats**, this is the
point to decide the float total-ordering story deferred from the floats slice (float `<`/`=`
are IEEE-partial; a `max`/sort over floats needs an explicit total order, Rust-`total_cmp`
style, surfaced at the call site rather than pretending IEEE ordering is total). **Quotations** (`[ ... ]` + `call`) as the sole iteration primitive,
plus the **internal loop primitive** they compile down to for constant-stack
iteration. Combinators (`each`/`map`/`filter`/`fold`/`while`/`times`) are ordinary
**library words** written in Sooth on top of quotations, with the compiler inlining
the common ones and their quotation arguments at the call site so they lower to tight
loops rather than a `call` per element. Quotations then become genuine first-class code
values in slice 7 (functions as values: downward closures on a frame-local environment,
upward ones on a `^` cell), which is what makes the functional style the combinators
imply actually writable. With quotations in hand, `if`
is redefined as an ordinary combinator (`cond [ then ] [ else ] if`, Factor-style) and
stops being a keyword, and a `cond` multi-way combinator lands alongside the others.

**Dispatch and uniformity (bundled here on purpose).** Several deferred ideas are one
conversation, and none is clean without quotations, so they land together in this phase:
(a) **`if` becomes an ordinary combinator** over quotations (above); (b) **generics /
minimal polymorphism** (above); (c) **ad-hoc dispatch** as static **overloading** (one
word name, several statically-known input types, e.g. `+` over `i64`/`f64`/`Vec2`); and (d) **`Bool` as a library enum**
(`type: Bool | False | True ;`) rather than a primitive. (d) waits for here specifically
because bool's specialness is not the *type* but that strict, quotation-less two-way
branching needs inline syntax; once `if` is a quotation combinator, `Bool`-as-enum +
`if`-as-word unify, and only then does making it a library type avoid re-adding special
cases (at `if`, at the `True`/`False` literals, and at type-directed printing). Slice 4's
clause-style match and `if/else/end` are deliberately designed not to foreclose any of
this: clause structure maps 1:1 onto a future quotation arm-table, and `if` staying a
keyword for now is the honest strict-eval choice, not a commitment.

**Exit:** polymorphic `dup`/`swap`/`max`; a constant-stack `each`/`fold` over a
collection; combinators verified to inline to loops, not per-element calls; and a quotation
held as a value, invoked through a path the compiler did not inline (slice 7).
**Dogfood:** write the combinator library (`each`/`map`/`fold`/`while`) in Sooth
itself, then rewrite an earlier program to use it; and rewrite `examples/vm.sth`'s dispatch
around a table of quotations.

**Slice plan** (dependency-ordered; each its own brief -> spec -> implement -> review,
same as Phases 2 and 3). None of these is a locked spec yet. The dispatch-and-uniformity
bundle above is **one design conversation, not one delivery unit**: settle the story
whole, land it in order. Phase 3's evidence for slicing rather than one big push is that
its Slice 8 had to split into 8a/8b/8c mid-flight (`close` had to exist before the
destructor mechanism could be designed against it) and its Slice 3 shipped covering only
direct self-recursion, needing Slice 4 as a follow-on — both boundaries discovered late,
under load. Process is calibrated per slice, not uniformly: slices 1–7 carry real design
risk and want full specs, while 9 is a mechanical migration that can run 8c-style (no
spec, one implementation pass plus one review).

**The paper pre-check that shaped this plan.** Before committing to the order, the four
headline combinators (`each`/`fold`/`filter`/`while`) were hand-written against the
planned feature set to see what they actually need. The exercise inverted the expected
answer: quotation *capture*, the question that looked hardest, turned out to be a
quality-of-life issue (see slice 4), while two things absent from the plan turned out to
be load-bearing: length polymorphism (now slice 1) and generic struct declarations
(originally slice 3, since moved to Phase 7 once slice 1's synthesized return bundles
removed `filter`'s need for it). Same technique as `vm.sth`, which shipped with zero
compiler changes and made that fact the Phase 2 exit verdict: write the program first,
then find out what the compiler owes it.

1. **Type variables + row variables + length variables + monomorphization (native).** The
   deepest change, and first for that reason: `'T` and `..s` mean a `Sig` stops being purely
   concrete, so unification and substitution touch every signature-checking path (blast
   radius comparable to Phase 2 Slice 1's typed-core spine, which is the closest precedent
   for how invasive it is). Monomorphise per concrete stack shape. Required operations (`>`
   for `max`) resolve at the concrete instantiation, Kitten-style, no formal trait system.
   **Native only — the REPL is slice 2**, split off because this slice already carries more
   than 8b did and 8b was Phase 3's largest, needing three review rounds precisely because it
   did the native and REPL halves together.
   **No inlining here** (see slice 6). The phrase "force-inline the small core words" that
   this entry used to carry is probably already satisfied and costs nothing: `dup`/`swap`/`+`
   and friends are match arms in `lower_call` that never emit an `Instr::Call`, and
   `check_shuffle` already dispatches on the concrete operand type, so "honest polymorphic
   signatures" may be a checker-side change with no lowering work at all. The brief should
   confirm that; if it holds, this slice adds no inlining machinery of any kind.
   Dogfood: rewrite existing examples to use polymorphic words, rather than adding a new
   example. The exit criterion ("polymorphic `dup`/`swap`/`max`") is a test, not a program,
   and the existing corpus is the honest measure of whether the generics are usable.
   **A length variable (`'N`) is required, not optional** — the pre-check's main finding.
   An array's length is part of its type, so `[i64 8]` and `[i64 4]` are distinct and a word
   taking one rejects the other, while the builtin `len` accepts both: the compiler already
   does length-polymorphism by hand and user words cannot. Since the only Phase 4 collection
   is the fixed-size array (`Vec` is Phase 7), without `'N` every combinator is per-length
   and **the phase exit criterion — a constant-stack `each`/`fold` over a collection — is
   unwritable**. This is also the third hand-rolled ad-hoc polymorphism site after `.` and
   the numeric-tower operators, which is exactly the tripwire slice 2 of Phase 3 set for `^`:
   the third instance is the signal that the special-casing has become the mechanism.
   **The linear spine is what makes this more than textbook generics**: `dup ( 'T -- 'T 'T )`
   is only sound when `'T` is `Copy`, so a *constraint* appears here whether or not one is
   wanted, and the slice has to decide whether `Copy` is an ordinary required-operation
   constraint or a privileged one. The pre-check settles the sub-question of whether
   constraints can be global: they cannot. `each` reads elements through `&> @`, which Slice
   6 restricts to a `Copy` referent, so `each`'s element variable needs the bound — while
   `fold`'s accumulator needs none and may legitimately be linear. **Per-variable, not a
   blanket "generics are Copy-only" rule.** The signature of a polymorphic `drop ( 'T -- )`
   is the same question pointed at 8b's per-type overloads; parked for slice 8 (static
   overloading, which says the same thing from its side), but its answer must not be
   foreclosed here.
   **Closes the multi-output lowering hole**, which stops being deferrable here: a `..s` in
   output position *is* a word with a statically-unknown number of outputs, so row variables
   cannot ship on a lowering path that panics on two. The defect is long-standing and
   independent of this phase; it was noted and passed over three times (the Slice 6 spec, and
   8b's brief and spec, the latter rejecting multi-output `extern:` at the declaration
   precisely to avoid it) and has never had a home. It lands here rather than as a standalone
   warm-up fix because **it is not a separate decision**: "how do two values cross a call
   boundary" and "how does `..s` cross a call boundary" are one question, and answering it
   early, without the constraint that actually stresses it, risks answering it twice and
   inconsistently. A stopgap diagnostic rejecting multi-output calls is also not worth taking,
   since this slice removes it again immediately.
   Two things the brief should start from. **The defect is narrower than "multi-output words
   are broken"**: defining one checks and lowers fine (ten existing tests assert exactly that,
   including `: w ( -- bool bool bool )` lowering successfully), and only the *call* desyncs
   the stack — `lower_call` builds a result only when `out_arity == 1`, `Instr::Call` carries
   an `Option<Value>`, and `env` stores a single `ret_ty`, so there is nowhere to put a second
   output. And **the likely answer already exists in the tree**: aggregate returns work today
   (`vm-pop ( Vm -- VmPop )` in `examples/vm.sth` is a shipped word returning a struct), so
   multi-output can plausibly synthesize the bundling users currently do by hand, against
   machinery that is already load-bearing. Out-parameters and a carried stack are the
   alternatives to weigh against it.
   **The float total-ordering decision, deferred from the floats slice, lands here**: float
   `<`/`=` are IEEE-partial, so a `max`/sort over floats needs an explicit total order
   (Rust-`total_cmp` style) surfaced at the call site rather than pretending IEEE ordering
   is total.
2. **REPL monomorphization.** Split from slice 1 to keep the phase's risk out of one commit,
   and placed immediately after so no later slice builds on a REPL that cannot see a
   polymorphic word. **The problem is retention.** `Session` keeps signatures in `env` but
   discards ordinary word *bodies* once their line compiles to a `.so`; the only bodies it
   retains are `drop_overloads`, which 8b added. A polymorphic word has no concrete
   instantiation to compile at its defining line, and a later line's `5 id` needs `id@i64`
   lowered into *that* line's `.so`, from a body the session threw away. So this generalizes
   8b's retention scheme from "drop overrides" to "every polymorphic word" — and inherits its
   hazards, which are documented in place and worth reading before designing: the stale-env
   problem that `drop_dropped_sites` caches around (re-checking an earlier body against a
   later line's env), and `RTLD_GLOBAL` symbol collision when two lines instantiate the same
   word at the same type, which is what the epoch-suffixing machinery already solves for
   destructors. A further collision the brief traced directly (`docs/phase4-slice2-brief.md`):
   `instantiation_symbol` (S1) is a pure function of word name and substitution with no
   generation component, so redefining a polymorphic word and re-instantiating it at a type it
   was already instantiated at mints the same symbol as the old body and silently keeps running
   it under `RTLD_GLOBAL`'s first-loaded-wins rule. And one binding question the brief settles
   rather than leaves open: an instantiation lowers against the resolver live at the word's own
   *defining* line, not the instantiating line's, matching the frozen-binding rule every other
   REPL word already follows (see DESIGN.md's Open / deferred: REPL late binding, for the larger
   live-patching question this brushed against and deferred rather than decided here).
3. **Aggregate-return aliasing: the loop-carried copy (fixed).** A self-tail-recursive
   loop lowers to header phis rather than a new frame each iteration, so any aggregate
   *storage reused across iterations* that is carried across the back-edge could alias:
   a value from iteration *k* pointed at the same slot iteration *k+1* overwrote, silently
   becoming the wrong value with no diagnostic. A by-value aggregate return (`%r =:T call
   $f()`, one QBE stack slot per call site) was the common instance and the reason this
   became urgent, but it was one instance of the mechanism, not the mechanism itself: an
   aggregate constructed inline each iteration, with no call at all, reused its
   entry-hoisted storage and reproduced the identical bug. Fixed by giving each
   loop-carried aggregate slot one entry-hoisted stable stack slot (no header phi for it)
   and an unconditional read-before-write staged blit on the back-edge, immune by
   construction to both a swap (two carried slots exchanging places) and an interior
   pointer into a carried slot (`field_value`'s `PtrOffset`), with no aliasing analysis.
   Scalars, references, ordinary (non-loop) join phis, and the fused destructor loops'
   own lowering are unchanged.
   **This displaced generic struct declarations from the slot** (moved to Phase 7, see
   there). That slice's whole claim to not being speculative structure was one named
   consumer, `filter` needing to bundle a filtered array with a count, and slice 1's
   synthesized multi-output return bundles closed that need: `: pass-through ( [i64 'N] --
   [i64 'N] usize ) len ;` is `filter`'s exact shape and compiles and runs at two different
   lengths today, verified against the built compiler.
4. **Quotations + the internal loop primitive.** `[ ... ]` + `call`, plus the loop
   primitive they compile down to for constant-stack iteration, plus call-site inlining.
   **Scoped by `docs/phase4-slice4-brief.md`; the decisions below are settled, not open.**
   `times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )` passes the iteration index and its body
   quotation returns the row it received, so effect inference only ever unifies an inner row
   against itself.
   The slice ships a runnable constant-stack loop rather than inert plumbing, so the phase's
   riskiest integration (type machinery against quotations against the loop primitive) gets
   a witness in the slice that builds it; the headline is `0 1000000 [ + ] times .` printing
   `499999500000` in constant stack, next to `examples/countdown.sth`'s hand-threaded
   self-recursive equivalent. The loop primitive stays internal (DESIGN.md:281-289: "not
   surface syntax, not user-facing"); what reaches it is the self-tail-call transform, so
   every combinator including `times` is library source (slice 10b).
   A quotation here is a **compile-time marker** carrying its inferred effect and body,
   fused at its `call`, never a runtime value: that defers the `Type`/`PolyType`/`IrType`/
   unification/mangling change to slice 7, where a consumer for it finally exists. The two
   "inlining"s the plan used to seem to double-book are different mechanisms and both are
   real: slice 4 owns **quotation-literal fusion** (splicing a literal's body at its `call`,
   never crossing a `:` word boundary), slice 6 owns the **interprocedural user-word
   inliner**.
   The brief's sharpest finding is what it rules *out* of this slice: a polymorphic word
   today can neither branch (`if` is rejected in a polymorphic body, `src/check.rs:2997`)
   nor loop in constant stack (`self_tail` is hardcoded `false` on the polymorphic
   instantiation path, `src/ir.rs:1176`), so combinators cannot be written as ordinary
   polymorphic library words at all. *This paragraph used to end "both gaps land in slice 6
   against their first real consumers"; slice 6b's paper pre-check falsified that by building
   the programs. Neither gap gates a combinator, because a combinator is checked by
   term-splicing at the concrete call site and never goes through `poly_term` at all. Both
   gaps survived 6a-6c untouched: the polymorphic `if` is now 6e, and the `self_tail`
   hardcode is unreachable until a polymorphic body can call a polymorphic word (slice 7).*
   After slice 1 because a combinator's signature has to *say* `[ 'a -- 'b ]`, which needs
   row variables to be expressible at all. **Capture is a quality-of-life question, not a
   soundness one** — downgraded by the pre-check, which expected the opposite. The fear was
   that a quotation capturing a linear value and called twice would dispose it twice, the
   split Rust spells `FnOnce`/`FnMut`/`Fn`. But `fold` and `while` need no capture at all:
   loop state rides the accumulator and the stack respectively, which is the concatenative
   structure doing the work. Capture is wanted only for read-only *ambient* context that is
   not loop state (`| threshold | arr [ threshold > ] each`), and even that has an
   expressible if ugly fallback in bundling the context into the accumulator. So the library
   is writable non-capturing, and the brief decides on ergonomics with a working fallback in
   hand rather than under threat. The likely resolution to reach for: capture is free
   precisely when the quotation is inlined, since the captured local is simply still in
   scope after inlining — no environment, no allocation, and no `FnOnce`/`FnMut`/`Fn` split
   if capture is restricted to `Copy` locals. Escaping quotations stay out of scope
   regardless — they need the uniform-runtime-stack fallback and Phase 7's alloc layer.
5. **Modules: multi-file compilation, word and type imports.** Pulled forward from Phase 7
   for two converging reasons, not one. Slice 6's combinator library needs somewhere to live
   besides copy-pasted into every dogfood — the original motivation — but the bigger one
   surfaced at this slice's brief conversation: a reusable component worth writing is rarely
   just words. A queue, a small `Result`-like type, a `Point`/`Vec2` bundle — the type is
   usually the component, the words around it are secondary. Word-only imports would serve
   the combinator library and almost nothing else worth calling a library, so this slice
   takes the harder half too: a file becomes a compilation unit, and an import brings
   another file's words *and* struct/enum declarations into scope by qualified name.
   **Split native/REPL, 8a-style**, on the precedent that already exists twice in this
   project: slice 1 held REPL monomorphization back to slice 2 rather than carry both, and
   Phase 3's 8b needed three review rounds precisely because it did the native and REPL
   halves together. The REPL has no file-loading path at all today and carries session state
   the native path does not (`drop_overloads` keyed by `StructId`, frozen resolver snapshots,
   per-name generations, every `.so` resident under `RTLD_GLOBAL`), so what an import *means*
   there is a separate design problem, not the same one repeated.
   **This settles the open design question Phase 7 had named**: how struct/enum
   declarations from separate files join one shared type registry without a name collision
   or a duplicate registration. Type resolution, move-tracking, and the R21 aliasing rule
   are value-flow-based, not file-based, so none of that checker machinery should need to
   know a boundary was crossed. But the *front* of the pipeline is not additive, and the
   brief should start from that: `driver::build` today is straight-line `read_to_string` ->
   `lex` -> `parse` -> `check` -> `lower` -> `emit`, and `check::check(&mut module)` takes
   exactly one module, so something has to locate imports, parse N files, reject import
   cycles, and merge before checking runs.
   **Encapsulation lands here too, because without it a type cannot hold an invariant.**
   A `type:` generates a constructor, getter, peek, setter, and destructure word per field,
   all ordinary words in the flat environment, so with no visibility rule an imported
   `Queue` hands every consumer `Queue<back` and the invariant it was written to protect is
   gone. Default private, with a per-file `export:` form naming what crosses. Because the
   generated accessors are just words, that list gives per-word granularity for free (export
   the type and its operations, withhold the raw setter) with no new visibility algebra.
   Elm's opaque types are the precedent and answer the one subtle question: exposing a type
   *name* and exposing its *constructors* are separable exports, and that separation is the
   whole encapsulation mechanism.
   Syntax proposal, to be confirmed at the brief: `import: queue "collections/queue.sth" ;`
   and `export: Queue push pop ;`, following the existing `name: ... ;` defining-word family,
   with access as a single token `queue::push`. `::` because `.` is the print word and `>`
   is already the generated field getter (`queue>push` would read as a getter on a type
   named `queue`). Qualified-only, so there is no cross-module collision rule to invent. A
   file with no `export:` exports nothing: it is a program, not a library, which keeps every
   existing single-file example valid.
   **Still narrower than Phase 7's eventual module system.** No serializable API description
   and no version diffing. Those are a packaging/publishing concern — letting other people
   depend on you with enforced semver — not a personal-reuse one, and
   `docs/dependency-management.md` still depends on Phase 7 for them; it consumes the export
   list this slice introduces rather than defining it.
   **Disposal crosses the export boundary, which constrains the export rule.** Two
   consequences of linearity meeting encapsulation, both wanting settled at this slice's
   brief. First, a destructor runs without being named, so an unexported `close` still
   executes in a consumer's program through the generated glue: the export list describes
   what a consumer can *call*, not what can *run*. Second, and sharper: whenever a type's
   disposal is not reachable through `drop` alone (see slice 8's constraint), an exported
   opaque linear type must also export the word that discharges its obligation, or a
   consumer who obtains one is stuck holding a value it cannot legally consume — unusable
   rather than encapsulated. That is a checkable rule; decide here whether the compiler
   enforces it or it stays the library author's job.

   **5a — native multi-file compilation, imports, and encapsulation.** Everything above, on
   the native path only. **`import:` at the REPL must be a clean located error in this
   slice, specified and tested, not left to whatever the parser happens to do.** This is the
   one lesson 5b cannot be trusted to carry: slice 1 shipped without pinning REPL behaviour
   for polymorphic words and slice 2's recon found the gap had produced a *silent
   miscompile* (a bogus `( -- )` mismatch, or a silent `defined` that never checked the
   body). An unimplemented feature that rejects is a deferral; one that quietly does the
   wrong thing is a defect, and the difference is one specified diagnostic.
   Open questions for its brief beyond the pipeline shape above: whether a qualified name
   becomes part of the `StructId` key and what that does to existing interning (structs are
   nominal, so two files each declaring `Point` collide); whether 8b's epoch/generation
   destructor-symbol scheme stays collision-free once two modules synthesize destructors for
   same-named types; and where privacy is enforced, since filtering unexported names at
   splice time is the simplest implementation but reports "unknown word" where it should say
   "`grow` is not exported by `queue`", and diagnostics are behaviour here.
   **Exit:** two files, one importing a type and a word from the other, compiling and
   linking as one program, with a non-exported word rejected at a use site in the second and
   `import:` at the REPL rejected with a located error.

   **5b — imports at the REPL.** Retired with the REPL (P7.S9): the REPL no longer exists,
   so a session-scoped `import:` is not a criterion. The module-system facts it exercised
   (a qualified word/type resolving across files, a locked export list, a rejected imported
   `main`, transitive closure, struct-id aliasing across a qualified/unqualified spelling)
   are covered natively by `tests/phase4_modules.rs`.

   **Dogfood (5a):** the combinator library (slice 6) lives in its own file; a small
   standalone type (e.g. a `Point`/`Vec2` or a stack-like struct) lives in another, exported
   opaquely, and is imported by an example that uses both.
6. **The combinator library in Sooth + inlining, and the machinery that work measured (the
   phase's headline exit).** *This heading used to end "+ closing the polymorphic-path gaps".
   It did not close them and no longer claims to: 6b's pre-check established that neither gap
   gates a combinator. 6a-6c are the library itself; 6d, 6e and 6f are gaps the library work
   surfaced but does not itself need, grouped here for ordering rather than subject matter.*
   `each`/`map`/`filter`/`fold`/`while` as ordinary library words over quotations,
   with the compiler inlining the common ones and their quotation arguments so they lower to
   tight loops rather than a `call` per element. Depends on 1–5a (5a is where the library
   gets its own file to live in instead of every consumer's; 5b is not a prerequisite), and
   comes *before* the dispatch slices deliberately: it is the first real integration test of the type machinery
   against quotations, so if the two are awkward together, that feedback should arrive before
   three more mechanisms are built on top. The paper pre-check is not a substitute for this,
   only a filter on the plan.
   **Split in three, on fault lines a paper pre-check against the built compiler measured**
   (findings below, all verified by compiling, not read off this document). `each`/`map`/
   `fold` are expressible over `times` alone and need none of the polymorphic-path machinery
   `filter` and `while` need; and the REPL half is its own design problem, exactly as it was
   at slice 1/2 and 5a/5b. Separate them rather than risk a slice splitting mid-flight the
   way Phase 3's 8 did.
   **What the pre-check overturned:** the combinator library is not "writable but slow"
   today, it is **not expressible at all**, so the inliner is the enabling mechanism rather
   than an optimization laid over a working library. Two independent walls, both measured: a
   quotation type cannot be spelled in a signature (`( i64 [ i64 -- i64 ] -- i64 )` fails in
   the *parser*, since `[ ... ]` is already the array-type syntax and the count must be a
   literal), and passing a quotation to any user word, monomorphic or polymorphic, is a hard
   located rejection. What the inliner must *produce*, by contrast, is fully expressible:
   hand-inlined `each`, `fold`, and `map` over a `[i64 4]` (a `times` loop, `&a i &> @`
   reads, `&!b i &!> v !` writes) compile and run correctly today, and the length/element
   polymorphism they need is already there (`: size ( ['T 'N] -- ['T 'N] usize ) len ;` runs
   at `[i64 4]` and `[f64 7]`). The gap is entirely front-end.

   **6a — quotation types in signatures + the inliner + `each`/`map`/`fold`** (native).
   **This slice owns the inliner, and it is the only one** — there is no inlining anywhere in
   the compiler today. Everything the source calls "inline" means "lowered straight to
   instructions rather than a call" (builtins and generated struct/enum words are match arms
   in `lower_call` that never emit `Instr::Call`); a user `:` word is always a real call.
   **Nothing downstream will do it either**: QBE is a per-function backend and emits
   `callq` even for a one-instruction function called with a constant (verified directly),
   and the driver runs `cc` on a `.s` with no `-O` flags, so that is only the assembler. The
   pass lands here rather than with monomorphization because combinators are *ordinary Sooth
   library words*, so inlining them is inlining monomorphized user words — there is one
   mechanism either way, and it should be designed against its first real consumer instead of
   two slices ahead of one. That library-words commitment is what forces the pass at all: the
   alternative is making combinators compiler-known and lowering them as match arms, cheaper
   but forfeiting the thing the phase is trying to prove, in the same way `vm.sth` shipping
   with zero compiler changes *was* the Phase 2 exit verdict.
   **A quotation becomes nameable in a signature here, and that is decided, not open.** For
   `: each ( ['T 'N] [ 'T -- ] -- )` to be checkable standalone, `Type`/`PolyType` gain a
   quotation variant carrying a declared effect, with unification, `apply_subst`, and
   mangling following, and `call`/`times` must accept an **abstract** quotation — one whose
   effect is known only from a declared parameter type — instead of demanding a
   statically-known literal as they do today (`src/check.rs:4763,4797`). New surface syntax
   is needed for the spelling, since `[ ... -- ... ]` collides with the array type. This is a
   slice-1-shaped change and the reason 6a is `hard`, **but it stops at the type**: no
   `IrType` variant, no calling convention, no per-literal environment struct, no `(code,
   env)` representation, no escape rules, no quotation in an array, struct field, or branch
   join. Every consumption site still resolves to a known literal, so lowering is untouched.
   Slice 7 keeps the representation half and gets cheaper by exactly what this pre-pays.
   Not split off into its own slice precisely because a quotation type with no inliner
   compiles no call site at all: it would be inert plumbing, which slice 4 refused on the
   same grounds, and "what is a quotation parameter's type" and "what happens at a call site
   that must inline" are one question in the way row variables and multi-output returns were.
   **Two constraints that fall out and belong in the brief.** Inlining stops being an
   optimization and becomes **total**: with a quotation type but no runtime value there is no
   fallback, so every call to a quotation-taking word must inline, transitively (`map` over
   `each` inlines twice), and anything un-inlinable — recursion among quotation-taking words,
   for one — must be a located error, never a silent real call. And the new variant must not
   bake in "a quotation is always statically known": keep that a predicate on the value, or
   slice 7 has to unpick it out of unification and the monomorphization walk to allow a
   genuine runtime closure.
   **The REPL is 6c, and 6a must pin its behaviour with a located rejection**, specified and
   tested — the 5a lesson, which slice 2's recon proved the hard way when slice 1 left REPL
   polymorphic words unpinned and the gap turned out to be a silent miscompile. The checker
   sees only signatures at a call site (`env: &HashMap<String, Sig>`), never bodies, so an
   inliner needs bodies threaded in; at the REPL bodies are discarded except drop overrides
   and `poly_words`, which is a retention problem, not the same problem repeated.
   Needs none of 6b's checker fixes, since `each`/`fold`/`map` need no branch.
   Accepts one known churn cost — the library is written against keyword `if` and gets
   rewritten when slice 9 turns `if` into a word — which is a handful of library words,
   cheaper than reordering the phase around it. **Exit:** `each`/`map`/`fold` written in
   Sooth over `times`, verified to inline to a tight loop rather than a `call` per element,
   with a quotation-taking word at the REPL a located rejection.
   Dogfood: rewrite an earlier program to use `each`/`map`/`fold`.

   **6b — `filter`/`while`, and the self-tail combinator loop.** *This entry previously said
   the slice closes the two "polymorphic-path gaps" slice 4's brief measured. Its own paper
   pre-check falsified that, by building the programs (see
   `docs/phase4-slice6b-brief.md`); the charter below is the corrected one.*
   Neither named gap is what `filter`/`while` need, and both are left in place. A combinator
   is checked by term-splicing at the *concrete* call site, never through `poly_term`, so the
   polymorphic-`if` rejection never gates one: `filter` compiles and runs today with **no
   compiler change at all**, `if`/`else` and all. And a polymorphic body cannot call any
   polymorphic word, self or other (`unknown word` in `poly_call_term`, long before
   lowering), which makes the `src/ir.rs` poly-instantiation `self_tail` hardcode currently
   *unreachable*: no poly word can self-call to reach it.
   `while`'s actual blocker is 6a's own D5 combinator-cycle rejection ("a quotation-taking
   word cannot be recursive"), which fires identically for a *monomorphic* self-recursive
   combinator, so it is not a polymorphism question at all. The work is therefore: relax D5
   for a self-*tail* combinator edge only (non-tail self-calls and mutual cycles stay hard
   errors, they need slice 7's runtime quotation), and lower that edge to a loop back-edge at
   splice time, reusing the mid-body loop `times` already opens (brief D8; specializing the
   combinator into a monomorphic `IrFunc` was weighed and rejected, since it reopens 6a's
   "inlining is total" and "a combinator mints no symbol"). The tail-vs-non-tail distinction
   is a *checker* change, not just IR work: `check_combinator_cycles` builds its edges from
   `all_calls`, which erases it.
   Ships `while` inheriting the R18 nested-loop limit that `each`/`map`/`fold` already have
   (brief D9); 6d lifts it for all of them at once. Depends on 6a for the inliner and the
   library's file/shape.
   **Exit:** `filter` and `while` written in Sooth and inlined the same way 6a's words are,
   with `while` running in constant stack and a non-tail combinator self-call still rejected.

   **6c — quotation-taking words at the REPL is implemented.** *This entry previously said
   the frozen-binding question of which generation of a callee an inlined body binds to
   should follow slice 2's answer for instantiations (the defining line's resolver snapshot).
   `docs/phase4-slice6c-brief.md`'s recon falsified that framing before any code changed: a
   combinator mints no `IrFunc` and no symbol, so it has no compile event of its own to freeze
   against, and it is re-checked and re-lowered at every splice site against that site's own
   live env, unlike a poly body which is checked once. There was no frozen resolver to
   design; the charter below is the corrected one.* Lifted 6a's located rejection: what it
   means to define and call a combinator in a live session. The problem was retention, the
   same shape slice 2 solved for polymorphic words and 8b for drop overrides — a session
   discards ordinary word bodies once a line compiles to its `.so`, and an inliner needs the
   body. The fix was a plain session store (`HashMap<String, WordDef>`, mono and poly
   combinators alike, replaced wholesale on redefinition, D1) projected on demand into the two
   shapes the checker's and lowerer's inline paths already read, threaded into the REPL entry
   points that hardcoded an empty combinators map. A review pass then closed a hygiene gap the
   brief's recon missed: an imported combinator's body call to a module-0 private word (and
   the forgeable variant, a REPL-declared name matching a multi-file closure's internal
   mangled spelling) resolved against the session's own env instead of the closure's,
   a silent-wrong-answer risk on a name collision. **Exit:** a session defining a
   quotation-taking word, calling it from a later line, and redefining it, with every splice
   at every call site reading that site's own live env (no frozen resolver, D1).
   **6d — nested constant-stack loops (the hoist-target split) is implemented.** Lifted R18,
   which rejected any `times` reached while a loop was already open. The limit was not
   hypothetical and not confined to `while`: it bit every combinator 6a shipped, because each
   one drives its own `times`. `2 [ | i | mk [ . ] c::each ] times` was a hard error ("a
   `times` cannot be nested in a loop yet"), so no combinator composed inside a loop, and the
   rejection was a deferral rather than a design decision.
   **The cause was one field doing two jobs.** `FuncBuilder::entry_block` (`src/ir.rs:2226`)
   was simultaneously the alloca home and the loop preheader. It must be the function's true
   entry block for allocation, since QBE's alloca bumps the frame pointer on every execution
   and never reclaims within a function, so an `Alloc` reached per-iteration grows the frame
   until the constant-stack guarantee is worthless. It must be the *loop's* preheader for a
   carried aggregate's stable-slot seeding blit, which has to run once per entry to that
   loop. Those two blocks coincide at exactly one loop level and diverge the moment loops
   nest, which was the whole of the bug. **The fix inverts, rather than matches, this entry's
   original framing of "split the field into an invariant alloca home and a per-loop
   preheader": only the alloca role moves.** `entry_block` keeps its meaning as the per-loop
   preheader — it was already correct at any depth — and a new, invariant `alloca_home` field
   takes over the allocation role; `push_alloc` routes into it, and `begin_loop` sets it once,
   on the outermost loop only. The rest of the loop state (`header`, `carried_slots`,
   `back_edges`) already saved and restored around a nested region (`src/ir.rs:2609-2687`,
   the `times` arm), so the phi bookkeeping was largely present already; `alloca_home` joined
   that same save/restore as a fifth field, behind one shared helper collapsing what had been
   duplicated at two call sites.
   **Handled with care, not as a rider on another slice.** This is the same loop-lowering
   code where the aggregate-return aliasing bug landed (see the Phase 4 slice 3 note above),
   whose fix, one entry-hoisted stable slot per carried aggregate plus an unconditional
   read-before-write staged blit on the back-edge, is exactly the invariant that rearranging
   hoist targets could silently break. Its guards (the seeding-blit reseed probe and the
   large-outer constant-stack witness) are mutation-tested, not merely run green: each is
   shown to fail (wrong value, or SIGSEGV) against the fix reversed.
   **Exit:** a combinator called inside a `times` body compiles and runs in constant stack —
   `examples/combinator_in_times.sth` dogfoods `each`-in-`times` against a hand-threaded twin
   — with the nested-loop goldens (all five combinators, any pairing, depth 3) and the slice-3
   aliasing guards green, and a destructor call inside a `times` body inheriting the fix for
   free (its fused loop already opens at its own `IrFunc`'s true entry).
   **6e — `if` in a polymorphic body. Complete.** Lifted the rejection in `poly_term`'s
   `TermKind::If` arm (`src/check.rs:3715`, formerly `` `if` in the polymorphic body of
   `{word}` is not yet supported ``), which had stood since slice 1 deferred it and which no
   later slice picked up. **A 6-family letter for ordering and
   discovery, not subject matter:** 6b's pre-check is what measured this gap (while ruling it
   out of 6b's scope), and it has to read before slice 7, which needs it. Unlike 6d, whose
   consumers are the 6a combinators themselves, this one's consumers sit outside the family
   entirely, below.
   **Two consumers, both real, which is the bar this plan keeps applying.** Slice 7: any
   closure-taking word worth writing branches on something. And the core library: a word is
   only a library word if it can be written in Sooth, and `max` cannot be, so it sits in
   `BUILTIN_WORDS` next to `+` as a compiler builtin. `: mymax ( 'T: Copy Ord 'T -- 'T ) over
   over > if drop else swap drop end ;` is rejected today (verified by compiling; its
   monomorphic `i64` twin builds and runs), and the `Ord` bound it needs already exists. That
   makes this the gate on separating the irreducible intrinsics (`+`, the shuffles, `fill`,
   the `>uN` conversions: things that lower to instructions no Sooth source can express) from
   the words that are builtins only because the front end cannot yet express them.
   **Three pieces, and shipping a subset is worse than shipping none** (`docs/phase4-slice1-spec.md:80`,
   which is the only existing plan for this): pop the condition off the `PolyType` stack, run
   a per-arm unconsumed-linear check, and join the two arms' move-state. None of the three is
   lifted to `PolyType` yet. The rejection exists *because* the half-built arm it replaced
   both spuriously rejected valid programs (`: choose ( 'T 'T bool -- 'T ) ...`, whose
   monomorphic sibling builds) and panicked the compiler outright (a `^i64` allocated on one
   arm reaching `ir.rs`'s `drop: non-empty stack`). `max`'s own body is a fair test rather
   than a trivial one: its arms `drop` different operands, so it exercises exactly the
   move-state join that is missing.
   **Not in scope: the quotation-in-a-polymorphic-body rejection** (`src/check.rs:3798`).
   It is a sibling wall, not this one, and it belongs to slice 7, which is where a quotation
   acquires the runtime representation that would let a polymorphic body carry one.
   Depended on slice 1 only; independent of 6a-6d and landed after 6d.
   **Exit, met:** a polymorphic word that branches, including one whose arms consume
   different operands, compiles and runs at two instantiations (`choose` and the newly
   library-writable `mymax` at `i64` and `f64`), with the linear checks that motivated the
   original deferral proven by tests (T2-T8, `src/check.rs`) that fail without them and by
   mutation-tested guards on the three-state move join. Slice 7's stated dependency on a
   branching polymorphic body is satisfied.

   **6f — liveness ends at last use. Implemented**; a gap found in review is closed, below. The
   motivation, as originally written: `live_derivs`
   (originally `src/check.rs:759`, now `:985`) chains the stack slots with the scope's bindings, so a reference left
   on the stack dies when a term consumes its slot, while a reference bound to a local stays
   live for the whole block. Chaining a borrow therefore compiles where naming it does not,
   and the rejection lands on the natural shape: borrow a place, write through the borrow,
   then consume the place, which fails with `cannot consume the borrowed local` pointing at
   the consume. Verified by compiling, in a straight-line word body with no loop and no
   quotation involved — so this is not about iteration scoping, which already expires a
   body's bindings per cycle (`leave_block` after the `times` body).
   **Not a lifetime system**, by DESIGN.md's own definition: no lifetime variables, no
   regions, nothing binding a reference's validity to a named scope. It is a rule about when
   a borrow ends inside one block, and the anonymous case already works that way, so this
   makes named references behave like the stack values the language is otherwise built
   around rather than adding a concept.
   **Why a slice and not a workaround.** Locals are the only readability tool a concatenative
   language has, and this makes them poison exactly where they help most; the workaround is
   to spell the whole projection as one chain, which is the opposite of the legibility the
   language trades on.
   **Before slice 7**, which points Phase 3 Slice 6's escape checking at a new carrier
   (closure captures, which inherit their captures' restrictions). Settling when a borrow
   *ends* after closures start capturing them means answering one question twice, the failure
   this plan keeps citing.
   Implementation shape the brief should start from: word bodies are flat term lists, so a
   backward scan gives a last-use index per reference local, and `live_derivs` filters the
   bindings whose last use has passed, given a current-term index threaded into the query.
   Two wrinkles: branches (last use is the max across arms) and loop bodies (any use means
   live for the whole body, which the identity-on-borrow-state check at `src/check.rs:6041`
   already wants).
   **Two tables, one rule.** The same lexical-vs-last-use question applies to
   `aliasing_origin` (originally `src/check.rs:854`, now `:1099`), which rejects a mutable borrow of a place a second
   live *name* denotes. Both scan a scope table for names that are merely in lexical scope,
   and both leave their stack halves alone; taking only the borrow half would answer one
   question twice. **Measured, not assumed:** stubbing `aliasing_origin` out makes a `Copy`
   aggregate accumulator thread through `fold` with **zero** per-iteration blits, printing the
   right answer and matching the linear case's shape, against four blits per iteration for the
   `dup` form it forces today. That gap is an expensive implicit memcpy standing where a
   reader expects a move, and it is reachable now only by making the type linear — paying a
   semantic price (a destructor obligation, no free `dup`, move-on-every-use) for a
   performance property. Seventeen tests guard the rule; the three sampled all use the
   aliasing name *after* the borrow, so they stay live and keep failing for the reason they
   state, with the risk concentrated in the six branch/merge cases where region unioning meets
   per-arm liveness. **The two halves differ in failure mode** and the slice must treat them
   differently: the borrow half wrongly accepts or rejects a program, while the alias half can
   silently produce a wrong *value* — the class DESIGN.md names as the one this language
   exists to turn into a compile error — so every alias-half test is mutation-tested, not just
   asserted.
   **A sibling gap, measured in the same investigation: linearity cannot be declared.**
   `is_copy` (`src/check.rs:239`) derives it structurally, and the only way to opt a struct
   out of `Copy` is to give it a `drop` overload. That matters because a linear aggregate
   accumulator *already* threads through `fold` with zero copies — verified: no `blit` in the
   loop body and stores straight into the carried slot, against four blits per iteration for
   the `Copy`-plus-`dup` form — so the zero-copy path exists and is reachable only by
   spelling "thread this in place" as "give this a destructor". Its answer is entangled with
   slice 1's parked question of whether `Copy` is a privileged constraint and with slice 8's
   polymorphic `drop`: do not settle it here, do not foreclose it either.
   Depends on nothing in 6a-6e; orderable against all of them, required before 7b.

   **Gap closed.** A quotation's captures no longer come from inferring which binding a
   literal belongs to syntactically. `Slot.quot`/`Binding.quot` already record the association
   directly, on both carriers a quotation can occupy — the checker now reads that instead of
   guessing from adjacency. This closes the two shapes the syntactic heuristic missed (a
   quotation separated from its `Bind` by another value; a `Bind` naming two or more
   quotations where a non-topmost one holds the capture) and a third, found while fixing the
   other two: a conflict arising while the quotation is still unbound on the stack, which the
   syntactic approach had no `Bind` to even key off. A quotation's free-name set is a pure,
   cached property of its own literal body; liveness itself is answered fresh at each query —
   a name is alive if any currently-reachable quotation captures it, where a quotation on the
   stack is unconditionally reachable and a bound one is reachable transitively through
   whichever other bound quotations are themselves still reachable — mirroring `live_derivs`'
   existing stack-half/scope-half shape rather than adding a second one beside it.

   **Exit:** a reference bound to a local, used and then finished with, no longer blocks
   consuming the place it borrowed; the in-place accumulator body compiles as written; and a
   test proves the borrow is still rejected when the reference is used *after* the consume.

   **6g — combinator splices never learned 6f's granting rule, so a consumed `Copy`-array
   local is misread as a live alias at every combinator call. Implemented.** `check_terms`
   (`src/check/terms.rs:11`) is the plain root-invocation entry point 6f's own contract names:
   "nothing is ancestor to those, so both [`outer_releasable`, `back_edge`] are empty/false."
   `inline_combinator` (`src/check/combinators.rs:227`) called it unconditionally when splicing
   a combinator's body (`filter`/`map`/`fold`/`while`/any user quotation-taking word), even
   though a splice is exactly the nested-invocation shape `times`/`if`/`call`
   (`src/check/terms.rs:400`, `:856`, `:306`) already routed through `check_terms_relaxed` with
   a `releasable_into`-computed set. Every array in
   Sooth is `Copy`, not because `fill` is the sole constructor (6h added a second, the raw
   `[ Type ; Count ]` constructor) but because `check_no_linear_array_elements` rejects any
   array *type* whose element is non-`Copy` at declaration/registry time, independent of which
   constructor built the value — so naming one never enters `Moves` (`Moves::take`'s own doc:
   "a Copy local never appears" in its map) — `aliasing_origin`'s move-check is permanently
   blind for arrays, leaving `Liveness::dead` as the only remaining guard. Because
   `inline_combinator` never granted the caller's already-consumed local into the spliced
   body's scan, that local had no entry in the splice's own `last_use` table and `dead()`'s
   fallback (`None => self.outer_releasable.contains(name)`) was always false for it, so it
   read as permanently live. Minimal repro, confirmed against the pre-fix compiler:

   ```
   0 4 fill | a |
   a [ 4 > ] c::filter drop drop
   → error: cannot borrow `arr__inl0` mutably: it is aliased by `a`
   ```

   Reported and deferred at 6b's own recon 10 ("a pre-existing 6a inliner limitation... whether
   to fix the underlying alias-after-move tracking is 6a's business, not this slice's") but
   not root-caused until now. The same granting rule was also independently wrong at a loop
   back-edge (a name used earlier in a `times`/`call` body was still granted at a later
   back-edge index), silently accepting a stale read through an alias with no combinator
   involved: `releasable_into` gains `live`/`at`, granting an ancestor name only when
   `live.dead(name, at + 1)`. A second, unrelated defect: `alpha_rename_locals` renames a
   callee's locals but leaves a call to a word/builtin untouched, so a caller local sharing a
   builtin's name was read in place of that builtin inside a spliced body. **D5:** in a
   monomorphic word body, binding a local whose name collides with a builtin, an `env` word, a
   poly word, or a combinator is now a compile error. Inside a generic word's body the check
   covers builtins and `env` words only: `poly_term` has no `PolyCtx`, so `poly.env` and
   `poly.combinators` are out of reach there (`src/check/poly.rs`), and a local named after a
   poly word or a poly combinator still binds. A recorded D5 gap, not a later slice's
   deliverable unless a witness turns up.
   **Fix:** a `releasable_into`-computed `outer_releasable` set, with `back_edge` a constant
   `true` (matching `times`'s own back-edge; not conditioned on self-tail-ness), is threaded
   through `inline_combinator`'s body-check and through the caller-literal check run against a
   combinator's declared parameter effect, switching both from `check_terms` to
   `check_terms_relaxed`, mirroring the three sites that already did this correctly.
   **Exit:** the recon-10 minimal case above compiles clean, as does the same shape through
   `while`, and `lib/arrays.sth`'s own `sort` called with the data and scratch arrays bound to
   locals -- this slice's exit criterion's `sort` dogfood -- runs and returns the sorted array.
   The stale aliasing-workaround paragraph that library's header comment carried for this bug
   (blaming a false "inlining and no-`while`" restructure) is deleted; `sort`'s code and its own
   fixed-bound-`times` rationale are untouched. Mutation tests prove the fix by reverting each
   of the three pieces independently and confirming the corresponding accept goldens fail
   again.
   Depends on 6f (uses its exact granting machinery); independent of 7-10's own mechanisms.
   **Lands before 10c**, which widens this bug's blast radius by turning the last
   compiler-known control-flow form, `if`, into a combinator splice: fixing this first keeps
   that slice from silently multiplying a pre-existing, already-load-bearing false rejection
   across most existing Sooth code.
7. **Functions as values: closures.** ✅ done. The slice that makes a quotation a real runtime value
   rather than a compile-time marker, so it can be branched to, stored, returned, and passed
   to something that is not inlined: `cond [ fast ] [ slow ] if call`, a dispatch table as an
   array of quotations, a strategy in a struct field, and genuine non-inlined higher-order
   words. After slice 6a because the combinator library is the consumer that makes the
   calling convention concrete (designing it with no caller is the anti-pattern this plan
   keeps citing). *This entry used to add "and after slice 6b because it lifts the polymorphic
   `if`"; 6b did not, and never claimed to once its pre-check corrected the charter. The
   polymorphic `if` any interesting closure-taking word needs is 6e, which is therefore a real
   prerequisite here -- now met.*
   **Split 7a/7b, the same reason 6d/6e/6f split off 6a-6c: one dependency the rest of the
   slice does not share. The line is no-captures / captures.** *This paragraph first drew it
   at "non-reference captures / reference captures"; 7a's brief falsified that by probing the
   built compiler before any code changed.* Splicing is textual, so a captured aggregate is
   re-read at the *call* site, while a materialized env would snapshot it at the *literal*:
   measured, a body capturing an array prints `99` today where a snapshot env would print `7`.
   That is a silent wrong value, and it is not academic — `map` in `lib/combinators.sth`
   writes its captured `arr` through `&!` each iteration and reads it back the next, so
   snapshot semantics would break a shipped library word. Preserving today's meaning under
   materialization therefore needs the env to hold a *reference*, which is Phase 3 Slice 6's
   escape checking pointed at a new carrier — exactly the machinery 6f settled the borrow-end
   rule for. So *every* capture waits for 6f, not just an explicitly reference-typed one, and
   what is left for 7a is the representation itself: a quotation that captures nothing has no
   env to disagree about.

   **7a — quotations as values: representation, calling convention, and non-capturing
   quotations. Implemented.**
   **Most of the machinery already exists, which is why this is a slice and not a phase.**
   The seam is already cut: `Type::Quotation`/`PolyType::Quotation` exist with unification and
   `apply_subst` following (6a), and what is missing is strictly downstream — `ir_type_of`'s
   arm is an `unreachable!` whose comment already names this slice as the lift, so the change
   is additive at a known point rather than a refactor.
   This pays the real type cost slice 4 deferred: a quotation must become nameable, so
   `Type`/`PolyType`/`IrType` gain a variant and unification, `apply_subst`, `Subst`,
   `instantiation_symbol` mangling, the monomorphization walk, layout, and the backend all
   follow. That is a slice-1-sized representation change and the second-largest item in the
   phase; slice 4's brief sized it deliberately before deferring it here. Representation:
   one uniform `(code, env)` pair, with 7a's non-capturing quotation carrying an unused env,
   so 7b fills the env in rather than changing the representation (DESIGN.md:480 names
   dynamic dispatch through escaping quotations as a hot-path enemy, so a distinct
   bare-pointer type is the one thing worth pricing at the spec).
   **Which half force-inlining lands in: neither, it stays.** 6a's D2 survives — `each` still
   mints no `IrFunc` and every call site splices it — which is what makes an erased quotation
   compose for free: `table @ each` splices the loop skeleton as always, the abstract
   parameter binds a runtime value, and the `call` inside the spliced body goes indirect.
   Provenance decides (`Slot.quot`, already the right bit and already a one-variant enum),
   never a size budget, which this plan called actively harmful before a fallback existed and
   which nothing about having one improves.
   **Upward closures and `^Env` ride with 7b, not here**: with no captures there is nothing to
   escape *with*, so a 7a quotation is a bare code pointer with no lifetime story at all.
   Depends on 6a (the calling convention's real consumer) and 6e (a closure-taking word that
   branches, now met). Independent of 6f. Brief written
   (`docs/phase4-slice7a-brief.md`).
   Dogfood: rewrite `examples/vm.sth`'s dispatch around a table of quotations and compare it
   against the enum-plus-clause version it replaces.
   **Exit:** a quotation stored in a struct field and in an array, returned from a word, and
   left by two differing branches of an `if`, all compile; `call` on each emits an indirect
   call and runs; `times` driving an erased quotation runs in constant stack; every 6a-6f
   golden still lowers to the same spliced tight loop with no per-element `Instr::Call`; and a
   *capturing* literal reaching a materialization boundary is a located error naming 7b.

   **7b — capturing closures is implemented.**
   Points Phase 3 Slice 6's escape checking at the env-struct carrier 7a introduces, using
   6f's settled last-use rule rather than duplicating the whole-block one it would otherwise
   have to invent standalone. The env holds a reference to a captured aggregate rather than a
   snapshot of it, which is what makes a materialized closure mean what the spliced one means
   (see the split rationale above); that is also what makes this, and not 7a, the slice that
   needs 6f. *This entry previously said the capture-set analysis "needs building, and none
   exists today either way"; 7b's brief found that 6f already built it as a side effect of its
   own liveness fix* — `capture_names`/`quotation_captures` (`src/check.rs:777`/`:544`) cache a
   quotation literal's free-name set by `QuotId` at intern time, a real set, not the bare
   predicate 7a's own D3 needed. *What 7b actually still lacks, which 6f's fix has no reason to
   have built*: a way for that set to survive a materialized quotation's identity erasing —
   `capture_alive_names` only ever reads the set through a still-`Known` marker, and `QuotRef`
   is single-variant by an explicit note that stops holding the moment two different capturing
   literals may join. Upward closures on `^Env` with their single-owner linearity, and the
   `Fn`/`FnMut`/`FnOnce`-equivalent split that falls out of `call` through `&q`, `&!q`, and by
   value, are both new surface and new checking from nothing (verified: `call` pops its
   operand unconditionally today, `src/check.rs:6716`; no reference-mode call exists anywhere).
   Depends on 6f (the exact rule this points at) and 7a (the carrier it points the rule at).
   Brief written (`docs/phase4-slice7b-brief.md`).
   Inherits one obligation from 6f: a quotation's captures are kept alive by whether the
   quotation itself is still reachable (on the stack, or bound and not yet dead, transitively
   through whatever else reaches it), which is sound only while a capturing quotation cannot
   escape the block that binds it. This slice is what lifts that restriction, so it has to
   re-ground that reachability rule against whatever escape rule it settles on rather than
   inherit it unexamined.
   **Exit:** a closure capturing an aggregate, called while that capture is still live,
   compiles and observes the same values the spliced form does; one captured past its last use
   (or, for an upward closure, past its owning frame) is rejected with a located error naming
   the capture; dropping a linear-capturing closure disposes its captures.
   **Deliberate narrowing of the last clause.** The env holds only references (never a
   snapshot of an aggregate, D4), so every closure this slice admits is Copy — there is no
   owned capture and so nothing for a `drop` to dispose. "Dropping a linear-capturing closure
   disposes its captures" is therefore vacuous here, not built; an owning `^Env` closure and
   its linearity are deferred, along with `&q`/`&!q` reference-mode `call` and the
   Fn/FnMut/FnOnce split. What shipped: the four materialization boundaries (struct field,
   array element, word output, branch join) admit a capturing literal via a four-way
   admission rule on capture kind (a scalar snapshots into the env unconditionally; an
   outer-rooted aggregate or reference is admitted at any boundary; a frame-rooted one is
   admitted only in-frame and rejected escaping with a located past-owning-frame error; a
   captured quotation-typed name is rejected as deferred). A one-reference capture stores
   inline in the `env` slot; two or more build a stack-allocated positional bundle, admissible
   only in-frame (an escaping 2+-capture closure needs a heap env, deferred). A new surviving
   capture set on the erased `Slot` keeps a same-frame capture's referent alive past erasure
   through to its `call`, feeding the existing last-use liveness scan so a referent consumed or
   exclusively re-borrowed before the call is a located past-last-use error; a differing-arm
   join unions both arms' surviving sets. `examples/capturing_dispatch.sth` dogfoods a dispatch
   table of same-frame capturing closures against a hand-spliced twin reading the same elements
   directly.

8. **Ad-hoc dispatch: static overloading.** ✅ done. One word name, several statically-known input
   types (`+` over `i64`/`f64`/`Vec2`). After slice 1 because a resolution rule defined over
   concrete types is a rule that gets rewritten once type variables exist.
   **Split 8a/8b, the same reason 7 split into 7a/7b: one piece is bounded, the other is the
   open question.** 8a's table is mechanical, but it is not behaviour-preserving: it is what
   makes user overloads reachable at all (see 8a's rules below). 8b is where `drop` picks up a
   bound it did not have before, exactly the kind of decision Phase 3's own Slice 8 took
   three review rounds to settle when it split into 8a/8b/8c mid-flight for the same reason.
   Landing 8a first also unblocks slice 9's dependency on dispatch early: slice 9 needs `.`'s
   overload (an 8a case) for `Bool`'s type-directed printing, not `drop`'s, so it does not
   have to wait on 8b's argument to be settled.

   **8a — the mechanism. ✅ done** (brief + spec: `docs/phase4-slice8a-brief.md`,
   `docs/phase4-slice8a-spec.md`). The compiler already does this by hand and this slice is where it
   stops: the numeric-tower operators and `.` (type-directed over any printable scalar)
   dispatch on the concrete operand type inside `check_operator`/`check_term` match arms
   rather than through any table — which is why `builtin_table` is empty. `len`'s
   length-polymorphism (slice 1) is the other such site. Retire the hardcoded match arms into
   a real overload table, with a golden asserting every existing operator/`.`/`len` call site
   lowers identically. Brief written (`docs/phase4-slice8a-brief.md`), which found a third
   latent collision while measuring this slice, fixed standalone rather than folded in:
   `qbe_name` (`src/backend/qbe.rs`) mapped every character outside `[A-Za-z0-9_.]` to `_`,
   so `+` and `-` defined in one file both emitted the bare symbol `_` and failed at the
   assembler as already-defined — a general bug reachable today independent of dispatch (two
   ordinary symbol-named words collide the same way), fixed by making the mangle injective
   rather than diagnosed, since there is nothing wrong with a program defining both.

   **Operators are just words, so a user overload needs no new syntax: `: + ( Vec2 Vec2 --
   Vec2 ) ;` is already the definition form, and it already parses and checks.** What blocks
   it is that `check_term` probes builtins by name (`BUILTIN_WORDS`, `check.rs`) *before* the
   word env is consulted at all, so that definition compiles, links, and can never be called:
   a call site with two `Vec2` operands dies in `check_operator`'s `is_numeric` gate before
   any lookup. Sooth today silently accepts a word it will never dispatch to, which is the
   exact class of Forth silent failure this language exists to reject. `drop` is the one
   existing counterexample, resolved by a bespoke registry (`find_drop_overloads`); 8a
   generalises that rather than inventing it. **Six rules, settled:**
   1. **No shadowing.** A user overload whose input types exactly match an existing candidate
      (builtin or imported) is a located error, not a silent override — the same shape as the
      duplicate-word check.
   2. **Exact match beats coercion.** `unify_pair`'s literal/size-type coercion (`check.rs`)
      ranks below an exact-type candidate, so adding an overload cannot silently steal a call
      site that previously coerced.
   3. **Overloads are imported, not carried by the type.** An importer of `Vec2` does not get
      `+` for it without importing `+`. Absence is a resolution error naming the missing
      overload, never a silent fallback. Two candidates with identical input types in one
      scope is rule 1's error regardless of whether they arrived locally or by import.
   4. **One arity per name in scope.** Candidates for a name must agree on how many inputs
      they take, because the resolver has to know how deep to read the stack before it can
      match any candidate. `: + ( Vec2 -- Vec2 ) ;` alongside `: + ( Vec2 Vec2 -- Vec2 ) ;`
      leaves `a b +` matching both, one consuming both operands and one leaving `a`, and both
      check locally. Disagreement is a located error where the second candidate *enters
      scope*: the definition site when one is local, the import site when both are imported.
      It is not a call-site ambiguity to be resolved by ranking; the clash is rejected before
      any call site is looked at.
   5. **Overlap between a concrete and a generic candidate is rejected, not ranked.**
      `: + ( 'T 'T -- 'T )` beside `: + ( i64 i64 -- i64 )` is not identity (a poly word's
      `effect` is empty by construction, so rule 1's textual match never fires) but the
      domains still overlap at every concrete type. No specialization ordering: reject it the
      same shape as rule 1. Nothing today needs a generic default and a concrete override to
      coexist, and inventing ordering semantics for a consumer that doesn't exist is the same
      mistake the generic-struct-declarations item already made once; loosen this later if a
      real consumer asks.
   6. **`.` gets N concrete rows, not a category key.** One row per `IrType` it already
      handles, each an exact-type match tagged to the existing `Instr::Print` lowering. Only
      `.`'s dispatch key changes, to the same exact-match shape every other row uses — which
      is also what makes a user's own `: . ( Vec2 -- ) ;` reachable through the same table.
      `.`'s lowering stays backend code for this slice because moving it is strictly larger
      work, not because it cannot move: `extern:` cannot express a variadic C call *today*,
      but QBE emits variadic calls natively (that is what `Instr::Print` already does), so no
      runtime shim library is needed — an earlier draft of this item claimed otherwise and was
      wrong. The real path to `.` as ordinary library code is DESIGN.md's bounded-row entry
      (`..N`) plus a way to decompose a `str` descriptor for `%.*s`; it would also move `.`
      from universally available to `hosted`-layer only, which is a semantic decision, not a
      refactor. None of that is 8a's job.

   Note what rule 3 costs `drop`, whose absence is *not* an error today but a silent
   structural fallback: see 8b's disposal-scope invariant, which is the reason `drop` cannot
   simply inherit these six rules, and which ends with `drop` no longer being the universal
   disposal verb at all.

   **These rules widen a check that shipped after this plan was written.**
   `check_duplicate_word_names` (`check.rs`) keys on `(module, name)` and exempts only
   `drop`, so two `+` definitions in one file are rejected as duplicates before any overload
   resolution runs. 8a widens that key to `(module, name, input types)` rather than exempting
   overloadable names: an exemption class would reproduce the bespoke registry
   (`find_drop_overloads`) that 8a exists to retire, and two collision checks that have to
   agree with each other is a worse failure mode than one check with a wider key. Deleting it
   and letting table registration own collisions is also wrong: it hands back the bare linker
   `symbol already defined` error that check was added to replace. Enforcement stays at the
   two sites that already exist — that check for definitions, and `check_selective_imports`'
   `selective_collision_error`/`selective_collides_with_local_error` for imports, which are
   already the two halves rule 4 needs. `drop`'s exemption survives 8a and dies in 8b, when
   `drop` moves onto the table; do not delete it early as tidying.

   **The brief's first job is the table's entry shape, which is not `Sig`.** Three measured
   constraints rule that out, all in `src/check.rs`: (a) `builtin_table` is
   `HashMap<String, Sig>`, one concrete effect per *name*, which cannot hold several
   candidates per name at all; (b) `len` is non-consuming over an array of *any* length and
   element type, which is a `PolySig`-shaped entry, not a finite set of concrete rows — so
   either the table carries generic entries from day one or `len` is carved out, and the
   roadmap's claim that `len` is simply absorbed is the thing to check first. `len` is also the
   case proving an entry cannot be a signature alone: its two *shipped* candidates differ in
   lowering and in consumption, not merely in operand type. The array case folds to the constant
   `N` off the type and leaves the array on the stack; `str` consumes its operand and emits a
   runtime `Instr::StrLen` load (`ir.rs`, R8: the length is carried at runtime, not derivable
   from the type). So an entry carries a lowering, not just an effect. A third candidate of the
   runtime kind appears if the deferred view type ever lands (DESIGN.md, *Slicing a buffer into
   a view*, which now records 8a as its ordering gate); the entry shape should not preclude it;
   (c) `.`'s category dispatch and the concrete/generic overlap are both settled above as
   rules 6 and 5, so the only genuinely open question `unify_pair` raises stands alone: it is
   one cross-cutting rule shared by a dozen binary operators, so the spec must say whether it
   runs before lookup (leaving the table to answer only "is this operator defined for these
   types") or becomes table rows (which would multiply entries and lose X10's "needs an
   explicit conversion to `usize`" specificity).
   **Exit:** `check_operator`/`check_term`'s type-directed arms are gone, `builtin_table` is
   populated, the full existing corpus (goldens, examples) is unchanged byte-for-byte, a
   user-defined `: + ( Vec2 Vec2 -- Vec2 ) ;` compiles *and dispatches*, and
   `: + ( i64 i64 -- i64 ) ;` beside `: - ( i64 i64 -- i64 ) ;` in one file compiles and links
   — with rule 1's collision, rule 3's missing import, rule 4's arity clash, and rule 5's
   overlap each a located error, and no definition left silently unreachable.

   **8b — `drop`'s import visibility and destructure guard, plus 8a's own operator
   module-scoping gap. ✅ done** (brief + spec: `docs/phase4-slice8b-brief.md`,
   `docs/phase4-slice8b-spec.md`). Disposal for
   a resource lives on a one-field leaf wrapper where the resource enters the language
   (`type: Fd n i64 ; : drop ( Fd -- ) ... ;`); a composite like `File` holds an undeclared
   `Fd` field and inherits disposal structurally, with no `type:` surface and no named-word
   declaration — `close ( File -- ) drop` is an ordinary word, and `examples/resources.sth`
   already ships this shape. Three things remain, none of them about naming:
   - **8a's module-scoped bare-name gap.** A module's own operator overload is unreachable
     from its own module the moment a second module joins the closure (`resolve::mangle`
     renames the declaration to `+__m1` while the own-module fix in `resolve.rs` leaves the
     call site bare, so byte-identical single-file source compiles and runs), and a
     selectively imported operator hijacks unrelated bare uses of that name in the importing
     module. The qualified form (`v::+`) already resolves correctly — bare names should
     resolve the way qualified ones do, per call site, against candidates visible to the
     calling module.
   - **Disposing an imported resource requires that resource's type to be visible to the
     disposing module.** `check_shuffle`/`lower_call` intercept `drop` before any `env`
     lookup, overrides live in a `StructId`-keyed registry `env` never sees, and `mangle`
     exempts `drop` from per-module renaming, so the ordinary import-visibility check other
     names get from table dispatch cannot reach it structurally. A dedicated gate at the
     interception site (D1) checks the disposed type's owning module against the calling
     module's imports instead: a qualified-only import that never names the type is a
     located error at the `drop`, naming the remedy (import the type by name, or dispose it
     in a module that declares it). A generic `drop ( 'T -- )` caller is unaffected, since no
     disposal word is ever renamed.
   - **Destructuring a type that declares `drop` is rejected.** Moving a field out via the
     generated destructure or field-getter (`R>`, `R>field`) would otherwise bypass whatever
     `drop` override `R` owns, the same hole Rust's E0509 closes (cannot move out of a type
     implementing `Drop`); the guard (D3) rejects it with a located error naming `R`, scoped
     to leaf wrappers only — a composite like `File` has no override to bypass (`File>fd`
     moving the still-linear `Fd` out is correct, not a hole), so the guard fires only where a
     type both owns a resource and declares `drop`.
   **Not this slice's problem:** whether *derived* disposal of a composite holding a resource
   that itself needs extra runtime inputs (a `Vec['T 'A]` field disposed via
   `free ( &!'A Vec['T 'A] -- )`, not `drop`) is possible at all remains open, and belongs to
   Phase 7's allocator-rework question (below); every disposal word in this design is
   `drop ( 'T -- )`, so nothing here answers it.
   **Exit:** a bare `drop` of an imported resource is a located error unless its type is visible
   to the disposing module (imported by name or declared locally), a generic `drop ( 'T -- )`
   still dispatches correctly by monomorphization, destructuring a type that declares `drop` is
   a located error, and a module's own operator overload is reachable from its own module in a
   ≥2-module build with a selectively imported operator no longer hijacking unrelated bare uses
   of that name in the importer, single-module corpus unchanged.
9. **`Bool` as a library enum. ✅ done** (brief + spec: `docs/phase4-slice9-brief.md`,
   `docs/phase4-slice9-spec.md`). `type: Bool | False | True ;` replaces the primitive, via a
   **general** zero-payload-enum → scalar-discriminant layout rule (`Bool` is that rule's
   first client, not a carve-out), so `Cmp`/`Jnz`/bitwise/internal-condition codegen stays
   register-resident and byte-for-byte. `.`'s `bool` row is retired in favour of a library
   `: . ( Bool -- ) ;` reached through 8a's dispatch — the concrete case the ROADMAP always
   cited as the reason to wait on 8a.
   **This entry previously said the whole slice, `if`-as-word included, was "8c-shaped
   delete-the-special-cases work" with dispatch as its only real dependency. Both halves of
   that were wrong, found by attempting it.** `Bool`-as-enum was genuinely
   8c-shaped (mechanical once the layout question was settled) and is the part that shipped.
   `if`-as-word is not: an ordinary clause-bodied word can dispatch on `Bool` today (clause
   dispatch is an independent primitive; the old `clause_bodied_quotation_word_error` guard
   blocking a quotation-taking clause body is stale, its own comment naming slice 7 — now
   shipped — as the intended lift), but its necessary signature,
   `: if ( ..i Bool ~[ ..i -- ..o ] ~[ ..i -- ..o ] -- ..o )`,
   needs a row variable inside a *quotation's declared effect*, which does not parse before
   slice 10a (below). So `if`/`cond`-as-words is split out as **slice 10c** — numbered into
   slice 10's lineage, not slice 9's, because what it depends on is 10a's row mechanism (this
   slice's `Bool` half is shipped and no longer a gate) and it extends that mechanism's own
   representation and grounding code. Findings, including why the two-differing-rows shape
   this entry first assumed was wrong, in `docs/phase4-slice10c-brief.md`.

10. **Rows in quotation effects: `times`, `if`/`unless`/`while` are library combinators.
    10a ✅ done; 10b ✅ done; 10c ✅ done.** The
    self-tail-call loop transform (slice 6) and quotation-parameter splicing (`while`, slice 6)
    are both already general, keyword-free machinery; the one loop shape user code could not
    write was `times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )`, because a row variable `..s` can
    be declared at a word's own top level but not inside a nested quotation's declared effect
    (slice 6a's R2/R28, deliberately deferred). Split 10a/10b/10c, the same shape as 6/7/8's
    splits: **10a** adds the mechanism (parsing and checking a row inside a quotation effect,
    plus the `~` inline-only quotation type below); **10b**
    (`docs/phase4-slice10b-spec.md`) makes `times` an ordinary exported word in
    `lib/combinators.sth` over a self-tail-recursive `times-helper`, itself exported too
    (a private helper reached only transitively would be unresolvable through the REPL's
    `dlopen` import path, which retains exported words only), leaving no
    compiler-known combinator at all — no `check_abstract_quotation_times`, no
    `check_term`/`ir.rs` `"times"` arms. It carries one checker change of its own, not the
pure delete-and-import it looks like: `check_linear_across_back_edge` takes a frame floor
    (`src/check/terms.rs:1060`), exempting a linear local the *enclosing* frame owns and
    disposes from a rejection meant for one the loop actually carries. **10c**
    (`docs/phase4-slice10c-spec.md`, formerly numbered slice 9b) retires `if` as a
    compiler-known construct: `TermKind::If` and the `if`/`else`/`end` grammar are deleted.
    The compiler keeps three machine primitives — `branch` (a 32-bit flag plus two quotation
    operands), `tag` (a scalar enum's discriminant read) and comparisons — and `bool` becomes
    an ordinary payload-free library enum with `if`/`unless` (`lib/core.sth`) and `while`
    (`lib/combinators.sth`) as ordinary words built from them. No `cond` ships. Three designs
    were tried before this one (clause dispatch on `Bool`; hand-written enum eliminators, ruled
    out because a clause body cannot receive a branch quotation); the shipped shape is the only
    one that closes the layering violation — a primitive depending on a library-defined type —
    without reintroducing it.
    **`~[ ... ]`, the inline-only quotation type, lands in 10a rather than 10c.** A row-bearing
    quotation parameter *must* be spliced — `QuotEffect` has no row field and a row's size
    isn't known at runtime — so every combinator today relies on that as an unstated
    guarantee. `~` states it: no runtime representation, not storable, unreachable by the
    runtime `call` path, which also makes a row-bearing quotation reaching an erasure boundary
    structurally impossible rather than a rule bolted onto four call sites. It belongs to 10a
    because if the guarantee holds for every combinator parameter, `~[ ..s i64 -- ..s ]` is the
    honest declaration for `times`'s own parameter, and shipping `times` with a type that then
    has to change is the expensive order. Two questions the spec posed are now settled:
    `call` **stays** the invocation syntax (a `call` on a `~` is statically always a splice,
    so the ban is on materialization, not invocation, and `lib/combinators.sth` needs no
    rewrite); and `~` retypes **only** `times`'s shape in 10a, leaving whether the rest of
    the library becomes the explicit combinator/closure boundary — with ordinary `[ ... ]`
    reserved for genuinely first-class capturing quotations (7b's territory) — to 10b.
    Brief written (`docs/phase4-slice10-brief.md`), which found the row is the
    smaller half of the gap: at every check point a combinator's row is concrete (combinators
    are spliced per call site and mint no `IrFunc`, per slice 6's R18/R20), so there is no
    abstract row unification or `Subst` change, only per-splice depth arithmetic. The bigger,
    row-independent bug the brief's paper pre-check found: the self-tail back-edge arm
    (`src/check.rs`) models its result as the combinator's non-quotation inputs, true only
    for `while`'s state-threading shape and false for any loop that consumes its counters —
    a fully concrete, row-free `times`-shaped combinator fails today with a stack-depth
    mismatch between its `if` branches. 10a fixes that arm (ground declared outputs, plus an
    explicit unify of the self-call's arguments against the ground declared inputs)
    independent of whether rows land. Sequenced after 7b and 8a land (all three touch
    `check_term`'s dispatch spine), not gated on 9. Spec written
    (`docs/phase4-slice10-spec.md`); a first review round found real gaps, so phase 3 derives
    its grounding from scratch rather than generalising a prototype. A second review
    round then rejected the draft on three blockers (the `~` representation was left to the
    implementer while one requirement demanded an outcome only one choice delivers; the
    back-edge rewrite destroyed the positional correspondence the surviving-set forward needs;
    and the grounding mechanism offered two plumbings, one unworkable against an interned
    `QuotEffect`). A fourth round then falsified the representation itself and found three
    ICE-class routing gaps. Settled state after four rounds: `~` is a **distinct `Type`
    variant** (`Type::InlineQuotation`), reached after two dead designs — poly-layer-only,
    falsified because `check_poly_combinator_standalone` grounds every declared input through
    `apply_subst` and hands a `poly: None` stand-in to the monomorphic checker; and a
    value-level `Slot` flag, rejected for failing open when any `Slot` construction drops it.
    The variant was originally rejected on an estimate of 96 affected match sites; measured by
    compiling, it breaks **three**, and it makes the no-widening rule free via structural type
    inequality. `~[` is a single lexer token so adjacency is required; the back-edge rewrite
    builds an explicit bottom-aligned source-index map and extends `SelfTailMarker`, since the
    arm cannot otherwise reach the ground outputs; and grounding is done callee-side in
    `check_literal_against_declared_effect` with a type-only region. The loop counter stays
    `i64`: an interim move to `usize` was reverted, since the honest fix is a bounded
    `'T: Int` (no `Int` bound exists — `Bound` is `{ Copy, Ord }` — and `+` on a bound type
    variable is unsupported), which is its own slice sequenced after 10b. The plan is seven
    phases; the audit of every silent `Type::Quotation` site is phase 1's deliverable and is
    pinned to a pasted grep, since four rounds established that enumerating those sites on
    paper does not converge.
11. **`inline` as a declared word property. ✅ done** (brief + spec:
    `docs/phase4-slice11-brief.md`, `docs/phase4-slice11-spec.md`). `: ClkDiv inline
    ( -- u32 u32 ) 8 4 ;` is spliced at every call site with no silent fallback to a real
    call, `~` is generalised beyond `times` to `lib/combinators.sth`, and
    `check_reference_free_signature` is exempted for every always-spliced word.
12. **Combinator recognition becomes declared, not inferred. ✅ done** (brief + spec:
    `docs/phase4-slice12-brief.md`, `docs/phase4-slice12-spec.md`). `inline` is the single
    route to the always-spliced property. A word declaring a `~[ ... ]` parameter must
    declare `inline`, a located error where it does not, and every library combinator does:
    `times`/`times-helper`/`each`/`map`/`fold`/`filter`/`while` (`lib/combinators.sth`),
    `if`/`unless` and the six comparisons (`lib/core.sth`), `bin_search`/`sort`
    (`lib/arrays.sth`). The tilde is required at the call site as well as in the signature:
    `~[ ... ]` is writable as a literal, and a `~` parameter takes only a `~` literal while
    an ordinary parameter takes only an ordinary one. An ordinary `[ ... ]` parameter is
    therefore a genuine call, minting an `IrFunc` and receiving the quotation as a
    `(code, env)` value (`examples/quotation_argument.sth`); the REPL rejects that shape at
    a located boundary, at both definition and import, rather than lowering it.
