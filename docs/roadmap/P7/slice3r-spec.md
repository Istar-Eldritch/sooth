# P7.S3r spec: `impl:` bodies instead of `impl:` bindings

Replace `impl:`'s **binding** form (a map from member names to separately-declared
concrete words) with a **body** form that declares the implementing words inside the
block, inheriting each member's signature from the trait:

```sooth
impl: Order for Point
  : cmp | a b | ... ;
;
```

Design, motivation, recon, and the resolved open questions are in
[`slice3r-brief.md`](./slice3r-brief.md); the end-to-end mangling/link spike is in
[`slice3r-spike-mangling.md`](./slice3r-spike-mangling.md); the paper dogfood and the
migration inventory are in [`slice3r-paper-dogfood.md`](./slice3r-paper-dogfood.md). This
spec does not re-derive them. It carries the rulings and lays out the phase plan.

The mechanism is a **parse-time desugar**. Each `: member ... ;` inside an `impl:` block
becomes a synthesized top-level `WordDef` spliced into `out.words`, plus exactly the
`(member, synth-name)` binding pair the current form writes by hand. `check::check_impl_decls`,
the `(TraitId, Type)` impl registry, bound-directed dispatch, and lowering are **unchanged**:
after parsing there is still no impl body, so there is still no impl-body lowering path.

## Rulings this spec fixes (do not re-open)

These are decided. Phases implement them; a reviewer checks against them, not against the
brief's open-question phrasing.

### R1 — One spelling. Delete the binding form and migrate (brief OQ1)

The body form is the only spelling. Every existing `impl:` site (both `.sth` sources and
Rust fixtures) is rewritten, and the binding form's parse path is removed, not deprecated.
Two spellings of one concept is the thing being avoided.

Consequence carried forward: the "one concrete word satisfies two traits' members"
convenience is withdrawn. Recon (brief §Pre-check 3, dogfood §B) confirmed **no program in
the tree uses it**, so deleting it strands no consumer; a future need is a one-line
forwarding body. The other withdrawn capability, binding a member to a pre-existing
independently-useful (often operator-named) word, becomes a forwarding body too; see the
migration of the two operator-named fixtures in Phase 3.

During implementation the two forms **coexist transiently** (Phase 1 adds the body branch;
Phase 4 removes the binding branch). This is scaffolding, not a supported end state, and the
discriminator is a single token: the first token after the `for`-type is `:` for a body
member, a bare word for the (soon-deleted) binding form.

### R2 — Signature inheritance, no restatement (brief decision 2)

The synthesized word's declared effect is the trait member signature with the trait's type
variable substituted by the `for` type, grounded to a concrete `StackEffect`
(`poly: None`). The grounding is exactly the shape logic `check::ground_member_type`
(`src/check/declarations.rs:495`) already performs (concrete / array / reference over `'T`;
`parse_trait_member_effect` at `src/parser.rs:2042` already rejects every other shape), so
the synthesized effect is **byte-identical** to the one `check_impl_decls` would compute.
That identity is what makes `check_impl_decls`' signature comparison vacuous (brief
motivation 1) and what makes the migration sound.

Writing an explicit signature inside an impl body is **rejected** (one spelling, not two).
Because `parse_worddef` (`src/parser.rs:1775`) mandates a `(` effect, the body-member parse
path cannot reuse it unchanged: it has its own path that parses `: member` then goes
straight to `| binders |` / body, with no `(`.

Move `ground_member_type` (`src/check/declarations.rs:495`) to **`src/ast.rs`**, the lowest
common ancestor of `parser` and `check`. This is a plain relocation, not a judgement call:
every type the function touches already lives there (`PolyType`, `Type`, `ArrayDecl`
`src/ast.rs:964`, `RefDecl` `:1020`), as do both interners it calls
(`intern_ref_type` `src/ast.rs:1044`, `intern_array_type` `src/ast.rs:1141`). It has no
remaining dependency on `check`. Calling from `parser` into `check` instead would invert the
dependency, and a parse-time twin would fork the exact grounding logic whose byte-identity
R2's vacuous-guard argument rests on. **Do not change its behaviour**; `check_impl_decls`'
own call site is untouched apart from the import.

