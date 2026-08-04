# Phase 4 Slice 4: quotations + the internal loop primitive

Base: `main` @ `0f88ccb`. Scoped by `docs/phase4-slice4-brief.md`; its "Decisions the
spec has to make" are settled constraints here, not options. Slice 1 gave `Sig`
type/row/length variables and native monomorphization, slice 2 carried them to the REPL,
slice 3 fixed the loop-carried aggregate copy. This slice adds the one piece of the
iteration story that cannot be a library (DESIGN.md:277): a quotation literal `[ ... ]`,
`call` to invoke it, and the internal loop primitive a quotation compiles to for
constant-stack iteration, exposed through the single compiler-known intrinsic `times`.

**Central constraint, from which everything follows.** There is no type, at any layer,
that can name a quotation (recon 2): `Type` (`src/ast.rs:566`), `PolyType`
(`src/ast.rs:406`), `Subst` (`src/ast.rs:443`), and `IrType` (`src/ir.rs:76`) have no
code-value variant, and adding one is a slice-1-sized representation change that only
non-inlined and escaping quotations need, both out of scope (Phase 6). So this slice makes
a quotation a **compile-time-only marker** that carries its body, is consumed by `call` or
by `times` via fusion, and never becomes a runtime value. The `Type`/`PolyType`/`IrType`/
unification/mangling change is deferred to slice 6, where a consumer for it finally exists.

## Locked decisions

- **D1: compile-time marker, no runtime type.** A quotation is a compile-time stack entry
  carrying the identity of its literal body, consumed by `call` or `times` through splicing,
  never lowered to a runtime code value. This defers the entire
  `Type`/`PolyType`/`IrType`/unification/mangling change (recon 2) to slice 6 alongside the
  escaping-quotation fallback that actually needs it. Taking the runtime type by reflex
  reopens the slice-1 representation at its most invasive for capability this slice cannot
  exercise (recon 9); the marker is not shaped to pre-empt that type, it is kept minimal.

- **D2: the marker rides the existing stacks as a phantom entry, forwarded through binds
  and shuffles.** On the checker side a quotation is an ordinary `Slot` (`src/check.rs:64`)
  distinguished by a new side-channel `quot: Option<QuotRef>` field, parallel to the existing
  `alias`/`deriv` discriminators, with a placeholder `ty` no user op accepts. On the lowering
  side it is a phantom `Value` id pushed with **no defining instruction**, recorded in a
  `quot_bodies: HashMap<Value, QuotId>` side map on `FuncBuilder`.

  Forwarding has **two sites, not one, and they are asymmetric across the stages**. Shuffles
  (`dup`/`swap`/`over`/`rot`) move a `Slot` verbatim because `Slot` is `Copy` (`src/check.rs:64`
  doc), so `quot` rides them for free on the checker side, and `lower_call`'s shuffle arms
  (`src/ir.rs:2505`+) move the phantom `Value` id verbatim on the lowering side. A **bind is a
  separate forwarding site on the checker side**: a local is stored as a `Binding`
  (`src/check.rs:613`), a distinct struct from `Slot`, and a local read *reconstructs* a fresh
  `Slot { alias: .., ..Slot::computed(ty) }` (`src/check.rs:4372`) that drops every non-`ty`
  side channel (this is why `int_val` does not survive a bind either). So `Binding` must also
  gain `quot`, forwarded explicitly at the local-read push (R4). On the **lowering** side
  there is no such asymmetry: a local is `self.locals: Vec<(String, Value)>` (`src/ir.rs:2167`,
  `:2439`, `:2452`), which forwards the phantom `Value` verbatim through a bind, so lowering's
  bind and shuffle are one mechanism while the checker's are two. This asymmetry is spelled
  out so a reader does not mistake the extra `Binding` field for an oversight. `Slot` stays
  `Copy` (a `QuotRef` is a `Copy` index).

- **D3: a quotation carries a body, not a pre-computed effect; its effect is realized at
  the consumption site by splicing.** `[ + ] call` type-checks *identically* to writing `+`
  at that point: the checker runs the body's terms against the live stack at the `call`.
  This sharpens the brief's "carries its inferred effect": there is no standalone effect to
  infer (a bare `[ + ]` would underflow an empty stack), and fusion at the consumption site
  is both the checking rule and the lowering rule. `times` realizes the body against the row
  plus a synthesized index and requires the result to equal the row (D6).

- **D4: `call` accepts only a statically-known literal.** With D1/D2, `call` type-checks
  only when the quotation on top of the stack is traceable to a single literal (directly, or
  forwarded through binds/shuffles). A quotation whose identity is lost at a branch merge, or
  that would have to be a runtime value (an array element, a non-inlined word parameter), is
  a **located rejection**, not a panic. These are the exact positions slice 6 later enables,
  so each gets its own diagnostic (R7–R10); diagnostics are behaviour here.

- **D5: the only inlining this slice owns is quotation-literal fusion.** Splicing a
  literal's body at its `call`, or at `times`, is a term-level local fusion in lowering,
  parallel to how builtins and generated struct/enum words already lower as `lower_call`
  arms that never emit `Instr::Call`. It **never crosses a `:` word boundary**; the
  interprocedural user-word inliner is slice 5's (recon 8).

- **D6: the floor is one intrinsic, `times`, passing the index; the IR back-edge machinery
  is reused unchanged.** `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` drives
  `begin_loop`/`finalize_loop` (`src/ir.rs:2301`/`:2348`) plus slice 3's carried-slot
  staging from a quotation loop rather than a syntactic self-`Call` (recon 5). The body
  quotation takes the iteration index and returns the same row it received, so effect
  realization only ever checks an inner row against itself. `while` was weighed as a second
  floor member (DESIGN.md:285 allows "one or two") and **declined** here: its condition
  quotation returns a `bool` on a passthrough row, strictly harder than `times` needs. The
  floor is permanent, not a bootstrap (DESIGN.md:281-289): slice 5 builds its library on
  `times`, it does not retire it.

- **D7: `if` is unchanged, and the polymorphic-path gaps are not this slice's.** `if` stays
  a keyword and stays rejected in a polymorphic body (`src/check.rs:2997`); this slice does
  not lift it to `PolyType` (slice 8). A polymorphic self-tail word still does not get the
  loop transform (`src/ir.rs:1176`). Both gaps land in slice 5 against their first real
  consumers (`filter`, `while`); neither blocks this slice, whose `times` witness is
  monomorphic.

## Requirements by stage

Diagnostics `Rn` marked *(located)* are behavioural negative tests asserting the message
**and** the named identifiers/positions, per the diagnostics-are-behaviour convention.

### Surface syntax / parsing (`src/lexer.rs`, `src/parser.rs`, `src/ast.rs`)

- **R1.** `TermKind` (`src/ast.rs:787`) gains `Quotation(Vec<Term>)`: an ordered term list
  parsed from between a term-level `[` and its matching `]`. Nesting is by construction
  (`[ 1 [ 2 + ] ... ]`), since the element list is `parse_terms`. No new token: `[`/`]`
  already lex (`src/lexer.rs:14-15,87-88`).

- **R2.** `parse_term` (`src/parser.rs:1463`) gains an arm for `Token::LBracket` that
  consumes the bracket, calls `parse_terms("`]` (unterminated quotation)", |t| matches
  `RBracket`)`, expects `]`, and yields `TermKind::Quotation`. The term-level `[` is
  **unambiguous** against the type-level `[`: every existing `[` reader
  (`parse_type_expr` `:1064`, `parse_array_type_expr` `:1159`, `parse_poly_slot` `:883`,
  `parse_field_type_expr` `:1232`) is reached only from signature/type parsing, never from
  `parse_term`, so no disambiguation logic is added; the two grammars simply never overlap.
  Today a `[` in a word body is a hard parse error (`unexpected token LBracket`, the
  `other =>` arm at `:1536`); R2 replaces that arm's reach for `[` only.

- **R3.** An unterminated quotation (`[` with no matching `]` before end-of-word or EOF) is
  a located parse error naming the unterminated quotation *(located)*, reusing
  `parse_terms`' EOF path; a stray `]` with no opening `[` is a located parse error parallel
  to the existing stray-`end`/`else` arm (`:1528`) *(located)*.

