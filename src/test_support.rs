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
///
/// P7.S3s: `core::cmp`'s six comparisons now dispatch a real trait member
/// (`cmp`) through `impl:` bindings, exactly as any other `trait:`/`impl:`
/// pair does -- resolving that binding is `check_impl_decls`' job
/// (`driver::assemble_module` runs it between parse and `check::check`), not
/// something `check::check` re-derives on its own. Run here too, mirroring
/// that order, so every caller of this helper gets a module whose bound
/// members are already resolved, the same as a real build's -- without this,
/// a caller that only runs `check::check` (most of them) would see `eq`/`lt`/
/// etc. fail with "binds no word for member `cmp`", a resolution gap in the
/// test harness, not in the checker.
pub fn parse_with_core(tokens: &[(Token, Span)]) -> Result<Module, String> {
    let mut all = tokens.to_vec();
    for src in CORE_SOURCES {
        all.extend(crate::lexer::lex(src).expect("a lib/ core module lexes"));
    }
    let module = crate::parser::parse(&all)?;
    crate::check::check_trait_decls(&module)?;
    let mut module = module;
    crate::check::check_impl_decls(&mut module)?;
    Ok(module)
}

/// A synthetic single-word `WordDef` for `check/`'s unit tests that drive a
/// checker helper directly rather than through a source program: those
/// helpers all take a `Ctx`, and every diagnostic they emit cites the
/// enclosing word, which is this one.
pub fn bare_word(name: &str, module: u32) -> WordDef {
    WordDef {
        name: name.to_string(),
        effect: crate::ast::StackEffect::default(),
        body: Vec::new(),
        poly: None,
        declares_inline: false,
        module,
        span: Span::default(),
        declared_globals: None,
    }
}
