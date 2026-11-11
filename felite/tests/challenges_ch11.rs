//! Tests for the Chapter 11 challenges: `cargo test --test challenges_ch11`.
use felite::{BinaryOp, Expr, ExprKind, Span, UnaryOp, Value, parse_expr};
use felite::{columns_used, depth, to_minimal_sql};

fn grouped(source: &str) -> String {
    parse_expr(source).unwrap().to_string()
}

#[test]
fn predict_the_grouping() {
    assert_eq!(grouped("a + b * c - d"), "((a + (b * c)) - d)");
    assert_eq!(grouped("a < b = c"), "((a < b) = c)");
    assert_eq!(grouped("- a * - b"), "((-a) * (-b))");
    assert_eq!(grouped("NOT a OR b AND NOT c"), "((NOT a) OR (b AND (NOT c)))");
    assert_eq!(grouped("a OR b OR c AND d"), "((a OR b) OR (c AND d))");
    assert_eq!(
        grouped("a % 2 = 0 AND NOT b IS NULL"),
        "(((a % 2) = 0) AND (NOT (b IS NULL)))"
    );
    assert_eq!(grouped("-a IS NOT NULL"), "((-a) IS NOT NULL)");
    assert_eq!(grouped("a = b IS NULL"), "((a = b) IS NULL)");
    assert_eq!(grouped("1 - (2 - 3)"), "(1 - (2 - 3))");
    assert_eq!(grouped("NOT - a < b"), "(NOT ((-a) < b))");
}

#[test]
fn walking_a_tree() {
    let expr =
        parse_expr("dep_delay > 15 AND (origin = 'SLC' OR flights.origin = dest)")
            .unwrap();
    assert_eq!(columns_used(&expr), ["dep_delay", "origin", "dest"]);
    assert_eq!(columns_used(&parse_expr("1 + 2").unwrap()), Vec::<String>::new());
    assert_eq!(depth(&parse_expr("7").unwrap()), 1);
    assert_eq!(depth(&parse_expr("1 + 2 * 3").unwrap()), 3);
    assert_eq!(depth(&parse_expr("1 + 2 + 3 + 4").unwrap()), 4);
    assert_eq!(depth(&parse_expr("(1 + 2) + (3 + 4)").unwrap()), 3);
    assert_eq!(depth(&parse_expr("NOT a IS NULL").unwrap()), 3);
}

#[test]
fn only_the_parentheses_that_matter() {
    let minimal = |source: &str| to_minimal_sql(&parse_expr(source).unwrap());
    assert_eq!(minimal("1 + (2 * 3)"), "1 + 2 * 3");
    assert_eq!(minimal("(1 + 2) * 3"), "(1 + 2) * 3");
    assert_eq!(minimal("(1 - 2) - 3"), "1 - 2 - 3");
    assert_eq!(minimal("1 - (2 - 3)"), "1 - (2 - 3)");
    assert_eq!(
        minimal("((a = 1) OR ((b = 2) AND (c = 3)))"),
        "a = 1 OR b = 2 AND c = 3"
    );
    assert_eq!(minimal("(a OR b) AND c"), "(a OR b) AND c");
    assert_eq!(minimal("NOT (a = b)"), "NOT a = b");
    assert_eq!(minimal("(NOT a) = b"), "(NOT a) = b");
    assert_eq!(minimal("NOT (a AND b)"), "NOT (a AND b)");
    assert_eq!(minimal("-(a + b)"), "-(a + b)");
    assert_eq!(minimal("-(-a)"), "-(-a)");
    assert_eq!(minimal("a - -b"), "a - -b");
    assert_eq!(minimal("(a + 1) IS NULL"), "a + 1 IS NULL");
    assert_eq!(minimal("(a OR b) IS NOT NULL"), "(a OR b) IS NOT NULL");
    assert_eq!(minimal("x = 'it''s'"), "x = 'it''s'");
}

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// A random expression at most `levels` levels deep.
fn random_expr(state: &mut u64, levels: u32) -> Expr {
    use BinaryOp::*;
    let pick = next(state) % if levels <= 1 { 4 } else { 10 };
    let kind = match pick {
        0 => ExprKind::Literal(Value::Integer((next(state) % 100) as i64)),
        1 => ExprKind::Literal(Value::Real((next(state) % 1000) as f64 / 8.0)),
        2 => ExprKind::Literal(
            [Value::Null, Value::Text("it's".to_string())][next(state) as usize % 2]
                .clone(),
        ),
        3 => ExprKind::Column {
            table: None,
            name: ["a", "b", "c"][next(state) as usize % 3].to_string(),
        },
        4 => ExprKind::Unary {
            op: [UnaryOp::Negate, UnaryOp::Not][next(state) as usize % 2],
            operand: Box::new(random_expr(state, levels - 1)),
        },
        5 => ExprKind::IsNull {
            operand: Box::new(random_expr(state, levels - 1)),
            negated: next(state).is_multiple_of(2),
        },
        _ => {
            let ops = [
                Add,
                Subtract,
                Multiply,
                Divide,
                Remainder,
                Equal,
                NotEqual,
                Less,
                LessEqual,
                Greater,
                GreaterEqual,
                And,
                Or,
            ];
            ExprKind::Binary {
                op: ops[next(state) as usize % ops.len()],
                left: Box::new(random_expr(state, levels - 1)),
                right: Box::new(random_expr(state, levels - 1)),
            }
        }
    };
    Expr { kind, span: Span { offset: 0, line: 1, column: 1 } }
}

#[test]
fn print_and_parse_agree() {
    let mut state = 0x9e37_79b9_7f4a_7c15;
    let mut shorter = 0;
    for _ in 0..2000 {
        let expr = random_expr(&mut state, 6);
        let full = expr.to_string();
        // Fully parenthesized text parses back to the same tree.
        assert_eq!(parse_expr(&full).unwrap().to_string(), full);
        // So does the text with only the parentheses it needs.
        let minimal = to_minimal_sql(&expr);
        assert_eq!(parse_expr(&minimal).unwrap().to_string(), full, "{minimal}");
        shorter += (minimal.len() < full.len()) as usize;
    }
    assert_eq!(shorter, 1186); // every tree that has an operation in it
}
