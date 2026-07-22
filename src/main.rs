#![allow(dead_code, unused)]

mod ast;
mod backend;
mod check;
mod driver;
mod ir;
mod lexer;
mod parser;

use std::path::Path;
use std::process::exit;

fn usage() -> ! {
    eprintln!(
        "sooth — the Sooth compiler (bootstrap)\n\n\
         usage:\n\
         \x20 sooth build <file.sooth>   compile to a native binary\n\
         \x20 sooth run   <file.sooth>   compile and run\n\
         \x20 sooth repl                 interactive session (Phase 1)\n"
    );
    exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("build") => driver::build(Path::new(args.get(2).unwrap_or_else(|| usage()))),
        Some("run") => driver::run(Path::new(args.get(2).unwrap_or_else(|| usage()))),
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
