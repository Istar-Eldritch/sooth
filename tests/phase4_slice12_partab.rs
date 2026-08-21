//! Phase 4 Slice 12, phase 1 exit criteria (parts A + B).
//!
//! `is_combinator` is declared, not inferred (R-A1, unit-tested in
//! `src/check/combinators.rs`); a `~[ ... ]` parameter without `inline` is a
//! located error (R-B1, unit-tested in `src/check/word_entry.rs`). This file
//! covers X2 (the nine library words are declared combinators, and still run)
//!. X4 (the retyped array words) is deliberately absent:
//! its subject moved to `examples/experiments/arrays.sth`, which is an
//! experiment rather than library code, and the transition it guarded (an
//! ordinary `[ ... ]` literal satisfying a `~[ ... ]` parameter) was superseded
//! when part C required the tilde.

/// The nine library words that gained `inline` this slice, paired with the file
/// that defines each. P8.S2 (R8) split the old `lib/core.sth` into `core::bool` and
/// `core::cmp`, so `if`/`unless` live in `bool.sth` now.
mod common;
const MIGRATED: [(&str, &str); 9] = [
    ("combinators.sth", "times-helper"),
    ("combinators.sth", "times"),
    ("combinators.sth", "each"),
    ("combinators.sth", "map"),
    ("combinators.sth", "fold"),
    ("combinators.sth", "filter"),
    ("combinators.sth", "while"),
    ("bool.sth", "if"),
    ("bool.sth", "unless"),
];

/// Build and run `src`, returning the built binary's path (left in place for
/// the caller to remove), stdout, and exit code.
fn build_and_run(name: &str, src: &str) -> (std::path::PathBuf, String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
    (
        binary,
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

/// X2, the discriminating half: each of the nine is still a combinator once
/// part A retires the inference leg, asserted on `is_combinator` itself over
/// the words as `lib/` actually spells them. A missed `inline` in the library
/// reddens this directly.
///
/// Asserted at the predicate rather than through `nm`, because a symbol-table
/// witness cannot discriminate *these nine*: all nine are polymorphic, and
/// `ir::driver`'s `poly_indices` already excludes a polymorphic word from the
/// symbol-minting env whether or not it is a combinator. The end-to-end
/// no-symbol witness lives on the one shape where minting does track
/// combinator-ness, a monomorphic `inline` word
/// (`phase4_slice11_inline.rs::inline_word_mints_no_symbol`).
#[test]
fn migrated_library_words_are_declared_combinators() {
    for (file, name) in MIGRATED {
        let path = format!("{}/lib/{file}", env!("CARGO_MANIFEST_DIR"));
        let src = std::fs::read_to_string(&path).expect("a library file should be readable");
        let tokens = sooth::lexer::lex(&src).expect("a library file should lex");
        let module = sooth::parser::parse(&tokens).expect("a library file should parse");
        let word = module
            .words
            .iter()
            .find(|w| w.name == name)
            .unwrap_or_else(|| panic!("`{name}` should be defined in lib/{file}"));
        assert!(
            sooth::check::is_combinator(word),
            "`{name}` must declare `inline` to stay a combinator once recognition is declared, \
             not inferred"
        );
    }
}

/// X2, the end-to-end half: every one of the nine is called, and the program
/// still builds and runs once recognition is declared rather than inferred.
#[test]
fn migrated_library_words_still_run() {
    let src = format!(
        "{}: main ( -- )\n\
         3 ~[ 1 add drop ] c::times\n\
         0 4 fill ~[ 1 add drop ] c::each\n\
         0 4 fill ~[ 1 add ] c::map drop\n\
         0 4 fill 0 ~[ add ] c::fold drop\n\
         0 4 fill ~[ 2 gt ] c::filter drop drop\n\
         0 ~[ dup 3 lt ~[ 1 add true ] ~[ false ] if ] c::while drop\n\
         true ~[ 1 ] ~[ 2 ] if drop\n\
         false ~[ 1 ] ~[ 2 ] unless drop ;\n",
        combinators_import("c"),
    );
    let (binary, _stdout, code) = build_and_run("slice12-partab-run", &src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
}

fn combinators_import(qualifier: &str) -> String {
    format!(
        "import: \"{}/lib/combinators.sth\" {qualifier} ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}