### R3 — Diagnostic rendering of a synthesized member name (brief decision 3, spike finding 5)

The synthesized name is trait-qualified and unforgeable by construction, using a lexer
delimiter: `cmp;Order;Point` (`member;Trait;Type`). The spike verified end-to-end that this
survives `resolve::mangle` with no new exemption, that `qbe_name` escapes `;` to `.3b.`
injectively at both definition and call site, and that the binary links and runs.

The spike's one real cost: the raw synthesized name **leaks verbatim into user diagnostics**
(`resolve::demangle_word`, `src/resolve.rs:105`, strips only a trailing `__m{n}`), so a
mis-written impl body reports `cmp;Order;Point`, a name the user never wrote and cannot type.

Ruling: diagnostics that render an impl-member word name must render it back to a form the
user can read. Chosen rendering: **`` `cmp` (member of trait `Order` for `Point`) ``**. This
is a required behaviour, specified as a golden in Phase 2; it is *the* replacement diagnostic
for the retired signature-mismatch class (see R6), so it must land before Phase 4.

### R4a — How a member name resolves inside its own body (brief decision 4)

The recursion story needs a stated mechanism, because the synthesized word is a plain
top-level `WordDef` named `cmp;Order;Point` and a bare `cmp` token in its body would
otherwise not find it.

Ruling: while desugaring a member body, the parser **rewrites every call token equal to that
member's own name into the synthesized name**, throughout the whole body including nested
quotations. A local binder sharing the member's name (`| cmp |` inside `cmp`'s own body) is a
located error rather than a silent winner or a silent casualty of the rewrite: the rewrite is
unconditional token equality, so the two cannot coexist, and this project refuses silent
shadowing. It is a single-name, body-scoped lexical rewrite, decided entirely at parse time;
no resolution rule elsewhere changes, and nothing in `check` learns about it. This is the
concrete content of brief decision 4's "self-binding", and of decision 5's scoped shadowing:
the rewritten name wins over any module-scope word of the same name, and only inside that
body.

Two consequences worth stating. Sibling members are **not** rewritten (R7), so a sibling call
resolves by ordinary lookup: it never reaches the sibling's synthesized word, and it is not
necessarily an error either, since it can bind a same-named library word instead (see R7). And
this rewrite is why R4 rejects at the trait declaration rather than treating the name as
cosmetic: for a name the checker dispatches *before* the environment, the rewrite would either
be bypassed (recursion silently
lost) or would displace a builtin.

Golden (Phase 1): `impl_body_member_calls_itself_recursively` — a member body that calls its
own member name compiles, links, and runs to the expected value. Mutation check: with the
rewrite disabled the program must fail to compile with an unknown-word error, so the golden
cannot pass a no-op implementation.

### R4 — Builtin-spelled member names are rejected at the trait declaration (brief OQ6)

A member name inside its own impl body resolves to that member before any module-scope
lookup (brief decision 4, the recursion story). Brief decision 5 admits that shadowing **for
this construct only**: it is visible at the point of use, the `impl: Trait for Type` header
is the immediately enclosing form, and it is statically decided.

For a builtin-spelled member (`add`, `max`, `.`, `dup`, `swap`, …) that self-binding would
shadow a **builtin** inside the body, not merely a user word. That is wider than decision 5
signed up for, and general/builtin shadowing is explicitly refused project-wide.

Ruling: **a trait may not declare a member spelled as a name that resolves ahead of the word
environment, nor as a name reserved against any word declaration.** The rejection lives at
the `trait:` declaration site (`parse_trait_decl`, `src/parser.rs:1977`), not at the impl
body. A member name is a located parse error when it is any of:

- a genuinely name-dispatched builtin: `is_builtin_word_name`
  (`src/check/declarations.rs:118`) **minus the six surface comparisons**. That is exactly
  the set `is_gated_intrinsic_name` (`src/check/declarations.rs:134`) already computes;
  reuse it, or extract the shared predicate under an accurate name. Note this set is a
  *function*, not the `BUILTIN_WORDS` const: it also claims every `>`-prefixed conversion
  (`>u8`, `>u32`), which the const does not list;
