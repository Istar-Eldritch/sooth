//! REPL: compile each line through the normal pipeline to a shared object and
//! `dlopen` it into the session process (no interpreter, no JIT).
//!
//! `Session` owns the persistent stack buffer and the word env (arity +
//! generation + symbol); the read-eval-print loop lexes/parses/checks/lowers/
//! emits/compiles/loads each line exactly like `build`, differing only in
//! target (`.so` not a binary) and in carrying state across lines.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::{BufRead, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::ast::{
    ArrayDecl, EnumDecl, Line, OwnedCellDecl, RefDecl, Span, StructDecl, StructId, Term, TermKind,
    Type, VariantDecl, WordDef,
};
use crate::check::{self, Sig};
use crate::driver;
use crate::ir::ArrayLayout;
use crate::ir::{self, EnumLayout, IrModule, StructLayout};
use crate::lexer::Token;
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
            override_epoch: None,
            buf: Vec::new(),
            top: 0,
            types: Vec::new(),
            libs: Vec::new(),
            seq: 0,
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

    /// Evaluate one line of input, writing any success output to `writer`.
    /// On error, the session (env, stack) is left untouched; the caller
    /// prints the returned diagnostic.
    fn eval_line(&mut self, src: &str, writer: &mut impl Write) -> Result<(), String> {
        let tokens = lexer::lex(src)?;
        if matches!(tokens.first(), Some((Token::Word(w), _)) if w == "type:") {
            return self.eval_typedef(&tokens, writer);
        }
        let line = parser::parse_line_with_structs(
            &tokens,
            &self.structs,
            &self.enums,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
        )?;
        match line {
            // R11: a `: drop` line never enters `self.env` or gets lowered
            // under its own name; it becomes the struct's destructor, the
            // same substitution `ir::lower` performs for a compiled module.
            Line::Def(word) if word.name == "drop" => self.eval_drop_overload(word, writer),
            Line::Def(word) => self.eval_def(word, writer),
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
        });
        let result = parser::parse_typedef_line(
            tokens,
            &self.structs,
            &self.enums,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
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
        });
        let result = parser::parse_enum_typedef_line(
            tokens,
            &self.structs,
            &self.enums,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
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
        let sites = check::check_def_collecting_drop_sites(
            &self.drop_overloads[&id].1,
            &self.enums,
            &env,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &self.structs,
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

    fn eval_def(&mut self, word: WordDef, writer: &mut impl Write) -> Result<(), String> {
        let name = word.name.clone();
        let sig = check::sig_of(&word.effect);

        let mut env = self.typed_env();
        check::check_def(
            &word,
            &self.enums,
            &env,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &self.structs,
        )?;

        let generation = next_generation(self.env.get(&name));
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
        let funcs = {
            let resolve = resolver_with_override(&self.env, &name, &symbol);
            let mut func = ir::lower_word(&word, &ir_lower_env, &resolve, regs);
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
        let net_stack = check::infer_line(
            terms,
            &self.types,
            &env,
            &mut self.arrays,
            &mut self.owned_cells,
            &mut self.refs,
            &self.structs,
            &self.enums,
        )?;
        let net_depth = net_stack.len();

        let ir_lower_env = ir_arity_env(&env);

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
}
