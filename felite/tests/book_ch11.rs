//! The examples in Chapter 11: `cargo test --test book_ch11`.
use felite::{
    BinaryOp, ColumnDef, ColumnType, CreateTable, Error, ExprKind, Insert, Select,
};
use felite::{SelectItem, Statement, Value, explain, parse, parse_expr};

// @listing helpers
/// An expression, parsed and written back out with every operation in
/// parentheses.
fn grouped(source: &str) -> String {
    parse_expr(source).unwrap().to_string()
}

/// The message of the syntax error a statement produces, and the text it points at.
fn error(source: &str) -> (String, String) {
    match parse(source) {
        Err(Error::Syntax(message, at)) => (message, source[at.offset..].to_string()),
        other => panic!("expected a syntax error, got {other:?}"),
    }
}
// @end

#[test]
fn a_tree_for_an_expression() {
    // @listing first-tree
    let expr = parse_expr("1 + 2 * 3").unwrap();
    // The root is the addition. Its right side is the multiplication.
    let ExprKind::Binary { op, left, right } = &expr.kind else { panic!() };
    assert_eq!(*op, BinaryOp::Add);
    assert_eq!(left.kind, ExprKind::Literal(Value::Integer(1)));
    let ExprKind::Binary { op, left, right } = &right.kind else { panic!() };
    assert_eq!(*op, BinaryOp::Multiply);
    assert_eq!(left.kind, ExprKind::Literal(Value::Integer(2)));
    assert_eq!(right.kind, ExprKind::Literal(Value::Integer(3)));

    // Printing the tree shows the same thing in one line.
    assert_eq!(expr.to_string(), "(1 + (2 * 3))");
    // @end
}

#[test]
fn precedence() {
    // @listing precedence
    assert_eq!(grouped("1 + 2 * 3"), "(1 + (2 * 3))");
    assert_eq!(grouped("1 * 2 + 3"), "((1 * 2) + 3)");
    assert_eq!(grouped("(1 + 2) * 3"), "((1 + 2) * 3)");
    // Operators of the same strength group from the left.
    assert_eq!(grouped("1 - 2 - 3"), "((1 - 2) - 3)");
    assert_eq!(grouped("8 / 4 / 2"), "((8 / 4) / 2)");
    assert_eq!(grouped("1 - 2 + 3"), "((1 - 2) + 3)");
    // Arithmetic binds tighter than comparison, and comparison tighter than logic.
    assert_eq!(grouped("a + 1 < b * 2"), "((a + 1) < (b * 2))");
    assert_eq!(
        grouped("a = 1 OR b = 2 AND c = 3"),
        "((a = 1) OR ((b = 2) AND (c = 3)))"
    );
    // @end
}

#[test]
fn prefix_operators() {
    // @listing prefix-test
    assert_eq!(grouped("-5"), "(-5)");
    assert_eq!(grouped("-a * b"), "((-a) * b)");
    assert_eq!(grouped("a - -b"), "(a - (-b))");
    assert_eq!(grouped("- - a"), "(-(-a))");
    // NOT is weaker than a comparison and stronger than AND.
    assert_eq!(grouped("NOT a = b"), "(NOT (a = b))");
    assert_eq!(grouped("NOT a = b OR c"), "((NOT (a = b)) OR c)");
    assert_eq!(grouped("NOT a AND b"), "((NOT a) AND b)");
    assert_eq!(grouped("NOT NOT a"), "(NOT (NOT a))");
    // @end
}

#[test]
fn is_null() {
    // @listing is-null-test
    assert_eq!(grouped("a IS NULL"), "(a IS NULL)");
    assert_eq!(grouped("a IS NOT NULL"), "(a IS NOT NULL)");
    assert_eq!(grouped("a + 1 IS NULL"), "((a + 1) IS NULL)");
    assert_eq!(grouped("a IS NULL OR b IS NULL"), "((a IS NULL) OR (b IS NULL))");
    assert_eq!(grouped("NOT a IS NULL"), "(NOT (a IS NULL))");
    // @end
}

#[test]
fn the_leaves_of_a_tree() {
    // @listing leaves
    assert_eq!(grouped("42"), "42");
    assert_eq!(grouped("1.0"), "1.0");
    assert_eq!(grouped("'it''s'"), "'it''s'");
    assert_eq!(grouped("NULL"), "NULL");
    assert_eq!(grouped("dep_delay"), "dep_delay");
    assert_eq!(grouped("flights.origin"), "flights.origin");
    assert_eq!(grouped("((7))"), "7"); // parentheses leave no trace in the tree
    let expr = parse_expr("flights.origin").unwrap();
    let ExprKind::Column { table, name } = expr.kind else { panic!() };
    assert_eq!((table.as_deref(), name.as_str()), (Some("flights"), "origin"));
    // @end
}

#[test]
fn create_table() {
    // @listing create-test
    let statement = parse(
        "CREATE TABLE flights (
            id INTEGER PRIMARY KEY,
            carrier TEXT NOT NULL,
            distance REAL
        );",
    )
    .unwrap();
    let column = |name: &str, kind, primary_key, not_null| ColumnDef {
        name: name.to_string(),
        kind,
        primary_key,
        not_null,
    };
    assert_eq!(
        statement,
        Statement::CreateTable(CreateTable {
            name: "flights".to_string(),
            columns: vec![
                column("id", ColumnType::Integer, true, false),
                column("carrier", ColumnType::Text, false, true),
                column("distance", ColumnType::Real, false, false),
            ],
        })
    );
    // @end
}