- an access word `@` / `!` / `+!` (`ACCESS_WORDS`, `src/parser.rs:150`);
- anything `reject_reserved_name` (`src/parser.rs:211`) already refuses for a word
  declaration: the `^`-led owning-cell names and the reserved ref names.

The last two categories are exactly what `parse_worddef` already enforces for every ordinary
word (`src/parser.rs:1775-1780`); a member becomes a word, so it inherits that policy rather
than inventing one. `parse_trait_decl` does **not** call `reject_reserved_name` on member
names today, so all of these are currently accepted (probed).

**Why the declaration site, not the impl body.** Under R1 the body form is the only way to
implement a member, so a member in a rejected category is unimplementable; the error belongs
where the unimplementable thing is written, not at the later site that discovers it. It also
subsumes the cross-module case for free (a trait in module A cannot be declared at all, so no
impl in module B can reach it), and `parse_trait_decl` (`src/parser.rs:2025`) is the single
funnel: the only other `TraitDecl` constructions are the seeded `Copy`/`Ord` predicate traits
(`src/ast.rs:1386,1393`), which carry `members: Vec::new()`.

The two sites are **mutually exclusive, not defence in depth.** With the declaration site
guarded, an impl-body check for the same condition is unreachable, and an unreachable check is
a placebo this project treats as a defect. Implement exactly one, here.

Two in-tree casualties, which the implementer will otherwise hit as red tests, and they take
opposite fixes:

- `tests/phase7_slice3e.rs:183` declares `trait: Show 'T tag ( -- i64 ) ;` purely to provoke
  the P7.S3p zero-input-receiver error, and `tag` is a rejected name (`BUILTIN_WORDS`, the
  slice-10c discriminant primitive). **Rename** that fixture's member to a non-builtin name so
  it still asserts the receiver diagnostic it is about; do not reorder the checks to keep it
  passing.
- `src/check/poly.rs`'s `bound_dispatch_and_a_builtin_named_member_coexist` declares
  `trait: Sum 'T add ( &'T &'T -- i64 ) ;`, and its *subject* is the operator-spelled member
  itself, so there is no rename that keeps its intent. **Delete** it: its own successor
  (`bound_dispatch_reaches_a_member_named_after_an_intercepting_builtin`, member `eq`) records
  that `add` never exercised R10's claimed partition in the first place (no dispatch-cascade arm
  matches `add`), carries the same coexistence half in its `main`, and stays legal under the
  corrected predicate. Note on that successor that the six surface comparisons are now the whole
  of the barrier it guards.

Why the predicate is not the `BUILTIN_WORDS` const: the const **does** contain `eq`, `lt`,
`gt`, `lte`, `gte`, `ne` (`src/check/declarations.rs:94-99`), but the comment there records
that those six are `lib/` words now, *not* name-dispatched: they are listed only so
`has_self_tail_call` does not read a trailing `lt` as a self-call. Rejecting on the raw const
would therefore make a member named `eq` illegal and take the **`Eq` trait and the planned
`Map` consumer (`Eq` + `Hash`) with it**, for a shadowing hazard that does not exist: an `eq`
member's self-binding shadows a *library word*, which is precisely the "shadows a user word"
case decision 5 already admits. `is_gated_intrinsic_name` excludes those six for the same
underlying reason (they live in `core::cmp`, not the intrinsics surface).

Why the predicate is not `resolve::is_operator_dispatch_name` (`src/resolve.rs:72`) either:
that lists only the 20 operator-dispatch names and misses `dup`/`drop`/`swap`/`over`/`rot`,
which `check_term` dispatches by name before the environment. Probed at `5338c06`:
`: dup ( i64 -- i64 ) 99 add ;` declares without complaint and the call site silently
resolves to the *builtin*, leaving the user word unreachable with no diagnostic. A member
named `dup` would therefore either silently lose its own recursion (R4a's self-binding never
reached, no error) or shadow a core linear-spine builtin.

