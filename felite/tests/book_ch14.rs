//! The examples in Chapter 14: `cargo test --test book_ch14`.
use felite::{
    Database, Filter, MemFile, Operator, Output, Project, Row, Scan, Scope, Value,
};
use felite::{bind, bind_condition, format_rows, parse_expr, respond};

// @listing helpers
/// A database in memory with a small table of flights in it, and its file.
fn flights() -> (Database, MemFile) {
    let file = MemFile::new();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    db.execute(
        "CREATE TABLE flights (
            id INTEGER PRIMARY KEY, carrier TEXT NOT NULL, origin TEXT,
            miles REAL, delay INTEGER
        )",
    )
    .unwrap();
    for values in [
        "(1432, 'DL', 'SLC', 588.5, -7)",
        "(89, 'UA', 'DEN', 862.0, 12)",
        "(2210, 'WN', 'LAS', 368.0, 0)",
        "(301, 'DL', 'ATL', 1590.0, NULL)",
        "(77, 'AA', 'DFW', 1235.5, 95)",
    ] {
        db.execute(&format!("INSERT INTO flights VALUES {values}")).unwrap();
    }
    (db, file)
}

/// Runs a query and returns its rows as text, one line for each row.
fn query(db: &mut Database, sql: &str) -> Vec<String> {
    let Output::Rows(result) = db.execute(sql).unwrap() else {
        panic!("not a query")
    };
    let line =
        |row: &Row| row.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
    result.rows.iter().map(line).collect()
}

/// The message of the error that a statement produces.
fn error(db: &mut Database, sql: &str) -> String {
    db.execute(sql).unwrap_err().to_string()
}
// @end

#[test]
fn operators_by_hand() {
    // @listing by-hand
    let (mut db, _) = flights();
    let table = db.table("flights").unwrap().clone();
    let scope = Scope::of(&table);
    let bound = |sql: &str| bind(&parse_expr(sql).unwrap(), &scope).unwrap();

    // The operators for: SELECT carrier, miles / 60.0 FROM flights WHERE delay > 0
    let scan = Scan::new(&table, db.pager()).unwrap();
    let condition =
        bind_condition(&parse_expr("delay > 0").unwrap(), &scope, "WHERE").unwrap();
    let filter = Filter { input: Box::new(scan), condition };
    let exprs = vec![bound("carrier"), bound("miles / 60.0")];
    let mut project = Project { input: Box::new(filter), exprs };

    let text = |s: &str| Value::Text(s.to_string());
    let mut next = || project.next(db.pager()).unwrap();
    assert_eq!(next(), Some(vec![text("AA"), Value::Real(1235.5 / 60.0)]));
    assert_eq!(next(), Some(vec![text("UA"), Value::Real(862.0 / 60.0)]));
    assert_eq!(next(), None);
    // @end
}

#[test]
fn the_first_query() {
    // @listing first-query
    let (mut db, _) = flights();
    let output =
        db.execute("SELECT carrier, origin FROM flights WHERE delay > 0").unwrap();
    let Output::Rows(result) = output else { panic!("a SELECT returns rows") };
    assert_eq!(result.columns, ["carrier", "origin"]);
    let text = |s: &str| Value::Text(s.to_string());
    assert_eq!(
        result.rows,
        [vec![text("AA"), text("DFW")], vec![text("UA"), text("DEN")]]
    );
    // @end
}

#[test]
fn rows_come_back_in_row_id_order() {
    // @listing scan-order
    let (mut db, _) = flights();
    // The rows went in as 1432, 89, 2210, 301, 77.
    assert_eq!(
        query(&mut db, "SELECT id FROM flights"),
        ["77", "89", "301", "1432", "2210"]
    );
    assert_eq!(
        query(&mut db, "SELECT * FROM flights WHERE id < 100"),
        ["77 AA DFW 1235.5 95", "89 UA DEN 862.0 12"]
    );
    // @end
}

