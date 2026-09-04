//! A hand-written lexer. Turns source text into a flat token stream.
//!
//! The lexer never panics on malformed input; every bad byte becomes a
//! `Trap::Parse`. It also never recurses, so input size alone can never blow
//! the host stack here.

use crate::error::Pos;
use crate::trap::Trap;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    // literals
    Int(i64),
    Str(String),
    Ident(String),
    // keywords
    Let,
    Fn,
    If,
    Else,
    While,
    Return,
    True,
    False,
    Nil,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Colon,
    // operators
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Lt,
    Gt,
    Le,
    Ge,
    EqEq,
    NotEq,
    AndAnd,
    OrOr,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub pos: Pos,
}

pub fn lex(src: &str) -> Result<Vec<Token>, Trap> {
    let bytes = src.as_bytes();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;
    let mut out = Vec::new();

    // Advance one byte, maintaining line/col. Multibyte UTF-8 is handled by
    // treating continuation bytes as column-1 each; positions stay monotonic.
    macro_rules! bump {
        () => {{
            if bytes[i] == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }

    while i < bytes.len() {
        let c = bytes[i];
        let start = Pos::new(line, col);

        // whitespace
        if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' {
            bump!();
            continue;
        }

        // line comment
        if c == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                bump!();
            }
            continue;
        }
        // `//` line comment
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                bump!();
            }
            continue;
        }

        // numbers
        if c.is_ascii_digit() {
            let mut val: i64 = 0;
            let mut overflowed = false;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                let d = (bytes[i] - b'0') as i64;
                match val.checked_mul(10).and_then(|v| v.checked_add(d)) {
                    Some(v) => val = v,
                    None => overflowed = true,
                }
                bump!();
            }
            if overflowed {
                return Err(Trap::Parse(format!("integer literal too large at {start}")));
            }
            out.push(Token {
                tok: Tok::Int(val),
                pos: start,
            });
            continue;
        }

        // identifiers and keywords
        if c == b'_' || c.is_ascii_alphabetic() {
            let begin = i;
            while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                bump!();
            }
            let word = &src[begin..i];
            let tok = match word {
                "let" => Tok::Let,
                "fn" => Tok::Fn,
                "if" => Tok::If,
                "else" => Tok::Else,
                "while" => Tok::While,
                "return" => Tok::Return,
                "true" => Tok::True,
                "false" => Tok::False,
                "nil" => Tok::Nil,
                _ => Tok::Ident(word.to_string()),
            };
            out.push(Token { tok, pos: start });
            continue;
        }

        // strings with a small escape set
        if c == b'"' {
            bump!(); // opening quote
            let mut s = String::new();
            loop {
                if i >= bytes.len() {
                    return Err(Trap::Parse(format!(
                        "unterminated string starting at {start}"
                    )));
                }
                let ch = bytes[i];
                if ch == b'"' {
                    bump!();
                    break;
                }
                if ch == b'\\' {
                    bump!();
                    if i >= bytes.len() {
                        return Err(Trap::Parse(format!(
                            "unterminated escape in string at {start}"
                        )));
                    }
                    let e = bytes[i];
                    match e {
                        b'n' => s.push('\n'),
                        b't' => s.push('\t'),
                        b'r' => s.push('\r'),
                        b'\\' => s.push('\\'),
                        b'"' => s.push('"'),
                        b'0' => s.push('\0'),
                        _ => {
                            return Err(Trap::Parse(format!(
                                "unknown escape '\\{}' at {}",
                                e as char, start
                            )))
                        }
                    }
                    bump!();
                } else {
                    // Copy the raw byte. Because we only special-case ASCII
                    // delimiters, multibyte UTF-8 sequences pass through intact.
                    s.push(ch as char);
                    if ch < 0x80 {
                        bump!();
                    } else {
                        // Reconstruct the full UTF-8 char to keep it valid.
                        s.pop();
                        let rest = &src[i..];
                        let ch = rest.chars().next().unwrap();
                        s.push(ch);
                        let len = ch.len_utf8();
                        for _ in 0..len {
                            bump!();
                        }
                    }
                }
            }
            out.push(Token {
                tok: Tok::Str(s),
                pos: start,
            });
            continue;
        }

        // multi-char operators
        let two = if i + 1 < bytes.len() {
            Some((c, bytes[i + 1]))
        } else {
            None
        };
        match two {
            Some((b'=', b'=')) => {
                bump!();
                bump!();
                out.push(Token {
                    tok: Tok::EqEq,
                    pos: start,
                });
                continue;
            }
            Some((b'!', b'=')) => {
                bump!();
                bump!();
                out.push(Token {
                    tok: Tok::NotEq,
                    pos: start,
                });
                continue;
            }
            Some((b'<', b'=')) => {
                bump!();
                bump!();
                out.push(Token {
                    tok: Tok::Le,
                    pos: start,
                });
                continue;
            }
            Some((b'>', b'=')) => {
                bump!();
                bump!();
                out.push(Token {
                    tok: Tok::Ge,
                    pos: start,
                });
                continue;
            }
            Some((b'&', b'&')) => {
                bump!();
                bump!();
                out.push(Token {
                    tok: Tok::AndAnd,
                    pos: start,
                });
                continue;
            }
            Some((b'|', b'|')) => {
                bump!();
                bump!();
                out.push(Token {
                    tok: Tok::OrOr,
                    pos: start,
                });
                continue;
            }
            _ => {}
        }

        // single-char tokens
        let single = match c {
            b'(' => Tok::LParen,
            b')' => Tok::RParen,
            b'{' => Tok::LBrace,
            b'}' => Tok::RBrace,
            b'[' => Tok::LBracket,
            b']' => Tok::RBracket,
            b',' => Tok::Comma,
            b';' => Tok::Semicolon,
            b':' => Tok::Colon,
            b'=' => Tok::Assign,
            b'+' => Tok::Plus,
            b'-' => Tok::Minus,
            b'*' => Tok::Star,
            b'/' => Tok::Slash,
            b'%' => Tok::Percent,
            b'!' => Tok::Bang,
            b'<' => Tok::Lt,
            b'>' => Tok::Gt,
            _ => {
                return Err(Trap::Parse(format!(
                    "unexpected character '{}' at {}",
                    c as char, start
                )))
            }
        };
        bump!();
        out.push(Token {
            tok: single,
            pos: start,
        });
    }

    out.push(Token {
        tok: Tok::Eof,
        pos: Pos::new(line, col),
    });
    Ok(out)
}
