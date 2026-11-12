//! Tests for the Chapter 24 challenges: `cargo test --test challenges_ch24`.
use felite::{Database, MemFile, Output, Pager, Row};

fn fixture() -> Database {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    for statement in include_str!("../fixtures/flights.sql").lines() {
        db.execute(statement).unwrap();
    }
    for index in [
        "INDEX flights_origin ON flights (origin)",
        "INDEX flights_carrier_day ON flights (carrier, day)",
        "INDEX flights_delay ON flights (delay)",
        "UNIQUE INDEX airports_code ON airports (code)",
    ] {
        db.execute(&format!("CREATE {index}")).unwrap();
    }
    db
}

fn plan(db: &mut Database, sql: &str) -> Vec<String> {
    let Output::Rows(result) = db.execute(&format!("EXPLAIN {sql}")).unwrap() else {
        panic!("not a query")
    };
    result.rows.iter().map(|row: &Row| row[0].to_string()).collect()
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

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

fn cost(file: &MemFile, planner: bool, sql: &str) -> (usize, u64) {
    let pager = Pager::with_capacity(Box::new(file.clone()), 20).unwrap();
    let mut db = Database::with_pager(pager).unwrap();
    db.set_planner(planner);
    let before = db.pager().stats().reads;
    let Output::Rows(result) = db.execute(sql).unwrap() else { panic!() };
    (result.rows.len(), db.pager().stats().reads - before)
}

// @challenge 2
use felite::selectivity;

// @challenge 3
use felite::distinct_counts;

// @challenge 4
use felite::join_orders;

// @challenge 1
#[test]
fn predict_plans() {
    let mut db = fixture();
    assert_eq!(
        plan(
            &mut db,
            "SELECT id FROM flights WHERE carrier = 'DL' AND origin = 'ATL'"
        ),
        [
            "Project id",
            "  Filter (carrier = 'DL')",
            "    Search flights with flights_origin for (origin = 'ATL'), \
             about 12 rows",
        ]
    );
    assert_eq!(
        plan(&mut db, "SELECT id FROM flights WHERE delay + 0 = 15"),
        [
            "Project id",
            "  Filter ((delay + 0) = 15)",
            "    Scan flights, about 240 rows"
        ]
    );
    assert_eq!(
        plan(
            &mut db,
            "SELECT id FROM flights WHERE origin = 'ATL' OR origin = 'SLC'"
        ),
        [
            "Project id",
            "  Filter ((origin = 'ATL') OR (origin = 'SLC'))",
            "    Scan flights, about 240 rows",
        ]
    );
    assert_eq!(
        plan(
            &mut db,
            "SELECT day FROM flights WHERE carrier = 'AS' AND day > '2024-01-15'
             ORDER BY day LIMIT 2"
        ),
        [
            "Stop after 2 rows",
            "  Project day",
            "    Search flights with flights_carrier_day for \
             (carrier = 'AS') AND (day > '2024-01-15'), about 4 rows",
        ]
    );
}

// @challenge 2
#[test]
fn guesses_against_the_truth() {
    let mut db = fixture();
    let mut actual = |condition: &str| {
        (selectivity(&mut db, "flights", condition).unwrap() * 1000.0).round()
            / 1000.0
    };
    // The planner guesses 0.05 for each of these.
    assert_eq!(actual("origin = 'ATL'"), 0.045);
    assert_eq!(actual("carrier = 'WN'"), 0.215);
    assert_eq!(actual("cancelled = 0"), 0.945);
    assert_eq!(actual("tail = 'N131EV'"), 0.005);
    // And 0.3 for each of these.
    assert_eq!(actual("delay > 200"), 0.015);
    assert_eq!(actual("delay > -30"), 0.895);
    assert_eq!(actual("delay > NULL"), 0.0);
}

// @challenge 3
#[test]
fn counting_distinct_values_per_column() {
    let mut db = fixture();
    let counts = distinct_counts(&mut db, "flights").unwrap();
    let of = |name: &str| counts.iter().find(|(column, _)| column == name).unwrap().1;
    assert_eq!(counts.len(), 13);
    assert_eq!((of("id"), of("carrier"), of("origin")), (200, 15, 83));
    assert_eq!((of("cancelled"), of("tail"), of("delay")), (2, 190, 86));
    // One row in 83 is a better guess for origin = ? than one in 20.
    assert_eq!((1000.0_f64 / of("origin") as f64).round(), 12.0);
}

// @challenge 4
#[test]
fn how_many_orders() {
    assert_eq!(join_orders(1), Some(1));
    assert_eq!(join_orders(3), Some(6));
    assert_eq!(join_orders(10), Some(3_628_800));
    assert_eq!(join_orders(20), Some(2_432_902_008_176_640_000));
    assert_eq!(join_orders(21), None);
}

// @challenge 5
#[test]
fn fooling_the_planner() {
    let file = big();
    let hinted = "SELECT l.id FROM log l JOIN ports p ON l.airport = p.id
                  WHERE p.id = 7 AND l.airport = 7 AND l.minutes > 900";
    assert_eq!(cost(&file, true, hinted), (5, 46));
    let wide = "SELECT id FROM log WHERE minutes >= 100 AND minutes < 600";
    let (planned, unplanned) = (cost(&file, true, wide), cost(&file, false, wide));
    assert_eq!(planned.0, unplanned.0);
    assert!(planned.1 >= 2 * unplanned.1, "{planned:?} {unplanned:?}");
}
