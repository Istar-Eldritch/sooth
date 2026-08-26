//! REPL: compile each line through the normal pipeline to a shared object and
//! `dlopen` it into the session process (no interpreter, no JIT).
//!
//! `Session` owns the persistent stack buffer and the word env (arity +
//! generation + symbol); the read-eval-print loop lexes/parses/checks/lowers/
//! emits/compiles/loads each line exactly like `build`, differing only in
//! target (`.so` not a binary) and in carrying state across lines.

use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::ast::Module;
use crate::ast::{
    ArrayDecl, ArrayId, CallInst, EnumDecl, EnumId, ImportTarget, Line, OwnedCellDecl, OwnedCellId,
    PolySig, PolyType, RefDecl, RefId, SliceDecl, SliceId, Span, StackEffect, StructDecl, StructId,
    Term, TermKind, Type, TypedSlot, VariantDecl, WordDef,
};
use crate::check::{self, word_span, Sig};
use crate::driver;
use crate::editor;
use crate::ir::ArrayLayout;
use crate::ir::{self, EnumLayout, IrModule, StructLayout};
use crate::lexer::Token;
use crate::resolve::split_destructure_suffix;
use crate::{backend, lexer, parser};

// RTLD_NOW is 2 on both Linux and macOS; RTLD_GLOBAL's value differs.
const RTLD_NOW: c_int = 2;
#[cfg(target_os = "linux")]
const RTLD_GLOBAL: c_int = 0x100;
#[cfg(target_os = "macos")]
const RTLD_GLOBAL: c_int = 0x8;

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
    fn fflush(stream: *mut c_void) -> c_int;
}

/// A loaded shared object. The session keeps every handle resident (never
/// `dlclose`) so symbols from earlier lines stay callable by later ones.
pub struct Library {
    handle: *mut c_void,
}

impl Library {
    /// Open a shared object with global visibility, so its exports resolve for
    /// objects loaded by later lines.
    pub fn open(path: &Path) -> Result<Library, String> {
        let cpath = CString::new(path.as_os_str().as_bytes())
            .map_err(|e| format!("path has interior nul: {e}"))?;
        // SAFETY: cpath is a valid nul-terminated C string for the call's duration.
        let handle = unsafe {
            dlerror(); // clear any stale error
            dlopen(cpath.as_ptr(), RTLD_NOW | RTLD_GLOBAL)
        };
        if handle.is_null() {
            return Err(format!("dlopen {path:?} failed: {}", last_dlerror()));
        }
        Ok(Library { handle })
    }

    /// Resolve an exported symbol to a raw pointer (caller transmutes to a fn).
    pub fn symbol(&self, name: &str) -> Result<*mut c_void, String> {
        let cname = CString::new(name).map_err(|e| format!("symbol has interior nul: {e}"))?;
        // SAFETY: handle came from a successful dlopen; cname is nul-terminated.
        let sym = unsafe {
            dlerror();
            dlsym(self.handle, cname.as_ptr())
        };
        if sym.is_null() {
            return Err(format!("dlsym {name:?} failed: {}", last_dlerror()));
        }
        Ok(sym)
    }
}

fn last_dlerror() -> String {
    // SAFETY: dlerror returns either null or a valid C string owned by libdl.
    unsafe {
        let p = dlerror();
        if p.is_null() {
            "unknown error".to_string()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// A session's knowledge of one user-defined word: its typed effect, the
/// generation counter it was last (re)defined at, and the mangled symbol that
/// generation exports. Redefinition bumps the generation and mints a new symbol;
/// calls compiled before the redefinition keep resolving to the old symbol (it's
/// still resident, never `dlclose`d), calls compiled after resolve to the new
/// one.
struct WordEntry {
    sig: Sig,
    generation: u64,
    symbol: String,
}

/// R4 (Slice 2): a session's knowledge of one polymorphic word. Unlike a
/// `WordEntry`, no symbol is minted at the defining line: a polymorphic word
/// has no concrete instantiation to lower there. The body is retained because
/// it is lowered *later*, once per instantiating line, from an AST the session
/// would otherwise have thrown away; `resolver` is the **frozen** callee-name
/// -> mangled-symbol map captured from `self.env` at the defining line (D3), so
/// an instantiation binds its callees against the defining line's generations,
/// not the instantiating line's. `generation` stamps every minted
/// instantiation symbol (`__gen{N}`), so a redefinition's instantiations can
/// never collide with an earlier generation's under `RTLD_GLOBAL` (R8).
///
/// `ir_lower_env` is the **frozen** callee arity map (D3), the other half of
/// the binding `resolver` freezes: lowering an instantiation must determine
/// each callee's arity/return type from the defining line's env, not the
/// instantiating line's, or a callee redefined at a different arity between
/// the two lines emits the resolved (frozen) symbol under the wrong ABI.
struct PolyWordEntry {
    generation: u64,
    word: WordDef,
    resolver: HashMap<String, String>,
    ir_lower_env: HashMap<String, ir::Arity>,
    /// The defining-line body's resolved-overload record (R7), frozen
    /// alongside `resolver`/`ir_lower_env`: a later instantiation lowers this
    /// word's own body, so it must dispatch a builtin-overloaded call site the
    /// same way the definition-time check resolved it, not the empty map.
    /// Omitting this compiled the call and then lowered it as the builtin,
    /// which crashed rather than merely mis-dispatched (segfault, not a wrong
    /// answer) on pointer/aggregate operands.
    builtin_overloads: HashMap<Span, String>,
}

/// Derive ir's arity map (RK2) from the typed checker env: ir needs the
/// input/output counts, the output `IrType`, and which inputs are ordinary
/// `[ ... ]` quotations (R-D2), not the full typed effect. The REPL's env
/// never carries more than one candidate per name (its redefinition model
/// keeps exactly one live binding), so the sole candidate answers.
fn ir_arity_env(env: &HashMap<String, Vec<check::Overload>>) -> HashMap<String, ir::Arity> {
    env.iter()
        .map(|(name, overloads)| {
            let sig = &overloads[0].sig;
            let ret = sig.outputs.first().map(|&ty| ir::ir_type_of(ty));
            (
                name.clone(),
                ir::Arity {
                    in_arity: sig.inputs.len(),
                    out_arity: sig.outputs.len(),
                    ret_ty: ret,
                    quot_inputs: ir::quot_input_slots(sig.inputs.iter().copied()),
                },
            )
        })
        .collect()
}

/// R2 (Slice 6c): project the session's combinator store into the checker's
/// inline view (`collect_combinators`'s shape), each value borrowing a stored
/// `WordDef`. A free function over the one field rather than a `&self` method,
/// so a caller can still borrow `self.arrays`/`self.owned_cells`/`self.refs`
/// mutably alongside it (disjoint fields).
fn checker_combinators(store: &HashMap<String, WordDef>) -> check::CombinatorEnv<'_> {
    store
        .iter()
        .map(|(name, word)| (name.clone(), vec![check::combinator_of(word)]))
        .collect()
}

/// R2 (Slice 6c): project the store into lowering's `combinator_bodies` view
/// (`ir::lower`'s shape), which since slice 10c is the same `CombinatorIndex`
/// the shared tail-splice predicate reads.
fn combinator_bodies(store: &HashMap<String, WordDef>) -> check::CombinatorIndex {
    check::combinator_index(store.values())
}

/// The mangled export symbol for `name` at `generation`.
fn mangled_symbol(name: &str, generation: u64) -> String {
    format!("{name}__gen{generation}")
}

/// R6 (slice 5b): the import-epoch symbol for a closure word. The `__import`
/// marker and the globally-unique `epoch` make it collision-free against an
/// ordinary word's `{name}__gen{N}` and against any other import event's
/// symbols, by construction.
fn import_symbol(name: &str, epoch: u64) -> String {
    format!("{name}__import{epoch}")
}

/// P7 slice 3i (R2): how an imported closure's `EnumId`s move into session
/// space. Every enum shifts by the append `base`, except the closure's own
/// `core::bool`, which *folds* onto the session's single seeded copy: the
/// session already holds that declaration, so appending a second one would
/// re-register `True`/`False` and shadow the session's own constructor words
/// (a `HashMap` env, last write wins), and shifting its references by `base`
/// instead would make a session `true` and an imported `if` disagree on the
/// type. The append skips the folded slot, so every enum *after* it shifts by
/// one less -- left out, that slot's absence would silently rename every
/// later enum to its neighbour.
///
/// Which slot folds starts from `resolve_bool_type`, the same shape test the
/// rest of the compiler resolves `bool` by, never by the name alone: an
/// imported enum that merely happens to be *called* `bool` (a payload-carrying
/// or three-variant one) is an unrelated type, and folding it onto the
/// session's one-cell scalar would read its tagged aggregate at the wrong
/// width.
///
/// Shape alone is not enough, though: an unrelated module can declare its own
/// `type: bool | On | Off ;` -- two payload-free variants, same name, and
/// nothing (no injection, not this module's own import) makes that a
/// `duplicate type` against the session's `core::bool` any more (P7 slice 3i
/// deleted the injection that used to guarantee it). `resolve_bool_type` alone
/// cannot tell that stranger from a real re-import of `core::bool`, so the
/// fold also requires the candidate's variant *spellings*, in order, to match
/// the session's own `bool` declaration (`False`, `True`) exactly. Folding the
/// stranger anyway would alias its `EnumId` onto the session's, so its own
/// discriminants (`On`=0/`Off`=1) get read against the session's tag meaning
/// (`False`=0/`True`=1): a session `true`/`false` would silently become a
/// legal argument to the stranger's own words, routed by tag through the
/// wrong arm.
#[derive(Clone, Copy)]
struct EnumRemap {
    base: usize,
    folded_bool: Option<EnumId>,
    session_bool: EnumId,
}

impl EnumRemap {
    fn new(
        module: &Module,
        base: usize,
        session_bool: EnumId,
        session_bool_variants: &[&'static str],
    ) -> EnumRemap {
        let folded_bool = match crate::ast::resolve_bool_type(&module.enums) {
            Some(Type::Enum(id, _))
                if module.enums[id.index()]
                    .variants
                    .iter()
                    .map(|v| v.name_static)
                    .eq(session_bool_variants.iter().copied()) =>
            {
                Some(id)
            }
            _ => None,
        };
        EnumRemap {
            base,
            folded_bool,
            session_bool,
        }
    }

    /// True for the one imported slot that maps onto the session's `bool`
    /// (and so is not appended).
    fn folds(&self, id: EnumId) -> bool {
        self.folded_bool == Some(id)
    }

    fn apply(&self, id: EnumId) -> EnumId {
        if self.folds(id) {
            return self.session_bool;
        }
        let after_folded = matches!(self.folded_bool, Some(b) if b.index() < id.index());
        EnumId::from_index(self.base + id.index() - usize::from(after_folded))
    }
}

/// R9 (slice 5b): shift a closure-local `Type`'s registry id into session
/// space by the session's registry lengths captured at splice time. A scalar
/// type carries no id and passes through unchanged.
#[allow(clippy::too_many_arguments)]
fn remap_type(
    ty: Type,
    enums: EnumRemap,
    struct_base: usize,
    array_base: usize,
    cell_base: usize,
    ref_base: usize,
    slice_base: usize,
) -> Type {
    match ty {
        Type::Struct(id, n) => Type::Struct(StructId::from_index(id.index() + struct_base), n),
        Type::Enum(id, n) => Type::Enum(enums.apply(id), n),
        Type::Array(id, n) => Type::Array(ArrayId::from_index(id.index() + array_base), n),
        Type::OwnedCell(id, n) => {
            Type::OwnedCell(OwnedCellId::from_index(id.index() + cell_base), n)
        }
        Type::Ref(id, m, n) => Type::Ref(RefId::from_index(id.index() + ref_base), m, n),
        // P7 slice 3c (R8.2): a `SliceId` indexes a per-module registry like
        // every id above, so an imported slice must shift into session space
        // too. Left in the `other => other` wildcard it would keep the
        // imported module's index and silently name whatever session slice sits
        // at that position -- the id-collision class of bug that a per-module
        // id landing in session space always is.
        Type::Slice(id, m, n) => Type::Slice(SliceId::from_index(id.index() + slice_base), m, n),
        other => other,
    }
}

/// R14 (slice 6c): the `PolyType` analogue of `remap_type` -- shift the
/// registry ids in a polymorphic signature into session space by the same
/// bases. A type/length *variable* carries no id and passes through; a
/// concrete or array-of type shifts its ids like every other imported decl.
#[allow(clippy::too_many_arguments)]
fn remap_poly_type(
    p: &PolyType,
    enums: EnumRemap,
    struct_base: usize,
    array_base: usize,
    cell_base: usize,
    ref_base: usize,
    slice_base: usize,
) -> PolyType {
    match p {
        PolyType::Concrete(t) => PolyType::Concrete(remap_type(
            *t,
            enums,
            struct_base,
            array_base,
            cell_base,
            ref_base,
            slice_base,
        )),
        PolyType::Var(id) => PolyType::Var(*id),
        // P7 slice 3b: a body-only marker, never in a declared signature.
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        PolyType::Array(inner, len) => PolyType::Array(
            Box::new(remap_poly_type(
                inner,
                enums,
                struct_base,
                array_base,
                cell_base,
                ref_base,
                slice_base,
            )),
            len.clone(),
        ),
        // Slice 13 (R-A9): the poly reference carries no `RefId` of its own
        // (that is minted only at grounding), so only the referent shifts;
        // the mutability passes through verbatim.
        PolyType::Ref(referent, mutable) => PolyType::Ref(
            Box::new(remap_poly_type(
                referent,
                enums,
                struct_base,
                array_base,
                cell_base,
                ref_base,
                slice_base,
            )),
            *mutable,
        ),
        // P7.S3n (R3): the poly cell carries no `OwnedCellId` of its own
        // (minted only at grounding), so only the payload shifts.
        PolyType::OwnedCell(payload) => PolyType::OwnedCell(Box::new(remap_poly_type(
            payload,
            enums,
            struct_base,
            array_base,
            cell_base,
            ref_base,
            slice_base,
        ))),
        PolyType::Quotation(ins, outs, is_inline, row_in, row_out) => PolyType::Quotation(
            ins.iter()
                .map(|q| {
                    remap_poly_type(
                        q,
                        enums,
                        struct_base,
                        array_base,
                        cell_base,
                        ref_base,
                        slice_base,
                    )
                })
                .collect(),
            outs.iter()
                .map(|q| {
                    remap_poly_type(
                        q,
                        enums,
                        struct_base,
                        array_base,
                        cell_base,
                        ref_base,
                        slice_base,
                    )
                })
                .collect(),
            *is_inline,
            *row_in,
            *row_out,
        ),
        // P7 slice 3a: remap each argument; the header `idx`/`module` pass
        // through unchanged (Phase 2's implementation note flags whether
        // that stays sound across import epochs -- out of scope here).
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args,
            name,
        } => PolyType::Generic {
            is_enum: *is_enum,
            idx: *idx,
            module: *module,
            args: args
                .iter()
                .map(|a| {
                    remap_poly_type(
                        a,
                        enums,
                        struct_base,
                        array_base,
                        cell_base,
                        ref_base,
                        slice_base,
                    )
                })
                .collect(),
            name,
        },
    }
}

/// R14 (slice 6c): clone a combinator's body, rewriting every `Call` that
/// names a module-0 export to its session-internal epoch-tagged spelling
/// (`rename`'s keys are the post-resolve body spellings, its values the
/// internal ones). This is the load-bearing part of import retention: an
/// imported `while`'s self-call `while` must become `{q}::while__import{epoch}`
/// or the self-tail recognizer (comparing against the combinator's `.name`)
/// misses and the splice recurses forever. A `Call` that is not a module-0
/// export (a builtin, the quotation parameter, a body local) is left alone.
fn rewrite_combinator_body_calls(terms: &[Term], rename: &HashMap<String, String>) -> Vec<Term> {
    terms
        .iter()
        .map(|term| {
            let kind = match &term.kind {
                TermKind::Call(name, type_args) => TermKind::Call(
                    rename.get(name).cloned().unwrap_or_else(|| name.clone()),
                    type_args.clone(),
                ),
                TermKind::Quotation(inner, is_inline, annot) => TermKind::Quotation(
                    rewrite_combinator_body_calls(inner, rename),
                    *is_inline,
                    annot.clone(),
                ),
                other => other.clone(),
            };
            Term {
                kind,
                span: term.span,
            }
        })
        .collect()
}

/// R13/R14 (slice 6c): build the session-retained copy of an imported module-0
/// combinator. Its signature ids are shifted into session space like every
/// other imported decl; its body calls are rewritten to their internal
/// spellings; and its `.name` is set to the internal epoch-tagged spelling the
/// checker's/lowerer's inline paths dispatch on (so a self-tail recognizer
/// comparing against `comb.word.name` still fires). No re-check: the closure is
/// already internally self-consistent from its own `check` (recon 2/5/6).
#[allow(clippy::too_many_arguments)]
fn remap_imported_combinator(
    w: &WordDef,
    internal: &str,
    body_rename: &HashMap<String, String>,
    module_base: u32,
    enums: EnumRemap,
    struct_base: usize,
    array_base: usize,
    cell_base: usize,
    ref_base: usize,
    slice_base: usize,
) -> WordDef {
    let remap_slot = |s: &TypedSlot| TypedSlot {
        name: s.name.clone(),
        ty: remap_type(
            s.ty,
            enums,
            struct_base,
            array_base,
            cell_base,
            ref_base,
            slice_base,
        ),
    };
    let effect = StackEffect {
        inputs: w.effect.inputs.iter().map(remap_slot).collect(),
        outputs: w.effect.outputs.iter().map(remap_slot).collect(),
    };
    let poly = w.poly.as_ref().map(|sig| {
        let mut sig = (**sig).clone();
        sig.inputs = sig
            .inputs
            .iter()
            .map(|p| {
                remap_poly_type(
                    p,
                    enums,
                    struct_base,
                    array_base,
                    cell_base,
                    ref_base,
                    slice_base,
                )
            })
            .collect();
        sig.outputs = sig
            .outputs
            .iter()
            .map(|p| {
                remap_poly_type(
                    p,
                    enums,
                    struct_base,
                    array_base,
                    cell_base,
                    ref_base,
                    slice_base,
                )
            })
            .collect();
        Box::new(sig)
    });
    let terms = &w.body;
    WordDef {
        name: internal.to_string(),
        effect,
        body: rewrite_combinator_body_calls(terms, body_rename),
        poly,
        declares_inline: w.declares_inline,
        module: module_base + w.module,
        span: w.span,
        declared_globals: w.declared_globals.clone(),
    }
}

/// True if `name` ends in `__m` or `__import` followed by one or more ascii
/// digits -- the resolver's cross-module mangle (`resolve::mangle`,
/// `{raw}__m{module}`) and the import epoch tag (`import_symbol`,
/// `{raw}__import{epoch}`), respectively. A multi-file closure's non-module-0
/// words are called, from a retained combinator's body, by exactly this
/// mangled spelling -- it is never rewritten to a `{q}::...__import{epoch}`
/// alias, since the existing body-call rewrite only covers module-0 words --
/// so a REPL-declared word whose bare name happens to equal it would silently
/// hijack that body call instead of the closure's own definition.
fn ends_with_mangled_digit_suffix(name: &str) -> bool {
    for marker in ["__import", "__m"] {
        if let Some(idx) = name.rfind(marker) {
            let digits = &name[idx + marker.len()..];
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

/// R8e (slice 5b) + review fix (slice 6c): a REPL-declared name may not
/// contain `::`, the separator reserved for a qualified imported spelling,
/// nor end in a resolver-mangled or import-epoch-tagged spelling (see
/// `ends_with_mangled_digit_suffix`); either way a user could forge an
/// internal spelling and hijack a closure's own resolution. A new, REPL-only
/// guard (native `.sth` declarations have the same latent gap but no tag to
/// collide with), located, naming the offending spelling.
fn reject_double_colon_name(kind: &str, name: &str, span: Span) -> Result<(), String> {
    if name.contains("::") {
        return Err(format!(
            "error: a REPL-declared {kind} name may not contain `::` (`{name}` at line {}, col {})",
            span.line, span.col
        ));
    }
    if ends_with_mangled_digit_suffix(name) {
        return Err(format!(
            "error: a REPL-declared {kind} name may not end in a mangled `__m<digits>` or `__import<digits>` spelling (`{name}` at line {}, col {})",
            span.line, span.col
        ));
    }
    Ok(())
}

/// P7.S3t (R10): an explicit type instantiation (`f[Point]`) anywhere in a
/// REPL line, rejected outright and located. A session routes through
/// `lower_instantiation` and skips the module-level checks the syntax's
/// correctness argument rests on, so this is a guard rather than a deferred
/// feature: it fails closed instead of printing success and binding whichever
/// specialization the session happened to find.
fn reject_explicit_instantiation(terms: &[Term]) -> Result<(), String> {
    for term in terms {
        match &term.kind {
            TermKind::Call(_, type_args) if !type_args.is_empty() => {
                return Err(format!(
                    "error: explicit type instantiation is not available at the REPL (line {}, col {})\n  note: `f[Point]` needs a whole-program impl registry a live session does not assemble",
                    term.span.line, term.span.col
                ));
            }
            TermKind::Quotation(inner, _, _) => reject_explicit_instantiation(inner)?,
            _ => {}
        }
    }
    Ok(())
}

/// R12 (slice 5b, phase 3): a selectively-exposed name colliding with a
/// session-local definition, naming the source qualifier and the local name --
/// the session-scope analogue of `check::selective_collides_with_local_error`.
fn session_selective_collides_with_local_error(name: &str, qualifier: &str, span: Span) -> String {
    format!(
        "error: selective import of `{name}` from module `{qualifier}` (line {}, col {}) collides with a local definition of `{name}`",
        span.line, span.col
    )
}

/// R12 (slice 5b, phase 3): a selectively-exposed name colliding with an
/// earlier selective import's unqualified name, naming both source
/// qualifiers -- the session-scope analogue of `check::selective_collision_error`.
fn session_selective_collision_error(name: &str, first: &str, second: &str, span: Span) -> String {
    format!(
        "error: selective import of `{name}` from module `{second}` (line {}, col {}) collides with the selective import of `{name}` from module `{first}`",
        span.line, span.col
    )
}

/// R5/R6 (slice 5b): bulk-lower a whole checked import closure to one `.so`.
/// Reuses `ir::lower` (the native single-module lowerer), then renames every
/// user word's func and its intra-closure call sites to the word's import-epoch
/// symbol, so the exported symbols are session-fresh and intra-closure calls
/// still resolve within this one `.so`.
fn compile_import_closure(module: &Module, epoch: u64) -> Result<Library, String> {
    let mut ir = ir::lower(module)?;
    let rename: HashMap<String, String> = module
        .words
        .iter()
        .filter(|w| w.poly.is_none() && w.name != "main" && w.name != "drop")
        .map(|w| (w.name.clone(), import_symbol(&w.name, epoch)))
        .collect();
    for func in &mut ir.funcs {
        if let Some(s) = rename.get(&func.name) {
            func.name = s.clone();
        }
        for block in &mut func.blocks {
            for instr in &mut block.instrs {
                if let ir::Instr::Call(_, sym, _) = instr {
                    if let Some(s) = rename.get(sym) {
                        *sym = s.clone();
                    }
                }
            }
        }
    }
    let ssa = backend::qbe::emit(&ir)?;
    let dir = driver::tempfile_dir()?;
    let so_path = dir.join(format!("import_epoch{epoch}.so"));
    driver::compile_so(&ssa, &so_path)?;
    Library::open(&so_path)
}

/// The generation a new definition of `name` should take: 0 if never defined,
/// else one past the current entry's generation.
fn next_generation(existing: Option<&WordEntry>) -> u64 {
    existing.map(|e| e.generation + 1).unwrap_or(0)
}

/// A resolver over the current generations (no override), for compiling an
/// expression line.
fn resolver_for(env: &HashMap<String, WordEntry>) -> impl Fn(&str) -> String + '_ {
    move |name: &str| {
        env.get(name)
            .map(|e| e.symbol.clone())
            .unwrap_or_else(|| name.to_string())
    }
}

/// A call-name resolver over the current generations in `env`, with
/// `override_name` forced to `override_symbol` regardless of what `env` says
/// (so a definition's own recursive calls bind its new generation, not
/// whatever `env` still holds from the previous definition).
fn resolver_with_override<'a>(
    env: &'a HashMap<String, WordEntry>,
    override_name: &'a str,
    override_symbol: &'a str,
) -> impl Fn(&str) -> String + 'a {
    move |name: &str| {
        if name == override_name {
            override_symbol.to_string()
        } else {
            env.get(name)
                .map(|e| e.symbol.clone())
                .unwrap_or_else(|| name.to_string())
        }
    }
}

/// Format the carried stack, bottom to top, for the session's per-expression
/// output line. `buf` holds the live carried bytes as 8-byte `i64` cells;
/// each slot's cell offset is computed from the per-slot sizes (a scalar is
/// one cell, a struct spans `ceil(size/8)` cells), so a scalar slot past a
/// struct still reads the right cell.
///
/// A struct slot renders as its type-name placeholder `<TypeName>`, reading no
/// field bytes. A float slot is reinterpreted from its stored bits via
/// `from_bits` (R21): displaying its `i64` bit pattern would be meaningless. An
/// `f32` slot reads only the low 32 bits (it was stored 4-wide, Q2). A `bool`
/// slot displays as `true`/`false` (matching `.`, not the raw 0/1). An
/// unsigned slot (a `uN` or `usize`) displays as its unsigned value: the raw
/// `i64` bit pattern of a high-bit-set unsigned value is negative and would
/// otherwise misprint as such.
pub fn format_stack(
    buf: &[i64],
    types: &[Type],
    layouts: &[StructLayout],
    enum_layouts: &[EnumLayout],
    array_layouts: &[ArrayLayout],
    bool_enum: EnumId,
) -> String {
    if types.is_empty() {
        return "stack: (empty)".to_string();
    }
    let mut cell = 0usize;
    let mut vals = Vec::with_capacity(types.len());
    for ty in types {
        match ty {
            // P7 slice 3i (R2): a bool is structurally an enum, but `:stack`
            // renders it as `true`/`false` rather than the generic
            // `<TypeName>` placeholder below, matching `.`; this arm must
            // precede the general `Type::Enum` one to win. Keyed on the
            // session's own seeded `bool` (`Session::bool_enum`) rather than
            // on the type name, so an imported enum merely *named* `bool`
            // (which `EnumRemap` keeps as a type of its own) renders as the
            // placeholder it is instead of being read one cell wide. The
            // scalar-layout check states that one-cell read directly.
            Type::Enum(id, _) if *id == bool_enum && enum_layouts[id.index()].is_scalar => {
                let v = buf[cell];
                vals.push(if v != 0 { "True" } else { "False" }.to_string());
                cell += 1;
            }
            Type::Struct(id, name) => {
                vals.push(format!("<{name}>"));
                let size = layouts[id.index()].size as usize;
                cell += size.div_ceil(8);
            }
            Type::Enum(id, name) => {
                // An enum slot renders as its `<TypeName>` placeholder (M4),
                // reusing the struct-placeholder path, and advances the buffer
                // by its tagged aggregate's cell span (no tag/payload shown).
                vals.push(format!("<{name}>"));
                let size = enum_layouts[id.index()].size as usize;
                cell += size.div_ceil(8);
            }
            Type::Array(id, name) => {
                // An array slot renders as its `<[T N]>` placeholder,
                // reusing the aggregate-placeholder path, and advances the
                // buffer by its inline aggregate's cell span.
                vals.push(format!("<{name}>"));
                let size = array_layouts[id.index()].size as usize;
                cell += size.div_ceil(8);
            }
            Type::OwnedCell(_, name) => {
                // An address is nondeterministic, so print a placeholder.
                vals.push(format!("<{name}>"));
                cell += 1;
            }
            // A `str`/`cstr` slot is an opaque address like a cell slot, so it
            // gets the same placeholder treatment rather than dereferencing
            // its descriptor/bytes to print content.
            Type::Str => {
                vals.push("<str>".to_string());
                cell += 1;
            }
            Type::Cstr => {
                vals.push("<cstr>".to_string());
                cell += 1;
            }
            // P7 slice 3c (R7): a slice renders as its `<Slice[T]>` type
            // placeholder, like the aggregates above, and advances the buffer
            // by its two-slot cell span -- getting that span wrong would
            // misread every slot above it, which is why it is not folded into
            // the one-cell scalar arm below.
            Type::Slice(_, _, name) => {
                vals.push(format!("<{name}>"));
                cell += (ir::slice_layout(ir::WORD_WIDTH).size as usize).div_ceil(8);
            }
            _ => {
                let v = buf[cell];
                vals.push(match ty {
                    Type::Float(ft) if ft.bits() == 32 => {
                        f32::from_bits(v as u64 as u32).to_string()
                    }
                    Type::Float(_) => f64::from_bits(v as u64).to_string(),
                    Type::Int(it) if !it.signed() => (v as u64).to_string(),
                    Type::Usize => (v as u64).to_string(),
                    Type::Isize => v.to_string(),
                    _ => v.to_string(),
                });
                cell += 1;
            }
        }
    }
    format!("stack: {}", vals.join(" "))
}

/// Reinterpret the carried-stack cells as bytes (R13/R14). Sound because `i64`
/// has no padding and every bit pattern is valid; this is the same native
/// memory `run_terms`'s wrapper already read/wrote through a raw pointer into
/// this allocation, so no copy or conversion changes what is observed.
fn as_bytes(buf: &[i64]) -> &[u8] {
    // SAFETY: `buf` is a valid, initialized `&[i64]`; every `i64` bit pattern
    // reads back as 8 valid bytes, and the resulting slice's lifetime is tied
    // to `buf`'s.
    unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, std::mem::size_of_val(buf)) }
}

