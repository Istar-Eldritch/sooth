//! Package boundaries and module-name import resolution: the nearest-ancestor
//! `sooth.pkg` lookup (OQ2 manifest locality), the path-join from a module name
//! to the file it names, and the located diagnostics resolution raises.
//!
//! Depends on `ast`, `lexer`, and `manifest` only. The driver hands one
//! `Import` in at a time and gets a path back, so `Closure` and the walk that
//! builds it stay entirely the driver's concern.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::ast::{Import, ImportAnchor, ImportTarget, ModuleName, Span};
use crate::lexer::{self, Token};
use crate::manifest::{self, DependsEntry, Manifest, PackageLayer};

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
    /// The target package's manifest, for the OQ4-C remedy line. `None` for
    /// `MissingDepends`, where no dependency manifest was ever resolved.
    pub pkg_manifest: Option<PathBuf>,
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

/// One package as import resolution sees it: where its manifest is, the root
/// its module names are relative to, and what that manifest declares.
pub(crate) struct PackageSite {
    manifest_path: PathBuf,
    root: PathBuf,
    manifest: Manifest,
}

impl PackageSite {
    fn new(manifest_path: PathBuf, manifest: Manifest) -> PackageSite {
        let root = manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        PackageSite {
            manifest_path,
            root,
            manifest,
        }
    }

    /// The file a module name path-joins to inside this package: the root, the
    /// name's segments as directories, and `.sth` on the last one. The
    /// extension is appended rather than `set_extension`ed, which would eat a
    /// `.` inside the segment itself (`ascii.io` -> `ascii.sth`).
    fn module_file(&self, segments: &[String]) -> PathBuf {
        let mut path = self.root.clone();
        let (last, dirs) = segments
            .split_last()
            .expect("a parsed module name has at least one segment");
        for dir in dirs {
            path.push(dir);
        }
        path.push(format!("{last}.sth"));
        path
    }
}

/// Nearest-ancestor `sooth.pkg` lookup for one build, parsing each manifest at
/// most once however many of the closure's files it owns.
#[derive(Default)]
pub(crate) struct ManifestCache {
    manifests: BTreeMap<PathBuf, Manifest>,
    #[cfg(test)]
    pub(crate) parses: usize,
}

impl ManifestCache {
    /// Every manifest path the cache has already parsed, sorted so
    /// `check_package_graph`'s walk order (and so its error choice, when more
    /// than one manifest has a defect) is deterministic.
    fn known_manifest_paths(&self) -> Vec<PathBuf> {
        self.manifests.keys().cloned().collect()
    }

    fn load(&mut self, manifest_path: &Path) -> Result<&Manifest, String> {
        if !self.manifests.contains_key(manifest_path) {
            let src = std::fs::read_to_string(manifest_path)
                .map_err(|e| format!("reading manifest {}: {e}", manifest_path.display()))?;
            let parsed = manifest::parse_manifest(&src, manifest_path)?;
            #[cfg(test)]
            {
                self.parses += 1;
            }
            self.manifests.insert(manifest_path.to_path_buf(), parsed);
        }
        Ok(&self.manifests[manifest_path])
    }

    /// The package owning `file`: its nearest ancestor manifest (OQ2 manifest
    /// locality, so an inner manifest wins over an outer one), or `None` for a
    /// file with no ancestor manifest, where quoted-path imports still work.
    pub(crate) fn package_of(&mut self, file: &Path) -> Result<Option<PackageSite>, String> {
        let Some(root) = find_package_root(file) else {
            return Ok(None);
        };
        let manifest_path = root.join("sooth.pkg");
        let manifest = self.load(&manifest_path)?.clone();
        Ok(Some(PackageSite::new(manifest_path, manifest)))
    }
}

/// R5: a resolved import path that does not exist or cannot be read is an error
/// naming the importing site (the `import:` line/col) and the path, distinct
/// from a lex/parse error on the target file.
fn missing_import_error(importer: &Path, imp: &Import) -> String {
    format!(
        "error: cannot read import `{}` at line {}, col {} (imported by {})",
        imp.target.render(),
        imp.span.line,
        imp.span.col,
        importer.display()
    )
}