### Representation / checking (`src/check.rs`, `src/ast.rs`)

- **R4.** `Slot` (`src/check.rs:64`) gains `quot: Option<QuotRef>`, defaulted `None` in
  `Slot::computed` and every existing constructor (addition-only, R16). `Binding`
  (`src/check.rs:613`) gains the same field, forwarded at the local-read push
  (`src/check.rs:4372`) per D2. `QuotRef` is a **single variant, `Known(QuotId)`**: `Known`
  indexes a per-check side table `Vec<QuotBody>` where `QuotBody` holds the literal's body
  terms (or a `Span` handle into the AST) and the literal's `Span`. There is **no `Merged`
  variant**: a branch join that would merge two *different* quotations is a located rejection
  at the join (R7), so no poisoned marker ever exists to carry, and R12's phantom-containment
  argument no longer depends on rejecting at consumption.

  The placeholder `ty` is pinned to **`Type::Cstr`** (`src/ast.rs:612`), an *existing*
  registry-free scalar, **never an aggregate / `Str` / `OwnedCell` / `Ref`** and **never a new
  `Type` variant** (D1 forbids one, and `Type` has no unit/never variant to borrow). Two
  distinct facts make this pinned, not a free choice:
  - *Registry-free, so it never panics.* An aggregate-shaped sentinel (`Type::Struct(
    StructId(9999), ..)` and its `Enum`/`Array`/`OwnedCell` kin) panics by registry index at
    `is_copy`/`is_linear`/`contains_reference` (`src/check.rs:194`/`:221`/`:240`), and those run
    at `TermKind::Bind` (`src/check.rs:4288`) **before** any R11 guard, so the
    forwarding-through-a-bind criterion would crash rather than diagnose. `Cstr` is a plain
    scalar in all three predicates, so it never panics.
  - *Fewest type-directed acceptors, so the audit is smallest.* A missed R11 guard on the
    placeholder is a **silent accept**, not an error (`match_slot`, `src/check.rs:132`, returns
    `Exact` on `ty` equality and reads no side channel), so the placeholder must be the scalar
    with the fewest sites that accept it. `Cstr`'s only type-directed acceptors are `.` (print,
    `src/check.rs:4782`), the `cstr` word and other `check_str_word` inputs, `extern`/`env`
    arguments, and the equality/comparison operators, **all of which are on R11's audit list**.
    `Bool` is the worst available choice and is deliberately rejected: it is the type of the
    `if` condition (`src/check.rs:4457`) and of `and`/`or`/`xor`/`not` and `.`, so a missed
    guard on the `if` condition would not even mismatch, it would *pass* `cond.ty != Type::Bool`
    and lower a `Jnz` over a phantom `Value` (a silent miscompile, not a diagnostic).

  State the consequence plainly: with `Cstr` (an inhabited scalar), a **missed** R11 guard is a
  silent accept, not an error. That is exactly what makes R11's audit list load-bearing rather
  than defensive, and why R11 is converted to a table-driven test (R11, criterion R11t) rather
  than left as prose. (Pin the lowering-side placeholder `IrType` for the panic reason too: R12.)

- **R5.** A new `TermKind::Quotation` arm in `check_term` (the `fn` is at `src/check.rs:4248`;
  the arm sits beside the other `TermKind` arms) interns the body into the side table and pushes
  a quotation `Slot` (`quot: Some(Known(id))`). It does **not** check the body here (D3): a bare
  body's input row is unknown until its consumption site.

  The mirror `poly_term` arm (`src/check.rs:2990`) **rejects eagerly at the literal**, and
  this is forced, not a preference. `poly_term`'s stack is `Vec<PolyType>` (`src/check.rs:2952`),
  not `Vec<Slot>`, so there is no `Slot` and nowhere to hang `quot`, and D1 forbids a
  `PolyType` variant; pushing `PolyType::Concrete(placeholder)` would erase the identity and
  let it flow into output unification, `Subst`, and mangling. The rejection mirrors the
  existing `if`-in-a-polymorphic-body rejection (`src/check.rs:2997`), whose comment already
  gives the reason (a partial version leaves the stack in a state that panics a later stage).
  Located wording: `` error: a quotation in the polymorphic body of `{word}` (line N) is not
  yet supported ``. The monomorphic `times` witness never enters `poly_term`, so this is a
  clean rejection, not deferred capability.

- **R6: `call` (`src/check.rs` builtin dispatch, `src/ir.rs` `lower_call`).** `call` is a
  new compiler-known word (grep confirms it is absent today), intercepted in `check_term`'s
  Call dispatch before user-word lookup. It requires a quotation `Slot` on top with
  `quot: Some(Known(id))`; it pops it and **splices** the interned body against the live
  stack via the ordinary term checker (`check_terms`, `:4215`), so `[ 1 + ] call` checks as
  `1 +`. The body sees the current locals/scope in lexical extent (capture is free by
  construction, recon 9). No standalone signature; net effect is whatever the splice yields.
  The splice is **bracketed with `scope.depth()` / `leave_block`** exactly as the `if` arms
  are (`src/check.rs:4463-4514`): without it a body that binds (`[ | x | x x + ] call`)
  leaks `x` past the `call`, so a second `call` fails with `rebound_local_error`
  (`src/check.rs:4295`), and a linear value bound inside the body escapes `leave_block`'s
  unconsumed-linear check, which is the only thing that catches it. The `times` body splice
  (R18) is bracketed the same way.

  The splice is driven with the `tail` flag pinned **`false`** (both the `call` splice here and
  the `times` splice in R18), mirroring the lowering-side pin (R13/R14) and forced by the same
  fact. `check_term`'s self-tail path fires on `tail && ctx.word_name() == Some(name)`
  (`src/check.rs:4440`), so a `call` splice inheriting the enclosing term's `tail` in
  `: f ( i64 -- i64 ) ... [ f ] call ;` would check the spliced `f` as a tail self-call and run
  the back-edge checks, while `has_self_tail_call` is **false** (`collect_tail_calls` sees the
  final term as `Call("call")`, `src/check.rs:2116`) so lowering emits a real `Instr::Call` and
  the program recurses. Pinning `tail = false` keeps the checker from validating a loop that
  lowering never builds. On interception order: `call`/`times` are intercepted
  **before every builtin family and user-word lookup** (see R11's dispatch-order list), not
  merely "before user-word lookup"; a local literally named `call` still wins.

- **R7: different quotations at a branch join are a located rejection** *(located)*. A
  monomorphic `if` (which is legal, unlike the polymorphic one) whose two arms each leave a
  quotation in the same stack position merges them at the join (`check_term`'s `If` arm,
  `src/check.rs:4573`). **The rejection fires at the join, not at consumption.** In the merge
  loop, when either merged slot has `quot.is_some()` and the two are not the *identical*
  `Known(id)`, error there. The rejection needs **two phrasings**, because the placeholder is
  `Cstr` (an inhabited scalar), so a branch can genuinely leave a real `Cstr` opposite a
  quotation with `t.ty == e.ty`, and the ordinary `t.ty != e.ty` mismatch (`src/check.rs:4525`)
  never fires to catch it. When **both** arms leave a quotation but not the same `Known(id)`:
  `` error: a quotation's body must be known where it is used, but these two branches leave
  different quotations at line N (a quotation cannot be a runtime value; higher-order values are
  Phase 6) ``. When **one** arm leaves a quotation and the other does not: `` error: one branch
  of the `if` at line N leaves a quotation and the other does not; a quotation cannot be a
  runtime value (higher-order values are Phase 6) ``. The single guard fires for both shapes
  (`either merged slot has quot.is_some() and the two are not the identical Known(id)`); the
  phrasing is selected on whether the *other* slot also has `quot.is_some()`. This is why R4 has
  no `Merged` variant, and it
  is what makes R12's containment true: rejecting at consumption is too late, because
  `lower_if` (`src/ir.rs:3691-3701`) emits a `Phi` for every stack position where the two
  arms' `Value` ids differ, so a merged quotation that is only ever `drop`ped (R11 leaves
  `drop` unguarded) would still reach a `Phi` over two phantoms. Two arms carrying the
  **same** `Known` id (one literal bound before the `if`, read in both arms) do **not** error:
  this is safe precisely because `lower_if`'s `t == e` fast path (`src/ir.rs:3696`) emits no
  `Phi` when the two arm `Value`s are identical. The R7 golden therefore fires at `end`, not
  at `call`.

