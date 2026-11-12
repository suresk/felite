//! The examples in Chapter 25: `cargo test --test book_ch25`.
use felite::{
    CrashDisk, Database, IMPORT_BATCH, MemFile, Output, Pager, Row, Storage,
};
use felite::{md5, read_record, respond, run_slt};
use std::io::Write;
use std::process::{Command, Stdio};

/// The example data, loaded into a database in memory, and the database's file.
fn fixture() -> (Database, MemFile) {
    let file = MemFile::new();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    for statement in include_str!("../fixtures/flights.sql").lines() {
        db.execute(statement).unwrap();
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

/// Opens the database in a file and its log.
fn open(file: impl Storage + 'static, log: impl Storage + 'static) -> Database {
    let pager = Pager::open_logged(Box::new(file), Box::new(log)).unwrap();
    Database::with_pager(pager).unwrap()
}

#[test]
fn reading_csv() {
    // @listing csv-test
    let text = "2736,ATL,\"Atlanta, GA\",-18\n\
                5472,,\"She said \"\"go\"\"\",\r\n\
                \"two\nlines\",x\n\
                \n\
                last";
    let mut input = text.as_bytes();
    let mut records = Vec::new();
    while let Some(record) = read_record(&mut input).unwrap() {
        records.push(record);
    }
    assert_eq!(
        records,
        [
            vec!["2736", "ATL", "Atlanta, GA", "-18"],
            vec!["5472", "", "She said \"go\"", ""],
            vec!["two\nlines", "x"],
            vec![""],
            vec!["last"],
        ]
    );
    // A quote that is never closed runs to the end of the input, and that is
    // an error.
    assert!(read_record(&mut "1,\"oops\n2,3\n".as_bytes()).is_err());
    // @end
}

#[test]
fn importing() {
    // @listing import-test
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute(
        "CREATE TABLE stops (id INTEGER PRIMARY KEY, code TEXT NOT NULL, miles REAL)",
    )
    .unwrap();
    // A header that names the columns is passed over. An empty field is NULL,
    // and a number for a REAL column may be written as a whole number.
    let csv = "id,code,miles\n7,ATL,170.5\n8,\"DHN\",\n9,SLC,1590\n";
    assert_eq!(db.import("stops", &mut csv.as_bytes()).unwrap(), 3);
    assert_eq!(
        query(&mut db, "SELECT * FROM stops"),
        ["7 ATL 170.5", "8 DHN NULL", "9 SLC 1590.0"]
    );
    // What INSERT refuses, an import refuses.
    let message = |db: &mut Database, csv: &str| {
        db.import("stops", &mut csv.as_bytes()).unwrap_err().to_string()
    };
    assert_eq!(message(&mut db, "10,BOS\n"), "row 1 has 2 fields, and not 3");
    assert_eq!(
        message(&mut db, "ten,BOS,1.0\n"),
        "\"ten\" is not a value for the column id"
    );
    assert_eq!(message(&mut db, "10,,1.0\n"), "the column code cannot be NULL");
    assert_eq!(
        message(&mut db, "9,BOS,1.0\n"),
        "the table stops already has a row with id 9"
    );
    // The prompt has a command for it.
    assert!(respond(&mut db, ".import nowhere.csv stops").starts_with("error: "));
    // @end
}

// @listing rows
/// `count` rows of comma-separated values: a number and a word.
fn rows(count: usize) -> String {
    (0..count).map(|n| format!("{n},row {n}\n")).collect()
}
// @end

#[test]
fn importing_in_batches() {
    // @listing batch-test
    let disk = CrashDisk::new();
    let log = disk.another_file();
    let mut db = open(disk.clone(), log.clone());
    db.execute("CREATE TABLE t (n INTEGER, word TEXT)").unwrap();
    let syncs = log.syncs();
    // 25,000 rows are three transactions, and so three syncs of the log.
    assert_eq!(db.import("t", &mut rows(25_000).as_bytes()).unwrap(), 25_000);
    assert_eq!(log.syncs() - syncs, 3);
    assert_eq!(query(&mut db, "SELECT COUNT(*) FROM t"), ["25000"]);

    // A bad row in the third batch takes that batch back, and leaves the two
    // before it.
    db.execute("DELETE FROM t").unwrap();
    let bad = rows(22_000) + "oops,row\n" + &rows(5);
    assert!(db.import("t", &mut bad.as_bytes()).is_err());
    assert_eq!(query(&mut db, "SELECT COUNT(*) FROM t"), ["20000"]);
    // @end
}

#[test]
fn a_crash_during_an_import() {
    // @listing crash-test
    let csv = rows(25_000);
    // Find out how many writes the import makes, and then stop a second run
    // at each of 16 places along the way.
    let writes = {
        let disk = CrashDisk::new();
        let mut db = open(disk.clone(), disk.another_file());
        db.execute("CREATE TABLE t (n INTEGER, word TEXT)").unwrap();
        let before = disk.writes();
        db.import("t", &mut csv.as_bytes()).unwrap();
        disk.writes() - before
    };
    let mut seen = Vec::new();
    for stop in (1..writes).step_by(writes as usize / 16) {
        let disk = CrashDisk::new();
        let log = disk.another_file();
        let mut db = open(disk.clone(), log.clone());
        db.execute("CREATE TABLE t (n INTEGER, word TEXT)").unwrap();
        disk.stop_after(stop);
        assert!(db.import("t", &mut csv.as_bytes()).is_err());
        // The power fails. Whatever comes back must be a whole number of
        // batches, and a sound database.
        let mut db = open(disk.power_cut(stop), log.power_cut(stop));
        db.check().unwrap();
        let count: usize =
            query(&mut db, "SELECT COUNT(*) FROM t")[0].parse().unwrap();
        assert_eq!(count % IMPORT_BATCH, 0, "after {stop} writes");
        seen.push(count);
    }
    seen.dedup();
    assert_eq!(seen, [0, 10_000, 20_000]);
    // @end
}

#[test]
fn kill_it() {
    // @listing kill-test
    let dir = std::env::temp_dir();
    let path = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let (db_path, csv_path) = (path("felite-ch25.db"), path("felite-ch25.csv"));
    for old in [db_path.clone(), format!("{db_path}-log")] {
        let _ = std::fs::remove_file(old);
    }
    std::fs::write(&csv_path, rows(300_000)).unwrap();

    // The real program, as a child process, with its prompt fed from a pipe.
    let felite = |input: String| {
        let program = env!("CARGO_BIN_EXE_felite");
        let mut child = Command::new(program)
            .arg(&db_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        child
    };
    let mut create =
        felite("CREATE TABLE t (n INTEGER, word TEXT);\n.quit\n".to_string());
    assert!(create.wait().unwrap().success());

    // Start the import, wait until the log shows that some of it is committed,
    // and kill the process where it stands.
    let mut import = felite(format!(".import {csv_path} t\n"));
    let log_size =
        || std::fs::metadata(format!("{db_path}-log")).map_or(0, |m| m.len());
    while log_size() < 1_000_000 {
        assert!(import.try_wait().unwrap().is_none(), "the import finished too soon");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    import.kill().unwrap();
    import.wait().unwrap();

    let mut db = Database::open_file(&db_path).unwrap();
    db.check().unwrap();
    let count: usize = query(&mut db, "SELECT COUNT(*) FROM t")[0].parse().unwrap();
    assert!((IMPORT_BATCH..300_000).contains(&count), "{count} rows");
    assert_eq!(count % IMPORT_BATCH, 0, "{count} rows");
    // @end
}

#[test]
fn somebody_elses_tests() {
    // @listing md5-test
    assert_eq!(md5(b""), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(md5(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    // @end

    // @listing slt-test
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    let report = run_slt(&mut db, include_str!("../fixtures/select1.test"));
    // Every statement that builds the table works.
    assert_eq!((report.statements_ok, report.statements_wrong), (31, 0));
    // Of 1,000 queries, felite answers 138, and all 138 answers are right.
    assert_eq!(
        (report.queries_ok, report.queries_wrong, report.queries_refused),
        (138, 0, 862)
    );
    // Why it refused the rest, the most common reasons first.
    let mut reasons: Vec<(usize, &str)> = report
        .refusals
        .iter()
        .map(|(reason, count)| (*count, reason.as_str()))
        .collect();
    reasons.sort_by_key(|(count, reason)| (std::cmp::Reverse(*count), *reason));
    assert_eq!(
        reasons[..6],
        [
            (227, "expected FROM, found the name WHEN"),
            (216, "expected an expression, found SELECT"),
            (206, "there is no function named abs"),
            (115, "expected FROM, found the name a"),
            (52, "expected the end of the statement, found BETWEEN"),
            (24, "there is no function named EXISTS"),
        ]
    );
    // @end
}

#[test]
fn the_questions_on_the_fixture() {
    // @listing questions-test
    let (mut db, _) = fixture();
    db.execute("CREATE INDEX flights_origin ON flights (origin, day)").unwrap();
    db.execute("CREATE INDEX flights_tail ON flights (tail)").unwrap();
    let mut all = |sql: &str| query(&mut db, sql).join(", ");
    assert_eq!(
        all("SELECT carrier, number, dest, delay FROM flights
             WHERE origin = 'ATL' AND day = '2024-01-22'"),
        "9E 4944 DHN -18"
    );
    assert_eq!(
        all("SELECT COUNT(*), COUNT(delay), AVG(delay) FROM flights"),
        "200 188 9.436170212765957"
    );
    assert_eq!(
        all("SELECT carrier, COUNT(*) AS n FROM flights
             GROUP BY carrier ORDER BY n DESC LIMIT 3"),
        "WN 43, AA 28, DL 27"
    );
    assert_eq!(
        all("SELECT c.name, o.city, d.city, f.delay FROM flights f
             JOIN carriers c ON c.code = f.carrier
             JOIN airports o ON o.code = f.origin
             JOIN airports d ON d.code = f.dest
             WHERE f.delay > 250 ORDER BY f.delay DESC"),
        "SkyWest Airlines Inc. Bend/Redmond San Francisco 315, \
         Southwest Airlines Co. Reno Las Vegas 292"
    );
    assert_eq!(
        all("SELECT day, carrier, origin, dest, delay FROM flights
             ORDER BY delay DESC LIMIT 1"),
        "2024-01-02 OO RDM SFO 315"
    );
    // @end
}
