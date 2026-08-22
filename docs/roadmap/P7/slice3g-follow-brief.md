# Phase 7 Slice 3g-follow: the self-tail loop transform for a polymorphic body (brief)

S3g lowers a self-call inside a non-inline generic body to an ordinary recursive
`Instr::Call`, deliberately deferred (D3): correct, but one stack frame per recursion
level, where a monomorphic self-tail word lowers to a loop back-edge instead. This slice
closes that gap. Probe-verified at HEAD, `loopg` from S3g's own golden:

```sooth
: iszero ( i64 -- Bool ) 0 eq ;
: loopg ( 'T: Copy i64 -- 'T )
  dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;
```

`loopg`'s recursive call is in tail position (it is the last term of the `if` combinator's
recursive-arm quotation), yet its lowering today gets `self_tail: false` unconditionally,
so no header/phi loop is built and every recursion level allocates a real call frame.

## Recon (verified against the source at HEAD)

1. **The roadmap frames this as two checker-and-lowering pieces; only one of the two is
   real, and it is smaller than framed.** The roadmap's own text says "tail-position
   detection has to run over a poly body first," implying new checker machinery is needed.
   It is not: `has_self_tail_call`/`tail_position_calls` (`src/check/drop_graph.rs:135-140,
   377-382`) is a purely syntactic name-walk over a `WordDef`'s body, and
   `declared_input_count` (`drop_graph.rs:135-140`) already has an explicit
   `word.poly.as_ref()` branch reading `sig.inputs.len()` — someone already made this
   function poly-aware. **Probe-verified directly, not inferred**: calling
   `has_self_tail_call(loopg_word, &real_combinator_index)` against the exact body above
   returns `true`, correctly recognizing the tail call through the `if` combinator splice,
   with zero changes to `poly.rs`.

2. **`ctx.is_self_tail_call()` inside `check_poly_body` is dead code and is not what
   this slice needs to touch.** `check_poly_body` builds its `Ctx` via `word_ctx(word, ...,
   &CombinatorIndex::new(), ...)` (`poly.rs:392,420-428`), so `Ctx::Word.self_tail_call` is
   always `false` there — but nothing in `poly.rs` ever reads it. `poly_call_term`'s
   self-call arm (S3g) only reads `ctx.mangled_name()`. The roadmap's framing of this as a
   "checker side" piece needing work is a red herring: fixing the empty index here would
   change nothing observable, since its only consumer is unwired.

