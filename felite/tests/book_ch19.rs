//! The examples in Chapter 19: `cargo test --test book_ch19`.
use felite::{CrashDisk, DEFAULT_FRAMES, Database, MemFile, Output, Pager, Storage};
use felite::{Transaction, Value};

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// @listing helpers
/// Opens the database in a file and its log.
fn open(file: impl Storage + 'static, log: impl Storage + 'static) -> Database {
    let pager = Pager::open_logged(Box::new(file), Box::new(log)).unwrap();
    Database::with_pager(pager).unwrap()
}

/// The bank of Section 16.8, with a log: 600 accounts of 100 each, on a disk
/// that can crash. Returns the database, its file and its log.
fn bank() -> (Database, CrashDisk, CrashDisk) {
    let disk = CrashDisk::new();
    let log = disk.another_file();
    let mut db = open(disk.clone(), log.clone());
    db.execute(
        "CREATE TABLE accounts (
            id INTEGER PRIMARY KEY, branch TEXT, balance INTEGER
        )",
    )
    .unwrap();
    for id in 1..=600 {
        let branch = if id % 2 == 0 { "north" } else { "south" };
        let insert = format!("INSERT INTO accounts VALUES ({id}, '{branch}', 100)");
        db.execute(&insert).unwrap();
    }
    (db, disk, log)
}

/// The integers in the first column of a query's result.
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

/// The sum of every balance.
fn total(db: &mut Database) -> i64 {
    numbers(db, "SELECT balance FROM accounts").iter().sum()
}
// @end

const BONUS: &str =
    "UPDATE accounts SET balance = balance + 5 WHERE branch = 'north'";

#[test]
fn a_statement_that_fails() {
    // @listing fail-test
    // With no log, as in every chapter before this one.
    let disk = CrashDisk::new();
    let mut db = Database::open(Box::new(disk.clone())).unwrap();
    db.execute(
        "CREATE TABLE accounts (
            id INTEGER PRIMARY KEY, branch TEXT, balance INTEGER
        )",
    )
    .unwrap();
    for id in 1..=600 {
        let branch = if id % 2 == 0 { "north" } else { "south" };
        let insert = format!("INSERT INTO accounts VALUES ({id}, '{branch}', 100)");
        db.execute(&insert).unwrap();
    }
    // The disk fails after two of the bonus's four page writes.
    disk.stop_after(2);
    assert!(db.execute(BONUS).is_err());
    // The statement failed, and the database goes on showing what it did.
    assert_eq!(total(&mut db), 61_500);

    // With a log, a statement that fails is undone.
    let (mut db, disk, _) = bank();
    disk.stop_after(2);
    assert!(db.execute(BONUS).is_err());
    assert_eq!(total(&mut db), 60_000);
    assert_eq!(db.pager().log_frames(), 612); // the two frames it wrote are forgotten
    db.check().unwrap();
    // @end
}

#[test]
fn begin_and_commit() {
    // @listing commit-test
    let (mut db, disk, log) = bank();
    let before = (disk.writes(), disk.syncs());
    assert_eq!(db.transaction(), Transaction::Closed);
    db.execute("BEGIN").unwrap();
    assert_eq!(db.transaction(), Transaction::Open);
    db.execute("UPDATE accounts SET balance = balance - 30 WHERE id = 1").unwrap();
    db.execute("UPDATE accounts SET balance = balance + 30 WHERE id = 600").unwrap();
    // Two statements have run, and the disk has not heard about either.
    assert_eq!((disk.writes(), disk.syncs()), before);
    // The transaction can see its own changes.
    let balances = "SELECT balance FROM accounts WHERE id = 1 OR id = 600";
    assert_eq!(numbers(&mut db, balances), [70, 130]);

    db.execute("COMMIT").unwrap();
    assert_eq!(db.transaction(), Transaction::Closed);
    // Two leaves changed: two frames, and one sync for both statements.
    assert_eq!((disk.writes() - before.0, disk.syncs() - before.1), (2, 1));

    let mut db = open(disk.kill(), log.kill());
    assert_eq!(numbers(&mut db, balances), [70, 130]);
    // @end

    // @listing misuse-test
    let error =
        |db: &mut Database, sql: &str| db.execute(sql).unwrap_err().to_string();
    assert_eq!(error(&mut db, "COMMIT"), "no transaction is open");
    assert_eq!(error(&mut db, "ROLLBACK"), "no transaction is open");
    db.execute("BEGIN").unwrap();
    assert_eq!(error(&mut db, "BEGIN"), "a transaction is already open");
    db.execute("COMMIT").unwrap();
    // A database with no log has no transactions.
    let mut plain = Database::open(Box::new(MemFile::new())).unwrap();
    assert_eq!(
        error(&mut plain, "BEGIN"),
        "a database with no log has no transactions"
    );
    // @end
}