/// Read up to 8 little-endian bytes of `region` into a `u64`, zero-padding any
/// width narrower than 8 bytes. The building block both scalar rendering and
/// enum-tag decoding read through.
fn read_uint_le(region: &[u8], width: usize) -> u64 {
    let mut tmp = [0u8; 8];
    tmp[..width].copy_from_slice(&region[..width]);
    u64::from_le_bytes(tmp)
}

/// The byte extent one `IrType`'s value occupies: a scalar's own width (not
/// rounded to a cell, unlike `ir::carried_slot_bytes`), or an aggregate's
/// `size` from its layout. Used to slice out exactly the bytes a nested field,
/// array element, or owning-cell payload spans for recursive rendering.
fn rich_value_size(
    ty: ir::IrType,
    layouts: &[StructLayout],
    enum_layouts: &[EnumLayout],
    array_layouts: &[ArrayLayout],
) -> usize {
    match ty {
        ir::IrType::Bool => 1,
        ir::IrType::Int { bits, .. } => (bits / 8) as usize,
        ir::IrType::Float { bits } => (bits / 8) as usize,
        ir::IrType::Usize | ir::IrType::Isize => ir::WORD_WIDTH as usize,
        ir::IrType::Ptr | ir::IrType::OwnedCell(_) | ir::IrType::Str | ir::IrType::Cstr => 8,
        // Slice 7a: a code handle is one word; a quotation value spans its
        // fixed three-slot layout (only reached as a struct/enum/array field).
        ir::IrType::Code => ir::WORD_WIDTH as usize,
        ir::IrType::Quotation(_) | ir::IrType::OwningQuotation(_) => {
            ir::quotation_layout(ir::WORD_WIDTH).size as usize
        }
        // P7 slice 3c (R2.2): a slice spans its whole two-slot layout, not one
        // word -- the `Str` answer above would under-read by the length slot.
        ir::IrType::Slice(_) => ir::slice_layout(ir::WORD_WIDTH).size as usize,
        ir::IrType::Struct(id) => layouts[id.index()].size as usize,
        ir::IrType::Enum(id) => enum_layouts[id.index()].size as usize,
        ir::IrType::Array(id) => array_layouts[id.index()].size as usize,
    }
}

/// R14/R15/R16 (Slice 3): render one value's bytes with its type recoverable
/// from the rendering. A struct/enum/array descends into its fields/variant/
/// elements via the same offset arithmetic `format_stack` uses only to *skip*
/// an aggregate slot; an owning cell (R16) dereferences its live heap payload
/// to render it, which is a read (no bookkeeping here ever marks a value
/// consumed or runs a destructor, so linearity is untouched). An
/// otherwise-ambiguous scalar carries a Rust-literal-style width/signedness
/// suffix (`1u8` vs `1i64`, R15).
fn render_rich_value(
    ty: ir::IrType,
    region: &[u8],
    layouts: &[StructLayout],
    enum_layouts: &[EnumLayout],
    array_layouts: &[ArrayLayout],
    cell_payloads: &[ir::IrType],
) -> String {
    match ty {
        ir::IrType::Bool => if region[0] != 0 { "True" } else { "False" }.to_string(),
        ir::IrType::Int { bits, signed } => {
            let raw = read_uint_le(region, (bits / 8) as usize);
            let kind = if signed { 'i' } else { 'u' };
            if signed {
                let shift = 64 - bits as u32;
                let v = ((raw << shift) as i64) >> shift;
                format!("{v}{kind}{bits}")
            } else {
                format!("{raw}{kind}{bits}")
            }
        }
        ir::IrType::Float { bits: 32 } => {
            let bytes: [u8; 4] = region[..4].try_into().unwrap();
            format!("{}f32", f32::from_le_bytes(bytes))
        }
        ir::IrType::Float { .. } => {
            let bytes: [u8; 8] = region[..8].try_into().unwrap();
            format!("{}f64", f64::from_le_bytes(bytes))
        }
        ir::IrType::Usize => format!("{}usize", read_uint_le(region, 8)),
        ir::IrType::Isize => format!("{}isize", read_uint_le(region, 8) as i64),
        ir::IrType::Ptr => "<ptr>".to_string(),
        ir::IrType::Str => "<str>".to_string(),
        ir::IrType::Cstr => "<cstr>".to_string(),
        // Slice 7a: a code handle / quotation value carries no printable
        // payload; reached only as a struct/enum/array field (a bare
        // quotation on the residual is rejected before rendering).
        ir::IrType::Code => "<code>".to_string(),
        ir::IrType::Quotation(_) | ir::IrType::OwningQuotation(_) => "<quotation>".to_string(),
        // P7 slice 3c (R2.2/R7): a placeholder, matching the checker's ruling
        // that a slice is not printable (`.`'s allowlist excludes it): showing
        // the elements would need an element loop and a separator policy this
        // renderer has no business choosing. A rendering path never panics, so
        // this is a placeholder rather than an `unreachable!`, even though the
        // residual-stack reference ban keeps a slice out of a rendered value.
        ir::IrType::Slice(_) => "<slice>".to_string(),
        ir::IrType::Struct(id) => {
            let layout = &layouts[id.index()];
            let fields: Vec<String> = layout
                .fields
                .iter()
                .map(|f| {
                    let off = f.offset as usize;
                    let span = rich_value_size(f.ty, layouts, enum_layouts, array_layouts);
                    render_rich_value(
                        f.ty,
                        &region[off..off + span],
                        layouts,
                        enum_layouts,
                        array_layouts,
                        cell_payloads,
                    )
                })
                .collect();
            format!("<{} {}>", layout.name, fields.join(" "))
        }
        ir::IrType::Enum(id) => {
            let layout = &enum_layouts[id.index()];
            let tag_width = match layout.tag_ty {
                ir::IrType::Int { bits, .. } => (bits / 8) as usize,
                _ => unreachable!("an enum tag is always a fixed-width integer"),
            };
            let tag = read_uint_le(&region[layout.tag_offset as usize..], tag_width) as usize;
            let variant = &layout.variants[tag];
            let payload: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let off = layout.payload_offset as usize + f.offset as usize;
                    let span = rich_value_size(f.ty, layouts, enum_layouts, array_layouts);
                    render_rich_value(
                        f.ty,
                        &region[off..off + span],
                        layouts,
                        enum_layouts,
                        array_layouts,
                        cell_payloads,
                    )
                })
                .collect();
            if payload.is_empty() {
                format!("<{}#{}>", layout.name, tag)
            } else {
                format!("<{}#{} {}>", layout.name, tag, payload.join(" "))
            }
        }
        ir::IrType::Array(id) => {
            let layout = &array_layouts[id.index()];
            let elem_size = rich_value_size(layout.elem, layouts, enum_layouts, array_layouts);
            let elems: Vec<String> = (0..layout.count as usize)
                .map(|i| {
                    let off = i * layout.stride as usize;
                    render_rich_value(
                        layout.elem,
                        &region[off..off + elem_size],
                        layouts,
                        enum_layouts,
                        array_layouts,
                        cell_payloads,
                    )
                })
                .collect();
            format!("<{} {}>", layout.name, elems.join(" "))
        }
        ir::IrType::OwnedCell(id) => {
            let ptr = read_uint_le(region, 8) as usize as *const u8;
            let payload_ty = cell_payloads[id.index()];
            let payload_size = rich_value_size(payload_ty, layouts, enum_layouts, array_layouts);
            // SAFETY (R16, load-bearing): a live owning cell on the residual
            // stack always points at a `malloc`ed payload of exactly
            // `payload_size` bytes (the shape `^`'s construction allocates,
            // `Cells::payload[id]`); reading it here for display touches no
            // linearity bookkeeping (that lives in `self.types`/the checker,
            // never consulted by this function) and runs no destructor, so
            // the value stays live and the carried stack is unchanged.
            let payload_bytes = unsafe { std::slice::from_raw_parts(ptr, payload_size) };
            format!(
                "^{}",
                render_rich_value(
                    payload_ty,
                    payload_bytes,
                    layouts,
                    enum_layouts,
                    array_layouts,
                    cell_payloads
                )
            )
        }
    }
}

/// D2/R13 (Slice 3): the tty-only rich stack formatter. Signature mirrors
/// `format_stack`, plus the fourth layout table (`cell_payloads`, an owning
/// cell's payload `IrType` per `OwnedCellId`) R16's read-through needs. The
/// piped path never calls this (F2); `Session::rich_stack` gates the choice at
/// the one shared call site (`eval_expr`).
pub fn format_stack_rich(
    buf: &[i64],
    types: &[Type],
    layouts: &[StructLayout],
    enum_layouts: &[EnumLayout],
    array_layouts: &[ArrayLayout],
    cell_payloads: &[ir::IrType],
) -> String {
    if types.is_empty() {
        return "stack: (empty)".to_string();
    }
    let bytes = as_bytes(buf);
    let mut cell = 0usize;
    let mut vals = Vec::with_capacity(types.len());
    for ty in types {
        let ir_ty = ir::ir_type_of(*ty);
        let span = rich_value_size(ir_ty, layouts, enum_layouts, array_layouts);
        let off = cell * 8;
        vals.push(render_rich_value(
            ir_ty,
            &bytes[off..off + span],
            layouts,
            enum_layouts,
            array_layouts,
            cell_payloads,
        ));
        cell += span.div_ceil(8);
    }
    format!("stack: {}", vals.join(" "))
}

/// A REPL session: the accumulated word env, the persistent stack buffer, and
/// every loaded shared object (kept resident for the session's lifetime).
pub struct Session {
    env: HashMap<String, WordEntry>,
    /// The struct registry, one entry per `type:` line, in declaration order
    /// so `StructId` = index stays stable (a carried `Type::Struct` keeps
    /// referring to the same struct across lines). Field types resolve against
    /// earlier entries plus the entry being declared.
    structs: Vec<StructDecl>,
    /// The enum registry, parallel to `structs`, one entry per enum `type:`
    /// line in declaration order so `EnumId` = index stays stable across
    /// lines. Variant field types resolve against the struct and enum
    /// registries.
    enums: Vec<EnumDecl>,
    /// The interned array-type registry, in interning order so `ArrayId` =
    /// index stays stable across lines. Grows as array type expressions and
    /// `fill` shapes resolve; shared by the checker and the layout builder.
    arrays: Vec<ArrayDecl>,
    /// The interned owning-cell registry, mirroring `arrays`: grows as `^T`
    /// type expressions resolve, persisting across lines in the same session.
    owned_cells: Vec<OwnedCellDecl>,
    /// The interned reference registry, mirroring `owned_cells`: grows as
    /// `&T`/`&!T` type expressions resolve, persisting across lines. A
    /// reference can never *survive* a line, but a word defined at the
    /// REPL may take one as an input.
    refs: Vec<RefDecl>,
    /// P7 slice 3c: the interned slice registry, mirroring `refs`: a
    /// `(element, mutable)` view shape per entry, persisting across lines so a
    /// `SliceId` keeps naming the same shape. Grown only by an import splice
    /// today; no session line can mint one until a slice has a surface
    /// spelling.
    slices: Vec<SliceDecl>,
    /// R11: every `drop` overload the session has seen, keyed the way
    /// destructor synthesis is keyed (by `StructId`, never by the shared
    /// literal name), holding the `override_epoch` it was last defined at and
    /// its body. The body is retained because a later line's re-synthesis
    /// still has to know the struct *has* an override (so it emits no glue
    /// under the pinned symbol), and because the defining line itself lowers
    /// it after the session has already been updated. The epoch lives here
    /// rather than in `self.env` because the override is deliberately absent
    /// from `env` (R1), so `next_generation`'s lookup could never see it.
    drop_overloads: HashMap<StructId, (u64, WordDef)>,
    /// Blocker 1 (post-implementation review): the same-struct-id-keyed
    /// `drop` call sites `check_def_collecting_drop_sites` recorded while
    /// checking each override in `drop_overloads`, cached at the line that
    /// defined it rather than re-derived later. A whole-session reachability
    /// query (`check_drop_overload_reachability`, run on every `: drop` line)
    /// needs every override's sites to catch a cycle closing through more
    /// than one struct, but re-checking an *earlier* override's body against
    /// a *later* line's env would reintroduce the stale-env hazard R11.2/
    /// R11.3 already fixed for lowering -- a recorded site's operand type
    /// never changes once observed, so caching it is exact, not stale.
    drop_dropped_sites: HashMap<StructId, Vec<Type>>,
    /// R4 (Slice 2): every polymorphic word the session has defined, retained
    /// out of `self.env` (a polymorphic word never enters the concrete env,
    /// R3, so a concrete call-site lookup and `next_generation` never see it),
    /// exactly as `drop_overloads` are. Holds the body, the frozen defining-
    /// line resolver snapshot, and the generation each was retained at, so a
    /// later line can instantiate it (R5/R7).
    poly_words: HashMap<String, PolyWordEntry>,
    /// R1 (Slice 6c): every quotation-taking word (combinator) the session has
    /// retained, mono and poly in one store (D2). The key is the name the
    /// checker dispatches on (a plain word name for a session-defined
    /// combinator; the import-internal spelling for an imported one, R13). A
    /// combinator mints no `IrFunc` and no symbol (R20/D1): its body is
    /// re-spliced, fresh, at every later call site under that site's own live
    /// env, so this holds the raw `WordDef` alone -- no generation, epoch, or
    /// symbol -- and a redefinition replaces the entry wholesale.
    combinators: HashMap<String, WordDef>,
    /// R7 (Slice 2, D2): the mangled symbols of every polymorphic instantiation
    /// already lowered with external linkage into some line's module. The
    /// symbol encodes `(name, generation, subst)`, so it *is* the dedup key:
    /// an instantiation whose symbol is already here emits nothing and binds,
    /// under `RTLD_GLOBAL`, to the earlier line's export, bounding `.so`
    /// growth across repeated same-type instantiations (trace B).
    exported_insts: HashSet<String>,
    /// R11.2: the session-wide override epoch, `None` until this session's
    /// first `drop` override is ever defined, then incremented by one on
    /// every subsequent override define/redefine event (of any struct, not
    /// only the same one). Stamped onto *every* linear struct's/enum's/
    /// cell's destructor symbol once `Some` (`apply_drop_generations`): a
    /// struct without its own override can still `Call`, inside its own
    /// destructor, one that composes an overridden struct, so its body
    /// changes across an override event too. One counter serves both jobs:
    /// an overridden struct's own symbol is stamped with the epoch its
    /// override was defined at (pinning it, R11.3), everything else with the
    /// session's current one.
    override_epoch: Option<u64>,
    /// The carried stack, as 8-byte `i64` cells. `top` is the live byte
    /// length; a slot may span more than one cell (a struct or enum), so the
    /// buffer is byte-addressable and slot offsets are computed from `types`,
    /// never `index * 8`.
    buf: Vec<i64>,
    top: usize,
    /// The `Type` of each carried slot, in stack order (deepest first). Slot
    /// byte sizes vary (a struct spans its aggregate size), so
    /// `types.len() != top / 8` in general.
    types: Vec<Type>,
    libs: Vec<Library>,
    seq: u64,
    /// Slice 5b (R6): the next import event's epoch, incremented once per
    /// successful `import:` line. Tags every spliced symbol and internal type
    /// name so a re-run `import:` (a redefinition of a whole batch of names)
    /// mints session-fresh spellings that never collide with a prior event's,
    /// leaving frozen callers bound to their own generation.
    import_epoch: u64,
    /// Slice 5b (R9): the next free session module id for an import event. An
    /// event's closure of N files reserves N consecutive ids (entry = base),
    /// and the counter only ever advances, so a rebind's ids never reuse a
    /// prior event's. Session-local decls live in module 0; import ids start
    /// at 1.
    next_import_module: u32,
    /// Slice 5b (R8c): every qualifier-bound user-facing spelling (`q::w`,
    /// `q::T`) mapped to its *current* internal (epoch-tagged) name. The
    /// body-position rewrite pass consults this before ordinary checking.
    import_aliases: HashMap<String, String>,
    /// Slice 5b (R15): per qualifier, the module-0 names that exist but are
    /// not `export:`ed (bare word names and every private type's accessor
    /// spellings), so a `q::x` that misses `import_aliases` can be told apart
    /// as `not exported` rather than unknown.
    import_private: HashMap<String, HashSet<String>>,
    /// Slice 5b (R8a/R8d): each bound qualifier mapped to its entry module's
    /// session id, the `imports` map the parser's type-position resolver reads.
    import_qualifier_module: HashMap<String, u32>,
    /// Slice 5b (R11, phase 3): each selectively-imported bare type name mapped
    /// to its target module id, the parser's `selective` map. Empty until
    /// selective import lands.
    import_selective_module: HashMap<String, u32>,
    /// Slice 5b (R8d): per session module id, that module's `export:` list,
    /// the parser's `exports` slice for gating a `q::T` type reference. Index 0
    /// (session-local decls) stays empty; import ids fill in from 1.
    import_exports: Vec<Vec<(String, Span)>>,
    /// D2 (Slice 3): whether the residual stack renders through the rich
    /// aggregate-contents formatter (`format_stack_rich`) instead of the
    /// plain placeholder one. `false` for every piped session (F2 keeps the
    /// existing goldens byte-for-byte); `run_tty` sets this once, at entry,
    /// for an interactive session.
    rich_stack: bool,
    /// P7 slice 3i (R2/R4): where `Session::new`'s `core::bool` seed landed in
    /// `enums`. The session's *one* boolean type: an imported module's own
    /// `bool` maps here (`remap_type`) rather than being appended a second
    /// time, and `:stack` renders a slot of this type as `true`/`false`.
    bool_enum: EnumId,
}

/// P7 slice 3i (R2): `core::bool` parsed from the real `lib/bool.sth`, the
/// session's startup seed. Embedded at build time rather than read from disk:
/// a session must resolve its boolean type without knowing where the library
/// tree sits relative to the running binary, and this keeps `lib/bool.sth` the
/// single source of the declaration all the same.
fn core_bool_module() -> Module {
    let tokens = lexer::lex(include_str!("../lib/bool.sth")).expect("`lib/bool.sth` lexes");
    parser::parse(&tokens).expect("`lib/bool.sth` parses")
}

impl Session {
    pub fn new() -> Session {
        // P7 slice 3i (R2): the session seeds `core::bool` from the real
        // `lib/bool.sth` source, embedded rather than read from a path so a
        // session does not depend on where the binary was installed relative
        // to the library tree. This is the REPL's equivalent of a file's
        // `import: core::bool ;`, and it is a seed rather than a written import
        // because the REPL cannot resolve a package-name import at all: without
        // it a bare `true` would be unusable in a session.
        //
        // Only the type and its `.` overload are seeded. `if`/`unless` are not:
        // a session imports those exactly as a file does (P8 S2 R3).
        let core_bool = core_bool_module();
        let bool_enum = EnumId::from_index(
            crate::ast::resolve_bool_type(&core_bool.enums)
                .and_then(|ty| match ty {
                    Type::Enum(id, _) => Some(id.index()),
                    _ => None,
                })
                .expect("`lib/bool.sth` declares the `Bool` enum"),
        );
        let mut session = Session {
            env: HashMap::new(),
            structs: Vec::new(),
            // The whole registry, order preserved, so every id in the seeded
            // word's signature and body still indexes the entry it was parsed
            // against -- no remap.
            enums: core_bool.enums,
            arrays: Vec::new(),
            owned_cells: Vec::new(),
            refs: Vec::new(),
            slices: Vec::new(),
            drop_overloads: HashMap::new(),
            drop_dropped_sites: HashMap::new(),
            poly_words: HashMap::new(),
            combinators: HashMap::new(),
            exported_insts: HashSet::new(),
            override_epoch: None,
            buf: Vec::new(),
            top: 0,
            types: Vec::new(),
            libs: Vec::new(),
            seq: 0,
            import_epoch: 0,
            next_import_module: 1,
            import_aliases: HashMap::new(),
            import_private: HashMap::new(),
            import_qualifier_module: HashMap::new(),
            import_selective_module: HashMap::new(),
            import_exports: Vec::new(),
            rich_stack: false,
            bool_enum,
        };
        // The `.` overload goes in through the ordinary `eval_def` path exactly
        // as a user definition would, so it dlopen's like any other REPL word
        // and resolves through the same `builtin_overloads` dispatch a compiled
        // module's imported copy uses.
        let print = core_bool
            .words
            .into_iter()
            .find(|w| w.name == "." && w.poly.is_none())
            .expect("`lib/bool.sth` declares the Bool `.` overload");
        session
            .eval_def(print, &mut std::io::sink())
            .expect("`core::bool`'s `.` overload always checks and compiles");
        // P8 S2 (R3): nothing else is seeded. `if`, `unless` and the six
        // comparisons used to be injected here from the compiler-baked
        // prelude; they are ordinary `core` words now, so a session that wants
        // them writes `import: core::prelude * ;` exactly as a file does, and a
        // bare comparison with no import is `unknown word` -- which is what a
        // compiled build does too.
        session
    }

