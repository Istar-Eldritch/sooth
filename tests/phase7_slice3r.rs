//! P7.S3r goldens: `impl:` bodies. A `: member ... ;` inside an `impl:` block
//! desugars at parse time to a synthesized top-level word whose effect is the
//! trait member's signature grounded at the `for` type, plus the binding pair
//! the hand-written form used to spell out. So these goldens assert the *whole*
//! pipeline (the synthesized word must mangle, link, and run), the parse-time
//! rejections the form introduces, and the resolution rules for a name inside a
//! member body -- its own (rewritten to the synthesized word, so a member can
//! recurse) and a sibling's (not rewritten, so it resolves by ordinary lookup).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A scratch package tree naming this repo's own `lib/` as `core`, so a fixture
/// can `import: core::prelude`/`core::bool` (every body form here needs `if`).
/// Removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3r-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sooth.pkg"), common::fixture_package(tag)).unwrap();
        Tree(dir)
    }

    /// `new`, plus `module:` entries so the tree's sibling files are importable
    /// as `self::<name>` (a file inside a package cannot be reached by path).
    fn with_modules(tag: &str, modules: &str) -> Tree {
        let t = Tree::new(tag);
        let pkg = common::fixture_package(tag)
            .replace("\nlayer:", &format!("\nmodule: {modules} ;\nlayer:"));
        std::fs::write(t.0.join("sooth.pkg"), pkg).unwrap();
        t
    }

    fn write(&self, name: &str, src: &str) {
        std::fs::write(self.0.join(name), src).unwrap();
    }

    fn entry(&self, src: &str) -> PathBuf {
        let path = self.0.join("main.sth");
        std::fs::write(&path, src).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn program(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.entry(src);
    (t, entry)
}

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

fn build_ok(entry: &Path) {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    std::fs::remove_file(entry.with_extension("")).ok();
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn build_and_run(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    std::fs::remove_file(&binary).ok();
    String::from_utf8_lossy(&run.stdout).into_owned()
}

/// Build, run, and also read the linked binary's symbol table: a synthesized
/// member word is only real if it survives mangling and reaches the executable
/// under the escaped spelling `qbe_name` gives its `;` delimiters.
fn build_run_and_symbols(entry: &Path) -> (String, Vec<String>) {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    let nm = Command::new("nm")
        .arg(&binary)
        .output()
        .expect("nm should run");
    std::fs::remove_file(&binary).ok();
    assert!(run.status.success(), "the built binary should exit 0");
    let names = String::from_utf8_lossy(&nm.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .collect();
    (String::from_utf8_lossy(&run.stdout).into_owned(), names)
}

/// The dogfood program: two single-member traits, each implemented for `Point`
/// by a body that never restates its signature. Both members are reached only
/// through `show_larger`'s bound, so this is the whole path -- desugar, mangle,
/// bound-directed dispatch, lowering, link -- in one program.
#[test]
fn impl_body_form_builds_and_runs() {
    let (_t, entry) = program(
        "body-form",
        "import: intrinsics * ;\n\
         import: core::prelude | if Bool lt gt | ;\n\
         type: Rank | Under | Same | Over ;\n\
         trait: Order 'T\n\
           : cmp ( &'T &'T -- Rank ) ;\n\
         ;\n\
         trait: Show 'T\n\
           : show ( &'T -- ) ;\n\
         ;\n\
         type: Point x i64 y i64 ;\n\
         impl: Order for Point\n\
           : cmp\n\
             | a b |\n\
             a &y @ | ay |\n\
             b &y @ | by |\n\
             ay by lt\n\
             ~[ Under ]\n\
             ~[\n\
               ay by gt\n\
               ~[ Over ]\n\
               ~[\n\
                 a &x @ | ax |\n\
                 b &x @ | bx |\n\
                 ax bx lt\n\
                 ~[ Under ]\n\
                 ~[ ax bx gt ~[ Over ] ~[ Same ] if ] if\n\
               ] if\n\
             ] if ;\n\
         ;\n\
         impl: Show for Point\n\
           : show\n\
             | p | \"(\" . p &x @ . \",\" . p &y @ . \")\" . ;\n\
         ;\n\
         : show_larger ( &'T: Order Show &'T -- )\n\
           | b | | a |\n\
           a b cmp\n\
           ~[ ( Under ) drop b show ]\n\
           ~[ ( Same ) drop a show ]\n\
           ~[ ( Over ) drop a show ]\n\
           Rank? ;\n\
         : main ( -- )\n\
           0 0 Point | origin |\n\
           3 4 Point | corner |\n\
           &origin &corner show_larger\n\
           origin drop\n\
           corner drop ;\n",
    );
    let (stdout, symbols) = build_run_and_symbols(&entry);
    assert_eq!(stdout, "(3\n,4\n)");
    for synth in [
        "cmp.3b.Order.3b.0.3b.Point__m0",
        "show.3b.Show.3b.0.3b.Point__m0",
    ] {
        assert!(
            symbols.iter().any(|s| s == synth),
            "the desugared member must reach the binary as `{synth}`; nm found:\n{symbols:#?}"
        );
    }
}

/// R2: the inherited signature is the only one. Restating it at the impl site
/// is a second spelling of the same thing, so it is rejected rather than
/// compared.
#[test]
fn impl_body_restated_signature_is_rejected() {
    let (_t, entry) = program(
        "restated-signature",
        "import: intrinsics * ;\n\
         type: Point n i64 ;\n\
         trait: Getter 'T : get ( &'T -- i64 ) ; ;\n\
         impl: Getter for Point\n\
           : get ( &Point -- i64 ) | p | p &n @ ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "impl member `get` must not restate its signature at line 5, col 7 \
             (it is inherited from trait `Getter`'s `get` with the `for` type)"
        ),
        "{err}"
    );
}

/// R6: a body naming something the trait does not require has no member to
/// bind. Left to become a free module-private word it would silently swallow a
/// misspelled member name.
#[test]
fn impl_body_non_member_is_rejected() {
    let (_t, entry) = program(
        "non-member",
        "import: intrinsics * ;\n\
         type: Point n i64 ;\n\
         trait: Getter 'T : get ( &'T -- i64 ) ; ;\n\
         impl: Getter for Point\n\
           : get | p | p &n @ ;\n\
           : bogus | p | p &n @ ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("`bogus` is not a member of trait `Getter` at line 6, col 3"),
        "{err}"
    );
}

/// R4a: the member name is the member's own word inside its body, so a `| ... |`
/// binder cannot also claim it. The rewrite is unconditional token equality, so
/// either reading would be a silent shadow.
#[test]
fn impl_body_binder_named_after_the_member_is_rejected() {
    let (_t, entry) = program(
        "binder-shadows-member",
        "import: intrinsics * ;\n\
         type: Point n i64 ;\n\
         trait: Getter 'T : get ( &'T -- i64 ) ; ;\n\
         impl: Getter for Point\n\
           : get | get | get &n @ ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "`get` binds a local inside its own impl body at line 5, col 7, \
             where the name already refers to the member itself"
        ),
        "{err}"
    );
}

/// R4: a member spelled as a name that resolves ahead of the word environment
/// is rejected at the `trait:` declaration -- the site where the unimplementable
/// member is written, and the site that also covers a trait no impl ever names.
/// Each category carries its own message: two of the three are the rejections
/// `parse_worddef` already produces for an ordinary word declaration, and
/// calling a caret name "a builtin word" would be wrong on its face.
#[test]
fn trait_member_named_after_a_builtin_is_rejected() {
    // The name-dispatched builtins: an operator, a shuffle, and a `>`-prefixed
    // conversion (which the `BUILTIN_WORDS` const does not even list -- the
    // predicate claims every non-empty `>`-prefixed name).
    for member in ["max", "dup", ">u8"] {
        let (_t, entry) = program(
            "builtin-member",
            &format!("trait: Getter 'T\n  : {member} ( &'T -- i64 ) ;\n;\n: main ( -- ) ;\n"),
        );
        let err = build_error(&entry);
        assert!(
            err.contains(&format!(
                "trait `Getter` declares a member named `{member}`, which is a builtin word (line 2, col 5)"
            )),
            "{err}"
        );
        assert!(
            err.contains(
                "note: a trait member becomes a word when implemented, and inside its own body \
                 the name would shadow the builtin"
            ),
            "{err}"
        );
    }
    let (_t, entry) = program(
        "access-word-member",
        "trait: Getter 'T\n  : @ ( &'T -- i64 ) ;\n;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "`@` is a builtin access word (`@`, `!`, `+!`) and cannot be redefined at line 2, col 5"
        ),
        "{err}"
    );
    let (_t, entry) = program(
        "caret-member",
        "trait: Getter 'T\n  : ^cell ( &'T -- i64 ) ;\n;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "`^cell` is reserved for the owning-cell syntax (`^`, `^>`, `^|>`) and cannot be \
             used as a word name at line 2, col 5"
        ),
        "{err}"
    );
}

