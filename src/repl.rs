//! REPL: compile each line through the normal pipeline to a shared object and
//! `dlopen` it into the session process (no interpreter, no JIT).
//!
//! `Session` owns the persistent stack buffer and the word env (arity +
//! generation + symbol); the read-eval-print loop lexes/parses/checks/lowers/
//! emits/compiles/loads each line exactly like `build`, differing only in
//! target (`.so` not a binary) and in carrying state across lines.

use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::{BufRead, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::ast::Module;
use crate::ast::{
    ArrayDecl, ArrayId, CallInst, EnumDecl, EnumId, Import, Line, OwnedCellDecl, OwnedCellId,
    PolySig, RefDecl, RefId, Span, StructDecl, StructId, Term, TermKind, Type, VariantDecl,
    WordBody, WordDef,
};
use crate::check::{self, word_span, Sig};
use crate::driver;
use crate::ir::ArrayLayout;
use crate::ir::{self, EnumLayout, IrModule, StructLayout};
use crate::lexer::Token;
use crate::resolve::split_accessor;
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
}

/// Derive ir's arity map (RK2) from the typed checker env: ir needs only the
/// input/output counts and the output `IrType`, not the full typed effect.
fn ir_arity_env(env: &HashMap<String, Sig>) -> HashMap<String, ir::Arity> {
    env.iter()
        .map(|(name, sig)| {
            let ret = sig.outputs.first().map(|&ty| ir::ir_type_of(ty));
            (name.clone(), (sig.inputs.len(), sig.outputs.len(), ret))
        })
        .collect()
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

/// R9 (slice 5b): shift a closure-local `Type`'s registry id into session
/// space by the session's registry lengths captured at splice time. A scalar
/// type carries no id and passes through unchanged.
fn remap_type(
    ty: Type,
    struct_base: usize,
    enum_base: usize,
    array_base: usize,
    cell_base: usize,
    ref_base: usize,
) -> Type {
    match ty {
        Type::Struct(id, n) => Type::Struct(StructId::from_index(id.index() + struct_base), n),
        Type::Enum(id, n) => Type::Enum(EnumId::from_index(id.index() + enum_base), n),
        Type::Array(id, n) => Type::Array(ArrayId::from_index(id.index() + array_base), n),
        Type::OwnedCell(id, n) => {
            Type::OwnedCell(OwnedCellId::from_index(id.index() + cell_base), n)
        }
        Type::Ref(id, m, n) => Type::Ref(RefId::from_index(id.index() + ref_base), m, n),
        other => other,
    }
}

/// R8e (slice 5b): a REPL-declared name may not contain `::`, the separator
/// reserved for a qualified imported spelling; otherwise a user could forge an
/// import's internal epoch-tagged name and hijack its accessor sigs. A new,
/// REPL-only guard (native `.sth` declarations have the same latent gap but no
/// tag to collide with), located, naming the offending spelling.
fn reject_double_colon_name(kind: &str, name: &str, span: Span) -> Result<(), String> {
    if name.contains("::") {
        return Err(format!(
            "error: a REPL-declared {kind} name may not contain `::` (`{name}` at line {}, col {})",
            span.line, span.col
        ));
    }
    Ok(())
}

/// R14/D4 (slice 5b): reject an imported closure that declares a word named
/// `main`, naming the declaring file and the word, before any codegen.
/// `mangle` (`src/resolve.rs`) never renames `main` regardless of module, so
/// a plain name scan over every file in the closure finds it, whichever file
/// it came from (recon #4's native collision turned into a diagnostic here;
/// the native path's own exposure stays unfixed, per D4).
fn check_no_main_in_closure(module: &Module, closure: &driver::Closure) -> Result<(), String> {
    let Some(main) = module.words.iter().find(|w| w.name == "main") else {
        return Ok(());
    };
    let path = closure.path_of(main.module);
    let span = word_span(main);
    Err(format!(
        "error: cannot import `{}`: it declares a word named `main` (line {}, col {}); a library file may not declare `main`",
        path.display(),
        span.line,
        span.col
    ))
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
) -> String {
    if types.is_empty() {
        return "stack: (empty)".to_string();
    }
    let mut cell = 0usize;
    let mut vals = Vec::with_capacity(types.len());
    for ty in types {
        match ty {
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
            _ => {
                let v = buf[cell];
                vals.push(match ty {
                    Type::Float(ft) if ft.bits() == 32 => {
                        f32::from_bits(v as u64 as u32).to_string()
                    }
                    Type::Float(_) => f64::from_bits(v as u64).to_string(),
                    Type::Bool => if v != 0 { "true" } else { "false" }.to_string(),
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
}

impl Session {
    pub fn new() -> Session {
        Session {
            env: HashMap::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            arrays: Vec::new(),
            owned_cells: Vec::new(),
            refs: Vec::new(),
            drop_overloads: HashMap::new(),
            drop_dropped_sites: HashMap::new(),
            poly_words: HashMap::new(),
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
        }
    }

    /// The checker's typed env: builtins, the generated struct words, the
    /// variant-constructor words, plus every successfully-defined user word.
    fn typed_env(&self) -> HashMap<String, Sig> {
        let mut env = check::builtin_table();
        for (name, sig) in check::struct_generated_sigs(&self.structs) {
            env.insert(name, sig);
        }
        for (name, sig) in check::enum_generated_sigs(&self.enums) {
            env.insert(name, sig);
        }
        for (name, entry) in &self.env {
            env.insert(name.clone(), entry.sig.clone());
        }
        env
    }

    /// R5 (Slice 2): the session poly-env threaded into every REPL check path,
    /// mapping each retained polymorphic word to its `PolySig` and the
    /// generation it was retained at (so `check_poly_call` mints the
    /// generation-stamped symbol, R2/R2b). Kept out of `typed_env` because a
    /// polymorphic word never enters the concrete env (R3).
    fn poly_env(&self) -> HashMap<String, (PolySig, Option<u64>)> {
        self.poly_words
            .iter()
            .map(|(name, entry)| {
                let sig = entry
                    .word
                    .poly
                    .as_deref()
                    .expect("a poly_words entry always has a polymorphic signature")
                    .clone();
                (name.clone(), (sig, Some(entry.generation)))
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
            funcs.push(ir::lower_instantiation(
                &inst.symbol,
                sig,
                &inst.subst,
                &entry.word.body,
                &entry.ir_lower_env,
                &resolve,
                regs,
                &self.arrays,
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
        }
        if matches!(tokens.first(), Some((Token::Word(w), _)) if w == "type:") {
            return self.eval_typedef(&tokens, writer);
        }
        let ctx = parser::ImportCtx {
            imports: &self.import_qualifier_module,
            selective: &self.import_selective_module,
            exports: &self.import_exports,
        };
        let mut line = parser::parse_line_with_structs(
            &tokens,
            &self.structs,
            &self.enums,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            ctx,
        )?;
        // R8c: rewrite body-position `q::w` / `q::T>field` calls to their
        // current internal (epoch-tagged) spelling before ordinary checking
        // runs; also raises R15's `not exported` for a private qualified name.
        self.rewrite_line_imports(&mut line)?;
        match line {
            // R11: a `: drop` line never enters `self.env` or gets lowered
            // under its own name; it becomes the struct's destructor, the
            // same substitution `ir::lower` performs for a compiled module.
            Line::Def(word) => {
                // R8e: a declared word name containing `::` would collide with
                // an imported name's internal tag; reject it up front (covers
                // the drop / def / poly fan-out with one check).
                reject_double_colon_name("word", &word.name, word_span(&word))?;
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
        if parser::typedef_line_is_enum(tokens) {
            self.eval_enum_typedef(tokens, name.clone(), span)?;
        } else {
            self.eval_struct_typedef(tokens, name.clone(), span)?;
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
            check::check_types(&self.structs, &self.enums, &self.arrays, &self.owned_cells)
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
        let variants = variant_names
            .into_iter()
            .map(|(vname, vspan)| VariantDecl {
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
            check::check_types(&self.structs, &self.enums, &self.arrays, &self.owned_cells)
        });
        if let Err(e) = result {
            self.enums.pop();
            return Err(e);
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
        // R3: the REPL's own top-level path resolves relative to the process
        // cwd; every transitive import inside the closure keeps 5a's
        // importer-relative rule (inside `discover_closure`).
        let closure = driver::discover_closure(Path::new(&import.path))?;
        let mut module = driver::assemble_module(&closure)?;
        check::check(&mut module)?;
        // R14/D4: an imported closure declaring `main` (in any of its files,
        // not only module 0) is rejected before any codegen, naming the file
        // and the word.
        check_no_main_in_closure(&module, &closure)?;
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
        self.splice_import(&import, &module, epoch, module_base);
        writeln!(writer, "imported {}", import.qualifier)
            .map_err(|e| format!("writing stdout: {e}"))
    }

    /// R8/R9/R15: splice module 0's exports into the session. Infallible: every
    /// error path is upstream in `eval_import`. Appends the whole closure's
    /// registries with a constant positional-id shift (R9), tags each decl
    /// with its event module id and epoch `.name` (R8a/R8b), binds exported
    /// words into `self.env` under their import-epoch symbol, records the
    /// qualifier's aliases / private names / export lists.
    fn splice_import(&mut self, import: &Import, module: &Module, epoch: u64, module_base: u32) {
        let q = &import.qualifier;
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
        self.import_aliases.retain(|k, _| !k.starts_with(&prefix));
        let struct_base = self.structs.len();
        let enum_base = self.enums.len();
        let array_base = self.arrays.len();
        let cell_base = self.owned_cells.len();
        let ref_base = self.refs.len();
        let remap =
            |ty: Type| remap_type(ty, struct_base, enum_base, array_base, cell_base, ref_base);
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
                    .insert(format!("{q}::{}", s.name_static), name);
            }
        }

        // R9: append every enum with remapped variant-field ids and module id.
        // No aliases are built this phase (enums are out of phase-1 fixtures),
        // but the ids must still remap so a later reference stays consistent.
        for (i, e) in module.enums.iter().enumerate() {
            let variants = e
                .variants
                .iter()
                .map(|v| VariantDecl {
                    name: v.name.clone(),
                    name_static: v.name_static,
                    fields: v
                        .fields
                        .iter()
                        .map(|(f, ty)| (f.clone(), remap(*ty)))
                        .collect(),
                    span: v.span,
                })
                .collect();
            self.enums.push(EnumDecl {
                name: format!("{}__import{epoch}__e{}", e.name_static, enum_base + i),
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
            let Some(w) = module
                .words
                .iter()
                .find(|w| w.module == 0 && w.poly.is_none() && w.name == mangled)
            else {
                continue; // an exported type name, handled in the struct/enum loop
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
            self.import_aliases.insert(format!("{q}::{raw}"), internal);
        }

        // R15: retain module 0's private names (bare word names, and for a
        // private type its bare name plus every accessor spelling), so a
        // `q::x` that misses the aliases can be told `not exported` rather than
        // unknown.
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
            for (f, _) in &s.fields {
                private.insert(format!("{t}>{f}"));
                private.insert(format!("{t}<{f}"));
                private.insert(format!("{t}|>{f}"));
            }
        }
        self.import_private.insert(q.clone(), private);

        // R8a/R8d: bind the qualifier to module 0's session id and record every
        // event-module's export list, the parser's type-position resolver maps.
        self.import_qualifier_module.insert(q.clone(), module_base);
        while self.import_exports.len() < (module_base + module.modules.len() as u32) as usize {
            self.import_exports.push(Vec::new());
        }
        for (m, info) in module.modules.iter().enumerate() {
            self.import_exports[module_base as usize + m] = info.exports.clone();
        }
    }

    /// R8c/R15: rewrite a just-parsed line's body-position calls, translating a
    /// user-facing `q::w` / `q::T>field` spelling to its current internal
    /// (epoch-tagged) one before ordinary checking runs, and raising R15's
    /// `not exported` for a private qualified name. Type-position references
    /// are already resolved by the parser (R8d) and are untouched here.
    fn rewrite_line_imports(&self, line: &mut Line) -> Result<(), String> {
        match line {
            Line::Expr(terms) => self.rewrite_terms_imports(terms),
            Line::Def(word) => self.rewrite_wordbody_imports(&mut word.body),
        }
    }

    fn rewrite_wordbody_imports(&self, body: &mut WordBody) -> Result<(), String> {
        match body {
            WordBody::Terms { terms } => self.rewrite_terms_imports(terms),
            WordBody::Clauses(clauses) => {
                for c in clauses.iter_mut() {
                    self.rewrite_terms_imports(&mut c.body)?;
                }
                Ok(())
            }
        }
    }

    fn rewrite_terms_imports(&self, terms: &mut [Term]) -> Result<(), String> {
        for term in terms.iter_mut() {
            match &mut term.kind {
                TermKind::Call(name) => {
                    if let Some(new) = self.rewrite_import_call(name, term.span)? {
                        *name = new;
                    }
                }
                TermKind::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    self.rewrite_terms_imports(then_branch)?;
                    self.rewrite_terms_imports(else_branch)?;
                }
                TermKind::Quotation(inner) => self.rewrite_terms_imports(inner)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// The single-call rewrite: `Some(new)` to replace the spelling, `None` to
    /// leave it (a local or a genuinely absent name falls through to the
    /// ordinary unknown-word path), `Err` for R15's `not exported`.
    fn rewrite_import_call(&self, name: &str, span: Span) -> Result<Option<String>, String> {
        let (base, suffix) = split_accessor(name);
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
                        let (base_name, _) = split_accessor(rest);
                        return Err(crate::resolve::not_exported_error(
                            base_name, qualifier, span,
                        ));
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
        let (sites, _insts) = check::check_def_collecting_drop_sites(
            &self.drop_overloads[&id].1,
            &self.enums,
            &env,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &self.structs,
            &HashMap::new(),
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
        let regs = ir::Registries {
            structs: &structs,
            enums: &enums,
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
        };
        let funcs = {
            let resolve = resolver_for(&self.env);
            ir::synthesize_aggregate_destructors(
                &ir_lower_env,
                &resolve,
                regs,
                &self.drop_override_bodies(Some(id)),
            )
        };

        let ssa = backend::qbe::emit(&IrModule {
            funcs,
            structs: structs.layouts,
            enums: enums.layouts,
            arrays: arrays.layouts,
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
        check::check_poly_body(&word, &sig, &env, &self.structs, &self.enums, &self.arrays)?;

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
        // R8: the two stores stay mutually exclusive per name (a polymorphic
        // word never enters the concrete env, R3), so defining `name` as poly
        // evicts any prior ordinary entry for it.
        self.env.remove(&name);
        self.poly_words.insert(
            name.clone(),
            PolyWordEntry {
                generation,
                word,
                resolver,
                ir_lower_env,
            },
        );
        writeln!(writer, "defined {name}").map_err(|e| format!("writing stdout: {e}"))?;
        Ok(())
    }

    fn eval_def(&mut self, word: WordDef, writer: &mut impl Write) -> Result<(), String> {
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
        let insts = check::check_def(
            &word,
            &self.enums,
            &env,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &self.structs,
            &poly_env,
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
        env.insert(name.clone(), sig.clone());
        let ir_lower_env = ir_arity_env(&env);
        let (mut structs, mut enums, arrays, mut cells, refs) = ir::build_registries(
            &self.structs,
            &self.enums,
            &self.arrays,
            &self.owned_cells,
            &self.refs,
        );
        self.apply_drop_generations(&mut structs, &mut enums, &mut cells);
        let regs = ir::Registries {
            structs: &structs,
            enums: &enums,
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
        };
        let mut funcs = {
            let resolve = resolver_with_override(&self.env, &name, &symbol);
            // R7 (Slice 2): thread the instantiation table + poly-arity map so
            // a call to a retained polymorphic word inside this body lowers to
            // its per-site symbol via `lower_poly_call`.
            let mut func =
                ir::lower_word(&word, &ir_lower_env, &resolve, regs, &insts, &poly_arities);
            func.name = symbol.clone();
            let mut funcs = vec![func];
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
            ));
            funcs
        };
        // R7 (Slice 2, D2): lower each not-yet-exported instantiation this
        // body recorded into this module, against the frozen snapshot resolver.
        funcs.extend(self.emit_instantiations(&insts, regs));

        let ssa = backend::qbe::emit(&IrModule {
            funcs,
            structs: structs.layouts,
            enums: enums.layouts,
            arrays: arrays.layouts,
        })?;
        let dir = driver::tempfile_dir()?;
        let so_path = dir.join(format!("{name}_gen{generation}.so"));
        driver::compile_so(&ssa, &so_path)?;
        let lib = Library::open(&so_path)?;

        // Only commit on success: env stays untouched on any earlier failure.
        self.libs.push(lib);
        // R8: an ordinary (re)definition evicts any prior poly entry for the
        // name, so a name lives in exactly one of the two stores at a time and
        // a later call never has to arbitrate between a poly and a concrete
        // entry for it.
        self.poly_words.remove(&name);
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
        let (structs, enums, arrays) = self.run_terms(terms, writer)?;
        let cells = self.top / 8;
        writeln!(
            writer,
            "{}",
            format_stack(
                &self.buf[..cells],
                &self.types,
                &structs.layouts,
                &enums.layouts,
                &arrays.layouts
            )
        )
        .map_err(|e| format!("writing stdout: {e}"))
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
                kind: TermKind::Call("drop".to_string()),
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
    ) -> Result<(ir::Structs, ir::Enums, ir::Arrays), String> {
        let env = self.typed_env();
        let entry_depth = self.types.len();
        // R5 (Slice 2): thread the session poly-env so a bare line can call a
        // retained polymorphic word; the relayed instantiation table drives
        // the per-site lowering below (R7).
        let poly_env = self.poly_env();
        let (net_stack, insts) = check::infer_line(
            terms,
            &self.types,
            &env,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &self.structs,
            &self.enums,
            &poly_env,
        )?;
        let net_depth = net_stack.len();

        let ir_lower_env = ir_arity_env(&env);
        let poly_arities = self.poly_arities();

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
        let regs = ir::Registries {
            structs: &structs,
            enums: &enums,
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
        };
        let (func, m, out_bytes, aggregate_destructors) = {
            let resolve = resolver_for(&self.env);
            let (func, m, out_bytes) = ir::lower_line(
                seq,
                terms,
                entry_depth,
                &self.types,
                &ir_lower_env,
                &resolve,
                regs,
                &insts,
                &poly_arities,
            );
            // R12: this line's module must carry its own struct/enum
            // destructors, or `drop` on a linear struct/enum dies at `dlopen`
            // with an undefined `sooth_struct_drop_N`/`sooth_enum_drop_N`.
            let aggregate_destructors = ir::synthesize_aggregate_destructors(
                &ir_lower_env,
                &resolve,
                regs,
                &self.drop_override_bodies(None),
            );
            (func, m, out_bytes, aggregate_destructors)
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
        funcs.extend(aggregate_destructors);
        // R7 (Slice 2, D2): lower each not-yet-exported instantiation this line
        // recorded into this module, against each poly word's frozen snapshot
        // resolver; an already-exported symbol emits nothing (trace B dedup).
        funcs.extend(self.emit_instantiations(&insts, regs));
        let ssa = backend::qbe::emit(&IrModule {
            funcs,
            structs: structs.layouts.clone(),
            enums: enums.layouts.clone(),
            arrays: arrays.layouts.clone(),
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

        Ok((structs, enums, arrays))
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

/// The read-eval-print loop: blank lines are skipped silently, `:quit` or EOF
/// exits cleanly (disposing any residual linear values first), and any stage
/// error prints the diagnostic without mutating session state.
pub fn run(mut reader: impl BufRead, mut writer: impl Write) -> Result<(), String> {
    let mut session = Session::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("reading stdin: {e}"))?;
        if n == 0 {
            return end_session(&mut session, &mut writer);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == ":quit" {
            return end_session(&mut session, &mut writer);
        }
        if let Err(e) = session.eval_line(trimmed, &mut writer) {
            writeln!(writer, "{e}").map_err(|e| format!("writing stdout: {e}"))?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend, check, driver, ir, lexer, parser};

    #[test]
    fn compiled_word_is_dlsymable_and_callable() {
        let src = ": sq ( i64 -- i64 ) | n | n n * ;";
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

    #[test]
    fn format_stack_bottom_to_top() {
        let types = vec![Type::I64, Type::I64, Type::I64];
        assert_eq!(
            format_stack(&[1, 2, 3], &types, &[], &[], &[]),
            "stack: 1 2 3"
        );
    }

    #[test]
    fn format_stack_empty_is_marker() {
        assert_eq!(format_stack(&[], &[], &[], &[], &[]), "stack: (empty)");
    }

    #[test]
    fn format_stack_f64_slot_renders_float_not_bits() {
        // A carried `f64` displays its value, not the `i64` bit pattern (R21).
        let bits = 2.5f64.to_bits() as i64;
        assert_eq!(
            format_stack(&[bits], &[Type::F64], &[], &[], &[]),
            "stack: 2.5"
        );
    }

    #[test]
    fn format_stack_f32_slot_reads_low_32_bits() {
        // An `f32` slot stores 4 bytes; display reads the low 32 bits (Q2/R21).
        let bits = 1.5f32.to_bits() as u64 as i64;
        let f32_ty = Type::from_name("f32").unwrap();
        assert_eq!(
            format_stack(&[bits], &[f32_ty], &[], &[], &[]),
            "stack: 1.5"
        );
    }

    #[test]
    fn format_stack_bool_slot_displays_as_true_or_false() {
        // Matches `.`'s print semantics: `true`/`false`, not the raw 0/1.
        assert_eq!(
            format_stack(&[1, 0], &[Type::Bool, Type::Bool], &[], &[], &[]),
            "stack: true false"
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
            format_stack(&[5, 6, 99], &[vec2, Type::I64], &layouts, &[], &[]),
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
            format_stack(&[123, 99], &[cell_ty, Type::I64], &[], &[], &[]),
            "stack: <^i64> 99"
        );
    }

    #[test]
    fn format_stack_str_slot_shows_placeholder_and_offsets_past_it() {
        // A `str` slot's descriptor address is not dereferenced for content;
        // it renders as `<str>` and offsets past its one carried cell, like a
        // cell slot.
        assert_eq!(
            format_stack(&[0, 99], &[Type::Str, Type::I64], &[], &[], &[]),
            "stack: <str> 99"
        );
    }

    #[test]
    fn format_stack_cstr_slot_shows_placeholder_and_offsets_past_it() {
        assert_eq!(
            format_stack(&[0, 99], &[Type::Cstr, Type::I64], &[], &[], &[]),
            "stack: <cstr> 99"
        );
    }

    #[test]
    fn format_stack_unsigned_slot_displays_unsigned_not_negative() {
        // A `u64` with the high bit set stores a negative `i64` bit pattern;
        // display must render its unsigned value, not that negative number.
        let u64_ty = Type::from_name("u64").unwrap();
        assert_eq!(
            format_stack(&[-1], &[u64_ty], &[], &[], &[]),
            "stack: 18446744073709551615"
        );
    }

    #[test]
    fn format_stack_usize_slot_displays_unsigned_not_negative() {
        // `Type::Usize` is a distinct variant from `Type::Int(u64)`; a
        // carried `usize` slot with the high bit set used to fall to the
        // catch-all `v.to_string()` arm and render negative.
        assert_eq!(
            format_stack(&[-1], &[Type::Usize], &[], &[], &[]),
            "stack: 18446744073709551615"
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
        let regs = ir::Registries {
            structs: &structs,
            enums: &enums,
            arrays: &arrays,
            cells: &cells,
            refs: &refs,
        };
        let env = ir_arity_env(&session.typed_env());
        let resolve = resolver_for(&session.env);
        ir::synthesize_aggregate_destructors(
            &env,
            &resolve,
            regs,
            &session.drop_override_bodies(declaring),
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
            .eval_line(": drop ( Res -- ) | r | r Res>n . ;", &mut out)
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
        format!("import: {qualifier} \"{}\" ;", path.display())
    }

    #[test]
    fn repl_assembles_checked_module_for_library() {
        // U3: `discover_closure` / `assemble_module` are reachable as
        // `pub(crate)` and yield a checked module for a library path (a
        // plumbing smoke test, no guarded invariant).
        let d = LibDir::new("u3");
        let lib = d.write("lib.sth", ": w ( -- i64 ) 42 ;\nexport: w ;\n");
        let closure = driver::discover_closure(&lib).expect("closure resolves");
        let mut module = driver::assemble_module(&closure).expect("assembles");
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
        let mut module = driver::assemble_module(&closure).expect("assembles");
        check::check(&mut module).expect("checks");
        let err = check_no_main_in_closure(&module, &closure).unwrap_err();
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
            .eval_line(": drop ( Res -- ) | r | r Res>n . ;", &mut out)
            .unwrap();
        let id = StructId::from_index(0);
        let first = destructor_symbols(&session, Some(id));
        session
            .eval_line(": drop ( Res -- ) | r | r Res>n 100 + . ;", &mut out)
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