Consequence, stated plainly: a trait *may* declare a member in a rejected category, but no
body-form impl can implement it, so such a member is effectively unimplementable. The
binding form could have implemented it; the binding form is being deleted. Live members are
unaffected: `cmp`, `show`, `hash` are absent from every rejected category, and `eq`/`lt`/`gt`
stay legal under the corrected predicate. This keeps decision 5's shadowing exactly at "shadows a user word", never a
builtin — the exception does not widen. Enforced at the body-desugar site (parse time), so
trait-declaration checking is untouched (constraint: nothing in check changes).

This does **not** touch the still-legal migration case where the *member* is ordinarily
named (`get`) and its *body* calls an operator builtin (`max`): there `max` resolves to the
builtin normally (Phase 3 migrates exactly this).

### R5 — The trailing `; ;` is accepted (brief OQ4)

The block parses as "loop over `: member ... ;` until a bare `Semicolon`, then expect the
closing `Semicolon`". The doubled terminator is accepted as-is: Sooth has no `end` to borrow
and no keyword is spent on a cosmetic wart. A missing final `;` is the existing
unterminated-declaration error.

### R6 — A non-member body is a located error (brief OQ5)

Under the binding form, `impl: Show for i64  bogus int-show` is rejected because `bogus` is
not a member (`check_impl_decls_unknown_member_is_error`, "is not a member of trait
`Show`"). Under the body form the analogue is `: bogus ... ;` inside the block. There is no
member to bind and decision 1 has no pair to synthesize.

Ruling: a located parse error preserving that diagnostic's intent (the alternative, a free
module-private word, silently swallows a typo). The intent of the retiring check-time
`unknown_member` guard is **preserved** and relocated to this parse-time rejection (Phase 1);
the check-time guard itself is deleted in Phase 4.

### R7 — No sibling-member access (brief OQ3)

A member body sees its own name (R4a's rewrite) but not its sibling members. Recon (dogfood
§C) found no consumer that needs it. Locked by absence of a consumer; not a proof no future
consumer wants it. No mechanism is added for it.

The sharp edge this leaves, now that the six comparison names are legal members (R4): a
sibling call is not an error, it resolves by ordinary lookup, so a `hash` body calling `eq`
binds `core::cmp`'s `eq` rather than the sibling member, silently. Phase 1 golden
`impl_body_sibling_call_does_not_reach_the_sibling` pins that resolution so the behaviour is
witnessed rather than discovered later; if a consumer ever needs sibling access, that golden
is the thing it must consciously overturn.

### R8 — REPL located rejection for `impl:` / `trait:`

The REPL wires trait/impl only through `assemble_module`, so `impl:`/`trait:` today produce
no located error (they fall into the term loop and report `unexpected token Semicolon`).
Mirror the `export:` / `global:` guards at `src/repl.rs:1597` / `src/repl.rs:1699`: a first
token of `impl:` or `trait:` returns a located rejection. (This is the standing hazard that
anything wired only into `assemble_module` is unenforced at the REPL.)

## The synthesized name

- Shape: `member;Trait;Type`, e.g. `cmp;Order;Point`. Trait-qualified to preserve recon O3
  (two traits may share a member name and its grounded signature for one type). Unforgeable
  because `;` is a hard lexer delimiter: a user source token can never contain it (spike
  finding 4), so no new name-rejection rule is required.
- The `Type` component is the `for`-type's rendered name (the same spelling a type expression
  produces). This is a synthesized internal name; it is never parsed back, so its exact
  spelling only needs to be injective per `(member, trait, type)`.
- It is spliced into `out.words` as an ordinary `WordDef` and flows through mangle, check,
  lowering, and emission unchanged (spike findings 1–3).

## What is deleted, and where its intent goes (Phase 4)

Five `check_impl_decls` guards go **vacuous** under signature inheritance: each keys on a
hand-declared implementing word that the desugar now guarantees. This project treats a test
that can no longer fail as a placebo defect, so each is **deleted with the binding form**,
not migrated. For each: what replaces its intent.

