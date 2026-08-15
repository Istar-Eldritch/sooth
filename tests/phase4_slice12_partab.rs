//! Phase 4 Slice 12, phase 1 exit criteria (parts A + B).
//!
//! `is_combinator` is declared, not inferred (R-A1, unit-tested in
//! `src/check/combinators.rs`); a `~[ ... ]` parameter without `inline` is a
//! located error (R-B1, unit-tested in `src/check/word_entry.rs`). This file
//! covers the two goldens that need a real build: X2 (the nine library words
//! mint no symbol) and X4 (`arrays.sth`'s retyped `bin_search`/`sort` still
//! run, an ordinary `[ ... ]` literal still satisfying their new `~[ ... ]`
//! parameter until part C requires the tilde).

/// Build and run `src`, returning the built binary's path (left in place so a
/// caller can inspect its symbol table), stdout, and exit code.
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

fn nm_symbols(binary: &std::path::Path) -> String {
    let nm = std::process::Command::new("nm")
        .arg(binary)
        .output()
        .expect("nm should run");
    String::from_utf8_lossy(&nm.stdout).into_owned()
}

/// X2: the nine `lib/combinators.sth`/`lib/core.sth` words that gained
/// `inline` this slice (`times-helper`, `times`, `each`, `map`, `fold`,
/// `filter`, `while`, `if`, `unless`) are still combinators after part A
/// retires the inference leg -- each mints no symbol.
#[test]
fn migrated_library_words_mint_no_symbol() {
    let src = format!(
        "{}: main ( -- )\n\
         3 [ 1 + drop ] c::times\n\
         0 4 fill [ 1 + drop ] c::each\n\
         0 4 fill [ 1 + ] c::map drop\n\
         0 4 fill 0 [ + ] c::fold drop\n\
         0 4 fill [ 2 > ] c::filter drop drop\n\
         0 [ dup 3 < [ 1 + true ] [ false ] if ] c::while drop\n\
         true [ 1 ] [ 2 ] if drop\n\
         false [ 1 ] [ 2 ] unless drop ;\n",
        combinators_import("c"),
    );
    let (binary, _stdout, code) = build_and_run("slice12-partab-nosym", &src);
    let symbols = nm_symbols(&binary);
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    for name in [
        "times-helper",
        "times",
        "each",
        "map",
        "fold",
        "filter",
        "while",
        "if",
        "unless",
    ] {
        let mangled = format!("{name}__m");
        assert!(
            !symbols.contains(&mangled),
            "`{name}` gained `inline` this slice and must mint no symbol; nm found:\n{symbols}"
        );
    }
    assert!(
        symbols.contains("main"),
        "sanity: nm reads this binary's symbols at all:\n{symbols}"
    );
}

/// X4: `arrays.sth`'s `bin_search`/`sort` retype their comparator to
/// `inline ~[ 'T 'T -- i64 ]` and still run, an ordinary `[ ... ]` literal at
/// the call site silently satisfying the new `~` parameter (the required-tilde
/// check is part C, not this phase).
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
         d s [ | x y | x y - ] a::sort\n\
         | ra rs | rs drop\n\
         &ra 0 >usize &> @ .\n\
         &ra 1 >usize &> @ .\n\
         &ra 2 >usize &> @ .\n\
         &ra 3 >usize &> @ .\n\
         ra 3 [ | x y | x y - ] a::bin_search | arr i found |\n\
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
