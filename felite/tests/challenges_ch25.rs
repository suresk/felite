//! Tests for the Chapter 25 challenges: `cargo test --test challenges_ch25`.
use felite::{Database, MemFile, Output, Row, Value, run_slt};

fn fixture() -> Database {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    for statement in include_str!("../fixtures/flights.sql").lines() {
        db.execute(statement).unwrap();
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
use felite::dump_sql;

// @challenge 4
use felite::sqlite_first_table;

// @challenge 1
#[test]
fn reading_the_refusals() {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    let report = run_slt(&mut db, include_str!("../fixtures/select1.test"));
    let count = |reason: &str| report.refusals.get(reason).copied().unwrap_or(0);
    // CASE … WHEN.
    assert_eq!(count("expected FROM, found the name WHEN"), 227);
    // A subquery, as a value or after EXISTS.
    let subqueries = count("expected an expression, found SELECT")
        + count("there is no function named EXISTS");
    assert_eq!(subqueries, 240);
    assert_eq!(count("there is no function named abs"), 206);
    assert_eq!(report.refusals.values().sum::<usize>(), 862);
}

// @challenge 2
#[test]
fn writing_a_database_as_sql() {
    let mut db = fixture();
    db.execute("CREATE UNIQUE INDEX airports_code ON airports (code)").unwrap();
    db.execute("INSERT INTO carriers VALUES ('O''H', 'Quote''s Air')").unwrap();
    let sql = dump_sql(&mut db).unwrap();
    assert_eq!(sql.lines().count(), 3 + 16 + 334 + 200 + 1);

    let mut copy = Database::open(Box::new(MemFile::new())).unwrap();
    for statement in sql.lines() {
        copy.execute(statement).unwrap();
    }
    for table in ["carriers", "airports", "flights"] {
        let all = format!("SELECT * FROM {table}");
        assert_eq!(query(&mut copy, &all), query(&mut db, &all));
    }
    assert_eq!(copy.table("airports").unwrap().indexes.len(), 1);
    assert_eq!(dump_sql(&mut copy).unwrap(), sql);
    copy.check().unwrap();
}

// @challenge 4
#[test]
fn reading_a_sqlite_file() {
    let rows =
        sqlite_first_table(include_bytes!("../fixtures/airports.sqlite")).unwrap();
    assert_eq!(rows.len(), 334);
    let text = |s: &str| Value::Text(s.to_string());
    // The INTEGER PRIMARY KEY column is NULL in the record, as it is in ours,
    // and its value is the row id.
    assert_eq!(
        rows[0],
        (
            10,
            vec![
                Value::Null,
                text("ABE"),
                text("Allentown/Bethlehem/Easton"),
                text("PA"),
                Value::Integer(-63),
                Value::Real(0.125)
            ]
        )
    );
    assert_eq!(rows[333].0, 3340);
    assert_eq!(rows[333].1[1..4], [text("YUM"), text("Yuma"), text("AZ")]);
    // The same airports, in the same order, as the fixture has.
    let mut db = fixture();
    let codes: Vec<String> = rows.iter().map(|(_, row)| row[1].to_string()).collect();
    assert_eq!(codes, query(&mut db, "SELECT code FROM airports"));
    // What SQLite itself says: 334 rows, 268 with a share, flights adding up
    // to 116,113.
    let flights: i64 = rows
        .iter()
        .map(|(_, row)| if let Value::Integer(n) = row[4] { n } else { 0 })
        .sum();
    let shares = rows.iter().filter(|(_, row)| row[5] != Value::Null).count();
    assert_eq!((flights, shares), (116_113, 268));

    assert!(sqlite_first_table(b"not a database").is_err());
    assert!(sqlite_first_table(&[0; 4096]).is_err());
}
