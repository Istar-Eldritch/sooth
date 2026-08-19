//! Phase 4 Slice 12, phase 1 exit criteria (parts A + B).
//!
//! `is_combinator` is declared, not inferred (R-A1, unit-tested in
//! `src/check/combinators.rs`); a `~[ ... ]` parameter without `inline` is a
//! located error (R-B1, unit-tested in `src/check/word_entry.rs`). This file
//! covers X2 (the nine library words are declared combinators, and still run)
//! and X4 (`arrays.sth`'s retyped `bin_search`/`sort` still run, an ordinary
//! `[ ... ]` literal still satisfying their new `~[ ... ]` parameter until part
//! C requires the tilde).

/// The nine `lib/combinators.sth`/`lib/core.sth` words that gained `inline`
/// this slice, paired with the file that defines each.
const MIGRATED: [(&str, &str); 9] = [
    ("combinators.sth", "times-helper"),
    ("combinators.sth", "times"),
    ("combinators.sth", "each"),
    ("combinators.sth", "map"),
    ("combinators.sth", "fold"),
    ("combinators.sth", "filter"),
    ("combinators.sth", "while"),
    ("core.sth", "if"),
    ("core.sth", "unless"),
];

/// Build and run `src`, returning the built binary's path (left in place for
/// the caller to remove), stdout, and exit code.
fn build_and_run(name: &str, src: &str) -> (std::path::PathBuf, String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
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

/// X4: `arrays.sth`'s `bin_search`/`sort` retype their comparator to
/// `inline ~[ 'T 'T -- i64 ]` and still run. The call site spells the tilde
/// (part C, `phase4_slice12_partc.rs`, requires it once this file's own
/// mechanical corpus migration lands).
#[test]
fn retyped_array_words_still_run() {
    let src = format!(
        "{}{}: main ( -- )\n\
         0 4 fill | d |\n\
         &!d 0 >usize &!> 4 !\n\
         &!d 1 >usize &!> 2 !\n\
         &!d 2 >usize &!> 1 !\n\
         &!d 3 >usize &!> 3 !\n\
         0 4 fill | s |\n\
         d s ~[ | x y | x y sub ] a::sort\n\
         | ra rs | rs drop\n\
         &ra 0 >usize &> @ .\n\
         &ra 1 >usize &> @ .\n\
         &ra 2 >usize &> @ .\n\
         &ra 3 >usize &> @ .\n\
         ra 3 ~[ | x y | x y sub ] a::bin_search | arr i found |\n\
         found . i >i64 . arr drop ;\n",
        combinators_import("c"),
        arrays_import("a"),
    );
    let (binary, stdout, code) = build_and_run("slice12-partab-arrays", &src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n2\n3\n4\ntrue\n2\n");
}

fn combinators_import(qualifier: &str) -> String {
    format!(
        "import: {qualifier} \"{}/lib/combinators.sth\" ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn arrays_import(qualifier: &str) -> String {
    format!(
        "import: {qualifier} \"{}/lib/arrays.sth\" ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}