#[test]
fn what_select_can_ask_for() {
    // @listing select-list-test
    let (mut db, _) = flights();
    let sql =
        "SELECT id, delay * 60 AS seconds, miles / 60.0, delay > 0 FROM flights";
    let Output::Rows(result) = db.execute(sql).unwrap() else { panic!() };
    // A column keeps its name, an alias is used if given, and anything else is
    // named by its own text.
    assert_eq!(result.columns, ["id", "seconds", "(miles / 60.0)", "(delay > 0)"]);
    assert_eq!(query(&mut db, sql)[0], "77 5700 20.591666666666665 1");

    // A star may be mixed with other items, and may appear more than once.
    assert_eq!(
        query(&mut db, "SELECT carrier, * FROM flights WHERE id = 89"),
        ["UA 89 UA DEN 862.0 12"]
    );
    // A value that does not depend on the row is repeated for every row.
    assert_eq!(query(&mut db, "SELECT 1 + 1 FROM flights").len(), 5);
    // @end
}

#[test]
fn where_and_null() {
    // @listing where-null
    let (mut db, _) = flights();
    // Flight 301 has no delay recorded. It is in neither of these.
    assert_eq!(
        query(&mut db, "SELECT id FROM flights WHERE delay > 0"),
        ["77", "89"]
    );
    assert_eq!(
        query(&mut db, "SELECT id FROM flights WHERE NOT delay > 0"),
        ["1432", "2210"]
    );
    // It has to be asked for.
    assert_eq!(query(&mut db, "SELECT id FROM flights WHERE delay IS NULL"), ["301"]);
    assert_eq!(
        query(&mut db, "SELECT id, delay + 1 FROM flights WHERE id = 301"),
        ["301 NULL"]
    );
    // @end
}

#[test]
fn inserting() {
    // @listing insert-test
    let (mut db, _) = flights();
    // Columns can be named, in any order. Those left out are NULL.
    let output =
        db.execute("INSERT INTO flights (carrier, id) VALUES ('B6', 500)").unwrap();
    assert_eq!(output, Output::Count(1));
    assert_eq!(
        query(&mut db, "SELECT * FROM flights WHERE id = 500"),
        ["500 B6 NULL NULL NULL"]
    );

    // With no id given, the row gets the next one after the largest.
    db.execute("INSERT INTO flights (carrier, origin) VALUES ('AS', 'SEA')").unwrap();
    db.execute("INSERT INTO flights VALUES (NULL, 'F9', 'DEN', 100.0, 3)").unwrap();
    assert_eq!(
        query(&mut db, "SELECT id, carrier FROM flights WHERE id > 2000"),
        ["2210 WN", "2211 AS", "2212 F9"]
    );
    // Values are expressions, and an integer is accepted where a real is wanted.
    db.execute("INSERT INTO flights VALUES (2 * 5, 'NK', 'FLL', 1000, 60 / 4)")
        .unwrap();
    assert_eq!(
        query(&mut db, "SELECT * FROM flights WHERE id = 10"),
        ["10 NK FLL 1000.0 15"]
    );
    // @end
}

#[test]
fn inserts_that_are_refused() {
    // @listing insert-errors
    let (mut db, _) = flights();
    let before = query(&mut db, "SELECT * FROM flights");
    for (sql, message) in [
        ("INSERT INTO flights VALUES (1, 'DL')", "2 values for 5 columns"),
        ("INSERT INTO flights (id, carrier) VALUES (1)", "1 values for 2 columns"),
        (
            "INSERT INTO flights (id, tail) VALUES (1, 'N1')",
            "the table flights has no column named tail",
        ),
        (
            "INSERT INTO flights (id, id) VALUES (1, 2)",
            "the column id is given a value twice",
        ),
        ("INSERT INTO flights (id) VALUES (1)", "the column carrier cannot be NULL"),
        (
            "INSERT INTO flights (id, carrier) VALUES (1, NULL)",
            "the column carrier cannot be NULL",
        ),
        (
            "INSERT INTO flights (id, carrier) VALUES (89, 'UA')",
            "the table flights already has a row with id 89",
        ),
        ("INSERT INTO nowhere VALUES (1)", "there is no table named nowhere"),
        (
            "INSERT INTO flights (carrier, delay) VALUES ('DL', 'late')",
            "the column delay is an INTEGER, and this is TEXT",
        ),
        (
            "INSERT INTO flights (carrier, delay) VALUES ('DL', 1.5)",
            "the column delay is an INTEGER, and this is a REAL",
        ),
        (
            "INSERT INTO flights (carrier, delay) VALUES ('DL', delay + 1)",
            "there is no column named delay",
        ),
        (
            "INSERT INTO flights (carrier, delay) VALUES ('DL', 1 / 0)",
            "division by zero",
        ),
    ] {
        assert!(error(&mut db, sql).ends_with(message), "{sql}");
    }
    // The last four are errors with a position, which is where the value begins.
    assert_eq!(
        error(&mut db, "INSERT INTO flights (carrier, delay) VALUES ('DL', 1 / 0)"),
        "error at line 1, column 52: division by zero"
    );
    // Not one of them changed the table.
    assert_eq!(query(&mut db, "SELECT * FROM flights"), before);
    // @end
}

