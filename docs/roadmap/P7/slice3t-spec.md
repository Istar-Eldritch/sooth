# Phase 7 Slice 3t: explicit call-site type instantiation, and the zero-receiver trait member

**Status:** implemented.
**Discovery:** `docs/roadmap/P7/slice3t-brief.md` (written against `6b8affa`; this spec
re-verified every citation against `6b8affa` and corrects two of them, below).
**Roadmap:** `docs/roadmap/P7-language-prereqs.md:668-676`.

## Problem

`trait: Default 'T fresh ( -- 'T ) ;` is rejected at `trait:` declaration time by
`member_binds_trait_var` (`src/check/declarations.rs:367`, wired at `:340-343`), which requires
the trait's variable as bare `'T` or `&'T` in some declared input. A nullary member has no
input, so the door is shut one layer before dispatch is ever attempted, and
`poly_trait_member_call`'s single-candidate path is unreachable for that shape.

Relaxing the gate alone ships a member that can be declared and never called. The brief
established that the missing grounding is **not** at the member call but one frame up, at the
call to the *wrapping generic word*: `check_poly_call` (`src/check/poly.rs:4550`) builds `Subst`
purely by unifying `sig.inputs[i]` against the caller's stack, and at `:4691`
`let Some(ty) = subst.ty_of(*v) else { continue };` skips bound checking and obligation
resolution outright for a variable no input grounds. For `f ( -- 'T: Default )` no input
mentions `'T`, so `subst` never binds it, the bound is never checked, no obligation resolves,
and `apply_subst` (`:5599`) would surface the output variable through
`poly_unbound_output_error` (`:6535`) — a message with zero references anywhere in `src/`,
`tests/` or `docs/`.

> **Phase 2 correction (twice).** The bound must sit in *output* position. A bound-carrying
> occurrence is an input slot, so `f ( 'T: Default -- 'T )` — the spelling this spec used at
> all three of its mentions — takes a `'T` operand that grounds it, and `f[Point]` is
> redundant on it. The zero references are also not evidence of unreachability; see R9.

So this slice is three things in dependency order: a surface syntax that names a concrete type
at a call, a checker path that seeds `Subst` from it, and only then the relaxed declaration
gate.

## Corrections to the brief

- **B1. The `~[` glue *does* generalize to an arbitrary-length word, and the brief is wrong
  that it does not.** The word scan (`src/lexer.rs:170-193`) breaks its loop on whitespace *or*
  a delimiter and leaves the offending char in `chars`; the `~[` test at `:195` is then just
  `text == "~" && chars.peek() == Some(&'[')`. Dropping the `text == "~"` conjunct would glue
  `foo[` and not `foo [`, for any `foo`, with no new lexer state. The brief's "fixed
  one-character sigil peeked immediately after its own lexeme" reading of why it works is not
  what the code does.
  This slice **still does not take that route** (R2): a glued `Word`+`[` token would fire in
  *type* position too, where `Box['T]`, `Slice[T]`, `Result[i64 i64]` and `&![ i64 -- ]` all
  lex as an adjacent word-then-bracket today, and every type-expression reader would have to
  learn the new token. The blast radius is the objection, not the mechanism.
