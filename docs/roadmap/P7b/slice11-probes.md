# P7b.S11 probes — bare generic-ctor grounding (dp round, 260907)

> Verbatim probe round at base `7404a71`. Fixtures `probes/dp_*.sth`; byte-exact
> stderr baseline `probes/dp_baseline.md`; invocation
> `cargo run -q -- run probes/<name>.sth --manifest tests/fixtures/sooth.pkg`.
> Reported by the probe worker; dp_f–dp_h were its own additions beyond the
> commissioned dp_a–dp_e set.

Probing only, no `src/` changes. HEAD `7404a71` (main). All probes run with
`cargo run -q -- run probes/&lt;name&gt;.sth --manifest tests/fixtures/sooth.pkg`.

## Summary table

| probe | snippet | exit | stderr excerpt | verdict |
|---|---|---|---|---|
| dp_a | `mkok(i64->Res[i64 i64]) Ok`, then `1 mkok drop` | 0 | (none) | accepted |
| dp_b | `showres(Res[i64 i64]->)`, then `1 Ok showres` | 0 | (none) | accepted |
| dp_c | `1 Ok drop`, nothing else mentions `Res[...]` | 1 | `unknown word \`Ok\`` | rejected, wrong-word diagnostic |
| dp_d | `1 mkok Ok drop` (nested; one mint exists) | 1 | `type mismatch ... expected \`i64\`, found \`Res[i64 i64]\`` | rejected, different diagnostic class than dp_c |
| dp_e | `1 Ok[i64 i64] drop` | 1 | `` `Ok` ... takes no type arguments; only a call to a polymorphic word may be explicitly instantiated `` | rejected at a different, upstream gate |
| dp_e2 | `1 Ok[i64 i64] ;` (no consumer) | 1 | byte-identical to dp_e modulo line number | rejected, same gate — consumer irrelevant |
| dp_f | dp_c's body + an **unused, uncalled** sibling `unused(Res[i64 i64]->)` | 0 | (none) | accepted — the sibling's mere declaration is enough |
| dp_g | dp_f + a **second**, differently-shaped unused sibling (`Res[i64 cstr]`) | 0 | (none) | accepted — no ambiguity error even with 2 candidates |
| dp_g2 | dp_g, consumed by a `Res[i64 cstr]`-only word, `i64 i64` sibling declared first | 1 | `type mismatch ... expected \`Res[i64 cstr]\`, found \`Res[i64 i64]\`` | picked the **first-declared** candidate, silently wrong here |
| dp_g3 | dp_g2 with declaration order swapped | 0 | (none) | picked the (now first-declared) `Res[i64 cstr]` candidate — confirms order dependence |
| dp_h | dp_f with `unused` declared **after** `main` | 0 | (none) | still accepted — minting is whole-module, order-independent |

## Q1 — Root cause

**Not ambiguity resolved down to a rejection. There is no per-call-site type-variable solving at all.** The rejection in `dp_c` is "zero candidates exist anywhere in the whole module for the name `Ok`", a much weaker and more accidental condition than "the error parameter is unbound so the family has no single candidate."

The real mechanism, traced end to end:

