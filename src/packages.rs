//! Package-boundary attribution and path-derived module naming. Operates on
//! plain file paths only, with no dependency on `driver::Closure`, so it is
//! unit-testable without constructing one. The driver walks its `Closure`,
//! passes the plain path list here, and gets back a `PackageGraph` to
//! resolve imports against.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::Span;
use crate::lexer::{self, Token};
use crate::manifest::{self, Manifest};

/// For one package, a map from module name to its canonical file path.
pub type ModuleTable = HashMap<String, PathBuf>;

/// A file's package attribution: the canonical path of its nearest ancestor
/// `sooth.pkg`, or `None` if it has no ancestor manifest.
pub type PackageAttribution = HashMap<PathBuf, Option<PathBuf>>;

/// One discovered package: its manifest, the manifest's own path, its root
/// directory (the manifest's parent), and the `ModuleTable` built from every
/// input file attributed to it.
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub manifest: Manifest,
    pub manifest_path: PathBuf,
    pub root: PathBuf,
    pub modules: ModuleTable,
}

/// Result of `attribute_packages`: every discovered package, keyed by its
/// manifest path, plus each input file's attribution. Carries only
/// path-derived data, with no import data; cross-package import resolution
/// reads this as a lookup table.
#[derive(Debug, Clone, Default)]
pub struct PackageGraph {
    pub packages: HashMap<PathBuf, PackageInfo>,
    pub attribution: PackageAttribution,
}

impl PackageGraph {
    /// The `PackageInfo` owning `file`, or `None` if `file` has no ancestor
    /// manifest.
    pub fn package_for_file(&self, file: &Path) -> Option<&PackageInfo> {
        let manifest_path = self.attribution.get(file)?.as_ref()?;
        self.packages.get(manifest_path)
    }
}

/// A cross-package import that resolution declined to turn into a closure
/// edge, recorded so `check_package_graph` can report it against the
/// `depends:`/`module:` tables. Import-triggered: the OQ4-A and OQ4-C
/// diagnostics name the offending `import:`, so they need the span and the
/// names the importer actually wrote.
#[derive(Debug, Clone)]
pub struct UnresolvedImport {
    pub importer_pkg: String,
    pub importer_manifest: PathBuf,
    pub importer: PathBuf,
    pub pkg: String,
    pub module: Vec<String>,
    pub span: Span,
    pub kind: UnresolvedKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedKind {
    MissingDepends,
    PrivateModule,
}

/// The nearest ancestor directory of `file` holding a `sooth.pkg`, i.e. the
/// package root.
pub fn find_package_root(file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d.join("sooth.pkg").is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// For each file path, walk upward to locate a `sooth.pkg`, derive its
/// module name relative to that manifest's package root, and accumulate the
/// result into a `PackageGraph`. Takes only paths, no import data.
///
/// Paths in and out are used verbatim: keys are canonical only if `files`
/// is canonical, so callers keying into the result must pass the same form
/// of path they later look up.
pub fn attribute_packages(files: &[PathBuf]) -> Result<PackageGraph, String> {
    let mut graph = PackageGraph::default();
    for file in files {
        let root = find_package_root(file);
        let manifest_path = root.as_ref().map(|r| r.join("sooth.pkg"));
        graph
            .attribution
            .insert(file.clone(), manifest_path.clone());
        let (Some(root), Some(manifest_path)) = (root, manifest_path) else {
            continue;
        };
        let info = match graph.packages.entry(manifest_path) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let manifest_path = e.key().clone();
                let src = std::fs::read_to_string(&manifest_path)
                    .map_err(|e| format!("reading manifest {}: {e}", manifest_path.display()))?;
                let manifest = manifest::parse_manifest(&src, &manifest_path)?;
                e.insert(PackageInfo {
                    manifest,
                    manifest_path,
                    root: root.clone(),
                    modules: ModuleTable::new(),
                })
            }
        };
        let module_name = derive_module_name(file, &root)?;
        info.modules.insert(module_name, file.clone());
    }
    Ok(graph)
}

/// A path segment must lex, on its own, as a single `Token::Word`: not a
/// comment marker (`\`), not an int/float literal, and not more than one
/// token (a delimiter or whitespace inside the segment). Additionally,
/// `:` is rejected even though it lexes as part of an ordinary word (it
/// would let `text::ascii.sth` derive the same module name as
/// `text/ascii.sth`), and a bare `*` is rejected (reserved for S2's
/// wildcard import target).
fn check_module_segment(seg: &str, file: &Path) -> Result<(), String> {
    let tokens = lexer::lex(seg).map_err(|e| {
        format!(
            "module name error: segment `{seg}` in {}: {e}",
            file.display()
        )
    })?;
    match tokens.as_slice() {
        [(Token::Word(_), _)] => {}
        _ => {
            return Err(format!(
                "module name error: segment `{seg}` in {} is not a single identifier",
                file.display()
            ));
        }
    }
    if seg.contains(':') {
        return Err(format!(
            "module name error: segment `{seg}` in {} contains `:`, which would collide with the `text/ascii.sth` derivation of the same name",
            file.display()
        ));
    }
    if seg == "*" {
        return Err(format!(
            "module name error: segment `{seg}` in {} is reserved for the wildcard import target",
            file.display()
        ));
    }
    Ok(())
}