/// The header every module-name resolution diagnostic shares (OQ4): the import
/// as written, its location, and the importing file.
fn import_header(importer: &Path, imp: &Import) -> String {
    format!(
        "error: import `{}` at line {}, col {} in {}:",
        imp.target.render(),
        imp.span.line,
        imp.span.col,
        importer.display()
    )
}

/// OQ4 failure mode D1: no file is where the module name joins to.
fn module_not_found_error(
    importer: &Path,
    imp: &Import,
    pkg: &str,
    module: &str,
    tried: &Path,
) -> String {
    format!(
        "{}\n  package `{pkg}` has no module `{module}` (looked for {})",
        import_header(importer, imp),
        tried.display()
    )
}

/// OQ4 failure mode D2: the joined path exists, but a more deeply nested
/// manifest owns it (OQ2 manifest locality), so it is not a module of the
/// package the import named.
fn nested_package_error(
    importer: &Path,
    imp: &Import,
    pkg: &str,
    module: &str,
    tried: &Path,
    inner_manifest: &Path,
) -> String {
    format!(
        "{}\n  package `{pkg}` has no module `{module}`: `{}` belongs to the nested package rooted at `{}`, not `{pkg}`",
        import_header(importer, imp),
        tried.display(),
        inner_manifest.display()
    )
}

/// OQ4: a Dependency-anchored target with no segments past the package name
/// identifies a package, not a module. Raised ahead of the `depends:` lookup so
/// a typo'd `import: core ;` says what is actually wrong even when a
/// `depends: core ...` entry does exist.
fn bare_package_name_error(importer: &Path, imp: &Import, pkg: &str) -> String {
    format!(
        "{}\n  `{pkg}` names a package, not a module -- import one of its `module:` entries instead",
        import_header(importer, imp)
    )
}

/// A file inside a package names its imports by module name; the quoted-path
/// form survives only for files with no ancestor manifest (S1b territory).
fn quoted_path_in_package_error(importer: &Path, imp: &Import, pkg: &str) -> String {
    format!(
        "error: quoted-path import at line {}, col {} in {}:\n  file is in package `{pkg}`: use a module name (`self::<name>`, or `<pkg>::<name>` for a dependency) instead",
        imp.span.line,
        imp.span.col,
        importer.display()
    )
}

/// A module name has no package to resolve against outside a package: there is
/// no `depends:` table and no package root to join to.
fn module_import_without_manifest_error(importer: &Path, imp: &Import) -> String {
    format!(
        "{}\n  {} has no `sooth.pkg` ancestor, so a module name has no package to resolve against\n  add a manifest, or use a quoted-path import for now",
        import_header(importer, imp),
        importer.display()
    )
}

/// A `depends:` entry whose path holds no manifest, located to the entry.
fn depends_manifest_missing_error(
    manifest_path: &Path,
    entry: &DependsEntry,
    tried: &Path,
) -> String {
    format!(
        "error: `depends:` entry `{}` at line {}, col {} in {}:\n  no manifest at {}",
        entry.pkg_name,
        entry.span.line,
        entry.span.col,
        manifest_path.display(),
        tried.display()
    )
}

/// OQ4-A: a cross-package import naming a package the importer's manifest has
/// no `depends:` entry for. Import-triggered, so it is worded from the
/// recorded `UnresolvedImport` rather than the manifest graph alone. `<path>`
/// in the remedy line is a literal placeholder: no dependency manifest was
/// ever located for `u.pkg`, so there is no real path to suggest.
fn missing_depends_error(u: &UnresolvedImport) -> String {
    format!(
        "error: import `{}::{}` at line {}, col {} in {}:\n  package `{}` has no `depends:` entry for `{}`\n  add `depends: {} path \"<path>\" ;` to {}",
        u.pkg,
        u.module.join("::"),
        u.span.line,
        u.span.col,
        u.importer.display(),
        u.importer_pkg,
        u.pkg,
        u.pkg,
        u.importer_manifest.display()
    )
}

