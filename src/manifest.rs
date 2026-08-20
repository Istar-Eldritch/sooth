//! Manifest (`sooth.pkg`) parsing: a package's declared name, layer,
//! dependencies, and public module list. Tokenised by the same
//! `lexer::lex` that tokenises `.sth` source, driven by a dedicated
//! keyword-loop parser -- not `parser::parse_bodies` -- since a manifest is
//! a flat declaration list with no word-declaration machinery involved.

use std::path::{Path, PathBuf};

use crate::ast::Span;
use crate::lexer::{self, Token};

/// The four fixed layers, in strict total order (`Core < Fixed < Alloc <
/// Hosted`). A fixed enum rather than a declared ordering: the layer check
/// is O(1) per `depends:` entry and every diagnostic can name a layer by a
/// stable word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PackageLayer {
    Core,
    Fixed,
    Alloc,
    Hosted,
}

impl PackageLayer {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "core" => Some(Self::Core),
            "fixed" => Some(Self::Fixed),
            "alloc" => Some(Self::Alloc),
            "hosted" => Some(Self::Hosted),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Fixed => "fixed",
            Self::Alloc => "alloc",
            Self::Hosted => "hosted",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DependsEntry {
    pub pkg_name: String,
    /// The raw quoted path string, unresolved: interpreting it relative to
    /// the declaring manifest's directory is a caller concern (`packages.rs`).
    pub path: PathBuf,
    /// `span.module` is always `0`: manifests are never part of the
    /// `.sth` module closure, so there is no real module id to stamp.
    /// Do not route this span through a module-id-keyed lookup (e.g. a
    /// closure's `path_of`) -- it would silently resolve to module 0's file.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub package: String,
    pub layer: PackageLayer,
    pub depends: Vec<DependsEntry>,
    pub modules: Vec<String>,
}

fn loc(span: &Span) -> String {
    format!("line {}, col {}", span.line, span.col)
}

fn expect_word(
    tokens: &[(Token, Span)],
    pos: &mut usize,
    what: &str,
    path: &Path,
) -> Result<(String, Span), String> {
    match tokens.get(*pos) {
        Some((Token::Word(w), span)) => {
            let w = w.clone();
            let span = *span;
            *pos += 1;
            Ok((w, span))
        }
        Some((tok, span)) => Err(format!(
            "manifest error: expected {what}, found {tok:?} at {} in {}",
            loc(span),
            path.display()
        )),
        None => Err(format!(
            "manifest error: expected {what}, found end of file in {}",
            path.display()
        )),
    }
}

fn expect_str(
    tokens: &[(Token, Span)],
    pos: &mut usize,
    what: &str,
    path: &Path,
) -> Result<String, String> {
    match tokens.get(*pos) {
        Some((Token::Str(s), _)) => {
            let s = s.clone();
            *pos += 1;
            Ok(s)
        }
        Some((tok, span)) => Err(format!(
            "manifest error: expected {what}, found {tok:?} at {} in {}",
            loc(span),
            path.display()
        )),
        None => Err(format!(
            "manifest error: expected {what}, found end of file in {}",
            path.display()
        )),
    }
}

fn expect_semicolon(
    tokens: &[(Token, Span)],
    pos: &mut usize,
    decl: &str,
    path: &Path,
) -> Result<(), String> {
    match tokens.get(*pos) {
        Some((Token::Semicolon, _)) => {
            *pos += 1;
            Ok(())
        }
        Some((tok, span)) => Err(format!(
            "manifest error: expected `;` terminating `{decl}`, found {tok:?} at {} in {}",
            loc(span),
            path.display()
        )),
        None => Err(format!(
            "manifest error: expected `;` terminating `{decl}`, found end of file in {}",
            path.display()
        )),
    }
}

/// Parse a `sooth.pkg` manifest: `package:` and `layer:` are mandatory,
/// `depends:` and `module:` are optional and accumulate. `path` is used only
/// for locating error messages.
pub fn parse_manifest(src: &str, path: &Path) -> Result<Manifest, String> {
    let tokens = lexer::lex(src).map_err(|e| format!("{e} in manifest {}", path.display()))?;
    let mut pos = 0;
    let mut package: Option<String> = None;
    let mut layer: Option<PackageLayer> = None;
    let mut depends = Vec::new();
    let mut modules = Vec::new();

    while pos < tokens.len() {
        let (tok, span) = &tokens[pos];
        let Token::Word(kw) = tok else {
            return Err(format!(
                "manifest error: expected a declaration keyword, found {tok:?} at {} in {}",
                loc(span),
                path.display()
            ));
        };
        match kw.as_str() {
            "package:" => {
                if package.is_some() {
                    return Err(format!(
                        "manifest error: duplicate `package:` at {} in {}",
                        loc(span),
                        path.display()
                    ));
                }
                pos += 1;
                let (name, _) = expect_word(&tokens, &mut pos, "a package name", path)?;
                expect_semicolon(&tokens, &mut pos, "package:", path)?;
                package = Some(name);
            }
            "layer:" => {
                if layer.is_some() {
                    return Err(format!(
                        "manifest error: duplicate `layer:` at {} in {}",
                        loc(span),
                        path.display()
                    ));
                }
                pos += 1;
                let (name, name_span) = expect_word(&tokens, &mut pos, "a layer name", path)?;
                let value = PackageLayer::from_name(&name).ok_or_else(|| {
                    format!(
                        "manifest error: unknown layer `{name}` at {} in {}: expected one of core, fixed, alloc, hosted",
                        loc(&name_span),
                        path.display()
                    )
                })?;
                expect_semicolon(&tokens, &mut pos, "layer:", path)?;
                layer = Some(value);
            }
            "depends:" => {
                let depends_span = *span;
                pos += 1;
                let (pkg_name, pkg_span) =
                    expect_word(&tokens, &mut pos, "a dependency package name", path)?;
                if pkg_name == "intrinsics" {
                    return Err(format!(
                        "manifest error: `depends: intrinsics` at {} in {}: `intrinsics` is compiler-provided and needs no `depends:` entry",
                        loc(&pkg_span),
                        path.display()
                    ));
                }
                let (kw2, kw2_span) = expect_word(&tokens, &mut pos, "`path`", path)?;
                if kw2 != "path" {
                    return Err(format!(
                        "manifest error: expected `path` after dependency name `{pkg_name}`, found `{kw2}` at {} in {}",
                        loc(&kw2_span),
                        path.display()
                    ));
                }
                let dep_path = expect_str(&tokens, &mut pos, "a quoted dependency path", path)?;
                expect_semicolon(&tokens, &mut pos, "depends:", path)?;
                depends.push(DependsEntry {
                    pkg_name,
                    path: PathBuf::from(dep_path),
                    span: depends_span,
                });
            }
            "module:" => {
                pos += 1;
                while let Some((Token::Word(w), _)) = tokens.get(pos) {
                    modules.push(w.clone());
                    pos += 1;
                }
                expect_semicolon(&tokens, &mut pos, "module:", path)?;
            }
            other => {
                return Err(format!(
                    "manifest error: unknown declaration `{other}` at {} in {}",
                    loc(span),
                    path.display()
                ));
            }
        }
    }

    let package = package.ok_or_else(|| {
        format!(
            "manifest error: `package:` is required, missing at end of file in {}",
            path.display()
        )
    })?;
    let layer = layer.ok_or_else(|| {
        format!(
            "manifest error: `layer:` is required, missing at end of file in {}",
            path.display()
        )
    })?;

    Ok(Manifest {
        package,
        layer,
        depends,
        modules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> std::path::PathBuf {
        PathBuf::from("sooth.pkg")
    }

    #[test]
    fn parse_manifest_minimal_ok() {
        let m = parse_manifest("package: core ; layer: core ;", &p()).unwrap();
        assert_eq!(m.package, "core");
        assert_eq!(m.layer, PackageLayer::Core);
        assert!(m.depends.is_empty());
        assert!(m.modules.is_empty());
    }

    #[test]
    fn parse_manifest_full_ok() {
        let src = r#"
            package: core ;
            layer: core ;
            depends: text path "../text" ;
            module: bool cmp text ;
        "#;
        let m = parse_manifest(src, &p()).unwrap();
        assert_eq!(m.package, "core");
        assert_eq!(m.layer, PackageLayer::Core);
        assert_eq!(m.depends.len(), 1);
        assert_eq!(m.depends[0].pkg_name, "text");
        assert_eq!(m.depends[0].path, PathBuf::from("../text"));
        assert_eq!(m.modules, vec!["bool", "cmp", "text"]);
    }

    #[test]
    fn parse_manifest_unknown_layer_is_error() {
        let src = "package: core ; layer: enterprise ;";
        let err = parse_manifest(src, &p()).unwrap_err();
        assert!(err.contains("unknown layer"), "unexpected message: {err}");
        assert!(err.contains("enterprise"), "unexpected message: {err}");
    }

    #[test]
    fn parse_manifest_duplicate_package_is_error() {
        let src = "package: core ; package: text ; layer: core ;";
        let err = parse_manifest(src, &p()).unwrap_err();
        assert!(
            err.contains("duplicate `package:`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_manifest_missing_package_is_error() {
        let src = "layer: core ;";
        let err = parse_manifest(src, &p()).unwrap_err();
        assert!(
            err.contains("`package:` is required"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_manifest_missing_layer_is_error() {
        let src = "package: core ;";
        let err = parse_manifest(src, &p()).unwrap_err();
        assert!(
            err.contains("`layer:` is required"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_manifest_depends_intrinsics_is_error() {
        let src = r#"package: core ; layer: core ; depends: intrinsics path "." ;"#;
        let err = parse_manifest(src, &p()).unwrap_err();
        assert!(
            err.contains("depends: intrinsics"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_manifest_missing_semicolon_is_error() {
        let src = "package: core layer: core ;";
        let err = parse_manifest(src, &p()).unwrap_err();
        assert!(
            err.contains("expected `;` terminating `package:`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_manifest_duplicate_layer_is_error() {
        let src = "package: core ; layer: core ; layer: fixed ;";
        let err = parse_manifest(src, &p()).unwrap_err();
        assert!(
            err.contains("duplicate `layer:`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_manifest_depends_missing_path_keyword_is_error() {
        let src = r#"package: core ; layer: core ; depends: text foo "../text" ;"#;
        let err = parse_manifest(src, &p()).unwrap_err();
        assert!(
            err.contains("expected `path` after dependency name `text`, found `foo`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_manifest_unknown_declaration_is_error() {
        let src = "package: core ; layer: core ; version: 1 ;";
        let err = parse_manifest(src, &p()).unwrap_err();
        assert!(
            err.contains("unknown declaration `version:`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn package_layer_ordering_core_lt_fixed() {
        assert!(PackageLayer::Core < PackageLayer::Fixed);
        assert!(PackageLayer::Hosted > PackageLayer::Alloc);
        assert_eq!(PackageLayer::Core.name(), "core");
        assert_eq!(PackageLayer::Fixed.name(), "fixed");
        assert_eq!(PackageLayer::Alloc.name(), "alloc");
        assert_eq!(PackageLayer::Hosted.name(), "hosted");
    }
}
