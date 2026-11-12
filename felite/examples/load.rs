//! Loads the example data set into a database file, and says how long it took:
//! `cargo run --release --example load`. Run `cargo xtask data` first.
use felite::Database;
use std::fs::File;
use std::io::BufReader;
use std::time::Instant;

// @listing load
const SCHEMA: [&str; 3] = [
    "CREATE TABLE carriers (code TEXT NOT NULL, name TEXT NOT NULL)",
    "CREATE TABLE airports (code TEXT NOT NULL, city TEXT NOT NULL,
     state TEXT NOT NULL)",
    "CREATE TABLE flights (id INTEGER PRIMARY KEY, day TEXT NOT NULL,
     carrier TEXT NOT NULL, number INTEGER NOT NULL, tail TEXT, origin TEXT NOT NULL,
     dest TEXT NOT NULL, departs INTEGER NOT NULL, dep_delay INTEGER, delay INTEGER,
     cancelled INTEGER NOT NULL, air_time INTEGER, miles REAL NOT NULL)",
];
const INDEXES: [&str; 4] = [
    "CREATE UNIQUE INDEX carriers_code ON carriers (code)",
    "CREATE UNIQUE INDEX airports_code ON airports (code)",
    "CREATE INDEX flights_origin ON flights (origin, day)",
    "CREATE INDEX flights_tail ON flights (tail)",
];

fn main() -> felite::Result<()> {
    let path = "data/flights.db";
    for old in [path.to_string(), format!("{path}-log")] {
        let _ = std::fs::remove_file(old);
    }
    let mut db = Database::open_file(path)?;
    for statement in SCHEMA {
        db.execute(statement)?;
    }
    let size =
        |path: &str| std::fs::metadata(path).map_or(0, |m| m.len()) as f64 / 1e6;
    for table in ["carriers", "airports", "flights"] {
        let start = Instant::now();
        let file = File::open(format!("data/{table}.csv"))?;
        let rows = db.import(table, &mut BufReader::new(file))?;
        let seconds = start.elapsed().as_secs_f64();
        println!(
            "{table}: {rows} rows in {seconds:.2} s, {:.0} rows a second",
            rows as f64 / seconds
        );
    }
    db.pager().checkpoint()?;
    println!("the file is {:.1} MB", size(path));
    for statement in INDEXES {
        let start = Instant::now();
        db.execute(statement)?;
        db.pager().checkpoint()?;
        let seconds = start.elapsed().as_secs_f64();
        println!("{statement}: {seconds:.2} s, and the file is {:.1} MB", size(path));
    }
    let start = Instant::now();
    db.check()?;
    println!("check: {:.2} s", start.elapsed().as_secs_f64());
    Ok(())
}
// @end