/// OQ4-C: a cross-package import naming a module the target package has not
/// listed in its `module:` public list.
fn private_module_error(u: &UnresolvedImport) -> String {
    let module = u.module.join("::");
    format!(
        "error: import `{}::{module}` at line {}, col {} in {}:\n  module `{module}` is not in `{}`'s public `module:` list\n  add `module: {module} ;` to {} to make it public",
        u.pkg,
        u.span.line,
        u.span.col,
        u.importer.display(),
        u.pkg,
        u.pkg_manifest
            .as_ref()
            .expect("PrivateModule always records the target's manifest")
            .display()
    )
}

/// OQ4-B: a `depends:` entry naming a package in a strictly higher layer.
/// Manifest-declared: fires from the `depends:` line itself, whether or not
/// anything actually imports across it. Located to the `depends:` entry's
/// span within the declaring manifest.
fn layer_violation_error(
    manifest_path: &Path,
    entry: &DependsEntry,
    importer_pkg: &str,
    importer_layer: PackageLayer,
    dep_pkg: &str,
    dep_layer: PackageLayer,
) -> String {
    format!(
        "error: layer violation in {}, line {}, col {}:\n  package `{importer_pkg}` is layer `{}` but depends on `{dep_pkg}` which is layer `{}`\n  a `{}` package may only depend on packages at the same layer or below",
        manifest_path.display(),
        entry.span.line,
        entry.span.col,
        importer_layer.name(),
        dep_layer.name(),
        importer_layer.name()
    )
}

/// A `depends:` entry naming a package whose own manifest declares a different
/// `package:` name: the entry's name never resolves to anything, since
/// resolution matches on the declared name, not the entry's spelling.
fn depends_name_mismatch_error(
    manifest_path: &Path,
    entry: &DependsEntry,
    actual_pkg: &str,
) -> String {
    format!(
        "error: `depends:` entry names `{}` at line {}, col {} in {}:\n  that package declares `package: {actual_pkg}` -- rename the entry to match",
        entry.pkg_name,
        entry.span.line,
        entry.span.col,
        manifest_path.display()
    )
}

/// Audits the whole package graph a build's `discover_closure` walk touched:
/// every `depends:` entry of every manifest reachable from one the walk
/// loaded (OQ4-B, and the `depends:` name-mismatch case), whether or not
/// anything actually imports across it, then every cross-package import
/// resolution declined to turn into an edge (`unresolved`, OQ4-A/OQ4-C). The
/// manifest walk runs first: those checks are the root cause when a `depends:`
/// entry is both an unimported layer violation and, separately, the source of
/// an import that could not resolve, and running it first also makes Golden
/// 2's outcome independent of whether its fixture's import line is present.
/// Runs after `discover_closure`, before `assemble_module`.
pub(crate) fn check_package_graph(
    manifests: &mut ManifestCache,
    unresolved: &[UnresolvedImport],
) -> Result<(), String> {
    // A worklist, not a one-shot iteration over the manifests the walk had
    // already loaded: a `depends:` entry may name a package `discover_closure`
    // never had reason to load (nothing imported from it), and that package's
    // own `depends:` entries are still part of the graph this audits. Seeded
    // from `known_manifest_paths()` (sorted, so the walk order -- and so which
    // of several defects is reported first -- is deterministic) and grown as
    // dependency manifests are loaded.
    let mut visited: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: VecDeque<PathBuf> = manifests.known_manifest_paths().into_iter().collect();
    for path in &queue {
        visited.insert(path.clone());
    }
    while let Some(importer_path) = queue.pop_front() {
        let importer = manifests.manifests[&importer_path].clone();
        let importer_root = importer_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        for entry in &importer.depends {
            let tried = importer_root.join(&entry.path).join("sooth.pkg");
            let dep_manifest_path = std::fs::canonicalize(&tried)
                .map_err(|_| depends_manifest_missing_error(&importer_path, entry, &tried))?;
            let dep = manifests.load(&dep_manifest_path)?.clone();
            if dep.package != entry.pkg_name {
                return Err(depends_name_mismatch_error(
                    &importer_path,
                    entry,
                    &dep.package,
                ));
            }
            if dep.layer > importer.layer {
                return Err(layer_violation_error(
                    &importer_path,
                    entry,
                    &importer.package,
                    importer.layer,
                    &dep.package,
                    dep.layer,
                ));
            }
            if visited.insert(dep_manifest_path.clone()) {
                queue.push_back(dep_manifest_path);
            }
        }
    }
    if let Some(u) = unresolved.first() {
        return match u.kind {
            UnresolvedKind::MissingDepends => Err(missing_depends_error(u)),
            UnresolvedKind::PrivateModule => Err(private_module_error(u)),
        };
    }
    Ok(())
}