/// Strips the package root and the `.sth` extension, replaces `/` with
/// `::`. Errors on a file not under `pkg_root` or on a path segment that
/// is not a valid Sooth identifier (see `check_module_segment`).
pub fn derive_module_name(file: &Path, pkg_root: &Path) -> Result<String, String> {
    let rel = file.strip_prefix(pkg_root).map_err(|_| {
        format!(
            "module name error: {} is not under package root {}",
            file.display(),
            pkg_root.display()
        )
    })?;
    let mut segments = Vec::new();
    for comp in rel.components() {
        let seg = comp.as_os_str().to_str().ok_or_else(|| {
            format!(
                "module name error: non-UTF-8 path segment in {}",
                file.display()
            )
        })?;
        segments.push(seg.to_string());
    }
    let Some(last) = segments.pop() else {
        return Err(format!(
            "module name error: {} equals its package root {}",
            file.display(),
            pkg_root.display()
        ));
    };
    let last = last.strip_suffix(".sth").ok_or_else(|| {
        format!(
            "module name error: {} does not end in `.sth`",
            file.display()
        )
    })?;
    segments.push(last.to_string());

    for seg in &segments {
        check_module_segment(seg, file)?;
    }
    Ok(segments.join("::"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new(tag: &str) -> Sandbox {
            static N: AtomicU64 = AtomicU64::new(0);
            let seq = N.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("sooth-packages-{}-{tag}-{seq}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Sandbox(dir)
        }
        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, contents).unwrap();
            path
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn derive_module_name_top_level() {
        let sb = Sandbox::new("top");
        let name = derive_module_name(&sb.0.join("foo.sth"), &sb.0).unwrap();
        assert_eq!(name, "foo");
    }

    #[test]
    fn derive_module_name_nested_one() {
        let sb = Sandbox::new("nest1");
        let name = derive_module_name(&sb.0.join("text/ascii.sth"), &sb.0).unwrap();
        assert_eq!(name, "text::ascii");
    }

    #[test]
    fn derive_module_name_nested_two() {
        let sb = Sandbox::new("nest2");
        let name = derive_module_name(&sb.0.join("a/b/c.sth"), &sb.0).unwrap();
        assert_eq!(name, "a::b::c");
    }

    #[test]
    fn derive_module_name_invalid_segment_is_error() {
        let sb = Sandbox::new("invalid");
        let err = derive_module_name(&sb.0.join("my file.sth"), &sb.0).unwrap_err();
        assert!(
            err.contains("is not a single identifier"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn derive_module_name_non_word_segment_is_error() {
        let sb = Sandbox::new("nonword");
        for name in ["\\.sth", "42.sth", "3.5.sth"] {
            let err = derive_module_name(&sb.0.join(name), &sb.0).unwrap_err();
            assert!(
                err.contains("is not a single identifier"),
                "case {name}: unexpected message: {err}"
            );
        }
    }

    #[test]
    fn derive_module_name_colon_in_filename_is_error() {
        let sb = Sandbox::new("colon");
        let err = derive_module_name(&sb.0.join("text::ascii.sth"), &sb.0).unwrap_err();
        assert!(err.contains("contains `:`"), "unexpected message: {err}");
    }

    #[test]
    fn derive_module_name_star_segment_is_error() {
        let sb = Sandbox::new("star");
        let err = derive_module_name(&sb.0.join("*.sth"), &sb.0).unwrap_err();
        assert!(
            err.contains("reserved for the wildcard"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn derive_module_name_dot_in_filename_is_ok() {
        let sb = Sandbox::new("dot");
        let name = derive_module_name(&sb.0.join("ascii.io.sth"), &sb.0).unwrap();
        assert_eq!(name, "ascii.io");
    }

    #[test]
    fn derive_module_name_without_sth_extension_is_error() {
        let sb = Sandbox::new("noext");
        let err = derive_module_name(&sb.0.join("foo.txt"), &sb.0).unwrap_err();
        assert!(
            err.contains("does not end in `.sth`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn derive_module_name_not_under_root_is_error() {
        let sb = Sandbox::new("outside");
        let other = Sandbox::new("outside-root");
        let err = derive_module_name(&other.0.join("foo.sth"), &sb.0).unwrap_err();
        assert!(err.contains("is not under package root"), "{err}");
    }

    #[test]
    fn attribute_packages_no_manifest_returns_none() {
        let sb = Sandbox::new("nomanifest");
        let file = sb.write("foo.sth", "");
        let graph = attribute_packages(std::slice::from_ref(&file)).unwrap();
        assert_eq!(graph.attribution.get(&file), Some(&None));
        assert!(graph.package_for_file(&file).is_none());
    }

    #[test]
    fn attribute_packages_nested_manifest_inner_wins() {
        let sb = Sandbox::new("nested");
        sb.write("sooth.pkg", "package: outer ; layer: core ;");
        sb.write("inner/sooth.pkg", "package: inner ; layer: core ;");
        let file = sb.write("inner/foo.sth", "");
        let graph = attribute_packages(std::slice::from_ref(&file)).unwrap();

        let pkg = graph.package_for_file(&file).unwrap();
        assert_eq!(pkg.manifest.package, "inner");
        assert_eq!(pkg.manifest_path, sb.0.join("inner/sooth.pkg"));
    }

    #[test]
    fn attribute_packages_builds_module_table() {
        let sb = Sandbox::new("withmanifest");
        sb.write("sooth.pkg", "package: core ; layer: core ;");
        let a = sb.write("a.sth", "");
        let b = sb.write("text/ascii.sth", "");
        let graph = attribute_packages(&[a.clone(), b.clone()]).unwrap();

        let a_pkg = graph.package_for_file(&a).unwrap();
        let b_pkg = graph.package_for_file(&b).unwrap();
        assert_eq!(a_pkg.manifest.package, "core");
        assert_eq!(a_pkg.manifest_path, b_pkg.manifest_path);
        assert_eq!(a_pkg.modules.get("a"), Some(&a));
        assert_eq!(a_pkg.modules.get("text::ascii"), Some(&b));
    }
}
