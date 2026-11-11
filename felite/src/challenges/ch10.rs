//! Chapter 10 challenges: quoted names, block comments, hexadecimal numbers, and
//! tokens turned back into text.

use crate::error::{Error, Result};
use crate::sql::{Span, Token};

// @listing quoted
/// Reads a name in double quotes from the front of `text`, where two quotes in a
/// row stand for one. Returns the name and how many bytes it took, or `None` if
/// the name is not closed.
pub fn scan_quoted(text: &str) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut chars = text.char_indices().skip(1).peekable();
    while let Some((at, c)) = chars.next() {
        if c != '"' {
            name.push(c);
        } else if chars.peek().is_some_and(|(_, next)| *next == '"') {
            name.push('"');
            chars.next();
        } else {
            return Some((name, at + 1));
        }
    }
    None
}
// @end

// @listing block-comments
/// Replaces every `/* … */` comment with spaces, keeping line ends, so that the
/// positions of everything else are unchanged. Comment marks inside a string are
/// left alone.
pub fn strip_block_comments(source: &str) -> Result<String> {
    let mut out = String::with_capacity(source.len());
    let mut at = Span { offset: 0, line: 1, column: 1 };
    let mut opened: Option<Span> = None; // where the comment we are inside began
    let mut in_string = false;
    let mut rest = source;
    while let Some(first) = rest.chars().next() {
        let opening = opened.is_none() && !in_string && rest.starts_with("/*");
        let closing = opened.is_some() && rest.starts_with("*/");
        if opening {
            opened = Some(at);
        }
        // A comment mark is taken whole. Anything else, one character at a time.
        let taken = if opening || closing { 2 } else { first.len_utf8() };
        for c in rest[..taken].chars() {
            if opened.is_none() || c == '\n' {
                out.push(c);
            } else {
                // One space for each byte, so that byte offsets stay the same.
                out.extend(std::iter::repeat_n(' ', c.len_utf8()));
            }
            at.offset += c.len_utf8();
            at.column += 1;
            if c == '\n' {
                (at.line, at.column) = (at.line + 1, 1);
            }
        }
        if closing {
            opened = None;
        } else if opened.is_none() && first == '\'' {
            in_string = !in_string;
        }
        rest = &rest[taken..];
    }
    match opened {
        Some(span) => Err(Error::Syntax("this comment is never closed".into(), span)),
        None => Ok(out),
    }
}
// @end

// @listing hex
/// The value of a literal such as `0x1F`, or `None` if `text` is not one. Sixteen
/// digits fill all 64 bits, so `0xFFFFFFFFFFFFFFFF` is -1.
pub fn hex_literal(text: &str) -> Option<i64> {
    let digits = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))?;
    if digits.is_empty() || digits.len() > 16 {
        return None;
    }
    u64::from_str_radix(digits, 16).ok().map(|bits| bits as i64)
}
// @end

// @listing to-sql
/// Tokens as SQL text, with one space between them. Tokenizing the result gives
/// the same tokens back.
pub fn to_sql(tokens: &[Token]) -> String {
    let mut words = Vec::new();
    for token in tokens {
        words.push(match token {
            Token::Keyword(keyword) => keyword.spelling().to_string(),
            Token::Ident(name) => name.clone(),
            Token::Integer(value) => value.to_string(),
            // {:?} always writes a real with a decimal point or an exponent.
            Token::Real(value) => format!("{value:?}"),
            Token::Text(text) => format!("'{}'", text.replace('\'', "''")),
            Token::Symbol(symbol) => symbol.spelling().to_string(),
            Token::End => continue,
        });
    }
    words.join(" ")
}
// @end
