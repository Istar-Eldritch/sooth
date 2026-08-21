//! Phase 4 slice 10c, P3: the machine-level primitives and the library words
//! over them.
//!
//! After this slice the compiler knows three primitives -- `branch` (a
//! conditional jump on a 32-bit flag taking two quotations), `tag` (a scalar
//! enum's discriminant as that flag) and the six comparison primitives that
//! produce it -- and `bool`, `if`, `unless` and `eq`/`lt`/`gt`/`lte`/`gte`/`ne` are
//! ordinary words in `lib/core.sth`. `TermKind::If` and the `if`/`else`/`end`
//! grammar are gone.

use sooth::ir::{lower, CmpOp, Instr, IrFunc, IrType, Terminator};
use sooth::{backend, check, lexer, test_support};

mod common;

fn lowered(src: &str) -> Vec<IrFunc> {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    lower(&module).expect("lowering should succeed").funcs
}

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

fn func<'f>(funcs: &'f [IrFunc], name: &str) -> &'f IrFunc {
    funcs
        .iter()
        .find(|f| f.name.starts_with(name))
        .unwrap_or_else(|| panic!("`{name}` is lowered"))
}

fn instrs(f: &IrFunc) -> Vec<&Instr> {
    f.blocks.iter().flat_map(|b| b.instrs.iter()).collect()
}

fn self_calls(f: &IrFunc) -> usize {
    instrs(f)
        .iter()
        .filter(|i| matches!(i, Instr::Call(_, sym, _) if *sym == f.name))
        .count()
}

fn back_edges(f: &IrFunc) -> usize {
    f.blocks
        .iter()
        .filter(|b| matches!(b.term, Terminator::Jmp(target) if target.0 <= b.id.0))
        .count()
}

// -- E-P3-1: `if` is a library word, not a primitive -------------------------

/// The load-bearing discriminator. `nm`-silence and "a call site emits a
/// jump-and-join with no `Instr::Call`" both pass *identically* whether `if` is
/// a library word or still a compiler-known construct, since the construct also
/// emitted a jump-and-join and minted no symbol; only the resolution tells the
/// two apart. Mutation: leave the primitive in place and this fails, because
/// `lib/bool.sth` would not be where `if` comes from.
#[test]
fn if_resolves_to_a_library_word_definition() {
    let core = test_support::core_lib_words();
    let if_word = core
        .iter()
        .find(|w| w.name == "if")
        .expect("`if` is a `lib/bool.sth` word definition");
    assert!(
        check::is_combinator(if_word),
        "`if` is an always-spliced word, so no call site mints a symbol for it"
    );
    let sig = if_word.poly.as_ref().expect("`if` is row-polymorphic");
    assert_eq!(
        sig.row_var_names,
        vec!["..a".to_string(), "..b".to_string()],
        "`if`'s two rows differ, which is what lets a branch change the stack shape"
    );
    assert_eq!(sig.inputs.len(), 3, "a condition and two branch quotations");
    assert!(
        core.iter().any(|w| w.name == "unless"),
        "`unless` ships beside it"
    );

    // ...and it is absent from builtin dispatch: nothing in the checker's
    // operator table or the name-dispatched builtin list claims `if`.
    assert!(
        !check::builtin_table().contains_key("if"),
        "`if` is not a builtin operator row"
    );
}

/// A user program is free to define `cond`, `then-arm` and `else-arm`: `if`
/// and `unless`'s own locals are `if--cond`/`if--then-arm`/`if--else-arm` (and
/// the `unless--` equivalents), not the bare names, so they never collide with
/// a word a program defines under those names -- including `cond`, which
/// DESIGN.md documents as a future multi-way branch word this same file would
/// otherwise grow into colliding with.
#[test]
fn if_locals_do_not_collide_with_user_words_named_cond_or_arm() {
    let src = ": cond ( i64 -- i64 ) 1 add ;\n\
               : then-arm ( i64 -- i64 ) 1 add ;\n\
               : else-arm ( i64 -- i64 ) 1 add ;\n\
               : main ( -- ) 1 cond then-arm else-arm . ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
}

/// The `if` a program actually calls is that definition, spliced: a body using
/// it mints no symbol for it and emits the jump-and-join directly.
#[test]
fn a_call_to_if_splices_the_library_definition() {
    let funcs = lowered(": w ( bool -- i64 ) ~[ 1 ] ~[ 2 ] if ;\n: main ( -- ) true w . ;\n");
    assert!(
        !funcs.iter().any(|f| f.name.starts_with("if")),
        "no `IrFunc` is minted for `if`"
    );
    let w = func(&funcs, "w");
    assert!(
        w.blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))),
        "the call site branches directly"
    );
}