- **R8: array element is a located rejection** *(located)*. Storing a quotation into an
  array rejects, because it would have to become a runtime value: `` error: a quotation
  cannot be stored in an array (escaping quotations are Phase 6) ``. This must cover **both**
  array-store paths, each witnessed by its **own** golden (criteria R8f, R8r) since one golden
  exercises one path:
  1. `fill` (guard placed *strictly above* its `contains_reference(element.ty, ..)` registry
     index at `src/check.rs:5543`, per R4's registry-free-scalar reasoning).
  2. a store through a reference `&!`/`!`/`+!` (`match_slot(value, referent)` at
     `src/check.rs:5447`). This path is the sharper one and its guard placement is
     load-bearing: the placeholder is `Cstr`, so storing a quotation into a `&!Cstr` referent
     makes `match_slot(value=Cstr, referent=Cstr)` return `Exact` (`src/check.rs:132`), a
     **silent accept**, not a `Mismatch`. The guard must sit strictly above the `match_slot`
     call or the store is accepted with no diagnostic at all. R8r therefore proves guard
     placement, not merely message wording.

- **R9: non-inlined word parameter is a located rejection** *(located)*. Passing a
  quotation as an argument to a user `:` word (or a polymorphic `'T`/row slot) rejects,
  since only `call`/`times` accept a quotation this slice: `` error: a quotation cannot be
  passed to `{word}`; only `call` and `times` accept one (higher-order user words are
  Phase 6) ``. This fires at **two** sites, both before ordinary unification so the message
  is specific, not a generic mismatch:
  1. the `env` argument loop (`src/check.rs:4427`), **before** `match_slot`, so
     `SlotMatch::Mismatch`'s generic message does not win first. This loop also covers
     generated struct constructor/setter fields and `extern` arguments (they are `env` words
     too, `src/check.rs:1011-1013`), so the wording says "word", not "user `:` word",
     to cover them.
  2. `check_poly_call`'s input loop (`src/check.rs:3289`, `fn` at `:3270`), **before**
     `unify_poly_input`. This is not
     optional: `check_poly_call` reads only `stack[base + i].ty` and `unify_poly_input` binds
     a `Var` to *any* concrete type, so a quotation passed to a polymorphic word does not
     fail unification, it **succeeds**, binds `'T` to the placeholder, and monomorphizes a
     real `Instr::Call` passing a phantom `Value`. Without the guard here the R9 diagnostic
     is unreachable for the polymorphic case R9 claims to cover.

- **R10: a quotation left on a word's exit gets its own located diagnostic** *(located)*.
  R10 is **not** the ordinary declared-vs-actual mismatch as first drafted: `check_outputs`
  only reaches the arity message when the counts differ, and on a *matching* count it emits a
  type mismatch that leaks the placeholder spelling (`src/check.rs:2064`). Add an explicit
  quotation-at-exit branch in `check_outputs`: `` error: `{word}` (line N) leaves a quotation
  on the stack; a quotation cannot be a declared output ``, and pin the exact string in the
  golden. (The surplus-linear probe at `src/check.rs:2031` also runs `is_linear` on a surplus
  slot, benign under the registry-free scalar placeholder, R4.)

- **R11: every other consumer rejects a quotation operand, as an audited default-deny**
  *(located, one helper)*. R11 is **not** an enumeration of a few ops; a placeholder `ty` is
  a real scalar and `match_slot` (`src/check.rs:127`) returns `Exact` on type equality reading
  no side channel, so any site that reads a `Slot`'s `ty` for a type-directed decision would
  silently accept it. The rule is therefore: a single guard
  `reject_quotation_operand(ctx, span, op)` (one located wording naming the op) is placed at
  **every** such site, and the spec lists them so a reviewer can re-derive completeness. The
  audited site list, from a walk of `check_term`'s Call dispatch order (locals ->
  `check_reference_word` -> `check_access_word` -> `check_shuffle` -> `check_operator` ->
  `check_str_word` -> `check_array_word` -> `check_owned_cell_word` -> `check_struct_peek_word`
  -> `check_struct_get_word` -> `poly` -> `env`, `src/check.rs:4312-4441`) plus the non-Call
  type-directed reads:
  - `check_operator` and print (`src/check.rs:4612`), conversions, and the `if` **condition**
    pop, which must be guarded **before** `src/check.rs:4457`'s `cond.ty != Type::Bool` return,
    or the generic mismatch (naming the placeholder) wins;
  - `check_str_word` (`cstr`/`len`), `check_access_word`, `check_array_word` and
    `check_array_index` (`:4940`), `check_owned_cell_word`, `check_struct_peek_word`,
    `check_struct_get_word`, `check_reference_word` (`&q` at `:5337` is currently
    `borrow_of_scalar_local_error`, whose message lies about the placeholder);
  - the `env` argument loop (`:4427`) and `check_poly_call`'s input loop (`:3289`) (these are
    R9's two sites, listed here for completeness of the default-deny);
  - the store paths (`:5447`, R8) and `fill` (`:5543`, R8);
  - `check_outputs` (`:2049`, R10) and the self-tail back-edge row (R18);
  - the REPL line boundary (`:1899`, R19).

  **The audit is a test artifact, not prose** (criterion R11t). Because a missed guard on the
  `Cstr` placeholder is a *silent accept* (R4), the completeness of this list is load-bearing
  and must be a regression guard, not a reviewer's one-time walk. R11 adds one table-driven
  checker unit over `(source, op_name)` pairs, one row per audited site (every op
  `check_operator`/print/`check_str_word`/`check_array_word`/`check_array_index`/
  `check_owned_cell_word`/`check_access_word`/`check_reference_word`/`check_struct_peek_word`/
  `check_struct_get_word` recognises, plus the `if` condition, minus the shuffles and `drop`),
  each asserting the shared `reject_quotation_operand` wording names that op. A new consumer
  added later without a guard fails this table.

  **Audit method** (stated so completeness is checkable): the guard covers every site that
  matches, mismatches, or otherwise branches on a popped/inspected `Slot.ty` for a
  type-directed decision; the exceptions are exactly the sites that move a `Slot` verbatim
  (`check_shuffle`) and `drop`, justified below.

  Shuffles (`dup`/`swap`/`over`/`rot`) and `drop` are **not** guarded: shuffles forward the
  marker verbatim (D2), and `drop` of a compile-time-only marker discards it with nothing to
  dispose. One caveat on `drop`: its checker arm pushes the dropped slot's `ty` into
  `prov.dropped` (`src/check.rs:5786`), feeding the drop-override reachability graph
  (`:2334`). Skip that push for a quotation slot (the registry-free scalar is inert in that
  graph regardless, but skipping is cleaner). Lowering's `drop` arm must treat the phantom as
  a pure pop (R12).