    /// D2 (Slice 3): switch this session's residual-stack rendering to the
    /// rich, aggregate-contents formatter. Called once by `run_tty`; never by
    /// the piped path.
    pub fn enable_rich_stack_rendering(&mut self) {
        self.rich_stack = true;
    }

    /// R19: the user-facing spelling for an `self.env` key. A session-local
    /// word's key already *is* its user-facing name, but an imported word's
    /// key is its internal, import-epoch-mangled spelling (`splice_import`
    /// inserts `q::raw__importN` as the env key and `q::raw` -> that spelling
    /// into `import_aliases`, R9); this reverses that map back to what the
    /// user actually typed. A selective import adds a second, unqualified
    /// alias at the same internal spelling, so ties prefer the qualified one
    /// (deterministic regardless of `HashMap` iteration order, not chosen by
    /// which alias happened to be inserted last).
    fn display_name(&self, internal: &str) -> String {
        let mut best: Option<&str> = None;
        for (alias, target) in &self.import_aliases {
            if target == internal && (best.is_none() || alias.contains("::")) {
                best = Some(alias);
            }
        }
        best.map(str::to_string)
            .unwrap_or_else(|| internal.to_string())
    }

    /// R19/R23: every defined word's user-facing name, sorted, at its current
    /// generation, including polymorphic words (`self.poly_words`, kept out
    /// of `self.env` per R3 so it needs folding in separately -- imports
    /// never splice a polymorphic word, so `poly_words` keys are always
    /// already user-facing and need no `display_name` reversal). Shared
    /// between `:words` (D3) and the editor's tab completion (R23), which is
    /// the point of pulling it out on its own.
    pub fn word_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.env.keys().map(|k| self.display_name(k)).collect();
        names.extend(self.poly_words.keys().cloned());
        names.sort();
        names
    }

    /// R19: `:words` listing, one `name ( ins -- outs )` line per defined
    /// word (concrete and polymorphic), sorted for deterministic golden
    /// output. Built directly from `self.env`/`self.poly_words` rather than
    /// through `word_names()` so each display name stays paired with the
    /// right signature lookup (an env key, not the possibly-aliased display
    /// name).
    fn words_listing(&self) -> Vec<String> {
        let mut entries: Vec<(String, String)> = self
            .env
            .iter()
            .map(|(internal, entry)| (self.display_name(internal), sig_str(&entry.sig)))
            .collect();
        entries.extend(self.poly_words.iter().map(|(name, entry)| {
            let sig = entry
                .word
                .poly
                .as_deref()
                .expect("a poly_words entry always has a polymorphic signature");
            (name.clone(), poly_sig_str(sig))
        }));
        entries.sort();
        entries
            .into_iter()
            .map(|(name, sig)| format!("{name} {sig}"))
            .collect()
    }

    /// R21: render the residual stack through whichever formatter the path
    /// uses (D2), without pushing or consuming anything. Layout registries
    /// are rebuilt (pure, no compilation) purely to size/skip aggregate slots.
    fn render_stack(&self) -> String {
        let (structs, enums, arrays, cells, _refs) = ir::build_registries(
            &self.structs,
            &self.enums,
            &self.arrays,
            &self.owned_cells,
            &self.refs,
        );
        let live_cells = self.top / 8;
        if self.rich_stack {
            format_stack_rich(
                &self.buf[..live_cells],
                &self.types,
                &structs.layouts,
                &enums.layouts,
                &arrays.layouts,
                &cells.payload,
            )
        } else {
            format_stack(
                &self.buf[..live_cells],
                &self.types,
                &structs.layouts,
                &enums.layouts,
                &arrays.layouts,
                self.bool_enum,
            )
        }
    }

    /// D4/R22: dispose the residual stack's linear values (the existing
    /// `dispose_residual` path, as `end_session` runs) before resetting env,
    /// stack, and every registry/generation counter to a fresh session --
    /// reset is scope-end, not a silent forget of live linear values.
    fn clear(&mut self, writer: &mut impl Write) -> Result<(), String> {
        self.dispose_residual(writer)?;
        let rich_stack = self.rich_stack;
        *self = Session::new();
        self.rich_stack = rich_stack;
        Ok(())
    }

    /// R20: lex -> parse -> check the rest of a `:type` line against the
    /// current stack types and print the resulting effect, executing and
    /// mutating nothing (no lower/emit/dlopen, no env/stack change).
    ///
    /// *(hazard)* `parse_line_with_structs` interns array/owned-cell/ref types
    /// into `self.arrays`/`owned_cells`/`refs` as a side effect of parsing; a
    /// `:type` that mentions one of those types must not grow the session's
    /// registries, so their lengths are snapshotted and restored regardless
    /// of whether checking succeeds.
    fn eval_type(&mut self, rest: &str, writer: &mut impl Write) -> Result<(), String> {
        let arrays_len = self.arrays.len();
        let cells_len = self.owned_cells.len();
        let refs_len = self.refs.len();
        let result = self.check_type_line(rest);
        self.arrays.truncate(arrays_len);
        self.owned_cells.truncate(cells_len);
        self.refs.truncate(refs_len);
        match result {
            Ok(effect) => writeln!(writer, "{effect}").map_err(|e| format!("writing stdout: {e}")),
            Err(e) => Err(e),
        }
    }

    fn check_type_line(&mut self, rest: &str) -> Result<String, String> {
        let tokens = lexer::lex(rest)?;
        let ctx = parser::ImportCtx {
            imports: &self.import_qualifier_module,
            selective: &self.import_selective_module,
            exports: &self.import_exports,
        };
        let line = parser::parse_line_with_structs(
            &tokens,
            &self.structs,
            &self.enums,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &mut self.slices,
            ctx,
        )?;
        let terms = match line {
            Line::Expr(terms) => terms,
            Line::Def(_) => {
                return Err(
                    "error: `:type` checks an expression against the current stack, not a word/type definition"
                        .to_string(),
                );
            }
        };
        let env = self.typed_env();
        let poly_env = self.poly_env();
        // R4 (Slice 6c): a `:type` line may name a retained combinator, so its
        // inference sees the session's inline view like any bare line.
        let combinators = checker_combinators(&self.combinators);
        let (net_stack, _insts, _overloads, _fields, _variant_fields) = check::infer_line(
            &terms,
            &self.types,
            &env,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &mut self.slices,
            &self.structs,
            &self.enums,
            &poly_env,
            &combinators,
        )?;
        Ok(type_effect_str(&self.types, &net_stack))
    }

    /// The checker's typed env: builtins, the generated struct words, the
    /// variant-constructor words, plus every successfully-defined user word.
    /// Slice 8a fix 1: every entry is a single-candidate overload set (the
    /// REPL's redefinition model keeps exactly one live binding per name, so
    /// unlike a native module's `env` this one never grows a second
    /// candidate); each candidate's `symbol` is its own bare name, matching
    /// the bare-name keys `ir_arity_env`/the REPL's own `resolve` closures
    /// already use, so a resolved overload record threaded into lowering
    /// (item 3) needs no extra translation step.
    fn typed_env(&self) -> HashMap<String, Vec<check::Overload>> {
        // Builtins are table-resolved in the checker, not held in the env.
        let mut env: HashMap<String, Vec<check::Overload>> = HashMap::new();
        for (name, symbol, sig) in check::struct_generated_sigs(&self.structs) {
            env.insert(name, vec![check::Overload { sig, symbol }]);
        }
        for (name, symbol, sig) in check::enum_generated_sigs(&self.enums) {
            env.insert(name, vec![check::Overload { sig, symbol }]);
        }
        for (name, symbol, sig) in check::variant_generated_sigs(&self.enums) {
            env.insert(name, vec![check::Overload { sig, symbol }]);
        }
        for (name, entry) in &self.env {
            let symbol = name.clone();
            env.insert(
                name.clone(),
                vec![check::Overload {
                    sig: entry.sig.clone(),
                    symbol,
                }],
            );
        }
        env
    }

    /// R5 (Slice 2): the session poly-env threaded into every REPL check path,
    /// mapping each retained polymorphic word to its `PolySig` and the
    /// generation it was retained at (so `check_poly_call` mints the
    /// generation-stamped symbol, R2/R2b). Kept out of `typed_env` because a
    /// polymorphic word never enters the concrete env (R3).
    fn poly_env(&self) -> check::PolyEnv {
        self.poly_words
            .iter()
            .map(|(name, entry)| {
                let sig = entry
                    .word
                    .poly
                    .as_deref()
                    .expect("a poly_words entry always has a polymorphic signature")
                    .clone();
                (name.clone(), vec![(sig, Some(entry.generation))])
            })
            .collect()
    }

    /// R8 (Slice 2): the generation a new definition of `name` should take,
    /// one past whichever of the ordinary env or the poly store currently
    /// holds it (a shared per-name counter, so a mono<->poly redefinition can
    /// never mint a colliding generation).
    fn next_shared_generation(&self, name: &str) -> u64 {
        let ordinary = next_generation(self.env.get(name));
        let polymorphic = self.poly_words.get(name).map_or(0, |e| e.generation + 1);
        ordinary.max(polymorphic)
    }

    /// R7 (Slice 2): the REPL analogue of native lowering's `poly_arities`
    /// (`name -> input arity`), so a call site to a retained polymorphic word
    /// resolves through `lower_poly_call` rather than the name-keyed env.
    fn poly_arities(&self) -> HashMap<String, usize> {
        self.poly_words
            .iter()
            .map(|(name, entry)| {
                let arity = entry
                    .word
                    .poly
                    .as_deref()
                    .expect("a poly_words entry always has a polymorphic signature")
                    .inputs
                    .len();
                (name.clone(), arity)
            })
            .collect()
    }

    /// R7 (Slice 2, D2): lower every not-yet-exported instantiation recorded
    /// while checking one compile unit (a bare line or a defined word body)
    /// into the compiling module, deduped against `exported_insts`. Each
    /// monomorphized `IrFunc` is lowered against the retained polymorphic
    /// word's frozen defining-line resolver snapshot (R4/D3), not the
    /// instantiating line's env, and emitted with external linkage so a later
    /// line resolves it under `RTLD_GLOBAL`. An already-exported symbol emits
    /// nothing (bounds `.so` growth, trace B). Sorted by symbol so the emitted
    /// order is deterministic across the `HashMap`'s randomized iteration.
    fn emit_instantiations(
        &mut self,
        insts: &HashMap<Span, CallInst>,
        regs: ir::Registries,
    ) -> Vec<ir::IrFunc> {
        let mut pending: Vec<&CallInst> = insts
            .values()
            .filter(|inst| !self.exported_insts.contains(&inst.symbol))
            .collect();
        pending.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        // R5 (Slice 6c): an instantiation body may call a retained combinator,
        // so it lowers against the session's combinator-bodies view like any
        // other REPL lowering entry point.
        let bodies = combinator_bodies(&self.combinators);
        // P7 slice 3a: a session-retained poly word's signature can never
        // carry a `PolyType::Generic` (the REPL has no way to declare a
        // generic `type:`, D2), so an empty, never-touched instantiator
        // suffices here.
        let empty_generics = crate::ast::GenericTypes::default();
        let mut funcs = Vec::new();
        let mut newly: Vec<String> = Vec::new();
        for inst in pending {
            // Two call sites in one unit can share a symbol (same word, same
            // θ, same generation): emit it once.
            if newly.contains(&inst.symbol) {
                continue;
            }
            let entry = &self.poly_words[&inst.callee];
            let sig = entry
                .word
                .poly
                .as_deref()
                .expect("a recorded callee is a retained polymorphic word");
            let resolve = |name: &str| {
                entry
                    .resolver
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.to_string())
            };
            funcs.extend(ir::lower_instantiation(
                &inst.symbol,
                &inst.callee,
                sig,
                &entry.builtin_overloads,
                // A generic body rejects a field projection outright
                // (`poly_reference_word`), so a retained polymorphic word
                // records none.
                ir::empty_resolved_fields(),
                &inst.subst,
                &entry.word.body,
                check::has_self_tail_call(&entry.word, &bodies),
                &entry.ir_lower_env,
                &resolve,
                regs,
                &self.arrays,
                &self.owned_cells,
                &self.refs,
                &empty_generics,
                &bodies,
            ));
            newly.push(inst.symbol.clone());
        }
        for symbol in newly {
            self.exported_insts.insert(symbol);
        }
        funcs
    }

    /// Evaluate one line of input, writing any success output to `writer`.
    /// On error, the session (env, stack) is left untouched; the caller
    /// prints the returned diagnostic.
    fn eval_line(&mut self, src: &str, writer: &mut impl Write) -> Result<(), String> {
        let tokens = lexer::lex(src)?;
        // R1 (slice 5b): `import:` as the first token routes to `eval_import`
        // (5a's R23 rejection is gone), guarded beside the `type:` special-case
        // and before `parse_line_with_structs` (which never learns qualifiers).
        // `export:` at the REPL is a new located rejection: a live session has
        // no export boundary to cross, and without this guard it would fall
        // through to an unrelated parse error.
        if let Some((Token::Word(w), span)) = tokens.first() {
            if w == "import:" {
                return self.eval_import(&tokens, writer);
            }
            if w == "export:" {
                return Err(format!(
                    "error: `export:` has no meaning at the REPL (line {}, col {})\n  note: a live session has no export boundary to cross",
                    span.line, span.col
                ));
            }
            // R8 (slice 3r): `impl:`/`trait:` are wired only through
            // `assemble_module`, so without this guard they fall through to
            // the term loop and report an unrelated parse error.
            if w == "impl:" {
                return Err(format!(
                    "error: `impl:` has no meaning at the REPL (line {}, col {})\n  note: a live session has no module to attach a trait implementation to",
                    span.line, span.col
                ));
            }
            if w == "trait:" {
                return Err(format!(
                    "error: `trait:` has no meaning at the REPL (line {}, col {})\n  note: a live session declares no trait to satisfy",
                    span.line, span.col
                ));
            }
        }
        if matches!(tokens.first(), Some((Token::Word(w), _)) if w == "type:") {
            return self.eval_typedef(&tokens, writer);
        }
        // A rejected line must leave the session's interned type registries
        // untouched: `parse_line_with_structs` (and `check_def`) intern any
        // array/cell/ref shape the line names *before* the line is accepted,
        // so a rejected line -- e.g. one R7a rejects for a quotation-nested
        // array/cell/ref -- would otherwise leave that entry resident and
        // re-trigger the per-line audit on every later line (the audit scans
        // the whole registry, and dedup means a re-parse of the same shape
        // hits the resident entry). Snapshot the lengths and truncate on error.
        let arrays_len = self.arrays.len();
        let cells_len = self.owned_cells.len();
        let refs_len = self.refs.len();
        let slices_len = self.slices.len();
        let result = self.eval_expr_or_def_line(&tokens, writer);
        if result.is_err() {
            self.arrays.truncate(arrays_len);
            self.owned_cells.truncate(cells_len);
            self.refs.truncate(refs_len);
            self.slices.truncate(slices_len);
        }
        result
    }

    /// The parse-and-dispatch tail of `eval_line` for an ordinary (non-`type:`,
    /// non-`import:`) line. Split out so `eval_line` can snapshot and restore
    /// the interned type registries around it (see the call site).
    fn eval_expr_or_def_line(
        &mut self,
        tokens: &[(Token, Span)],
        writer: &mut impl Write,
    ) -> Result<(), String> {
        let ctx = parser::ImportCtx {
            imports: &self.import_qualifier_module,
            selective: &self.import_selective_module,
            exports: &self.import_exports,
        };
        let mut line = parser::parse_line_with_structs(
            tokens,
            &self.structs,
            &self.enums,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &mut self.slices,
            ctx,
        )?;
        // P7.S3t (R10): before any rewriting or checking, since the guard is
        // about the session's whole lowering path rather than about this
        // line's names.
        match &line {
            Line::Expr(terms) => reject_explicit_instantiation(terms)?,
            Line::Def(word) => reject_explicit_instantiation(&word.body)?,
        }
        // R8c: rewrite body-position `q::w` / `q::T>` calls to their
        // current internal (epoch-tagged) spelling before ordinary checking
        // runs; also raises R15's `not exported` for a private qualified name.
        self.rewrite_line_imports(&mut line)?;
        // R7a (item 2): a quotation type is legal only as a direct word
        // parameter this slice; reject it in any audited registry position
        // (a struct/enum field, an array element, a cell payload, a reference
        // referent -- all interned during the parse above) before lowering can
        // reach `ir_type_of`'s `unreachable!` arm and brick the session. The
        // native `check` runs this after `check_types`; the REPL's per-line
        // `check_types` path skipped it (`type:` lines run their own copy).
        // P7 slice 3c (R1.2, phase 3 review fix): a signature's `Slice[T]`
        // interns into `self.slices` at parse time above, on every dispatch
        // path below (ordinary def, combinator, poly, quotation-param
        // rejection alike) -- reject a disallowed element here, before any of
        // them can reach `slice_layout`/`scalar_size_align` and brick the
        // session, mirroring the quotation-type audit just above.
        check::check_slice_element_gate(&self.structs, &self.enums, &self.arrays, &self.slices)?;
        check::audit_quotation_type_registries(
            &self.structs,
            &self.enums,
            &self.arrays,
            &self.owned_cells,
            &self.refs,
        )?;
        match line {
            // R11: a `: drop` line never enters `self.env` or gets lowered
            // under its own name; it becomes the struct's destructor, the
            // same substitution `ir::lower` performs for a compiled module.
            Line::Def(word) => {
                // R8e: a declared word name containing `::` would collide with
                // an imported name's internal tag; reject it up front (covers
                // the drop / def / poly fan-out with one check).
                reject_double_colon_name("word", &word.name, word_span(&word))?;
                // Phase 6 slice 3: the session analogue of
                // `assemble_module`'s own shadowing check. Eliminator
                // interception runs ahead of the env lookup here too, so a
                // word named `Shape?` would be accepted and then permanently
                // unreachable -- every call to it routes to the generated
                // eliminator instead.
                check::check_no_word_shadows_eliminator(std::slice::from_ref(&word), &self.enums)?;
                // R6: `check_globals` runs only in `assemble_module`, so a
                // `global:` clause here would be accepted and never checked.
                // A live session declares no statics, so no entry could ever
                // resolve; reject the clause, as `export:` is rejected above.
                if let Some(entry) = word.declared_globals.as_ref().and_then(|e| e.first()) {
                    return Err(format!(
                        "error: `global:` has no meaning at the REPL (line {}, col {})\n  note: a live session declares no `static:` storage, so a word's global set cannot be checked",
                        entry.span.line, entry.span.col
                    ));
                }
                if word.name == "drop" {
                    self.eval_drop_overload(word, writer)
                } else {
                    self.eval_def(word, writer)
                }
            }
            Line::Expr(terms) => self.eval_expr(&terms, writer),
        }
    }

    /// Register a `type:` struct declaration. The new name is appended to the
    /// registry first (so a self-reference in its own fields resolves, and is
    /// then rejected as recursion); fields resolve against the whole registry.
    /// On any error the appended entry is rolled back, leaving the session
    /// untouched.
    fn eval_typedef(
        &mut self,
        tokens: &[(Token, Span)],
        writer: &mut impl Write,
    ) -> Result<(), String> {
        let (name, span) = match tokens.get(1) {
            Some((Token::Word(w), span)) => (w.clone(), *span),
            _ => return Err("parse error: `type:` must be followed by a type name".to_string()),
        };
        parser::reject_reserved_name("type", &name, span)?;
        // R8e: a REPL-declared type name may not carry the `::` reserved for
        // qualified imported spellings.
        reject_double_colon_name("type", &name, span)?;
        // Item 1: parsing a `type:` line interns any array/cell/ref shape its
        // fields name (e.g. a quotation-nested `[ [ i64 -- ] 3 ]`) into the
        // session registries *before* the R7a audit rejects it. The
        // struct/enum helpers roll back only `self.structs` / `self.enums`, so
        // a poisoned interned entry would survive the failed line and re-fire
        // the per-line audit forever, bricking the session. Snapshot and
        // truncate the interned registries here, mirroring `eval_line`'s
        // guard on the non-`type:` path.
        let arrays_len = self.arrays.len();
        let cells_len = self.owned_cells.len();
        let refs_len = self.refs.len();
        let result = if parser::typedef_line_is_enum(tokens) {
            self.eval_enum_typedef(tokens, name.clone(), span)
        } else {
            self.eval_struct_typedef(tokens, name.clone(), span)
        };
        if result.is_err() {
            self.arrays.truncate(arrays_len);
            self.owned_cells.truncate(cells_len);
            self.refs.truncate(refs_len);
            return result;
        }
        writeln!(writer, "defined type {name}").map_err(|e| format!("writing stdout: {e}"))?;
        Ok(())
    }

    fn eval_struct_typedef(
        &mut self,
        tokens: &[(Token, Span)],
        name: String,
        span: Span,
    ) -> Result<(), String> {
        let idx = self.structs.len();
        self.structs.push(StructDecl {
            name: name.clone(),
            name_static: Box::leak(name.into_boxed_str()),
            fields: Vec::new(),
            span,
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        });
        let ctx = parser::ImportCtx {
            imports: &self.import_qualifier_module,
            selective: &self.import_selective_module,
            exports: &self.import_exports,
        };
        let result = parser::parse_typedef_line(
            tokens,
            &self.structs,
            &self.enums,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            ctx,
        )
        .and_then(|fields| {
            self.structs[idx].fields = fields;
            check::check_types(
                &self.structs,
                &self.enums,
                &[],
                &[],
                &self.arrays,
                &self.owned_cells,
                &self.slices,
            )?;
            // R7a (item 2): a quotation-typed struct field never reaches the
            // native `unreachable!` because the native `check` audits it; the
            // REPL must run the same audit or the field bricks the session.
            check::audit_quotation_type_registries(
                &self.structs,
                &self.enums,
                &self.arrays,
                &self.owned_cells,
                &self.refs,
            )
        });
        if let Err(e) = result {
            self.structs.pop();
            return Err(e);
        }
        Ok(())
    }

    /// Register a `type:` enum declaration (D1). The name and its variant-name
    /// skeleton are appended first (so a self/forward reference in a variant
    /// field resolves, then is rejected as recursion), fields resolve against
    /// the struct and enum registries, and the whole entry is rolled back on
    /// any error.
    fn eval_enum_typedef(
        &mut self,
        tokens: &[(Token, Span)],
        name: String,
        span: Span,
    ) -> Result<(), String> {
        let variant_names = parser::enum_variant_names(tokens);
        for (vname, vspan) in &variant_names {
            parser::reject_reserved_name("variant", vname, *vspan)?;
        }
        // Phase 6 slice 3: the same collision arrived at from the other side
        // -- a session word already holds the name this enum's eliminator
        // would take, which would make that word unreachable from the next
        // line on. A session declares one thing per line, so each ordering
        // has to be caught where it happens; `assemble_module` sees both at
        // once and needs only its whole-module scan.
        let eliminator_name = format!("{name}?");
        if self.word_names().contains(&eliminator_name)
            || self.combinators.contains_key(&eliminator_name)
        {
            return Err(check::word_shadows_eliminator_error(
                &eliminator_name,
                span,
                &name,
            ));
        }
        let variants = variant_names
            .into_iter()
            .map(|(vname, vspan)| VariantDecl {
                display_static: Box::leak(format!("{name}.{vname}").into_boxed_str()),
                name: vname.clone(),
                name_static: Box::leak(vname.into_boxed_str()),
                fields: Vec::new(),
                span: vspan,
            })
            .collect();
        let idx = self.enums.len();
        self.enums.push(EnumDecl {
            name: name.clone(),
            name_static: Box::leak(name.into_boxed_str()),
            variants,
            span,
            module: 0,
        });
        let ctx = parser::ImportCtx {
            imports: &self.import_qualifier_module,
            selective: &self.import_selective_module,
            exports: &self.import_exports,
        };
        let result = parser::parse_enum_typedef_line(
            tokens,
            &self.structs,
            &self.enums,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            ctx,
        )
        .and_then(|variant_fields| {
            for (vidx, fields) in variant_fields.into_iter().enumerate() {
                self.enums[idx].variants[vidx].fields = fields;
            }
            check::check_types(
                &self.structs,
                &self.enums,
                &[],
                &[],
                &self.arrays,
                &self.owned_cells,
                &self.slices,
            )?;
            // R7a (item 2): a quotation-typed enum-variant payload, same hazard.
            check::audit_quotation_type_registries(
                &self.structs,
                &self.enums,
                &self.arrays,
                &self.owned_cells,
                &self.refs,
            )
        });
        if let Err(e) = result {
            self.enums.pop();
            return Err(e);
        }
        Ok(())
    }

    /// R12: the session-scope selective-collision check. A bare selected name
    /// colliding with an existing session name -- a locally-defined word or
    /// type (bare, `module == 0` in the session's own registries; import
    /// splices always carry `module >= 1`), or a prior selective import's
    /// unqualified name (a bare `import_aliases` key, which only a selective
    /// import ever inserts) -- is a located error at the second occurrence,
    /// naming both sources. No precedence, no shadowing, no use-site
    /// disambiguation, exactly 5a R21's rule.
    fn check_session_selective_collisions(
        &self,
        qualifier: &str,
        selective: &[(String, Span)],
    ) -> Result<(), String> {
        for (name, span) in selective {
            if self.env.contains_key(name)
                || self
                    .structs
                    .iter()
                    .any(|s| s.module == 0 && s.name_static == name)
                || self
                    .enums
                    .iter()
                    .any(|e| e.module == 0 && e.name_static == name)
            {
                return Err(session_selective_collides_with_local_error(
                    name, qualifier, *span,
                ));
            }
            if let Some(existing) = self.import_aliases.get(name) {
                let first = existing
                    .split_once("::")
                    .map_or(existing.as_str(), |(q, _)| q);
                // A rebind of the same qualifier is a reload (R13), not a
                // collision: its own prior bare alias is purged in
                // `splice_import` before the new one lands. Only a *different*
                // qualifier already exposing `name` is R12's collision.
                if first != qualifier {
                    return Err(session_selective_collision_error(
                        name, first, qualifier, *span,
                    ));
                }
            }
        }
        Ok(())
    }

    /// R1-R10/R15/R16 (slice 5b): evaluate an `import:` line. Reuses the native
    /// pipeline (`discover_closure` -> `assemble_module` -> `check::check`) to
    /// produce a checked closure `Module`, bulk-lowers it to one `.so` with
    /// each word minted a session-fresh import-epoch symbol (R6), and splices
    /// module 0's exports into the session's env and registries with a full
    /// positional type-id remap (R9). Every fallible step runs before any
    /// mutation of `self`, so a failed import leaves the session untouched
    /// (R16).
    fn eval_import(
        &mut self,
        tokens: &[(Token, Span)],
        writer: &mut impl Write,
    ) -> Result<(), String> {
        // R2: parse the line through the shared form parser, so a malformed
        // `import:` yields R9's construct-naming located error unchanged.
        let import = parser::scan_imports(tokens)?
            .into_iter()
            .next()
            .ok_or_else(|| "parse error: expected an `import:` form".to_string())?;
        // P8 slice 1a: resolving a module-name import needs a manifest to
        // resolve it against, and nothing in `assemble_module` supplies one
        // for the REPL -- so it is rejected outright rather than falling
        // through to a resolution it cannot do.
        let ImportTarget::Path(path) = &import.target else {
            return Err(format!(
                "error: module-name import at line {}, col {} in <repl>:\n  the REPL cannot resolve a module-name import yet\n  use a quoted-path import instead",
                import.span.line, import.span.col
            ));
        };
        // A wildcard import binds no qualifier, and nothing here gives it a
        // visibility effect to splice in: it would bring in nothing at all,
        // silently.
        let Some(qualifier) = import.qualifier() else {
            return Err(format!(
                "error: wildcard import at line {}, col {} in <repl>:\n  a wildcard import binds no names in the REPL\n  use a qualified import instead",
                import.span.line, import.span.col
            ));
        };
        // R3: the REPL's own top-level path resolves relative to the process
        // cwd; every transitive import inside the closure keeps 5a's
        // importer-relative rule (inside `discover_closure`).
        let closure = driver::discover_closure(Path::new(path))?;
        let mut module = driver::assemble_module(&closure, false)?;
        check::check(&mut module)?;
        // R14/D4: an imported closure declaring `main` (in any of its files,
        // not only module 0) is rejected before any codegen, naming the file
        // and the word. `allowed_module: None` -- no file in an imported
        // closure may declare `main`, unlike `driver::build`'s native path,
        // where module 0 is the program's own entry point.
        driver::check_no_main_in_closure(&module, &closure, None)?;
        // Slice 12 (R-D5/E4): `eval_def` declines an ordinary `[ ... ]`-parameter
        // word, and the import path has to decline the same shape or the
        // boundary is only half a boundary. `splice_import`'s two binding loops
        // would otherwise bind it as an ordinary word, and the later call site
        // builds its quotation argument in the *session's* translation unit,
        // where the `__quot0` code pointer is a non-PIC relocation: the line
        // dies in `ld` instead of at the boundary. Rejected here, above the
        // commit point, so a refused import leaves the session untouched (R16),
        // and for both exported and private module-0 words, since a retained
        // combinator's body can call a private one.
        if let Some(w) = module.words.iter().find(|w| {
            w.module == 0
                && w.poly.is_none()
                && !check::is_combinator(w)
                && check::word_declares_quotation_parameter(w)
        }) {
            let span = check::word_span(w);
            return Err(format!(
                "error: word `{}` takes a `[ ... ]` quotation parameter and lowers to a real call, which is not supported in the REPL ({}, line {}, col {})",
                crate::resolve::demangle_word(&w.name),
                closure.path_of(w.module).display(),
                span.line,
                span.col
            ));
        }
        // R12 (slice 6c): a closure exporting a quotation-taking word is no
        // longer rejected -- `splice_import` retains the combinator (D5).
        // R11: each selectively-imported name must be exported by module 0,
        // the R16 visibility error, checked against a synthesized entry for
        // the REPL's own top-level selection (the closure-internal check,
        // `check::check_selective_imports`, validates a module's own selective
        // imports against its own locals, the wrong scope for the REPL's,
        // which has no module of its own to be local to).
        for (name, span) in import.selective() {
            if !module.modules[0].exports.iter().any(|(n, _)| n == name) {
                return Err(check::selective_not_exported_error(
                    name,
                    Some(qualifier),
                    *span,
                ));
            }
        }
        // R12: a selectively-exposed name colliding with an existing session
        // name (a locally-defined word/type, or a prior selective import's
        // unqualified name) is a located error at the second, naming both
        // sources -- 5a R21's dumb collision rule extended to session scope.
        self.check_session_selective_collisions(qualifier, import.selective())?;
        // R6/R9: read (do not yet advance) this event's epoch and module-id
        // base, so every fallible step below leaves `self` untouched (R16).
        let epoch = self.import_epoch;
        let module_base = self.next_import_module;
        let n_modules = module.modules.len() as u32;
        // R5/R6: bulk-lower the whole closure to one `.so`, each word renamed
        // to its import-epoch symbol.
        let lib = compile_import_closure(&module, epoch)?;
        // ---- commit (infallible from here) ----
        self.import_epoch += 1;
        self.next_import_module += n_modules;
        self.libs.push(lib);
        self.splice_import(qualifier, import.selective(), &module, epoch, module_base);
        writeln!(writer, "imported {qualifier}").map_err(|e| format!("writing stdout: {e}"))
    }

    /// R8/R9/R15: splice module 0's exports into the session. Infallible: every
    /// error path is upstream in `eval_import`. Appends the whole closure's
    /// registries with a constant positional-id shift (R9), tags each decl
    /// with its event module id and epoch `.name` (R8a/R8b), binds exported
    /// words into `self.env` under their import-epoch symbol, records the
    /// qualifier's aliases / private names / export lists.
    fn splice_import(
        &mut self,
        qualifier: &str,
        selective: &[(String, Span)],
        module: &Module,
        epoch: u64,
        module_base: u32,
    ) {
        let q = qualifier;
        // R13: a rebind (`q` already bound, same path or a different one)
        // must not leave a stale alias from the old epoch's export set that
        // this splice doesn't recreate -- e.g. the old file exported `foo`
        // and the new one doesn't, at all. Purging every `q::`-prefixed alias
        // up front, before this splice re-adds whatever the new closure
        // actually exports, is what makes a post-rebind `q::foo` fall through
        // to `not exported`/`unknown word` judged against the new file only,
        // never a stale hit on the old file's export status. The underlying
        // `self.env`/`self.structs` rows the old alias pointed at are never
        // touched here (R9 positional stability): only the alias, the lookup
        // key, is replaced.
        let prefix = format!("{q}::");
        // R11/R13: a selective import also adds a *bare* alias (no `q::`
        // prefix on the key), so a rebind must purge those too, or a stale
        // bare alias from the old epoch's selective list would survive a
        // rebind that no longer selects that name. A bare alias's *value*
        // always carries the `q::` prefix (it points at the same internal
        // spelling the qualified alias does), so purging on either side of
        // the entry catches both shapes without needing a separate ownership
        // map.
        self.import_aliases
            .retain(|k, v| !k.starts_with(&prefix) && !v.starts_with(&prefix));
        // R11/R13: purge any selective type-position mapping this qualifier's
        // old epoch installed, so a rebind that no longer selectively imports
        // a type doesn't leave a stale `selective` entry resolving to the old
        // event's module id.
        if let Some(&old_module) = self.import_qualifier_module.get(q) {
            self.import_selective_module.retain(|_, m| *m != old_module);
        }
        let selective_names: HashSet<&str> = selective.iter().map(|(n, _)| n.as_str()).collect();
        let struct_base = self.structs.len();
        let enum_base = self.enums.len();
        // The session's own `bool` variant spellings, snapshotted before any
        // append below -- `&'static str` is `Copy`, so this borrows nothing
        // and is safe to hold across the mutation loop that follows.
        let session_bool_variants: Vec<&'static str> = self.enums[self.bool_enum.index()]
            .variants
            .iter()
            .map(|v| v.name_static)
            .collect();
        let enum_remap = EnumRemap::new(module, enum_base, self.bool_enum, &session_bool_variants);
        let array_base = self.arrays.len();
        let cell_base = self.owned_cells.len();
        let ref_base = self.refs.len();
        let slice_base = self.slices.len();
        let remap = |ty: Type| {
            remap_type(
                ty,
                enum_remap,
                struct_base,
                array_base,
                cell_base,
                ref_base,
                slice_base,
            )
        };
        // Module 0's export list (words and types), the only names that cross
        // into callable session state (R8).
        let exports0: HashSet<&str> = module.modules[0]
            .exports
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        // A multi-file closure mangles module-0 words to `{name}__m0`
        // (`resolve_modules`); a single-file one leaves them raw. This maps an
        // export's raw name to the word's post-resolve `.name`.
        let multi = module.modules.len() >= 2;
        let mangled_of = |raw: &str| -> String {
            if multi && raw != "main" && raw != "drop" {
                format!("{raw}__m0")
            } else {
                raw.to_string()
            }
        };

        // R9: append arrays / owned-cells / refs, remapping the ids their inner
        // types carry, order preserved so each id shifts by a constant base.
        for a in &module.arrays {
            self.arrays.push(ArrayDecl {
                element: remap(a.element),
                count: a.count,
                name_static: a.name_static,
            });
        }
        for c in &module.owned_cells {
            self.owned_cells.push(OwnedCellDecl {
                payload: remap(c.payload),
                name_static: c.name_static,
            });
        }
        for r in &module.refs {
            self.refs.push(RefDecl {
                referent: remap(r.referent),
                mutable: r.mutable,
                name_static: r.name_static,
            });
        }
        // P7 slice 3c (R8.2): slices append like refs, so a `SliceId` shifts
        // by a constant base and `remap_type`'s rebase has a registry to land
        // in. Empty for every module today (no surface spelling mints one), but
        // the append has to exist for the base to be a real length rather than
        // a hardcoded zero.
        for sl in &module.slices {
            self.slices.push(SliceDecl {
                element: remap(sl.element),
                mutable: sl.mutable,
                name_static: sl.name_static,
            });
        }

        // R8a/R8b/R9: append every struct, remapping field ids and its own
        // module id, tagging `.name`. An exported module-0 struct gets the
        // alias-target tag `{q}::{T}__import{epoch}`; every other row gets a
        // unique inert tag so it never collides in `struct_generated_sigs`.
        for (i, s) in module.structs.iter().enumerate() {
            let fields = s
                .fields
                .iter()
                .map(|(f, ty)| (f.clone(), remap(*ty)))
                .collect();
            let is_export = s.module == 0 && !s.is_bundle && exports0.contains(s.name_static);
            let name = if is_export {
                format!("{q}::{}__import{epoch}", s.name_static)
            } else {
                format!("{}__import{epoch}__i{}", s.name_static, struct_base + i)
            };
            self.structs.push(StructDecl {
                name: name.clone(),
                name_static: s.name_static,
                fields,
                span: s.span,
                has_drop_overload: s.has_drop_overload,
                is_bundle: s.is_bundle,
                module: module_base + s.module,
            });
            if is_export {
                self.import_aliases
                    .insert(format!("{q}::{}", s.name_static), name.clone());
                // R11: a selectively-imported type's bare name is a *second*
                // alias at the same internal spelling (one `StructId` behind
                // both), plus the parallel `selective` map entry R8d's
                // type-position resolver reads, pointing at the same module
                // id the qualified spelling already targets.
                if selective_names.contains(s.name_static) {
                    self.import_aliases.insert(s.name_static.to_string(), name);
                    self.import_selective_module
                        .insert(s.name_static.to_string(), module_base + s.module);
                }
            }
        }

        // R9: append every enum with remapped variant-field ids and module id.
        // No aliases are built this phase (enums are out of phase-1 fixtures),
        // but the ids must still remap so a later reference stays consistent.
        //
        // P7 slice 3i (R2): the imported closure's own `core::bool` is the one
        // enum not appended -- `EnumRemap` folds it onto the session's seeded
        // copy instead, and holds the shape test that keeps an unrelated enum
        // merely *named* `bool` out of that fold.
        for (i, e) in module
            .enums
            .iter()
            .enumerate()
            .filter(|(i, _)| !enum_remap.folds(EnumId::from_index(*i)))
        {
            let variants = e
                .variants
                .iter()
                .map(|v| VariantDecl {
                    name: v.name.clone(),
                    name_static: v.name_static,
                    // Built before the enclosing `EnumDecl` computes its
                    // import-mangled name (below), so this forwards the
                    // existing spelling; cosmetic (pre-import), not correctness.
                    display_static: v.display_static,
                    fields: v
                        .fields
                        .iter()
                        .map(|(f, ty)| (f.clone(), remap(*ty)))
                        .collect(),
                    span: v.span,
                })
                .collect();
            self.enums.push(EnumDecl {
                name: format!(
                    "{}__import{epoch}__e{}",
                    e.name_static,
                    enum_remap.apply(EnumId::from_index(i)).index()
                ),
                name_static: e.name_static,
                variants,
                span: e.span,
                module: module_base + e.module,
            });
        }

        // R8: bind each exported module-0 word into `self.env` under its
        // epoch-tagged internal spelling, symbol = its import-epoch symbol,
        // `Sig` remapped.
        for (raw, _span) in &module.modules[0].exports {
            let mangled = mangled_of(raw);
            // R13 (slice 6c): skip an exported *combinator* here -- it mints no
            // `IrFunc`/symbol (R20), so binding it into `self.env` under an
            // import symbol would point at nothing. It is retained instead in
            // the combinator loop below.
            let Some(w) = module.words.iter().find(|w| {
                w.module == 0 && w.poly.is_none() && w.name == mangled && !check::is_combinator(w)
            }) else {
                continue; // an exported type name or combinator, handled elsewhere
            };
            let sig = Sig {
                inputs: w.effect.inputs.iter().map(|s| remap(s.ty)).collect(),
                outputs: w.effect.outputs.iter().map(|s| remap(s.ty)).collect(),
            };
            let internal = format!("{q}::{raw}__import{epoch}");
            let symbol = import_symbol(&w.name, epoch);
            self.env.insert(
                internal.clone(),
                WordEntry {
                    sig,
                    generation: epoch,
                    symbol,
                },
            );
            self.import_aliases
                .insert(format!("{q}::{raw}"), internal.clone());
            // R11: a selectively-imported word is exposed unqualified too, a
            // second alias at the same internal spelling.
            if selective_names.contains(raw.as_str()) {
                self.import_aliases.insert(raw.clone(), internal);
            }
        }

        // Review fix (slice 6c): bind each *private* (non-exported) module-0
        // ordinary word into `self.env` too, under the same internal spelling
        // an export gets, but with no `import_aliases` entry -- R15 privacy
        // holds, a session-typed `q::name` still misses. Without this, a
        // retained combinator's body call to a private word (e.g. `apply2`
        // calling `helper`) is left at its bare closure spelling by
        // `body_rename` below and falls through to whatever the *session's*
        // own env happens to hold under that name: a hygiene break that
        // silently returns a wrong answer instead of resolving against the
        // closure's own private definition.
        for w in &module.words {
            if w.module != 0
                || w.poly.is_some()
                || w.name == "main"
                || w.name == "drop"
                // An operator overload named `.` is not a per-file private
                // word to bind: the session seeds `core::bool`'s own copy
                // (`Session::new`), and binding an imported one as
                // `q::.__importN` here would leak a redundant, un-aliased
                // entry into completion for no benefit -- a call to bare `.`
                // inside a retained combinator's body already resolves
                // through the session's own identical
                // copy with no rename needed.
                || w.name == "."
                || check::is_combinator(w)
            {
                continue;
            }
            let raw = if multi {
                w.name.strip_suffix("__m0").unwrap_or(&w.name)
            } else {
                w.name.as_str()
            };
            if exports0.contains(raw) {
                continue; // already bound by the exports loop above
            }
            let sig = Sig {
                inputs: w.effect.inputs.iter().map(|s| remap(s.ty)).collect(),
                outputs: w.effect.outputs.iter().map(|s| remap(s.ty)).collect(),
            };
            let internal = format!("{q}::{raw}__import{epoch}");
            let symbol = import_symbol(&w.name, epoch);
            self.env.insert(
                internal,
                WordEntry {
                    sig,
                    generation: epoch,
                    symbol,
                },
            );
        }

        // R13/R14 (slice 6c): retain each module-0 exported *combinator* (mono
        // or poly) in the combinator store under its internal epoch-tagged
        // spelling, so a *later* session line calling `q::name` inlines its
        // body at that site's own live env (D1/D5), exactly as a session-
        // defined combinator does. Symmetric to the exported-ordinary-word
        // loop above, which filters on `poly.is_none()` and now also skips a
        // combinator, so a poly combinator like `filter`/`while` is never seen
        // there and needs this loop. Unlike an ordinary word this keeps no
        // `self.env` row and no symbol, only the raw terms and the alias.
        //
        // `body_rename` maps *every* module-0 word's body spelling to its
        // internal spelling -- not only its exports (review fix, slice 6c): a
        // retained body's call to a module-0 export (itself included, R14)
        // or to a module-0 *private* word must both resolve at the session
        // splice site against the closure's own definitions, never against
        // whatever the session's own env holds under that bare spelling. The
        // self-call rewrite is also what keeps an imported `while`'s
        // self-tail edge recognizable.
        let body_rename: HashMap<String, String> = module
            .words
            .iter()
            // Slice 9 phase 2 (R6): `.` is excluded for the same reason the
            // env-splice loop above skips it -- a spliced combinator's call
            // to bare `.` resolves against the session's own identical
            // injected copy with no rename needed, and no `q::.__importN`
            // env row exists to rename it to. P8 S2 (R3): the `core` words
            // used to be excluded here for the same reason -- they existed
            // twice, injected into the imported closure *and* seeded into the
            // session. The prelude is gone, so they exist once, in the
            // closure, and epoch-rename like any other imported word.
            // P8 S2 (R3): a *dependency* module's always-spliced word is
            // renamed too, not just module 0's. Module 0's own body may call
            // one (`lib/combinators.sth`'s `while` calls `core::bool`'s `if`),
            // and until the prelude was deleted that call named the injected,
            // never-mangled `if` and resolved against the session's own seeded
            // copy. It is `if__m2` now -- a real word of a real dependency
            // module -- so it has to be renamed and retained like any other.
            // Its mangled spelling is already module-unique, so it is used
            // as-is rather than stripped.
            .filter(|w| w.name != "main" && w.name != "drop" && w.name != ".")
            .map(|w| {
                let raw = match (w.module, multi) {
                    (0, true) => w.name.strip_suffix("__m0").unwrap_or(&w.name),
                    _ => w.name.as_str(),
                };
                (w.name.clone(), format!("{q}::{raw}__import{epoch}"))
            })
            .collect();
        // The dependency-module half of the retention below: an always-spliced
        // word module 0's own retained bodies call. No alias and no `self.env`
        // row -- it is reachable only through a spliced body, never by a name a
        // session line could write, so `q::if` still misses (R15 privacy).
        for w in &module.words {
            if w.module == 0 || !check::is_combinator(w) {
                continue;
            }
            let Some(internal) = body_rename.get(&w.name).cloned() else {
                continue;
            };
            let stored = remap_imported_combinator(
                w,
                &internal,
                &body_rename,
                module_base,
                enum_remap,
                struct_base,
                array_base,
                cell_base,
                ref_base,
                slice_base,
            );
            self.combinators.insert(internal, stored);
        }
        for (raw, _span) in &module.modules[0].exports {
            let mangled = mangled_of(raw);
            let Some(w) = module
                .words
                .iter()
                .find(|w| w.module == 0 && w.name == mangled && check::is_combinator(w))
            else {
                continue;
            };
            let internal = format!("{q}::{raw}__import{epoch}");
            let stored = remap_imported_combinator(
                w,
                &internal,
                &body_rename,
                module_base,
                enum_remap,
                struct_base,
                array_base,
                cell_base,
                ref_base,
                slice_base,
            );
            self.combinators.insert(internal.clone(), stored);
            self.import_aliases
                .insert(format!("{q}::{raw}"), internal.clone());
            // R13: a selectively-imported combinator is exposed unqualified
            // too, a second alias at the same internal spelling.
            if selective_names.contains(raw.as_str()) {
                self.import_aliases.insert(raw.clone(), internal);
            }
        }

        // R15: retain module 0's private names (bare word names, and for a
        // private type its bare name plus its destructure), so a `q::x` that
        // misses the aliases can be told `not exported` rather than unknown.
        let mut private: HashSet<String> = HashSet::new();
        for w in &module.words {
            if w.module != 0 || w.poly.is_some() || w.name == "main" || w.name == "drop" {
                continue;
            }
            let raw = if multi {
                w.name.strip_suffix("__m0").unwrap_or(&w.name)
            } else {
                w.name.as_str()
            };
            if !exports0.contains(raw) {
                private.insert(raw.to_string());
            }
        }
        for s in &module.structs {
            if s.module != 0 || s.is_bundle || exports0.contains(s.name_static) {
                continue;
            }
            let t = s.name_static;
            private.insert(t.to_string());
            private.insert(format!("{t}>"));
        }
        self.import_private.insert(q.to_string(), private);

        // R8a/R8d: bind the qualifier to module 0's session id and record every
        // event-module's export list, the parser's type-position resolver maps.
        self.import_qualifier_module
            .insert(q.to_string(), module_base);
        while self.import_exports.len() < (module_base + module.modules.len() as u32) as usize {
            self.import_exports.push(Vec::new());
        }
        for (m, info) in module.modules.iter().enumerate() {
            self.import_exports[module_base as usize + m] = info.exports.clone();
        }
    }

    /// R8c/R15: rewrite a just-parsed line's body-position calls, translating a
    /// user-facing `q::w` / `q::T>` spelling to its current internal
    /// (epoch-tagged) one before ordinary checking runs, and raising R15's
    /// `not exported` for a private qualified name. Type-position references
    /// are already resolved by the parser (R8d) and are untouched here.
    fn rewrite_line_imports(&self, line: &mut Line) -> Result<(), String> {
        match line {
            Line::Expr(terms) => self.rewrite_terms_imports(terms),
            Line::Def(word) => self.rewrite_wordbody_imports(&mut word.body),
        }
    }

    fn rewrite_wordbody_imports(&self, body: &mut [Term]) -> Result<(), String> {
        self.rewrite_terms_imports(body)
    }

    fn rewrite_terms_imports(&self, terms: &mut [Term]) -> Result<(), String> {
        for term in terms.iter_mut() {
            match &mut term.kind {
                TermKind::Call(name, _) => {
                    if let Some(new) = self.rewrite_import_call(name, term.span)? {
                        *name = new;
                    }
                }
                TermKind::Quotation(inner, _, _) => self.rewrite_terms_imports(inner)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// The single-call rewrite: `Some(new)` to replace the spelling, `None` to
    /// leave it (a local or a genuinely absent name falls through to the
    /// ordinary unknown-word path), `Err` for R15's `not exported`.
    fn rewrite_import_call(&self, name: &str, span: Span) -> Result<Option<String>, String> {
        // Phase 6 slice 3 review fix (cycle 3): the *whole* spelling first. A
        // word may itself be named `ok?` or `foo>` -- an ordinary spelling,
        // aliased whole -- and teaching `split_destructure_suffix` about the
        // eliminator's `?` made every such imported word miss its alias and
        // report `unknown word` (a call that resolved before this slice).
        // Only when no alias holds the whole name is a trailing sigil a
        // *generated* word's suffix on an aliased type (`q::P>`).
        if let Some(internal) = self.import_aliases.get(name) {
            return Ok(Some(internal.clone()));
        }
        let (base, suffix) = split_destructure_suffix(name);
        if let Some(internal) = self.import_aliases.get(base) {
            return Ok(Some(format!("{internal}{suffix}")));
        }
        // R15: a `q::x` whose base misses the aliases but names a private
        // word/type of a bound qualifier is `not exported`, distinct from
        // unknown.
        if let Some((qualifier, rest)) = name.split_once("::") {
            if self.import_qualifier_module.contains_key(qualifier) {
                if let Some(private) = self.import_private.get(qualifier) {
                    if private.contains(rest) {
                        // R15 gates a generated word by its *type*'s export
                        // status, so the message names the type (`q::P>` ->
                        // `P`). A private type is retained under its bare name
                        // beside its generated one, which is what tells that
                        // case from a word whose own name ends in `>`/`?`
                        // (`ok?`, which names itself).
                        let (head, _) = split_destructure_suffix(rest);
                        let named = match private.contains(head) {
                            true => head,
                            false => rest,
                        };
                        return Err(crate::resolve::not_exported_error(named, qualifier, span));
                    }
                }
            }
        }
        Ok(None)
    }

    /// R11: the retained overrides as the map destructor synthesis consumes,
    /// borrowed from the session instead of from a module's `words` (the REPL
    /// has no persistent `module.words` to index into).
    ///
    /// R11.3: only `declaring`, the struct whose `: drop` line is being
    /// evaluated right now, contributes a body to lower; every other override
    /// is `AlreadyLoaded`. A retained body was checked against the env of its
    /// own line, so re-lowering it into a later line's module resolves its
    /// callees against an env it was never checked against: a callee redefined
    /// at a different arity panics lowering, and one redefined at the same
    /// arity would silently never take effect anyway, since the pinned symbol
    /// keeps the first-loaded body under `RTLD_GLOBAL`. Lowering an override
    /// exactly once, on its own line, gives it the same snapshot semantics an
    /// ordinary word already has (a word's body binds the callee generations
    /// visible when it was defined).
    fn drop_override_bodies(&self, declaring: Option<StructId>) -> ir::DropOverrides<'_> {
        self.drop_overloads
            .iter()
            .map(|(id, (_, word))| {
                let entry = if Some(*id) == declaring {
                    ir::DropOverride::Body(word)
                } else {
                    ir::DropOverride::AlreadyLoaded
                };
                (*id, entry)
            })
            .collect()
    }

    /// R11.2: stamp *every* linear struct's/enum's/cell's destructor symbol
    /// with the session's current override epoch, once the session has ever
    /// defined a `drop` override (`self.override_epoch` is `None` until
    /// then). Not only the overridden struct's own: a struct/enum/cell with no
    /// override of its own can still `Call`, inside its own destructor, one
    /// that composes an overridden struct, so its body's callee changes across
    /// an override event too. (An overridden struct's own symbol is the one
    /// exception, stamped with the epoch its override was *defined* at so it
    /// never moves again -- R11.3, at the loop below.) Redefining a `: drop` without this would
    /// define one unmangled global (the overridden struct's own symbol, or
    /// worse, an *enclosing* one that merely calls it) twice with two
    /// different bodies, ambiguous under the session's `RTLD_GLOBAL` loading,
    /// which keeps whichever definition loaded first -- silently pinning a
    /// stale callee forever rather than merely failing to load. Applied to
    /// the built layouts because that is where every symbol-minting site
    /// (destructor synthesis and `emit_drop`) reads it from. Before any
    /// override, epoch is `None` and every symbol stays unsuffixed, matching
    /// the build path.
    fn apply_drop_generations(
        &self,
        structs: &mut ir::Structs,
        enums: &mut ir::Enums,
        cells: &mut ir::Cells,
    ) {
        for (idx, layout) in structs.layouts.iter_mut().enumerate() {
            // R11.3: an override's symbol is pinned to its defining epoch, so
            // the body compiled on that line stays the resolved destructor
            // (here and at every `emit_drop` call site) without being
            // re-lowered on every later line. Later override events still move
            // every *other* symbol, so an enclosing aggregate's glue is
            // re-emitted and re-resolves to whichever epoch each override is
            // pinned at.
            layout.drop_generation = match self.drop_overloads.get(&StructId::from_index(idx)) {
                Some((epoch, _)) => Some(*epoch),
                None => self.override_epoch,
            };
        }
        for layout in &mut enums.layouts {
            layout.drop_generation = self.override_epoch;
        }
        for generation in &mut cells.drop_generations {
            *generation = self.override_epoch;
        }
    }

    /// R11: define (or redefine) a struct's `drop` overload. The body is
    /// checked exactly as any other word body, then compiled straight into
    /// the struct's destructor symbol rather than under its own name: nothing
    /// resolves a `drop` call site by name, at the REPL or natively. On any
    /// failure the session is left as it was.
    fn eval_drop_overload(&mut self, word: WordDef, writer: &mut impl Write) -> Result<(), String> {
        let id = check::drop_overload_struct_id(&word)?;
        // R11.2: bump the session-wide epoch before compiling, so this line's
        // own destructor set (the override's and every other linear type's)
        // mints fresh symbols reflecting it; rolled back below on failure,
        // exactly like `drop_overloads` and `has_drop_overload`. Redefinition
        // follows the session's ordinary generation-bump rule, not the
        // per-module duplicate-override rejection: a second `: drop` line
        // replaces the first, like any other redefinition.
        let previous_epoch = self.override_epoch;
        let epoch = previous_epoch.map_or(0, |e| e + 1);
        let had_overload = self.structs[id.index()].has_drop_overload;
        let previous = self.drop_overloads.insert(id, (epoch, word));
        let previous_sites = self.drop_dropped_sites.remove(&id);
        // Set before the body is checked and before this line's own
        // destructor synthesis: the receiver must already be linear while its
        // own `drop` body is checked, and the defining line must emit the
        // override, not one last round of generic glue.
        self.structs[id.index()].has_drop_overload = true;
        self.override_epoch = Some(epoch);

        match self.compile_drop_overload(id, epoch) {
            Ok(lib) => {
                self.libs.push(lib);
                let name = self.structs[id.index()].name.clone();
                writeln!(writer, "defined drop for {name}")
                    .map_err(|e| format!("writing stdout: {e}"))
            }
            Err(e) => {
                match previous {
                    Some(entry) => self.drop_overloads.insert(id, entry),
                    None => self.drop_overloads.remove(&id),
                };
                match previous_sites {
                    Some(sites) => self.drop_dropped_sites.insert(id, sites),
                    None => self.drop_dropped_sites.remove(&id),
                };
                self.structs[id.index()].has_drop_overload = had_overload;
                self.override_epoch = previous_epoch;
                Err(e)
            }
        }
    }

    /// R11: check the just-registered override's body and compile this
    /// epoch's destructor set into one loadable object. The only place the
    /// override's body is ever lowered (R11.3).
    fn compile_drop_overload(&mut self, id: StructId, epoch: u64) -> Result<Library, String> {
        let env = self.typed_env();
        // R5 (Slice 2): the drop-overload site collector passes the **empty**
        // poly-env so drop-reachability stays byte-identical to the pre-slice
        // native path (a `drop` overload is never polymorphic, and the
        // native-shared reachability code must not diverge). The relayed
        // instantiation table is empty and discarded.
        // R4 (Slice 6c): a `drop` override body may call a retained combinator,
        // so its site collection sees the session's inline view. The poly-env
        // stays empty above (a `drop` overload is never polymorphic).
        let combinators = checker_combinators(&self.combinators);
        // Item 3: a `drop` override body's own resolved-overload sites are
        // discarded here, same as `_insts` -- `synthesize_aggregate_destructors`
        // below has no threading for them yet (a narrower, pre-existing gap
        // than the crash item 3 fixes; see its call site). Its field
        // projections (R2) are *not* discarded: this is the only place the
        // override's body is lowered (R11.3), so they reach that lowering.
        let (sites, _insts, _overloads, fields, variant_fields) =
            check::check_def_collecting_drop_sites(
                &self.drop_overloads[&id].1,
                &self.enums,
                &env,
                &mut self.arrays,
                &mut self.owned_cells,
                &mut self.refs,
                &mut self.slices,
                &self.structs,
                &HashMap::new(),
                &combinators,
            )?;
        self.drop_dropped_sites.insert(id, sites);

        // R6 at the REPL: checking this override's body in isolation cannot
        // ask the whole-session reachability question (this override, or one
        // reachable through it, disposing itself), so it is asked separately
        // here, against every override currently live in the session (this
        // line's own included) and each one's *cached* drop sites -- never a
        // re-check of an earlier line's body against this line's env, which
        // would reintroduce the stale-env hazard R11.2/R11.3 already fixed
        // for lowering.
        let overrides: Vec<(StructId, &WordDef, &[Type])> = self
            .drop_overloads
            .iter()
            .map(|(&sid, (_, word))| (sid, word, self.drop_dropped_sites[&sid].as_slice()))
            .collect();
        check::check_drop_overload_reachability(
            &overrides,
            &self.structs,
            &self.enums,
            &self.arrays,
            &self.owned_cells,
        )?;

        let ir_lower_env = ir_arity_env(&env);
        let (mut structs, mut enums, arrays, mut cells, refs) = ir::build_registries(
            &self.structs,
            &self.enums,
            &self.arrays,
            &self.owned_cells,
            &self.refs,
        );
        self.apply_drop_generations(&mut structs, &mut enums, &mut cells);
        let slices = ir::build_slices(&self.slices, &structs, &enums, &arrays);
        let regs = ir::Registries {
            structs: &structs,
            enums: &enums,
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
            slices: &slices,
            statics: ir::empty_statics(),
        };
        let funcs = {
            let resolve = resolver_for(&self.env);
            ir::synthesize_aggregate_destructors(
                &ir_lower_env,
                &resolve,
                regs,
                &self.drop_override_bodies(Some(id)),
                &fields,
                &variant_fields,
                &combinator_bodies(&self.combinators),
            )
        };

        let quot_sigs =
            ir::collect_quot_sigs(&funcs, &structs.layouts, &enums.layouts, &arrays.layouts);
        let ssa = backend::qbe::emit(&IrModule {
            funcs,
            structs: structs.layouts,
            enums: enums.layouts,
            arrays: arrays.layouts,
            quot_sigs,
            statics: Vec::new(),
        })?;
        let dir = driver::tempfile_dir()?;
        let so_path = dir.join(format!("drop_{}_epoch{epoch}.so", id.index()));
        driver::compile_so(&ssa, &so_path)?;
        Library::open(&so_path)
    }

    /// R3/R4/R7 (Slice 2): accept a polymorphic REPL definition. The body is
    /// checked by the native poly body-checker `check_poly_body` (X1 on an
    /// ill-typed body), a `>= 2`-output signature is a clean located deferral
    /// (X3, no return bundle is interned at the REPL), and the word is retained
    /// in `poly_words` with a frozen defining-line resolver snapshot and its
    /// generation. Nothing is compiled here: a polymorphic word has no concrete
    /// instantiation to lower until a later line calls it at a concrete type.
    fn eval_poly_def(&mut self, word: WordDef, writer: &mut impl Write) -> Result<(), String> {
        let sig = word
            .poly
            .as_deref()
            .expect("eval_poly_def is only reached for a polymorphic word")
            .clone();
        let env = self.typed_env();
        // R3: check the body over a `PolyType` stack, always first, so the
        // multi-output gate below only ever sees a body that already
        // type-checked (`: twice ( 'T -- 'T 'T ) dup ;` fails the `Copy` gate
        // here as X1, never reaching the gate despite its two outputs).
        // R7: this body's resolved-overload call sites, frozen into
        // `PolyWordEntry` alongside the resolver/arity snapshot below, so a
        // later instantiation's lowering dispatches through them instead of
        // an empty map.
        let mut builtin_overloads: HashMap<Span, String> = HashMap::new();
        // P7 slice 3b-follow (R2): the session's retained combinators, so a
        // poly body defined at the REPL reaches the same row-typed consumer
        // dispatch a native one does.
        let combinators = checker_combinators(&self.combinators);
        // P7 slice 3a: a session-defined poly word can never name a generic
        // `type:` (the REPL has no way to declare one, D2), so `None` here
        // is correct, not a gap.
        check::check_poly_body(
            &word,
            &sig,
            &env,
            &combinators,
            &self.structs,
            &self.enums,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &mut self.slices,
            &[],
            None,
            &mut builtin_overloads,
            // P7.S3e (R18): a session declares no `trait:`, so the only table
            // entry is the pre-seeded `Copy` predicate (P7.S3s: `Ord` is an
            // ordinary `core::cmp` trait now, and a session carries no
            // registry to resolve it against) and no `Bound::User` can reach
            // here; the obligations it records are scratch, the same bypass
            // `structs`/`enums` already follow.
            &mut check::TraitCtx::scratch(&mut Vec::new()),
            // P7.S3k: an empty callee registry, so a session line calling
            // another polymorphic word still gets `unknown word` rather than
            // grounding. Deliberate: REPL lowering resolves an instantiation
            // through its own per-generation store (`Session::poly_words`),
            // which nothing composes a cross-call's substitution into, so
            // admitting the call here would check clean and then mis-lower.
            &mut check::CrossCtx {
                env: &HashMap::new(),
                calls: &mut Vec::new(),
            },
            None,
        )?;

        // R3/R7/X3: a body resolving to two or more concrete outputs, or an
        // output row variable, is a clean located deferral. REPL lowering
        // interns no return bundle (`word_ret_ty`'s first-output-only
        // fallback), so lowering such an instantiation would silently drop all
        // but the first output -- the exact miscompile this slice removes. A
        // length variable sizes an array *within* one output slot and never
        // changes the output count, so it is not part of the trigger.
        if sig.outputs.len() >= 2 || sig.row_out.is_some() {
            let span = word_span(&word);
            let locator = if span == Span::default() {
                String::new()
            } else {
                format!(" (line {}, col {})", span.line, span.col)
            };
            return Err(format!(
                "error: polymorphic word `{}`{locator} resolves to {} outputs, which is not yet supported at the REPL\n  a REPL instantiation returning more than one value needs a return bundle, which is deferred to a later slice",
                word.name,
                sig.outputs.len()
            ));
        }

        let name = word.name.clone();
        let generation = self.next_shared_generation(&name);
        // R4/D3: freeze the callee-name -> mangled-symbol map at this defining
        // line, so an instantiation of this word binds its callees against the
        // generations visible now, not the instantiating line's.
        let resolver: HashMap<String, String> = self
            .env
            .iter()
            .map(|(callee, entry)| (callee.clone(), entry.symbol.clone()))
            .collect();
        // D3: freeze the callee arity map from the same defining-line env the
        // resolver is captured from. Lowering an instantiation reads callee
        // arity/return type from this, not the instantiating line's live env,
        // so a callee redefined at a different arity in between cannot make
        // the frozen-resolved call emit under the wrong ABI.
        let ir_lower_env = ir_arity_env(&env);
        // R8/R11: the name-shape stores stay mutually exclusive (D4), so
        // defining `name` as poly evicts any prior ordinary *and* combinator
        // entry for it (combinator dispatch runs first, so a stale combinator
        // entry would otherwise win).
        self.env.remove(&name);
        self.combinators.remove(&name);
        self.poly_words.insert(
            name.clone(),
            PolyWordEntry {
                generation,
                word,
                resolver,
                ir_lower_env,
                builtin_overloads,
            },
        );
        writeln!(writer, "defined {name}").map_err(|e| format!("writing stdout: {e}"))?;
        Ok(())
    }

    /// R6-R10 (Slice 6c): accept a quotation-taking word (a combinator, mono or
    /// poly) at a session line and retain it as raw terms, minting no `.so`, no
    /// symbol, and no generation (D1/D3). A combinator has no compile event of
    /// its own to freeze against: it is spliced, fresh, at every later call
    /// site under that site's own live env, so retention is plumbing (the
    /// session store), not a freezing mechanism (contrast the slice-2 poly
    /// resolver snapshot).
    fn eval_combinator_def(
        &mut self,
        word: WordDef,
        writer: &mut impl Write,
    ) -> Result<(), String> {
        let name = word.name.clone();
        // R7: build the checker view *including the definee itself* (any prior
        // same-name entry replaced), mirroring native's `collect_combinators`,
        // which contains the word being checked -- so a self-reference or
        // self-tail call in the body dispatches through the inline path, not an
        // unknown word. Built from references (no clone: `WordDef` is not
        // `Clone`): the view borrows every stored combinator plus the local
        // definee, which outlives the check calls below.
        let mut combinators = checker_combinators(&self.combinators);
        combinators.insert(name.clone(), vec![check::combinator_of(&word)]);
        // R8: reject a cycle formed *across lines* (define `a`; define `b`
        // calling `a`; redefine `a` calling `b`) as the same located
        // `combinator_cycle_error`, while a self-*tail* edge stays permitted
        // (6b D5). Run before storing, so a rejected def leaves the store
        // untouched.
        check::check_combinator_cycles(&combinators)?;
        // The definee's own name is dropped from the concrete/poly envs so its
        // body's calls resolve through the combinator view (dispatched first),
        // not a stale prior entry the redefinition is about to evict.
        let mut env = self.typed_env();
        env.remove(&name);
        let mut poly_env = self.poly_env();
        poly_env.remove(&name);
        // R9: body check, branching on shape but storing into the one store.
        if let Some(sig) = word.poly.as_deref() {
            // A polymorphic combinator (`filter`/`while` shape) is checked
            // standalone, *not* via `eval_poly_def`: it is spliced inline and
            // never lowered to a bundle-returning `IrFunc`, so `eval_poly_def`'s
            // `>= 2`-outputs deferral (which `filter`'s two outputs would trip)
            // must not fire.
            check::check_poly_combinator_repl(
                &word,
                sig,
                &self.enums,
                &env,
                &mut self.arrays,
                &mut self.owned_cells,
                &mut self.refs,
                &mut self.slices,
                &self.structs,
                &poly_env,
                &combinators,
            )?;
        } else {
            // A monomorphic combinator: `check_def` already handles it
            // identically to any word (the instantiation records it returns are
            // scratch -- the combinator mints no `IrFunc`, R20).
            check::check_def(
                &word,
                &self.enums,
                &env,
                &mut self.arrays,
                &mut self.owned_cells,
                &mut self.refs,
                &mut self.slices,
                &self.structs,
                &poly_env,
                &combinators,
            )?;
        }
        // R10/R11: commit. No lowering, `.so`, symbol, or generation (D3). The
        // two rival name-shape stores are evicted so combinator dispatch (which
        // runs first, `check.rs`) can never be shadowed by a stale entry (D4).
        // No `arrays`/`owned_cells`/`refs` rows are purged: those rows are
        // positionally stable and never revisited, so a stale row is inert,
        // exactly as for an ordinary redefinition.
        self.env.remove(&name);
        self.poly_words.remove(&name);
        self.combinators.insert(name.clone(), word);
        writeln!(writer, "defined {name}").map_err(|e| format!("writing stdout: {e}"))?;
        Ok(())
    }

    fn eval_def(&mut self, word: WordDef, writer: &mut impl Write) -> Result<(), String> {
        // R7a (item 2): a quotation type in a word's *output* row never
        // reaches the native `unreachable!` because the native `check` audits
        // it; the REPL must run the same
        // per-word audit, before the R6 combinator route below (so a quotation
        // in a non-input position is still rejected). A poly word's effect is
        // empty, so its output-position check runs on the poly path.
        check::audit_word_quotation_positions(&word, &self.structs, &self.enums, &self.arrays)?;
        // P7.S3h (phase 2): before the combinator retention route below, which
        // is where an `owning` parameter would otherwise land -- it makes the
        // word a combinator, so the session keeps its body and re-splices it,
        // and the splice route compares a caller's literal on the
        // inline-versus-ordinary axis only, not plain-versus-owning. Without
        // this line the session accepts `: f inline ( owning [ -- ] -- )` and
        // runs `[ 1 . ] f`, admitting a plain literal into an owning slot. The
        // ordinary-parameter shape is declined below either way.
        check::reject_owning_quotation_declarations(&word)?;
        // Slice 11 (R3): the same per-word `inline` rejections native `check`
        // runs as a pre-pass, at the REPL's own per-word gate -- the poly half
        // in particular, which the retention route below would otherwise carry
        // into `check_poly_combinator_repl` as a legitimate poly combinator.
        check::check_inline_declaration(&word)?;
        // Slice 12 (R-B2): the same missing-`inline`-on-a-`~`-parameter gate
        // native `check` runs in its pre-pass, run here too so the REPL never
        // accepts a `~[ ... ]` parameter without `inline`.
        check::check_inline_quotation_requires_inline(&word)?;
        // R6 (Slice 6c): a combinator is *retained* rather than R23-rejected.
        // It routes here (both mono and poly, D2), skipping lowering entirely
        // (D3): the session keeps its body as raw terms and re-splices it,
        // fresh, at every later call site under that site's own live env,
        // which is what the inliner needs (R20). Gated on `is_combinator`
        // (R-D5), the one predicate the batch compiler recognizes a splice by,
        // so REPL retention cannot diverge from it -- in particular a declared
        // `inline` word that takes no quotation at all must not fall through to
        // the ordinary lowering path below and mint a `.so` and a symbol, which
        // is exactly the silent fall-back to a real call D2 forbids.
        if check::is_combinator(&word) {
            return self.eval_combinator_def(word, writer);
        }
        // Slice 12 (R-D5/E4): the remaining quotation-taking shape is a word
        // with an ordinary `[ ... ]` parameter, which lowers to a real call
        // (part D). The REPL declines it: the `(code, env)` ABI across a
        // `dlopen` boundary -- the quotation built on a later line, its code
        // pointer resolved through `RTLD_GLOBAL` -- is untested surface, so
        // this is a scope boundary, not a mis-lowering waiting to happen.
        if check::word_declares_quotation_parameter(&word) {
            let display = crate::resolve::demangle_word(&word.name);
            return Err(format!(
                "error: word `{display}` takes a `[ ... ]` quotation parameter and lowers to a real call, which is not supported in the REPL (line {}, col {})",
                word.span.line, word.span.col
            ));
        }
        // R3 (Slice 2): a polymorphic word's signature lives entirely in
        // `word.poly` (`word.effect` is empty), so it takes a wholly separate
        // acceptance path; the concrete path below would mis-check its body
        // against a zero-arity `Sig` derived from that empty effect.
        if word.poly.is_some() {
            return self.eval_poly_def(word, writer);
        }

        let name = word.name.clone();
        let sig = check::sig_of(&word.effect);

        let mut env = self.typed_env();
        // R5 (Slice 2): thread the session poly-env so this defined word's own
        // body can call a retained polymorphic word; the relayed instantiation
        // table drives the per-site lowering below (R7).
        // R8: the definee's own name is removed so that redefining a name from
        // poly to ordinary binds its self-calls to this new ordinary word, not
        // the stale poly entry this line is about to evict (the two stores are
        // mutually exclusive per name).
        let mut poly_env = self.poly_env();
        poly_env.remove(&name);
        // R4 (Slice 6c): this ordinary word's body may call a retained
        // combinator; thread the session's inline view so it inlines exactly as
        // native inlines one drawn from `module.words`.
        let combinators = checker_combinators(&self.combinators);
        // Item 3: `overloads` is this body's own resolved overload-dispatch
        // call sites, threaded into `ir::lower_word` below so it dispatches an
        // overloaded call exactly as a native word body does, rather than
        // silently mis-lowering through the name-directed builtin arm.
        let (insts, overloads, fields, variant_fields) = check::check_def(
            &word,
            &self.enums,
            &env,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &mut self.slices,
            &self.structs,
            &poly_env,
            &combinators,
        )?;
        let poly_arities = self.poly_arities();

        // R8: one past whichever of the ordinary env or the poly store holds
        // the name (a shared per-name counter), so redefining across the
        // mono<->poly boundary can never remint a resident generation's symbol
        // under `RTLD_GLOBAL`.
        let generation = self.next_shared_generation(&name);
        let symbol = mangled_symbol(&name, generation);

        // Self-recursive calls in the body must bind this new generation, not
        // whatever generation `env` still holds for `name`; seed the definee's
        // own signature so ir derives its return type. The arity map for ir is
        // derived from the typed env (RK2): ir needs only counts + output type.
        env.insert(
            name.clone(),
            vec![check::Overload {
                sig: sig.clone(),
                symbol: name.clone(),
            }],
        );
        let ir_lower_env = ir_arity_env(&env);
        let (mut structs, mut enums, arrays, mut cells, refs) = ir::build_registries(
            &self.structs,
            &self.enums,
            &self.arrays,
            &self.owned_cells,
            &self.refs,
        );
        self.apply_drop_generations(&mut structs, &mut enums, &mut cells);
        let slices = ir::build_slices(&self.slices, &structs, &enums, &arrays);
        let regs = ir::Registries {
            structs: &structs,
            enums: &enums,
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
            slices: &slices,
            statics: ir::empty_statics(),
        };
        let mut funcs = {
            let resolve = resolver_with_override(&self.env, &name, &symbol);
            // R7 (Slice 2): thread the instantiation table + poly-arity map so
            // a call to a retained polymorphic word inside this body lowers to
            // its per-site symbol via `lower_poly_call`.
            // R9: element 0 is this word; any quotation literal it materialized
            // at a boundary follows, each its own `IrFunc`.
            let mut funcs = ir::lower_word(
                &word,
                &ir_lower_env,
                &resolve,
                regs,
                &insts,
                &overloads,
                &fields,
                &variant_fields,
                &poly_arities,
                &combinator_bodies(&self.combinators),
                ir::empty_splice_records(),
                ir::empty_splice_trait_calls(),
            );
            funcs[0].name = symbol.clone();
            // R12: this module must carry its own struct/enum destructors
            // (they are not emitted elsewhere in the REPL, unlike the build
            // path's single shared module), or `drop` on a linear struct/enum
            // dies at `dlopen` with an undefined `sooth_struct_drop_N`/
            // `sooth_enum_drop_N`.
            // R11.3: an overridden struct's destructor is *not* re-emitted
            // here -- its symbol is pinned to the epoch it was defined at and
            // resolves through `RTLD_GLOBAL` to that line's module; every
            // other linear struct gets generic glue as before.
            funcs.extend(ir::synthesize_aggregate_destructors(
                &ir_lower_env,
                &resolve,
                regs,
                &self.drop_override_bodies(None),
                // Every override here is `AlreadyLoaded` (lowered at its own
                // defining line), so no body reaches this call and no
                // projection needs resolving.
                ir::empty_resolved_fields(),
                ir::empty_resolved_variant_fields(),
                &combinator_bodies(&self.combinators),
            ));
            funcs
        };
        // R7 (Slice 2, D2): lower each not-yet-exported instantiation this
        // body recorded into this module, against the frozen snapshot resolver.
        funcs.extend(self.emit_instantiations(&insts, regs));

        let quot_sigs =
            ir::collect_quot_sigs(&funcs, &structs.layouts, &enums.layouts, &arrays.layouts);
        let ssa = backend::qbe::emit(&IrModule {
            funcs,
            structs: structs.layouts,
            enums: enums.layouts,
            arrays: arrays.layouts,
            quot_sigs,
            statics: Vec::new(),
        })?;
        let dir = driver::tempfile_dir()?;
        let so_path = dir.join(format!("{name}_gen{generation}.so"));
        driver::compile_so(&ssa, &so_path)?;
        let lib = Library::open(&so_path)?;

        // Only commit on success: env stays untouched on any earlier failure.
        self.libs.push(lib);
        // R8/R11: an ordinary (re)definition evicts any prior poly *and*
        // combinator entry for the name (D4), so a name lives in exactly one of
        // the three stores at a time and a later call never has to arbitrate
        // between them. Combinator dispatch runs first (`check.rs`), so a stale
        // combinator entry would otherwise silently win.
        self.poly_words.remove(&name);
        self.combinators.remove(&name);
        self.env.insert(
            name.clone(),
            WordEntry {
                sig,
                generation,
                symbol,
            },
        );
        writeln!(writer, "defined {name}").map_err(|e| format!("writing stdout: {e}"))?;
        Ok(())
    }

    fn eval_expr(&mut self, terms: &[Term], writer: &mut impl Write) -> Result<(), String> {
        let (structs, enums, arrays, cells) = self.run_terms(terms, writer)?;
        let live_cells = self.top / 8;
        // D2/R13: the rich formatter is a tty-only affordance (`rich_stack`,
        // set once by `run_tty`); the piped path always renders through the
        // plain `format_stack`, keeping every piped golden byte-for-byte (F2).
        let line = if self.rich_stack {
            format_stack_rich(
                &self.buf[..live_cells],
                &self.types,
                &structs.layouts,
                &enums.layouts,
                &arrays.layouts,
                &cells.payload,
            )
        } else {
            format_stack(
                &self.buf[..live_cells],
                &self.types,
                &structs.layouts,
                &enums.layouts,
                &arrays.layouts,
                self.bool_enum,
            )
        };
        writeln!(writer, "{line}").map_err(|e| format!("writing stdout: {e}"))
    }

    /// The end of the REPL-main scope: dispose every linear value still on the
    /// residual stack, top first, since a live session's body is never
    /// provably complete for the ordinary forgotten-disposal check. Word
    /// definitions entered at the REPL keep the strict rule. Disposal goes
    /// through the ordinary expression path (a synthesized run of `drop`
    /// terms), so it reuses the same drop glue a compiled program does
    /// instead of a second, drifting implementation.
    fn dispose_residual(&mut self, writer: &mut impl Write) -> Result<(), String> {
        let Some(deepest) = self
            .types
            .iter()
            .position(|ty| check::is_linear(*ty, &self.structs, &self.enums, &self.arrays))
        else {
            return Ok(());
        };
        let terms: Vec<Term> = (deepest..self.types.len())
            .map(|_| Term {
                kind: TermKind::Call("drop".to_string(), Vec::new()),
                span: Span::default(),
            })
            .collect();
        self.run_terms(&terms, writer).map(|_| ())
    }

    /// Compile and run one bare term sequence against the carried stack,
    /// committing the resulting stack state. Returns the aggregate registries it
    /// built, so the caller can render the residual stack from the same
    /// layouts.
    fn run_terms(
        &mut self,
        terms: &[Term],
        writer: &mut impl Write,
    ) -> Result<(ir::Structs, ir::Enums, ir::Arrays, ir::Cells), String> {
        let env = self.typed_env();
        let entry_depth = self.types.len();
        // R5 (Slice 2): thread the session poly-env so a bare line can call a
        // retained polymorphic word; the relayed instantiation table drives
        // the per-site lowering below (R7).
        let poly_env = self.poly_env();
        // R4 (Slice 6c): a bare line may call a retained combinator; thread the
        // session's inline view so it inlines like native's `module.words` one.
        let combinators = checker_combinators(&self.combinators);
        let (net_stack, insts, line_overloads, line_fields, line_variant_fields) =
            check::infer_line(
                terms,
                &self.types,
                &env,
                &mut self.arrays,
                &mut self.owned_cells,
                &mut self.refs,
                &mut self.slices,
                &self.structs,
                &self.enums,
                &poly_env,
                &combinators,
            )?;
        let net_depth = net_stack.len();

        let ir_lower_env = ir_arity_env(&env);
        let poly_arities = self.poly_arities();
        // R5 (Slice 6c): the combinator-bodies view for this line's lowering,
        // so a call to a retained combinator splices in place rather than
        // lowering to an `Instr::Call` to a never-minted symbol.
        let bodies = combinator_bodies(&self.combinators);

        self.seq += 1;
        let seq = self.seq;
        let (mut structs, mut enums, arrays, mut cells, refs) = ir::build_registries(
            &self.structs,
            &self.enums,
            &self.arrays,
            &self.owned_cells,
            &self.refs,
        );
        self.apply_drop_generations(&mut structs, &mut enums, &mut cells);
        let slices = ir::build_slices(&self.slices, &structs, &enums, &arrays);
        let regs = ir::Registries {
            structs: &structs,
            enums: &enums,
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
            slices: &slices,
            statics: ir::empty_statics(),
        };
        let (func, quot_funcs, m, out_bytes, aggregate_destructors) = {
            let resolve = resolver_for(&self.env);
            let (func, quot_funcs, m, out_bytes) = ir::lower_line(
                seq,
                terms,
                entry_depth,
                &self.types,
                &ir_lower_env,
                &resolve,
                regs,
                &insts,
                &line_overloads,
                &line_fields,
                &line_variant_fields,
                &poly_arities,
                &bodies,
            );
            // R12: this line's module must carry its own struct/enum
            // destructors, or `drop` on a linear struct/enum dies at `dlopen`
            // with an undefined `sooth_struct_drop_N`/`sooth_enum_drop_N`.
            let aggregate_destructors = ir::synthesize_aggregate_destructors(
                &ir_lower_env,
                &resolve,
                regs,
                &self.drop_override_bodies(None),
                ir::empty_resolved_fields(),
                ir::empty_resolved_variant_fields(),
                &bodies,
            );
            (func, quot_funcs, m, out_bytes, aggregate_destructors)
        };
        // `m` (the wrapper's emitted output slot count) and `net_depth` (the
        // checker's independently-inferred net effect) are the same depth
        // simulation and must always agree; `out_bytes` is what the wrapper
        // actually writes and sizes the buffer. Assert the checker agrees
        // rather than trusting two separately-computed counts to stay in sync
        // as codegen evolves.
        debug_assert_eq!(
            m, net_depth,
            "lowering emitted a different depth than the checker inferred"
        );

        let mut funcs = vec![func];
        // R9: the line's materialized quotation callees.
        funcs.extend(quot_funcs);
        funcs.extend(aggregate_destructors);
        // R7 (Slice 2, D2): lower each not-yet-exported instantiation this line
        // recorded into this module, against each poly word's frozen snapshot
        // resolver; an already-exported symbol emits nothing (trace B dedup).
        funcs.extend(self.emit_instantiations(&insts, regs));
        let quot_sigs =
            ir::collect_quot_sigs(&funcs, &structs.layouts, &enums.layouts, &arrays.layouts);
        let ssa = backend::qbe::emit(&IrModule {
            funcs,
            structs: structs.layouts.clone(),
            enums: enums.layouts.clone(),
            arrays: arrays.layouts.clone(),
            quot_sigs,
            statics: Vec::new(),
        })?;
        let dir = driver::tempfile_dir()?;
        let so_path = dir.join(format!("line{seq}.so"));
        driver::compile_so(&ssa, &so_path)?;
        let lib = Library::open(&so_path)?;
        let sym = lib.symbol(&format!("sooth_line_{seq}"))?;
        // SAFETY: emitted as `export function l $sooth_line_{seq}(l %v0, l %v1)`,
        // i.e. a C-ABI `(u64, u64) -> u64` function on this 64-bit target,
        // matching the `(*mut u8, usize) -> usize` transmute below.
        let wrapper: extern "C" fn(*mut u8, usize) -> usize = unsafe { std::mem::transmute(sym) };

        // Size the buffer (in 8-byte cells) to cover the wrapper's output
        // bytes; it already covers the entry bytes (`self.top`) from the line
        // that produced them. `out_bytes` is always a multiple of 8
        // (`carried_slot_bytes` rounds each slot up), so `div_ceil` is exact.
        let out_cells = out_bytes.div_ceil(8);
        if self.buf.len() < out_cells {
            self.buf.resize(out_cells, 0);
        }
        // Flush any host-buffered stdout first so it interleaves deterministically
        // with the loaded code's own `printf` (a separate C stdio buffer).
        writer
            .flush()
            .map_err(|e| format!("flushing stdout: {e}"))?;
        let base_ptr = self.buf.as_mut_ptr() as *mut u8;
        // SAFETY: `base_ptr` points into a `Vec<i64>` grown to at least
        // `out_cells` cells (`out_bytes` bytes); `self.top` is the live byte
        // length on entry, a multiple of 8 and `<= self.buf.len() * 8`. The
        // wrapper only reads/writes within `[0, max(self.top, out_bytes))`.
        let new_top = wrapper(base_ptr, self.top);

        // Flush the loaded code's C stdio buffer so its `.`/printf output lands
        // on the fd before the host writes the residual-stack line.
        // SAFETY: fflush(NULL) flushes all open C streams; always sound.
        unsafe { fflush(std::ptr::null_mut()) };
        self.top = new_top;
        self.types = net_stack;
        self.libs.push(lib);

        Ok((structs, enums, arrays, cells))
    }
}

impl Default for Session {
    fn default() -> Session {
        Session::new()
    }
}

/// End the session: run the residual stack's destructors (the REPL-main scope
/// end), reporting a failure the way any other line's failure is reported.
fn end_session(session: &mut Session, writer: &mut impl Write) -> Result<(), String> {
    if let Err(e) = session.dispose_residual(writer) {
        writeln!(writer, "{e}").map_err(|e| format!("writing stdout: {e}"))?;
    }
    Ok(())
}

/// What the shared dispatch helper decided about a committed line.
enum Dispatch {
    Continue,
    Quit,
}

/// R9: whether a committed-so-far token stream is a complete logical line or
/// still has an open construct. A balance count, not a full parse: it must
/// never reject a well-formed prefix, only defer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    NeedMore,
}