/// R4's negative space, and the regression guard on the one predicate choice
/// that ruling could get wrong. The six surface comparisons sit in
/// `BUILTIN_WORDS` for `has_self_tail_call`'s benefit only: they are `core::cmp`
/// words, not name-dispatched, so an `eq` member's self-binding shadows a
/// library word, which is exactly the shadowing the body form admits. Rejecting
/// on the raw const instead would take the `Eq` trait with it.
#[test]
fn trait_member_named_after_a_comparison_is_accepted() {
    let (_t, entry) = program(
        "comparison-member",
        "import: intrinsics * ;\n\
         import: core::prelude | lt | ;\n\
         import: core::bool | Bool | ;\n\
         type: Point n i64 ;\n\
         trait: Keyed 'T : eq ( &'T &'T -- Bool ) ; ;\n\
         impl: Keyed for Point\n\
           : eq | a b | a &n @ b &n @ lt ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    build_ok(&entry);
}

/// R4a: a call of the member's own name inside its body is rewritten to the
/// synthesized word, so a member body can recurse. Without the rewrite the bare
/// `count` binds nothing at all (the member name is not a module-scope word --
/// `main` reaches it only through `counted`'s bound), so this program is exactly
/// as good at failing on a no-op implementation as at passing on a real one.
#[test]
fn impl_body_member_calls_itself_recursively() {
    let (_t, entry) = program(
        "recursive-member",
        "import: intrinsics * ;\n\
         import: core::prelude | if Bool gt | ;\n\
         type: Counter n i64 ;\n\
         trait: Countdown 'T : count ( 'T -- i64 ) ; ;\n\
         impl: Countdown for Counter\n\
           : count\n\
             | c |\n\
             c Counter> | n |\n\
             n 0 gt\n\
             ~[ n 1 sub Counter count 1 add ]\n\
             ~[ 0 ]\n\
             if ;\n\
         ;\n\
         : counted ( 'T: Countdown -- i64 ) count ;\n\
         : main ( -- ) 3 Counter counted . ;\n",
    );
    assert_eq!(build_and_run(&entry), "3\n");
}

/// Recon O3's functional witness: two traits sharing a member name (`get`), both
/// implemented for `Point`, reached through two different bounds. The literal-name
/// mutation tests only prove the synthesized spelling *contains* the trait; this proves
/// the trait component actually disambiguates the two call sites at dispatch time.
#[test]
fn impl_body_trait_qualifier_disambiguates_shared_member_name() {
    let (_t, entry) = program(
        "shared-member-name",
        "import: intrinsics * ;\n\
         type: Point x i64 y i64 ;\n\
         trait: Getter 'T : get ( &'T -- i64 ) ; ;\n\
         trait: Setter 'T : get ( &'T -- i64 ) ; ;\n\
         impl: Getter for Point\n\
           : get | p | p &x @ ;\n\
         ;\n\
         impl: Setter for Point\n\
           : get | p | p &y @ ;\n\
         ;\n\
         : via_getter ( &'T: Getter -- i64 ) get ;\n\
         : via_setter ( &'T: Setter -- i64 ) get ;\n\
         : main ( -- )\n\
           3 4 Point | p |\n\
           &p via_getter .\n\
           &p via_setter .\n\
           p drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "3\n4\n");
}

/// Recon O3 across modules: the two traits sharing the member name `get` are
/// *also* named the same, differing only in declaring module. The trait
/// component of the synthesized name therefore has to carry the module id --
/// the bare declared name alone would collide, surfacing as a `duplicate
/// word` between the two members' synthesized names.
#[test]
fn impl_body_disambiguates_same_named_traits_from_two_modules() {
    let t = Tree::with_modules("same-named-traits", "a b");
    t.write(
        "a.sth",
        "export: Getter ;\ntrait: Getter 'T : get ( &'T -- i64 ) ; ;\n",
    );
    t.write(
        "b.sth",
        "export: Getter ;\ntrait: Getter 'T : get ( &'T -- i64 ) ; ;\n",
    );
    let entry = t.entry(
        "import: intrinsics * ;\n\
         import: self::a ;\n\
         import: self::b ;\n\
         type: Point x i64 y i64 ;\n\
         impl: a::Getter for Point\n\
           : get | p | p &x @ ;\n\
         ;\n\
         impl: b::Getter for Point\n\
           : get | p | p &y @ ;\n\
         ;\n\
         : via_a ( &'T: a::Getter -- i64 ) get ;\n\
         : via_b ( &'T: b::Getter -- i64 ) get ;\n\
         : main ( -- )\n\
           3 4 Point | p |\n\
           &p via_a .\n\
           &p via_b .\n\
           p drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "3\n4\n");
}

/// The unterminated-block EOF path: the block runs to end of file with no
/// closing `;` at all, so the existing EOF diagnostic fires.
#[test]
fn impl_body_unterminated_block_at_eof_is_error() {
    let (_t, entry) = program(
        "unterminated-eof",
        "import: intrinsics * ;\n\
         type: Point n i64 ;\n\
         trait: Getter 'T : get ( &'T -- i64 ) ; ;\n\
         impl: Getter for Point\n\
           : get | p | p &n @ ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("unterminated `impl:` declaration"), "{err}");
}

/// The unterminated-block absorption path: a missing closing `;` followed by another
/// top-level `: name ... ;` declaration has no lookahead to distinguish that declaration
/// from a further member, so it is consumed as an attempted member instead -- surfacing
/// as a non-member error naming the absorbed declaration, not as an EOF error. Nothing is
/// silently swallowed (the absorbed declaration is still rejected, just under a different
/// diagnostic), but the two unterminated-block paths are different and both are pinned.
#[test]
fn impl_body_unterminated_block_absorbs_next_decl() {
    let (_t, entry) = program(
        "unterminated-absorbs",
        "import: intrinsics * ;\n\
         type: Point n i64 ;\n\
         trait: Getter 'T : get ( &'T -- i64 ) ; ;\n\
         impl: Getter for Point\n\
           : get | p | p &n @ ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("`main` is not a member of trait `Getter`"),
        "{err}"
    );
}

/// R3 (Phase 2): a body-form impl member whose body leaves the wrong effect is
/// rejected by ordinary in-body stack-effect checking -- the retired
/// signature-mismatch guard's intent, relocated (R6/Phase 4) -- and the message
/// names the member readably, never the raw synthesized spelling a user never
/// wrote and cannot type.
#[test]
fn impl_body_wrong_effect_names_readable_member() {
    let (_t, entry) = program(
        "wrong-effect",
        "import: intrinsics * ;\n\
         type: Ordering | Less | Equal | Greater ;\n\
         type: Point x i64 y i64 ;\n\
         trait: Order 'T : cmp ( &'T &'T -- Ordering ) ; ;\n\
         impl: Order for Point\n\
           : cmp | a b | a drop b drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert_eq!(
        err,
        "error: error: stack effect mismatch in `cmp` (member of trait `Order` for `Point`) (line 6)\n  body leaves 0 values, but ( … ) declares 1 outputs\n  note: declared ( &Point &Point -- Ordering )\n"
    );
}

/// P7.S3r Phase 4 (R6): the relocated intent of the retired
/// `check_impl_decls_signature_mismatch_is_error` guard. The neighbouring
/// goldens use a single-member impl, where the only signature in scope is
/// trivially the right one. Here a two-member impl breaks only its *second*
/// member, so the `note:` is the discriminating part: it proves `cmp`'s body
/// was checked against `cmp`'s own inherited signature rather than `lo`'s.
/// Nothing in the source restates either one for the checker to read.
#[test]
fn impl_body_wrong_effect_is_rejected_in_body() {
    let (_t, entry) = program(
        "wrong-effect-rejected-in-body",
        "import: intrinsics * ;\n\
         type: Ordering | Less | Equal | Greater ;\n\
         type: Point x i64 y i64 ;\n\
         trait: Order 'T\n\
           : lo ( &'T -- i64 ) ;\n\
           : cmp ( &'T &'T -- Ordering ) ;\n\
         ;\n\
         impl: Order for Point\n\
           : lo | p | p &x @ ;\n\
           : cmp | a b | a drop b drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert_eq!(
        err,
        "error: error: stack effect mismatch in `cmp` (member of trait `Order` for `Point`) (line 10)\n  body leaves 0 values, but ( … ) declares 1 outputs\n  note: declared ( &Point &Point -- Ordering )\n"
    );
}

/// R3 (Phase 2), the type-mismatch sibling of the arity golden above: a body
/// that leaves the declared *count* of outputs but the wrong *type* is the
/// closest analogue of the retired signature-mismatch class, and its message
/// must render the member readably too, not just the arity variant.
#[test]
fn impl_body_wrong_effect_type_names_readable_member() {
    let (_t, entry) = program(
        "wrong-effect-type",
        "import: intrinsics * ;\n\
         type: Ordering | Less | Equal | Greater ;\n\
         type: Point x i64 y i64 ;\n\
         trait: Order 'T : cmp ( &'T &'T -- Ordering ) ; ;\n\
         impl: Order for Point\n\
           : cmp | a b | a drop b drop 0 ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert_eq!(
        err,
        "error: error: type mismatch in `cmp` (member of trait `Order` for `Point`) (line 6)\n  body leaves `i64` where the declaration requires `Ordering`\n  note: declared ( &Point &Point -- Ordering )\n"
    );
}

/// R3 (Phase 2), the *in-body operand* family: the two goldens above both come
/// from checking the body's overall effect against the declaration, which is one
/// diagnostic constructor. A wrong body far more often trips an operand check
/// mid-body instead, a different constructor reading the enclosing word out of
/// the same `Ctx` -- so the member has to render readably there too, or R3 holds
/// only for the message a user is least likely to see first.
#[test]
fn impl_body_underflow_names_readable_member() {
    let (_t, entry) = program(
        "body-underflow",
        "import: intrinsics * ;\n\
         type: Point n i64 ;\n\
         trait: Getter 'T : get ( &'T -- i64 ) ; ;\n\
         impl: Getter for Point\n\
           : get | p | p drop add ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert_eq!(
        err,
        "error: error: stack effect mismatch in `get` (member of trait `Getter` for `Point`) (line 5)\n  `add` needs 2 values, but the stack holds 0\n  note: declared ( &Point -- i64 )\n"
    );
    // Survives a re-blessing of the wording above: the raw synthesized
    // spelling is unforgeable (`;` is a lexer delimiter), so its appearance in
    // any diagnostic is always the leak R3 forbids, whatever the message says.
    assert!(!err.contains("get;Getter"), "{err}");
}

/// R3 (Phase 2), the unknown-word family: a third constructor, and the one a
/// typo in a member body reaches. Distinct from the operand golden above in that
/// the offending word does not resolve at all, so the message names two words
/// (the unknown one, and the member enclosing it) and only the second renders.
#[test]
fn impl_body_unknown_word_names_readable_member() {
    let (_t, entry) = program(
        "body-unknown-word",
        "import: intrinsics * ;\n\
         type: Point n i64 ;\n\
         trait: Getter 'T : get ( &'T -- i64 ) ; ;\n\
         impl: Getter for Point\n\
           : get | p | p bogus ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert_eq!(
        err,
        "error: error: unknown word `bogus` in `get` (member of trait `Getter` for `Point`) (line 5)\n"
    );
    assert!(!err.contains("get;Getter"), "{err}");
}

/// R3 (Phase 2), the other half of the mechanism: the three goldens above all
/// reach diagnostics that read `Ctx::Word`'s `mangled` directly, while a second
/// family goes through the `rendered_word_or` accessor. Reverting that accessor
/// alone leaves those three passing, so this fixture -- an intrinsic called in a
/// member body of a file with no `import: intrinsics` line -- is what pins it.
#[test]
fn impl_body_ungated_intrinsic_names_readable_member() {
    let (_t, entry) = program(
        "body-ungated-intrinsic",
        "type: Point n i64 ;\n\
         trait: Getter 'T : get ( &'T -- i64 ) ; ;\n\
         impl: Getter for Point\n\
           : get | p | p &n @ 1 add ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert_eq!(
        err,
        "error: error: `add` is an intrinsic and is not imported in `get` (member of trait `Getter` for `Point`) (line 4, col 22)\n  add `import: intrinsics * ;` (or `import: intrinsics | add ... | ;`) to this file\n"
    );
    assert!(!err.contains("get;Getter"), "{err}");
}

/// R7: a member body sees its own name, never its siblings'. `hash`'s body calls
/// `eq`, which is *not* rewritten, so it resolves by ordinary lookup to
/// `core::cmp`'s `eq` on two `i64`s -- not to the sibling member, whose grounded
/// effect (`&Point &Point -- Bool`) would reject those operands. Compiling and
/// printing 7 is the witness. If sibling access is ever wanted, this is the
/// golden that has to be consciously overturned.
#[test]
fn impl_body_sibling_call_does_not_reach_the_sibling() {
    let (_t, entry) = program(
        "sibling-call",
        "import: intrinsics * ;\n\
         import: core::prelude | if eq lt | ;\n\
         import: core::bool | Bool | ;\n\
         type: Point n i64 ;\n\
         trait: Keyed 'T\n\
           : eq ( &'T &'T -- Bool ) ;\n\
           : hash ( &'T -- i64 ) ;\n\
         ;\n\
         impl: Keyed for Point\n\
           : eq | a b | a &n @ b &n @ lt ;\n\
           : hash | p | p &n @ | n | n n eq ~[ 7 ] ~[ 9 ] if ;\n\
         ;\n\
         : hashed ( &'T: Keyed -- i64 ) hash ;\n\
         : main ( -- ) 4 Point | p | &p hashed . p drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "7\n");
}