#[test]
fn a_transfer_at_every_crash_point() {
    // @listing transfer-test
    let balances = "SELECT balance FROM accounts WHERE id = 1 OR id = 600";
    for writes in 0..=2 {
        let (mut db, disk, log) = bank();
        disk.stop_after(writes);
        db.execute("BEGIN").unwrap();
        db.execute("UPDATE accounts SET balance = balance - 30 WHERE id = 1")
            .unwrap();
        db.execute("UPDATE accounts SET balance = balance + 30 WHERE id = 600")
            .unwrap();
        assert!(db.execute("COMMIT").is_err());
        for seed in 0..20 {
            let mut db = open(disk.power_cut(seed), log.power_cut(seed + 1000));
            db.check().unwrap();
            let found = numbers(&mut db, balances);
            // Both halves of the transfer, or neither. Never one.
            assert!(found == [100, 100] || found == [70, 130]);
            assert_eq!(total(&mut db), 60_000);
        }
    }
    // @end
}

#[test]
fn rolling_back() {
    // @listing rollback-test
    let (mut db, disk, _) = bank();
    let before = (db.pager().page_count().unwrap(), db.pager().log_frames());
    assert_eq!(before, (7, 612));

    db.execute("BEGIN").unwrap();
    db.execute(BONUS).unwrap();
    db.execute("CREATE TABLE loans (id INTEGER PRIMARY KEY, amount INTEGER)")
        .unwrap();
    for amount in 0..500 {
        db.execute(&format!("INSERT INTO loans (amount) VALUES ({amount})")).unwrap();
    }
    assert_eq!(total(&mut db), 61_500);
    assert_eq!(numbers(&mut db, "SELECT amount FROM loans").len(), 500);
    assert_eq!(db.pager().page_count().unwrap(), 11);

    db.execute("ROLLBACK").unwrap();
    // The bonus is gone. So is the table, from the catalog and from the file:
    // the header's page count is back to 7, and no page was leaked or freed.
    assert_eq!(total(&mut db), 60_000);
    assert!(db.table("loans").is_err());
    assert_eq!((db.pager().page_count().unwrap(), db.pager().log_frames()), before);
    assert_eq!(db.pager().free_count().unwrap(), 0);
    db.check().unwrap();
    assert_eq!(disk.pending(), 0);
    // @end
}

#[test]
fn rolling_back_what_reached_the_log() {
    // @listing evicted-rollback-test
    let (mut db, disk, log) = bank();
    db.pager().set_capacity(4);
    db.execute("BEGIN").unwrap();
    db.execute("CREATE TABLE loans (id INTEGER PRIMARY KEY, amount INTEGER)")
        .unwrap();
    for amount in 0..2000 {
        db.execute(&format!("INSERT INTO loans (amount) VALUES ({amount})")).unwrap();
    }
    // Four frames of memory were not enough, so pages have gone to the log.
    let frames = db.pager().log_frames();
    assert_eq!(frames, 625);

    db.execute("ROLLBACK").unwrap();
    assert_eq!(db.pager().log_frames(), 612);
    assert!(db.table("loans").is_err());
    db.check().unwrap();

    // The next commit writes over the abandoned frames, and a crash after it
    // finds the bonus and no trace of the loans.
    db.pager().set_capacity(DEFAULT_FRAMES);
    db.execute(BONUS).unwrap();
    assert_eq!(db.pager().log_frames(), 616);
    let mut db = open(disk.kill(), log.kill());
    assert_eq!(total(&mut db), 61_500);
    assert!(db.table("loans").is_err());
    db.check().unwrap();
    // @end
}

#[test]
fn rolling_back_a_new_database() {
    // @listing new-rollback-test
    let mut db = open(MemFile::new(), MemFile::new());
    db.execute("BEGIN").unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
    db.execute("ROLLBACK").unwrap();
    // Nothing was ever committed, so this is a new, empty database again.
    assert_eq!(db.tables().count(), 0);
    assert_eq!(db.pager().page_count().unwrap(), 2);
    db.check().unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
    db.check().unwrap();
    // @end
}

#[test]
fn a_transaction_that_fails() {
    // @listing failed-test
    let (mut db, _, _) = bank();
    db.execute("BEGIN").unwrap();
    db.execute("INSERT INTO accounts VALUES (601, 'north', 100)").unwrap();
    // An account number that is taken.
    let error =
        db.execute("INSERT INTO accounts VALUES (1, 'north', 100)").unwrap_err();
    assert_eq!(error.to_string(), "the table accounts already has a row with id 1");
    assert_eq!(db.transaction(), Transaction::Failed);

    // Account 601 went with it, and nothing more will be accepted.
    let refused = "this transaction has failed, and only ROLLBACK can end it";
    for sql in ["SELECT * FROM accounts", BONUS, "COMMIT"] {
        assert_eq!(db.execute(sql).unwrap_err().to_string(), refused);
    }
    db.execute("ROLLBACK").unwrap();
    assert_eq!(db.transaction(), Transaction::Closed);
    assert_eq!(numbers(&mut db, "SELECT id FROM accounts WHERE id > 600"), []);
    assert_eq!(total(&mut db), 60_000);
    // @end
}

