use sooth::driver;
use std::path::PathBuf;
use std::process::exit;

fn usage() -> ! {
    eprintln!(
        "sooth — the Sooth compiler (bootstrap)\n\n\
         usage:\n\
         \x20 sooth build <file.sth> [--manifest <path>]   compile to a native binary\n\
         \x20 sooth run   <file.sth> [--manifest <path>]   compile and run\n\
         \x20 sooth repl                 interactive session (Phase 1)\n\n\
         \x20 --manifest <path>  resolve the entry file's dependency-anchored\n\
         \x20                    imports against this manifest, overriding an\n\
         \x20                    ancestor manifest (entry file only)\n"
    );
    exit(2);
}

/// Split `build`/`run`'s trailing arguments into the entry file and an
/// optional `--manifest <path>` (which may appear before or after the entry
/// file). `Err` for a `--manifest` with no following path, a second
/// `--manifest`, any other `--flag`, a second entry file, or no entry file at
/// all: each is a usage error at the call site, kept out of this function so
/// it stays testable without exiting the process.
fn parse_entry_and_manifest(args: &[String]) -> Result<(PathBuf, Option<PathBuf>), ()> {
    let mut entry: Option<PathBuf> = None;
    let mut manifest: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--manifest" {
            if manifest.is_some() {
                return Err(());
            }
            let path = args.get(i + 1).ok_or(())?;
            manifest = Some(PathBuf::from(path));
            i += 2;
        } else if args[i].starts_with("--") {
            return Err(());
        } else {
            if entry.is_some() {
                return Err(());
            }
            entry = Some(PathBuf::from(&args[i]));
            i += 1;
        }
    }
    let entry = entry.ok_or(())?;
    Ok((entry, manifest))
}

fn entry_and_manifest_or_usage(args: &[String]) -> (PathBuf, Option<PathBuf>) {
    parse_entry_and_manifest(args).unwrap_or_else(|()| usage())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("build") => {
            let (entry, manifest) = entry_and_manifest_or_usage(&args[2..]);
            driver::build_with_manifest(&entry, manifest.as_deref()).map(|_| ())
        }
        Some("run") => {
            let (entry, manifest) = entry_and_manifest_or_usage(&args[2..]);
            match driver::run_with_manifest(&entry, manifest.as_deref()) {
                Ok(status) => exit(status.code().unwrap_or(1)),
                Err(e) => Err(e),
            }
        }
        Some("repl") => driver::repl(),
        None | Some("-h") | Some("--help") => usage(),
        Some(other) => {
            eprintln!("unknown command: {other}\n");
            usage();
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn manifest_flag_before_entry() {
        let (entry, manifest) =
            parse_entry_and_manifest(&args(&["--manifest", "m.pkg", "a.sth"])).unwrap();
        assert_eq!(entry, PathBuf::from("a.sth"));
        assert_eq!(manifest, Some(PathBuf::from("m.pkg")));
    }

    #[test]
    fn manifest_flag_after_entry() {
        let (entry, manifest) =
            parse_entry_and_manifest(&args(&["a.sth", "--manifest", "m.pkg"])).unwrap();
        assert_eq!(entry, PathBuf::from("a.sth"));
        assert_eq!(manifest, Some(PathBuf::from("m.pkg")));
    }

    #[test]
    fn no_manifest_flag_is_none() {
        let (entry, manifest) = parse_entry_and_manifest(&args(&["a.sth"])).unwrap();
        assert_eq!(entry, PathBuf::from("a.sth"));
        assert_eq!(manifest, None);
    }

    #[test]
    fn manifest_flag_missing_path_is_usage_error() {
        assert!(parse_entry_and_manifest(&args(&["a.sth", "--manifest"])).is_err());
    }

    #[test]
    fn duplicate_manifest_flag_is_usage_error() {
        assert!(parse_entry_and_manifest(&args(&[
            "a.sth",
            "--manifest",
            "m.pkg",
            "--manifest",
            "n.pkg"
        ]))
        .is_err());
    }

    #[test]
    fn no_entry_file_is_usage_error() {
        assert!(parse_entry_and_manifest(&args(&["--manifest", "m.pkg"])).is_err());
    }

    #[test]
    fn duplicate_entry_file_is_usage_error() {
        assert!(parse_entry_and_manifest(&args(&["a.sth", "b.sth"])).is_err());
    }

    #[test]
    fn unrecognized_flag_is_usage_error() {
        assert!(parse_entry_and_manifest(&args(&["--verbose"])).is_err());
        assert!(parse_entry_and_manifest(&args(&["a.sth", "--verbose"])).is_err());
    }
}
