//! The examples in Chapter 24: `cargo test --test book_ch24`.
use felite::{Database, MemFile, Output, Pager, Row, Statement, conjuncts, parse};

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

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// @listing helpers
/// The example data with four indexes on it.
fn indexed() -> (Database, MemFile) {
    let (mut db, file) = fixture();
    for index in [
        "INDEX flights_origin ON flights (origin)",
        "INDEX flights_carrier_day ON flights (carrier, day)",
        "INDEX flights_delay ON flights (delay)",
        "UNIQUE INDEX airports_code ON airports (code)",
    ] {
        db.execute(&format!("CREATE {index}")).unwrap();
    }
    (db, file)
}

/// The plan for a query, a line for each node.
fn plan(db: &mut Database, sql: &str) -> Vec<String> {
    query(db, &format!("EXPLAIN {sql}"))
}

/// How many rows a query returns and how many pages it reads, starting with
/// nothing in memory, with the planner on or off.
fn cost(file: &MemFile, planner: bool, sql: &str) -> (usize, u64) {
    let pager = Pager::with_capacity(Box::new(file.clone()), 20).unwrap();
    let mut db = Database::with_pager(pager).unwrap();
    db.set_planner(planner);
    let before = db.pager().stats().reads;
    let rows = query(&mut db, sql).len();
    (rows, db.pager().stats().reads - before)
}
// @end

// @listing big
/// A file with a larger database in it: 50 airports, and a log of 5,000 flights
/// with an index on the airport and one on the minutes. Airport 0 has half
/// the flights.
fn big() -> MemFile {
    let file = MemFile::new();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    db.execute("CREATE TABLE ports (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .unwrap();
    for id in 0..50 {
        db.execute(&format!("INSERT INTO ports VALUES ({id}, 'Airport {id}')"))
            .unwrap();
    }
    db.execute(
        "CREATE TABLE log (id INTEGER PRIMARY KEY, airport INTEGER, minutes INTEGER,
         note TEXT)",
    )
    .unwrap();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for id in 1..=5000 {
        let n = next(&mut state);
        let airport = if n.is_multiple_of(2) { 0 } else { n % 49 + 1 };
        let (minutes, note) = (next(&mut state) % 1000, "on time ".repeat(8));
        let values = format!("({id}, {airport}, {minutes}, '{note}')");
        db.execute(&format!("INSERT INTO log VALUES {values}")).unwrap();
    }
    db.execute("CREATE INDEX log_airport ON log (airport)").unwrap();
    db.execute("CREATE INDEX log_minutes ON log (minutes)").unwrap();
    file
}
// @end

#[test]
fn the_same_query_two_ways() {
    // @listing two-ways
    let file = big();
    let sql = "SELECT l.id, p.name FROM log l JOIN ports p ON l.airport = p.id
               WHERE l.airport = 7 AND l.minutes > 900";
    // As Chapters 14 to 22 run it: join every flight in the log to its
    // airport, and then throw away all but five of the joined rows.
    assert_eq!(cost(&file, false, sql), (5, 107));
    // As this chapter will run it.
    assert_eq!(cost(&file, true, sql), (5, 46));
    // @end
}

#[test]
fn splitting_at_and() {
    // @listing conjuncts-test
    let Statement::Select(select) = parse(
        "SELECT id FROM flights
         WHERE origin = 'ATL' AND (delay > 30 OR cancelled = 1) AND miles < 500.0",
    )
    .unwrap() else {
        panic!()
    };
    let mut parts = Vec::new();
    conjuncts(&select.filter.unwrap(), &mut parts);
    let parts: Vec<String> = parts.iter().map(|part| part.to_string()).collect();
    // Three conditions, all of which must hold. The OR is one of them, whole.
    assert_eq!(
        parts,
        ["(origin = 'ATL')", "((delay > 30) OR (cancelled = 1))", "(miles < 500.0)"]
    );
    // @end
}

#[test]
fn estimating_rows() {
    // @listing estimate-test
    let (mut db, _) = fixture();
    db.execute("CREATE TABLE empty (n INTEGER)").unwrap();
    let mut estimate = |name: &str| {
        let tree = db.table(name).unwrap().tree;
        (tree.estimate(db.pager()).unwrap(), tree.check(db.pager()).unwrap().entries)
    };
    // The guess, and the truth.
    assert_eq!(estimate("carriers"), (15, 15));
    assert_eq!(estimate("flights"), (240, 200));
    assert_eq!(estimate("airports"), (387, 334));
    assert_eq!(estimate("empty"), (0, 0));
    // @end
}