- **R18: `times` typing (`src/check.rs`), the checker rule the lowering (R14) presupposes.**
  `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` is a compiler-known word intercepted in Call
  dispatch alongside `call` (R6/R11 ordering). It requires a quotation `Slot` on top with
  `quot: Some(Known(id))` and an `i64` count beneath, and splices the body against the row
  plus a synthesized index via the ordinary term checker, bracketed by
  `scope.depth()`/`leave_block` and with the `tail` flag pinned **`false`** (R6). Four
  obligations, the first two of which the drafted spec got wrong:
  - **The splice must be identity on the move/borrow state, not only on the row.** The body
    is spliced *once* but runs *N* times, so a body that consumes a linear local checks clean
    (`scope.moves.take(name, span)` succeeds exactly once, `src/check.rs:4350`) and disposes
    N times at runtime. The rule is: **clone `scope` before the splice and require it
    unchanged after.** `Scope`/`Binding` derive only `Debug, Clone` (`src/check.rs:604`,
    `:612`), no `PartialEq`, so "unchanged" is spelled out concretely as two comparisons:
    `scope.moves.states` (a `HashMap<String, MoveState>`, `src/check.rs:532`, and `MoveState`
    is `Copy + PartialEq`, `:520`) equal before and after, **and** the derivations live across
    the splice equal before and after. Note `live_derivs` (`src/check.rs:677`) returns
    `impl Iterator<Item = DerivId>`, not a collection, so it cannot be compared with `==`:
    `.collect()` each side into a `HashSet<DerivId>` and compare those. Do **not** "equivalently" invoke
    `check_linear_across_back_edge` / `check_reference_across_back_edge`: those are a *cruder*
    and *wrong* check here. `check_linear_across_back_edge` (`src/check.rs:4062-4083`) errors
    on **any** `is_linear` slot below the args, but for `times` the row *is* the stack, so it
    would reject any linear aggregate carried through the row, contradicting criterion 5c; its
    second clause errors on **any** unconsumed linear local in scope whether the body touches
    it or not; and its message (`linear_across_back_edge_error`) says "self-tail-call
    back-edge", a lie at a `times`. The clone-and-compare rule neither over- nor under-rejects,
    for three reasons: a body that only *borrows* an outer local consumes the derived reference
    within the splice so `live_derivs` is unchanged (accepted, a borrow is idempotent per
    iteration); a body that *binds and consumes its own* locals is fine because `Scope::leave`
    removes departing bindings' `moves` entries (`src/check.rs:661-666`) so the map returns to
    its pre-splice contents; and a body that reads a `Copy` local never enters `moves` at all
    (`:528` doc, `Scope::bind` inserts only for linear values). Located wording: `` error: a
    `times` body cannot consume `{name}` (line N): the body runs more than once, so the value
    would be disposed of more than once ``. This is the single most important checker fix; the
    `0 1000000 [ + ] times` witness never exercises it, so a negative golden is required.
  - **Reject a `times` nested in a loop, here in the checker** (moved from R14 step 0, which
    could not produce a located diagnostic: `src/ir.rs` has no error channel, `grep -c 'Err('
    src/ir.rs` is `0`, and `lower_call`/`lower_term`/`lower_terms` all return `()`). Both cases
    are decidable in the checker, where a span and the diagnostic machinery exist:
    - a `times` inside a **self-tail word**: `has_self_tail_call(word)` (`src/check.rs:2130`),
      the same whole-word predicate lowering uses to decide `begin_loop` (`begin_loop` runs
      before the body, so it is whole-word rather than lexical, matching R14's
      `self.header.is_some()`). **Neither predicate is reachable from `check_term` as the tree
      stands, so both need plumbing (R16).** `has_self_tail_call` takes a `&WordDef`, and
      `check_term`'s only word-level input is `Ctx`, whose `Word` variant carries just
      `name`/`effect`/`structs`/`enums` (`src/check.rs:814`) with no body and no self-tail
      flag. Compute the predicate once in `word_ctx` (`:829`) and carry the resulting `bool`
      on `Ctx::Word`; do not try to thread the `WordDef` itself.
    - a `times` inside **another `times` body**: a splice-depth counter incremented on the
      checker's `times` path around the body splice. No threaded struct has such a field
      today, so add one to `Provenance` (R16). `scope.depth()` is **not** a substitute: it
      increments for every block, not only for a `times`. The counter must be **restored**
      after each body splice, not merely decremented on a success path, or criterion 15's two
      *sequential* `times` would false-positive as nested.
    Located wording: `` error: a `times` cannot be nested in a loop yet (line N): nested
    constant-stack loops need a hoist-target split deferred to a later slice ``. Criterion N is
    a `check_error` golden asserting this wording *with* a line number. R15's save/restore is
    unaffected and still required.
  - **Guard every slot of the row, not just the consumed top.** `times`'s row is `..s`, the
    entire remaining stack; a quotation anywhere in the row reaches `begin_loop`, which does
    `value_type(p)` and emits `Instr::Phi` over a phantom with no defining instruction
    (`src/ir.rs:2311`, `:2329`). Reject a quotation in any row slot at `times` (same wording
    family as R9), and state the same for the self-tail back-edge row (R11's site list).
  - The body's net effect on the row must equal the row (D6); a mismatch is the ordinary
    row-effect error, no new diagnostic. This is a named obligation with its own negative
    golden (criterion R18c).

- **R19: a quotation left on a REPL line's residual stack is a located rejection** *(located)*.
  `infer_line` returns the line's final stack as types (`src/check.rs:1906`) and the session
  persists them across lines; a REPL line has **no declared outputs**, so R10's `check_outputs`
  route does not apply, and the `quot` side channel dies at the boundary while lowering has
  already pushed a phantom `Value` the residual spill would marshal. Add a rejection parallel
  to the existing no-stored-reference sixth position (`src/check.rs:1897-1904`): `` error: a
  quotation cannot be left on the stack at the end of a line: the session carries it into the
  next line, and only `call` and `times` accept a quotation (higher-order values are Phase 6)
  ``. This is slice 4's work, not slice 6's; a golden is required.

### Lowering: fusion + the `times` primitive (`src/ir.rs`)

- **R12: the quotation literal lowers to a phantom, no instruction.** `lower_term`'s new
  `TermKind::Quotation` arm (`src/ir.rs:2410`) interns the body into a `quot_bodies` table,
  mints a fresh `Value` with a placeholder `IrType` and **emits no `Instr`**, pushes it, and
  records `Value -> QuotId`. Because it defines no instruction and the checker guarantees it
  reaches only `call`/`times`/shuffle/bind, this phantom never enters an `Instr` operand, a
  `Phi`, or a `Terminator`. The containment rests on **R7's join rejection** (not on a
  consumption-time check): the only construct that would build a `Phi` over the phantom is a
  branch merge of two *different* quotations (`lower_if`, `src/ir.rs:3691-3701`), and R7 now
  rejects that at the join, so a merged quotation never reaches lowering even when it is only
  `drop`ped. The identical-`Known`-id case reaches `lower_if`'s `t == e` fast path
  (`src/ir.rs:3696`), which emits no `Phi`. Pin the placeholder `IrType` to **`I64`**, a
  non-aggregate scalar, never `Struct`/`Enum`/`Array`/`Str`/`OwnedCell` (the IR side has no
  `if`-condition concern, so the checker's `Cstr` choice does not bind here; any non-aggregate
  scalar is safe and `I64` is the plainest): `dup` blits for
  aggregates (`src/ir.rs:2472-2496`) and `drop` emits a destructor `Instr::Call` for a linear
  aggregate (`:2503`, `:3378-3399`), both dispatching on `value_type`, and R11 deliberately
  leaves both unguarded, so an aggregate placeholder would blit from or call with the phantom.
  Lowering's bind forwarding needs no `Binding` analogue: `self.locals: Vec<(String, Value)>`
  carries the phantom verbatim (D2).

- **R13: `call`-of-literal fusion.** `lower_call`'s new `"call"` arm pops the phantom
  quotation `Value`, resolves its `QuotId`, and lowers the body's terms in place via
  `lower_terms(body, false)` (`src/ir.rs:2399`), emitting **no `Instr::Call`** and creating no
  runtime code value. `[ 1 + ] call` lowers exactly as `1 +`. This is the only inlining
  slice 4 owns (D5) and never crosses a `:` word boundary. The `tail` flag is **`false`**, and
  this is load-bearing: the self-tail arm fires on `tail && self.header.is_some() && name ==
  self.cur_word_name` (`src/ir.rs:2683`), so splicing with `tail = true` inside a word that is
  self-tail-recursive by some other path would back-edge through a path the checker never
  validated (`collect_tail_calls`'s `_ => {}` arm never records a `Quotation`/`call` as a self
  edge, `src/check.rs:2116`). R13 also gets its own lowering unit
  (`call_of_literal_emits_no_call_instr`), reusing criterion 6's `Instr::Call` counting helper.

