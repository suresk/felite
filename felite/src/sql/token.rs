//! Cutting a line of SQL into tokens.

use crate::error::{Error, Result};

// @listing span
/// Where a token starts in the text it came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    /// Bytes from the start of the text.
    pub offset: usize,
    /// The line, counting from 1.
    pub line: u32,
    /// The character within the line, counting from 1.
    pub column: u32,
}
// @end

// @listing token
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A word that SQL reserves, such as SELECT.
    Keyword(Keyword),
    /// Any other word: the name of a table, a column or a function.
    Ident(String),
    Integer(i64),
    Real(f64),
    /// The contents of a quoted string, with its quotes removed.
    Text(String),
    Symbol(Symbol),
    /// The end of the text. Every token list finishes with one.
    End,
}
// @end

// @listing keywords
#[derive(Debug, Clone, Copy, PartialEq)]
#[rustfmt::skip]
pub enum Keyword {
    And, As, Asc, Begin, Between, By, Commit, Create, Delete, Desc, Drop, Explain,
    From, Group, Having, In, Index, Inner, Insert, Integer, Into, Is, Join, Key,
    Left, Like, Limit, Not, Null, Offset, On, Or, Order, Primary, Real, Rollback,
    Select, Set, Table, Text, Update, Values, Where,
}

/// Every keyword with its spelling, in alphabetical order.
#[rustfmt::skip]
const KEYWORDS: &[(&str, Keyword)] = {
    use Keyword::*;
    &[
        ("AND", And), ("AS", As), ("ASC", Asc), ("BEGIN", Begin),
        ("BETWEEN", Between), ("BY", By), ("COMMIT", Commit), ("CREATE", Create),
        ("DELETE", Delete), ("DESC", Desc), ("DROP", Drop), ("EXPLAIN", Explain),
        ("FROM", From), ("GROUP", Group), ("HAVING", Having), ("IN", In),
        ("INDEX", Index), ("INNER", Inner), ("INSERT", Insert), ("INTEGER", Integer),
        ("INTO", Into), ("IS", Is), ("JOIN", Join), ("KEY", Key), ("LEFT", Left),
        ("LIKE", Like), ("LIMIT", Limit), ("NOT", Not), ("NULL", Null),
        ("OFFSET", Offset), ("ON", On), ("OR", Or), ("ORDER", Order),
        ("PRIMARY", Primary), ("REAL", Real), ("ROLLBACK", Rollback),
        ("SELECT", Select), ("SET", Set), ("TABLE", Table), ("TEXT", Text),
        ("UPDATE", Update), ("VALUES", Values), ("WHERE", Where),
    ]
};
// @end

impl Keyword {
    // @listing keyword-lookup
    /// The keyword a word spells, in any mix of upper and lower case.
    pub fn from_word(word: &str) -> Option<Keyword> {
        let found =
            KEYWORDS.iter().find(|(spelling, _)| spelling.eq_ignore_ascii_case(word));
        found.map(|(_, keyword)| *keyword)
    }

    /// How the keyword is written, in capitals.
    pub fn spelling(self) -> &'static str {
        let found = KEYWORDS.iter().find(|(_, keyword)| *keyword == self);
        found.map_or("?", |(spelling, _)| spelling)
    }
    // @end
}

// @listing symbols
#[derive(Debug, Clone, Copy, PartialEq)]
#[rustfmt::skip]
pub enum Symbol {
    LeftParen, RightParen, Comma, Semicolon, Dot, Star, Plus, Minus, Slash, Percent,
    Equal, NotEqual, Less, LessEqual, Greater, GreaterEqual,
}

/// Every symbol with its spelling. A symbol of two characters comes before any
/// symbol that is its first character alone.
#[rustfmt::skip]
const SYMBOLS: &[(&str, Symbol)] = {
    use Symbol::*;
    &[
        ("<=", LessEqual), (">=", GreaterEqual), ("<>", NotEqual), ("!=", NotEqual),
        ("(", LeftParen), (")", RightParen), (",", Comma), (";", Semicolon),
        (".", Dot), ("*", Star), ("+", Plus), ("-", Minus), ("/", Slash),
        ("%", Percent), ("=", Equal), ("<", Less), (">", Greater),
    ]
};
// @end

impl Symbol {
    // @listing symbol-spelling
    pub fn spelling(self) -> &'static str {
        let found = SYMBOLS.iter().find(|(_, symbol)| *symbol == self);
        found.map_or("?", |(spelling, _)| spelling)
    }
    // @end
}

// @listing scanner
/// The text that has not been read yet, and where it starts.
struct Scanner<'a> {
    rest: &'a str,
    at: Span,
}

impl<'a> Scanner<'a> {
    fn peek(&self) -> Option<char> {
        self.rest.chars().next()
    }

    /// Moves past one character, keeping the line and column in step.
    fn bump(&mut self) {
        let Some(c) = self.peek() else { return };
        self.rest = &self.rest[c.len_utf8()..];
        self.at.offset += c.len_utf8();
        if c == '\n' {
            self.at.line += 1;
            self.at.column = 1;
        } else {
            self.at.column += 1;
        }
    }