1. **Grounding is not runtime unification — it's parse-time, whole-module, and incidental.** `resolve_type_or_apply` (`src/parser.rs:7294`, the `find_enum` arm ~7345-7358) calls `self.generics.instantiate_enum(...)` the moment it parses *any* signature spelling a concrete application (`Res[i64 i64]`), anywhere in the module, in *any* word's declared effect — used or not, called or not. This mints the monomorph into the shared generic-types registry before checking of any word body even starts.
2. **`env` is built before this can be reflected**, so a bare call to `Ok` misses `env` and falls to `mint_fallback_candidates` (`src/check/terms.rs:2010`), which re-derives candidates by scanning the *current, whole* extended type-slice registry for a name match — it has no notion of "which word is asking" or "what does this call site actually need"; it just reports whatever monomorphs of `Res` happen to already exist, program-wide.
3. If that scan returns **zero** candidates (nothing in the whole module ever spelled a concrete `Res[...]`, as in `dp_c`), the fall-through hits the unconditional `unknown_word_error` at `src/check/terms.rs:923` — the same generic "nothing else claims this name" diagnostic used for an actually-undefined word. There is no ambiguity-detection branch in this path at all; "zero mints" and "genuinely undefined name" are indistinguishable to it.
4. If the scan returns **exactly one** candidate — regardless of whether it's the *right* one for this call site — it's taken unconditionally (`terms.rs:951`, the `[only] =>` arm). `dp_d`'s failure is this arm firing on the *wrong* candidate: `mkok`'s signature having minted `Res[i64 i64]` makes it the sole mint in the whole program, so the *second*, nested `Ok` in `main` (which should construct a fresh `Res[Res[i64 i64] i64]`) is forced onto that same single monomorph and then fails ordinary operand type-checking (`Ok` expects `i64`, got a `Res[i64 i64]` value) — a completely different diagnostic path (`check_outputs`/underflow-style operand match) than dp_c's.
5. If the scan returns **two or more** candidates (`dp_g`), there is still no ambiguity check: `select_overload_fallback_sourced` (`src/check/builtins.rs:164-183`) runs a tier-1 own-module check, and on a miss, **falls back to `matching.first()`** — declaration-order-first among ties, explicitly by design. The function's own doc comment (`builtins.rs:150-163`) and `mint_fallback_candidates`'s doc comment (`terms.rs:1985-1989`, *"first-wins on a genuine collision, no ambiguity check; this fallback must not invent a stricter rule than a present `env` entry would have had"*) both name this as deliberate, not an oversight. `dp_g2`/`dp_g3` demonstrate it concretely: swapping which sibling word is declared first silently changes which `Res[i64, ?]` a bare `Ok` produces, with **zero diagnostic either way** — one ordering happens to satisfy the downstream consumer, the other silently constructs the wrong type and only surfaces as a type mismatch three lines later, naming `Ok`'s already-resolved (wrong) output type rather than anything about the choice made.
6. `dp_h` confirms the minting is genuinely whole-module and declaration-order-independent for the *minting* step itself (an unused sibling declared *after* the caller still grounds it) — only the *tie-break among 2+ existing mints* is first-declared-wins.

So: `dp_c`'s "ambiguity" framing from the parent conversation is too generous. It is better described as **"no local grounding mechanism exists at all; the outcome is entirely a function of which monomorphs some unrelated part of the same module happened to already mint, with silent first-wins on ties and a generic unknown-word error on zero mints."**

## Q2 — Explicit type args

**No well-formed explicit-args spelling is accepted on this path today**, for either a consumed (`dp_e`) or unconsumed (`dp_e2`) construction — both produce the byte-identical diagnostic:

```
error: `Ok` (line N) takes no type arguments; only a call to a polymorphic word may be explicitly instantiated
```