/// OQ2: why a module-name segment cannot name a module, or `None` if it can.
///
/// A segment must lex, on its own, as a single `Token::Word`, so the file it
/// path-joins to carries a name Sooth can actually write: not a comment marker
/// (`\`), not an int or float literal (`42`, `3.5`), not several tokens (a
/// delimiter or whitespace inside it). Two further exclusions the token rule
/// admits on its own: `:`, which `::` already claims as the segment separator,
/// and a bare `*`, reserved for the wildcard import target (OQ3). A `.` is
/// deliberately not excluded: `ascii.io` names `ascii.io.sth` and collides with
/// nothing, since only one trailing `.sth` is ever appended.
fn segment_defect(seg: &str) -> Option<String> {
    if seg.contains(':') {
        return Some(format!(
            "module-name segment `{seg}` contains `:`, which is reserved for the `::` separator"
        ));
    }
    if seg == "*" {
        return Some(format!(
            "module-name segment `{seg}` is reserved for the wildcard import target"
        ));
    }
    match lexer::lex(seg).as_deref() {
        Ok([(Token::Word(_), _)]) => None,
        _ => Some(format!(
            "module-name segment `{seg}` is not a single identifier"
        )),
    }
}

/// OQ2, on the import target rather than the file: a module name is checked
/// before it is path-joined, so a file whose name breaks the rule (`42.sth`,
/// `*.sth`) is unnameable rather than quietly importable.
fn check_module_name(importer: &Path, imp: &Import, name: &ModuleName) -> Result<(), String> {
    match name.segments.iter().find_map(|s| segment_defect(s)) {
        Some(defect) => Err(format!("{}\n  {defect}", import_header(importer, imp))),
        None => Ok(()),
    }
}

/// The canonical path of an existing module file, or `None` when nothing is
/// there (D1). A directory is not a module however it is named, so the
/// `is_file` test comes before canonicalization.
fn existing_module_file(tried: &Path) -> Option<PathBuf> {
    if !tried.is_file() {
        return None;
    }
    std::fs::canonicalize(tried).ok()
}

/// The manifest owning `file`, by OQ2's nearest-ancestor rule.
fn owning_manifest_of(file: &Path) -> Option<PathBuf> {
    find_package_root(file).map(|root| root.join("sooth.pkg"))
}

/// Resolve one import to the file it names. `None` means the import adds no
/// closure edge: the reserved `intrinsics` name (F6), or a cross-package import
/// recorded in `unresolved` for `check_package_graph` to word later.
pub(crate) fn resolve_import(
    importer: &Path,
    importer_dir: &Path,
    imp: &Import,
    site: Option<&PackageSite>,
    manifests: &mut ManifestCache,
    unresolved: &mut Vec<UnresolvedImport>,
) -> Result<Option<PathBuf>, String> {
    let name = match &imp.target {
        ImportTarget::Path(path) => {
            if let Some(site) = site {
                return Err(quoted_path_in_package_error(
                    importer,
                    imp,
                    &site.manifest.package,
                ));
            }
            let raw = importer_dir.join(path);
            return std::fs::canonicalize(&raw)
                .map(Some)
                .map_err(|_| missing_import_error(importer, imp));
        }
        ImportTarget::Module(name) => name,
    };
    // Well-formedness of the name itself, ahead of every lookup: an ill-formed
    // segment names nothing under any anchor.
    check_module_name(importer, imp, name)?;
    // F6: the reserved name is matched ahead of any anchor-based lookup, so it
    // never reaches a `depends:` lookup and so can never fail one.
    // `self::intrinsics` is not it: that is an ordinary own-package module name.
    if name.anchor == ImportAnchor::Dependency && name.segments == ["intrinsics"] {
        return Ok(None);
    }
    let Some(site) = site else {
        return Err(module_import_without_manifest_error(importer, imp));
    };
    match name.anchor {
        ImportAnchor::SelfPackage => resolve_self_module(importer, imp, name, site),
        ImportAnchor::Dependency => {
            resolve_dependency_module(importer, imp, name, site, manifests, unresolved)
        }
    }
}

