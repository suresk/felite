//! The examples in Chapter 21: `cargo test --test book_ch21`.
use felite::{Aggregate, AggregateCall, Database, ExprKind, Function, MemFile};
use felite::{Operator, Output, Row, Scan, Scope, Statement, Value};
use felite::{bind, parse, parse_expr};
use std::collections::BTreeMap;

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
fn parsing_aggregates() {
    // @listing parse-test
    let sql = "select carrier, count(*), avg(delay) from flights where miles > 500.0 \
               group by carrier having count(*) > 5 order by sum(miles) desc";
    let Statement::Select(select) = parse(sql).unwrap() else { panic!() };
    assert_eq!(select.group_by.len(), 1);
    assert_eq!(select.having.unwrap().to_string(), "(COUNT(*) > 5)");
    assert_eq!(
        parse(sql).unwrap().to_string(),
        "SELECT carrier, COUNT(*), AVG(delay) FROM flights WHERE (miles > 500.0) \
         GROUP BY carrier HAVING (COUNT(*) > 5) ORDER BY SUM(miles) DESC"
    );

    // A call is an expression like any other, and can be part of a larger one.
    let expr = parse_expr("max(delay) - min(delay)").unwrap();
    assert_eq!(expr.to_string(), "(MAX(delay) - MIN(delay))");
    let call = parse_expr("Sum(miles / 60.0)").unwrap();
    let ExprKind::Call { function, arg } = call.kind else { panic!() };
    assert_eq!(function, Function::Sum);
    assert_eq!(arg.unwrap().to_string(), "(miles / 60.0)");

    let message = |sql: &str| {
        let message = parse_expr(sql).unwrap_err().to_string();
        message.split_once(": ").unwrap().1.to_string()
    };
    assert_eq!(message("median(delay)"), "there is no function named median");
    assert_eq!(message("sum(*)"), "expected an expression, found `*`");
    assert_eq!(message("count()"), "expected an expression, found `)`");
    assert_eq!(message("count(delay, miles)"), "expected `)`, found `,`");
    // @end
}

