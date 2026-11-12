//! Ten questions asked of the full data set, with each one's plan, its time and
//! the pages it read: `cargo run --release --example queries`. Run the `load`
//! example first.
use felite::{Database, Output};
use std::time::Instant;

// @listing queries
const QUERIES: [&str; 10] = [
    "SELECT * FROM flights WHERE id = 500000",
    "SELECT carrier, number, dest, delay FROM flights
     WHERE origin = 'SLC' AND day = '2024-02-14' ORDER BY delay DESC LIMIT 3",
    "SELECT day, origin, dest FROM flights WHERE tail = 'N131EV'
     ORDER BY day LIMIT 3",
    "SELECT id, day FROM flights ORDER BY id LIMIT 3",
    "SELECT COUNT(*), COUNT(delay), AVG(delay) FROM flights",
    "SELECT carrier, COUNT(*) AS n, AVG(delay) FROM flights
     GROUP BY carrier ORDER BY n DESC LIMIT 3",
    "SELECT day, SUM(cancelled) AS gone FROM flights
     GROUP BY day ORDER BY gone DESC LIMIT 3",
    "SELECT c.name, COUNT(*) AS n FROM flights f
     JOIN carriers c ON c.code = f.carrier GROUP BY c.name ORDER BY n DESC LIMIT 3",
    "SELECT c.name, o.city, d.city, f.delay FROM flights f
     JOIN carriers c ON c.code = f.carrier JOIN airports o ON o.code = f.origin
     JOIN airports d ON d.code = f.dest WHERE f.delay > 1800 ORDER BY f.delay DESC",
    "SELECT day, carrier, origin, dest, delay FROM flights
     ORDER BY delay DESC LIMIT 3",
];

fn main() -> felite::Result<()> {
    let mut db = Database::open_file("data/flights.db")?;
    for (n, sql) in QUERIES.iter().enumerate() {
        println!(
            "\n{}. {}",
            n + 1,
            sql.split_whitespace().collect::<Vec<_>>().join(" ")
        );
        if let Output::Rows(plan) = db.execute(&format!("EXPLAIN {sql}"))? {
            plan.rows.iter().for_each(|line| println!("     {}", line[0]));
        }
        let (start, before) = (Instant::now(), db.pager().stats().reads);
        let Output::Rows(result) = db.execute(sql)? else { continue };
        let (time, pages) = (start.elapsed(), db.pager().stats().reads - before);
        for row in result.rows.iter().take(3) {
            let shown: Vec<String> =
                row.iter().map(|value| value.to_string()).collect();
            println!("  -> {}", shown.join(" | "));
        }
        println!("  {} rows, {pages} pages read, {time:.1?}", result.rows.len());
    }
    Ok(())
}
// @end
