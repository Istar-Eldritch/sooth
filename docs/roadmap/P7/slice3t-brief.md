# P7.S3t brief — Explicit call-site type instantiation for a zero-receiver trait member

## Problem, confirmed live against current `main`

`trait: Default 'T fresh ( -- 'T ) ;` is rejected at **`trait:` declaration time**, before
`impl:`, before any body, before any call site:

```
$ cargo run -- build /tmp/probe1.sth
error: error: trait member `fresh` of `Default` (line 1, col 8) never takes `'T` (or `&'T`)
directly as an input, so a call has nothing to dispatch on
  note: a variable nested inside a composite input (an array element, say) does not count;
  a nullary member is the zero-receiver case tracked as P7.S3t
```

`member_binds_trait_var` (`src/check/declarations.rs:367`), consulted from `check_trait_decls`
(`:341-346`), requires the trait's variable as `'T`/`&'T` directly in some input; a nullary
member has none. This means **`poly_trait_member_call`'s `matched.len() == 1` path for a
nullary member is unreachable dead code today** — the only door into the language currently
in existence for this shape is closed one layer before dispatch is ever attempted.

## The naive framing is wrong: the syntax does not belong on the trait-member call

The obvious reading of "supply theta explicitly" is `fresh[Point]`, written where `fresh` is
called, inside a bounded generic word's body (`f ( 'T: Default -- 'T ) fresh ;`). Probed and
rejected as the wrong target:

`poly_trait_member_call` (`src/check/poly.rs:899-1041`) has **no state anywhere in its scope**
representing "the concrete type this bound variable should ground to," and would need none
even with the declaration gate relaxed: with `inputs = vec![]` (a nullary member's declared
input list), the underflow check trivially passes, the per-input operand loop doesn't
execute, and the outputs pushed (`:1031-1033`) are `substitute_member_var(out, var)` —
**still abstract** (`PolyType::Var(var)`), not a concrete type. The obligation recorded
(`:1034-1039`, `TraitObligation { span, var, trait_id, member }`) carries only the bound
variable id.

Concrete grounding happens one call-frame **up**, in `check_poly_call` (`:4550`) — the check
of a call to the *wrapping generic word* (`f`), not the trait member (`fresh`). `Subst` is
built purely by unifying `sig.inputs[i]` against the caller's stack (`:4578-4591`); at
`:4691`, `let Some(ty) = subst.ty_of(*v) else { continue };` **skips bound-checking and
obligation resolution entirely for a variable no input of the wrapping word grounds**, with an
explicit comment (`:4682-4685`) calling this correct today: "no obligation can name a variable
the body could not have dispatched on." Since `f`'s own signature `( 'T: Default -- 'T )` never
mentions `'T` in its inputs either, `subst` never binds it — **this is a second, independent
gap one frame above the trait member itself, and it is the one that actually blocks
resolution end-to-end.**