#[test]
fn one_answer_from_many_rows() {
    // @listing whole-table-test
    let (mut db, _) = fixture();
    let mut one = |sql: &str| query(&mut db, sql).join(" | ");
    assert_eq!(one("SELECT COUNT(*) FROM flights"), "200");
    assert_eq!(one("SELECT SUM(cancelled), SUM(miles) FROM flights"), "11 165332.0");
    assert_eq!(one("SELECT MIN(delay), MAX(delay) FROM flights"), "-46 315");
    // MIN and MAX work on anything that has an order.
    assert_eq!(
        one("SELECT MIN(day), MAX(day) FROM flights"),
        "2024-01-01 2024-01-31"
    );
    // A filter comes first: the aggregate sees only the rows that pass it.
    assert_eq!(one("SELECT COUNT(*) FROM flights WHERE carrier = 'WN'"), "43");
    // Aggregates can be parts of a larger expression. This is the average speed
    // of the whole sample, in miles an hour.
    assert_eq!(
        one("SELECT SUM(miles) / SUM(air_time) * 60.0 FROM flights
             WHERE air_time IS NOT NULL"),
        "425.77233089794953"
    );
    // @end
}

#[test]
fn nulls_in_aggregates() {
    // @listing nulls-test
    let (mut db, _) = fixture();
    let mut one = |sql: &str| query(&mut db, sql).join(" | ");
    // Twelve flights have no arrival delay, and four have no tail number.
    assert_eq!(
        one("SELECT COUNT(*), COUNT(delay), COUNT(tail) FROM flights"),
        "200 188 196"
    );
    // The average is over the 188 flights that have a delay, not over all 200.
    assert_eq!(
        one("SELECT SUM(delay), AVG(delay) FROM flights"),
        "1774 9.436170212765957"
    );
    assert_eq!(1774.0 / 188.0, 9.436170212765957);
    // So the average is not the sum divided by the number of rows.
    assert_eq!(one("SELECT SUM(delay) / COUNT(*) FROM flights"), "8");

    // AVG is a real even when every value is an integer. SUM keeps the type.
    assert_eq!(one("SELECT SUM(number), AVG(number) FROM flights"), "465622 2328.11");
    // @end
}

#[test]
fn no_rows_at_all() {
    // @listing empty-test
    let (mut db, _) = fixture();
    let mut one = |sql: &str| query(&mut db, sql).join(" | ");
    // No airline has the code XX. One row comes back all the same.
    assert_eq!(
        one("SELECT COUNT(*), COUNT(delay), SUM(delay), AVG(delay), MIN(delay)
             FROM flights WHERE carrier = 'XX'"),
        "0 0 NULL NULL NULL"
    );
    // The same happens when there are rows and every value is NULL.
    assert_eq!(
        one("SELECT COUNT(*), COUNT(delay), SUM(delay), MAX(delay)
             FROM flights WHERE cancelled = 1"),
        "11 0 NULL NULL"
    );
    // With a GROUP BY, no rows means no groups, and so no rows of output.
    assert_eq!(
        one("SELECT carrier, COUNT(*) FROM flights WHERE carrier = 'XX'
             GROUP BY carrier"),
        ""
    );
    // @end
}

#[test]
fn aggregate_by_hand() {
    // @listing by-hand
    let (mut db, _) = fixture();
    let table = db.table("flights").unwrap().clone();
    let scope = Scope::of(&table);
    let bound = |sql: &str| bind(&parse_expr(sql).unwrap(), &scope).unwrap();

    // The operators for: SELECT cancelled, COUNT(*), COUNT(delay), MAX(miles)
    //                    FROM flights GROUP BY cancelled
    let span = parse_expr("0").unwrap().span;
    let call = |function, arg: Option<&str>| AggregateCall {
        function,
        arg: arg.map(bound),
        span,
    };
    let calls = vec![
        call(Function::Count, None),
        call(Function::Count, Some("delay")),
        call(Function::Max, Some("miles")),
    ];
    let scan = Scan::new(&table, db.pager()).unwrap();
    let mut aggregate =
        Aggregate::new(Box::new(scan), vec![bound("cancelled")], calls);

    let mut rows = Vec::new();
    while let Some(row) = aggregate.next(db.pager()).unwrap() {
        rows.push(row);
    }
    // Two groups. Each row is the group's key, then the three aggregates.
    use Value::{Integer, Real};
    assert_eq!(
        rows,
        [
            vec![Integer(0), Integer(189), Integer(188), Real(2846.0)],
            vec![Integer(1), Integer(11), Integer(0), Real(2717.0)],
        ]
    );
    // @end
}

#[test]
fn group_by() {
    // @listing group-test
    let (mut db, _) = fixture();
    assert_eq!(
        query(&mut db, "SELECT carrier, COUNT(*) FROM flights GROUP BY carrier")
            .join(", "),
        "9E 6, AA 28, AS 6, B6 8, DL 27, F9 5, G4 3, HA 3, \
         MQ 7, NK 7, OH 6, OO 21, UA 22, WN 43, YX 8"
    );
    // Several aggregates over each group, and the five largest groups first.
    assert_eq!(
        query(
            &mut db,
            "SELECT carrier, COUNT(*) AS flights, SUM(cancelled), AVG(delay)
             FROM flights GROUP BY carrier ORDER BY flights DESC LIMIT 5"
        ),
        [
            "WN 43 0 13.325581395348838",
            "AA 28 1 9.185185185185185",
            "DL 27 0 6.2592592592592595",
            "UA 22 4 7.611111111111111",
            "OO 21 2 28.11111111111111"
        ]
    );
    // @end
}

#[test]
fn groups_of_several_kinds() {
    // @listing keys-test
    let (mut db, _) = fixture();
    let mut all = |sql: &str| query(&mut db, sql).join(", ");
    // Two keys make a group for each pair of values that occurs.
    assert_eq!(
        all("SELECT carrier, origin, COUNT(*) FROM flights
             WHERE carrier = 'DL' OR carrier = 'UA'
             GROUP BY carrier, origin HAVING COUNT(*) > 2"),
        "DL ATL 4, DL SEA 3, UA EWR 5, UA ORD 3"
    );
    // A key can be any expression. This one is true, false or unknown, and the
    // rows where it is unknown are a group like the others.
    assert_eq!(
        all("SELECT delay > 15, COUNT(*) FROM flights GROUP BY delay > 15"),
        "NULL 12, 0 142, 1 46"
    );
    // A SELECT list can leave the key out, though the result is hard to read.
    assert_eq!(all("SELECT COUNT(*) FROM flights GROUP BY cancelled"), "189, 11");
    // @end
}

#[test]
fn having() {
    // @listing having-test
    let (mut db, _) = fixture();
    let mut all = |sql: &str| query(&mut db, sql).join(", ");
    // The airports that more than six of the flights left from.
    assert_eq!(
        all("SELECT origin, COUNT(*) AS n FROM flights GROUP BY origin
             HAVING COUNT(*) > 6 ORDER BY n DESC, origin"),
        "DFW 12, ORD 10, ATL 9, LAX 9, EWR 8, DEN 7"
    );
    // WHERE chooses rows before they are grouped, and HAVING chooses groups
    // afterwards. A query can do both, and they are different questions.
    assert_eq!(
        all("SELECT carrier, COUNT(*) FROM flights WHERE delay > 60
             GROUP BY carrier HAVING COUNT(*) >= 3"),
        "OO 3, UA 3, WN 3"
    );
    // HAVING can use an aggregate that the SELECT list does not.
    assert_eq!(
        all("SELECT carrier FROM flights GROUP BY carrier HAVING MAX(delay) > 200"),
        "AA, OO, WN"
    );
    // @end
}

#[test]
fn what_you_may_select() {
    // @listing rule-test
    let (mut db, _) = fixture();
    let mut refused = |sql: &str| error(&mut db, sql);
    assert_eq!(
        refused("SELECT carrier, origin, COUNT(*) FROM flights GROUP BY carrier"),
        "error at line 1, column 17: \
         origin must be in the GROUP BY or inside an aggregate"
    );
    // An aggregate with no GROUP BY makes the whole table one group, so the same
    // rule applies.
    assert_eq!(
        refused("SELECT day, MAX(delay) FROM flights"),
        "error at line 1, column 8: \
         day must be in the GROUP BY or inside an aggregate"
    );
    assert_eq!(
        refused("SELECT carrier FROM flights GROUP BY carrier HAVING delay > 60"),
        "error at line 1, column 53: \
         delay must be in the GROUP BY or inside an aggregate"
    );
    assert_eq!(
        refused("SELECT * FROM flights GROUP BY carrier"),
        "a query with groups can't select *"
    );
    // @end

    // @listing misuse-test
    // An aggregate needs groups to work on, and there are none yet in a WHERE,
    // or inside another aggregate.
    assert_eq!(
        refused("SELECT carrier FROM flights WHERE COUNT(*) > 5 GROUP BY carrier"),
        "error at line 1, column 35: COUNT is an aggregate, and can't be used here"
    );
    assert_eq!(
        refused("SELECT MAX(COUNT(*)) FROM flights GROUP BY carrier"),
        "error at line 1, column 12: COUNT is an aggregate, and can't be used here"
    );
    assert_eq!(
        refused("SELECT SUM(carrier) FROM flights"),
        "error at line 1, column 8: SUM needs a number, and this is TEXT"
    );
    assert_eq!(
        refused("SELECT carrier FROM flights GROUP BY carrier HAVING COUNT(*)"),
        "error at line 1, column 53: HAVING needs a condition, and this is an INTEGER"
    );
    // @end
}

#[test]
fn what_matches_a_group() {
    // @listing match-test
    let (mut db, _) = fixture();
    let mut all = |sql: &str| query(&mut db, sql).join(", ");
    // The key is an expression, and the SELECT list may use that expression, in
    // any mix of case, and build on it.
    assert_eq!(
        all("SELECT MILES > 2000.0, (miles > 2000.0) IS NULL, COUNT(*)
             FROM flights GROUP BY miles > 2000.0"),
        "0 0 184, 1 0 16"
    );
    // The same aggregate written twice is worked out once.
    assert_eq!(
        all("SELECT MAX(delay) - MIN(delay), MAX(delay) FROM flights"),
        "361 315"
    );
    // @end
    // `miles` by itself is not the key, though the key is made from it.
    assert!(
        db.execute("SELECT miles, COUNT(*) FROM flights GROUP BY miles > 2000.0")
            .is_err()
    );
}

#[test]
fn sums_that_overflow() {
    // @listing overflow-test
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute("CREATE TABLE t (n INTEGER, x REAL)").unwrap();
    for n in [i64::MAX, 1, -5] {
        db.execute(&format!("INSERT INTO t VALUES ({n}, {n})")).unwrap();
    }
    assert_eq!(
        error(&mut db, "SELECT SUM(n) FROM t"),
        "error at line 1, column 8: the sum is too large for an integer"
    );
    // The same numbers as reals are no trouble, and nor is their average.
    assert_eq!(
        query(&mut db, "SELECT SUM(x), AVG(n) FROM t"),
        ["9.223372036854776e18 3.0744573456182584e18"]
    );
    // @end
}

#[test]
fn grouping_against_a_model() {
    // @listing model
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute(
        "CREATE TABLE t (id INTEGER PRIMARY KEY, k INTEGER, word TEXT, n INTEGER)",
    )
    .unwrap();
    // For each group: its rows, how many have a number, and their sum, least
    // and greatest.
    type Group = (i64, i64, Option<(i64, i64, i64)>);
    let mut model: BTreeMap<(Option<i64>, &str), Group> = BTreeMap::new();
    let words = ["ATL", "DEN", "SLC"];
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for id in 1..=600 {
        let (a, b, c) = (next(&mut state), next(&mut state), next(&mut state));
        let k = if a % 6 == 0 { None } else { Some((a % 5) as i64) };
        let word = words[(b % 3) as usize];
        let n = if c % 4 == 0 { None } else { Some((c % 200) as i64 - 100) };
        let sql = |v: Option<i64>| v.map_or("NULL".to_string(), |v| v.to_string());
        db.execute(&format!(
            "INSERT INTO t VALUES ({id}, {}, '{word}', {})",
            sql(k),
            sql(n)
        ))
        .unwrap();

        let group = model.entry((k, word)).or_insert((0, 0, None));
        group.0 += 1;
        if let Some(n) = n {
            group.1 += 1;
            let (sum, least, most) = group.2.unwrap_or((0, n, n));
            group.2 = Some((sum + n, least.min(n), most.max(n)));
        }
    }
    // @end
    // @listing model-check
    let show = |v: Option<i64>| v.map_or("NULL".to_string(), |v| v.to_string());
    let expected: Vec<String> = model
        .iter()
        .map(|((k, word), (rows, counted, stats))| {
            let (sum, least, most) =
                (stats.map(|s| s.0), stats.map(|s| s.1), stats.map(|s| s.2));
            format!(
                "{} {word} {rows} {counted} {} {} {}",
                show(*k),
                show(sum),
                show(least),
                show(most)
            )
        })
        .collect();
    assert_eq!(expected.len(), 18);
    assert_eq!(
        query(
            &mut db,
            "SELECT k, word, COUNT(*), COUNT(n), SUM(n), MIN(n), MAX(n) FROM t
             GROUP BY k, word"
        ),
        expected
    );
    // @end
}