This fires at a gate strictly upstream of grounding: `TermKind::Call`'s top-of-arm check (`src/check/terms.rs:184-219`) calls `poly_call_takes_type_args` (`src/check/terms.rs:1300-1335`) before any candidate lookup happens at all. That predicate allows an explicit type-argument list only for (a) a name registered in `poly.env` (a user-declared polymorphic *word*, i.e. one with its own `'T`-parameterized signature) or (b) a trait member name. A bare generic enum's variant constructor is neither of those categories — enum variant constructors are concrete overloads (`enum_generated_sigs`/`variant_generated_sigs`), never entered into `poly.env`, which is reserved for words the user writes with type variables in their own declared effect. So `Ok[i64 i64]` is rejected on *category*, before grounding logic is ever reached; there is no spelling of explicit type arguments on a bare constructor that gets further than this gate.

## Q3 — Scope input for the B feature

**What the bare-constructor path currently honors:** nothing at the call site itself. Grounding is 100% a side effect of whatever concrete type applications happen to already be parsed elsewhere in the module (Q1). No literal-driven inference, no expected-type flow from the immediate consumer, no explicit annotation.

**What it does *not* honor, all of which is candidate B scope:**

- **Explicit type arguments** (`Ok[i64 i64]`) — blocked at the category gate (Q2), not merely unimplemented at the grounding step. Widening scope: `poly_call_takes_type_args` needs a new admitted category (a bare generic-ctor/destructure name with a matching header), not just a change inside the grounding fall-through itself.
- **Literal-driven partial inference** — `1`'s type (`i64`) is available at the call site and could pin `'T` even with `'E` left needing an explicit arg or a consumer; today this information is discarded and only matters insofar as it happens to match whichever already-minted candidate wins.
- **Consumer-driven inference for a call site with no existing mint** — `dp_c`'s `drop` accepts anything, so it can't pin `'E` regardless of mechanism, but even a concretely-typed consumer (as in `dp_b`) only works today *because* that consumer's own signature parses to a mint — not because the checker performed any lookahead from the call to the consumer. A genuine consumer-driven grounding mechanism (in the S2-9 "obligation" style, re-grounding at the resolve loop once the consumer's constraint is known) is unbuilt for this path; today's apparent "it flows in" cases are a mint-registry coincidence, not inference.

**No-consumer variant (`dp_e2`) answer:** ambiguity/groundedness is **not** the only blocker — the type-argument category gate rejects the call regardless of whether anything consumes the result, before the question of grounding is even reached. A B design that only fixes grounding-with-unbound-parameters (the `1 Ok drop` shape) but doesn't touch `poly_call_takes_type_args` will still leave `Ok[i64 i64]` illegal as a spelling.

## What this changes in the B spec

1. **Scope is larger than "fix grounding for an unbound parameter."** Three separable defects, only the first of which the parent conversation's diagnosis covered:
   - (i) zero-mint case reports the generic `unknown word` instead of a diagnostic naming the actual unbound type variable (`dp_c`);
   - (ii) single-*wrong*-mint case silently forces a construction onto an unrelated existing monomorph and reports an operand type mismatch that never mentions grounding at all (`dp_d`) — worse than misleading, this is a case where *rejection is correct* but the mechanism reaching it (forced reuse of an irrelevant mint) is accidental, not principled;
   - (iii) multi-mint case **does not reject**, silently picking a declaration-order-first candidate among structurally tied overloads with no ambiguity diagnostic at all (`dp_g`/`dp_g2`/`dp_g3`) — this is a **correctness gap, not a diagnostics gap**: two Sooth programs differing only in unrelated declaration order can construct different runtime types from the same bare `Ok` call, silently.
2. **Explicit type-argument syntax on a bare generic ctor doesn't exist yet as a category**, independent of grounding. If B's design leans on "annotate explicitly when ambiguous," that requires widening `poly_call_takes_type_args` (`src/check/terms.rs:1300`) to recognize a bare generic-ctor/destructure name paired with a matching header — new surface, not a grounding-only change.
3. **The (iii) silent-wrong-pick shape is arguably the most important finding of this round** for prioritization: it means today's behavior is not merely "some legal programs are rejected with a bad error" (dp_c) but "some illegal-looking programs are silently accepted and construct the wrong type" (dp_g2 would have compiled clean and run with the *other* branch's shape had its consumer been anything less specific than a concretely-typed word — e.g. had it flowed into another bare polymorphic sink). Any B spec should treat closing (iii) as at least as urgent as sharpening (i)'s diagnostic.
4. **No collision with S9/S10 has been found in this round.** `dp_c`/`dp_d`'s fall-through (the `env.get`-miss branch, `terms.rs:888-928`) is upstream of and structurally separate from `foreign_single_candidate_grounding` (which only runs for the struct/enum *generated-word cross-module* shape via `bare_generated_word_own_module_grounding`, reached only when candidates has already collapsed to `[only]`, `terms.rs:939-949`). The two share the same outer `Call` arm but not the same decision logic; `probes/dp_baseline.md` freezes today's exact stderr for `dp_c`/`dp_d` so an implementation pass can diff against it.