    /// Takes characters for as long as `wanted` says yes, and returns them.
    fn take_while(&mut self, wanted: impl Fn(char) -> bool) -> &'a str {
        let start = self.rest;
        while self.peek().is_some_and(&wanted) {
            self.bump();
        }
        &start[..start.len() - self.rest.len()]
    }
}
// @end

// @listing syntax-error
fn syntax<T>(span: Span, message: impl Into<String>) -> Result<T> {
    Err(Error::Syntax(message.into(), span))
}
// @end

impl Scanner<'_> {
    // @listing skip
    /// Moves past spaces, line ends and comments.
    fn skip_blank(&mut self) {
        loop {
            self.take_while(char::is_whitespace);
            if !self.rest.starts_with("--") {
                return;
            }
            self.take_while(|c| c != '\n');
        }
    }
    // @end

    // @listing word
    fn word(&mut self) -> Token {
        let word = self.take_while(|c| c.is_alphanumeric() || c == '_');
        match Keyword::from_word(word) {
            Some(keyword) => Token::Keyword(keyword),
            None => Token::Ident(word.to_string()),
        }
    }
    // @end

    // @listing exponent
    /// Takes an exponent such as `e3` or `E-3`, if one comes next, and says
    /// whether it did. An `e` with no digits after it is left where it is.
    fn exponent(&mut self) -> bool {
        let mut ahead = self.rest.chars();
        if !matches!(ahead.next(), Some('e' | 'E')) {
            return false;
        }
        let mut next = ahead.next();
        let signed = matches!(next, Some('+' | '-'));
        if signed {
            next = ahead.next();
        }
        if !next.is_some_and(|c| c.is_ascii_digit()) {
            return false;
        }
        for _ in 0..1 + signed as usize {
            self.bump();
        }
        self.take_while(|c| c.is_ascii_digit());
        true
    }
    // @end

    // @listing number
    fn number(&mut self) -> Result<Token> {
        let start = self.at;
        let text = self.rest;
        self.take_while(|c| c.is_ascii_digit());
        let fraction = self.peek() == Some('.');
        if fraction {
            self.bump();
            self.take_while(|c| c.is_ascii_digit());
        }
        let exponent = self.exponent();
        if self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            return syntax(start, "a number runs straight into a word");
        }
        let text = &text[..text.len() - self.rest.len()];
        if fraction || exponent {
            match text.parse() {
                Ok(value) => Ok(Token::Real(value)),
                Err(_) => syntax(start, format!("{text} is not a number")),
            }
        } else {
            match text.parse() {
                Ok(value) => Ok(Token::Integer(value)),
                Err(_) => {
                    syntax(start, format!("{text} is too large for an integer"))
                }
            }
        }
    }
    // @end

    // @listing string
    fn string(&mut self) -> Result<Token> {
        let start = self.at;
        self.bump(); // the opening quote
        let mut text = String::new();
        loop {
            text.push_str(self.take_while(|c| c != '\''));
            if self.peek().is_none() {
                return syntax(start, "this string is never closed");
            }
            self.bump(); // a quote: the end, unless another follows at once
            if self.peek() != Some('\'') {
                return Ok(Token::Text(text));
            }
            text.push('\'');
            self.bump();
        }
    }
    // @end

    // @listing symbol
    fn symbol(&mut self) -> Result<Token> {
        let Some((spelling, symbol)) =
            SYMBOLS.iter().find(|(s, _)| self.rest.starts_with(s))
        else {
            let c = self.peek().unwrap_or(' ');
            return syntax(self.at, format!("unexpected character {c:?}"));
        };
        for _ in spelling.chars() {
            self.bump();
        }
        Ok(Token::Symbol(*symbol))
    }
    // @end
}

// @listing tokenize
/// The tokens of `source`, each with the place it started. The last is `End`.
pub fn tokenize(source: &str) -> Result<Vec<(Token, Span)>> {
    let mut scanner =
        Scanner { rest: source, at: Span { offset: 0, line: 1, column: 1 } };
    let mut tokens = Vec::new();
    loop {
        scanner.skip_blank();
        let start = scanner.at;
        let Some(c) = scanner.peek() else {
            tokens.push((Token::End, start));
            return Ok(tokens);
        };
        let digit_follows =
            scanner.rest.chars().nth(1).is_some_and(|c| c.is_ascii_digit());
        let token = if c.is_alphabetic() || c == '_' {
            scanner.word()
        } else if c.is_ascii_digit() || (c == '.' && digit_follows) {
            scanner.number()?
        } else if c == '\'' {
            scanner.string()?
        } else {
            scanner.symbol()?
        };
        tokens.push((token, start));
    }
}
// @end

// @listing explain
/// An error as a person would want to read it. A syntax error is shown with the
/// line it is on and a mark under the place.
pub fn explain(source: &str, error: &Error) -> String {
    let Error::Syntax(message, span) = error else {
        return error.to_string();
    };
    let line = source.lines().nth(span.line as usize - 1).unwrap_or("");
    let indent = " ".repeat(span.column as usize - 1);
    format!("syntax error: {message}\n  {line}\n  {indent}^\n")
}
// @end