/// `NeedMore` while a `:` word definition or `type:` declaration is open
/// (opened by the leading `Word(":")`/`Word("type:")`, closed by
/// `Token::Semicolon`) or while `[`/`]` brackets are unbalanced. `Complete`
/// otherwise, including on an over-closed bracket (a real error, left for the
/// parser to report rather than buffered forever).
///
/// Slice 6h: a `Token::Semicolon` only closes `open_def` at bracket depth
/// `0`. An array constructor's own `;` (`[ i64 ; 4 ]`) sits inside a bracket,
/// so without this guard a first REPL line like `: f ( -- ) [ i64 ; 4 ]`
/// (a word definition containing a constructor, no closing `;` of its own
/// yet) would be judged `Complete` on the constructor's `;` and submitted
/// unterminated.
pub fn input_is_complete(tokens: &[Token]) -> Completeness {
    let mut bracket_depth: i32 = 0;
    let mut open_def = false;
    for tok in tokens {
        match tok {
            Token::LBracket => bracket_depth += 1,
            Token::RBracket => bracket_depth -= 1,
            Token::Word(w) if w == ":" || w == "type:" => open_def = true,
            Token::Semicolon if bracket_depth == 0 => open_def = false,
            _ => {}
        }
    }
    if bracket_depth > 0 || open_def {
        Completeness::NeedMore
    } else {
        Completeness::Complete
    }
}

