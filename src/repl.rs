//! REPL: compile each line through the normal pipeline to a shared object and
//! `dlopen` it into the session process (no interpreter, no JIT).
//!
//! Phase 1 lands only the load-and-call mechanism; the line surface, wrapper
//! marshalling, and session loop arrive in later phases.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend, check, driver, ir, lexer, parser};

    #[test]
    fn compiled_word_is_dlsymable_and_callable() {
        let src = ": sq ( int -- int ) | n | n n * ;";
        let tokens = lexer::lex(src).unwrap();
        let module = parser::parse(&tokens).unwrap();
        check::check(&module).unwrap();
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
}
