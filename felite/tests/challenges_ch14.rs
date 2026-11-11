//! Tests for the Chapter 14 challenges: `cargo test --test challenges_ch14`.
use felite::{Counted, Distinct, insert_select, insert_values};
use felite::{
    Database, Filter, MemFile, Operator, Output, Project, Row, Scan, Scope, Value,
};
use felite::{bind, bind_condition, parse_expr};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

fn flights() -> Database {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute(
        "CREATE TABLE flights (id INTEGER PRIMARY KEY, carrier TEXT NOT NULL, origin TEXT,
         miles REAL, delay INTEGER)",
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

#[test]
fn predict_the_output() {
    let mut db = flights();
    assert_eq!(
        query(&mut db, "SELECT carrier FROM flights WHERE delay <= 0"),
        ["DL", "WN"]
    );
    assert_eq!(
        query(
            &mut db,
            "SELECT id, delay FROM flights WHERE delay < 50 OR delay IS NULL"
        ),
        ["89 12", "301 NULL", "1432 -7", "2210 0"]
    );
    assert_eq!(
        query(
            &mut db,
            "SELECT id / 100, miles > 1000.0 FROM flights WHERE carrier = 'DL'"
        ),
        ["3 1", "14 0"]
    );
    assert_eq!(
        query(&mut db, "SELECT delay / delay FROM flights WHERE delay > 0"),
        ["1", "1"]
    );
    // The fifth row scanned has a delay of 0.
    let error =
        db.execute("SELECT delay / delay FROM flights").unwrap_err().to_string();
    assert_eq!(error, "error at line 1, column 8: division by zero");
    assert_eq!(
        query(&mut db, "SELECT 7 FROM flights WHERE NULL"),
        Vec::<String>::new()
    );
}

#[test]
fn calls_per_row() {
    let mut db = flights();
    let table = db.table("flights").unwrap().clone();
    let scope = Scope::of(&table);
    let counter = || Rc::new(Cell::new(0));
    let (scans, filters, projects) = (counter(), counter(), counter());

    let scan = Scan::new(&table, db.pager()).unwrap();
    let scan = Counted { input: Box::new(scan), calls: scans.clone() };
    let condition =
        bind_condition(&parse_expr("delay > 0").unwrap(), &scope, "WHERE").unwrap();
    let filter = Filter { input: Box::new(scan), condition };
    let filter = Counted { input: Box::new(filter), calls: filters.clone() };
    let exprs = vec![bind(&parse_expr("carrier").unwrap(), &scope).unwrap()];
    let project = Project { input: Box::new(filter), exprs };
    let mut top = Counted { input: Box::new(project), calls: projects.clone() };

    let mut rows = 0;
    while top.next(db.pager()).unwrap().is_some() {
        rows += 1;
    }
    assert_eq!(rows, 2);
    // Five rows and one "no more" from the scan. Two rows and one "no more" from
    // each operator above the filter.
    assert_eq!((scans.get(), filters.get(), projects.get()), (6, 3, 3));
}

#[test]
fn distinct_rows() {
    let mut db = flights();
    let table = db.table("flights").unwrap().clone();
    let scope = Scope::of(&table);
    let scan = Scan::new(&table, db.pager()).unwrap();
    let exprs = vec![bind(&parse_expr("carrier").unwrap(), &scope).unwrap()];
    let project = Project { input: Box::new(scan), exprs };
    let mut distinct = Distinct { input: Box::new(project), seen: BTreeSet::new() };
    let mut carriers = Vec::new();
    while let Some(row) = distinct.next(db.pager()).unwrap() {
        carriers.push(row[0].to_string());
    }
    // In the order they first appear in the scan. DL is there once.
    assert_eq!(carriers, ["AA", "UA", "DL", "WN"]);
}

#[test]
fn values_without_sql() {
    let mut db = flights();
    let text = |s: &str| Value::Text(s.to_string());
    let row =
        vec![Value::Null, text("AS"), text("SEA"), Value::Integer(680), Value::Null];
    assert_eq!(insert_values(&mut db, "flights", row).unwrap(), 2211);
    assert_eq!(
        query(&mut db, "SELECT * FROM flights WHERE id = 2211"),
        ["2211 AS SEA 680.0 NULL"]
    );

    let bad = |db: &mut Database, row: Row| {
        insert_values(db, "flights", row).unwrap_err().to_string()
    };
    let row =
        vec![Value::Integer(1), Value::Null, Value::Null, Value::Null, Value::Null];
    assert_eq!(bad(&mut db, row), "the column carrier cannot hold NULL");
    let row = vec![
        Value::Integer(1),
        text("AS"),
        Value::Integer(5),
        Value::Null,
        Value::Null,
    ];
    assert_eq!(bad(&mut db, row), "the column origin cannot hold 5");
    let row = vec![text("one"), text("AS"), Value::Null, Value::Null, Value::Null];
    assert_eq!(bad(&mut db, row), "the column id cannot hold one");
    let row =
        vec![Value::Integer(89), text("AS"), Value::Null, Value::Null, Value::Null];
    assert_eq!(bad(&mut db, row), "there is already a row with id 89");
    assert_eq!(bad(&mut db, vec![Value::Null]), "1 values for 5 columns");
    assert_eq!(query(&mut db, "SELECT id FROM flights").len(), 6);
}

#[test]
fn filling_a_table_from_a_query() {
    let mut db = flights();
    db.execute(
        "CREATE TABLE late (flight INTEGER PRIMARY KEY, carrier TEXT, hours REAL)",
    )
    .unwrap();
    let count = insert_select(
        &mut db,
        "late",
        "SELECT id, carrier, delay / 60.0 FROM flights WHERE delay > 0",
    )
    .unwrap();
    assert_eq!(count, 2);
    assert_eq!(
        query(&mut db, "SELECT * FROM late"),
        ["77 AA 1.5833333333333333", "89 UA 0.2"]
    );

    // The query's columns have to suit the table's.
    assert!(
        insert_select(&mut db, "late", "SELECT carrier, id, miles FROM flights")
            .is_err()
    );
    assert!(insert_select(&mut db, "late", "SELECT id FROM flights").is_err());
    assert!(insert_select(&mut db, "late", "DROP TABLE flights").is_err());
    assert_eq!(query(&mut db, "SELECT flight FROM late"), ["77", "89"]);
    assert!(db.table("flights").is_ok()); // the statement that was not a query never ran
}