#[test]
fn rows_of_any_size_through_sql() {
    // @listing big-rows
    let (mut db, file) = flights();
    let essay = "delayed ".repeat(2000); // 16,000 characters
    db.execute(&format!(
        "INSERT INTO flights (id, carrier, origin) VALUES (9000, 'DL', '{essay}')"
    ))
    .unwrap();
    for id in 3000..3500 {
        db.execute(&format!(
            "INSERT INTO flights VALUES ({id}, 'UA', 'ORD', 1.5, {id} % 7)"
        ))
        .unwrap();
    }
    assert_eq!(query(&mut db, "SELECT id FROM flights WHERE delay = 6").len(), 72);
    drop(db);

    // All of it was flushed, statement by statement, and is there in a new session.
    let mut db = Database::open(Box::new(file)).unwrap();
    assert_eq!(query(&mut db, "SELECT id FROM flights").len(), 506);
    assert_eq!(query(&mut db, "SELECT origin FROM flights WHERE id = 9000"), [essay]);
    // @end
}

#[test]
fn a_result_as_text() {
    // @listing format-test
    let (mut db, _) = flights();
    let Output::Rows(result) = db
        .execute("SELECT id, carrier, miles, delay FROM flights WHERE miles > 600.0")
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        format_rows(&result),
        "id   carrier  miles   delay\n\
         --   -------  -----   -----\n\
         77   AA       1235.5  95\n\
         89   UA       862.0   12\n\
         301  DL       1590.0  NULL\n"
    );
    // @end
}

#[test]
fn a_session_at_the_prompt() {
    // @listing transcript
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    let mut say = |input: &str| respond(&mut db, input);
    assert_eq!(say("CREATE TABLE airports (code TEXT NOT NULL, city TEXT);"), "ok\n");
    assert_eq!(
        say("INSERT INTO airports VALUES ('SLC', 'Salt Lake City');"),
        "1 row\n"
    );
    assert_eq!(say("INSERT INTO airports VALUES ('DEN', 'Denver');"), "1 row\n");
    assert_eq!(
        say("SELECT * FROM airports WHERE code <> 'DEN';"),
        "code  city\n----  ----\nSLC   Salt Lake City\n(1 row)\n"
    );
    assert_eq!(say(".tables"), "airports\n");
    assert_eq!(
        say(".schema"),
        "CREATE TABLE airports (code TEXT NOT NULL, city TEXT);\n"
    );
    assert_eq!(
        say("SELECT * FROM airport;"),
        "error: there is no table named airport\n"
    );
    assert_eq!(
        say("SELECT code FROM airports WHERE city = 5;"),
        "error: this is an INTEGER, and = compares it with TEXT\n  \
           SELECT code FROM airports WHERE city = 5;\n  \
           \x20                                      ^\n"
    );
    assert_eq!(
        say("SELECT code airports;"),
        "syntax error: expected `FROM`, found the name `airports`\n  \
           SELECT code airports;\n  \
           \x20           ^\n"
    );
    // @end
}