#[test]
fn what_a_transaction_costs() {
    // @listing cost-test
    let insert = |db: &mut Database, n: i64| {
        db.execute(&format!("INSERT INTO t (n) VALUES ({n})")).unwrap();
    };
    // 500 rows, a statement at a time.
    let disk = CrashDisk::new();
    let mut db = open(disk.clone(), disk.another_file());
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)").unwrap();
    let before = (disk.writes(), disk.syncs());
    for n in 0..500 {
        insert(&mut db, n);
    }
    assert_eq!((disk.writes() - before.0, disk.syncs() - before.1), (506, 500));

    // The same rows in one transaction.
    let disk = CrashDisk::new();
    let mut db = open(disk.clone(), disk.another_file());
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)").unwrap();
    let before = (disk.writes(), disk.syncs());
    db.execute("BEGIN").unwrap();
    for n in 0..500 {
        insert(&mut db, n);
    }
    db.execute("COMMIT").unwrap();
    assert_eq!((disk.writes() - before.0, disk.syncs() - before.1), (5, 1));
    // @end
}

// @listing script
/// A bank of 120 accounts set up in one transaction, and then 40 transactions
/// that each make two transfers between accounts picked by a seed. Every fourth
/// one is rolled back. The owners' names are long, to spread the accounts over
/// several pages.
fn script() -> Vec<String> {
    let mut sql = vec!["BEGIN".to_string()];
    let columns = "id INTEGER PRIMARY KEY, owner TEXT, balance INTEGER";
    sql.push(format!("CREATE TABLE accounts ({columns})"));
    for id in 1..=120 {
        let owner = "x".repeat(100);
        sql.push(format!("INSERT INTO accounts VALUES ({id}, '{owner}', 100)"));
    }
    sql.push("COMMIT".to_string());
    let mut state = 19;
    for transaction in 0..40 {
        sql.push("BEGIN".to_string());
        for _ in 0..2 {
            let from = 1 + next(&mut state) % 120;
            let to = 1 + next(&mut state) % 120;
            let amount = 1 + next(&mut state) % 50;
            let change = "UPDATE accounts SET balance = balance";
            sql.push(format!("{change} - {amount} WHERE id = {from}"));
            sql.push(format!("{change} + {amount} WHERE id = {to}"));
        }
        let end = if transaction % 4 == 3 { "ROLLBACK" } else { "COMMIT" };
        sql.push(end.to_string());
    }
    sql
}
// @end

// @listing run
/// Every balance, in order of account, or nothing if there is no bank yet.
fn balances(db: &mut Database) -> Vec<i64> {
    match db.table("accounts") {
        Ok(_) => numbers(db, "SELECT balance FROM accounts"),
        Err(_) => Vec::new(),
    }
}

/// Runs the script on a database in these two files until a statement fails.
/// Returns how many statements finished, and adds to `states` what the bank
/// holds after each as far as a crash is concerned: its last committed balances.
fn run(disk: &CrashDisk, log: &CrashDisk, states: &mut Vec<Vec<i64>>) -> usize {
    let pager = Pager::open_logged(Box::new(disk.clone()), Box::new(log.clone()));
    let Ok(mut db) = pager.and_then(Database::with_pager) else { return 0 };
    db.pager().checkpoint_after(20);
    let (mut done, mut committed) = (0, Vec::new());
    for statement in &script() {
        if db.execute(statement).is_err() {
            break;
        }
        done += 1;
        if db.transaction() == Transaction::Closed {
            committed = balances(&mut db);
        }
        states.push(committed.clone());
    }
    done
}
// @end

#[test]
fn transactions_at_every_crash_point() {
    // @listing crash-test
    // What a crash should leave after each statement, from a run without one.
    let disk = CrashDisk::new();
    let mut states = vec![Vec::new()];
    assert_eq!(run(&disk, &disk.another_file(), &mut states), script().len());
    assert_eq!((script().len(), disk.writes()), (363, 108));

    let mut crashes = 0;
    for writes in 0..=disk.writes() {
        let disk = CrashDisk::new();
        let log = disk.another_file();
        disk.stop_after(writes);
        let done = run(&disk, &log, &mut Vec::new());
        for seed in 0..3 {
            let mut db = open(disk.power_cut(seed), log.power_cut(seed + 1000));
            db.check().unwrap();
            let found = balances(&mut db);
            // The last commit before the crash, or the one the crash interrupted.
            assert!(found == states[done] || Some(&found) == states.get(done + 1));
            // Money is never made or lost.
            assert!(found.is_empty() || found.iter().sum::<i64>() == 12_000);
            crashes += 1;
        }
    }
    assert_eq!(crashes, 327);
    // @end
}
