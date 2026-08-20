//! Cross-package import bookkeeping shared with the driver: the nearest-
//! ancestor `sooth.pkg` lookup (OQ2 manifest locality), and the record of a
//! cross-package import resolution declined to turn into a closure edge.

use std::path::{Path, PathBuf};

use crate::ast::Span;

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
    fn find_package_root_no_manifest_returns_none() {
        let sb = Sandbox::new("nomanifest");
        let file = sb.write("foo.sth", "");
        assert_eq!(find_package_root(&file), None);
    }

    #[test]
    fn find_package_root_nested_manifest_inner_wins() {
        let sb = Sandbox::new("nested");
        sb.write("sooth.pkg", "package: outer ; layer: core ;");
        sb.write("inner/sooth.pkg", "package: inner ; layer: core ;");
        let file = sb.write("inner/foo.sth", "");
        assert_eq!(find_package_root(&file), Some(sb.0.join("inner")));
    }
}
