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
//! This is deliberately not a compiler path: nothing in `driver`, `check`, `ir`
//! or `backend` calls it, and a file built by `sooth build` gets `if` only from
//! its own `import: core::prelude * ;`.

use crate::ast::{Module, Span, WordDef};
use crate::lexer::Token;

/// The typed core's words -- `if`/`unless` from `core::bool` and the six
/// comparisons from `core::cmp` -- parsed from the real `lib/` sources rather
/// than mirrored by hand, so a change to either file reaches these paths.
pub fn core_lib_words() -> Vec<WordDef> {
    let mut words = Vec::new();
    for src in [
        include_str!("../lib/bool.sth"),
        include_str!("../lib/cmp.sth"),
    ] {
        let tokens = crate::lexer::lex(src).expect("a lib/ core module lexes");
        words.extend(
            crate::parser::parse(&tokens)
                .expect("a lib/ core module parses")
                .words,
        );
    }
    words
}

/// `parser::parse` with the typed core appended. Appended, not prepended, so a
/// source's own words keep their positions in `words` and its own line numbers
/// stay the ones a located diagnostic reports.
pub fn parse_with_core(tokens: &[(Token, Span)]) -> Result<Module, String> {
    let mut module = crate::parser::parse(tokens)?;
    module.words.extend(core_lib_words());
    Ok(module)
}