| Guard / test (`src/check/declarations.rs`) | Keys on | Fate of the intent |
| --- | --- | --- |
| `check_impl_decls_signature_mismatch_is_error` ("does not match") | a restated signature disagreeing with the trait | **Relocated.** Inheritance removes the restatement; a wrong body now fails ordinary in-body stack-effect checking, located inside the body. Replaced by a Phase 4 golden (a body whose effect is wrong → stack-effect error naming the readable member via R3). |
| `check_impl_decls_does_not_bind_a_word_from_another_module` ("unknown word") | a binding target in another module | **Gone with the feature.** There is no separate word to bind; the member *is* the body. No cross-module binding exists to reject. |
| `check_impl_decls_polymorphic_member_is_error` ("polymorphic") | a poly implementing word | **Gone with the feature.** The synth word's effect is the inherited concrete grounded sig (R2) and an explicit body signature is rejected, so a member body is concrete by construction. |
| `check_impl_decls_polymorphic_member_with_a_zero_slot_member_is_error` ("polymorphic", "`p`") | same, via a zero-slot member | **Gone with the feature.** Same guarantee as the row above. |
| `check_impl_decls_drop_overload_member_is_error` (binds `eat` to `drop`) | a member bound to a destructor overload | **Gone with the feature.** There is no binding-to-`drop`; a body may legally *call* `drop`, and a member is never named `drop`. |

Also deleted outright with the bare-binding parse branch:

- The odd-binding-token parse error and its coverage in `src/parser.rs` (the message about a
  member having no implementing word before the `;`, tagged "odd binding-token count"): the
  body form has no member/word pairs, so the error is unreachable.
- `check_impl_decls_unknown_member_is_error` (check-time): its **intent is preserved** and
  re-pinned by the Phase 1 body-form non-member rejection (R6), so the check-time test is
  removed as a placebo, not lost.

Guards that **stay** (their subject is not the binding form): duplicate `(Trait, Type)`
(`check_impl_decls_duplicate_impl_is_error`), missing required member
(`check_impl_decls_missing_member_is_error`), the orphan rule
(`check_impl_decls_orphan_scalar_target_names_only_the_trait_module`), the static/trait
collision, export/selective-import, and the P7.S3p receiver rules. These live at the
`trait:` decl or the impl registry and only need their fixtures' **syntax** migrated
(Phase 3).

## Fixture classification (drives Phase 3 vs Phase 4)

Recon (dogfood §B): 2 real `.sth` declarations (`examples/traits.sth:55,56`), 0 in `lib/`,
~55 Rust fixture declarations across 6 files (`tests/phase7_slice3e.rs` 17,
`src/check/poly.rs` 12, `src/check/declarations.rs` 11, `src/driver.rs` 7,
`src/parser.rs` 5, `src/ir/driver.rs` 3). Classify by **test subject**, not by grep count.

- **Phase 3 (migrate: rewrite syntax, keep intent).** Every positive dispatch/lowering
  fixture; every structural rejection that lives at the trait decl or registry (list above);
  the two real `.sth` lines. The two operator-named fixtures migrate **differently from each
  other**, and the difference is load-bearing:
  - `src/ir/driver.rs:1622` (member `get ( &'T &'T -- i64 )`, word `: max ( &Pt &Pt -- i64 )`)
    migrates to a forwarding body `: get | a b | a b max ;`. Probed at `5338c06`: this
    builds, runs, and keeps `max__m0` in the symbol table, called from the synthesized
    member. Its assertion changes from "the member's own call symbol is `max`" to "`max`
    survives pruning as a symbol called from the synthesized `get;Getter;Pt`"; the pruning
    concern is still probed and its subject is unchanged.
  - `tests/phase7_slice3e.rs:559` (member `show ( &'T -- i64 )`, word `: max ( &Pt -- i64 )`)
    **cannot use a forwarding body at all.** A one-input word named `max` is unreachable by
    name anywhere: the call resolves to the two-input builtin, probed at `5338c06` as
    ``error: `max` needs 2 values, but the stack holds 1``. Migrate it by *inlining* the
    body (`: show | p | p &n @ ;`) and deleting the now-unreferenced `: max` word. The
    scenario it pinned (an operator-spelled name reachable only through a bound) is not
    re-expressible under the body form and is not preserved here; R4 makes it a rejection
    instead, and the Phase 1 operator-member golden is what now covers that ground.