/// F2: a `self::` target names a module of the importing file's own package,
/// package-root-relative. `module:` visibility is never consulted here -- the
/// public/private distinction applies only across a package boundary.
fn resolve_self_module(
    importer: &Path,
    imp: &Import,
    name: &ModuleName,
    site: &PackageSite,
) -> Result<Option<PathBuf>, String> {
    let module = name.segments.join("::");
    let tried = site.module_file(&name.segments);
    let file = existing_module_file(&tried).ok_or_else(|| {
        module_not_found_error(importer, imp, &site.manifest.package, &module, &tried)
    })?;
    match owning_manifest_of(&file) {
        Some(owner) if owner == site.manifest_path => Ok(Some(file)),
        Some(owner) => Err(nested_package_error(
            importer,
            imp,
            &site.manifest.package,
            &module,
            &tried,
            &owner,
        )),
        None => Err(module_not_found_error(
            importer,
            imp,
            &site.manifest.package,
            &module,
            &tried,
        )),
    }
}

/// F2: a bare first segment names a `depends:` entry, and the rest names a
/// module of that dependency. A missing entry and a non-public module are audit
/// failures recorded for `check_package_graph`; a module that is not there at
/// all, or that a nested manifest owns, is a locate failure raised here.
fn resolve_dependency_module(
    importer: &Path,
    imp: &Import,
    name: &ModuleName,
    site: &PackageSite,
    manifests: &mut ManifestCache,
    unresolved: &mut Vec<UnresolvedImport>,
) -> Result<Option<PathBuf>, String> {
    let (dep_name, segments) = name
        .segments
        .split_first()
        .expect("a parsed module name has at least one segment");
    if segments.is_empty() {
        return Err(bare_package_name_error(importer, imp, dep_name));
    }
    let module = segments.join("::");
    let mut record = |kind, pkg_manifest| {
        unresolved.push(UnresolvedImport {
            importer_pkg: site.manifest.package.clone(),
            importer_manifest: site.manifest_path.clone(),
            importer: importer.to_path_buf(),
            pkg: dep_name.clone(),
            module: segments.to_vec(),
            span: imp.span,
            kind,
            pkg_manifest,
        })
    };
    let Some(entry) = site
        .manifest
        .depends
        .iter()
        .find(|d| d.pkg_name == *dep_name)
    else {
        record(UnresolvedKind::MissingDepends, None);
        return Ok(None);
    };
    let tried_manifest = site.root.join(&entry.path).join("sooth.pkg");
    let dep_manifest = std::fs::canonicalize(&tried_manifest)
        .map_err(|_| depends_manifest_missing_error(&site.manifest_path, entry, &tried_manifest))?;
    let dep = PackageSite::new(dep_manifest.clone(), manifests.load(&dep_manifest)?.clone());
    let tried = dep.module_file(segments);
    let file = existing_module_file(&tried).ok_or_else(|| {
        module_not_found_error(importer, imp, &dep.manifest.package, &module, &tried)
    })?;
    if !dep.manifest.modules.contains(&module) {
        record(
            UnresolvedKind::PrivateModule,
            Some(dep.manifest_path.clone()),
        );
        return Ok(None);
    }
    match owning_manifest_of(&file) {
        Some(owner) if owner == dep.manifest_path => Ok(Some(file)),
        Some(owner) => Err(nested_package_error(
            importer,
            imp,
            &dep.manifest.package,
            &module,
            &tried,
            &owner,
        )),
        None => Err(module_not_found_error(
            importer,
            imp,
            &dep.manifest.package,
            &module,
            &tried,
        )),
    }
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

    /// A module name path-joins one directory per leading segment and appends
    /// `.sth` to the last: `text::ascii` is `text/ascii.sth`.
    #[test]
    fn module_file_joins_segments_under_the_root() {
        let site = PackageSite::new(
            PathBuf::from("/pkg/sooth.pkg"),
            manifest::parse_manifest("package: p ; layer: core ;", Path::new("/pkg/sooth.pkg"))
                .unwrap(),
        );
        assert_eq!(
            site.module_file(&["text".to_string(), "ascii".to_string()]),
            PathBuf::from("/pkg/text/ascii.sth")
        );
    }

    /// OQ2: `.sth` is appended, not `set_extension`ed, so a `.` inside the
    /// last segment survives (`ascii.io` is `ascii.io.sth`, not `ascii.sth`).
    #[test]
    fn module_file_appends_extension_keeping_a_dotted_segment() {
        let site = PackageSite::new(
            PathBuf::from("/pkg/sooth.pkg"),
            manifest::parse_manifest("package: p ; layer: core ;", Path::new("/pkg/sooth.pkg"))
                .unwrap(),
        );
        assert_eq!(
            site.module_file(&["ascii.io".to_string()]),
            PathBuf::from("/pkg/ascii.io.sth")
        );
    }

    /// OQ2: the segments that lex as a single `Token::Word` are exactly the
    /// ones that can name a module.
    #[test]
    fn module_segment_single_word_is_ok() {
        for seg in ["text", "ascii", "foo?", "ascii.io", "a-b", "+"] {
            assert_eq!(segment_defect(seg), None, "segment `{seg}` should be legal");
        }
    }

    /// OQ2: a segment that lexes as anything but one `Token::Word` -- a
    /// comment marker, an int, a float, or more than one token -- names no
    /// module.
    #[test]
    fn module_segment_non_word_is_rejected() {
        for seg in ["\\", "42", "3.5", "my file", "a;b", "(", "\"q\""] {
            let defect =
                segment_defect(seg).unwrap_or_else(|| panic!("segment `{seg}` should be rejected"));
            assert!(
                defect.contains("is not a single identifier"),
                "segment `{seg}`: unexpected reason: {defect}"
            );
        }
    }

    /// OQ2: `:` lexes as part of an ordinary word, so the token rule admits it;
    /// it is excluded separately because `::` is the segment separator.
    #[test]
    fn module_segment_colon_is_rejected() {
        let defect = segment_defect("text:ascii").expect("a `:` segment is rejected");
        assert!(
            defect.contains("contains `:`, which is reserved for the `::` separator"),
            "unexpected reason: {defect}"
        );
    }

    /// OQ2/OQ3: a bare `*` lexes as an ordinary word too, and is reserved for
    /// the wildcard import target.
    #[test]
    fn module_segment_star_is_rejected() {
        let defect = segment_defect("*").expect("a bare `*` segment is rejected");
        assert!(
            defect.contains("is reserved for the wildcard import target"),
            "unexpected reason: {defect}"
        );
    }

    fn unresolved(
        kind: UnresolvedKind,
        importer_pkg: &str,
        pkg: &str,
        module: &[&str],
        span: Span,
        pkg_manifest: Option<&str>,
    ) -> UnresolvedImport {
        UnresolvedImport {
            importer_pkg: importer_pkg.to_string(),
            importer_manifest: PathBuf::from("/irrelevant/sooth.pkg"),
            importer: PathBuf::from("/irrelevant/main.sth"),
            pkg: pkg.to_string(),
            module: module.iter().map(|s| s.to_string()).collect(),
            span,
            kind,
            pkg_manifest: pkg_manifest.map(PathBuf::from),
        }
    }

    /// OQ4-A: a consumer importing `pkg::mod` where the importer's manifest
    /// has no `depends:` entry for `pkg`. Mutation-test by deleting the check.
    #[test]
    fn check_package_graph_missing_depends_is_error() {
        let u = unresolved(
            UnresolvedKind::MissingDepends,
            "app",
            "collections",
            &["vec"],
            Span {
                line: 3,
                col: 5,
                module: 0,
            },
            None,
        );
        let mut manifests = ManifestCache::default();
        let err = check_package_graph(&mut manifests, &[u]).unwrap_err();
        assert_eq!(
            err,
            "error: import `collections::vec` at line 3, col 5 in /irrelevant/main.sth:\n\
             \x20 package `app` has no `depends:` entry for `collections`\n\
             \x20 add `depends: collections path \"<path>\" ;` to /irrelevant/sooth.pkg"
        );
    }

    /// OQ4-C: a consumer importing `pkg::private` where `private` is absent
    /// from `pkg`'s `module:` list. Mutation-test by deleting the check.
    #[test]
    fn check_package_graph_private_module_is_error() {
        let u = unresolved(
            UnresolvedKind::PrivateModule,
            "app",
            "core",
            &["detail"],
            Span {
                line: 2,
                col: 1,
                module: 0,
            },
            Some("/dep/sooth.pkg"),
        );
        let mut manifests = ManifestCache::default();
        let err = check_package_graph(&mut manifests, &[u]).unwrap_err();
        assert_eq!(
            err,
            "error: import `core::detail` at line 2, col 1 in /irrelevant/main.sth:\n\
             \x20 module `detail` is not in `core`'s public `module:` list\n\
             \x20 add `module: detail ;` to /dep/sooth.pkg to make it public"
        );
    }

    /// OQ4-B: a `core`-layer package `depends:` on a `hosted`-layer package,
    /// manifest-declared -- fires whether or not anything imports across it,
    /// so no import is involved in this fixture. Mutation-test by deleting
    /// the check.
    #[test]
    fn check_package_graph_layer_violation_is_error() {
        let sb = Sandbox::new("layer-violation");
        let core_manifest = sb.write(
            "core/sooth.pkg",
            r#"package: core ; layer: core ; depends: app path "../app" ;"#,
        );
        let core_main = sb.write("core/main.sth", "");
        sb.write("app/sooth.pkg", "package: app ; layer: hosted ;");
        let mut manifests = ManifestCache::default();
        manifests.package_of(&core_main).unwrap();
        let err = check_package_graph(&mut manifests, &[]).unwrap_err();
        assert_eq!(
            err,
            format!(
                "error: layer violation in {}, line 1, col 31:\n\
                 \x20 package `core` is layer `core` but depends on `app` which is layer `hosted`\n\
                 \x20 a `core` package may only depend on packages at the same layer or below",
                core_manifest.display()
            )
        );
    }

    /// OQ4-B's boundary: two `core`-layer packages depending on each other is
    /// legal. Not a guard-deletion test (deleting the layer check entirely
    /// still leaves this passing); its only real mutation is `>` flipped to
    /// `>=`.
    #[test]
    fn check_package_graph_layer_equal_is_ok() {
        let sb = Sandbox::new("layer-equal");
        sb.write(
            "core/sooth.pkg",
            r#"package: core ; layer: core ; depends: sibling path "../sibling" ;"#,
        );
        let core_main = sb.write("core/main.sth", "");
        sb.write("sibling/sooth.pkg", "package: sibling ; layer: core ;");
        let mut manifests = ManifestCache::default();
        manifests.package_of(&core_main).unwrap();
        check_package_graph(&mut manifests, &[]).expect("equal-layer depends is legal");
    }

    /// A `depends:` entry naming `foo` whose own manifest declares `package:
    /// bar` never resolves to anything -- resolution matches on the declared
    /// name, not the entry's spelling. Mutation-test by deleting the check.
    #[test]
    fn check_package_graph_depends_name_mismatch_is_error() {
        let sb = Sandbox::new("name-mismatch");
        sb.write(
            "core/sooth.pkg",
            r#"package: core ; layer: core ; depends: text path "../other" ;"#,
        );
        let core_main = sb.write("core/main.sth", "");
        sb.write("other/sooth.pkg", "package: nottext ; layer: core ;");
        let mut manifests = ManifestCache::default();
        manifests.package_of(&core_main).unwrap();
        let err = check_package_graph(&mut manifests, &[]).unwrap_err();
        assert!(
            err.contains("`depends:` entry names `text`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("that package declares `package: nottext`"),
            "unexpected message: {err}"
        );
    }
}