/// Lex `text` and apply `input_is_complete` (R10). A lex error is treated as
/// complete: a permanent lexical error (e.g. a bad character) should surface
/// immediately rather than buffer forever waiting for input that can never
/// close it.
pub fn text_is_complete(text: &str) -> bool {
    match lexer::lex(text) {
        Ok(tokens) => {
            let toks: Vec<Token> = tokens.into_iter().map(|(t, _)| t).collect();
            input_is_complete(&toks) == Completeness::Complete
        }
        Err(_) => true,
    }
}

/// D1: the single point every committed logical line funnels through, on both
/// the piped and the tty path, so the call sequence the piped goldens observe
/// is preserved by construction. Blank lines are skipped, `:quit` requests a
/// clean exit, and any stage error prints the diagnostic without mutating
/// session state.
/// `name ( ins -- outs )` for a defined word's signature (R19), mirroring
/// `check::effect_str`'s notation but over a resolved `Sig` rather than a
/// declared `StackEffect`.
fn sig_str(sig: &Sig) -> String {
    let ins: Vec<String> = sig.inputs.iter().map(|t| t.to_string()).collect();
    let outs: Vec<String> = sig.outputs.iter().map(|t| t.to_string()).collect();
    let mut parts = vec!["--".to_string()];
    if !outs.is_empty() {
        parts.push(outs.join(" "));
    }
    if !ins.is_empty() {
        parts.insert(0, ins.join(" "));
    }
    format!("( {} )", parts.join(" "))
}