#[test]
fn insert_and_select() {
    // @listing statements-test
    let Statement::Insert(Insert { table, columns, values }) =
        parse("INSERT INTO flights (id, carrier) VALUES (1432, 'DL')").unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (table.as_str(), columns),
        ("flights", vec!["id".to_string(), "carrier".to_string()])
    );
    assert_eq!(values.len(), 2);
    assert_eq!(values[1].kind, ExprKind::Literal(Value::Text("DL".to_string())));

    // With no column list, the list is empty.
    let Statement::Insert(insert) =
        parse("insert into t values (1, 2 + 3, null)").unwrap()
    else {
        panic!()
    };
    assert!(insert.columns.is_empty());
    assert_eq!(insert.values[1].to_string(), "(2 + 3)");

    let Statement::Select(Select { items, from, filter }) =
        parse("SELECT *, dep_delay / 60.0 AS hours FROM flights WHERE dep_delay > 0")
            .unwrap()
    else {
        panic!()
    };
    assert_eq!(from, "flights");
    assert_eq!(items[0], SelectItem::Star);
    let SelectItem::Expr { expr, alias } = &items[1] else { panic!() };
    assert_eq!(
        (expr.to_string().as_str(), alias.as_deref()),
        ("(dep_delay / 60.0)", Some("hours"))
    );
    assert_eq!(filter.unwrap().to_string(), "(dep_delay > 0)");
    // @end
}

#[test]
fn statements_print_as_sql() {
    // @listing print-test
    let lines = [
        "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL, x REAL)",
        "INSERT INTO t (id, name) VALUES (1, 'it''s')",
        "INSERT INTO t VALUES ((1 + 2), NULL)",
        "SELECT *, (x * 2.0) AS twice FROM t WHERE ((x > 1.5) AND (name IS NOT NULL))",
        "SELECT name FROM t",
    ];
    for line in lines {
        assert_eq!(parse(line).unwrap().to_string(), line);
    }
    // Whatever the layout and the case of the keywords, the printed form is one.
    let messy = "select name ,x+1 as y\n  from   t where not x<2;";
    assert_eq!(
        parse(messy).unwrap().to_string(),
        "SELECT name, (x + 1) AS y FROM t WHERE (NOT (x < 2))"
    );
    // @end
}

#[test]
fn good_error_messages() {
    // @listing errors
    let message = |source: &str| error(source).0;
    assert_eq!(
        message("SELECT name WHERE age > 21"),
        "expected `FROM`, found `WHERE`"
    );
    assert_eq!(message("SELECT FROM people"), "expected an expression, found `FROM`");
    assert_eq!(
        message("SELECT name FROM"),
        "expected a table name, found the end of the statement"
    );
    assert_eq!(message("SELECT 1 + FROM t"), "expected an expression, found `FROM`");
    assert_eq!(message("SELECT (1 + 2 FROM t"), "expected `)`, found `FROM`");
    assert_eq!(
        message("SELECT a FROM t WHERE"),
        "expected an expression, found the end of the statement"
    );
    assert_eq!(
        message("SELECT a FROM t t2"),
        "expected the end of the statement, found the name `t2`"
    );
    assert_eq!(
        message("INSERT flights VALUES (1)"),
        "expected `INTO`, found the name `flights`"
    );
    assert_eq!(message("INSERT INTO t VALUES 1"), "expected `(`, found the number 1");
    assert_eq!(
        message("CREATE TABLE t (id BIGINT)"),
        "expected a column type (`INTEGER`, `REAL` or `TEXT`), found the name `BIGINT`"
    );
    assert_eq!(
        message("CREATE TABLE t (id INTEGER NOT)"),
        "expected `NULL`, found `)`"
    );
    assert_eq!(
        message("CREATE TABLE select (id INTEGER)"),
        "expected a table name, found `SELECT`"
    );
    assert_eq!(message("DROP TABLE t"), "expected a statement, found `DROP`");
    assert_eq!(message("a IS 3"), "expected a statement, found the name `a`");
    // An error from the tokenizer comes through unchanged.
    assert_eq!(message("SELECT 'oops FROM t"), "this string is never closed");
    // @end
}

#[test]
fn errors_point_at_the_right_word() {
    // @listing error-place
    let source = "SELECT name, age\nWHERE age > 21";
    assert_eq!(error(source).1, "WHERE age > 21");
    assert_eq!(
        explain(source, &parse(source).unwrap_err()),
        "syntax error: expected `FROM`, found `WHERE`\n  WHERE age > 21\n  ^\n"
    );
    // The span of an expression is where it starts.
    let expr = parse_expr("a + b * c").unwrap();
    let ExprKind::Binary { right, .. } = &expr.kind else { panic!() };
    assert_eq!((expr.span.column, right.span.column), (1, 5));
    // @end
}

#[test]
fn nesting_has_a_limit() {
    // @listing depth-test
    // A hundred thousand opening parentheses, then a 1, then as many closing.
    let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
    let Err(error) = parse_expr(&deep) else { panic!("this should not parse") };
    assert_eq!(
        error.to_string(),
        "syntax error at line 1, column 101: an expression more than 100 levels deep"
    );
    // The same for a hundred thousand minus signs, and for a sum of that many ones.
    assert!(parse_expr(&format!("{}1", "-".repeat(100_000))).is_err());
    assert!(parse_expr(&format!("1{}", " + 1".repeat(100_000))).is_err());

    // Expressions of a sensible size are untouched by the limit.
    assert!(parse_expr(&format!("{}1{}", "(".repeat(90), ")".repeat(90))).is_ok());
    let sum = parse_expr(&format!("1{}", " + 1".repeat(90))).unwrap();
    assert_eq!(sum.to_string().matches('+').count(), 90);
    // @end
}
