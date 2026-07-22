//! Tokeniser. Phase 0: `: ;`, integers, words, `( ... )` stack effects, `| ... |`.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Colon,
    Semicolon,
    LParen,
    RParen,
    Pipe,
    Int(i64),
    Word(String),
}

pub fn lex(_src: &str) -> Result<Vec<Token>, String> {
    todo!("Phase 0: lexer")
}