- **B2. `poly_var_conflict_error` already exists** (`src/check/poly.rs:6476`, raised from
  `unify_poly_input`'s `Var` arm at `:5316-5326`) and already handles "this variable was bound
  twice, incompatibly". The brief treats the explicit-vs-inferred mismatch as needing a
  diagnostic invented from nothing; it needs a *sibling* of an existing one (R5).
- Line citations otherwise hold as written: `member_binds_trait_var` `:367`,
  `zero_receiver_member_error` `:375`, `poly_trait_member_call` `src/check/poly.rs:904` (the
  brief says `:899`; the `fn` line is `:904`), `check_poly_call` `:4550`, the `:4691` skip,
  `apply_subst` `:5584`, `poly_unbound_output_error` `:6535`, `resolve_user_bound` `:5145`,
  `Subst` `src/ast.rs:1953`, `parse_term` `src/parser.rs:5184`,
  `parse_type_arguments` `:4689`.

## Design

Surface syntax `f[Point]` at the call site of the generic word, parsed by adjacency on the
existing token spans, carried on `TermKind::Call`, and applied as a seed of `Subst` before
operand unification in `check_poly_call`. Nothing new reaches lowering: a seeded `Subst` is
indistinguishable downstream from an inferred one, and specialization already keys on `Subst`.

### R1 — The syntax attaches to the poly call, not the member call, and only from a concrete caller

`f[Point]`, written wherever `f` is called from a **concrete** (non-generic) body — an
ordinary `main`-shaped word, or an inlined combinator body reached from one. Written from
inside *another* generic word's own body is out of scope for this slice (R3): that path is
checked symbolically by `poly_call_term`, which never calls `check_poly_call` and has no seed
to receive, and reaching it is exactly the multi-hop forwarding case (an abstract `'U` as the
argument) already deferred. `fresh[Point]` on the member call inside `f`'s body is **not**
introduced:
per the brief's probe, `poly_trait_member_call` has no state to receive it and pushes an
abstract `PolyType::Var(var)` output either way. The obligation it records
(`src/check/poly.rs:1034-1039`) stays keyed on `(span, var, trait_id, member)` and is resolved,
unchanged, by `resolve_user_bound` once θ has a binding for `var`.

Bracket syntax rather than a `Type::word` prefix form, because it extends to several variables
(`f[Point Other]`) without inventing a second call form.

### R2 — Adjacency is decided in the parser from token spans, with no lexer change

In `parse_term` (`src/parser.rs:5184`), the `Token::Word(w)` arm looks ahead one token. An
explicit type-argument list opens when the next token is `Token::LBracket` **and** its span is
glued to the word's:

```text
bracket.line == word.line  &&  bracket.col == word.col + w.chars().count()
```

Both spans already exist (`src/lexer.rs:94,111,165`), `col` is incremented per char over the
whole scan, and the word text is exactly the chars consumed, so the arithmetic is exact rather
than heuristic. `src/lexer.rs` is **out of bounds for this slice**; an edit there means the
design went wrong (B1).

The list body is `parse_type_arguments`' loop, reusing `parse_type_expr` per element, but not
`parse_type_arguments` itself: that function takes a known `arity` and reports
`generic_arity_error` against a *type* constructor. A call-site list has no arity known at
parse time (the callee's `PolySig` is not available in the parser), so the parser reads
`Vec<Type>` until the matching `]` and arity is checked in the checker (R4). An unterminated
list is a located parse error naming the call.

**This narrows an existing grammar.** Today `foo[Point]` parses as `Call("foo")` followed by a
quotation literal, identically to `foo [Point]`, and `foo[ i64 ; 3 ]` as a call followed by an
array constructor. Both spellings are re-pointed by this rule. Verified: the `.sth` corpus
(`lib/`, `examples/`, every fixture) contains **no** term-position word-adjacent `[` — every
in-tree adjacency is in type position, which `parse_term` never reaches. The break is therefore
real but unexercised, and the fix is a space. A failure inside the list must say so: when
`parse_type_expr` fails on the first element, the error carries a note reading "a glued
bracket is an explicit type instantiation; insert a space for a quotation or array literal".

### R3 — `TermKind::Call` is widened, not joined by a new variant

`TermKind::Call(String)` becomes `TermKind::Call(String, Vec<Type>)`, empty for every
call without an explicit list. 42 sites in `src/` and 1 in `tests/` construct or match it; the
widening makes each a compile error, so the sweep is compiler-driven.

A *new* variant would be the wrong shape here for exactly the reason the codebase has been
bitten by before: every existing `TermKind::Call(name)` arm would keep matching the old variant
and silently ignore the new one, and an ignored instantiation list is a wrong-symbol link, not
a diagnostic. Widening fails closed.

Most sites correctly ignore the list (`alpha_rename_locals`, the drop graph, capture analysis,
resolve's renaming, lowering — check time already recorded θ on the `CallInst`). Exactly one
site must *act*: the `poly.env` interception in `src/check/terms.rs:718-724`, the sole caller of
`check_poly_call` in the tree. `poly_call_term` (`src/check/poly.rs:1044`), the symbolic
in-generic-body call-checking path, is a **reject** site for this slice, not an act site (R1):
an explicit list written on a call inside a generic body has no consumer here, so it must be a
located error, not a silent drop. Every other checker route that consumes a `Call` — builtin
dispatch, a local reference, a cast, an operator, a combinator inline, a concrete `env` word, a
trait member call — must likewise reject a non-empty list with a located error rather than drop
it:

```text
error: `dup` (line L) takes no type arguments; only a call to a polymorphic word may be
explicitly instantiated
```

The phase's exit requires an *inventory* of all 43 sites classified ignore/act/reject, in the
commit message, not a grep count.

### R4 — Arity is exact, over the callee's declared type variables, positionally

The list binds `sig.ty_var_names` in declaration order, one concrete `Type` each. `n` args
against a callee with `m` declared type variables is an error unless `n == m`:

```text
error: `f` (line L) declares 1 type variable (`'T`) but was given 2 type arguments
```

Not a prefix rule and not "only the ungrounded ones". A partial list would make the meaning of
position `i` depend on which of the callee's variables its inputs happen to ground, so adding
an input to `f` would silently re-point every existing `f[...]` call site. Exact arity keeps the
list's meaning a property of the callee's signature alone.

**Length variables (`'N`) and row variables (`..s`) are not addressable** by this list, which
covers `Subst.ty` only, never `Subst.len`. `len_var_names`/`row_var_names` do not participate
in the count. A length variable is grounded by an array operand's own registry entry and no
zero-receiver case needs it; extending the syntax there is out of scope and cheaply added later
since arity is checked against `ty_var_names` alone.

### R5 — Explicit and inferred bindings must agree; the conflict names the explicit one

The seed is pushed into `subst.ty` before the pass-1 unification loop (`src/check/poly.rs:4592`
onwards), so `unify_poly_input`'s `Var` arm (`:5316`) finds `prev` already bound and takes its
existing conflict branch for free. That branch is redirected: when the prior binding came from
the explicit list, raise `explicit_instantiation_conflict_error` instead of
`poly_var_conflict_error`, because the two sources are not symmetric and the user needs to know
which end to change.

```text
error: `f` in `main` (line L) was instantiated at `'T` = `Point` but its operand is `i64`
```

`poly_var_conflict_error`'s existing text ("resolved `'T` to both `X` and `Y`") stays
byte-identical for the two-operand case; a golden pins that it did not drift.

Discriminating the two needs the seeded ids known at the conflict site. Carry them as a
`Vec<u32>` local in `check_poly_call` (the seeded variable ids) threaded into
`unify_poly_input` as `&[u32]`, not as a new field on `Subst`: monomorph dedup keys on
`instantiation_symbol(&callee, &subst, generation)` (`src/ir/driver.rs:265`, `ast.rs:2056-2074`),
a string rendered from `subst.ty`/`subst.len` in **vector order** — not on a `Subst == Subst`
comparison, which does not occur anywhere in the tree. A provenance field on `Subst` would be
inert to that dedup (harmless to add, just unnecessary); the real hazard R5 must actually guard
is push order: seed the explicit bindings in ascending type-variable id (not `ty_var_names`
declaration order, which need not be ascending, and not interleaved with the pass-1 unification
pushes), so the same θ always renders the same symbol regardless of whether a given call bound
it explicitly or by inference. This matters most once R6 makes redundant explicit
instantiation legal, since that is precisely where a seeded and an inferred path for the same
callee coexist.

### R6 — Explicit instantiation is legal on **any** poly call, not gated to ungrounded variables

Ruling on the brief's first open question, in the permissive direction. `f[Point]` is accepted
even where operand unification alone would have succeeded; R5 makes a disagreement a located
error, so the permissive form has a real check behind it rather than a free-floating
annotation. The gated alternative would make the legality of a call site depend on the callee's
input list, which is R4's objection again.

Not extended: explicit instantiation does **not** participate in overload selection.
`resolve_poly_overload` (`src/check/poly.rs:4283`, called at `:4569`) runs first and unchanged; the list is applied
to whichever candidate it chose, and an arity mismatch against that candidate is R4's error.
Using type arguments to pick an overload is out of scope and stated so.

### R7 — Type arguments are concrete types only; a generic word cannot forward its own variable

`parse_type_expr` resolves a user type name through `resolve_type_or_apply`
(`src/parser.rs:3545-3546`) and has no production for a type *variable*, so `f['U]` inside a
generic word `g ( 'U: Default -- 'U )` does not parse. That is the ruled behaviour, not an
oversight, and it needs its own message rather than `unknown type 'U`:

```text
error: `'U` (line L, col C) is a type variable; an explicit instantiation takes concrete types
  note: forwarding a caller's type variable through an explicit instantiation is not supported
```

Consequence, stated plainly because it bounds what S3t delivers: a zero-receiver member is
reachable from a **concrete** call site through one bounded generic word. Chaining it through a
second bounded generic word is not expressible. That is a genuine gap and belongs in the
roadmap text S3t leaves behind, not silently in a test.

### R8 — Declaration-time relaxation admits the nullary case only

`member_binds_trait_var` (`src/check/declarations.rs:367`) becomes: an input list that is
**empty** is admitted; a non-empty input list must still contain `PolyType::Var(0)` or
`PolyType::Ref(Var(0), _)` directly. The nested-mention rejection (`sum ( ['T 4] -- i64 )`) is
unchanged — grounding it needs structural unification through the array type, which dispatch
still does not attempt, and no syntax in this slice reaches it.

`zero_receiver_member_error` (`:375`) loses its "a nullary member is the zero-receiver case
tracked as P7.S3t" note, since that half is no longer rejected; the nested case keeps its own
text. The function is renamed to match what survives (`nested_receiver_member_error`), because a
message that only ever fires for a nested mention should not be named after the case it stopped
covering.

Selection is unaffected: `poly_trait_member_call` matches the member's declared input list
against the stack window at `base = stack.len() - inputs.len()`, which for a nullary member is
`base == stack.len()` and an empty loop — already correct, and the reason S3p's ruling 3 needs
no amendment. Ambiguity is still counted per variable (S3p ruling 4); a nullary member of two
traits on one variable is the unchanged single-variable ambiguity error, and across two
variables `candidate_fitting_the_operands` (`src/check/poly.rs:814`) fits *every* nullary
candidate, so that path must report `ambiguous_trait_member_error`, not silently take the first.
That is a golden, not an assumption.

### R9 — `poly_unbound_output_error` is revived as a deliberate diagnostic, not deleted

Ruling on the brief's fifth open question. Once R8 lands, `f ( -- 'T: Default )` called as a
bare `f` with no explicit list and no grounding operand is a legal program shape that reaches
`apply_subst`'s `Var` arm (`:5599`). The existing message stays word for word and gains a note
pointing at the new syntax:

```text
error: `f` in `main` (line L) has output variable `'T` that no input binds
  note: supply it explicitly: `f[SomeType]`
```

**Phase 2 correction: the arm is not reached "for the first time", and the message has a
second, wrong-worded caller.** `check_poly_call`'s pass 2 grounds each declared *quotation
input* through `apply_subst` too (P7.S3l), so `: q ( [ 'T -- ] 'U -- 'U ) swap drop ;` called
as `[ drop ] 8 q` has reached the `Var` arm since long before this slice — verified by
building it at the slice's parent commit. There the message reports an "output variable `'T`"
for a `q` that declares no output variable at all. R9 freezes the text, so **this slice does
not reword it**: the input-position misdescription is a recorded open gap, and a later slice
that touches this diagnostic should split it (output case keeps the text, input case names the
quotation parameter). What R9 revives is the message's *use*, not its reachability.

The `:4691` `continue` in the bound loop stays as it is, and its comment is corrected: the claim
"no obligation can name a variable the body could not have dispatched on" becomes false under
R8, but the `continue` is still right, because an ungrounded variable that reaches an output is
caught by `apply_subst` above, and one that reaches no output is genuinely unconstrained. The
comment is load-bearing documentation of a now-different reason and must be rewritten, not left.

### R10 — The REPL rejects an explicit instantiation, located

`impl:` and `trait:` are already located REPL rejections (S3r R8), so a zero-receiver member
cannot be *declared* in a session. But an imported poly word can be called, and the REPL routes
through `lower_instantiation`, bypassing the module-level checks — the documented shape where a
session prints success and binds the wrong thing. So the REPL rejects the syntax outright rather
than inheriting an unverified path:

```text
error: explicit type instantiation is not available at the REPL (line L, col C)
  note: `f[Point]` needs a whole-program impl registry a live session does not assemble
```

This is a guard, not a feature: it is cheap, it fails closed, and it keeps the REPL out of the
slice's correctness argument entirely.

## Codebase map

| Anchor | Role in this slice |
| --- | --- |
| `src/lexer.rs:170-199` | the word scan and the `~[` glue — **read, not edited** (R2, B1) |
| `src/parser.rs:5184-5230` | `parse_term`'s `Word` arm; R2's adjacency lookahead |
| `src/parser.rs:4689-4713` | `parse_type_arguments`, the element-loop shape R2 mirrors |
| `src/parser.rs:3528-3548` | `parse_type_expr`, reused per element; R7's rejection sits here |
| `src/ast.rs:2624-2653` | `TermKind`; R3's widening of `Call` |
| `src/ast.rs:2665-2700` | `alpha_rename_locals` / `rename_local`, an ignore-site the sweep hits |
| `src/check/terms.rs:718-724` | the `poly.env` interception; one of R3's two act-sites |
| `src/check/poly.rs:4550-4600` | `check_poly_call`, overload pick then `Subst` build; R5's seed |
| `src/check/poly.rs:5294-5330` | `unify_poly_input`'s `Var` arm; R5's redirected conflict |
| `src/check/poly.rs:4682-4695` | the ungrounded-variable `continue` and its now-false comment (R9) |
| `src/check/poly.rs:5584-5602` | `apply_subst`; R9's revived reachability |
| `src/check/poly.rs:6476-6494` | `poly_var_conflict_error`, whose text must not drift (R5) |
| `src/check/poly.rs:6535-6542` | `poly_unbound_output_error` (R9) |
| `src/check/poly.rs:904-1041` | `poly_trait_member_call` — **unchanged**; R8's empty-input path |
| `src/check/poly.rs:5145-5179` | `resolve_user_bound`, unchanged: it takes a resolved `Type` |
| `src/check/declarations.rs:340-382` | the gate and its message (R8) |
| `src/repl.rs` | R10's located rejection |

## Tests

End-to-end, `tests/phase7_slice3t.rs` (through the real binary, each negative pinning the exact
diagnostic string, never `is_err()`):

- `a_zero_receiver_member_dispatches_through_an_explicit_instantiation` — the headline. Two
  impls of `Default` (`Point`, `Other`) with distinguishable `fresh` bodies, a bounded
  `f ( -- 'T: Default )`, and `main` calling `f[Point]` and `f[Other]`; the printed output
  discriminates *which* impl ran. A golden that only proves it compiles is a placebo here, since
  a wrong-symbol link is exactly the failure mode. **The spelling is load-bearing** (phase 2
  finding): `f ( 'T: Default -- 'T )` declares a `'T` *input*, which an operand grounds, so it
  cannot witness a zero-receiver call at all. Probed at the end of phase 2:
  `: f ( -- 'T: Copy ) f ;` builds under `f[i64]`, and under `f[Res]` (a struct with a `drop`
  overload, hence linear) reports that `Res` is linear and has no `Copy` instance; the same
  signature at `'T: Ord` rejects `f[Blip]`. So the seed already reaches the bound loop, and
  phase 3 is a declaration-gate change only.
- `an_uninstantiated_bounded_call_names_the_unbound_output` (R9), pinning the revived message
  plus its new note.
- `an_explicit_instantiation_disagreeing_with_an_operand_is_rejected` (R5) and
  `two_operands_disagreeing_still_report_the_old_conflict` — the blast-radius guard on
  `poly_var_conflict_error`'s unchanged text.
- `an_explicit_instantiation_on_an_already_inferable_call_is_accepted` (R6).
- `a_wrong_arity_instantiation_is_rejected` (R4), both directions (too few, too many).
- `a_spaced_bracket_after_a_word_is_still_a_quotation` (R2's non-break witness) and
  `a_glued_bracket_after_a_builtin_is_rejected` (R3's reject-site, e.g. `dup[Point]`).
- `a_type_variable_argument_is_rejected` (R7), pinning the note rather than `unknown type`.
- `a_nested_receiver_member_is_still_rejected` (R8), pinning the surviving message.
- `a_nullary_member_of_two_traits_on_one_variable_is_ambiguous` and
  `a_nullary_member_across_two_variables_is_ambiguous` (R8's two ambiguity paths).
- `explicit_instantiation_is_rejected_at_the_repl` (R10).

Unit, beside the code:

- `src/parser.rs`: the adjacency predicate both ways on one source pair; the unterminated-list
  error; the malformed-first-element note; a multi-argument list.
- `src/check/poly.rs`: `check_poly_call` with a seeded `Subst` — accept, conflict, arity — and
  `unify_poly_input` finding a seeded `prev` and taking the explicit branch.
- `src/check/declarations.rs`: `member_binds_trait_var` accepting the empty input list and still
  rejecting the nested one (extend `member_binds_trait_var_accepts_any_receiver_position`'s
  neighbours rather than adding a fourth near-duplicate).

**Mutation-test before each phase exit**, deleting what each guards and proving the test fails:
R2's adjacency conjunct (delete the `col` test — `a_spaced_bracket_after_a_word_is_still_a_quotation`
must fail), R3's reject-arm (delete it — `a_glued_bracket_after_a_builtin_is_rejected` must
fail), R5's seed ordering (move the seed after unification — the conflict golden must fail), and
R8's empty-input admission (restore the old predicate — the headline must fail). The headline
golden's impl-discrimination is itself a mutation subject: swap the two impls' bodies and it must
fail.

## Phase 1 — call-site explicit type-argument syntax (hard)

**Scope.** `src/parser.rs` (R2, R7), `src/ast.rs` (R3's widening and the 43-site sweep),
every file the sweep touches for the ignore/reject classification (R3), `src/repl.rs` (R10),
plus the parser unit tests and the four parser/reject goldens above.

**Out of bounds.** `src/lexer.rs` (R2/B1), `src/check/poly.rs`'s `Subst` handling,
`src/check/declarations.rs`, `src/ir/` beyond whatever the widening mechanically forces,
`lib/`, `examples/`.

**Entry.** None; `6b8affa` is green.

**Exit.** A non-empty list parses and reaches `check_poly_call` (where phase 1 still ignores
it — this phase's own goldens are the parse, the reject-sites, and the REPL); the 43-site
inventory is in the commit message classified ignore/act/reject; the two named mutation checks
fail when their guard is removed; `cargo fmt --check && cargo clippy --all-targets -- -D
warnings && cargo test --no-fail-fast` green.

**Note.** No pre-staged plumbing: the list must be *consumed* somewhere in this phase or clippy
kills the unused field. Phase 1's consumers are the reject-sites, which is a real consumer, not
a placeholder.

## Phase 2 — seed `Subst` from the explicit list (hard)

**Scope.** `src/check/poly.rs` (R5's seed and conflict, R6, R9's revived diagnostic and the
corrected `:4682-4685` comment), `src/check/terms.rs:718-724` (R3's act-site), the poly unit
tests, and the R4/R5/R6/R9 goldens.

**Out of bounds.** `src/check/declarations.rs` (phase 3), `poly_trait_member_call`,
`resolve_user_bound`, `src/ir/`, monomorphization.

**Entry.** Phase 1 landed and green.

**Exit.** `f[Point]` grounds `'T` on an *ordinary* generic word end to end (no trait member
needed yet — this phase is observable on its own, which is why it is its own phase); the R9
message is reachable and pinned; `poly_var_conflict_error`'s text is unchanged; full green.

## Phase 3 — relax the declaration gate and dogfood `Default` (standard)

**Scope.** `src/check/declarations.rs:340-382` (R8, including the rename), the two ambiguity
paths' goldens, the headline dogfood golden, and the roadmap text at
`docs/roadmap/P7-language-prereqs.md:630-634,668-676` (the gate's description and S3t's entry,
the latter recording R7's remaining gap).

**Out of bounds.** Anything in `src/parser.rs` or `src/check/poly.rs` — if phase 3 needs an edit
there, phases 1-2 were wrong and that is the finding to report. `lib/` (no library trait gains a
nullary member in this slice).

**Entry.** Phase 2 landed and green.

**Exit.** The headline golden runs and discriminates the two impls; the impl-swap mutation
fails it; a second golden grounds the wrapping word's `'T` from an ordinary operand instead,
so it isolates the gate from phases 1-2's seed (the seed mutation must leave it green and the
pre-S3t predicate must red it); the nested-receiver rejection keeps its lead sentence and its
"nested inside a composite input" note, losing only R8's retracted `P7.S3t` clause; full
green.

## Out of scope

- `fresh[Point]` syntax on the trait-member call itself (R1).
- Length (`'N`) and row (`..s`) arguments (R4).
- Explicit arguments as an overload discriminator (R6).
- Forwarding a type variable through an instantiation, and therefore any chain of two bounded
  generic words to a zero-receiver member (R7) — a stated residual gap. Also out of scope: an
  explicit-instantiation call written inside another generic word's own body at all (R1/R3),
  since that path (`poly_call_term`) has no seed to receive one and reaching it is the same
  multi-hop case. Both track as a follow-on slice that depends on the existing, separately
  unresolved generic-calls-generic gap (an `inline` callee is the only shape that works
  end-to-end today) — this slice must not invent a parallel forwarding mechanism just for
  `Default`.
- The nested-composite receiver (`sum ( ['T 4] -- i64 )`), still rejected (R8).
- REPL support (R10 is a rejection, not an implementation).
- Lowering, IR, monomorphization: a seeded `Subst` is structurally identical to an inferred one
  and mints the same specialization by the same key.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "call-site explicit type argument list parsed by span adjacency, TermKind::Call widened, non-poly sites reject", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "seed Subst from the explicit list in check_poly_call with conflict and arity diagnostics", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "relax member_binds_trait_var to admit a nullary member and dogfood Default/fresh", "effort": "S", "difficulty": "standard" }
  ]
}
```
