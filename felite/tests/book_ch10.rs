//! The examples in Chapter 10: `cargo test --test book_ch10`.
use felite::{Error, Keyword, Span, Symbol, Token, explain, tokenize};

// @listing helpers
/// The tokens of a line of SQL, without their positions or the final `End`.
fn tokens(source: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> =
        tokenize(source).unwrap().into_iter().map(|(token, _)| token).collect();
    assert_eq!(tokens.pop(), Some(Token::End));
    tokens
}

/// The message and position of the error that a line of SQL produces.
fn error(source: &str) -> (String, u32, u32) {
    match tokenize(source) {
        Err(Error::Syntax(message, at)) => (message, at.line, at.column),
        other => panic!("expected a syntax error, got {other:?}"),
    }
}

fn ident(name: &str) -> Token {
    Token::Ident(name.to_string())
}
// @end

#[test]
fn a_statement_in_tokens() {
    // @listing first
    use Keyword::{From, Select, Where};
    assert_eq!(
        tokens("SELECT name FROM people WHERE age >= 21;"),
        [
            Token::Keyword(Select),
            ident("name"),
            Token::Keyword(From),
            ident("people"),
            Token::Keyword(Where),
            ident("age"),
            Token::Symbol(Symbol::GreaterEqual),
            Token::Integer(21),
            Token::Symbol(Symbol::Semicolon),
        ]
    );
    // Nothing at all is one token: the end.
    assert_eq!(tokenize("").unwrap().len(), 1);
    assert_eq!(tokens("   \n  "), []);
    // @end
}

#[test]
fn words() {
    // @listing words-test
    // Keywords are recognized in any case. Other words keep the case they came in.
    let select = Token::Keyword(Keyword::Select);
    assert_eq!(
        tokens("select Select SELECT"),
        [select.clone(), select.clone(), select]
    );
    assert_eq!(tokens("Flights flights"), [ident("Flights"), ident("flights")]);
    // A word may contain digits and underscores, and may not start with a digit.
    assert_eq!(
        tokens("dep_delay x2 _hidden"),
        [ident("dep_delay"), ident("x2"), ident("_hidden")]
    );
    // A keyword inside a longer word is not a keyword.
    assert_eq!(tokens("selector fromage"), [ident("selector"), ident("fromage")]);
    assert_eq!(Keyword::Select.spelling(), "SELECT");
    assert_eq!(Keyword::from_word("table"), Some(Keyword::Table));
    assert_eq!(Keyword::from_word("tables"), None);
    // @end
}

#[test]
fn numbers() {
    // @listing numbers-test
    assert_eq!(
        tokens("42 007 0"),
        [Token::Integer(42), Token::Integer(7), Token::Integer(0)]
    );
    assert_eq!(
        tokens("1.5 1. .5"),
        [Token::Real(1.5), Token::Real(1.0), Token::Real(0.5)]
    );
    assert_eq!(
        tokens("1e3 2.5E-3 6e+2"),
        [Token::Real(1000.0), Token::Real(0.0025), Token::Real(600.0)]
    );
    assert_eq!(tokens("9223372036854775807"), [Token::Integer(i64::MAX)]);
    // A minus sign is a token of its own. The parser decides what it means.
    assert_eq!(tokens("-7"), [Token::Symbol(Symbol::Minus), Token::Integer(7)]);
    // A dot with no digit after it is only a dot.
    assert_eq!(tokens("t.x"), [ident("t"), Token::Symbol(Symbol::Dot), ident("x")]);

    assert_eq!(
        error("SELECT 9223372036854775808"),
        ("9223372036854775808 is too large for an integer".to_string(), 1, 8)
    );
    assert_eq!(
        error("12abc"),
        ("a number runs straight into a word".to_string(), 1, 1)
    );
    assert_eq!(error("1e"), ("a number runs straight into a word".to_string(), 1, 1));
    // @end
}

#[test]
fn strings() {
    // @listing strings-test
    let text = |s: &str| Token::Text(s.to_string());
    assert_eq!(tokens("'Ada'"), [text("Ada")]);
    assert_eq!(tokens("''"), [text("")]);
    assert_eq!(tokens("'it''s'"), [text("it's")]);
    assert_eq!(tokens("''''"), [text("'")]); // a string holding one quote
    assert_eq!(
        tokens("'SELECT -- not a comment'"),
        [text("SELECT -- not a comment")]
    );
    assert_eq!(tokens("'two\nlines'"), [text("two\nlines")]);
    assert_eq!(tokens("'a' 'b'"), [text("a"), text("b")]);

    // The error points at the quote that opened the string, not at the end.
    assert_eq!(
        error("SELECT 'Ada FROM people"),
        ("this string is never closed".to_string(), 1, 8)
    );
    // @end
}

#[test]
fn symbols() {
    // @listing symbols-test
    use Symbol::*;
    let symbols = |source: &str| -> Vec<Symbol> {
        tokens(source)
            .into_iter()
            .map(|token| match token {
                Token::Symbol(symbol) => symbol,
                other => panic!("{other:?} is not a symbol"),
            })
            .collect()
    };
    assert_eq!(symbols("<="), [LessEqual]);
    assert_eq!(symbols("< ="), [Less, Equal]);
    assert_eq!(symbols("<> !="), [NotEqual, NotEqual]);
    assert_eq!(symbols("<>="), [NotEqual, Equal]);
    let all =
        [LeftParen, Plus, RightParen, Star, Comma, Semicolon, Dot, Slash, Percent];
    assert_eq!(symbols("(+)*,;./%"), all);
    assert_eq!(Symbol::NotEqual.spelling(), "<>");

    assert_eq!(error("a ! b"), ("unexpected character '!'".to_string(), 1, 3));
    assert_eq!(error("price @ 2"), ("unexpected character '@'".to_string(), 1, 7));
    // @end
}

#[test]
fn comments_and_positions() {
    // @listing positions
    let source = "SELECT name -- who\n  FROM people  -- where from\n";
    let found = tokenize(source).unwrap();
    let places: Vec<(usize, u32, u32)> =
        found.iter().map(|(_, at)| (at.offset, at.line, at.column)).collect();
    // SELECT, name, FROM, people, and the end.
    assert_eq!(places, [(0, 1, 1), (7, 1, 8), (21, 2, 3), (26, 2, 8), (48, 3, 1)]);
    assert_eq!(
        found[2],
        (Token::Keyword(Keyword::From), Span { offset: 21, line: 2, column: 3 })
    );

    // A column counts characters, and an offset counts bytes. They part company
    // after the first character that takes more than one byte.
    let found = tokenize("'Zürich' x").unwrap();
    assert_eq!(found[1].1, Span { offset: 10, line: 1, column: 10 });
    // @end
}

#[test]
fn errors_for_people() {
    // @listing explain-test
    let source = "SELECT name\nFROM people\nWHERE age > 21 @ 3";
    let error = tokenize(source).unwrap_err();
    assert_eq!(
        error.to_string(),
        "syntax error at line 3, column 16: unexpected character '@'"
    );
    assert_eq!(
        explain(source, &error),
        "syntax error: unexpected character '@'\n  \
           WHERE age > 21 @ 3\n  \
           \x20              ^\n"
    );
    // @end
}
