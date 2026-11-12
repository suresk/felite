//! Tests for the Chapter 19 challenges: `cargo test --test challenges_ch19`.
use felite::{
    CrashDisk, Database, Error, MemFile, Output, Pager, Transaction, Value,
};

fn logged() -> Database {
    let pager =
        Pager::open_logged(Box::new(MemFile::new()), Box::new(MemFile::new()));
    Database::with_pager(pager.unwrap()).unwrap()
}

fn numbers(db: &mut Database, sql: &str) -> Vec<i64> {
    let Output::Rows(result) = db.execute(sql).unwrap() else {
        panic!("not a query")
    };
    let number = |row: &Vec<Value>| match row[0] {
        Value::Integer(n) => n,
        _ => 0,
    };
    result.rows.iter().map(number).collect()
}

// @challenge 2
use felite::execute_script;

// @challenge 3
use felite::atomically;

// @challenge 4
use felite::dry_run;

// @challenge 5
use felite::insert_in_batches;

// @challenge 1
#[test]
fn predict_the_table() {
    let mut db = logged();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)").unwrap();
    let mut run = |sql: &str| db.execute(sql).is_ok();
    assert!(run("INSERT INTO t VALUES (1, 10)"));
    assert!(run("BEGIN"));
    assert!(run("INSERT INTO t VALUES (2, 20)"));
    assert!(run("UPDATE t SET n = n + 1"));
    assert!(run("ROLLBACK"));
    assert!(run("BEGIN"));
    assert!(run("INSERT INTO t VALUES (3, 30)"));
    assert!(run("COMMIT"));
    assert!(run("BEGIN"));
    assert!(run("INSERT INTO t VALUES (4, 40)"));
    assert!(!run("INSERT INTO t VALUES (3, 99)")); // id 3 is taken
    assert!(!run("INSERT INTO t VALUES (5, 50)")); // the transaction has failed
    assert!(!run("COMMIT"));
    assert!(run("ROLLBACK"));
    assert!(run("INSERT INTO t VALUES (6, 60)"));
    assert!(!run("INSERT INTO t VALUES (6, 61)")); // fails alone, and undoes only itself
    assert_eq!(numbers(&mut db, "SELECT n FROM t"), [10, 30, 60]);
}

// @challenge 2
#[test]
fn running_a_script() {
    let mut db = logged();
    let outputs = execute_script(
        &mut db,
        "CREATE TABLE t (id INTEGER PRIMARY KEY, note TEXT);
         BEGIN;
         INSERT INTO t (note) VALUES ('one; two');  -- a semicolon inside a string
         INSERT INTO t (note) VALUES ('three');
         COMMIT;;
         SELECT id FROM t",
    )
    .unwrap();
    assert_eq!(outputs.len(), 6);
    assert_eq!(outputs[2], Output::Count(1));
    assert_eq!(numbers(&mut db, "SELECT id FROM t"), [1, 2]);

    // A script stops at the first error. What it had committed stays.
    let failed =
        execute_script(&mut db, "INSERT INTO t (note) VALUES ('four'); nonsense;");
    assert!(matches!(failed, Err(Error::Syntax(..))));
    assert_eq!(numbers(&mut db, "SELECT id FROM t"), [1, 2, 3]);
    assert!(execute_script(&mut db, "  ;; ").unwrap().is_empty());
}

// @challenge 3
#[test]
fn a_transaction_around_a_closure() {
    let mut db = logged();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)").unwrap();
    let count = atomically(&mut db, |db| {
        db.execute("INSERT INTO t VALUES (1, 10)")?;
        db.execute("INSERT INTO t VALUES (2, 20)")?;
        Ok(2)
    });
    assert_eq!(count.unwrap(), 2);

    // A statement fails partway through the work.
    let failed = atomically(&mut db, |db| {
        db.execute("INSERT INTO t VALUES (3, 30)")?;
        db.execute("INSERT INTO t VALUES (1, 99)")?;
        Ok(())
    });
    assert!(failed.is_err());
    assert_eq!(db.transaction(), Transaction::Closed);
    assert_eq!(numbers(&mut db, "SELECT id FROM t"), [1, 2]);

    // The closure itself decides to give up, with every statement having worked.
    let failed: felite::Result<()> = atomically(&mut db, |db| {
        db.execute("INSERT INTO t VALUES (3, 30)")?;
        Err(Error::Sql("on second thought, no".to_string()))
    });
    assert_eq!(failed.unwrap_err().to_string(), "on second thought, no");
    assert_eq!(db.transaction(), Transaction::Closed);
    assert_eq!(numbers(&mut db, "SELECT id FROM t"), [1, 2]);
}

// @challenge 4
#[test]
fn a_dry_run() {
    let mut db = logged();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)").unwrap();
    for n in 1..=5 {
        db.execute(&format!("INSERT INTO t VALUES ({n}, {n})")).unwrap();
    }
    assert_eq!(
        dry_run(&mut db, "DELETE FROM t WHERE n > 2").unwrap(),
        Output::Count(3)
    );
    assert_eq!(dry_run(&mut db, "DROP TABLE t").unwrap(), Output::Done);
    assert_eq!(numbers(&mut db, "SELECT n FROM t"), [1, 2, 3, 4, 5]);
    // Unlike Challenge 15.4, it finds the statements that would fail.
    let error = dry_run(&mut db, "UPDATE t SET id = 1 WHERE id = 5").unwrap_err();
    assert_eq!(error.to_string(), "two rows of t would have the id 1");
    assert_eq!(db.transaction(), Transaction::Closed);
    db.execute("BEGIN").unwrap();
    assert!(dry_run(&mut db, "DELETE FROM t").is_err());
    assert_eq!(db.transaction(), Transaction::Open);
}

// @challenge 5
#[test]
fn inserting_in_batches() {
    let mut syncs = Vec::new();
    for batch in [1, 10, 100, 1000] {
        let disk = CrashDisk::new();
        let pager =
            Pager::open_logged(Box::new(disk.clone()), Box::new(disk.another_file()));
        let mut db = Database::with_pager(pager.unwrap()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)").unwrap();
        let before = disk.syncs();
        insert_in_batches(&mut db, 1000, batch).unwrap();
        syncs.push(disk.syncs() - before);
        assert_eq!(numbers(&mut db, "SELECT n FROM t").len(), 1000);
    }
    // The first figure includes one checkpoint, which is two syncs.
    assert_eq!(syncs, [1002, 100, 10, 1]);
}
