//! P7b.S3 exit goldens: the zero-cost splice of a trait member reached
//! through a bound. Phase 2 covers the splice-*caller* path (S3-6 .. S3-10)
//! and lands two positives: P#1, the exit witness, and P#2, the row-7 shape
//! S3-8 exists for. Harness style from `tests/phase7b_slice2.rs`
//! (`single_file_hosted`), with the binary *kept* after the run because
//! S3-13's clauses 2-5 are assertions about the emitted binary.

mod common;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use common::call_graph;

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs3-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `tests/phase7b_slice2.rs`'s hosted single-file fixture, verbatim but for
/// the package name.
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs3 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

/// Build, run, and keep the binary: `(binary, stdout)`. The binary is
/// deleted with the `Tree`, so clauses 2-5 can still read its symbols.
fn build_run_keep(tag: &str, src: &str) -> (Tree, PathBuf, String) {
    let (t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the binary should run");
    assert_eq!(
        run.status.code(),
        Some(0),
        "the built binary should exit 0; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    (t, binary, stdout)
}

/// `binary`'s symbol *names*: `nm`'s last whitespace-separated field per
/// line. S3-13 requires the split field rather than a `contains()` over the
/// whole output -- a substring test is satisfied by a typo and by any longer
/// symbol that embeds the name.
fn symbols(binary: &Path) -> Vec<String> {
    let nm = Command::new("nm")
        .arg(binary)
        .output()
        .expect("nm should run");
    let text = String::from_utf8_lossy(&nm.stdout).into_owned();
    let names: Vec<String> = text
        .lines()
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .collect();
    assert!(
        names.iter().any(|s| s == "sooth_main"),
        "sanity: nm reads this binary's symbols at all:\n{text}"
    );
    names
}

/// Every symbol reachable from `sooth_main` through `objdump`'s call edges,
/// plus every edge target named on the way. Clause 3 is about reachable
/// edges, not about what the binary happens to link.
fn reachable_from_main(binary: &Path) -> HashSet<String> {
    let graph = call_graph(binary);
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from(["sooth_main".to_string()]);
    while let Some(name) = queue.pop_front() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(callees) = graph.get(&name) {
            queue.extend(callees.iter().cloned());
        }
    }
    seen
}

/// The two symbol shapes a `Sized::size` member can take: the mangled member
/// word (`size.3b.Sized.3b.<idx>.3b.<target>`) and the monomorph
/// (`sooth_mono_size_Sized_<idx>_<target>...`). S3-13 clause 2 requires
/// *both*: a generic-target member mints the monomorph form, so an assertion
/// written only against the `.3b.` form is vacuous for exactly the shapes
/// this slice is about.
fn is_size_member_symbol(s: &str) -> bool {
    s.contains("size.3b.Sized") || s.starts_with("sooth_mono_size_Sized")
}

/// The fixture the whole matrix is cut from (spec S3-3's p3 shape): a Star-kind
/// trait, a *generic* impl target, and an inline bound caller. `member_inline`
/// selects matrix rows 3 (`true`) and 7 (`false`); `caller_inline` selects the
/// caller flavour.
fn sized_box_fixture(member_inline: bool, caller_inline: bool) -> String {
    let member = if member_inline { "size inline" } else { "size" };
    let caller = if caller_inline {
        ": usesize inline['S: Sized] ( 'S -- i64 ) size ;"
    } else {
        ": usesize['S: Sized] ( 'S -- i64 ) size ;"
    };
    format!(
        "trait: Sized['S] :\n  {member} ( 'S -- i64 ) ;\n;\n\
         type: Box['T] v 'T ;\n\
         impl: Sized for Box['T]\n  : size drop 1 ;\n;\n\
         {caller}\n\
         : mkbox ( i64 -- Box[i64] ) Box ;\n\
         : main ( -- ) 3 mkbox usesize . ;\n"
    )
}

/// Golden P#1 (matrix row 3), the Phase 2 exit witness: an `inline` trait
/// member on a *generic* impl target, reached through a bound on an `inline`
/// caller, splices. The full S3-13 contract:
///
/// 1. stdout and exit code;
/// 2. `nm`: neither the mangled member symbol nor the monomorph is present;
/// 3. `objdump`: no call edge to either, reachable from `sooth_main`;
/// 4. the non-inline member twin in this same test *does* mint the monomorph
///    and *is* called -- without it, (2) and (3) are assertions that a string
///    is absent from a binary, which a typo in the pattern satisfies just as
///    well as a splice;
/// 5. the caller declares `inline`, so its own frame must be absent too.
///    A *poly* caller mints no `W__m0` symbol in either flavour (it is
///    monomorphized per instantiation), so the load-bearing half of clause 5
///    is the absence of `sooth_mono_usesize*`, with the non-inline-caller
///    control below as the positive twin that shows the frame present.
#[test]
fn inline_member_splices_into_an_inline_bound_caller() {
    let (_t, binary, stdout) = build_run_keep("p1-row3", &sized_box_fixture(true, true));

    // Clause 1.
    assert_eq!(stdout, "1\n");

    // Clause 2.
    let syms = symbols(&binary);
    let member: Vec<&String> = syms.iter().filter(|s| is_size_member_symbol(s)).collect();
    assert!(
        member.is_empty(),
        "an `inline` member mints no symbol; nm found: {member:?}"
    );

    // Clause 3.
    let reached = reachable_from_main(&binary);
    let called: Vec<&String> = reached
        .iter()
        .filter(|s| is_size_member_symbol(s))
        .collect();
    assert!(
        called.is_empty(),
        "no call edge to the member is reachable from `sooth_main`: {called:?}"
    );

    // Clause 5: the inline caller's own frame.
    let caller: Vec<&String> = syms
        .iter()
        .filter(|s| *s == "usesize__m0" || s.starts_with("sooth_mono_usesize"))
        .collect();
    assert!(
        caller.is_empty(),
        "an `inline` caller mints no frame either; nm found: {caller:?}"
    );
    let caller_called: Vec<&String> = reached
        .iter()
        .filter(|s| *s == "usesize__m0" || s.starts_with("sooth_mono_usesize"))
        .collect();
    assert!(
        caller_called.is_empty(),
        "no call edge to the inline caller's frame: {caller_called:?}"
    );

    // Clause 4: the non-inline member twin, byte-identical but for the
    // keyword. It monomorphizes, so the symbol IS there and IS called --
    // which is what makes the absences above non-vacuous.
    let (_t2, twin, twin_stdout) = build_run_keep("p1-row3-twin", &sized_box_fixture(false, true));
    assert_eq!(twin_stdout, "1\n");
    let twin_syms = symbols(&twin);
    assert!(
        twin_syms.iter().any(|s| is_size_member_symbol(s)),
        "control: the non-inline twin mints the member symbol; nm: {twin_syms:?}"
    );
    let twin_reached = reachable_from_main(&twin);
    assert!(
        twin_reached.iter().any(|s| is_size_member_symbol(s)),
        "control: the non-inline twin's member is called from `sooth_main`"
    );

    // Clause 5's positive twin: the same program with a *non-inline* caller
    // keeps its caller frame, so the `sooth_mono_usesize*` assertion above is
    // not a placebo. (Matrix row 2's concrete target: row 4 -- a generic
    // target through a non-inline caller -- is Phase 3's P#5.)
    let concrete = "\
trait: Sized['S] :
  size inline ( 'S -- i64 ) ;
;
type: Boxi v i64 ;
impl: Sized for Boxi
  : size drop 7 ;
;
: usesize['S: Sized] ( 'S -- i64 ) size ;
: main ( -- ) 3 Boxi usesize . ;
";
    let (_t3, mono_caller, mono_stdout) = build_run_keep("p1-caller-frame-control", concrete);
    assert_eq!(mono_stdout, "7\n");
    let mono_syms = symbols(&mono_caller);
    assert!(
        mono_syms
            .iter()
            .any(|s| s.starts_with("sooth_mono_usesize")),
        "control: a non-inline poly caller keeps its own frame; nm: {mono_syms:?}"
    );
}

/// Golden P#2 (matrix row 7), the row S3-8 exists for: a **non**-inline
/// member on a generic impl target, reached from an `inline` caller. Before
/// S3-8 this was rejected (`` `Box[i64]` does not satisfy `Sized` ``) purely
/// because the splice path's impl lookup ran with empty registries and
/// `generics: None`; after it, the row must behave exactly like row 8 (the
/// non-inline caller): accept, run, and monomorphize.
///
/// So this golden asserts S3-13 clause 1 plus the **presence** of the
/// monomorph. Clause 5 is exempt (a monomorphization golden asserting the
/// member symbol's presence cannot also assert the caller frame is absent
/// without contradicting its own point).
#[test]
fn non_inline_member_on_a_generic_target_from_an_inline_caller() {
    let (_t, binary, stdout) = build_run_keep("p2-row7", &sized_box_fixture(false, true));
    assert_eq!(stdout, "1\n");
    let syms = symbols(&binary);
    let mono: Vec<&String> = syms
        .iter()
        .filter(|s| s.starts_with("sooth_mono_size_Sized_0_Box"))
        .collect();
    assert!(
        !mono.is_empty(),
        "a non-inline member on a generic target monomorphizes; nm: {syms:?}"
    );
}

/// The p4 HKT fixture (spec S3-3), P#3's program: `Opt`/`Res`, an
/// Arrow-kinded `Functor` bound, and one `twice` caller dispatching `map`
/// through both impls. `member_inline`/`caller_inline` cut the twins.
fn functor_fixture(member_inline: bool, caller_inline: bool) -> String {
    let member = if member_inline { "map inline" } else { "map" };
    let caller = if caller_inline {
        ": twice inline['F: Functor 'T 'E] ( 'F['T 'E] [ 'T -- 'T ] -- 'F['T 'E] )"
    } else {
        ": twice['F: Functor 'T 'E] ( 'F['T 'E] [ 'T -- 'T ] -- 'F['T 'E] )"
    };
    format!(
        "type: Opt['T 'E] | None | Some 'T 'E ;\n\
         type: Res['T 'E] | Ok 'T | Err 'E ;\n\
         trait: Functor['F: * -> * -> *] :\n\
           {member} ( 'F['T 'E] [ 'T -- 'U ] -- 'F['U 'E] ) ;\n\
         ;\n\
         impl: Functor for Opt\n\
           : map swap ~[ ( Some ) Some> swap rot call swap Some ] ~[ ( None ) drop drop None ] Opt? ;\n\
         ;\n\
         impl: Functor for Res\n\
           : map swap ~[ ( Ok ) Ok> swap call Ok ] ~[ ( Err ) Err> swap drop Err ] Res? ;\n\
         ;\n\
         {caller}\n\
           | q |\n\
           q map\n\
           q map ;\n\
         : showopt ( Opt[i64 i64] -- ) ~[ ( Some ) Some> drop . ] ~[ ( None ) drop ] Opt? ;\n\
         : showres ( Res[i64 i64] -- ) ~[ ( Ok ) Ok> . ] ~[ ( Err ) Err> . ] Res? ;\n\
         : mkopt ( i64 -- Opt[i64 i64] ) dup Some ;\n\
         : mkres ( i64 -- Res[i64 i64] ) Ok ;\n\
         : main ( -- ) 1 mkopt [ 1 sub ] twice showopt\n\
           5 mkres [ 1 sub ] twice showres ;\n"
    )
}

fn is_map_member_symbol(s: &str) -> bool {
    s.contains("map") && (s.contains("Functor") || s.starts_with("sooth_mono_map"))
}

/// Golden P#3 (the slice's exit criterion): the HKT member (`map inline` on
/// an Arrow-kinded bound) splices through an `inline` caller. Full S3-13
/// contract, clauses 1-5, with the non-inline twin as clause 4's control.
#[test]
fn hkt_member_splices_through_an_inline_bound_caller() {
    let (_t, binary, stdout) = build_run_keep("p3-hkt", &functor_fixture(true, true));

    // Clause 1: both impls dispatched from one `twice`, correct values.
    assert_eq!(stdout, "-1\n3\n");

    // Clause 2: no member symbol, no monomorph.
    let syms = symbols(&binary);
    let member: Vec<&String> = syms.iter().filter(|s| is_map_member_symbol(s)).collect();
    assert!(
        member.is_empty(),
        "an `inline` HKT member mints no symbol; nm found: {member:?}"
    );

    // Clause 3: no call edge reachable from `sooth_main`.
    let reached = reachable_from_main(&binary);
    let called: Vec<&String> = reached.iter().filter(|s| is_map_member_symbol(s)).collect();
    assert!(
        called.is_empty(),
        "no call edge to the member from `sooth_main`: {called:?}"
    );

    // Clause 5: the inline caller's own frame is absent.
    let caller: Vec<&String> = syms
        .iter()
        .filter(|s| *s == "twice__m0" || s.starts_with("sooth_mono_twice"))
        .collect();
    assert!(
        caller.is_empty(),
        "an `inline` caller mints no frame; nm found: {caller:?}"
    );

    // Clause 4 twin. A single-MEMBER-axis twin (`functor_fixture(false,
    // true)`, row 7 HKT) is NOT constructible today -- measured: a non-inline
    // HKT member reached from an inline caller fails grounding its
    // output-only `'U` against the CtorImage-bound `'F`
    // (`ground_member_sig_via_theta` has no output-only recovery; only the
    // S3-1.c hop's seeded path does). Pinned below so this confound flips
    // loudly when the row starts building; until then the clause 4 twin cuts
    // both axes.
    {
        let (_t_rej, entry) = single_file_hosted("p3-hkt-row7", &functor_fixture(false, true));
        let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
            .arg("build")
            .arg(&entry)
            .output()
            .expect("sooth build should spawn");
        let stderr = String::from_utf8_lossy(&build.stderr).into_owned();
        assert!(
            !build.status.success() && stderr.contains("grounded against the constructor image"),
            "row 7 HKT is a known rejection today; if it builds, promote it to \
             the single-axis clause 4 twin here: {stderr}"
        );
    }
    let (_t2, twin, twin_stdout) = build_run_keep("p3-hkt-twin", &functor_fixture(false, false));
    assert_eq!(twin_stdout, "-1\n3\n");
    let twin_syms = symbols(&twin);
    assert!(
        twin_syms.iter().any(|s| is_map_member_symbol(s)),
        "control: the non-inline twin mints member symbols; nm: {twin_syms:?}"
    );

    // Clause 5's positive twin: byte-identical to the golden but for the
    // CALLER's `inline` keyword only (row 4 HKT builds). The non-inline poly
    // caller keeps its own monomorph frame, which is what makes the
    // `sooth_mono_twice*` absence above an assertion about splicing rather
    // than about a string never minted at all.
    let (_t3, caller_ctl, ctl_stdout) =
        build_run_keep("p3-hkt-caller-frame-control", &functor_fixture(true, false));
    assert_eq!(ctl_stdout, "-1\n3\n");
    let ctl_syms = symbols(&caller_ctl);
    assert!(
        ctl_syms.iter().any(|s| s.starts_with("sooth_mono_twice")),
        "control: a non-inline poly caller keeps its `twice` frame; nm: {ctl_syms:?}"
    );
}

/// The P#4 fixture, cut by the caller's `inline` keyword only (the member is
/// always `inline`): two splices of one member body at two θ.
fn take_fixture(caller_inline: bool) -> String {
    let caller = if caller_inline {
        ": take2 inline['F: Take 'T 'E] ( 'F['T 'E] 'E -- 'E ) take ;"
    } else {
        ": take2['F: Take 'T 'E] ( 'F['T 'E] 'E -- 'E ) take ;"
    };
    format!(
        "type: P v i64 w i64 ;\n\
         type: Opt['T 'E] | None | Some 'T 'E ;\n\
         trait: Take['F: * -> * -> *] :\n\
           take inline ( 'F['T 'E] 'E -- 'E ) ;\n\
         ;\n\
         impl: Take for Opt\n\
           : take | o d | o ~[ ( Some ) Some> swap drop d drop ] ~[ ( None ) drop d ] Opt? ;\n\
         ;\n\
         {caller}\n\
         : mk1 ( i64 i64 -- Opt[i64 i64] ) Some ;\n\
         : mk2 ( P i64 -- Opt[P i64] ) Some ;\n\
         : main ( -- ) 1 2 mk1 7 take2 .\n\
           1 2 P 4 mk2 9 take2 . ;\n"
    )
}

/// The two symbol shapes the `Take::take` member can mint, mirroring
/// `is_size_member_symbol` (both are required or the assertion is vacuous for
/// the generic-target shape).
fn is_take_member_symbol(s: &str) -> bool {
    s.contains("take.3b.Take") || s.starts_with("sooth_mono_take_Take")
}

/// Golden P#4: two splices of one member body at two θ resolve their enum
/// sites independently -- without per-splice (`(uid, span)`) resolution this
/// is a miscompile (both splices share one bare-key family layout), not a
/// rejection, so the golden runs and checks stdout (clause 1, the
/// load-bearing clause: mutation check 1 fails here). The
/// two-different-`EnumId`s assertion lives in the in-process unit tests
/// (`check/poly.rs`), since this harness cannot see `Module` state. Clauses
/// 2/3/5 are carried too: the member and the `inline` caller both splice, so
/// neither mints a symbol nor takes a call edge.
#[test]
fn two_splices_of_one_member_at_two_thetas_resolve_independently() {
    let (_t, binary, stdout) = build_run_keep("p4-two-thetas", &take_fixture(true));

    // Clause 1.
    assert_eq!(
        stdout, "2\n4\n",
        "each splice must resolve its enum sites at its own theta"
    );

    // Clause 2: neither member symbol shape is present.
    let syms = symbols(&binary);
    let member: Vec<&String> = syms.iter().filter(|s| is_take_member_symbol(s)).collect();
    assert!(
        member.is_empty(),
        "an `inline` member mints no symbol; nm found: {member:?}"
    );

    // Clause 3: no call edge to either shape, reachable from `sooth_main`.
    let reached = reachable_from_main(&binary);
    let called: Vec<&String> = reached
        .iter()
        .filter(|s| is_take_member_symbol(s))
        .collect();
    assert!(
        called.is_empty(),
        "no call edge to the member from `sooth_main`: {called:?}"
    );

    // Clause 5: `take2` declares `inline`, so its own frame is absent too
    // (the W__m0 recipe: a poly caller mints no `take2__m0` in either
    // flavour, so the load-bearing half is the monomorph's absence).
    let caller: Vec<&String> = syms
        .iter()
        .filter(|s| *s == "take2__m0" || s.starts_with("sooth_mono_take2"))
        .collect();
    assert!(
        caller.is_empty(),
        "an `inline` caller mints no frame; nm found: {caller:?}"
    );

    // Clause 5's positive control: byte-identical but for the caller's
    // `inline` keyword. The non-inline caller keeps its monomorph frame,
    // making the absence above non-vacuous.
    let (_t2, ctl, ctl_stdout) = build_run_keep("p4-caller-frame-control", &take_fixture(false));
    assert_eq!(ctl_stdout, "2\n4\n");
    let ctl_syms = symbols(&ctl);
    assert!(
        ctl_syms.iter().any(|s| s.starts_with("sooth_mono_take2")),
        "control: a non-inline poly caller keeps its `take2` frame; nm: {ctl_syms:?}"
    );
}

/// Golden P#5 (matrix row 4), S3-1.d's witness: the same `inline` member on a
/// generic target reached through a **non-inline** poly caller. Clauses 1-4;
/// clause 5 exempt (the caller keeps its frame -- that is the point). The
/// load-bearing half: the member's monomorph symbol is now absent while the
/// program still runs.
#[test]
fn inline_member_on_generic_target_splices_from_a_non_inline_poly_caller() {
    let (_t, binary, stdout) = build_run_keep("p5-row4", &sized_box_fixture(true, false));

    // Clause 1.
    assert_eq!(stdout, "1\n");

    // Clause 2: the member's monomorph is absent (present pre-S3-1.d, F9).
    let syms = symbols(&binary);
    let member: Vec<&String> = syms.iter().filter(|s| is_size_member_symbol(s)).collect();
    assert!(
        member.is_empty(),
        "a diverted `inline` member instantiation mints no monomorph; nm: {member:?}"
    );

    // Clause 3: no call edge from `sooth_main`.
    let reached = reachable_from_main(&binary);
    let called: Vec<&String> = reached
        .iter()
        .filter(|s| is_size_member_symbol(s))
        .collect();
    assert!(
        called.is_empty(),
        "no call edge to the member from `sooth_main`: {called:?}"
    );

    // Clause 5's exemption, witnessed: the caller keeps its own monomorph
    // frame (that is this golden's point).
    assert!(
        syms.iter().any(|s| s.starts_with("sooth_mono_usesize")),
        "the non-inline caller keeps its frame; nm: {syms:?}"
    );

    // Clause 4 control proper: a byte-identical program but for the member's
    // `inline` keyword mints the member monomorph -- the positive twin that
    // makes the load-bearing absence assertion above non-vacuous.
    let (_t2, twin, twin_stdout) =
        build_run_keep("p5-row4-member-twin", &sized_box_fixture(false, false));
    assert_eq!(twin_stdout, "1\n");
    let twin_syms = symbols(&twin);
    assert!(
        twin_syms.iter().any(|s| is_size_member_symbol(s)),
        "control: the non-inline member twin mints its monomorph; nm: {twin_syms:?}"
    );
}
