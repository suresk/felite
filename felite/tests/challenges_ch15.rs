//! Tests for the Chapter 15 challenges: `cargo test --test challenges_ch15`.
use felite::{Database, MemFile, Output, Row, Value};

fn flights() -> (Database, MemFile) {
    let file = MemFile::new();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
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
    (db, file)
}

fn query(db: &mut Database, sql: &str) -> Vec<String> {
    let Output::Rows(result) = db.execute(sql).unwrap() else {
        panic!("not a query")
    };
    let line =
        |row: &Row| row.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
    result.rows.iter().map(line).collect()
}

fn changed(db: &mut Database, sql: &str) -> usize {
    let Output::Count(count) = db.execute(sql).unwrap() else {
        panic!("not a count")
    };
    count
}

// @challenge 1 4
use felite::would_affect;

// @challenge 2
use felite::{account_database, truncate_table};

// @challenge 3
use felite::delete_returning;

// @challenge 1 4
#[test]
fn predict_rows_affected() {
    let (mut db, _) = flights();
    assert_eq!(
        would_affect(&mut db, "DELETE FROM flights WHERE delay >= 0").unwrap(),
        3
    );
    assert_eq!(
        would_affect(&mut db, "DELETE FROM flights WHERE NOT delay >= 0").unwrap(),
        1
    );
    assert_eq!(
        would_affect(&mut db, "UPDATE flights SET delay = 0 WHERE delay = 0")
            .unwrap(),
        1
    );
    assert_eq!(would_affect(&mut db, "UPDATE flights SET delay = NULL").unwrap(), 5);
    assert_eq!(
        would_affect(&mut db, "DELETE FROM flights WHERE delay = NULL").unwrap(),
        0
    );
    assert_eq!(query(&mut db, "SELECT id FROM flights").len(), 5); // nothing was changed

    // An update counts a row it matched even if the new value is the old one.
    assert_eq!(changed(&mut db, "UPDATE flights SET delay = 0 WHERE delay = 0"), 1);
    // Every row is moved, and the statement is one change to each.
    assert_eq!(changed(&mut db, "UPDATE flights SET id = id + 1"), 5);
    assert_eq!(
        changed(&mut db, "UPDATE flights SET delay = delay + 1 WHERE delay > 0"),
        2
    );
    assert_eq!(
        changed(&mut db, "DELETE FROM flights WHERE delay > 0 OR delay IS NULL"),
        3
    );
    assert_eq!(changed(&mut db, "DELETE FROM flights"), 2);
    assert!(would_affect(&mut db, "SELECT * FROM flights").is_err());
}

// @challenge 2
#[test]
fn emptying_a_table_at_once() {
    let (mut db, file) = flights();
    for id in 5000..7000 {
        db.execute(&format!(
            "INSERT INTO flights VALUES ({id}, 'UA', 'ORD', 1.0, 0)"
        ))
        .unwrap();
    }
    let reads = db.pager().stats().reads;
    truncate_table(&mut db, "flights").unwrap();
    assert_eq!(query(&mut db, "SELECT * FROM flights"), Vec::<String>::new());
    assert_eq!(db.pager().stats().reads, reads);
    account_database(&mut db).unwrap();
    db.execute("INSERT INTO flights (carrier) VALUES ('B6')").unwrap();

    let mut db = Database::open(Box::new(file)).unwrap();
    assert_eq!(query(&mut db, "SELECT id, carrier FROM flights"), ["1 B6"]);
    account_database(&mut db).unwrap();
}

// @challenge 3
#[test]
fn getting_back_what_was_deleted() {
    let (mut db, _) = flights();
    let rows = delete_returning(&mut db, "flights", "carrier = 'DL'").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][0], Value::Integer(301));
    assert_eq!(rows[1][2], Value::Text("SLC".to_string()));
    assert_eq!(query(&mut db, "SELECT id FROM flights"), ["77", "89", "2210"]);
    assert!(delete_returning(&mut db, "flights", "carrier +").is_err());
    assert!(delete_returning(&mut db, "flights", "carrier").is_err());
    assert_eq!(delete_returning(&mut db, "flights", "id > 5000").unwrap().len(), 0);
}