// -- E-P3-2: the grammar and the node are gone -------------------------------

/// `grep -r "TermKind::If" src/` is empty. Run as a test rather than by hand so
/// a reintroduced arm cannot pass unnoticed.
#[test]
fn term_kind_if_has_no_remaining_references() {
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("src/ is readable") {
            let path = entry.expect("a readable dir entry").path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("a readable source file");
                // A mention inside a comment is not a reference to the
                // variant: the grep the criterion states is over code, and the
                // comments that record *why* the arm is gone are exactly what a
                // later reader needs.
                let code_mentions = text
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .any(|l| l.contains("TermKind::If"));
                if code_mentions {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    let mut hits = Vec::new();
    walk(std::path::Path::new("src"), &mut hits);
    assert!(
        hits.is_empty(),
        "`TermKind::If` is deleted; still referenced in {hits:?}"
    );
}

/// A source written against the old grammar no longer parses, and the
/// diagnostic names the replacement rather than falling out as a bare unknown
/// word (`if` itself now parses fine -- it is an ordinary call).
#[test]
fn the_if_else_end_grammar_no_longer_parses() {
    let tokens = lexer::lex(": w ( bool -- i64 ) if 1 else 2 end ;").expect("lexing succeeds");
    let err = test_support::parse_with_core(&tokens).expect_err("the old grammar is gone");
    assert!(err.contains("`else`"), "unexpected message: {err}");
    assert!(
        err.contains("~[ then ] ~[ else ] if"),
        "the diagnostic points at the replacement: {err}"
    );
}

// -- E-P3-3: `tag`'s domain --------------------------------------------------

/// `tag` on a scalar enum is a genuine no-op: operand and result are both
/// 32-bit, so the lowered `tag` carries **no width conversion** and **no memory
/// access**. Stated in bit widths, never a QBE register class (`src/ir/types.rs`
/// keeps the IR backend-neutral).
///
/// Mutation: make `tag` return a target-width integer and a `Conv` appears;
/// make it read a discriminant field and a `FieldLoad` appears.
#[test]
fn tag_on_a_scalar_enum_is_a_no_op() {
    let funcs = lowered(": w ( bool -- u32 ) tag ;\n: main ( -- ) true w drop ;\n");
    let w = func(&funcs, "w");
    let body = instrs(w);
    let tag = body
        .iter()
        .find_map(|i| match i {
            Instr::Tag(dst, src) => Some((*dst, *src)),
            _ => None,
        })
        .expect("the `tag` operation is lowered");
    assert_eq!(
        w.value_types[tag.0 .0 as usize],
        IrType::Int {
            bits: 32,
            signed: false
        },
        "the flag is a 32-bit unsigned integer"
    );
    assert!(
        !body.iter().any(|i| matches!(i, Instr::Conv(..))),
        "no width conversion: operand and result are the same width"
    );
    assert!(
        !body.iter().any(|i| matches!(
            i,
            Instr::Load(..) | Instr::FieldLoad(..) | Instr::Store(..) | Instr::FieldStore(..)
        )),
        "no memory access: a scalar enum value already *is* its discriminant"
    );
}

/// `tag` outside its domain is a located error at **check** time, which is why
/// the `is_scalar` predicate is computed from the enum declaration rather than
/// read out of `ir::layout`.
#[test]
fn tag_on_a_payload_carrying_enum_is_a_located_check_error() {
    let err = check_error(
        "type: E | None | Some v i64 ;\n: w ( E -- u32 ) tag ;\n: main ( -- ) None w drop ;\n",
    );
    assert!(
        err.contains("`tag` requires an enum whose variants all carry no payload"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`E`"), "names the offending enum: {err}");
    assert!(err.contains("line 2"), "the error is located: {err}");
}

#[test]
fn tag_on_a_non_enum_is_a_located_check_error() {
    let err = check_error(": w ( i64 -- u32 ) tag ;\n: main ( -- ) 1 w drop ;\n");
    assert!(
        err.contains("`tag` requires an enum operand"),
        "unexpected message: {err}"
    );
    assert!(err.contains("line 1"), "the error is located: {err}");
}

// -- E-P3-4: comparisons are library words, at no cost -----------------------

/// Part 1: the six surface names resolve to `lib/` definitions and no
/// comparison builtin row remains.
#[test]
fn the_six_comparisons_are_library_words() {
    let core = test_support::core_lib_words();
    let table = check::builtin_table();
    for name in ["eq", "lt", "gt", "lte", "gte", "ne"] {
        let word = core
            .iter()
            .find(|w| w.name == name)
            .unwrap_or_else(|| panic!("`{name}` is a `lib/cmp.sth` word"));
        assert!(
            word.declares_inline,
            "`{name}` is `inline`, or every comparison becomes a real call"
        );
        let sig = word
            .poly
            .as_ref()
            .unwrap_or_else(|| panic!("`{name}` stays polymorphic over the numeric tower"));
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
        assert!(
            !table.contains_key(name),
            "`{name}` left `BUILTIN_TABLE` for `lib/`"
        );
    }
    for name in ["ueq", "ult", "ugt", "ulte", "ugte", "une"] {
        assert!(
            table.contains_key(name),
            "`{name}` carries the per-numeric-type rows now"
        );
    }
}

/// Part 2: the comparison *primitive* emits the same `Instr::Cmp` op over the
/// same operands the retired builtin row did.
#[test]
fn a_comparison_primitive_emits_one_cmp_over_its_operands() {
    let funcs = lowered(": w ( i64 i64 -- u32 ) ult ;\n: main ( -- ) 1 2 w drop ;\n");
    let w = func(&funcs, "w");
    let body = instrs(w);
    let cmps: Vec<_> = body
        .iter()
        .filter_map(|i| match i {
            Instr::Cmp(dst, op, lhs, rhs) => Some((*dst, *op, *lhs, *rhs)),
            _ => None,
        })
        .collect();
    assert_eq!(cmps.len(), 1, "one comparison, no diamond");
    let (dst, op, lhs, rhs) = cmps[0];
    assert_eq!(op, CmpOp::Lt);
    assert_eq!(
        (w.value_types[lhs.0 as usize], w.value_types[rhs.0 as usize]),
        (w.params[0], w.params[1]),
        "the two declared operands, in order"
    );
    assert_eq!(
        w.value_types[dst.0 as usize],
        IrType::Int {
            bits: 32,
            signed: false
        }
    );
}

/// Operators-as-words (Phase 1): all six renamed unsigned primitives still
/// lower to their own `CmpOp`, one comparison each, over the declared
/// operands in order -- the same shape `a_comparison_primitive_emits_one_cmp_over_its_operands`
/// checks for `ult` alone, now covering the whole family so a rename that
/// missed a `CmpOp` mapping fails here.
#[test]
fn check_ueq_family_lowers_to_cmpop() {
    for (name, op) in [
        ("ueq", CmpOp::Eq),
        ("ult", CmpOp::Lt),
        ("ugt", CmpOp::Gt),
        ("ulte", CmpOp::Le),
        ("ugte", CmpOp::Ge),
        ("une", CmpOp::Ne),
    ] {
        let funcs = lowered(&format!(
            ": w ( i64 i64 -- u32 ) {name} ;\n: main ( -- ) 1 2 w drop ;\n"
        ));
        let w = func(&funcs, "w");
        let cmps: Vec<_> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::Cmp(dst, cmp_op, lhs, rhs) => Some((*dst, *cmp_op, *lhs, *rhs)),
                _ => None,
            })
            .collect();
        assert_eq!(cmps.len(), 1, "`{name}`: one comparison, no diamond");
        let (_, cmp_op, lhs, rhs) = cmps[0];
        assert_eq!(cmp_op, op, "`{name}` lowers to the wrong `CmpOp`");
        assert_eq!(
            (w.value_types[lhs.0 as usize], w.value_types[rhs.0 as usize]),
            (w.params[0], w.params[1]),
            "`{name}`: the two declared operands, in order"
        );
    }
}

/// Part 3: the canonical `a b eq if ... ...` pattern costs nothing. The library
/// `eq` is spliced, so its call site mints no symbol and emits no `Instr::Call`;
/// the branch-and-construct diamond it adds in IR is what QBE folds away.
///
/// Measured mutation: drop `inline` from a comparison word and the program
/// stops building at all -- a polymorphic word's `effect` is empty, so without
/// the splice its own body checks against a zero-arity signature. The `inline`
/// declaration is not an optimisation on this path, it is what makes a
/// polymorphic comparison word exist.
#[test]
fn the_canonical_comparison_and_branch_costs_no_call() {
    let funcs = lowered(": w ( i64 i64 -- i64 ) eq ~[ 1 ] ~[ 2 ] if ;\n: main ( -- ) 1 2 w . ;\n");
    assert!(
        !funcs.iter().any(|f| f.name.starts_with("eq")),
        "no `IrFunc` is minted for the library `eq`"
    );
    let w = func(&funcs, "w");
    assert!(
        !instrs(w).iter().any(|i| matches!(i, Instr::Call(..))),
        "the comparison and the branch are both spliced, so no call is emitted"
    );
    assert!(
        instrs(w)
            .iter()
            .any(|i| matches!(i, Instr::Cmp(_, CmpOp::Eq, _, _))),
        "the comparison is still one `Cmp`"
    );
}

/// The emitted assembly of word `w` in `src`, isolated from the surrounding
/// runtime shims. The word is emitted unmangled (`w:`), so the block runs from
/// its label to the `.type` directive QBE closes every function with.
fn word_w_assembly(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = lower(&module).expect("lowering should succeed");
    let il = backend::qbe::emit(&ir).expect("QBE IL emission should succeed");

    let dir = std::env::temp_dir().join(format!(
        "slice10c_mc_{}_{:p}",
        std::process::id(),
        src.as_ptr()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let ssa = dir.join("w.ssa");
    let asm = dir.join("w.s");
    std::fs::write(&ssa, &il).expect("writing QBE IL");
    let status = std::process::Command::new("qbe")
        .arg(&ssa)
        .arg("-o")
        .arg(&asm)
        .status()
        .expect("qbe runs (the corpus_stdout suite already requires it)");
    assert!(status.success(), "qbe exited {status}");
    let s = std::fs::read_to_string(&asm).expect("reading assembly");
    std::fs::remove_dir_all(&dir).ok();

    let start = s
        .find("\nw:")
        .expect("`w` is emitted as an unmangled label")
        + 1;
    let end = start
        + s[start..]
            .find(".type w,")
            .expect("QBE closes `w` with a `.type` directive");
    s[start..end].trim_end().to_string()
}

/// Part 3, at the machine-code level. The IR-level test above pins that the
/// library `eq`/`if` mint no call and add one `Cmp`; this pins the stronger
/// claim the spec's R-P3-3a actually makes: the library form costs *nothing*
/// over the raw primitives. `eq [ 1 ] [ 2 ] if` lowers to two branch-and-
/// construct diamonds in QBE IL (one for `eq`'s `bool`, one for `if`); the raw
/// primitive `ueq [ 1 ] [ 2 ] branch` lowers to one. QBE folds both to the same
/// branchless machine code, so the abstraction is free.
///
/// This is a *relative* equivalence through one QBE on one host, not a pinned
/// absolute assembly string: the rest of the suite pins portable QBE IL for a
/// reason, and an x86 golden would rot on a different target or QBE version.
/// The primitive `ueq ... branch` form is exactly the post-migration lowering of
/// the pre-migration `= if 1 else 2 end`; that the two agree was cross-checked
/// out of band against the compiler rebuilt at the pre-P3 checkpoint (builtin
/// `eq` + `TermKind::If`), whose `w` was the same `cmov`.
///
/// Mutation: if the library `if`/`eq` stop splicing (a real `call`, an
/// unfoldable extra `bool` materialisation, a lost `inline`) the two blocks
/// diverge or the build breaks.
#[test]
fn the_library_if_folds_to_the_same_machine_code_as_the_branch_primitive() {
    let library = word_w_assembly(": w ( i64 i64 -- i64 ) eq ~[ 1 ] ~[ 2 ] if ;");
    let primitive = word_w_assembly(": w ( i64 i64 -- i64 ) ueq [ 1 ] [ 2 ] branch ;");
    assert_eq!(
        library, primitive,
        "library `=`/`if` must fold to the same machine code as raw `u=`/`branch`"
    );
}

/// Part 4: the library replacement stayed `'T: Copy Ord`-polymorphic over the
/// whole tower rather than silently narrowing to `i64`, which a sole `i64`
/// worked example would have masked.
///
/// Mutation: monomorphize a comparison word to `i64` and this fails to check.
#[test]
fn the_library_comparisons_cover_non_i64_numeric_types() {
    let src = ": main ( -- )\n  \
               1 >u32 2 >u32 lt .\n  \
               5 >i8 5 >i8 eq .\n  \
               3 >u32 2 >u32 gte .\n  \
               1.5 2.5 ne . ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("the comparisons cover `u32`, `i8` and `f64` too");
}

// -- E-P3-5: constant stack preserved, on IR ---------------------------------

/// `gcd`, `countdown` and `filter_while` still lower to a `jmp` back-edge with
/// no self `Instr::Call`, now through the library `if`. Asserted on lowered IR
/// rather than inferred from output: real recursion computes the right answer
/// too, right up until it overflows.
///
/// Mutation: mis-shape `if`'s body so `branch` is not its tail term, and the
/// self-call reappears as an `Instr::Call`.
#[test]
fn corpus_loops_still_lower_to_a_back_edge_through_the_library_if() {
    for (source, word) in [
        ("examples/gcd.sth", "gcd__m0"),
        // `qbe_name` escapes the `-`, so `sum-to` is emitted `sum.2d.to`.
        ("examples/countdown.sth", "sum.2d.to__m0"),
        ("examples/filter_while_hand.sth", "fixpoint__m0"),
    ] {
        let ssa = sooth::driver::emit_ssa_with_manifest(
            std::path::Path::new(source),
            common::manifest_for(std::path::Path::new(source)).as_deref(),
        )
        .unwrap_or_else(|e| panic!("emitting {source}: {e}"));
        // The word's own emitted function block, so an unrelated runtime
        // helper's `call` cannot satisfy or break the assertion.
        let start = ssa
            .find(&format!("export function l ${word}"))
            .unwrap_or_else(|| panic!("`{word}` is emitted in {source}"));
        let end = start + ssa[start..].find("\n}\n").expect("the block closes");
        let block = &ssa[start..end];
        assert!(
            !block.contains(&format!("call ${word}(")),
            "`{word}` still recurses: {block}"
        );
        assert!(
            block.contains("\tjmp @blk1\n"),
            "`{word}` has no loop back-edge: {block}"
        );
    }
}

/// The IR-level twin, over a hand-written program rather than the corpus, so
/// the shape is asserted on `Instr`/`Terminator` and not on emitted text.
#[test]
fn a_self_tail_through_the_library_if_lowers_to_a_back_edge() {
    let funcs = lowered(
        ": sum-to ( i64 i64 -- i64 )\n  \
         | n | | acc | n 0 eq ~[ acc ] ~[ acc n add n 1 sub sum-to ] if ;\n\
         : main ( -- ) 0 10 sum-to . ;\n",
    );
    let w = func(&funcs, "sum-to");
    assert_eq!(self_calls(w), 0, "the self-call became the back-edge");
    assert!(back_edges(w) >= 1, "a loop back-edge is emitted");
}

// -- the whole-slice witness -------------------------------------------------

/// A program using `if`, `unless` and `while` entirely from `lib/`,
/// self-tailing through a branch: the same output and the same loop shape (a
/// `jmp` back-edge, no self `Instr::Call`) as its pre-slice equivalent built
/// with the compiler-known `if`. Ties P1's tail-splice recognition, P2's row
/// gate and P3's primitives together in one program.
#[test]
fn the_whole_slice_witness_runs_and_keeps_its_loop_shape() {
    let src = format!(
        "import: \"{}/lib/combinators.sth\" c ;\n\
               : classify ( i64 -- i64 ) dup 10 lt ~[ 1 ] ~[ 2 ] if swap drop ;\n\
               : countdown ( i64 i64 -- i64 )\n  \
               | n | | acc | n 0 eq ~[ acc ] ~[ acc n add n 1 sub countdown ] if ;\n\
               : main ( -- )\n  \
               3 classify .\n  \
               30 classify .\n  \
               true ~[ 0 ] ~[ 1 ] unless .\n  \
               0 ~[ dup 5 lt ~[ 1 add true ] ~[ false ] if ] c::while .\n  \
               0 100 countdown . ;\n",
        env!("CARGO_MANIFEST_DIR")
    );
    let path = std::env::temp_dir().join(format!("sooth-10c-witness-{}.sth", std::process::id()));
    common::write_fixture(&path, &src).expect("writing the witness should succeed");
    let binary = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("the witness builds");
    let out = std::process::Command::new(&binary)
        .output()
        .expect("the witness runs");
    let ssa = sooth::driver::emit_ssa_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("the witness emits");
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&binary).ok();

    assert_eq!(
        String::from_utf8(out.stdout).expect("utf-8"),
        "1\n2\n1\n5\n5050\n"
    );
    let start = ssa
        .find("export function l $countdown__m0")
        .expect("`countdown` is emitted");
    let end = start + ssa[start..].find("\n}\n").expect("the block closes");
    let block = &ssa[start..end];
    assert!(
        !block.contains("call $countdown__m0("),
        "the self-call is the back-edge, not a call: {block}"
    );
    assert!(block.contains("\tjmp @blk1\n"), "a loop is opened: {block}");
}
