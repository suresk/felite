//! The examples in Chapter 20: `cargo test --test book_ch20`.
use felite::{Database, Limit, MemFile, Operator, Output, Row, Scan, Scope, Value};
use felite::{SORT_BUDGET, Sort, SortKey, SortStats, Statement, TempSpace};
use felite::{bind, order, parse, parse_expr};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

// @listing fixture
/// The example data, loaded into a database in memory, and the database's file.
fn fixture() -> (Database, MemFile) {
    let file = MemFile::new();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    for statement in include_str!("../fixtures/flights.sql").lines() {
        db.execute(statement).unwrap();
    }
    (db, file)
}
// @end

/// Runs a query and returns its rows as text, one line for each row.
fn query(db: &mut Database, sql: &str) -> Vec<String> {
    let Output::Rows(result) = db.execute(sql).unwrap() else {
        panic!("not a query")
    };
    let line =
        |row: &Row| row.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
    result.rows.iter().map(line).collect()
}

/// The message of the error that a statement produces.
fn error(db: &mut Database, sql: &str) -> String {
    db.execute(sql).unwrap_err().to_string()
}

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn the_example_data() {
    // @listing fixture-test
    let (mut db, file) = fixture();
    let count = |db: &mut Database, sql: &str| query(db, sql).len();
    assert_eq!(count(&mut db, "SELECT code FROM carriers"), 15);
    assert_eq!(count(&mut db, "SELECT code FROM airports"), 334);
    assert_eq!(count(&mut db, "SELECT id FROM flights"), 200);
    assert_eq!(file.bytes().len(), 12 * 4096);

    // The first flight of the 200, and the airport it left from.
    assert_eq!(
        query(&mut db, "SELECT * FROM flights WHERE id = 2736"),
        ["2736 2024-01-22 9E 4944 N131EV ATL DHN 1539 -9 -18 0 36 170.0"]
    );
    assert_eq!(
        query(&mut db, "SELECT * FROM airports WHERE code = 'DHN'"),
        ["DHN Dothan AL"]
    );
    assert_eq!(
        query(&mut db, "SELECT name FROM carriers WHERE code = '9E'"),
        ["Endeavor Air Inc."]
    );

    // Eleven flights were cancelled. Those have no delays and no time in the
    // air. One more took off and was diverted, so it has no arrival delay.
    assert_eq!(count(&mut db, "SELECT id FROM flights WHERE cancelled = 1"), 11);
    assert_eq!(count(&mut db, "SELECT id FROM flights WHERE dep_delay IS NULL"), 11);
    assert_eq!(count(&mut db, "SELECT id FROM flights WHERE delay IS NULL"), 12);
    assert_eq!(count(&mut db, "SELECT id FROM flights WHERE tail IS NULL"), 4);
    // @end
}

#[test]
fn parsing_order_by() {
    // @listing parse-test
    let sql = "select id from flights where delay > 0 \
               order by carrier, delay desc, miles asc limit 5 offset 10";
    let Statement::Select(select) = parse(sql).unwrap() else { panic!() };
    let keys: Vec<(String, bool)> =
        select.order_by.iter().map(|k| (k.expr.to_string(), k.descending)).collect();
    assert_eq!(
        keys,
        [
            ("carrier".to_string(), false),
            ("delay".to_string(), true),
            ("miles".to_string(), false)
        ]
    );
    assert_eq!((select.limit, select.offset), (Some(5), Some(10)));
    assert_eq!(
        parse(sql).unwrap().to_string(),
        "SELECT id FROM flights WHERE (delay > 0) \
         ORDER BY carrier, delay DESC, miles LIMIT 5 OFFSET 10"
    );

    // @end
    // @listing parse-errors
    // Each of these is a syntax error at the place the text went wrong.
    let message = |sql: &str| {
        let message = parse(sql).unwrap_err().to_string();
        message.split_once(": ").unwrap().1.to_string()
    };
    assert_eq!(
        message("SELECT id FROM flights ORDER delay"),
        "expected `BY`, found the name `delay`"
    );
    assert_eq!(
        message("SELECT id FROM flights LIMIT -1"),
        "expected a count, found `-`"
    );
    assert_eq!(
        message("SELECT id FROM flights LIMIT 5 ORDER BY id"),
        "expected the end of the statement, found `ORDER`"
    );
    // @end
}