- **R14: `times` lowering into the back-edge machinery.** `lower_call`'s new `"times"` arm
  drives a constant-stack loop, reusing `begin_loop`/`finalize_loop` (D6):
  0. **Nested `times` is already rejected by the checker (R18), so lowering may assume
     `self.header.is_none()` here.** The rejection does **not** live in lowering: `src/ir.rs`
     has no error channel (`grep -c 'Err(' src/ir.rs` is `0`; `lower_call`/`lower_term`/
     `lower_terms` all return `()`), so a located diagnostic here would need either a `panic!`
     (violating D4/R10's not-a-panic rule) or threading `Result` through all of lowering (the
     refactor R16 disclaims). R18's two checker predicates (`has_self_tail_call` for a `times`
     in a self-tail word, splice-depth for a `times` in a `times` body) are exactly R14's
     would-be `self.header.is_some()` test, decided one stage earlier. The reason nesting
     cannot ride R15 alone (recorded here since it is a lowering fact): `begin_loop`
     unconditionally sets `entry_block = entry` (`src/ir.rs:2307`), so an inner loop either
     hoists its allocs into a block that runs once per *outer* iteration (killing the outer
     constant-stack guarantee) or, if the outer `entry_block` is kept, seeds its stable slot
     once and reads a stale slot on later outer iterations. The clean hoist-target split is a
     later slice's. Lowering may keep a `debug_assert!(self.header.is_none())` documenting the
     checker's guarantee, but emits no user-facing diagnostic.
  1. Pop the phantom quotation `Value` (top) and resolve its body; pop the `i64` **count**.
  2. Synthesize an induction `Value` seeded `Const 0`. Call
     `begin_loop(&[row..., index_seed], true)` where `row` is the remaining stack (the `..s`
     the body threads): each row slot gets its slice-3 carried-slot treatment (scalar phi, or
     an entry-hoisted **stable slot + staging** for an aggregate, `:2301`), and the index gets
     a scalar phi. `stage_aggregates = true` is load-bearing (R17).
  3. In the header (current after `begin_loop`), emit `cmp = Cmp(Lt, index_phi, count)` and
     seal it with `Terminator::Jnz(cmp, body_block, exit_block)` (`src/ir.rs:998`).
  4. In `body_block`: set `self.stack = row_phis`, push `index_phi` (the body reads the
     index as its top input), and splice with `lower_terms(body, false)`. `tail = false` for
     the same reason as R13: with `self.header` set to the *`times`* header, `tail = true`
     would back-edge to it using the word's arity against the loop's carried-slot count,
     panicking in `finalize_loop` or building the wrong loop; the checker never sanctions such
     a call as a tail call (`collect_tail_calls`, `src/check.rs:2116`), so lowering must match.
  5. Compute `index_next = Bin(Add, index_phi, Const 1)`. Require `!self.terminated` before
     sealing (with `tail = false` and no `Return` in a body nothing can terminate, so this is
     a cheap invariant, not a case to handle; a double seal emits a duplicate `BlockId`,
     `src/ir.rs:1527-1530`). Record the back-edge exactly as a self-tail call does
     (`back_edges.push((body_pred, [row'..., index_next]))`, `src/ir.rs:2687`) and seal
     `body_block` with `Jmp(header)`.
  6. `finalize_loop()` back-patches the scalar phis (row scalars + index) and appends the
     aggregate read-before-write staging blits on the back-edge, unchanged from slice 3.
  7. Start `exit_block` and **reset `self.terminated = false`** (step 5's body seal set it, and
     `lower_if` resets the same way at its own joins, `src/ir.rs:3688`+; miss this and every
     term after the `times` is silently dropped). Then `self.stack =` the `Vec<Value>`
     `begin_loop` returned, minus the trailing index. Do **not** describe these as
     "header-phi outputs": an aggregate carried
     slot has no header phi, `begin_loop` returns the entry-hoisted stable slot pointer
     (`src/ir.rs:2312-2323`), and synthesizing an exit phi over it would be a bug. It is sound
     because pass 2's staging writes land before the already-stored back-edge `Jmp`
     (`:2374-2395`), so the stable slot is current at every header entry and at the exit the
     header dominates.

