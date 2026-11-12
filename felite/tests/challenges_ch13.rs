//! Tests for the Chapter 13 challenges: `cargo test --test challenges_ch13`.
use felite::{
    Bound, BoundKind, Scope, ScopeColumn, Type, Value, bind, eval, parse_expr,
};

fn scope() -> Scope {
    let column = |name: &str, ty| ScopeColumn {
        table: "f".to_string(),
        name: name.to_string(),
        ty,
    };
    Scope {
        columns: vec![
            column("delay", Type::Integer),
            column("miles", Type::Real),
            column("origin", Type::Text),
        ],
    }
}

fn bound(sql: &str) -> Bound {
    bind(&parse_expr(sql).unwrap(), &scope()).unwrap()
}

fn value(sql: &str) -> Value {
    let row =
        [Value::Integer(15), Value::Real(588.5), Value::Text("SLC".to_string())];
    eval(&bound(sql), &row).unwrap()
}

// @challenge 2
use felite::fold_constants;

// @challenge 3
use felite::like;

// @challenge 4
use felite::cast;

// @challenge 5
use felite::can_be_null;

// @challenge 1
#[test]
fn predict_with_nulls() {
    let (yes, no, unknown) = (Value::Integer(1), Value::Integer(0), Value::Null);
    assert_eq!(value("NULL = NULL OR delay = 15"), yes);
    assert_eq!(value("NULL = NULL AND delay = 15"), unknown);
    assert_eq!(value("NULL = NULL AND delay = 16"), no);
    assert_eq!(value("NOT (NULL > 1)"), unknown);
    assert_eq!(value("(NULL > 1) IS NULL"), yes);
    assert_eq!(value("NULL + 1 IS NULL"), yes);
    assert_eq!(value("delay <> NULL"), unknown);
    assert_eq!(value("delay = 15 OR 1 / 0 = 1"), yes);
    assert_eq!(value("0 * NULL"), unknown);
    assert_eq!(value("NULL IS NULL AND NULL IS NOT NULL"), no);
    // x = x is not always true, and x <> x is not always false.
    assert_eq!(value("(delay + NULL) = (delay + NULL)"), unknown);
}

// @challenge 2
#[test]
fn folding_constants() {
    let folded = |sql: &str| fold_constants(bound(sql)).unwrap();
    assert_eq!(
        folded("60 * 60 * 24").kind,
        BoundKind::Literal(Value::Integer(86_400))
    );
    assert_eq!(folded("1 = 1 AND NULL = 1").kind, BoundKind::Literal(Value::Null));

    // delay > 60 * 24: the right side becomes one number, and the rest stays.
    let BoundKind::Binary { left, right, .. } = folded("delay > 60 * 24").kind else {
        panic!()
    };
    assert_eq!(left.kind, BoundKind::Column(0));
    assert_eq!(right.kind, BoundKind::Literal(Value::Integer(1440)));
    assert_eq!(right.ty, Type::Integer);

    // Folding does not change what an expression evaluates to.
    let row =
        [Value::Integer(15), Value::Real(588.5), Value::Text("SLC".to_string())];
    for sql in [
        "delay * (2 + 3) - miles / (1.0 + 1)",
        "origin = 'SLC' AND 2 > 1",
        "-(3 - 5) + delay",
    ] {
        assert_eq!(
            eval(&folded(sql), &row).unwrap(),
            eval(&bound(sql), &row).unwrap()
        );
    }
    // A constant that cannot be computed is reported when it is folded.
    assert!(fold_constants(bound("delay + 1 / 0")).is_err());
}

// @challenge 3
#[test]
fn like_patterns() {
    assert!(like("Salt Lake City", "Salt%"));
    assert!(like("Salt Lake City", "%City"));
    assert!(like("Salt Lake City", "%Lake%"));
    assert!(like("Salt Lake City", "S%L%C%"));
    assert!(like("SLC", "S_C"));
    assert!(like("SLC", "___"));
    assert!(like("", ""));
    assert!(like("", "%"));
    assert!(like("abc", "%%%"));
    assert!(like("Zürich", "Z_rich"));
    assert!(!like("SLC", "__"));
    assert!(!like("SLC", "S_"));
    assert!(!like("Salt Lake City", "salt%")); // case matters here
    assert!(!like("abc", "abcd"));
    assert!(!like("abc", ""));
    assert!(like(&"a".repeat(30), "%a%a%a%a%a%"));
}

// @challenge 4
#[test]
fn casting() {
    let text = |s: &str| Value::Text(s.to_string());
    assert_eq!(cast(&Value::Integer(7), Type::Real).unwrap(), Value::Real(7.0));
    assert_eq!(cast(&Value::Real(7.9), Type::Integer).unwrap(), Value::Integer(7));
    assert_eq!(cast(&Value::Real(-7.9), Type::Integer).unwrap(), Value::Integer(-7));
    assert_eq!(cast(&Value::Integer(42), Type::Text).unwrap(), text("42"));
    assert_eq!(cast(&Value::Real(1.0), Type::Text).unwrap(), text("1.0"));
    assert_eq!(cast(&text(" 15 "), Type::Integer).unwrap(), Value::Integer(15));
    assert_eq!(cast(&text("2.5"), Type::Real).unwrap(), Value::Real(2.5));
    assert_eq!(cast(&Value::Null, Type::Integer).unwrap(), Value::Null);
    assert!(cast(&text("fifteen"), Type::Integer).is_err());
    assert!(cast(&text("2.5"), Type::Integer).is_err());
    assert!(cast(&Value::Real(1e19), Type::Integer).is_err());
    assert!(cast(&Value::Real(f64::NAN), Type::Integer).is_err());
    assert!(cast(&text("inf"), Type::Real).is_err());
    let error = cast(&text("fifteen"), Type::Integer).unwrap_err().to_string();
    assert_eq!(error, "cannot turn 'fifteen' into an INTEGER");
}

// @challenge 5
#[test]
fn what_can_be_null() {
    // delay is declared NOT NULL. miles and origin are not.
    let not_null = [true, false, false];
    let nullable = |sql: &str| can_be_null(&bound(sql), &not_null);
    assert!(!nullable("delay"));
    assert!(nullable("miles"));
    assert!(!nullable("delay * 60 + 1"));
    assert!(nullable("delay + miles"));
    assert!(nullable("NULL"));
    assert!(!nullable("delay > 10"));
    assert!(nullable("origin = 'SLC'"));
    assert!(!nullable("origin IS NULL"));
    assert!(!nullable("miles IS NOT NULL AND delay > 0"));
    assert!(nullable("miles > 100 OR delay > 0")); // a cautious answer
}