`apply_subst`'s `PolyType::Var` arm (`:5599-5601`) is the only place an unbound output
variable would surface, via `poly_unbound_output_error` (`:6535-6542`, `"has output variable
'{var}' that no input binds"`). Grepped the whole tree (`src/`, `tests/`, `docs/`): zero
references to that exact string anywhere except its own definition. **It is dead code today**,
guarding a case (a word's own bound variable, unbound by any of its inputs) that no legal
program can currently construct — the only route to such a signature is a trait member, and
S3p's declaration gate forecloses it before this code is ever reached.

`check_poly_call` is invoked from ordinary (non-generic) bodies too (`src/check/terms.rs:720`),
not only from other poly bodies — so `main` (concrete) calling `f[Point]` directly would route
through exactly this function. **The syntax this slice needs is `f[Point]` at the generic
word's own call site, not `fresh[Point]` on the member call inside its body.**

## Parser: `word[Type]` collides with existing grammar, confirmed live

`parse_term` (`src/parser.rs:5184`) has no lookahead from a `Word` token to a following
bracket; the two existing `[`-in-term-position arms (array constructor, `:5242`, triggered by
a top-depth `;`; ordinary quotation literal, `:5243-5252`) are reached independent of the
preceding token. Confirmed:

```sooth
: foo ( -- i64 ) 1 ;
: main ( -- i64 ) foo[Point] ;
```

```
error: error: `main` (line 2) leaves a quotation on the stack; a quotation cannot be a
declared output
```

The check-time (not parse-time) shape of that error confirms `foo[Point]` parses today as two
separate terms — `Call("foo")` then a quotation literal containing `Call("Point")` — identical
to `foo [Point]` with a space. `[` is a hard lexer delimiter (`is_delimiter`,
`src/lexer.rs:31`) that terminates a `Word` scan regardless of intervening whitespace, so **the
lexer already discards the space/no-space distinction for `word[...]` by design.** The one
existing precedent for adjacency-sensitive lexing, `~[` (`src/lexer.rs:22-27`, glued with zero
whitespace vs. `~ [` as two tokens and a parse error), works only because `~` is a fixed
one-character sigil peeked immediately after its own lexeme — it does not generalize
mechanically to an arbitrary-length `Word`.

`parse_type_arguments` (`src/parser.rs:4689-4713`, the `Box[i64]`/`Result[i64 i64]` reader) is
structurally reusable (bracket-delimited loop of `parse_type_expr`, arity check) but is only
ever reached from type-position parsing, where `[` has one unambiguous meaning; dropping it
into term position does not itself resolve the term-position ambiguity above.

## What already exists to build on

`Subst` (`src/ast.rs:1953-1958`, `{ ty: Vec<(u32, Type)>, len: Vec<(u32, u32)> }`) is populated
by direct `subst.ty.push((v, ty))` calls from several sites (`poly.rs:396`, `:5036`, `:5328`);
there is no dedicated setter, only the read accessors `ty_of`/`len_of`. `resolve_user_bound`
(`:5145-5179`) takes an already-resolved concrete `Type` and looks it up in the whole-program
`(TraitId, Type)` impl registry — it has no dependency on *how* that `Type` was obtained. If an
explicit-instantiation call site seeded `subst.ty` with `(v, Type_of_Point)` before the
`:4691` bound-checking loop runs, `resolve_user_bound` and the rest of obligation resolution
would work unmodified. The reusable surface is real; what does not exist anywhere in the
codebase today is any call path that seeds `Subst` from something other than operand
unification.

## Not yet recon'd / open for the spec phase

- Whether `f[Point]` should be legal generally (any poly call, an explicit override even where
  operand-driven inference already succeeds) or gated to fire only for a variable no input
  grounds — the two differ in whether a mismatch between an explicit argument and an
  operand-inferred one needs its own diagnostic.
- The actual parser mechanism for distinguishing `f[Point]` (explicit instantiation) from
  `f [Point]` / `f` followed by an unrelated quotation literal — not investigated beyond
  confirming the current grammar's ambiguity; needs either an adjacency rule (new lexer state
  tracking end-column of a `Word` vs. start-column of a following `[`) or a different surface
  spelling entirely (a distinct sigil/delimiter that doesn't collide with `[`).
- Whether this needs to reach the REPL's separate bound-directed dispatch path (`bypassed via
  lower_instantiation`, out of scope per S3p) — not probed; the REPL has a documented history
  of gaps other slices didn't inherit fixes into for free.
- Multi-variable nullary members (`fresh['T 'U]`-shaped) — whether `Type::word` vs. bracket
  syntax matters here was raised in discussion; bracket syntax (`f[Point]`, extending to
  `f[Point Point2]`) scales to this without inventing a second call form, `Type::word` doesn't.
- Whether relaxing `member_binds_trait_var` should also revive `poly_unbound_output_error`'s
  reachability check as a *deliberate* diagnostic for the case where a call to `f` supplies no
  explicit instantiation and no operand grounds `'T` either (i.e. keep the existing message,
  now finally reachable, rather than replacing it).

## Scope guess for the spec phase (not binding)

Likely three phases, in dependency order: (1) parser — a call-site explicit type-argument list
(`f[Point]`) as a new term-position production, however the adjacency ambiguity is resolved;
(2) `check_poly_call` — seed `Subst` from the explicit argument list ahead of operand
unification, sequenced before the `:4691` bound-checking loop; (3) `declarations.rs` — relax
`member_binds_trait_var` to admit a nullary member, now that a grounding mechanism exists to
receive it. Phase 3 depends on phases 1-2 landing first, or a nullary `trait:` becomes legal
with no way to ever call its member. No lowering/IR involvement expected for phases 1-2 beyond
what `CallInst`/`Subst` already carry; phase 3 is check-time only, same layer as S3p.