/// R19: `sig_str`'s polymorphic analogue for `:words`, reusing `check`'s own
/// `PolyType` renderer (`poly_type_str`) so a type variable prints its
/// declared surface spelling (`'T`) rather than a bare index. A row variable
/// prints as `..name`; the REPL only ever retains a poly word with `row_out`
/// `None` (`eval_poly_def`'s multi-output gate), but `row_in` alone is a
/// legal signature, so both are handled.
fn poly_sig_str(sig: &PolySig) -> String {
    let mut ins: Vec<String> = Vec::new();
    if let Some(r) = sig.row_in {
        ins.push(format!("..{}", sig.row_var_names[r as usize]));
    }
    ins.extend(sig.inputs.iter().map(|t| check::poly_type_str(t, sig)));
    let mut outs: Vec<String> = Vec::new();
    if let Some(r) = sig.row_out {
        outs.push(format!("..{}", sig.row_var_names[r as usize]));
    }
    outs.extend(sig.outputs.iter().map(|t| check::poly_type_str(t, sig)));
    let mut parts = vec!["--".to_string()];
    if !outs.is_empty() {
        parts.push(outs.join(" "));
    }
    if !ins.is_empty() {
        parts.insert(0, ins.join(" "));
    }
    format!("( {} )", parts.join(" "))
}

/// `( before -- after )` for a `:type` line's checked effect (R20): the stack
/// types on entry and the resulting types after the checked expression.
fn type_effect_str(before: &[Type], after: &[Type]) -> String {
    let ins: Vec<String> = before.iter().map(|t| t.to_string()).collect();
    let outs: Vec<String> = after.iter().map(|t| t.to_string()).collect();
    let mut parts = vec!["--".to_string()];
    if !outs.is_empty() {
        parts.push(outs.join(" "));
    }
    if !ins.is_empty() {
        parts.insert(0, ins.join(" "));
    }
    format!("( {} )", parts.join(" "))
}

/// R18: `:help`'s listing, one line per meta-command.
const HELP_LINES: [&str; 6] = [
    ":help              list the meta-commands",
    ":words             list defined words with their signatures",
    ":type <line>       check <line> against the current stack, print its effect, run nothing",
    ":stack             print the residual stack",
    ":clear             dispose the residual stack, then reset the session",
    ":quit              end the session",
];

/// D3/R17: recognize a meta-command in the shared dispatch helper, before
/// `eval_line`, so `:help`/`:words`/`:type`/`:stack`/`:clear` work piped
/// (golden-testable) and interactively alike. `None` if `line` is not a
/// meta-command (an ordinary Sooth line, dispatched by the caller).
fn dispatch_meta(
    session: &mut Session,
    line: &str,
    writer: &mut impl Write,
) -> Option<Result<(), String>> {
    let (cmd, rest) = match line.split_once(char::is_whitespace) {
        Some((cmd, rest)) => (cmd, rest.trim()),
        None => (line, ""),
    };
    let result = match cmd {
        ":help" => (|| {
            for line in HELP_LINES {
                writeln!(writer, "{line}").map_err(|e| format!("writing stdout: {e}"))?;
            }
            Ok(())
        })(),
        ":words" => (|| {
            for line in session.words_listing() {
                writeln!(writer, "{line}").map_err(|e| format!("writing stdout: {e}"))?;
            }
            Ok(())
        })(),
        ":type" => session.eval_type(rest, writer),
        ":stack" => writeln!(writer, "{}", session.render_stack())
            .map_err(|e| format!("writing stdout: {e}")),
        ":clear" => session.clear(writer),
        _ => return None,
    };
    Some(result)
}

fn dispatch_line(
    session: &mut Session,
    line: &str,
    writer: &mut impl Write,
) -> Result<Dispatch, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(Dispatch::Continue);
    }
    if trimmed == ":quit" {
        return Ok(Dispatch::Quit);
    }
    if let Some(result) = dispatch_meta(session, trimmed, writer) {
        if let Err(e) = result {
            writeln!(writer, "{e}").map_err(|e| format!("writing stdout: {e}"))?;
        }
        return Ok(Dispatch::Continue);
    }
    if let Err(e) = session.eval_line(trimmed, writer) {
        writeln!(writer, "{e}").map_err(|e| format!("writing stdout: {e}"))?;
    }
    Ok(Dispatch::Continue)
}

/// The read-eval-print loop. D1: branch once, at entry, on whether stdin is a
/// terminal. Not a tty -> the piped `read_line` loop, byte-for-byte as today
/// (F2). A tty -> the raw-mode line editor. Both funnel each committed line
/// through `dispatch_line`.
pub fn run(reader: impl BufRead, mut writer: impl Write) -> Result<(), String> {
    let mut session = Session::new();
    if std::io::stdin().is_terminal() {
        run_tty(&mut session, &mut writer)
    } else {
        run_piped(&mut session, reader, &mut writer)
    }
}

/// The piped (non-tty) path: identical in shape and order to the pre-editor
/// REPL, only routed through the shared `dispatch_line` (F2/D1).
fn run_piped(
    session: &mut Session,
    mut reader: impl BufRead,
    writer: &mut impl Write,
) -> Result<(), String> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("reading stdin: {e}"))?;
        if n == 0 {
            return end_session(session, writer);
        }
        match dispatch_line(session, &line, writer)? {
            Dispatch::Quit => return end_session(session, writer),
            Dispatch::Continue => {}
        }
    }
}

/// The primary prompt (tty only; never written on the piped path, F2).
const PROMPT: &str = "sooth> ";

/// The continuation prompt (tty only), shown while a multi-line definition
/// or bracket is still open (R10).
const CONTINUATION_PROMPT: &str = "  ... ";

/// The pure action-to-outcome mapping `run_tty`'s loop acts on: `Commit`
/// dispatches the line, `Abort` (Ctrl-C) just continues the loop without
/// dispatching or quitting, and `Eof` (Ctrl-D on an empty line, or a closed
/// stdin) quits. Pulled out of `run_tty` -- which also does real termios/fd
/// I/O and so cannot run under `cargo test` -- so the one decision that
/// distinguishes "Ctrl-C aborts the line" from "Ctrl-C ends the session" is
/// unit-testable on its own, independent of a real terminal.
#[derive(Debug, PartialEq)]
enum LoopStep {
    Continue,
    Dispatch(String),
    Quit,
}

fn loop_step(action: editor::Action) -> LoopStep {
    match action {
        editor::Action::Commit(line) => LoopStep::Dispatch(line),
        editor::Action::Abort => LoopStep::Continue,
        editor::Action::Eof => LoopStep::Quit,
    }
}