#[test]
fn access_paths() {
    // @listing access-test
    let (mut db, _) = indexed();
    let mut first = |sql: &str| plan(&mut db, sql).last().unwrap().trim().to_string();
    // No condition, or none that anything can be done with: read it all.
    assert_eq!(first("SELECT * FROM flights"), "Scan flights, about 240 rows");
    assert_eq!(
        first("SELECT id FROM flights WHERE miles > 2000.0"),
        "Scan flights, about 240 rows"
    );
    // The row id is the best there is.
    assert_eq!(
        first("SELECT * FROM flights WHERE id = 2736 AND origin = 'ATL'"),
        "Look up row 2736 of flights"
    );
    // One column of an index, both columns, and a column with a range.
    assert_eq!(
        first("SELECT id FROM flights WHERE origin = 'ATL'"),
        "Search flights with flights_origin for (origin = 'ATL'), about 12 rows"
    );
    assert_eq!(
        first("SELECT id FROM flights WHERE day = '2024-01-12' AND 'HA' = carrier"),
        "Search flights with flights_carrier_day for \
         ('HA' = carrier) AND (day = '2024-01-12'), about 1 rows"
    );
    assert_eq!(
        first("SELECT id FROM flights WHERE carrier = 'WN' AND day >= '2024-01-29'"),
        "Search flights with flights_carrier_day for \
         (carrier = 'WN') AND (day >= '2024-01-29'), about 4 rows"
    );
    // A unique index promises one row.
    assert_eq!(
        first("SELECT city FROM airports WHERE code = 'DHN'"),
        "Search airports with airports_code for (code = 'DHN'), about 1 rows"
    );
    // @end
}

#[test]
fn what_is_left_over() {
    // @listing leftover-test
    let (mut db, _) = indexed();
    // The index takes one condition, and the other two are filters above it.
    assert_eq!(
        plan(
            &mut db,
            "SELECT id FROM flights
             WHERE delay > 30 AND origin = 'ATL' AND dest <> 'LGA'"
        ),
        [
            "Project id",
            "  Filter (dest <> 'LGA')",
            "    Filter (delay > 30)",
            "      Search flights with flights_origin for (origin = 'ATL'), \
             about 12 rows",
        ]
    );
    // An index on (carrier, day) is no use for the day alone.
    assert_eq!(
        plan(&mut db, "SELECT id FROM flights WHERE day = '2024-01-12'"),
        [
            "Project id",
            "  Filter (day = '2024-01-12')",
            "    Scan flights, about 240 rows"
        ]
    );
    // A constant has to be of the column's type before an index can use it. An
    // integer becomes a real. A real can't become an integer.
    db.execute("CREATE INDEX flights_miles ON flights (miles)").unwrap();
    assert_eq!(query(&mut db, "SELECT id FROM flights WHERE miles = 170"), ["2736"]);
    let by_real = plan(&mut db, "SELECT id FROM flights WHERE delay = 2.5");
    assert_eq!(by_real[2], "    Scan flights, about 240 rows");
    // @end
}

#[test]
fn every_range() {
    // @listing range-model
    // A table with an index on (a, b), and NULLs in both.
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, a INTEGER, b INTEGER)")
        .unwrap();
    db.execute("CREATE INDEX t_ab ON t (a, b)").unwrap();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    let mut value = |range: u64| {
        let n = next(&mut state);
        if n.is_multiple_of(9) { "NULL".to_string() } else { (n % range).to_string() }
    };
    for id in 1..=400 {
        let (a, b) = (value(6), value(12));
        db.execute(&format!("INSERT INTO t VALUES ({id}, {a}, {b})")).unwrap();
    }
    // @end
    // @listing range-model-pairs
    // Every pair of comparisons, against several constants. The planner must
    // return what a scan and a filter return, whatever range it works out.
    let ops = ["=", "<", "<=", ">", ">="];
    let (mut tried, mut searched) = (0, 0);
    for first in ops {
        for second in ops {
            for (x, y) in [(0, 0), (2, 5), (3, 11), (5, 6), (9, 3)] {
                let tests = format!("a {first} {x} AND b {second} {y}");
                let sql = format!("SELECT id FROM t WHERE {tests} ORDER BY id");
                db.set_planner(true);
                let planned = query(&mut db, &sql);
                searched += plan(&mut db, &sql)
                    .iter()
                    .filter(|l| l.contains("Search"))
                    .count();
                db.set_planner(false);
                assert_eq!(planned, query(&mut db, &sql), "{sql}");
                tried += 1;
            }
        }
    }
    // @end
    // @listing range-model-two
    // Two bounds on one column, with and without the first column fixed.
    for (low, high) in [(">", "<"), (">=", "<="), (">", "<="), (">=", "<")] {
        for (x, y) in [(0, 5), (1, 3), (2, 2), (4, 1), (3, 30)] {
            for sql in [
                format!(
                    "SELECT id FROM t WHERE a {low} {x} AND a {high} {y} ORDER BY id"
                ),
                format!(
                    "SELECT id FROM t WHERE a = 2 AND b {low} {x} AND b {high} {y}"
                ),
            ] {
                db.set_planner(true);
                let mut planned = query(&mut db, &sql);
                searched += plan(&mut db, &sql)
                    .iter()
                    .filter(|l| l.contains("Search"))
                    .count();
                db.set_planner(false);
                let mut scanned = query(&mut db, &sql);
                planned.sort();
                scanned.sort();
                assert_eq!(planned, scanned, "{sql}");
                tried += 1;
            }
        }
    }
    assert_eq!((tried, searched), (165, 65));
    // @end
}