3. **What actually needs to change is at the two `lower_word_parts` call sites, both of
   which already have everything `has_self_tail_call` needs and choose not to call it.**
   - Native build (`src/ir/driver.rs:262-268`, the poly-instantiation monomorphization
     loop): passes `false` for `self_tail` with a comment claiming `has_self_tail_call`
     "only recognizes a plain-name `Call`, never a `CallInst` lookup" as the reason not to
     reuse it. **This reasoning does not hold up under finding 1's probe**: a self-call
     *is* a plain-name `Term::Call`, exactly the shape `has_self_tail_call` already
     recognizes correctly, no `CallInst` involved either way (S3g finding 6: the checker
     records no `CallInst` for a self-call at all). The comment is stale, in the same
     pattern as two other stale claims already found and fixed in this file this session
     (S3g's own review rounds). The real, still-valid reason `self_tail` is `false` here
     is D3's deferral of the *lowering* back-edge machinery (finding 4 below), not any
     limitation of the tail-detection predicate. `combinator_bodies`, the real populated
     `CombinatorIndex`, is already in scope at this call site (threaded to
     `lower_word_parts` as its `combinators` argument one line below) and is exactly what
     `has_self_tail_call` needs as its second argument.
   - REPL (`src/ir/driver.rs:799-815`, `lower_instantiation`; sole caller
     `src/repl.rs:1554`): also hardcodes `false`, and also already receives a real
     `combinators: &CombinatorIndex` parameter (not the empty index S3g's own brief
     worried about — that empty-index concern was about `check_poly_body`'s `ctx`, a
     different, unrelated call site). The caller in `repl.rs` holds `entry.word`, the full
     retained `WordDef`, at the exact point it calls `lower_instantiation` (`repl.rs:1554`,
     `&entry.word.body` is already passed) — `has_self_tail_call(&entry.word, &bodies)` is
     computable right there with no new plumbing into `lower_instantiation`'s own
     signature, matching how the native path computes it inline before its call.

4. **The real, still-open half is the lowering back-edge dispatch in
   `func_builder/calls.rs`, and its shape is fully determined by two already-built
   mechanisms that don't currently talk to each other.** R7 (`calls.rs:674-701`) fires the
   back-edge (`self.back_edges.push(...)`, `Terminator::Jmp(self.header...)`) when `tail &&
   self.header.is_some() && name == self.cur_word_name`. A poly self-call's AST-level
   callee `name` is always the bare poly name (`loopg`) — never rewritten to a
   per-instantiation symbol like `loopg$$0` — so `name == self.cur_word_name` can never be
   true for it; R7 is structurally unreachable from a poly self-call today, not merely
   untested. R7 also reads arity via `self.env.get(name)` (`calls.rs:687`), which would
   panic on a poly name (the exact reason S3g's own poly arm exists as a separate branch
   ahead of R7, per its comment at `calls.rs:663-664`). S3g's poly arm
   (`calls.rs:662-679`, gated on `self.cur_poly_callee`) runs *before* R7 and today
   unconditionally falls to `emit_user_call`, regardless of `tail`. The fix is a back-edge
   branch **inside** that existing poly arm — `if tail && self.header.is_some() { ... }` —
   using the arity S3g already stores on `self.cur_poly_callee` (`func_builder/mod.rs:754`,
   built from the current instantiation's own `effect`) as the phi/arg-count source instead
   of `self.env.get(name)`, then the same `materialize_quot_args`/`back_edges.push`/`Jmp`
   sequence R7 already runs; falling through to the existing `emit_user_call` call
   unchanged when `tail` is false or no header exists.

5. **`begin_loop` (`func_builder/mod.rs:543-570+`), the header/phi construction R6 already
   drives for a monomorphic self-tail word, is parameterized only by already-lowered SSA
   `Value`s and builder-internal state (`self.header`, `self.entry_block`,
   `self.alloca_home`) — nothing in it inspects whether the enclosing word is polymorphic.
   `lower_word_parts` (`func_builder/mod.rs:779-783`) already calls it unconditionally
   whenever its `self_tail` parameter is `true`, for any caller, poly instantiation
   included: `let entry_values = if self_tail { b.begin_loop(&params_values, true) } else {
   params_values };`. Threading a correctly-computed `self_tail` into this call for a poly
   instantiation should build a working loop header with no changes to `begin_loop` itself
   — the params it phi's are the current instantiation's own concrete SSA values either
   way.

## Locked decisions carried forward from S3g

- **Structural, non-unifying self-call check (S3g finding 5/9).** Untouched by this slice:
  the loop transform is a lowering-only change to *how* an already-typechecked self-tail
  call is emitted, not a new check. No interaction with the termination argument S3g's
  design locked.
- **Ordinary recursive call remains the fallback (S3g finding 7).** A self-call that is not
  in tail position, or whose word has no loop header (non-self-tail poly words, the
  overwhelming majority), keeps lowering exactly as S3g shipped it —
  `emit_user_call(&arity, self.cur_word_name.clone())`. This slice only adds a branch
  ahead of that fallback, never removes it.

## Open questions

1. **Resolved by direct probe, not left open: yes, the hazard is reachable in a poly body
   today, and this slice needs its own guard.** `check/terms.rs:822`'s gate
   (`check_linear_across_back_edge`, `terms.rs:1041-1075`) lives entirely in the concrete
   checker's own term walker, which `poly_walk` never runs through (S3g finding: the two
   walkers are fully separate). Probed directly: a poly self-tail word carrying a linear
   value stranded below its own recursive call's argument window --

   ```sooth
   type: Spy tag i64 ;
   : drop ( Spy -- ) | s | s Spy> drop ;
   : iszero ( i64 -- Bool ) 0 eq ;
   : loopg ( Spy 'T: Copy i64 -- Spy 'T )
     dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;
   ```

   -- typechecks clean (`check::check` returns `Ok(())`) at HEAD, with `loopg`'s
   self-call in the recursive arm genuinely tail-position and `Spy` genuinely stranded
   below the `('T i64)` window the call consumes, the exact shape
   `linear_across_back_edge_error` exists to reject in a monomorphic word today
   ("linear values across a loop are not supported yet" -- itself still a *located
   rejection*, not full back-edge disposal support even in the concrete case, per its own
   doc comment: "Deferred to a later Phase 3 slice"). Nothing in `poly_walk` replicates
   that rejection. **This slice must add a poly-side equivalent of
   `check_linear_across_back_edge`** (adapted to `PolySlot`/the poly stack representation,
   gated identically: tail position, self-name match, `has_self_tail_call`-true) *before*
   `self_tail: true` starts reaching a poly instantiation's lowering -- otherwise this
   exact program silently starts getting the loop transform at lowering with no destructor
   plan for `Spy`, a strictly worse outcome than today's plain stack-depth cost, and an
   inconsistency with the concrete checker explicitly barring the same shape. This is a
   real checker-side piece after all, contrary to finding 1's collapse of the *tail-
   detection* half -- the two are different mechanisms (detecting a tail self-call vs.
   guarding what may cross its back-edge) and only the first turned out to already exist.
2. **Once `self_tail` is correctly computed per-instantiation, does every existing S3g
   golden/test still pass unchanged?** `poly_self_call_lowers_to_ordinary_recursive_call`
   (`src/ir/driver.rs`) and the D3-defers regression test (`b49ef63`,
   `tests/phase7_slice3g.rs`) specifically assert the *absence* of a loop header for a
   self-tail body — those either need to move to a "non-tail self-call" fixture, or become
   this slice's negative regression showing a *non*-tail-position self-call still correctly
   declines the loop.
3. **Is there a length-variable or `θ`-substitution interaction** (S3a's array-length
   discharge, `concrete_effect`) **that changes phi typing across iterations for a
   self-tail poly loop** — i.e., can the loop-carried type ever differ from one call to the
   `has_self_tail_call`-detected recursive call to the next within one instantiation? Not
   traced; likely "no" since a single instantiation is one fixed `θ` for its whole
   lowering, but worth a one-line confirmation in the spec.

## Out of scope

- The self-call check itself (S3g, shipped) — untouched.
- P7.S3k (generic-calls-generic) — a self-tail loop is specific to a call to *the word
  being lowered*; no interaction traced.
- Any change to `resolve::mangle`, `env`, or the `instantiations`/`CallInst` machinery —
  this slice is confined to `has_self_tail_call`'s two call sites (driver.rs, repl.rs) and
  the existing poly self-call arm in `func_builder/calls.rs`.

## The golden

`loopg` (the S3g golden, self-recursing through `if` in tail position) compiled and run at
a large counter (large enough that the current stack-consuming lowering would visibly
either be slow/deep or, ideally, be probed against a stack-depth-sensitive count) showing
constant-stack behavior — the roadmap's own framed exit ("a generic countdown over a large
counter runs in constant stack"). Plus a regression: the *existing* S3g golden and
mangled-name/mismatch tests keep passing unchanged, since none of them have a genuinely
tail-position self-call today (open question 2) and must not silently start building a
loop where none was asserted. Plus, per open question 1's resolved probe, a located rejection for a linear value
stranded below a poly self-tail call's own argument window (the `Spy`/`loopg` shape
above) — the poly-side analogue of `linear_across_back_edge_error`.

## Ready to spec?

**Yes.** Findings 1-5 collapse the roadmap's "two pieces, both absent" into one real
lowering piece (the `calls.rs` back-edge dispatch, finding 4) plus two small,
well-understood plumbing changes (finding 3, computing `self_tail` correctly at two call
sites that already hold everything needed) — but open question 1's probe adds back a
third, genuinely new piece the roadmap didn't name: a poly-side
`check_linear_across_back_edge` equivalent, confirmed necessary by a program that
typechecks clean today and must not once this slice ships. Sizing: **S**, with three
pieces now precisely located rather than two vaguely gestured at — none of them open a new
design question, since the concrete precedent (locate-and-reject, don't yet implement
back-edge disposal) is exactly what the poly side should mirror.

**A stale code comment was found and should be corrected as part of this slice's own
diff**, not filed separately: `driver.rs:265-267`'s claim that `has_self_tail_call` "only
recognizes a plain-name `Call`, never a `CallInst` lookup" as the reason `self_tail` stays
`false` is contradicted by this brief's own probe (finding 1) and should be rewritten to
name the real reason (D3's now-being-closed lowering deferral) once this slice's
`calls.rs` fix lands.
