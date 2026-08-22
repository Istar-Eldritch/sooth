//! The library seeding the in-process compile paths do for themselves.
//!
//! `parser::parse` used to append the compiler-baked prelude's words to every
//! file it parsed, so any path that lexed a source and checked it got `if` and
//! the comparisons for free. P8 S2 deleted that: those are ordinary `core`
//! words now, reached by an `import:` that only the driver's closure discovery
//! resolves. The in-process paths -- every `check_src`-style unit test, the
//! `ir`/`backend` lowering helpers, the parser's own goldens -- resolve no
//! `import:` at all, so they seed what they need here instead, which is what
//! keeps a test source's meaning independent of package resolution.
//!
//! P7 slice 3i adds the *type* `bool` to what has to be seeded: it is
//! `core::bool`'s declaration now, not a registry slot the parser reserves. A
//! registry entry cannot be appended after the fact the way a `WordDef` can (an
//! `EnumId` in an already-parsed signature indexes a position, so appending
//! `bool` behind a test's own enums would leave `core`'s words naming the wrong
//! ones), so the core sources are lexed and parsed *with* the test's tokens as
//! one file.
//!
//! This is deliberately not a compiler path: nothing in `driver`, `check`, `ir`
//! or `backend` calls it, and a file built by `sooth build` gets `bool` and `if`
//! only from its own `import:` lines.

use crate::ast::{EnumDecl, Module, Span, WordDef};
use crate::lexer::Token;

/// The typed core's sources: `core::bool` (the `bool` type, `if`/`unless` and
/// the bool `.` overload) and `core::cmp` (the six comparisons), embedded rather
/// than mirrored by hand so a change to either file reaches these paths.
const CORE_SOURCES: [&str; 2] = [
    include_str!("../lib/bool.sth"),
    include_str!("../lib/cmp.sth"),
];

/// The typed core's words, parsed from the real `lib/` sources.
///
/// One parse across both sources, because `core::cmp`'s effects name `bool`,
/// which `core::bool` declares. A caller that also has source of its own wants
/// `parse_with_core`: the merged parse is what keeps the registry ids in these
/// words' signatures pointing at the right entries.
pub fn core_lib_words() -> Vec<WordDef> {
    parse_with_core(&[])
        .expect("the lib/ core modules parse")
        .words
}

/// `core::bool`'s registry as its own parse produces it -- the `bool` enum and
/// nothing else. The stand-in a bare-line (REPL-shaped) helper uses for the
/// startup seed `Session::new` performs, for the same reason: `infer_line`
/// takes a registry, not a source.
pub fn core_bool_enums() -> Vec<EnumDecl> {
    let tokens = crate::lexer::lex(CORE_SOURCES[0]).expect("`lib/bool.sth` lexes");
    crate::parser::parse(&tokens)
        .expect("`lib/bool.sth` parses")
        .enums
}

/// `parser::parse` over the caller's tokens with the typed core's sources
/// appended as though they were part of the same file, so the core's `bool`
/// enum and words land in the caller's own registries.
///
/// Appended, not prepended, and that is load-bearing twice over: a source's own
/// words keep their positions in `words` and its own line numbers stay the ones
/// a located diagnostic reports, and its own `type:` declarations keep the
/// registry positions they would have had alone (so `bool` never displaces a
/// test's first enum).
pub fn parse_with_core(tokens: &[(Token, Span)]) -> Result<Module, String> {
    let mut all = tokens.to_vec();
    for src in CORE_SOURCES {
        all.extend(crate::lexer::lex(src).expect("a lib/ core module lexes"));
    }
    crate::parser::parse(&all)
}