#[test]
fn filters_sink() {
    // @listing sink-test
    let (mut db, _) = fixture();
    // Three conditions in one WHERE. Each goes to the lowest place where its
    // columns exist: two to their tables, and one to the join.
    assert_eq!(
        plan(
            &mut db,
            "SELECT f.id FROM flights f JOIN airports a ON a.code = f.origin
             WHERE a.state = 'HI' AND f.delay > 0 AND f.dest <> a.code"
        ),
        [
            "Project id",
            "  Put the columns in the order written",
            "    Filter (f.dest <> a.code)",
            "      Hash join on (a.code = f.origin)",
            "        Filter (a.state = 'HI')",
            "          Scan a, about 387 rows",
            "        Filter (f.delay > 0)",
            "          Scan f, about 240 rows",
        ]
    );
    // @end
}

#[test]
fn join_order() {
    // @listing order-test
    let (mut db, _) = fixture();
    // Written smallest table first. The planner reads the largest as a stream
    // and hashes the others, whatever order they are written in.
    let three = plan(
        &mut db,
        "SELECT c.name, a.city, f.delay FROM carriers c
         JOIN flights f ON f.carrier = c.code
         JOIN airports a ON a.code = f.origin WHERE f.delay > 100",
    );
    assert_eq!(
        three,
        [
            "Project name, city, delay",
            "  Put the columns in the order written",
            "    Hash join on (f.carrier = c.code)",
            "      Hash join on (a.code = f.origin)",
            "        Scan a, about 387 rows",
            "        Filter (f.delay > 100)",
            "          Scan f, about 240 rows",
            "      Scan c, about 15 rows",
        ]
    );
    // @end
}

#[test]
fn sorts_you_do_not_need() {
    // @listing no-sort-test
    let (mut db, file) = indexed();
    let mut sorts =
        |sql: &str| plan(&mut db, sql).iter().any(|line| line.contains("Sort"));
    // A scan is in order of id, so nothing has to be done.
    assert!(!sorts("SELECT id FROM flights ORDER BY id"));
    assert!(sorts("SELECT id FROM flights ORDER BY id DESC"));
    assert!(sorts("SELECT id FROM flights ORDER BY delay"));
    // An index with its first column fixed is in order of its second.
    assert!(!sorts("SELECT id FROM flights WHERE carrier = 'HA' ORDER BY day"));
    assert!(!sorts("SELECT id FROM flights WHERE carrier = 'HA' ORDER BY day, id"));
    assert!(sorts("SELECT id FROM flights WHERE carrier = 'HA' ORDER BY delay"));
    // Groups come out in order of their keys.
    assert!(!sorts(
        "SELECT carrier, COUNT(*) FROM flights GROUP BY carrier ORDER BY carrier"
    ));
    assert!(sorts(
        "SELECT carrier, COUNT(*) FROM flights GROUP BY carrier ORDER BY COUNT(*)"
    ));
    // One row is in any order you like.
    assert!(!sorts("SELECT id FROM flights WHERE id = 2736 ORDER BY delay DESC"));
    assert!(!sorts("SELECT COUNT(*) FROM flights ORDER BY COUNT(*)"));

    // With no sort in the way, a LIMIT stops the scan after one leaf, as
    // Section 20.10 said it would.
    let first_three = "SELECT id FROM flights ORDER BY id LIMIT 3";
    assert_eq!(cost(&file, false, first_three), (3, 5));
    assert_eq!(cost(&file, true, first_three), (3, 2));
    // @end
}

#[test]
fn having_moves_down() {
    // @listing having-test
    let (mut db, _) = fixture();
    let sql = "SELECT carrier, COUNT(*) FROM flights GROUP BY carrier
               HAVING COUNT(*) > 20 AND carrier <> 'WN'";
    assert_eq!(
        plan(&mut db, sql),
        [
            "Project carrier, COUNT(*)",
            "  Filter (COUNT(*) > 20)",
            "    Aggregate by carrier: COUNT(*)",
            "      Filter (carrier <> 'WN')",
            "        Scan flights, about 240 rows",
        ]
    );
    assert_eq!(query(&mut db, sql), ["AA 28", "DL 27", "OO 21", "UA 22"]);
    // @end
}