- **Phase 4 (delete: the test's subject is the retiring binding form).** The five vacuous
  guards' tests, the check-time `unknown_member` test, and the odd-token parse test — a test
  that names the binding form's own validation surface belongs to the deletion, not the
  migration.

## Phase plan

Every phase is independently green (`cargo fmt --check && cargo clippy -- -D warnings &&
cargo test`) and adds no plumbing ahead of its first use (an import scheduled before its call
site is clippy-fatal here). Exit criteria are golden tests.

### Phase 1 — Body-form parse + desugar, coexisting with the binding form

Add the body-member branch to `parse_impl_decl` (`src/parser.rs:2096`+): when the first token
after the `for`-type is `:`, parse `: member [| binders |] body ;` members until the closing
`;` (R5). For each member: resolve it against the trait's `members` (R6), ground its
signature to a concrete `StackEffect` (R2), synthesize a `WordDef` named `member;Trait;Type`
spliced into `out.words`, and push `(member, synth-name)` into `ImplDecl::bindings` so
`check_impl_decls` resolves it exactly as today. The bare-binding branch is untouched.

New parse-time rejections (exact text; `L`/`C` are the offending token's line/col).
Explicit signature restated in a body (R2):

```text
error: impl member `cmp` must not restate its signature at line L, col C (it is inherited from trait `Order`'s `cmp` with the `for` type)
```

A body naming a non-member (R6):

```text
error: `bogus` is not a member of trait `Show` at line L, col C
```

A builtin-spelled member (R4), emitted from `parse_trait_decl`, not from the impl body.
**Three messages, not one**, because R4's three categories are not the same mistake and two of
them already have messages that this site inherits rather than replaces. Only the
name-dispatched-builtin category gets new text:

```text
error: trait `Getter` declares a member named `max`, which is a builtin word (line L, col C)
  note: a trait member becomes a word when implemented, and inside its own body the name would shadow the builtin
```

An access-word member (`@`, `!`, `+!`) reuses the existing `shadowed_access_word_error`
(`src/parser.rs:154`), and a caret or reserved-ref member reuses `reject_reserved_name`'s
existing errors (`src/parser.rs:211`, rendered by `reserved_caret_name_error` at
`src/parser.rs:116`) — the same messages `parse_worddef` already produces for an ordinary word
declaration. Emitting the "spelled as a builtin word" text for a caret name would be wrong on
its face: a caret name is reserved syntax, not a builtin.

Unterminated block (existing shape, reachable via the new branch): the loop between
members has no lookahead past the next `:`, so what error surfaces depends on what
follows the missing closing `;`. If the block runs to end of file, it is the current
`;` (unterminated `impl:` declaration) EOF error, unchanged. If another `: name ...`
declaration follows in the file, that declaration's tokens are consumed as an attempted
next member instead, surfacing as a non-member or duplicate-member error naming the
following declaration rather than as an EOF or a syntax error. Nothing is silently
swallowed either way (every absorbable shape still hits a located error), but the two
paths are different diagnostics and both are pinned by goldens rather than only the
first.

Exit goldens:

- `impl_body_form_builds_and_runs`: a complete program (below) builds and runs to exit 0,
  emitting `(3,4)`. Assert the synthesized symbols `cmp.3b.Order.3b.Point__m0` and
  `show.3b.Show.3b.Point__m0` are present via `nm` (proves the desugar spliced real words and
  they linked — matches spike finding 2).
- `impl_body_restated_signature_is_rejected`: the explicit-signature error above.
- `impl_body_non_member_is_rejected`: the non-member error above.
- `trait_member_named_after_a_builtin_is_rejected`: covers **each** rejected category from R4,
  since any one of them alone would miss the others: an operator (`max`), a shuffle builtin
  (`dup`), a `>`-prefixed conversion (`>u8`, which the `BUILTIN_WORDS` const does not list at
  all), an access word (`@`), and a reserved caret name. Each category asserts **its own**
  message per the three-message split above, not one shared string; asserting the builtin text
  for `@` or a caret name would pass only against a wrong implementation. The fixture is a
  bare `trait:` declaration with no `impl:` at all — that is the point of the site change, and
  it is what distinguishes this from an impl-body check. All five are accepted as member names
  today (probed against the built compiler), so each fixture is constructible.
- Rename the member in `tests/phase7_slice3e.rs:183` (`trait: Show 'T tag ( -- i64 ) ;`) so it
  keeps asserting the P7.S3p receiver diagnostic instead of newly hitting R4.
- `trait_member_named_after_a_comparison_is_accepted`: the negative-space companion, and the
  regression guard for the P0 this ruling nearly shipped. A trait with an `eq` member and a
  body-form `: eq ... ;` must **compile**, since the six surface comparisons are library
  words rather than name-dispatched builtins. Without this, a later tightening of the
  predicate to the raw `BUILTIN_WORDS` const would silently kill the `Eq` trait again.
- `impl_body_member_calls_itself_recursively` (R4a): the recursion golden described above.
- `impl_body_trait_qualifier_disambiguates_shared_member_name`: recon O3's functional
  witness, not just the literal-name assertions the mutation tests already cover. Two
  traits each declare a member named `get`, both implemented for one type, each reached
  through a different bound; the program prints both results correctly, proving the
  trait component of `member;Trait;Type` actually disambiguates rather than merely
  appearing in the synthesized name.
- `impl_body_unterminated_block_at_eof_is_error` / `impl_body_unterminated_block_absorbs_next_decl`:
  the two diagnostics an unterminated block can produce, per the note above.
- Coexistence: the existing binding-form goldens still pass unchanged.

The Phase 1 positive golden source (complete, compilable under `examples/sooth.pkg`
conventions — this is the dogfood's Part A program):

```sooth
import: intrinsics * ;
import: core::prelude | if Bool lt gt | ;

type: Ordering | Less | Equal | Greater ;

trait: Order 'T
  cmp ( &'T &'T -- Ordering )
;

trait: Show 'T
  show ( &'T -- )
;

type: Point x i64 y i64 ;

impl: Order for Point
  : cmp
    | a b |
    a &y @ | ay |
    b &y @ | by |
    ay by lt
    ~[ Less ]
    ~[
      ay by gt
      ~[ Greater ]
      ~[
        a &x @ | ax |
        b &x @ | bx |
        ax bx lt
        ~[ Less ]
        ~[ ax bx gt ~[ Greater ] ~[ Equal ] if ] if
      ] if
    ] if ;
;

impl: Show for Point
  : show
    | p | "(" . p &x @ . "," . p &y @ . ")" . ;
;

: show_larger ( &'T: Order Show &'T -- )
  | b | | a |
  a b cmp
  ~[ ( Less ) drop b show ]
  ~[ ( Equal ) drop a show ]
  ~[ ( Greater ) drop a show ]
  Ordering? ;

: main ( -- )
  0 0 Point | origin |
  3 4 Point | corner |
  &origin &corner show_larger
  origin drop
  corner drop ;
```

Difficulty: **hard** (the desugar, grounding elevation, and the new parse path).

Growth-structure re-check (CLAUDE.md, at phase exit): this phase adds the body-member
branch, its rejections, and their helpers to `parser.rs`, growing it past 8000 lines. Kept
as-is: the new code sits beside the other declaration parsers it extends
(`parse_trait_decl`, `parse_impl_decl`), pulls no dependency the file doesn't already have,
and none of the split signals (import divergence, X-and-Y-and-Z responsibilities, dead
cross-calls, a forced circular dependency) fire together. Re-check again at Phase 4, once
the binding-form branch it coexists with is deleted.

### Phase 2 — Readable rendering of a synthesized member name (R3)

Give diagnostics a way to render `member;Trait;Type` back to
`` `cmp` (member of trait `Order` for `Point`) ``. Extend the `demangle_word` /
`demangle_call` path (`src/resolve.rs:105`+) so a name whose (post-`__m` strip) body
contains the `;` delimiter is split on it and rendered in that form; a name without `;`
is unchanged. Lookups keep using the mangled name; only the rendered string changes (the
existing `demangle_word` contract).

Exit golden `impl_body_wrong_effect_names_readable_member`: a body-form impl whose body
leaves the wrong effect (e.g. a `cmp` body that drops without pushing an `Ordering`) is
rejected by ordinary in-body stack-effect checking, and the message names
`` `cmp` (member of trait `Order` for `Point`) `` — never the raw `cmp;Order;Point`. This is
the concrete replacement for the retired signature-mismatch class (R6/Phase 4). Include a
`nm`-free negative check in the golden that the raw delimiter spelling does **not** appear in
the diagnostic.

### Phase 3 — Migrate `.sth` sources and mechanical fixtures to the body form

Rewrite to the body form: `examples/traits.sth:55,56` (the two `impl:` lines fold into their
blocks; `point-cmp`/`point-show` and their restated signatures vanish, per dogfood Part A);
and every Phase-3-classified fixture across the six files, keeping each test's intent. Adjust
the two operator-named fixtures' assertions as classified above. Do **not** touch any
Phase-4-classified fixture yet (the binding form still parses this phase). Note the two
operator-named fixtures take *different* migrations (forwarding body vs. inlining); a
forwarding body for the one-input case does not compile.

Exit: `examples/traits.sth` builds and runs; the migrated fixtures pass with intent
preserved. No new diagnostic is introduced, so the exit criterion is the existing goldens'
continued behaviour under the new syntax (the `sort3` golden, the dispatch/lowering fixtures,
the operator-pruning fixtures with adjusted assertions).

### Phase 4 — Delete the binding form and its now-vacuous guards

Remove the bare-binding branch from `parse_impl_decl`, the odd-binding-token parse error, and
the five vacuous `check_impl_decls` guards plus the check-time `unknown_member` guard (their
intent is already relocated per R6 and the deletion table). Delete the Phase-4-classified
fixtures. `ImplDecl::bindings` is now populated only by the desugar; its comment and
`ast.rs`'s "a pure name map, never a body" doc lose the binding-form framing (state the
current design only; no history, per project convention).

Add the replacement golden `impl_body_wrong_effect_is_rejected_in_body` (the relocated
signature-mismatch intent, R6): distinct from Phase 2's rendering golden in that it asserts
the *rejection happens and is located inside the impl body*, whereas Phase 2 asserts the
*name renders readably*.

Exit: the whole suite green with the binding form gone; `grep` finds no `member word`
binding syntax in any `.sth` or fixture; the deleted guards' names no longer exist.

### Phase 5 — REPL located rejection for `impl:` / `trait:` (R8)

In `eval_line` (`src/repl.rs`, the first-token guard block beside the `export:` rejection),
add located rejections for a first token of `impl:` and `trait:`, mirroring the `export:` /
`global:` shape.

Exit goldens (REPL unit tests, mirroring the existing `export:`/`global:` REPL rejection
tests):

- `repl_rejects_impl`: an `impl: Order for Point ...` line is rejected with:

```text
error: `impl:` has no meaning at the REPL (line L, col C)
  note: a live session has no module to attach a trait implementation to
```

- `repl_rejects_trait`: a `trait: Order 'T ...` line is rejected with:

```text
error: `trait:` has no meaning at the REPL (line L, col C)
  note: a live session declares no trait to satisfy
```

## Out of scope

- General / builtin word shadowing beyond R4's construct-scoped rejection.
- Any change to bound-directed dispatch, the impl registry, monomorphization, or lowering.
- P7.S3o and P7.S3n.

## Phases (machine-readable)

```json
{
  "phases": [
    { "phase": 1, "focus": "body-form impl parse and desugar coexisting with binding form", "difficulty": "hard" },
    { "phase": 2, "focus": "readable rendering of synthesized impl member names in diagnostics", "difficulty": "standard" },
    { "phase": 3, "focus": "migrate sth sources and mechanical fixtures to the body form", "difficulty": "standard" },
    { "phase": 4, "focus": "delete the binding form and its vacuous check guards", "difficulty": "standard" },
    { "phase": 5, "focus": "repl located rejection for impl and trait declarations", "difficulty": "standard" }
  ]
}
```
