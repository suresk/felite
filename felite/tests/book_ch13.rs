//! The examples in Chapter 13: `cargo test --test book_ch13`.
use felite::{Bound, BoundKind, Error, Scope, ScopeColumn, Type, Value};
use felite::{bind, bind_condition, eval, explain, parse_expr, truth};

// @listing helpers
/// The columns the tests can refer to: three of one flight, in a table `f`.
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

/// A row for those columns: 15 minutes late, 588.5 miles, from SLC.
fn row() -> Vec<Value> {
    vec![Value::Integer(15), Value::Real(588.5), Value::Text("SLC".to_string())]
}

/// Parses, binds and evaluates an expression against that row.
fn value(sql: &str) -> Value {
    let bound = bind(&parse_expr(sql).unwrap(), &scope()).unwrap();
    eval(&bound, &row()).unwrap()
}

/// The message of the error that an expression produces, when it is bound or when
/// it is evaluated, and the column of the text that the error points at.
fn error(sql: &str) -> (String, u32) {
    let result =
        bind(&parse_expr(sql).unwrap(), &scope()).and_then(|b| eval(&b, &row()));
    match result {
        Err(Error::Invalid(message, at)) => (message, at.column),
        other => panic!("expected an error, got {other:?}"),
    }
}
// @end

#[test]
fn names_become_positions() {
    // @listing bind-test
    let bound = bind(&parse_expr("miles / 60.0").unwrap(), &scope()).unwrap();
    assert_eq!(bound.ty, Type::Real);
    let BoundKind::Binary { left, right, .. } = &bound.kind else { panic!() };
    assert_eq!((&left.kind, left.ty), (&BoundKind::Column(1), Type::Real));
    assert_eq!(right.kind, BoundKind::Literal(Value::Real(60.0)));

    // Names match in any case, with the table's name in front or without it.
    for name in ["origin", "ORIGIN", "f.origin", "F.Origin"] {
        let Bound { kind, ty, .. } =
            bind(&parse_expr(name).unwrap(), &scope()).unwrap();
        assert_eq!((kind, ty), (BoundKind::Column(2), Type::Text));
    }
    assert_eq!(
        error("delay + dealy"),
        ("there is no column named dealy".to_string(), 9)
    );
    assert_eq!(
        error("flights.delay"),
        ("there is no column flights.delay".to_string(), 1)
    );
    // @end
}

#[test]
fn every_expression_has_a_type() {
    // @listing types-test
    let ty = |sql: &str| bind(&parse_expr(sql).unwrap(), &scope()).unwrap().ty;
    assert_eq!(ty("delay + 1"), Type::Integer);
    assert_eq!(ty("delay + 1.0"), Type::Real); // an integer and a real make a real
    assert_eq!(ty("delay * miles"), Type::Real);
    assert_eq!(ty("-delay"), Type::Integer);
    assert_eq!(ty("delay > 10"), Type::Condition);
    assert_eq!(ty("delay > 10 AND origin = 'SLC'"), Type::Condition);
    assert_eq!(ty("origin IS NULL"), Type::Condition);
    assert_eq!(ty("NULL"), Type::Null);
    assert_eq!(ty("delay + NULL"), Type::Integer); // NULL takes the type beside it
    // @end
}

#[test]
fn type_errors_are_found_before_anything_runs() {
    // @listing type-errors
    assert_eq!(
        error("1 + 'a'"),
        ("+ needs a number, and this is TEXT".to_string(), 5)
    );
    assert_eq!(
        error("origin * 2"),
        ("* needs a number, and this is TEXT".to_string(), 1)
    );
    assert_eq!(
        error("-origin"),
        ("- needs a number, and this is TEXT".to_string(), 2)
    );
    assert_eq!(
        error("delay = origin"),
        ("this is TEXT, and = compares it with an INTEGER".to_string(), 9)
    );
    assert_eq!(
        error("origin < 5"),
        ("this is an INTEGER, and < compares it with TEXT".to_string(), 10)
    );
    assert_eq!(
        error("delay AND miles > 1"),
        ("AND needs a condition, and this is an INTEGER".to_string(), 1)
    );
    assert_eq!(
        error("NOT delay"),
        ("NOT needs a condition, and this is an INTEGER".to_string(), 5)
    );
    assert_eq!(
        error("(delay > 1) + 1"),
        ("+ needs a number, and this is a condition".to_string(), 2)
    );

    // WHERE takes a condition and nothing else.
    let Err(error) =
        bind_condition(&parse_expr("delay + 1").unwrap(), &scope(), "WHERE")
    else {
        panic!()
    };
    assert_eq!(
        error.to_string(),
        "error at line 1, column 1: WHERE needs a condition, and this is an INTEGER"
    );
    // @end
}

#[test]
fn arithmetic() {
    // @listing arithmetic-test
    assert_eq!(value("1 + 2 * 3"), Value::Integer(7));
    assert_eq!(value("delay * 60"), Value::Integer(900));
    assert_eq!(value("miles * 2"), Value::Real(1177.0));
    // Dividing two integers gives an integer, rounded toward zero.
    assert_eq!(value("7 / 2"), Value::Integer(3));
    assert_eq!(value("-7 / 2"), Value::Integer(-3));
    assert_eq!(value("7 % 2"), Value::Integer(1));
    // If either side is a real, so is the answer.
    assert_eq!(value("7 / 2.0"), Value::Real(3.5));
    assert_eq!(value("7.5 % 2"), Value::Real(1.5));
    assert_eq!(value("- -delay"), Value::Integer(15));
    // @end
}