#[test]
fn an_order_for_values() {
    // @listing order-test
    let (null, text) = (Value::Null, |s: &str| Value::Text(s.to_string()));
    assert_eq!(order(&Value::Integer(-7), &Value::Integer(12)), Ordering::Less);
    assert_eq!(order(&Value::Integer(2), &Value::Real(1.5)), Ordering::Greater);
    assert_eq!(order(&text("ATL"), &text("ATL")), Ordering::Equal);
    // Text is compared byte by byte, so every capital comes before every small
    // letter, and a word comes after any word it starts with.
    assert_eq!(order(&text("Zurich"), &text("atlanta")), Ordering::Less);
    assert_eq!(order(&text("ATL"), &text("ATLANTA")), Ordering::Less);
    // NULL is before everything, and level with itself.
    assert_eq!(order(&null, &Value::Integer(i64::MIN)), Ordering::Less);
    assert_eq!(order(&text(""), &null), Ordering::Greater);
    assert_eq!(order(&null, &null), Ordering::Equal);

    let mut delays =
        [Value::Integer(12), null.clone(), Value::Integer(-7), Value::Integer(0)];
    delays.sort_by(order);
    let shown: Vec<String> = delays.iter().map(|v| v.to_string()).collect();
    assert_eq!(shown, ["NULL", "-7", "0", "12"]);
    // @end
}

#[test]
fn order_by() {
    // @listing order-by-test
    let (mut db, _) = fixture();
    // Each query's rows, on one line.
    let mut row = |sql: &str| query(&mut db, sql).join(", ");
    // Hawaiian's three flights, least delayed first.
    assert_eq!(
        row("SELECT id, delay FROM flights WHERE carrier = 'HA' ORDER BY delay"),
        "229824 17, 232560 30, 235296 56"
    );
    // Alaska's six. Three were cancelled, and NULL sorts first.
    assert_eq!(
        row("SELECT delay FROM flights WHERE carrier = 'AS' ORDER BY delay"),
        "NULL, NULL, NULL, -31, -27, -4"
    );
    // DESC turns the whole order around, NULLs included.
    assert_eq!(
        row("SELECT delay FROM flights WHERE carrier = 'AS' ORDER BY delay DESC"),
        "-4, -27, -31, NULL, NULL, NULL"
    );
    // @end
}