/// The tty path: put stdin in raw mode (restored on any exit by the guard's
/// `Drop`, D5), then read bytes into the line editor, dispatching each
/// committed line through `dispatch_line`. Ctrl-C abandons the line (and any
/// pending multi-line buffer, R11); Ctrl-D on an empty line and a closed
/// stdin both end the session.
fn run_tty(session: &mut Session, writer: &mut impl Write) -> Result<(), String> {
    session.enable_rich_stack_rendering();
    // A failed `tcgetattr`/`tcsetattr` here means there is no sound cooked
    // state to restore later; propagate rather than proceed into raw mode on
    // a guard that would write back garbage termios on `Drop`.
    let _guard = editor::raw_mode_stdin().map_err(|e| format!("entering raw mode: {e}"))?;
    let mut ed = editor::Editor::new(
        PROMPT,
        CONTINUATION_PROMPT,
        editor::History::load(),
        text_is_complete,
    );
    let w = |r: std::io::Result<()>| r.map_err(|e| format!("writing stdout: {e}"));
    w(ed.redraw(writer))?;
    w(writer.flush())?;
    loop {
        let byte = editor::read_stdin_byte().map_err(|e| format!("reading stdin: {e}"))?;
        let Some(byte) = byte else {
            w(writer.write_all(b"\r\n"))?;
            return end_session(session, writer);
        };
        let action = ed
            .push_byte(byte, writer)
            .map_err(|e| format!("writing stdout: {e}"))?;
        // Every byte redraws something (an inserted char, a moved cursor, a
        // history recall) even when it doesn't complete an Action; stdout is
        // line-buffered and `redraw` never writes a `\n`, so without an
        // explicit flush here each keystroke sits invisible until the next
        // `\r\n` (e.g. Enter) flushes the whole backlog at once.
        w(writer.flush())?;
        let Some(action) = action else { continue };
        match loop_step(action) {
            LoopStep::Dispatch(cmd) => {
                w(writer.write_all(b"\r\n"))?;
                match dispatch_line(session, &cmd, writer)? {
                    Dispatch::Quit => return end_session(session, writer),
                    Dispatch::Continue => {}
                }
                // R23: a defined word is completable right away.
                ed.set_words(session.word_names());
                w(ed.redraw(writer))?;
            }
            LoopStep::Continue => {
                w(writer.write_all(b"\r\n"))?;
                w(ed.redraw(writer))?;
            }
            LoopStep::Quit => {
                w(writer.write_all(b"\r\n"))?;
                return end_session(session, writer);
            }
        }
        w(writer.flush())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend, check, driver, ir, lexer, parser};

    #[test]
    fn compiled_word_is_dlsymable_and_callable() {
        let src = ": sq ( i64 -- i64 ) | n | n n mul ;";
        let tokens = lexer::lex(src).unwrap();
        let mut module = parser::parse(&tokens).unwrap();
        check::check(&mut module).unwrap();
        let ir = ir::lower(&module).unwrap();
        let ssa = backend::qbe::emit(&ir).unwrap();

        let dir = driver::tempfile_dir().unwrap();
        let so = dir.join("libsq.so");
        driver::compile_so(&ssa, &so).expect("compile_so should succeed");

        let lib = Library::open(&so).expect("dlopen should succeed");
        let sym = lib.symbol("sq").expect("dlsym should find the word");
        // SAFETY: `sq` was emitted as `export function l $sq(l %v0)`, i.e. a
        // C-ABI `l`-taking, `l`-returning function on this 64-bit target.
        let sq: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(sym) };
        assert_eq!(sq(5), 25);
    }

    /// The Ctrl-C-doesn't-exit-the-process decision, guarded directly rather
    /// than only through the editor's `Action::Abort` (which has no way to
    /// exit a process at all): `run_tty`'s match on `loop_step` is the one
    /// place that decision lives, so this asserts all three arms rather than
    /// letting a swapped `Abort`/`Eof` mapping ship undetected.
    #[test]
    fn loop_step_maps_commit_abort_eof_expected() {
        assert_eq!(
            loop_step(editor::Action::Commit("1 2 add".to_string())),
            LoopStep::Dispatch("1 2 add".to_string())
        );
        assert_eq!(loop_step(editor::Action::Abort), LoopStep::Continue);
        assert_eq!(loop_step(editor::Action::Eof), LoopStep::Quit);
    }

    #[test]
    fn continuation_unclosed_def_needs_more() {
        let tokens = lexer::lex(": sq ( i64 -- i64 )").unwrap();
        let toks: Vec<Token> = tokens.into_iter().map(|(t, _)| t).collect();
        assert_eq!(input_is_complete(&toks), Completeness::NeedMore);
    }

    #[test]
    fn continuation_unclosed_typedef_needs_more() {
        let tokens = lexer::lex("type: Vec2 x i64 y i64").unwrap();
        let toks: Vec<Token> = tokens.into_iter().map(|(t, _)| t).collect();
        assert_eq!(input_is_complete(&toks), Completeness::NeedMore);
    }

    #[test]
    fn continuation_unbalanced_bracket_needs_more() {
        let tokens = lexer::lex("[ i64 4").unwrap();
        let toks: Vec<Token> = tokens.into_iter().map(|(t, _)| t).collect();
        assert_eq!(input_is_complete(&toks), Completeness::NeedMore);
    }

    #[test]
    fn continuation_balanced_line_is_complete() {
        let tokens = lexer::lex(": sq ( i64 -- i64 ) dup mul ;").unwrap();
        let toks: Vec<Token> = tokens.into_iter().map(|(t, _)| t).collect();
        assert_eq!(input_is_complete(&toks), Completeness::Complete);

        let tokens = lexer::lex("1 2 add").unwrap();
        let toks: Vec<Token> = tokens.into_iter().map(|(t, _)| t).collect();
        assert_eq!(input_is_complete(&toks), Completeness::Complete);
    }

    #[test]
    fn repl_input_with_a_constructor_is_not_complete_until_the_definition_ends() {
        // Slice 6h: the array constructor's own `;` sits inside a bracket
        // (depth 1), so it must not close `open_def`. Without the guard, this
        // first line would be judged `Complete` on the constructor's `;` and
        // submitted with the `:` definition still open.
        let tokens = lexer::lex(": f ( -- ) [ i64 ; 4 ]").unwrap();
        let toks: Vec<Token> = tokens.into_iter().map(|(t, _)| t).collect();
        assert_eq!(input_is_complete(&toks), Completeness::NeedMore);

        let tokens = lexer::lex(": f ( -- ) [ i64 ; 4 ] drop ;").unwrap();
        let toks: Vec<Token> = tokens.into_iter().map(|(t, _)| t).collect();
        assert_eq!(input_is_complete(&toks), Completeness::Complete);
    }

    /// #24: `:type` prints the checked effect and touches no session state.
    #[test]
    fn repl_type_prints_effect_without_executing() {
        let mut session = Session::new();
        let mut out = Vec::new();
        session.eval_type("1 2 add", &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "( -- i64 )\n");
        assert!(session.types.is_empty());
        assert_eq!(session.top, 0);
    }

    /// #25 (load-bearing, mutation-tested): `:type` mentioning an array/
    /// owned-cell/ref type must not grow the session's interning registries,
    /// even though `parse_line_with_structs` interns as a side effect of
    /// parsing (the hazard R20 calls out). The probe line mentions all three:
    /// `0 4 fill` interns an array, `7 ^` an owned cell, and `&a 0 &> @` a ref
    /// (borrowing an element of the freshly-bound array `a`) -- each registry
    /// assertion below is load-bearing on its own probe term, not decorative.
    /// `.unwrap()`, not `.unwrap_or(())`: a future `eval_type` regression that
    /// makes this line stop checking must fail loudly here, not silently
    /// degrade all three assertions to a vacuous pass.
    #[test]
    fn repl_type_does_not_grow_registries() {
        let mut session = Session::new();
        let mut out = Vec::new();
        let arrays_before = session.arrays.len();
        let cells_before = session.owned_cells.len();
        let refs_before = session.refs.len();
        session
            .eval_type("0 4 fill | a | &a 0 &> @ 7 ^", &mut out)
            .unwrap();
        assert_eq!(session.arrays.len(), arrays_before);
        assert_eq!(session.owned_cells.len(), cells_before);
        assert_eq!(session.refs.len(), refs_before);
    }

    #[test]
    fn format_stack_bottom_to_top() {
        let types = vec![Type::I64, Type::I64, Type::I64];
        assert_eq!(
            format_stack(&[1, 2, 3], &types, &[], &[], &[], EnumId::from_index(0)),
            "stack: 1 2 3"
        );
    }

    #[test]
    fn format_stack_empty_is_marker() {
        assert_eq!(
            format_stack(&[], &[], &[], &[], &[], EnumId::from_index(0)),
            "stack: (empty)"
        );
    }

    #[test]
    fn format_stack_f64_slot_renders_float_not_bits() {
        // A carried `f64` displays its value, not the `i64` bit pattern (R21).
        let bits = 2.5f64.to_bits() as i64;
        assert_eq!(
            format_stack(&[bits], &[Type::F64], &[], &[], &[], EnumId::from_index(0)),
            "stack: 2.5"
        );
    }

    #[test]
    fn format_stack_f32_slot_reads_low_32_bits() {
        // An `f32` slot stores 4 bytes; display reads the low 32 bits (Q2/R21).
        let bits = 1.5f32.to_bits() as u64 as i64;
        let f32_ty = Type::from_name("f32").unwrap();
        assert_eq!(
            format_stack(&[bits], &[f32_ty], &[], &[], &[], EnumId::from_index(0)),
            "stack: 1.5"
        );
    }

    /// One `EnumLayout` under the given name, scalar or not: the layout half of
    /// the `:stack` bool arm's test, which reads `is_scalar` as well as the name.
    fn enum_layout_named(name: &'static str, is_scalar: bool) -> EnumLayout {
        EnumLayout {
            name,
            tag_offset: 0,
            tag_ty: ir::IrType::Int {
                bits: 32,
                signed: true,
            },
            payload_offset: 8,
            size: if is_scalar { 1 } else { 16 },
            align: if is_scalar { 1 } else { 8 },
            variants: vec![
                ir::VariantLayout { fields: vec![] },
                ir::VariantLayout { fields: vec![] },
            ],
            is_scalar,
            is_linear: false,
            drop_generation: None,
        }
    }

    #[test]
    fn format_stack_bool_slot_displays_as_true_or_false() {
        // Matches `.`'s print semantics: `true`/`false`, not the raw 0/1.
        let bool_ty = Type::Enum(EnumId::from_index(0), crate::ast::BOOL_TYPE_NAME);
        let layouts = vec![enum_layout_named(crate::ast::BOOL_TYPE_NAME, true)];
        assert_eq!(
            format_stack(
                &[1, 0],
                &[bool_ty, bool_ty],
                &[],
                &layouts,
                &[],
                EnumId::from_index(0)
            ),
            "stack: True False"
        );
    }

    #[test]
    fn format_stack_non_scalar_enum_named_bool_shows_the_placeholder() {
        // P7 slice 3i (R2): the arm requires a scalar layout on top of the
        // session's own id. `bool` is an ordinary declared name now, so a type
        // that merely shares it must not be read as one cell of `true`/`false`
        // -- it renders (and strides) like the aggregate it is.
        let forged = Type::Enum(EnumId::from_index(0), crate::ast::BOOL_TYPE_NAME);
        let layouts = vec![enum_layout_named(crate::ast::BOOL_TYPE_NAME, false)];
        assert_eq!(
            format_stack(
                &[1, 0, 7],
                &[forged, Type::I64],
                &[],
                &layouts,
                &[],
                EnumId::from_index(0)
            ),
            "stack: <Bool> 7"
        );
    }

    #[test]
    fn format_stack_scalar_enum_named_bool_that_is_not_the_sessions_shows_the_placeholder() {
        // P7 slice 3i (R2): an imported closure may declare its own enum
        // called `bool`, and one that happens to be a payload-free pair is a
        // *scalar* one -- so the name and the layout together still do not
        // identify the session's boolean. Only the session's own id renders as
        // `true`/`false`; a same-named, same-shaped stranger is the placeholder
        // its variant names would otherwise be misreported as.
        let stranger = Type::Enum(EnumId::from_index(1), crate::ast::BOOL_TYPE_NAME);
        let layouts = vec![
            enum_layout_named(crate::ast::BOOL_TYPE_NAME, true),
            enum_layout_named(crate::ast::BOOL_TYPE_NAME, true),
        ];
        assert_eq!(
            format_stack(&[1], &[stranger], &[], &layouts, &[], EnumId::from_index(0)),
            "stack: <Bool>"
        );
    }

    #[test]
    fn format_stack_struct_slot_shows_placeholder_and_offsets_past_it() {
        use crate::ast::StructId;
        // A 16-byte struct (two 8-byte cells) at StructId 0, then a scalar
        // slot. The struct renders as its `<Vec2>` placeholder reading no
        // field bytes, and the trailing scalar reads the cell *past* the
        // struct's two cells, not `index * 8`.
        let layouts = vec![StructLayout {
            name: "Vec2",
            size: 16,
            align: 8,
            fields: vec![],
            is_linear: false,
            has_drop_overload: false,
            bundle: false,
            drop_generation: None,
        }];
        let vec2 = Type::Struct(StructId::from_index(0), "Vec2");
        assert_eq!(
            format_stack(
                &[5, 6, 99],
                &[vec2, Type::I64],
                &layouts,
                &[],
                &[],
                EnumId::from_index(0)
            ),
            "stack: <Vec2> 99"
        );
    }

    #[test]
    fn format_stack_cell_slot_shows_placeholder_and_offsets_past_it() {
        use crate::ast::OwnedCellId;
        // A cell slot (one carried cell), then a scalar slot. The cell
        // renders as its `<^i64>` placeholder reading no heap bytes, and the
        // trailing scalar reads the cell *past* it, not `index * 8`.
        let cell_ty = Type::OwnedCell(OwnedCellId::from_index(0), "^i64");
        assert_eq!(
            format_stack(
                &[123, 99],
                &[cell_ty, Type::I64],
                &[],
                &[],
                &[],
                EnumId::from_index(0)
            ),
            "stack: <^i64> 99"
        );
    }

    #[test]
    fn format_stack_str_slot_shows_placeholder_and_offsets_past_it() {
        // A `str` slot's descriptor address is not dereferenced for content;
        // it renders as `<str>` and offsets past its one carried cell, like a
        // cell slot.
        assert_eq!(
            format_stack(
                &[0, 99],
                &[Type::Str, Type::I64],
                &[],
                &[],
                &[],
                EnumId::from_index(0)
            ),
            "stack: <str> 99"
        );
    }

    #[test]
    fn format_stack_cstr_slot_shows_placeholder_and_offsets_past_it() {
        assert_eq!(
            format_stack(
                &[0, 99],
                &[Type::Cstr, Type::I64],
                &[],
                &[],
                &[],
                EnumId::from_index(0)
            ),
            "stack: <cstr> 99"
        );
    }

    /// P7 slice 3c (R7/R2.2): a slice slot renders as its `<Slice[T]>`
    /// placeholder (matching the checker's ruling that a slice is not
    /// printable) and spans **two** cells, so the scalar above it reads the
    /// cell past the length word rather than the length word itself.
    #[test]
    fn format_stack_renders_slice() {
        let mut slices = Vec::new();
        let slice = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        assert_eq!(
            format_stack(
                &[0, 5, 99],
                &[slice, Type::I64],
                &[],
                &[],
                &[],
                EnumId::from_index(0)
            ),
            "stack: <Slice[i64]> 99"
        );
        // The rich renderer agrees on both the span and the placeholder.
        let ir_ty = ir::ir_type_of(slice);
        assert_eq!(rich_value_size(ir_ty, &[], &[], &[]), 16);
        assert_eq!(
            render_rich_value(ir_ty, &[0u8; 16], &[], &[], &[], &[]),
            "<slice>"
        );
    }

    /// P7 slice 3c (R8.2): the soundness wildcard. An imported `SliceId`
    /// indexes the *imported* module's registry, so it must shift by the
    /// session's registry length like every other id; left in `other => other`
    /// it would silently name whichever session slice sits at that index. The
    /// mutability and spelling ride along untouched, as they do for a `Ref`.
    #[test]
    fn remap_type_rebases_sliceid_across_modules() {
        let mut slices = Vec::new();
        let mutable = crate::ast::intern_slice_type(&mut slices, Type::I64, true);
        let no_bool = EnumRemap {
            base: 0,
            folded_bool: None,
            session_bool: EnumId::from_index(0),
        };
        let Type::Slice(id, m, name) = remap_type(mutable, no_bool, 0, 0, 0, 0, 3) else {
            panic!("a remapped slice is still a slice");
        };
        assert_eq!(id.index(), 3, "the id shifted by the session's slice base");
        assert!(m, "mutability rides along");
        assert_eq!(name, "!Slice[i64]");
        // A zero base is the identity, so a slice-free session is unaffected.
        assert_eq!(remap_type(mutable, no_bool, 9, 9, 9, 9, 0), mutable);
    }

    #[test]
    fn format_stack_unsigned_slot_displays_unsigned_not_negative() {
        // A `u64` with the high bit set stores a negative `i64` bit pattern;
        // display must render its unsigned value, not that negative number.
        let u64_ty = Type::from_name("u64").unwrap();
        assert_eq!(
            format_stack(&[-1], &[u64_ty], &[], &[], &[], EnumId::from_index(0)),
            "stack: 18446744073709551615"
        );
    }

    #[test]
    fn format_stack_usize_slot_displays_unsigned_not_negative() {
        // `Type::Usize` is a distinct variant from `Type::Int(u64)`; a
        // carried `usize` slot with the high bit set used to fall to the
        // catch-all `v.to_string()` arm and render negative.
        assert_eq!(
            format_stack(&[-1], &[Type::Usize], &[], &[], &[], EnumId::from_index(0)),
            "stack: 18446744073709551615"
        );
    }

    #[test]
    fn format_rich_struct_shows_field_values() {
        // D2/R14: unlike `format_stack`'s `<Vec2>` placeholder, the rich
        // formatter walks the struct's fields and renders their values.
        use crate::ast::StructId;
        let layouts = vec![StructLayout {
            name: "Vec2",
            size: 16,
            align: 8,
            fields: vec![
                ir::FieldLayout {
                    offset: 0,
                    ty: ir::IrType::I64,
                    size: 8,
                    align: 8,
                },
                ir::FieldLayout {
                    offset: 8,
                    ty: ir::IrType::I64,
                    size: 8,
                    align: 8,
                },
            ],
            is_linear: false,
            has_drop_overload: false,
            bundle: false,
            drop_generation: None,
        }];
        let vec2 = Type::Struct(StructId::from_index(0), "Vec2");
        assert_eq!(
            format_stack_rich(&[5, 6], &[vec2], &layouts, &[], &[], &[]),
            "stack: <Vec2 5i64 6i64>"
        );
    }

    #[test]
    fn format_rich_enum_shows_variant_and_payload() {
        // D2/R14: the active variant (by discriminant) and its payload field
        // values, not the `<TypeName>` placeholder.
        use crate::ast::EnumId;
        let enum_layouts = vec![EnumLayout {
            name: "E",
            tag_offset: 0,
            tag_ty: ir::IrType::Int {
                bits: 32,
                signed: true,
            },
            payload_offset: 8,
            size: 16,
            align: 8,
            variants: vec![
                ir::VariantLayout { fields: vec![] },
                ir::VariantLayout {
                    fields: vec![ir::FieldLayout {
                        offset: 0,
                        ty: ir::IrType::I64,
                        size: 8,
                        align: 8,
                    }],
                },
            ],
            is_scalar: false,
            is_linear: false,
            drop_generation: None,
        }];
        let e = Type::Enum(EnumId::from_index(0), "E");
        // Tag 1 (second variant) in the low 32 bits of the first cell, the
        // payload in the second.
        assert_eq!(
            format_stack_rich(&[1, 42], &[e], &[], &enum_layouts, &[], &[]),
            "stack: <E#1 42i64>"
        );
    }

    #[test]
    fn format_rich_array_shows_elements() {
        // D2/R14: every element, not the `<[T N]>` placeholder.
        use crate::ast::ArrayId;
        let array_layouts = vec![ArrayLayout {
            name: "[i64 3]",
            elem: ir::IrType::I64,
            count: 3,
            stride: 8,
            size: 24,
            align: 8,
            is_linear: false,
        }];
        let arr = Type::Array(ArrayId::from_index(0), "[i64 3]");
        assert_eq!(
            format_stack_rich(&[10, 20, 30], &[arr], &[], &[], &array_layouts, &[]),
            "stack: <[i64 3] 10i64 20i64 30i64>"
        );
    }

    #[test]
    fn format_rich_u8_distinguished_from_i64() {
        // R15: a `u8` `1` and an `i64` `1` must not both render as bare `1`.
        let u8_ty = Type::from_name("u8").unwrap();
        assert_eq!(
            format_stack_rich(&[1, 1], &[u8_ty, Type::I64], &[], &[], &[], &[]),
            "stack: 1u8 1i64"
        );
    }

    #[test]
    fn format_rich_owned_cell_read_does_not_consume() {
        // R16 (load-bearing): rendering an owning cell's payload is a read.
        // The heap value must still be there, unchanged, afterward.
        use crate::ast::OwnedCellId;
        let payload = Box::into_raw(Box::new(99i64));
        let cell_ty = Type::OwnedCell(OwnedCellId::from_index(0), "^i64");
        let buf = [payload as i64];
        let out = format_stack_rich(&buf, &[cell_ty], &[], &[], &[], &[ir::IrType::I64]);
        assert_eq!(out, "stack: ^99i64");
        // SAFETY: `payload` is still a live, exclusively-held allocation;
        // rendering ran no destructor and freed nothing.
        unsafe {
            assert_eq!(
                *payload, 99,
                "rendering must not have freed or mutated the cell"
            );
            drop(Box::from_raw(payload));
        }
    }

    /// Phase 6 slice 3 review (finding 4): the session define path runs the
    /// same eliminator interception `check_term` does, so a word named
    /// `Shape?` was accepted ("defined Shape?") and then permanently
    /// unreachable -- the next call to it routed to the generated eliminator
    /// and failed on the scrutinee's type. Rejected at the declaration
    /// instead, both ways round, since a session declares one thing per line
    /// and never sees the pair at once the way `assemble_module` does.
    #[test]
    fn session_rejects_a_word_shadowing_an_eliminator_either_declaration_order() {
        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line("type: Shape | Circle r i64 | Rect w i64 h i64 ;", &mut out)
            .unwrap();
        let err = session
            .eval_line(": Shape? ( i64 -- i64 ) 1 add ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("has the same name as the generated eliminator for enum `Shape`"),
            "unexpected message: {err}"
        );

        let mut session = Session::new();
        session
            .eval_line(": Shape? ( i64 -- i64 ) 1 add ;", &mut out)
            .unwrap();
        let err = session
            .eval_line("type: Shape | Circle r i64 | Rect w i64 h i64 ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("has the same name as the generated eliminator for enum `Shape`"),
            "unexpected message: {err}"
        );
        // The rejected `type:` line leaves the session as it was: the word it
        // would have shadowed is still callable.
        out.clear();
        session.eval_line("5 Shape? .", &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "stack: (empty)\n");
    }

    /// F1/F2: everything above (`format_rich_*`) exercises `format_stack_rich`
    /// directly with hand-built layout structs; `enable_rich_stack_rendering`,
    /// `render_stack`'s rich branch, and `eval_expr`'s rich branch
    /// (the actual live wiring `run_tty` uses) are otherwise never exercised
    /// by any test, since a real tty is required to reach `run_tty` itself. A
    /// wrong cell index or an off-by-one there would ship green without this.
    #[test]
    fn session_rich_rendering_shows_struct_contents_through_real_session() {
        let mut session = Session::new();
        session.enable_rich_stack_rendering();
        let mut out = Vec::new();
        session
            .eval_line("type: Vec2 x i64 y i64 ;", &mut out)
            .unwrap();
        session.eval_line("5 6 Vec2", &mut out).unwrap();
        assert_eq!(session.render_stack(), "stack: <Vec2 5i64 6i64>");

        // R16's session-level half: after rendering an owning cell through
        // the real session (not just the isolated formatter, per
        // `format_rich_owned_cell_read_does_not_consume` above), it must
        // still be genuinely live and droppable, not merely "not freed in
        // isolation".
        session.eval_line("drop", &mut out).unwrap(); // dispose the Vec2 first
        session.eval_line("99 ^", &mut out).unwrap();
        assert_eq!(session.render_stack(), "stack: ^99i64");
        session.eval_line("drop", &mut out).unwrap();
        assert_eq!(session.render_stack(), "stack: (empty)");
    }

    /// R-D2/X13: the REPL's env builder marks an ordinary `[ ... ]` parameter
    /// slot with the quotation `IrType` its `(code, env)` aggregate carries, so
    /// a call site lowered in a session materializes its phantom argument
    /// exactly as the batch compiler's does. Built from a really parsed
    /// signature, since a `Type::Quotation` carries an interned effect no test
    /// should hand-fabricate. Left unpopulated, the REPL half of the real-call
    /// path lowers a bare phantom into `Instr::Call` and this reads `I64`.
    #[test]
    fn ir_arity_env_marks_an_ordinary_quotation_parameter_slot() {
        let src = ": apply ( [ i64 -- i64 ] i64 -- i64 ) | n | | f | n f call ;";
        let tokens = lexer::lex(src).unwrap();
        let module = parser::parse(&tokens).unwrap();
        let sig = check::sig_of(&module.words[0].effect);
        let env = HashMap::from([(
            "apply".to_string(),
            vec![check::Overload {
                sig,
                symbol: "apply".to_string(),
            }],
        )]);

        let arity = ir_arity_env(&env).remove("apply").expect("the sole entry");
        assert_eq!(arity.in_arity, 2);
        assert_eq!(arity.quot_inputs.len(), 1, "only slot 0 is a quotation");
        let (slot, ty) = arity.quot_inputs[0];
        assert_eq!(slot, 0);
        assert!(
            matches!(ty, ir::IrType::Quotation(_)),
            "the slot carries the parameter's quotation type, got {ty:?}"
        );
    }

    fn entry(generation: u64) -> WordEntry {
        WordEntry {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![Type::I64],
            },
            generation,
            symbol: mangled_symbol("sq", generation),
        }
    }

    #[test]
    fn resolve_binds_current_generation() {
        let mut env = HashMap::new();
        env.insert("sq".to_string(), entry(2));
        let resolve = resolver_with_override(&env, "__none__", "__none__");
        assert_eq!(resolve("sq"), "sq__gen2");
        assert_eq!(resolve("dup"), "dup");
    }

    #[test]
    fn redefinition_bumps_generation() {
        let mut env = HashMap::new();
        assert_eq!(next_generation(env.get("sq")), 0);
        env.insert("sq".to_string(), entry(0));
        assert_eq!(next_generation(env.get("sq")), 1);
    }

    /// The destructor symbols one REPL line emits, built through the same
    /// session state and the same synthesis call every line uses. `declaring`
    /// is the struct whose `: drop` line is being evaluated, `None` for an
    /// ordinary line (R11.3: an ordinary line emits no override body).
    fn destructor_symbols(session: &Session, declaring: Option<StructId>) -> Vec<String> {
        let (mut structs, mut enums, arrays, mut cells, refs) = ir::build_registries(
            &session.structs,
            &session.enums,
            &session.arrays,
            &session.owned_cells,
            &session.refs,
        );
        session.apply_drop_generations(&mut structs, &mut enums, &mut cells);
        let slices = ir::build_slices(&session.slices, &structs, &enums, &arrays);
        let regs = ir::Registries {
            structs: &structs,
            enums: &enums,
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
            slices: &slices,
            statics: ir::empty_statics(),
        };
        let env = ir_arity_env(&session.typed_env());
        let resolve = resolver_for(&session.env);
        ir::synthesize_aggregate_destructors(
            &env,
            &resolve,
            regs,
            &session.drop_override_bodies(declaring),
            ir::empty_resolved_fields(),
            ir::empty_resolved_variant_fields(),
            &combinator_bodies(&session.combinators),
        )
        .into_iter()
        .map(|f| f.name)
        .collect()
    }

    #[test]
    fn repl_drop_overload_is_kept_by_struct_id_and_out_of_env() {
        // R11.1: the override is retained under the struct's id, so a later
        // line can re-synthesize its destructor from a body whose own line is
        // long gone; and it never enters `env`, mirroring R1's exclusion (a
        // `drop` call site is intercepted before any name lookup, so an entry
        // there would be dead).
        let mut session = Session::new();
        let mut out = Vec::new();
        session.eval_line("type: Res n i64 ;", &mut out).unwrap();
        session
            .eval_line(": drop ( Res -- ) | r | r Res> . ;", &mut out)
            .unwrap();

        let id = StructId::from_index(0);
        assert!(!session.env.contains_key("drop"));
        assert_eq!(session.drop_overloads[&id].0, 0);
        assert!(session.structs[0].has_drop_overload);
    }

    /// A scratch directory of `.sth` library files, removed on drop; mirrors
    /// `driver`'s own closure-test sandbox. Import lines in these unit tests
    /// embed the returned absolute path, so cwd (R3) is not exercised here (a
    /// golden covers that under a lock).
    struct LibDir(std::path::PathBuf);
    impl LibDir {
        fn new(tag: &str) -> LibDir {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            let seq = N.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("sooth-replimp-{}-{tag}-{seq}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            LibDir(dir)
        }
        fn write(&self, name: &str, contents: &str) -> std::path::PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, contents).unwrap();
            path
        }
    }
    impl Drop for LibDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn import_line(qualifier: &str, path: &std::path::Path) -> String {
        format!("import: \"{}\" {qualifier} ;", path.display())
    }

    #[test]
    fn repl_assembles_checked_module_for_library() {
        // U3: `discover_closure` / `assemble_module` are reachable as
        // `pub(crate)` and yield a checked module for a library path (a
        // plumbing smoke test, no guarded invariant).
        let d = LibDir::new("u3");
        let lib = d.write("lib.sth", ": w ( -- i64 ) 42 ;\nexport: w ;\n");
        let closure = driver::discover_closure(&lib).expect("closure resolves");
        let mut module = driver::assemble_module(&closure, false).expect("assembles");
        check::check(&mut module).expect("checks");
        assert!(module.words.iter().any(|w| w.name == "w"));
    }

    #[test]
    fn imported_aggregate_ids_remap_to_session_space() {
        // U1: a spliced imported struct's field id points at the *session*
        // index of the struct it names, not the closure-local index. A local
        // struct declared first forces a non-zero base, so a naive splice that
        // kept closure-local ids would point one entry too low.
        let d = LibDir::new("u1");
        let lib = d.write(
            "lib.sth",
            "type: Inner a i64 ;\ntype: Outer i Inner ;\nexport: Inner Outer ;\n",
        );
        let mut session = Session::new();
        let mut out = Vec::new();
        session.eval_line("type: Local z i64 ;", &mut out).unwrap();
        session
            .eval_line(&import_line("q", &lib), &mut out)
            .unwrap();

        let inner_idx = session
            .structs
            .iter()
            .position(|s| s.name_static == "Inner" && s.module != 0)
            .expect("Inner spliced");
        let outer = session
            .structs
            .iter()
            .find(|s| s.name_static == "Outer" && s.module != 0)
            .expect("Outer spliced");
        match outer.fields[0].1 {
            Type::Struct(id, _) => assert_eq!(
                id.index(),
                inner_idx,
                "Outer's field id must point at Inner's session index"
            ),
            other => panic!("expected Outer's field to be a struct, got {other:?}"),
        }
        assert!(inner_idx > 0, "the local struct forced a non-zero base");
    }

    #[test]
    fn import_epoch_symbols_are_session_fresh() {
        // U2: a spliced word's symbol carries the `__import{epoch}` marker,
        // is distinct across import events, and never collides with an
        // ordinary word's `__gen{N}`.
        let d = LibDir::new("u2");
        let lib = d.write("lib.sth", ": w ( -- i64 ) 7 ;\nexport: w ;\n");
        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line(&import_line("q", &lib), &mut out)
            .unwrap();
        session
            .eval_line(&import_line("r", &lib), &mut out)
            .unwrap();
        session
            .eval_line(": ordinary ( -- i64 ) 1 ;", &mut out)
            .unwrap();

        let q_sym = &session.env["q::w__import0"].symbol;
        let r_sym = &session.env["r::w__import1"].symbol;
        assert_eq!(q_sym, "w__import0");
        assert_eq!(r_sym, "w__import1");
        assert_ne!(q_sym, r_sym, "each import event mints a distinct symbol");
        assert!(!q_sym.contains("__gen"), "never an ordinary-word symbol");
        assert!(
            session.env["ordinary"].symbol.contains("__gen"),
            "an ordinary word still mints `__gen`"
        );
    }

    /// Phase 6 slice 3 review fix (cycle 3): teaching
    /// `split_destructure_suffix` about the eliminator's `?` made an imported
    /// word whose own name ends in `?` (`ok?`, an ordinary spelling) split to
    /// a base the import event never aliased, so a `q::ok?` call that resolved
    /// before this slice reported `unknown word`. The whole spelling is tried
    /// first, and the split still serves a genuinely generated name (`q::P>`).
    /// The `not exported` wording follows the same rule: a generated word is
    /// gated by its type, a suffix-spelled word by itself.
    #[test]
    fn import_call_to_a_word_named_like_a_generated_one_resolves() {
        let d = LibDir::new("suffix-word");
        let lib = d.write(
            "lib.sth",
            "type: P x i64 ;\n\
             type: H y i64 ;\n\
             : ok? ( i64 -- i64 ) drop 1 ;\n\
             : hidden? ( i64 -- i64 ) drop 1 ;\n\
             export: ok? P ;\nimport: intrinsics * ;\n",
        );
        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line(&import_line("q", &lib), &mut out)
            .unwrap();
        let at = |name: &str| session.rewrite_import_call(name, Span::default());

        assert_eq!(
            at("q::ok?").unwrap(),
            Some("q::ok?__import0".to_string()),
            "an exported word whose name ends in `?` resolves whole"
        );
        assert_eq!(
            at("q::P>").unwrap(),
            Some("q::P__import0>".to_string()),
            "a generated destructure still resolves through the split base"
        );
        let hidden = at("q::hidden?").unwrap_err();
        assert!(
            hidden.contains("`hidden?` is not exported"),
            "a private word names itself, suffix included: {hidden}"
        );
        let private_type = at("q::H>").unwrap_err();
        assert!(
            private_type.contains("`H` is not exported"),
            "a generated word is gated by its type, and names it: {private_type}"
        );
    }

    #[test]
    fn import_private_names_distinguish_not_exported_from_absent() {
        // U6: the retained private-name set answers `not exported` for a real
        // but unexported name and leaves a genuinely absent name to fall
        // through to the ordinary unknown-word path.
        let d = LibDir::new("u6");
        let lib = d.write(
            "lib.sth",
            ": pub ( -- i64 ) 1 ;\n: secret ( -- i64 ) 2 ;\nexport: pub ;\n",
        );
        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line(&import_line("q", &lib), &mut out)
            .unwrap();

        let private = &session.import_private["q"];
        assert!(private.contains("secret"), "a private word is retained");
        assert!(!private.contains("pub"), "an exported word is not private");

        let err = session
            .rewrite_import_call("q::secret", Span::default())
            .unwrap_err();
        assert!(
            err.contains("not exported") && err.contains("secret"),
            "a private name is `not exported`: {err}"
        );
        assert!(
            session
                .rewrite_import_call("q::absent", Span::default())
                .unwrap()
                .is_none(),
            "a genuinely absent name falls through to unknown-word"
        );
    }

    #[test]
    fn import_private_names_omit_the_retired_accessor_spellings() {
        // P7 slice 1 (D1/R11 deletion guard): a private type retains its bare
        // name and its destructure, and nothing else. Retaining `Point>x` and
        // siblings would answer `not exported` for a spelling no longer in the
        // language, where a native build says `unknown word` -- the REPL and
        // the module path disagreeing on a retired feature is exactly R5's
        // hazard.
        let d = LibDir::new("p5acc");
        let lib = d.write(
            "lib.sth",
            "type: Point x i64 y i64 ;\n: pub ( -- i64 ) 1 ;\nexport: pub ;\n",
        );
        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line(&import_line("p5acc", &lib), &mut out)
            .unwrap();

        let private = &session.import_private["p5acc"];
        assert!(private.contains("Point"), "the bare type name is retained");
        assert!(private.contains("Point>"), "the destructure is retained");
        for retired in ["Point>x", "Point<x", "Point|>x"] {
            assert!(
                !private.contains(retired),
                "`{retired}` is retired and must not be retained"
            );
            assert!(
                session
                    .rewrite_import_call(&format!("p5acc::{retired}"), Span::default())
                    .unwrap()
                    .is_none(),
                "`{retired}` falls through to unknown-word, as in a native build"
            );
        }
    }

    #[test]
    fn import_epoch_symbols_are_session_fresh_across_a_reimport() {
        // U7 (phase 2): reloading the same qualifier overwrites its
        // `import_aliases` entry to the new epoch's spelling -- the old
        // epoch's registry row (its env entry / symbol) stays resident, only
        // unreferenced by any current alias, never removed (R9 positional
        // stability, R13).
        let d = LibDir::new("u7");
        let lib = d.write("lib.sth", ": w ( -- i64 ) 1 ;\nexport: w ;\n");
        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line(&import_line("q", &lib), &mut out)
            .unwrap();
        session
            .eval_line(&import_line("q", &lib), &mut out)
            .unwrap();

        assert_eq!(
            session.import_aliases.len(),
            1,
            "a reload overwrites the alias rather than appending a second one"
        );
        assert_eq!(
            session.import_aliases["q::w"], "q::w__import1",
            "the alias now resolves to the second import event's spelling"
        );
        assert!(
            session.env.contains_key("q::w__import0"),
            "the first event's env row stays resident, unremoved"
        );
        assert!(
            session.env.contains_key("q::w__import1"),
            "the second event's row is the fresh one"
        );
    }

    #[test]
    fn word_names_includes_display_name_import_and_poly_word() {
        // R23: `word_names()` is completion's actual feed (`:words`' own
        // listing is a separate, parallel implementation and does not prove
        // this function does the right thing on its own). Reverting either
        // the `display_name` reversal or the `poly_words` fold inside
        // `word_names()` must fail this test, not just `:words`' listing.
        let d = LibDir::new("word_names");
        let lib = d.write(
            "lib.sth",
            ": inc ( i64 -- i64 ) 1 add ;\nexport: inc ;\nimport: intrinsics * ;\n",
        );
        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line(&import_line("m", &lib), &mut out)
            .unwrap();
        session
            .eval_line(": alen ( ['T 'N] -- ) drop ;", &mut out)
            .unwrap();

        let names = session.word_names();
        assert!(
            names.contains(&"m::inc".to_string()),
            "an imported word is completable under its user-facing spelling, not the import-epoch-mangled env key: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("__import")),
            "no mangled spelling leaks into completion: {names:?}"
        );
        assert!(
            names.contains(&"alen".to_string()),
            "a polymorphic word is completable, not just concrete ones: {names:?}"
        );
    }

    #[test]
    fn session_selective_collision_is_rejected() {
        // U5 (phase 3): the session-scope selective-collision check (R12)
        // rejects a selectively-exposed name that already names an existing
        // session word, and a second selective import that exposes a name a
        // prior one already exposed, naming both sources.
        let d = LibDir::new("u5");
        let lib_a = d.write("a.sth", ": shared ( -- i64 ) 1 ;\nexport: shared ;\n");
        let lib_b = d.write("b.sth", ": other ( -- i64 ) 2 ;\nexport: other ;\n");
        let lib_c = d.write("c.sth", ": other ( -- i64 ) 3 ;\nexport: other ;\n");

        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line(": shared ( -- i64 ) 9 ;", &mut out)
            .unwrap();
        let err = session
            .eval_line(
                &format!("import: \"{}\" q | shared | ;", lib_a.display()),
                &mut out,
            )
            .unwrap_err();
        assert!(
            err.contains("shared") && err.contains('q'),
            "collides with a local definition, naming both: {err}"
        );
        assert!(
            !session.import_qualifier_module.contains_key("q"),
            "the rejected import leaves the session untouched"
        );

        session
            .eval_line(
                &format!("import: \"{}\" r | other | ;", lib_c.display()),
                &mut out,
            )
            .unwrap();
        let err = session
            .eval_line(
                &format!("import: \"{}\" s | other | ;", lib_b.display()),
                &mut out,
            )
            .unwrap_err();
        assert!(
            err.contains("other") && err.contains('r') && err.contains('s'),
            "collides with an earlier selective import, naming both sources: {err}"
        );
    }

    #[test]
    fn imported_main_is_rejected_by_scan() {
        // U4: the main-in-closure scan rejects an imported `main`, naming the
        // declaring file and the word, regardless of where in the closure it
        // sits (`mangle` never renames `main`).
        let d = LibDir::new("u4");
        let lib = d.write(
            "lib.sth",
            ": helper ( -- i64 ) 1 ;\n: main ( -- ) ;\nexport: helper ;\n",
        );
        let closure = driver::discover_closure(&lib).expect("closure resolves");
        let mut module = driver::assemble_module(&closure, false).expect("assembles");
        check::check(&mut module).expect("checks");
        let err = driver::check_no_main_in_closure(&module, &closure, None).unwrap_err();
        assert!(err.contains("main"), "names the word: {err}");
        assert!(
            err.contains(lib.file_name().unwrap().to_str().unwrap()),
            "names the file: {err}"
        );
    }

    #[test]
    fn repl_drop_overload_declaration_shape_is_validated() {
        // R11.1: a REPL line gets R1's declaration-shape rule, not a laxer
        // one, and a rejected line leaves the session untouched.
        let mut session = Session::new();
        let mut out = Vec::new();
        let err = session
            .eval_line(": drop ( i64 -- ) drop ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("must take a `type:`-declared struct"),
            "unexpected message: {err}"
        );
        assert!(session.drop_overloads.is_empty());
    }

    #[test]
    fn repl_global_clause_is_a_located_rejection() {
        // R6: the boundary check lives in `assemble_module`, which the REPL
        // never reaches, so without this gate the clause is accepted and
        // silently unchecked -- the same line a file build rejects for naming
        // no such static. The error is located at the offending entry, and the
        // word does not enter the session.
        let mut session = Session::new();
        let mut out = Vec::new();
        let err = session
            .eval_line(": tick ( -- i64 ) global: NOPE w 1 ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("`global:` has no meaning at the REPL") && err.contains("col 27"),
            "unexpected message: {err}"
        );
        assert!(!session.env.contains_key("tick"));
    }

    #[test]
    fn repl_rejects_impl() {
        // R8: `impl:` is wired only through `assemble_module`, so without
        // this gate the line falls into the term loop and reports an
        // unrelated "unexpected token Semicolon" error.
        let mut session = Session::new();
        let mut out = Vec::new();
        let err = session
            .eval_line("impl: Order for Point : cmp | a b | a b ; ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("`impl:` has no meaning at the REPL")
                && err.contains("col 1")
                && err.contains(
                    "note: a live session has no module to attach a trait implementation to"
                ),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn repl_rejects_trait() {
        let mut session = Session::new();
        let mut out = Vec::new();
        let err = session
            .eval_line("trait: Order 'T cmp ( &'T &'T -- Ordering ) ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("`trait:` has no meaning at the REPL")
                && err.contains("col 1")
                && err.contains("note: a live session declares no trait to satisfy"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn repl_generic_typedef_is_a_located_not_supported_error() {
        // Phase 5 slice 1 review fix: without this gate, a generic header
        // ran straight into the concrete `parse_typedef`/`parse_enum_typedef`
        // field loop and reported a nonsense "unknown type 'T" error naming a
        // type variable, rather than a diagnostic naming the real gap.
        let mut session = Session::new();
        let mut out = Vec::new();
        let err = session
            .eval_line("type: Box 'T val 'T ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("not supported in the REPL yet"),
            "unexpected message: {err}"
        );
        // Session state rolls back cleanly: the failed line registers no type.
        assert!(session.structs.is_empty());

        let err = session
            .eval_line("type: Result 'T | Ok val 'T ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("not supported in the REPL yet"),
            "unexpected message: {err}"
        );
    }

    /// Phase 6 slice 2 (R6/R-OQ2): the variant accessors register into the
    /// session env too, not only into a native build's. No REPL line can mint
    /// a `Type::Variant` operand until slice 3's eliminator, so what
    /// discriminates "registered" from "never wired into `typed_env`" is which
    /// diagnostic a bare call gets: a registered word underflows, an
    /// unregistered one is an unknown word. Per-field access is a
    /// receiver-directed projection now (R4), not a generated word, so
    /// `Circle>r`/`Dot>x` are simply unknown.
    #[test]
    fn repl_variant_accessor_sigs_reach_the_session_env() {
        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line("type: Shape | Circle r i64 | Dot ;", &mut out)
            .unwrap();
        for call in ["Circle>", "Dot>"] {
            let err = session.eval_type(call, &mut out).unwrap_err();
            assert!(
                err.contains("stack underflow: needs 1 values"),
                "{call}: unexpected message: {err}"
            );
        }
        for call in ["Circle>r", "Dot>x"] {
            let err = session.eval_type(call, &mut out).unwrap_err();
            assert!(
                err.contains(&format!("unknown word `{call}`")),
                "unexpected: {err}"
            );
        }
    }

    /// P7.S3k (review fix): `eval_poly_def` passes an empty callee registry
    /// on purpose (see its own comment above `check::CrossCtx` in
    /// `eval_poly_def`) -- session lowering has no composition step for a
    /// cross-call's substitution, so grounding it here would check clean and
    /// then mis-lower rather than reject cleanly. Pins the deliberate
    /// `unknown word` outcome so a future implementer who "finishes the
    /// thread-through" by swapping in `self.poly_env()` gets a failing test
    /// instead of a green suite and a panic
    /// (`self.env.get(name).expect("checked user word exists")`,
    /// `calls.rs:725`) the first time a session line actually runs one.
    #[test]
    fn repl_poly_word_calling_another_poly_word_is_unknown_word_not_grounded() {
        let mut session = Session::new();
        let mut out = Vec::new();
        session.eval_line(": id ( 'T -- 'T ) ;", &mut out).unwrap();
        let err = session
            .eval_line(": g ( 'T -- 'T ) id ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("unknown word `id`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn repl_self_recursive_drop_overload_is_a_located_error_not_a_crash() {
        // Blocker 1: `check_def` alone only validates this override's body in
        // isolation, so without `check_drop_overload_reachability` this line
        // would register a `drop` override whose own body drops its own
        // receiver -- a compile-time R6 rejection natively, but an unbounded
        // runtime recursion (stack overflow) at the REPL, since nothing else
        // ever asks the reachability question here.
        let mut session = Session::new();
        let mut out = Vec::new();
        session.eval_line("type: Res n i64 ;", &mut out).unwrap();
        let err = session
            .eval_line(": drop ( Res -- ) | r | r drop ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `Res`"),
            "unexpected message: {err}"
        );
        assert!(session.drop_overloads.is_empty());
        assert!(!session.structs[0].has_drop_overload);
    }

    #[test]
    fn repl_same_body_indirect_drop_recursion_through_a_composing_type_is_a_located_error() {
        // Blocker 1's other crashing shape: the override never calls `drop`
        // on its own receiver directly, only on a freshly built `Box` that
        // *composes* it -- `Box` has no override of its own, so disposing one
        // runs generic field glue back into `Res`'s override (R6 case (b),
        // the same shape
        // `check_drop_body_recursion_through_a_containing_aggregate_is_error`
        // exercises natively).
        let mut session = Session::new();
        let mut out = Vec::new();
        session.eval_line("type: Res n i64 ;", &mut out).unwrap();
        session.eval_line("type: Box f Res ;", &mut out).unwrap();
        let err = session
            .eval_line(": drop ( Res -- ) | r | r Box drop ;", &mut out)
            .unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `Res`"),
            "unexpected message: {err}"
        );
        assert!(session.drop_overloads.is_empty());
        assert!(!session.structs[0].has_drop_overload);
    }

    #[test]
    fn repl_redefining_drop_overload_does_not_collide_under_rtld_global() {
        // Criterion 22/R11.2: two generations of one struct's override are two
        // different bodies, so they must be two different global symbols --
        // every REPL library loads `RTLD_GLOBAL`, and the unsuffixed name is
        // only safe while every generation's body is identical glue.
        let mut session = Session::new();
        let mut out = Vec::new();
        session.eval_line("type: Res n i64 ;", &mut out).unwrap();
        session
            .eval_line(": drop ( Res -- ) | r | r Res> . ;", &mut out)
            .unwrap();
        let id = StructId::from_index(0);
        let first = destructor_symbols(&session, Some(id));
        session
            .eval_line(": drop ( Res -- ) | r | r Res> 100 add . ;", &mut out)
            .unwrap();
        let second = destructor_symbols(&session, Some(id));

        assert_eq!(first, vec!["sooth_struct_drop_0__gen0".to_string()]);
        assert_eq!(second, vec!["sooth_struct_drop_0__gen1".to_string()]);
        assert_eq!(session.drop_overloads[&id].0, 1);
        // R11.3: an ordinary line in between emits neither, leaving the
        // pinned symbol to resolve through `RTLD_GLOBAL`.
        assert!(destructor_symbols(&session, None).is_empty());
    }

    #[test]
    fn repl_generic_glue_symbol_stays_unsuffixed_while_session_has_no_override() {
        // R11.2: every symbol stays unsuffixed only while the session has
        // never defined a `drop` override -- once one exists (even for an
        // unrelated struct), generic glue starts carrying the session's
        // override epoch too (see the next test), matching the build path
        // only in the no-override case.
        let mut session = Session::new();
        let mut out = Vec::new();
        session
            .eval_line("type: Pair a i64 b i64 ;", &mut out)
            .unwrap();
        // Force linearity by hand rather than through a real `: drop` line
        // (mirrors slice 8b's `has_drop_overload` bit): this test's whole
        // point is the state *before* the session has ever evaluated a
        // `drop` override, so `session.drop_overloads` must stay empty.
        session.structs[0].has_drop_overload = true;
        assert_eq!(
            destructor_symbols(&session, None),
            vec!["sooth_struct_drop_0".to_string()]
        );
    }

    #[test]
    fn repl_unrelated_overload_suffixes_a_composing_structs_glue_too() {
        // R11.2 (code review, phase 4): `Holder` never has its own `drop`
        // override, but its destructor `Call`s `Res`'s -- so once `Res` gets
        // an override, `Holder`'s own symbol must also change, or its
        // frozen first-loaded body (still calling the pre-override callee)
        // would keep winning under `RTLD_GLOBAL` forever.
        let mut session = Session::new();
        let mut out = Vec::new();
        session.eval_line("type: Res n i64 ;", &mut out).unwrap();
        // Force `Res` linear by hand rather than through a real `: drop`
        // line yet (mirrors slice 8b's `has_drop_overload` bit): the
        // override below must still be the session's *first*, so
        // `session.drop_overloads` has to stay empty until then.
        session.structs[0].has_drop_overload = true;
        session.eval_line("type: Holder r Res ;", &mut out).unwrap();
        let before = destructor_symbols(&session, None);
        session
            .eval_line(": drop ( Res -- ) | r | 42 . r Res> drop ;", &mut out)
            .unwrap();
        let id = StructId::from_index(0);
        let defining_line = destructor_symbols(&session, Some(id));
        let later_line = destructor_symbols(&session, None);

        assert_eq!(
            before,
            vec![
                "sooth_struct_drop_0".to_string(),
                "sooth_struct_drop_1".to_string(),
            ]
        );
        assert_eq!(
            defining_line,
            vec![
                "sooth_struct_drop_0__gen0".to_string(),
                "sooth_struct_drop_1__gen0".to_string(),
            ]
        );
        // R11.3: `Holder`'s glue is re-emitted on every later line (harmless,
        // its body is identical mechanical glue and it must exist in a module
        // that drops a `Holder`), and it calls `Res`'s pinned symbol; `Res`'s
        // own destructor is not re-emitted.
        assert_eq!(later_line, vec!["sooth_struct_drop_1__gen0".to_string()]);
    }

    /// P8 slice 1a: a module-name import at the REPL has no manifest to
    /// resolve against and is wired into nothing the REPL runs -- so it is
    /// rejected outright, not silently accepted.
    #[test]
    fn repl_module_name_import_is_rejected() {
        let mut session = Session::new();
        let mut out = Vec::new();
        let err = session
            .eval_line("import: core::cmp c ;", &mut out)
            .unwrap_err();
        assert_eq!(
            err,
            "error: module-name import at line 1, col 1 in <repl>:\n  the REPL cannot resolve a module-name import yet\n  use a quoted-path import instead"
        );
    }

    /// P8 slice 1a: a wildcard import binds no qualifier, and nothing gives
    /// it a visibility effect here -- so it is rejected outright at the REPL
    /// rather than silently splicing in nothing. The target is a quoted path
    /// (not a module name), so it is the wildcard rejection that fires here,
    /// not the module-name one above.
    #[test]
    fn repl_wildcard_import_is_rejected() {
        let mut session = Session::new();
        let mut out = Vec::new();
        let err = session
            .eval_line("import: \"lib/queue.sth\" * ;", &mut out)
            .unwrap_err();
        assert_eq!(
            err,
            "error: wildcard import at line 1, col 1 in <repl>:\n  a wildcard import binds no names in the REPL\n  use a qualified import instead"
        );
    }

    #[test]
    fn eval_line_reserved_caret_variant_name_is_error() {
        // R12a at REPL scope: a variant name is a word-generating declaration
        // site too, mirroring the module-parser pre-pass check.
        let mut session = Session::new();
        let mut out = Vec::new();
        let err = session
            .eval_line("type: E | ^ x i64 | B y i64 ;", &mut out)
            .unwrap_err();
        assert!(err.contains("reserved"), "unexpected message: {err}");
        assert!(err.contains('^'), "unexpected message: {err}");
    }

    #[test]
    fn poly_instantiation_repeated_at_one_type_dedups_in_exported_insts() {
        // Criterion 2 (R7/D2): a second same-type instantiation of a
        // retained polymorphic word must not mint a second exported symbol,
        // or `.so` growth is unbounded across the session's life (trace B).
        let mut session = Session::new();
        let mut out = Vec::new();
        session.eval_line(": id ( 'T -- 'T ) ;", &mut out).unwrap();
        session.eval_line("5 id .", &mut out).unwrap();
        session.eval_line("7 id .", &mut out).unwrap();
        assert_eq!(
            session.exported_insts.len(),
            1,
            "expected exactly one exported instantiation symbol, got: {:?}",
            session.exported_insts
        );
    }
}