- **R15: `times` saves and restores loop state so it composes.** `begin_loop` sets
  `self.header`/`self.entry_block`/`self.carried_slots`/`self.back_edges`
  (`src/ir.rs:2301`+). R14 saves those four fields on entry and restores them after
  `finalize_loop`. This is required regardless of R14 step 0's nesting rejection, and for a
  different reason: `finalize_loop` `mem::take`s only `carried_slots`/`back_edges`
  (`src/ir.rs:2350-2351`) and **never clears `header`/`entry_block`**, so without the restore a
  `times` in an otherwise-ordinary word leaves `entry_block` set, and any later `Alloc` in the
  same word wrongly hoists into the dead `times` entry block (and a later same-word `times`
  would trip R14 step 0's `debug_assert!(self.header.is_none())`). Restoring `header` to `None`
  is exactly what lets two sequential `times` in one word both run (criterion 15). The headline
  witness `main` opens one `times` on an empty saved state, but the save/restore is correctness,
  not decoration. R15 gets both an end-to-end golden (criterion 15, extended to construct an
  aggregate *after* the first `times` and print its field, witnessing the `entry_block` leak,
  not only the `header` leak) and an `ir.rs` unit asserting all four saved fields
  (`header`/`entry_block`/`carried_slots`/`back_edges`) are back to their pre-`times` values
  after the `times` arm returns.

- **R16: addition-only, but name the forced edits.** Two of these exist only to make R18's
  nested-`times` rejection reachable, and both are additions rather than refactors:
  `Ctx::Word` (`src/check.rs:814`) gains a `self_tail: bool` computed in `word_ctx` (`:829`),
  and `Provenance` gains a `times` splice-depth counter. Beyond those, `Slot` and `Binding`
  each gain a
  defaulted `quot` field; no existing golden or unit test changes expected output; no existing
  `Instr`/`Terminator` variant is added or changed (`Jnz`, `Cmp`, `Bin`, `Phi`, `Blit`,
  `Alloc` are all extant); `qbe.rs` is untouched. Rust has no default field values, so a new
  `Slot` field forces edits at the two **full** `Slot` struct literals, `src/check.rs:4267`
  (the `IntLit` push) and `:4573` (the branch merge, which R7 touches anyway); the
  `..Slot::computed`/`..top` spread sites (`:4372`, `:5693`, `:5745`, `:5778`) are free. Named
  so a reviewer is not surprised, not because a test changes.

### The constant-stack guarantee (`src/ir.rs`)

- **R17.** The loop runs in constant stack because (a) the carried row's aggregates ride
  slice 3's stable-slot staging (`begin_loop(_, true)`), so no carried aggregate re-allocates
  per iteration, and (b) any aggregate the body **constructs** each iteration is emitted
  while `entry_block.is_some()`, so `push_alloc` (`src/ir.rs:2252`) hoists its `Alloc` into
  the entry block (one slot reused every iteration) rather than bumping the stack per
  iteration (slice 3 recon 6: QBE `alloc` emits inline with no hoisting). R14 must keep
  `entry_block` set across the body splice (it is, until `finalize_loop`), so the
  body-constructed aggregate hazard is neutralized by the exact mechanism slice 3 built.

  R17 is witnessed **deterministically** by an IR-shape assertion, not only by the `ulimit`
  subprocess (which is coarse and depends on frame-size arithmetic): for criterion 5a/5b's
  source, every `Instr::Alloc` appears in the loop's **entry block** and **none** in the loop
  **body block** (criterion 6, extended). That is the literal content of R17 and it fails the
  moment hoisting is dropped, without relying on signal behaviour. The 1e6-iteration bounded
  run (5b) stays as a coarse end-to-end backstop; the frame-size arithmetic that gives it
  detection power is pinned in criterion 5a/5b's source (the 16-byte `Vec2` at 1e6 iterations).

## Success criteria

Goldens in `tests/phase4_generics.rs` (the Phase 4 home). A value/effect assertion uses
`run_src` (`tests/phase4_generics.rs:12`, `(String, i32)`); a constant-stack assertion uses
the existing signal-aware `run_stack_bounded_src` (`:234-248`, `ulimit -s 1024`), which returns
`Option<i32>` (the exit code only, **not** stdout), so a semantics claim can never ride it
alone. A diagnostic golden uses a `check_error`/`parse_error` helper this slice **adds** to
`tests/phase4_generics.rs` (declared in *Sanctioned edits*, since roughly ten criteria are
diagnostic negatives and the file has no such helper today, unlike `tests/phase3_locals.rs:59,65`
and `tests/phase3_refs.rs:45`). `run_src`/`run_stack_bounded_src` are unchanged. Naming is
`thing_condition_expected`. `Rn` diagnostics are behavioural negatives asserting message +
named identifiers. Phase labels `2a`/`2b` are the two halves of the split phase 2 (Delivery).

Every source below is **pinned** (R8r pins the store path and guard placement rather than exact
chars, since the reference-store surface syntax is intricate): an unpinned diagnostic golden can
go green on the wrong message, and an unpinned value/constant-stack golden can lose its
detection power (a small iteration count makes the 1 MB bound un-trippable). `SPY_DEF` is the
linear stand-in at `tests/phase3_locals.rs:75` (`type: Spy tag i64 ;` + a `drop` overload); a
source prefixed with it shifts every line number by 2.

| # | criterion (pinned source) | golden | phase |
|---|---|---|---|
| 1 | `[ ... ]` parses into `TermKind::Quotation`; nested `[ [ ] ]` parses | `quotation_literal_parses_into_quotation_term` (parser unit) | 1 |
| 1b | an unterminated `[` is a located parse error (R3) | `unterminated_quotation_is_located_parse_error` (parser unit) | 1 |
| 1c | a stray `]` with no opening `[` is a located parse error (R3) | `stray_closing_bracket_is_located_parse_error` (parser unit) | 1 |
| 2 | `: main ( -- ) 1 2 [ + ] call . ;` prints `3` (fusion runs) | `call_of_literal_quotation_fuses_and_runs` | 2a |
| 3 | quotation forwarded through a bind then called: `: main ( -- ) [ + ] \| q \| 1 2 q call . ;` → `3\n` (cross-checks R4's `Binding` forwarding) | `quotation_forwarded_through_bind_still_calls` | 2a |
| 3b | quotation body reads an enclosing local: `: main ( -- ) 7 \| t \| 1 [ t + ] call . ;` → `8\n` (R6 capture) | `quotation_body_reads_enclosing_local` | 2a |
| 6b | `call` of a literal emits no `Instr::Call` (R13 lowering unit) | `call_of_literal_emits_no_call_instr` (lowering unit) | 2a |
| R12u | `lower_term`'s `TermKind::Quotation` arm emits no `Instr` and records `quot_bodies` (R12 lowering unit) | `quotation_literal_emits_no_instr_and_records_body` (lowering unit) | 2a |
| Cu1 | a quotation survives `dup`/`swap`/`over`/`rot` and a bind (checker unit) | `quotation_survives_dup_swap_and_bind` (checker unit) | 2a |
| R7 | `: main ( -- ) true if [ 1 + ] else [ 1 - ] end drop ;` rejects **at `end`** naming the join line (both quotations) | `different_quotations_at_a_join_are_error` | 2b |
| R7n | one arm leaves a quotation, the other a **real `Cstr`** (equal `ty`, so the ordinary mismatch cannot fire): `: main ( -- ) true if [ 1 + ] else "x" cstr end drop ;` fires R7's *second* phrasing | `quotation_versus_value_at_a_join_is_error` | 2b |
| Cu2 | two different quotations at a join are rejected; the same `Known` id in both arms is not (checker unit) | `merged_quotations_are_rejected_at_the_join` (checker unit) | 2b |
| R8f | `: main ( -- ) [ + ] 8 fill drop ;` rejects at `fill` (guard above `contains_reference`) | `quotation_stored_in_array_by_fill_is_error` | 2b |
| R8r | storing a quotation through a `&!Cstr` reference rejects (guard strictly above `match_slot`, which would return `Exact` on the `Cstr` placeholder and silently accept) | `quotation_stored_through_a_reference_is_error` | 2b |
| R9 | passing a quotation to a user `:` word rejects, naming the word | `quotation_passed_to_user_word_is_error` | 2b |
| R9p | passing a quotation to a polymorphic word rejects (the `check_poly_call` guard) | `quotation_passed_to_polymorphic_word_is_error` | 2b |
| R5p | a quotation literal in a polymorphic body rejects | `quotation_in_polymorphic_body_is_error` | 2b |
| R10 | `: f ( -- i64 ) [ + ] ;` (count matches, so the new branch must beat the type mismatch) gets the dedicated output diagnostic, not a panic and not an arity error | `quotation_left_on_stack_is_output_error` | 2b |
| R11 | `: main ( -- ) 1 [ + ] + ;` (quotation as an operator operand) rejects, naming `+` | `quotation_as_operator_operand_is_error` | 2b |
| R11if | `: main ( -- ) [ + ] if 1 . else 2 . end ;` rejects with the `reject_quotation_operand` wording naming `if`, **not** a `Bool` mismatch | `quotation_as_if_condition_is_error` | 2b |
| R11drop | the one legal unguarded-consumer program: `: main ( -- ) 1 [ + ] drop . ;` → `1\n` (`drop` is a pure pop, R11 carve-out + R12) | `quotation_dropped_is_a_pure_pop` | 2b |
| R11t | table-driven checker unit over `(source, op_name)`, one row per audited site, each asserting the shared `reject_quotation_operand` wording names that op | `quotation_as_operand_is_rejected_at_every_audited_site` (checker unit) | 2b |
| R6br1 | `: main ( -- ) 2 [ \| x \| x x + ] call . 3 [ \| x \| x x + ] call . ;` → `4\n6\n` (fails `rebound_local_error` if the `leave_block` bracket is dropped) | `two_calls_of_a_binding_quotation_body_both_run` | 2b |
| R6br2 | `SPY_DEF` + `: main ( -- ) [ 5 Spy \| s \| 42 ] call . ;` rejects: a linear value bound inside the body is left unconsumed at the `call` | `linear_bound_inside_a_quotation_body_is_error` | 2b |
| R19 | a quotation left on a REPL line's residual stack rejects (R19 wording) | `quotation_left_on_repl_line_is_error` | 2b |
| 4a | headline value: `: main ( -- ) 0 1000000 [ + ] times . ;` prints exactly `499999500000` (via `run_src`) | `times_loop_computes_the_index_sum` | 3 |
| 4b | cheap regression tripwire (not an R17 witness): 4a's source runs under 1 MB, `Some(0)`; emits no `Alloc`, so criterion 6 already witnesses its shape | `times_loop_runs_in_constant_stack` | 3 |
| 5a | `type: Vec2 x i64 y i64 ;` + `: main ( -- ) 0 1000000 [ \| i \| i i Vec2 Vec2>x + ] times . ;` → `499999500000\n` (constructs a 16-byte `Vec2` each iteration; without the R17 hoist that is ~16 MB against the 1 MB bound) | `times_body_constructing_aggregate_computes_expected` | 3 |
| 5b | same source as 5a runs in constant stack, `Some(0)` (R17 end-to-end backstop) | `times_body_constructing_aggregate_runs_in_constant_stack` | 3 |
| 5c | carrying an aggregate **through the row**: `type: Vec2 x i64 y i64 ;` + `: main ( -- ) 3 4 Vec2 0 1000000 [ drop over Vec2>x + ] times . drop ;` → `3000000\n` (first `CarriedSlot::Aggregate` staging from a non-self-tail driver) | `times_carrying_an_aggregate_through_the_row_runs` | 3 |
| 5z | non-zero, non-index seed so the exit cannot alias the index: `: main ( -- ) 7 0 [ + ] times . ;` → `7\n` | `times_zero_trip_yields_seed_row` | 3 |
| R18a | `SPY_DEF` + `: main ( -- ) 5 Spy \| s \| 0 1000000 [ \| i \| s Spy>tag + ] times . ;` rejects naming `s` with R18's "body runs more than once" wording | `times_body_consuming_a_linear_local_is_error` | 3 |
| R18b | a `times` with a quotation anywhere in its row rejects (whole-row guard) | `times_with_a_quotation_in_its_row_is_error` | 3 |
| R18c | body net effect ≠ row: `: main ( -- ) 0 1000000 [ + 1 ] times . ;` fires the ordinary row-effect error | `times_body_changing_the_row_is_error` | 3 |
| R18u | checker unit over R18's three typing obligations (move-state identity, whole-row guard, row-effect equality) | `times_typing_obligations` (checker unit) | 3 |
| N | a `times` nested in a loop rejects **in the checker** with a line number (self-tail via `has_self_tail_call`; `times`-in-`times` via splice depth) | `times_nested_in_a_loop_is_rejected` (`check_error`) | 3 |
| 15 | two sequential `times` in one word both run **and** an aggregate constructed after the first `times` prints its field (R15 restores `entry_block`, not only `header`) | `two_sequential_times_in_one_word_both_run` | 3 |
| R15u | after the `times` arm returns, `header`/`entry_block`/`carried_slots`/`back_edges` equal their pre-`times` values (R15 lowering unit) | `times_saves_and_restores_loop_state` (lowering unit) | 3 |
| 6 | IR-shape: `times` builds a header `Block` with an index header `Phi`, a header `Terminator::Jnz`, a back-edge `Terminator::Jmp`, **no** per-iteration `Instr::Call`, and (on 5a's source) every `Instr::Alloc` in the entry block, none in the body block | `times_lowers_to_a_loop_header_not_a_per_iteration_call` (lowering unit) | 3 |
| 7 | dogfood `examples/times.sth` (`0 1000000 [ 1 + + ] times .`) builds and prints `500000500000`, matching `examples/countdown.sth`'s hand-threaded sum 1..1e6 | `times_example_matches_hand_threaded_countdown` | 4 |

Criterion 6 is the primary direct witness the internal loop primitive gets, since the
primitive is deliberately not user-facing (DESIGN.md:283): it asserts on IR structure (a
header `Block` with an index `Phi` and a `Terminator::Jnz`, reached by a back-edge
`Terminator::Jmp`, the absence of an `Instr::Call` in the lowered `main`, and every `Alloc`
hoisted into the entry block on 5a's source), mirroring slice 3's `header_phis`/`loop_header`
structural tests, not on emitted IL text. The index `Phi` + header `Jnz` are pinned because
"header + back-edge `Jmp` + no `Call`" alone is also true of a one-trip or infinite loop.
Criteria 4/5 are split into a value golden (`run_src`, exact string) and a constant-stack
golden (`run_stack_bounded_src`, `Some(0)`) because the bounded harness never captures stdout,
so a single-number semantics claim would otherwise be a placebo (a `times` that ran zero,
`count-1`, or wrong-index iterations still exits 0). 4b is a **cheap regression tripwire, not
an R17 witness**: 4a's source emits no `Alloc`, so no plausible R14 lowering fails 4b while
passing criterion 6; the real R17 witness is 5b plus criterion 6's entry-block `Alloc`
assertion on 5a's source. Example 7 computes the **same** number as `countdown.sth`
(`[ 1 + + ]` sums `i+1` for `i` in `0..999999`), so the dogfood actually demonstrates parity
rather than a value off by 1e6.

**Unit coverage beside every changed stage function** (CLAUDE.md convention; the three test
mods already exist at `src/check.rs:5833`, `src/parser.rs:1559`, `src/ir.rs:3848`). The table
rows tagged `(… unit)` are the load-bearing ones: R12u (`lower_term` quotation arm), R11t
(`reject_quotation_operand` completeness), R18u (`times` typing's three obligations), R15u
(loop-state save/restore), Cu1/Cu2 (checker forwarding + join). In addition, and required by
the same convention, each of these changed checker functions gets a unit beside its
end-to-end golden: `check_outputs`' quotation-at-exit branch (R10), `infer_line`'s REPL
rejection (R19), `check_poly_call`'s guard (R9p), and `poly_term`'s rejection (R5p). These
are cheaper and more targeted than leaning on the compile-and-run goldens alone, which would
also go red for ten unrelated reasons.

## Sanctioned edits to existing tests

**One sanctioned addition, declared here:** phase 2b adds a `check_error` helper (and, if a
parse-error negative wants it, a `parse_error` helper) to `tests/phase4_generics.rs`,
copied from `tests/phase3_locals.rs:59,65` / `tests/phase3_refs.rs:45`. The file has neither
today, and roughly ten of this slice's criteria are diagnostic goldens, so this is a required
test-harness addition, not a behaviour change; it is called out so "addition-only" is not
contradicted by an unannounced helper. No existing test's expected output changes.

Otherwise none expected. This slice is addition-only at the representation level (R16): `Slot`
and `Binding` each gain a defaulted `quot` field (edited at the two full `Slot` literals
`src/check.rs:4267`/`:4573`, per R16, both addition-only, no test output changes),
`parse_term` gains an arm that only fires on `[`, and `call`/`times` are new dispatch arms.
The criterion-4/5 split rides two **existing** harnesses (`run_src`, `run_stack_bounded_src`)
with no signature change. If a `parse_term` refactor forces a change to an existing
`unexpected token LBracket` negative test, that is the one other place a sanctioned edit could
appear (no test asserts that string today, so it is unlikely); call it out explicitly in the
implementing commit the way slice 3 sanctioned its two phi-count edits, so a reviewer can tell
a sanctioned edit from a silently weakened one.

## Out of scope

A first-class runtime quotation type and the `Type`/`PolyType`/`IrType`/unification/mangling
changes it implies (recon 2, deferred to slice 6); escaping quotations and the
uniform-runtime-stack fallback (Phase 6); the interprocedural user-word inliner and the
`each`/`map`/`filter`/`fold`/`while` library (slice 5); a `while` intrinsic as a second
floor member (D6, declined); lifting `if` to polymorphic bodies and giving polymorphic
self-tail words the loop transform (recon 6/7, both slice 5); `if` as a combinator and
`Bool` as an enum (slice 8); nested/mutual quotation-loop optimization beyond R15's
save/restore; any new `Instr`/`Terminator`; any backend (`qbe.rs`) change; any REPL-facing
quotation work **beyond R19**: a REPL `times` rides the same `lower_call` arm, so it is in by
construction, and no REPL-specific *retention* like slice 2's is added, but a quotation left
on a REPL line's residual stack **is** rejected this slice (R19), not deferred, because a REPL
line has no declared outputs for R10 to catch and the phantom would otherwise be marshalled
into the session's carried stack. (The earlier draft's "no REPL-specific work needed" was
wrong on this half and right on the `times`-rides-the-same-arm half.)

## Delivery

Each phase leaves the tree green (`cargo fmt --check && cargo clippy -- -D warnings && cargo
test`) and coherent.

- **Phase 1, surface syntax + AST (parse only).** `TermKind::Quotation` (R1); `parse_term`
  bracket arm + unterminated/stray diagnostics (R2, R3). Exhaustive-match stubs in
  `check_term`/`poly_term`/`lower_term` so the tree compiles: the checker temporarily rejects
  a quotation with a **deliberately distinct temporary string**, `"error: TEMP-quotation
  consumer not yet wired (phase 1/2a stopgap)"`, chosen so it shares no wording with R5p's
  *permanent* "is not yet supported" diagnostic; phase 2b must delete the stopgap **by that
  exact string**, so a grep for `TEMP-quotation` returns nothing at slice end. Lowering is
  unreachable behind it.
  Exit: criterion 1 (parser unit tests) + green build.

Phase 2 is split into **2a** and **2b** because the original single phase carried a
representation change, two intern arms, the `call` splice, the fusion lowering, *and* the
full located-rejection set (now widened by the blockers); each half leaves the tree green.

- **Phase 2a, the marker + `call`-of-literal fusion.** `Slot`/`Binding` gain `quot` and the
  side table (R4, D2); `check_term` interns a literal and `poly_term` rejects one (R5); `call`
  splices against the live stack, bracketed with `tail = false` (R6); the fusion lowering
  (R12, R13) makes `[ + ] call` end-to-end. Keep phase 1's `TEMP-quotation` stopgap for every
  *other* consumer (one guard, replaced wholesale in 2b), so no panic path opens between 2a
  and 2b. Exit: criteria 2, 3, 3b, 6b, R12u, Cu1.

- **Phase 2b, the located rejections replace the stopgap.** R7 (join, both phrasings), R8
  (fill + reference paths), R9/R9p, R5p, R10, R11's audited default-deny (including the `if`
  condition and the `drop` carve-out), R6's `leave_block` bracketing, and R19 land, deleting
  the `TEMP-quotation` stopgap by string. Exit: R7, R7n, Cu2, R8f, R8r, R9, R9p, R5p, R10,
  R11, R11if, R11drop, R11t, R6br1, R6br2, R19.

- **Phase 3, the `times` intrinsic and the constant-stack loop.** `times` typing (R18: splice
  against row + index, requiring the body return the row, identity on move/borrow state via
  clone-and-compare, the whole-row guard, and the nested-`times` rejection in the checker) and
  its lowering into `begin_loop`/`finalize_loop` with a synthesized index, header `Jnz`, and
  back-edge (R14, `tail = false`, `debug_assert!(self.header.is_none())`), loop-state
  save/restore (R15), and the constant-stack guarantee (R17). Exit: criteria 4a, 4b, 5a, 5b,
  5c, 5z, R18a, R18b, R18c, R18u, N, 15, R15u, 6.

- **Phase 4, dogfood + docs.** Add `examples/times.sth` (`0 1000000 [ 1 + + ] times .`) beside
  `examples/countdown.sth`, computing the same sum; mark ROADMAP.md's slice-4 entry
  implemented; record D1–D6 and the marker/fusion/`times` design in DESIGN.md's iteration
  section. Exit: criterion 7.

## Non-functional / invariants

- Green unchanged: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- No new `Instr`/`Terminator`; `times` and fusion reuse `Jnz`/`Cmp`/`Bin`/`Phi`/`Blit`/`Alloc`
  and the existing `begin_loop`/`finalize_loop` staging.
- Backend stays **QBE**; `Ptr` opaque; no LLVM, no native backend, no JIT/comptime.
- `Type`/`PolyType`/`IrType` gain **no** variant (D1); the runtime quotation type is slice 6.
- `core` stays `no_std`; a non-escaping quotation is core (DESIGN.md:497,512).
- Constant stack preserved: every loop-body `Alloc` is entry-hoisted (R17), no per-iteration
  stack bump, witnessed deterministically by criterion 6's entry-block `Alloc` assertion on
  5a's source and backed end-to-end by the 1 MB bounded run at 1e6 iterations (criterion 5b;
  4b is a cheap tripwire only).

## Where the brief was underspecified, and what this spec did

- **"Carries its inferred effect."** There is no standalone effect to infer for a bare body
  (it would underflow); the spec resolves this as D3, the marker carries the **body**, and
  the effect is realized by splicing at the consumption site. This is strictly simpler and is
  what makes `call` "check identically to writing the body inline" true.
- **"How `Slot` grows, or quotations tracked off-stack."** The spec picks the on-`Slot`
  side-channel (D2/R4) over an off-stack table, because the `Copy`-moved-verbatim property of
  `Slot` gives *shuffle* forwarding for free and mirrors the existing `alias`/`deriv`
  discriminators. A bind is a **second, explicit** forwarding site on the checker side (a
  local is a `Binding`, not a `Slot`, and a local read reconstructs a fresh `Slot`, D2), so
  `Binding` also grows the field; lowering has no such asymmetry. An off-stack table would
  have to re-derive stack ordering by hand.
- **The lowering carrier.** The brief did not fix how a forwarded quotation reaches its
  `call` in lowering. The spec picks the phantom-`Value` with no defining instruction (D2/R12)
  over a checker→lowering Span map, because it needs no new cross-stage channel and reuses the
  fact that `lower_call` already moves `Value` ids verbatim through shuffles/binds; the
  checker's rejections guarantee the phantom never reaches a real `Instr`/`Phi`/`Terminator`.
- **`times` nested in a loop.** The spec **rejects** a `times` nested inside a loop, and keeps
  loop-state save/restore (R15) regardless. The rejection lives in the **checker** (R18), not
  lowering: `src/ir.rs` has no error channel, so a located diagnostic there would need a panic
  or a `Result`-threading refactor R16 disclaims. The checker decides both cases with a span
  in hand (`has_self_tail_call` for a `times` in a self-tail word; splice depth for a `times`
  in a `times` body), which is exactly R14's would-be `self.header.is_some()` test one stage
  earlier; lowering keeps only a `debug_assert!(self.header.is_none())`. Rejection and
  save/restore are not alternatives: rejection closes the hoist-target hazard (an inner loop's
  `Alloc` hoisting once-per-outer-iteration, or seeding a stable slot once and reading it
  stale), while save/restore closes a *separate* hazard (a `times` in an ordinary word leaves
  `header`/`entry_block` set because `finalize_loop` never clears them). The clean
  hoist-target split that would allow nesting is deferred to a later slice.

## Settled since the draft (previously "left for spec review")

The three open questions the draft left are now resolved by the blocker fixes, not left to
the implementer:

- **R5's polymorphic-body arm rejects eagerly at the literal**: forced, because
  `poly_term`'s stack is `Vec<PolyType>` with no `Slot` to carry `quot`, and D1 forbids a
  `PolyType` variant.
- **The placeholder `ty` is pinned to `Type::Cstr`** (R4/R12): an existing registry-free
  scalar (an aggregate-shaped sentinel panics by registry index at
  `is_copy`/`is_linear`/`contains_reference` before any guard, and a new variant is forbidden
  by D1) chosen for the *fewest* type-directed acceptors, all on R11's audit list, so the
  audit (converted to the table-driven R11t) is smallest. It is safe only when paired with
  R11's audited default-deny, since a missed guard is a silent accept, not an error.
- **`examples/times.sth` computes the same number as `countdown.sth`** (criterion 7/F4):
  `0 1000000 [ 1 + + ] times .` sums `1..1e6`, so the dogfood demonstrates parity; criterion
  5 gets its own pinned in-test source rather than doubling as the example.

```json
{
  "phases": [
    { "phase": 1, "focus": "Phase 1, surface syntax and AST: TermKind::Quotation, parse_term bracket arm distinct from type-position brackets, unterminated/stray located diagnostics, exhaustive-match stubs where the checker rejects a quotation with a distinct TEMP-quotation stopgap string (deleted by string in 2b) keeping the tree green", "difficulty": "standard" },
    { "phase": 2, "focus": "Phase 2a, the marker and call-of-literal fusion: Slot.quot and Binding.quot side-channels plus the body side table with a Cstr placeholder ty, check_term interns a literal and poly_term rejects one, call splices against the live stack with leave_block bracketing and tail=false, and the call fusion lowering, keeping phase 1's TEMP-quotation stopgap for every other consumer", "difficulty": "hard" },
    { "phase": 3, "focus": "Phase 2b, the located rejections replace the stopgap: reject two different quotations at the if join (both phrasings), array store (fill and reference paths), user and polymorphic word arguments, a quotation literal in a polymorphic body, the dedicated word-exit output error, R11's audited default-deny as a table-driven test over every type-directed slot read including the if condition and the drop carve-out, R6's leave_block bracketing, and the REPL residual-stack rejection", "difficulty": "hard" },
    { "phase": 4, "focus": "Phase 3, the times intrinsic and the constant-stack loop: times typing (splice identity on row and move/borrow state via clone-and-compare, whole-row guard, row-effect equality, and the nested-times rejection in the checker), lowering into begin_loop/finalize_loop with a synthesized index, header Jnz and back-edge (tail=false, debug_assert self.header.is_none), loop-state save/restore, the constant-stack guarantee, and the split value/constant-stack witnesses plus the IR-shape test", "difficulty": "hard" },
    { "phase": 5, "focus": "Phase 4, dogfood and docs: examples/times.sth beside countdown.sth computing the same sum, mark ROADMAP slice 4 implemented, record the marker/fusion/times design in DESIGN.md", "difficulty": "standard" }
  ]
}
```
