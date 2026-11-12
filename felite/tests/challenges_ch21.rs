//! Tests for the Chapter 21 challenges: `cargo test --test challenges_ch21`.
use felite::{Database, MemFile, Operator, Output, Row, Scan, Scope, Value};
use felite::{SORT_BUDGET, Sort, SortKey, TempSpace, bind, parse_expr};

fn fixture() -> Database {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    for statement in include_str!("../fixtures/flights.sql").lines() {
        db.execute(statement).unwrap();
    }
    db
}

fn five_flights() -> Database {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute(
        "CREATE TABLE flights (id INTEGER PRIMARY KEY, carrier TEXT NOT NULL,
         origin TEXT, miles REAL, delay INTEGER)",
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
    db
}

fn query(db: &mut Database, sql: &str) -> Vec<String> {
    let Output::Rows(result) = db.execute(sql).unwrap() else {
        panic!("not a query")
    };
    let line =
        |row: &Row| row.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
    result.rows.iter().map(line).collect()
}

// @challenge 2
use felite::count_distinct;

// @challenge 3
use felite::hash_counts;

// @challenge 4
use felite::SortedCount;

// @challenge 5
use felite::median;

// @challenge 1
#[test]
fn predict_groups() {
    let mut db = five_flights();
    assert_eq!(
        query(
            &mut db,
            "SELECT carrier, COUNT(*), COUNT(delay), SUM(delay), AVG(delay)
             FROM flights GROUP BY carrier"
        ),
        ["AA 1 1 95 95.0", "DL 2 1 -7 -7.0", "UA 1 1 12 12.0", "WN 1 1 0 0.0"]
    );
    assert_eq!(query(&mut db, "SELECT AVG(delay) FROM flights"), ["25.0"]);
    assert_eq!(
        query(&mut db, "SELECT COUNT(*), AVG(delay) FROM flights WHERE delay > 100"),
        ["0 NULL"]
    );
    assert_eq!(
        query(
            &mut db,
            "SELECT delay IS NULL, COUNT(*), COUNT(delay) FROM flights
             GROUP BY delay IS NULL"
        ),
        ["0 4 4", "1 1 0"]
    );
}

// @challenge 2
#[test]
fn counting_distinct_values() {
    let mut db = fixture();
    assert_eq!(count_distinct(&mut db, "flights", "carrier").unwrap(), 15);
    assert_eq!(count_distinct(&mut db, "flights", "origin").unwrap(), 83);
    // 196 flights have a tail number, and six aircraft appear twice.
    assert_eq!(count_distinct(&mut db, "flights", "tail").unwrap(), 190);
    assert_eq!(count_distinct(&mut db, "flights", "delay").unwrap(), 86);
    assert_eq!(count_distinct(&mut db, "flights", "delay > 0").unwrap(), 2);
    assert_eq!(count_distinct(&mut db, "flights", "NULL").unwrap(), 0);
    assert!(count_distinct(&mut db, "flights", "nothing").is_err());
}

// @challenge 3
#[test]
fn grouping_with_a_hash_table() {
    let mut db = fixture();
    for expr in ["carrier", "delay", "origin", "miles > 1000.0", "tail IS NULL"] {
        let groups = hash_counts(&mut db, "flights", expr).unwrap();
        let shown: Vec<String> =
            groups.iter().map(|(value, count)| format!("{value} {count}")).collect();
        let sql = format!("SELECT {expr}, COUNT(*) FROM flights GROUP BY {expr}");
        assert_eq!(shown, query(&mut db, &sql), "{expr}");
    }
}

// @challenge 4
#[test]
fn grouping_sorted_input() {
    let mut db = fixture();
    for expr in ["carrier", "delay", "day"] {
        let table = db.table("flights").unwrap().clone();
        let key = bind(&parse_expr(expr).unwrap(), &Scope::of(&table)).unwrap();
        let scan = Scan::new(&table, db.pager()).unwrap();
        let keys = vec![SortKey { expr: key.clone(), descending: false }];
        let sort = Sort::new(Box::new(scan), keys, SORT_BUDGET, TempSpace::Memory);
        let mut counts = SortedCount::new(Box::new(sort), key);
        let mut shown = Vec::new();
        while let Some(row) = counts.next(db.pager()).unwrap() {
            shown.push(format!("{} {}", row[0], row[1]));
        }
        let sql = format!("SELECT {expr}, COUNT(*) FROM flights GROUP BY {expr}");
        assert_eq!(shown, query(&mut db, &sql), "{expr}");
    }
    // Input with no rows has no groups.
    db.execute("CREATE TABLE empty (n INTEGER)").unwrap();
    let table = db.table("empty").unwrap().clone();
    let key = bind(&parse_expr("n").unwrap(), &Scope::of(&table)).unwrap();
    let scan = Scan::new(&table, db.pager()).unwrap();
    let mut counts = SortedCount::new(Box::new(scan), key);
    assert!(counts.next(db.pager()).unwrap().is_none());
}

// @challenge 5
#[test]
fn the_middle_value() {
    let mut db = fixture();
    assert_eq!(median(&mut db, "flights", "delay").unwrap(), Value::Integer(-5));
    assert_eq!(median(&mut db, "flights", "miles").unwrap(), Value::Real(645.0));
    assert_eq!(median(&mut db, "flights", "NULL").unwrap(), Value::Null);
    db.set_sort_budget(500);
    assert_eq!(median(&mut db, "flights", "delay").unwrap(), Value::Integer(-5));
    let mut five = five_flights();
    assert_eq!(median(&mut five, "flights", "delay").unwrap(), Value::Integer(0));
}