#[test]
fn several_keys_and_ties() {
    // @listing keys-test
    let (mut db, _) = fixture();
    let mut row = |sql: &str| query(&mut db, sql).join(", ");
    // JetBlue's flights by origin, and the longest first within each origin.
    assert_eq!(
        row("SELECT origin, miles FROM flights WHERE carrier = 'B6'
             ORDER BY origin, miles DESC"),
        "ATL 762.0, BOS 496.0, BOS 399.0, FLL 793.0, \
         JFK 2422.0, JFK 1598.0, LAX 2342.0, LGA 1076.0"
    );
    // @end
    // @listing ties-test
    // Nine flights on one day, by carrier. The two Southwest flights have equal
    // keys, and come out in the order of their ids.
    assert_eq!(
        row("SELECT carrier, id FROM flights WHERE day = '2024-01-12'
             ORDER BY carrier"),
        "AA 54720, AS 101232, G4 221616, HA 232560, MQ 251712, \
         UA 514368, WN 317376, WN 342000, YX 391248"
    );
    // @end
    // @listing alias-test
    // A key can be an expression, or the name the SELECT list gave to one.
    let fast = "SELECT carrier, miles / air_time * 60.0 AS mph FROM flights
                WHERE miles > 2400.0 AND air_time > 350";
    let expected =
        ["AS 459.1011235955056", "HA 446.6850828729282", "NK 400.10869565217394"];
    assert_eq!(query(&mut db, &format!("{fast} ORDER BY mph DESC")), expected);
    let spelled_out = format!("{fast} ORDER BY miles / air_time * 60.0 DESC");
    assert_eq!(query(&mut db, &spelled_out), expected);

    // The keys are checked like any other expression.
    assert_eq!(
        error(&mut db, "SELECT id FROM flights ORDER BY delay, arrival"),
        "error at line 1, column 40: there is no column named arrival"
    );
    assert_eq!(
        error(&mut db, "SELECT id FROM flights ORDER BY carrier + 1"),
        "error at line 1, column 33: + needs a number, and this is TEXT"
    );
    // @end
}

// @listing sort-flights
/// Sorts the flights by one expression, by hand, and returns what the sort did
/// and the ids in the order they came out.
fn sort_flights(
    db: &mut Database,
    key: &str,
    budget: usize,
) -> (SortStats, Vec<i64>) {
    let table = db.table("flights").unwrap().clone();
    let expr = bind(&parse_expr(key).unwrap(), &Scope::of(&table)).unwrap();
    let scan = Scan::new(&table, db.pager()).unwrap();
    let keys = vec![SortKey { expr, descending: false }];
    let mut sort = Sort::new(Box::new(scan), keys, budget, TempSpace::Memory);
    let mut ids = Vec::new();
    while let Some(row) = sort.next(db.pager()).unwrap() {
        let Value::Integer(id) = row[0] else { panic!() };
        ids.push(id);
    }
    (sort.stats(), ids)
}
// @end

#[test]
fn sorting_in_memory() {
    // @listing memory-test
    let (mut db, _) = fixture();
    let (stats, ids) = sort_flights(&mut db, "delay", SORT_BUDGET);
    // The 200 flights are 12,290 bytes as records with prefixes, an average of
    // 61 bytes each. That fits in 4 MiB with room to spare, so nothing was
    // written anywhere.
    assert_eq!(stats, SortStats { rows: 200, runs: 0, peak: 12_290, spilled: 0 });
    // Twelve with no delay come first, in the order of their ids. After them is
    // the earliest arrival, which was 46 minutes ahead of schedule.
    assert_eq!(ids.len(), 200);
    assert_eq!(ids[..3], [54_720, 98_496, 101_232]);
    assert_eq!(ids[11..13], [533_520, 459_648]);
    assert_eq!(
        query(&mut db, "SELECT delay FROM flights WHERE id = 459648"),
        ["-46"]
    );
    // The 200 flights have 87 different delays between them, counting NULL as
    // one, so most of them tie with some other.
    let mut delays = query(&mut db, "SELECT delay FROM flights");
    delays.sort();
    delays.dedup();
    assert_eq!(delays.len(), 87);
    // @end
}

#[test]
fn sorting_in_runs() {
    // @listing runs-test
    let (mut db, _) = fixture();
    let (_, in_memory) = sort_flights(&mut db, "delay", SORT_BUDGET);

    // A budget of a tenth of the data.
    let (stats, ids) = sort_flights(&mut db, "delay", 1_229);
    assert_eq!(
        stats,
        SortStats { rows: 200, runs: 11, peak: 1_219, spilled: 12_290 }
    );
    // Every row went to the temporary file and came back, in the same order
    // as before.
    assert_eq!(ids, in_memory);

    // Smaller budgets make more runs and never a different answer. With too
    // little room for even one row, every row is a run by itself.
    for (budget, runs, peak) in
        [(4096, 4, 4087), (512, 25, 499), (100, 200, 64), (0, 200, 64)]
    {
        let (stats, ids) = sort_flights(&mut db, "delay", budget);
        assert_eq!((stats.runs, stats.peak), (runs, peak));
        assert_eq!(ids, in_memory);
    }
    // @end
}

#[test]
fn a_heap_of_run_heads() {
    // @listing heap-test
    // Three sorted runs. The heap holds the next number of each, with the
    // number of the run it came from.
    let runs = [vec![3, 8, 9], vec![1, 8], vec![2, 4, 7]];
    let mut at = [0; 3];
    let mut heap = BinaryHeap::new();
    for (run, numbers) in runs.iter().enumerate() {
        heap.push(Reverse((numbers[0], run)));
    }
    let mut merged = Vec::new();
    while let Some(Reverse((number, run))) = heap.pop() {
        merged.push((number, run));
        at[run] += 1;
        if let Some(next) = runs[run].get(at[run]) {
            heap.push(Reverse((*next, run)));
        }
    }
    // The 8 from run 0 comes out before the 8 from run 1.
    assert_eq!(
        merged,
        [(1, 1), (2, 2), (3, 0), (4, 2), (7, 2), (8, 0), (8, 1), (9, 0)]
    );
    // @end
}

#[test]
fn the_budget_does_not_change_the_answer() {
    // @listing budget-test
    let (mut db, _) = fixture();
    let sql = "SELECT id, day, carrier, delay FROM flights
               ORDER BY carrier DESC, delay, day DESC";
    let expected = query(&mut db, sql);
    assert_eq!(expected.len(), 200);
    assert_eq!(expected[0], "396720 2024-01-08 YX -25");
    for budget in [8192, 1000, 300, 1] {
        db.set_sort_budget(budget);
        assert_eq!(query(&mut db, sql), expected);
    }
    // @end
}

#[test]
fn temporary_files() {
    // @listing temp-test
    let dir = std::env::temp_dir();
    let path = dir.join("felite-ch20.db").to_str().unwrap().to_string();
    // How many temporary files there are beside the database.
    let beside = || {
        let names = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name());
        names
            .filter(|n| n.to_string_lossy().starts_with("felite-ch20.db-temp"))
            .count()
    };
    let before = beside();

    let space = TempSpace::Beside(path.clone());
    let (mut one, mut two) = (space.create().unwrap(), space.create().unwrap());
    one.write_at(0, b"a sorted run").unwrap();
    two.write_at(0, b"another").unwrap();
    assert_eq!((one.size().unwrap(), two.size().unwrap()), (12, 7));
    let mut back = [0; 6];
    one.read_at(2, &mut back).unwrap();
    assert_eq!(&back, b"sorted");
    // Two files, which are deleted when their handles are dropped.
    assert_eq!(beside(), before + 2);
    drop((one, two));
    assert_eq!(beside(), before);

    // @end
    // @listing temp-db-test
    // A database opened from a file sorts beside that file, and cleans up.
    for old in [path.clone(), format!("{path}-log")] {
        let _ = std::fs::remove_file(old);
    }
    let mut db = Database::open_file(&path).unwrap();
    db.execute("CREATE TABLE t (n INTEGER)").unwrap();
    db.execute("BEGIN").unwrap();
    for n in 0..300 {
        db.execute(&format!("INSERT INTO t VALUES ({n} * 7 % 300)")).unwrap();
    }
    db.execute("COMMIT").unwrap();
    db.set_sort_budget(256);
    let sorted = query(&mut db, "SELECT n FROM t ORDER BY n DESC");
    assert_eq!(
        (sorted.len(), sorted[0].as_str(), sorted[299].as_str()),
        (300, "299", "0")
    );
    assert_eq!(beside(), before);
    // @end
}

#[test]
fn limit_and_offset() {
    // @listing limit-test
    let (mut db, _) = fixture();
    // The three latest arrivals of the month.
    assert_eq!(
        query(
            &mut db,
            "SELECT day, carrier, origin, dest, delay FROM flights
             ORDER BY delay DESC LIMIT 3"
        ),
        [
            "2024-01-02 OO RDM SFO 315",
            "2024-01-10 WN RNO LAS 292",
            "2024-01-19 AA PDX PHX 202"
        ]
    );
    // The next two.
    let page = "SELECT id, delay FROM flights ORDER BY delay DESC LIMIT 2 OFFSET";
    assert_eq!(query(&mut db, &format!("{page} 3")), ["487008 178", "538992 148"]);
    // The last row of 200 is a NULL, because the order is descending. Past the
    // end there is nothing, and a limit of nothing is nothing.
    assert_eq!(query(&mut db, &format!("{page} 199")), ["533520 NULL"]);
    assert_eq!(query(&mut db, &format!("{page} 200")), Vec::<String>::new());
    assert_eq!(
        query(&mut db, "SELECT id FROM flights LIMIT 0"),
        Vec::<String>::new()
    );
    // OFFSET by itself drops rows from the front.
    assert_eq!(
        query(&mut db, "SELECT code FROM carriers OFFSET 12"),
        ["UA", "WN", "YX"]
    );
    // @end
}

#[test]
fn limit_stops_asking() {
    // @listing stops-test
    let (_, file) = fixture();
    // Open the file again, so that no page is in memory, and count the pages
    // that each query reads.
    let reads = |sql: &str| {
        let mut db = Database::open(Box::new(file.clone())).unwrap();
        let before = db.pager().stats().reads;
        let rows = query(&mut db, sql).len();
        (rows, db.pager().stats().reads - before)
    };
    assert_eq!(reads("SELECT id FROM flights"), (200, 5));
    assert_eq!(reads("SELECT id FROM flights LIMIT 3"), (3, 2));
    // With a sort underneath, the limit saves nothing.
    assert_eq!(reads("SELECT id FROM flights ORDER BY delay LIMIT 3"), (3, 5));
    // @end

    // The operator, by hand: it never asks for a row it will not hand on.
    let (mut db, _) = fixture();
    let table = db.table("flights").unwrap().clone();
    let scan = Scan::new(&table, db.pager()).unwrap();
    let mut limit = Limit { input: Box::new(scan), skip: 198, take: Some(5) };
    let mut rows = 0;
    while limit.next(db.pager()).unwrap().is_some() {
        rows += 1;
    }
    assert_eq!(rows, 2);
}

#[test]
fn sorting_against_a_model() {
    // @listing model-data
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute(
        "CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER, word TEXT, x REAL)",
    )
    .unwrap();
    // 500 rows with few distinct values, so that ties are everywhere, and a
    // NULL in one row of eight.
    let words = ["ATL", "atl", "", "DEN", "DENVER"];
    let mut rows: Vec<(i64, Option<i64>, &str, f64)> = Vec::new();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for id in 1..=500 {
        let (a, b, c) = (next(&mut state), next(&mut state), next(&mut state));
        let n = if a % 8 == 0 { None } else { Some((a % 7) as i64 - 3) };
        let (word, x) = (words[(b % 5) as usize], (c % 9) as f64 / 2.0);
        let number = n.map_or("NULL".to_string(), |n| n.to_string());
        db.execute(&format!(
            "INSERT INTO t VALUES ({id}, {number}, '{word}', {x:?})"
        ))
        .unwrap();
        rows.push((id, n, word, x));
    }
    let ids = |rows: &[(i64, Option<i64>, &str, f64)]| -> Vec<String> {
        rows.iter().map(|row| row.0.to_string()).collect()
    };
    // @end
    // @listing model
    for budget in [SORT_BUDGET, 4096, 700, 50] {
        db.set_sort_budget(budget);
        // Rust's sort is stable too, and `None` sorts before `Some`.
        rows.sort_by_key(|row| row.0);
        rows.sort_by_key(|row| row.1);
        assert_eq!(query(&mut db, "SELECT id FROM t ORDER BY n"), ids(&rows));

        rows.sort_by_key(|row| row.0);
        rows.sort_by(|a, b| (a.2, Reverse(a.1)).cmp(&(b.2, Reverse(b.1))));
        assert_eq!(
            query(&mut db, "SELECT id FROM t ORDER BY word, n DESC"),
            ids(&rows)
        );

        rows.sort_by_key(|row| row.0);
        rows.sort_by(|a, b| (b.3 + 1.0).total_cmp(&(a.3 + 1.0)).then(a.1.cmp(&b.1)));
        let sql = "SELECT id, x + 1.0 AS y FROM t ORDER BY y DESC, n LIMIT 400";
        let got: Vec<String> = query(&mut db, sql)
            .iter()
            .map(|l| l.split(' ').next().unwrap().into())
            .collect();
        assert_eq!(got, ids(&rows)[..400]);
    }
    // @end
}
