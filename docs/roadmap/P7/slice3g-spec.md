# Phase 7 Slice 3g: self-recursion in a non-inline generic body (spec — delivered)

## Goal

A **non-inline** polymorphic word can now call itself. Before this slice a `Term::Call`
inside a poly word's own body naming that very word fell through `poly_call_term` to
`poly_calls_poly_word_error` (P8.S2's generic-calls-generic diagnostic), which claimed the
word was "another polymorphic word across a module boundary" — it is the word being checked.
Self-recursion is the one instance of that gap needing no cross-word registry lookup: the
callee's signature *is* the `sig` the walk already holds.

**This slice delivers exactly the self-call.** Calling a *different* polymorphic word still
returns `poly_calls_poly_word_error` unchanged; that is P7.S3k's gap.
