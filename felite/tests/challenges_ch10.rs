//! Tests for the Chapter 10 challenges: `cargo test --test challenges_ch10`.
use felite::{Error, Keyword, Symbol, Token, tokenize};

fn tokens(source: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> =
        tokenize(source).unwrap().into_iter().map(|(token, _)| token).collect();
    tokens.pop();
    tokens
}

// @challenge 2
use felite::scan_quoted;

// @challenge 3
use felite::strip_block_comments;

// @challenge 4
use felite::hex_literal;

// @challenge 5
use felite::to_sql;

// @challenge 1
#[test]
fn predict_the_tokens() {
    use Symbol::*;
    let ident = |name: &str| Token::Ident(name.to_string());
    let text = |s: &str| Token::Text(s.to_string());
    assert_eq!(
        tokens("select*from t where x<=-1.5e2--and y\nor 'a''--b'<>''"),
        [
            Token::Keyword(Keyword::Select),
            Token::Symbol(Star),
            Token::Keyword(Keyword::From),
            ident("t"),
            Token::Keyword(Keyword::Where),
            ident("x"),
            Token::Symbol(LessEqual),
            Token::Symbol(Minus),
            Token::Real(150.0),
            Token::Keyword(Keyword::Or),
            text("a'--b"),
            Token::Symbol(NotEqual),
            text(""),
        ]
    );
    assert_eq!(tokens("1.e2"), [Token::Real(100.0)]);
    assert_eq!(tokens("1..2"), [Token::Real(1.0), Token::Real(0.2)]);
    assert_eq!(tokens("a.b"), [ident("a"), Token::Symbol(Dot), ident("b")]);
    assert_eq!(tokens("NULLs"), [ident("NULLs")]);
    assert_eq!(
        tokens("x- -1"),
        [ident("x"), Token::Symbol(Minus), Token::Symbol(Minus), Token::Integer(1)]
    );
    assert_eq!(tokens("x--1"), [ident("x")]);
}

// @challenge 2
#[test]
fn names_in_double_quotes() {
    assert_eq!(
        scan_quoted(r#""first name" FROM"#),
        Some(("first name".to_string(), 12))
    );
    assert_eq!(scan_quoted(r#""select""#), Some(("select".to_string(), 8)));
    assert_eq!(
        scan_quoted(r#""say ""hi""" x"#),
        Some((r#"say "hi""#.to_string(), 12))
    );
    assert_eq!(scan_quoted(r#""""#), Some((String::new(), 2)));
    assert_eq!(scan_quoted(r#""Zürich", x"#), Some(("Zürich".to_string(), 9)));
    assert_eq!(scan_quoted(r#""never closed"#), None);
    assert_eq!(scan_quoted(r#""ends in a pair"""#), None);
}

// @challenge 3
#[test]
fn block_comments() {
    let stripped = strip_block_comments("SELECT /* all of */ name FROM t").unwrap();
    assert_eq!(stripped, "SELECT              name FROM t");
    // Positions after a comment are what they were before it was removed.
    let source = "SELECT a, /* two\n   lines */ b @";
    let stripped = strip_block_comments(source).unwrap();
    assert_eq!(stripped, "SELECT a,       \n            b @");
    let Err(Error::Syntax(_, at)) = tokenize(&stripped) else { panic!() };
    assert_eq!((at.line, at.column, &source[at.offset..]), (2, 15, "@"));
    // Inside a string, the marks mean nothing.
    assert_eq!(strip_block_comments("'/* not */'").unwrap(), "'/* not */'");
    assert_eq!(strip_block_comments("a /* é */ b").unwrap(), "a          b");
    assert_eq!(strip_block_comments("x /**/ y /* */").unwrap(), "x      y      ");
    let Err(Error::Syntax(message, at)) = strip_block_comments("a\n b /* oops")
    else {
        panic!()
    };
    assert_eq!(
        (message.as_str(), at.line, at.column),
        ("this comment is never closed", 2, 4)
    );
}

// @challenge 4
#[test]
fn hexadecimal() {
    assert_eq!(hex_literal("0x1F"), Some(31));
    assert_eq!(hex_literal("0Xff"), Some(255));
    assert_eq!(hex_literal("0x0"), Some(0));
    assert_eq!(hex_literal("0x7FFFFFFFFFFFFFFF"), Some(i64::MAX));
    assert_eq!(hex_literal("0xFFFFFFFFFFFFFFFF"), Some(-1));
    assert_eq!(hex_literal("0x10000000000000000"), None); // seventeen digits
    assert_eq!(hex_literal("0x"), None);
    assert_eq!(hex_literal("0xG1"), None);
    assert_eq!(hex_literal("1F"), None);
}

// @challenge 5
#[test]
fn tokens_back_to_text() {
    let lines = [
        "SELECT name, age FROM people WHERE age >= 21 AND name <> 'O''Brien';",
        "insert into t values (1, -2.5, 1e300, 'x', null)",
        "SELECT 1.0, 2., .5, 1e3, 100000000000000000000.0",
        "a<=b<>c!=d",
    ];
    for line in lines {
        let first = tokens(line);
        let text = to_sql(&first);
        assert_eq!(tokens(&text), first, "{text}");
    }
    assert_eq!(
        to_sql(&tokens("select  X from\nT where y!=1.")),
        "SELECT X FROM T WHERE y <> 1.0"
    );
}