#[test]
fn arithmetic_that_fails() {
    // @listing arithmetic-errors
    let too_large = "the result is too large for an integer".to_string();
    assert_eq!(error("9223372036854775807 + 1"), (too_large.clone(), 1));
    assert_eq!(error("delay * 9223372036854775807"), (too_large.clone(), 1));
    assert_eq!(error("-9223372036854775807 - 2"), (too_large.clone(), 1));
    assert_eq!(error("1 + 2 / (delay - 15)"), ("division by zero".to_string(), 5));
    assert_eq!(error("delay % 0"), ("division by zero".to_string(), 1));
    assert_eq!(error("miles / 0.0"), ("division by zero".to_string(), 1));
    assert_eq!(
        error("1e308 * 10"),
        ("the result is too large for a real".to_string(), 1)
    );
    // @end
}

#[test]
fn comparisons() {
    // @listing comparison-test
    let (yes, no) = (Value::Integer(1), Value::Integer(0));
    assert_eq!(value("delay > 10"), yes);
    assert_eq!(value("delay >= 15 AND delay <= 15"), yes);
    assert_eq!(value("delay <> 15"), no);
    assert_eq!(value("delay < miles"), yes); // an integer against a real
    assert_eq!(value("1 = 1.0"), yes);
    assert_eq!(value("origin = 'SLC'"), yes);
    assert_eq!(value("origin = 'slc'"), no); // text is compared exactly
    assert_eq!(value("'SLC' < 'SLD'"), yes);
    assert_eq!(value("'Z' < 'a'"), yes); // by bytes: capitals come first
    assert_eq!(value("'SL' < 'SLC'"), yes);
    // @end
}

#[test]
fn null_is_not_a_value() {
    // @listing null-test
    assert_eq!(value("NULL + 1"), Value::Null);
    assert_eq!(value("delay * NULL"), Value::Null);
    assert_eq!(value("-NULL"), Value::Null);
    assert_eq!(value("NULL = 1"), Value::Null);
    assert_eq!(value("NULL <> 1"), Value::Null);
    // Not even NULL is equal to NULL. Nor is it different from it.
    assert_eq!(value("NULL = NULL"), Value::Null);
    assert_eq!(value("NULL <> NULL"), Value::Null);
    // The way to ask is IS NULL, which always answers yes or no.
    assert_eq!(value("NULL IS NULL"), Value::Integer(1));
    assert_eq!(value("delay IS NULL"), Value::Integer(0));
    assert_eq!(value("delay IS NOT NULL"), Value::Integer(1));
    assert_eq!(value("(delay + NULL) IS NULL"), Value::Integer(1));
    // @end
}

#[test]
fn three_valued_logic() {
    // @listing truth-tables
    // T is true, F is false, and U is unknown: a NULL.
    let spelled = |letter: char| match letter {
        'T' => "1 = 1",
        'F' => "1 = 2",
        _ => "NULL = 1",
    };
    let letter = |value: Value| match truth(&value) {
        Some(true) => 'T',
        Some(false) => 'F',
        None => 'U',
    };
    let table = |op: &str| -> String {
        let mut out = String::new();
        for left in ['T', 'F', 'U'] {
            for right in ['T', 'F', 'U'] {
                out.push(letter(value(&format!(
                    "({}) {op} ({})",
                    spelled(left),
                    spelled(right)
                ))));
            }
            out.push(' ');
        }
        out
    };
    // Each group of three is one value of the left side, against T, F and U.
    assert_eq!(table("AND"), "TFU FFF UFU ");
    assert_eq!(table("OR"), "TTT TFU TUU ");
    let not: String = ['T', 'F', 'U']
        .map(|l| letter(value(&format!("NOT ({})", spelled(l)))))
        .iter()
        .collect();
    assert_eq!(not, "FTU");
    // @end
}

#[test]
fn the_right_side_is_not_always_needed() {
    // @listing short-circuit
    // A false on the left settles an AND, and the division is never attempted.
    assert_eq!(value("delay > 100 AND 1 / (delay - 15) = 1"), Value::Integer(0));
    assert_eq!(value("delay < 100 OR 1 / (delay - 15) = 1"), Value::Integer(1));
    // When the left side does not settle it, the right side runs, and fails.
    assert_eq!(error("delay < 100 AND 1 / (delay - 15) = 1").0, "division by zero");
    assert_eq!(error("1 / (delay - 15) = 1 AND delay > 100").0, "division by zero");
    // @end
}

#[test]
fn what_where_does_with_unknown() {
    // @listing where-test
    let keeps = |sql: &str| {
        let bound =
            bind_condition(&parse_expr(sql).unwrap(), &scope(), "WHERE").unwrap();
        truth(&eval(&bound, &row()).unwrap()) == Some(true)
    };
    assert!(keeps("delay > 10"));
    assert!(!keeps("delay > 20"));
    // Unknown is not true, so the row is not kept.
    assert!(!keeps("delay > NULL"));
    // And the opposite of unknown is unknown, so it is not kept then either.
    assert!(!keeps("NOT delay > NULL"));
    assert!(!keeps("NULL"));
    assert!(keeps("delay > NULL OR origin = 'SLC'"));
    // @end
}

#[test]
fn errors_that_point() {
    // @listing explain-test
    let source = "delay * 60 + origin";
    let error = bind(&parse_expr(source).unwrap(), &scope()).unwrap_err();
    assert_eq!(
        explain(source, &error),
        "error: + needs a number, and this is TEXT\n  \
           delay * 60 + origin\n  \
           \x20            ^\n"
    );
    // @end
}