#[test]
fn where_the_planner_does_nothing() {
    // @listing left-test
    let (mut db, _) = indexed();
    assert_eq!(
        plan(
            &mut db,
            "SELECT a.code, f.id FROM airports a
             LEFT JOIN flights f ON f.origin = a.code WHERE a.state = 'HI'"
        ),
        [
            "Project code, id",
            "  Filter (a.state = 'HI')",
            "    Left hash join on (f.origin = a.code)",
            "      Scan a, about 387 rows",
            "      Scan f, about 240 rows",
        ]
    );
    // @end
}

#[test]
fn before_and_after() {
    // @listing cost-test
    let file = big();
    let both = |sql: &str| (cost(&file, false, sql).1, cost(&file, true, sql).1);
    // Pages read without the planner, and with it.
    assert_eq!(both("SELECT * FROM log WHERE id = 2500"), (106, 3));
    assert_eq!(both("SELECT id FROM log WHERE airport = 7"), (106, 45));
    assert_eq!(both("SELECT id FROM log ORDER BY id LIMIT 3"), (106, 2));
    assert_eq!(
        both("SELECT id FROM log WHERE minutes >= 100 AND minutes < 110"),
        (106, 57)
    );
    assert_eq!(
        both(
            "SELECT p.name FROM log l JOIN ports p ON p.id = l.airport
             WHERE l.id = 77"
        ),
        (107, 4)
    );
    // Nothing to be gained: every row is needed.
    assert_eq!(
        both("SELECT airport, COUNT(*) FROM log GROUP BY airport"),
        (106, 107)
    );
    // @end
}

#[test]
fn where_the_planner_is_wrong() {
    // @listing wrong-test
    let file = big();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    // Thirty-eight flights in 5,000 took more than 990 minutes. The planner
    // guesses that a bound selects three rows in ten, and so it scans.
    let long = "SELECT id FROM log WHERE minutes > 990";
    assert_eq!(plan(&mut db, long)[2], "    Scan log, about 5040 rows");
    assert_eq!(cost(&file, true, long), (38, 107));
    // Half the flights are from airport 0. The planner guesses that an equality
    // selects one row in twenty, and so it uses the index.
    let half = "SELECT id FROM log WHERE airport = 0";
    assert_eq!(
        plan(&mut db, half)[1],
        "  Search log with log_airport for (airport = 0), about 252 rows"
    );
    assert_eq!(cost(&file, true, half), (2464, 134));
    assert_eq!(cost(&file, false, half), (2464, 106));
    // Worse, a range of airports that is also half the flights.
    let range = "SELECT id FROM log WHERE airport >= 1 AND airport <= 49";
    assert_eq!(cost(&file, true, range), (2536, 2016));
    assert_eq!(cost(&file, false, range), (2536, 106));
    // And an inference it does not make. The join says that l.airport equals
    // p.id, and the WHERE says that p.id is 7, so l.airport must be 7, and
    // the index on it would serve. The planner sees no condition on the log.
    let indirect = "SELECT l.id FROM log l JOIN ports p ON l.airport = p.id
                    WHERE p.id = 7 AND l.minutes > 900";
    assert_eq!(cost(&file, true, indirect), (5, 108));
    // @end
}

#[test]
fn planned_against_unplanned() {
    // @listing model
    let (mut db, _) = indexed();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    let mut pick = |from: &[&str]| {
        from[(next(&mut state) % from.len() as u64) as usize].to_string()
    };
    let tables = [
        "flights f JOIN airports a ON a.code = f.origin",
        "airports a JOIN flights f ON f.dest = a.code",
        "flights f JOIN carriers c ON c.code = f.carrier
         JOIN airports a ON a.code = f.dest",
        "carriers c JOIN flights f ON f.carrier = c.code AND f.delay > 0
         JOIN airports a ON f.origin = a.code",
        "flights f JOIN airports a ON a.state = 'HI' AND f.miles > 2000.0",
        "airports a LEFT JOIN flights f ON f.origin = a.code",
    ];
    let conditions = [
        "f.origin = 'ATL'",
        "f.carrier = 'WN'",
        "f.delay > 60",
        "f.delay < 0",
        "f.delay IS NULL",
        "a.state = 'TX'",
        "a.code > 'S'",
        "f.id < 100000",
        "f.day >= '2024-01-20'",
        "f.carrier = 'DL' OR a.state = 'GA'",
        "f.miles > 1000.0",
        "a.city <> f.dest",
    ];
    for _ in 0..100 {
        let from = pick(&tables);
        let sql = format!(
            "SELECT f.id, a.code, f.delay FROM {from} WHERE {} AND {}
             ORDER BY f.id, a.code",
            pick(&conditions),
            pick(&conditions)
        );
        db.set_planner(true);
        let planned = query(&mut db, &sql);
        db.set_planner(false);
        assert_eq!(planned, query(&mut db, &sql), "{sql}");
    }
    // @end
}
