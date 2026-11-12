//! The examples in Chapter 22: `cargo test --test book_ch22`.
use felite::{Database, HashJoin, MemFile, NestedLoop, Operator, Output, Row, Scan};
use felite::{Scope, Statement, Table, bind, bind_condition, parse, parse_expr};

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
fn parsing_joins() {
    // @listing parse-test
    let sql = "select f.id, a.city from flights f \
               join airports as a on f.origin = a.code \
               left join carriers c on c.code = f.carrier where f.delay > 60";
    let Statement::Select(select) = parse(sql).unwrap() else { panic!() };
    assert_eq!(
        (select.from.as_str(), select.alias.as_deref()),
        ("flights", Some("f"))
    );
    let joins: Vec<(&str, Option<&str>, bool, String)> = select
        .joins
        .iter()
        .map(|j| (j.table.as_str(), j.alias.as_deref(), j.left, j.on.to_string()))
        .collect();
    assert_eq!(
        joins,
        [
            ("airports", Some("a"), false, "(f.origin = a.code)".to_string()),
            ("carriers", Some("c"), true, "(c.code = f.carrier)".to_string()),
        ]
    );
    assert_eq!(
        parse(sql).unwrap().to_string(),
        "SELECT f.id, a.city FROM flights f JOIN airports a ON (f.origin = a.code) \
         LEFT JOIN carriers c ON (c.code = f.carrier) WHERE (f.delay > 60)"
    );
    // @end
    // @listing parse-more
    // INNER JOIN is JOIN said at greater length.
    let inner = "SELECT id FROM flights INNER JOIN carriers ON carrier = code";
    assert_eq!(
        parse(inner).unwrap().to_string(),
        "SELECT id FROM flights JOIN carriers ON (carrier = code)"
    );

    let message = |sql: &str| {
        let message = parse(sql).unwrap_err().to_string();
        message.split_once(": ").unwrap().1.to_string()
    };
    assert_eq!(
        message("SELECT id FROM flights JOIN carriers"),
        "expected `ON`, found the end of the statement"
    );
    assert_eq!(
        message("SELECT id FROM flights LEFT carriers ON carrier = code"),
        "expected `JOIN`, found the name `carriers`"
    );
    // @end
}

#[test]
fn names_with_two_parts() {
    // @listing names-test
    let (mut db, _) = fixture();
    // `name` is in one table only, so it needs no table in front of it. `code` is
    // in both, and has to say which it means.
    assert_eq!(
        query(
            &mut db,
            "SELECT id, name FROM flights JOIN carriers ON carrier = carriers.code
             WHERE delay > 200 ORDER BY id"
        ),
        [
            "38304 American Airlines Inc.",
            "311904 Southwest Airlines Co.",
            "454176 SkyWest Airlines Inc."
        ]
    );
    let mut refused = |sql: &str| error(&mut db, sql);
    // The first ON is fine, since only one table in it has a `code`. By the
    // second there are two.
    assert_eq!(
        refused(
            "SELECT id FROM flights JOIN carriers ON carrier = code \
             JOIN airports ON origin = code"
        ),
        "error at line 1, column 82: the name code could mean two columns"
    );
    // @end
    // @listing names-errors
    // Once a table has been given another name, that is its only name.
    assert_eq!(
        refused("SELECT flights.id FROM flights f"),
        "error at line 1, column 8: there is no column flights.id"
    );
    // A table can be joined to itself, but only under two names.
    assert_eq!(
        refused("SELECT id FROM flights JOIN flights ON id = id"),
        "flights is in this query twice, and needs two different names"
    );
    assert_eq!(
        refused("SELECT f.id FROM flights f JOIN airports a ON f.origin"),
        "error at line 1, column 47: ON needs a condition, and this is TEXT"
    );
    // @end
}

// @listing joined
/// Two tables of the example data, a scope with the columns of both, and a scan
/// of the first.
fn two_tables(
    db: &mut Database,
    outer: &str,
    inner: &str,
) -> (Table, Scope, Box<dyn Operator>) {
    let (outer, inner) =
        (db.table(outer).unwrap().clone(), db.table(inner).unwrap().clone());
    let mut scope = Scope::of(&outer);
    scope.columns.extend(Scope::of(&inner).columns);
    let scan = Scan::new(&outer, db.pager()).unwrap();
    (inner, scope, Box::new(scan))
}

/// Pulls every row out of an operator.
fn drain(operator: &mut dyn Operator, db: &mut Database) -> Vec<Row> {
    let mut rows = Vec::new();
    while let Some(row) = operator.next(db.pager()).unwrap() {
        rows.push(row);
    }
    rows
}
// @end

#[test]
fn the_nested_loop() {
    // @listing nested-test
    let (mut db, _) = fixture();
    let (carriers, scope, flights) = two_tables(&mut db, "flights", "carriers");
    let on = parse_expr("flights.carrier = carriers.code").unwrap();
    let condition = bind_condition(&on, &scope, "ON").unwrap();
    let mut join = NestedLoop::new(flights, &carriers, condition, false);
    let rows = drain(&mut join, &mut db);

    // Every flight has one carrier, so there are as many rows as flights. Each
    // is a flight's 13 values followed by its carrier's 2.
    assert_eq!(rows.len(), 200);
    let shown: Vec<String> = rows[0].iter().map(|v| v.to_string()).collect();
    assert_eq!(
        shown.join(" "),
        "2736 2024-01-22 9E 4944 N131EV ATL DHN 1539 -9 -18 0 36 170.0 \
         9E Endeavor Air Inc."
    );
    // To find them, the condition was tried on every flight with every carrier.
    assert_eq!(join.pairs, 200 * 15);
    // @end
}

#[test]
fn hashing_one_side() {
    // @listing hash-test
    let (mut db, _) = fixture();
    let (carriers, scope, flights) = two_tables(&mut db, "flights", "carriers");
    let on = parse_expr("flights.carrier = carriers.code").unwrap();
    let condition = bind_condition(&on, &scope, "ON").unwrap();
    let mut nested = NestedLoop::new(flights, &carriers, condition, false);
    let expected = drain(&mut nested, &mut db);

    // The same join with a hash table. Each key is bound against its own table.
    let (carriers, _, flights) = two_tables(&mut db, "flights", "carriers");
    let key = |table: &str, column: &str| {
        let scope = Scope::of(db.table(table).unwrap());
        bind(&parse_expr(column).unwrap(), &scope).unwrap()
    };
    let keys = (key("flights", "carrier"), key("carriers", "code"));
    let scan = Box::new(Scan::new(&carriers, db.pager()).unwrap());
    let mut join = HashJoin::new(flights, scan, keys, 2, false);
    let rows = drain(&mut join, &mut db);

    // The same rows in the same order, for 215 steps in place of 3,000.
    assert_eq!(rows, expected);
    assert_eq!((join.built, join.probes), (15, 200));
    // @end
}

#[test]
fn what_each_costs() {
    // @listing cost-test
    let (mut db, _) = fixture();
    // Each flight with the airport it left from. The inner table is 334 rows
    // now, where the carriers were 15.
    let (airports, scope, flights) = two_tables(&mut db, "flights", "airports");
    let on = parse_expr("origin = code").unwrap();
    let condition = bind_condition(&on, &scope, "ON").unwrap();
    let mut nested = NestedLoop::new(flights, &airports, condition, false);
    let expected = drain(&mut nested, &mut db);
    assert_eq!((expected.len(), nested.pairs), (200, 66_800));

    let (airports, _, flights) = two_tables(&mut db, "flights", "airports");
    let key = |table: &str, column: &str| {
        let scope = Scope::of(db.table(table).unwrap());
        bind(&parse_expr(column).unwrap(), &scope).unwrap()
    };
    let keys = (key("flights", "origin"), key("airports", "code"));
    let scan = Box::new(Scan::new(&airports, db.pager()).unwrap());
    let mut hash = HashJoin::new(flights, scan, keys, 3, false);
    assert_eq!(drain(&mut hash, &mut db), expected);
    assert_eq!((hash.built, hash.probes), (334, 200));
    // @end
}

#[test]
fn joins_in_sql() {
    // @listing sql-test
    let (mut db, _) = fixture();
    // Hawaiian's three flights, and where each one left from.
    assert_eq!(
        query(
            &mut db,
            "SELECT f.id, a.city, a.state FROM flights f
             JOIN airports a ON f.origin = a.code WHERE f.carrier = 'HA'
             ORDER BY f.id"
        ),
        ["229824 Las Vegas NV", "232560 Kahului HI", "235296 Honolulu HI"]
    );
    // A join's rows can be grouped like any others: the states that the most
    // flights left from.
    assert_eq!(
        query(
            &mut db,
            "SELECT a.state, COUNT(*) AS n FROM flights f
             JOIN airports a ON f.origin = a.code
             GROUP BY a.state ORDER BY n DESC, a.state LIMIT 4"
        ),
        ["TX 24", "CA 19", "FL 16", "IL 13"]
    );
    // @end
}

#[test]
fn any_condition() {
    // @listing condition-test
    let (mut db, _) = fixture();
    let mut one = |sql: &str| query(&mut db, sql).join(", ");
    // Pairs of flights by the same aircraft. The condition is more than one
    // equality, so this runs as a nested loop over 40,000 pairs.
    let pairs = "SELECT f.id, g.id, f.tail FROM flights f JOIN flights g";
    let by_id = "ORDER BY f.id";
    assert_eq!(
        one(&format!(
            "{pairs} ON f.tail = g.tail AND f.id < g.id WHERE f.id < 100000 {by_id}"
        )),
        "32832 84816 N305NX, 35568 43776 N970UY"
    );
    // The same question with the second half in the WHERE clause. Now the ON is
    // a single equality, and a hash join finds the candidates.
    assert_eq!(
        one(&format!(
            "{pairs} ON f.tail = g.tail WHERE f.id < g.id AND f.id < 100000 {by_id}"
        )),
        "32832 84816 N305NX, 35568 43776 N970UY"
    );
    // A condition need not mention both tables, or be an equality at all.
    assert_eq!(
        one("SELECT COUNT(*) FROM flights f JOIN airports a
             ON f.miles > 2500.0 AND a.state = 'HI'"),
        "30"
    );
    // @end
}

#[test]
fn left_join() {
    // @listing left-test
    let (mut db, _) = fixture();
    // Hawaii's five airports, and the flights of the sample that left from each.
    assert_eq!(
        query(
            &mut db,
            "SELECT a.code, f.id, f.dest FROM airports a
             LEFT JOIN flights f ON f.origin = a.code
             WHERE a.state = 'HI' ORDER BY a.code, f.id"
        ),
        [
            "HNL 235296 OGG",
            "HNL 290016 LIH",
            "ITO NULL NULL",
            "KOA NULL NULL",
            "LIH NULL NULL",
            "OGG 232560 KOA",
            "OGG 331056 HNL",
            "OGG 500688 SFO"
        ]
    );
    // With JOIN in place of LEFT JOIN, the three airports with no flights vanish.
    assert_eq!(
        query(
            &mut db,
            "SELECT a.code FROM airports a JOIN flights f ON f.origin = a.code
             WHERE a.state = 'HI'"
        )
        .len(),
        5
    );
    // @end
}

#[test]
fn counting_what_is_missing() {
    // @listing missing-test
    let (mut db, _) = fixture();
    let mut one = |sql: &str| query(&mut db, sql).join(", ");
    let joined = "FROM airports a LEFT JOIN flights f ON f.origin = a.code";
    // 200 rows for the flights, and one more for each airport that has none.
    assert_eq!(one(&format!("SELECT COUNT(*), COUNT(f.id) {joined}")), "451 200");
    // Those airports are the rows where the flight's side is all NULL.
    assert_eq!(one(&format!("SELECT COUNT(*) {joined} WHERE f.id IS NULL")), "251");
    // COUNT(f.id) skips those NULLs, so it is the true number of flights from
    // each airport, zero included.
    assert_eq!(
        one(&format!(
            "SELECT a.code, COUNT(f.id) {joined} WHERE a.state = 'HI' GROUP BY a.code"
        )),
        "HNL 2, ITO 0, KOA 0, LIH 0, OGG 3"
    );
    // @end
}

#[test]
fn on_or_where() {
    // @listing on-where-test
    let (mut db, _) = fixture();
    let mut rows = |sql: &str| query(&mut db, sql).len();
    let airports = "SELECT a.code, f.id FROM airports a LEFT JOIN flights f";
    // In the ON, the extra condition decides which flights match an airport. An
    // airport with no Hawaiian flights is kept, with NULLs.
    assert_eq!(
        rows(&format!("{airports} ON f.origin = a.code AND f.carrier = 'HA'")),
        334
    );
    // In the WHERE, it is applied to the joined rows, and a row of NULLs fails
    // it. Only the three Hawaiian flights are left.
    assert_eq!(
        rows(&format!("{airports} ON f.origin = a.code WHERE f.carrier = 'HA'")),
        3
    );
    // @end
}

#[test]
fn nulls_never_match() {
    // @listing nulls-test
    let (mut db, _) = fixture();
    let mut one = |sql: &str| query(&mut db, sql).join(", ");
    let same_aircraft = "flights g ON f.tail = g.tail";
    // Each of the 196 flights with a tail number matches itself, and the six
    // aircraft that appear twice add two more pairs each. The four flights with
    // no tail number match nothing, not themselves and not each other.
    assert_eq!(
        one(&format!("SELECT COUNT(*) FROM flights f JOIN {same_aircraft}")),
        "208"
    );
    // A LEFT JOIN keeps those four, each with nothing beside it.
    assert_eq!(
        one(&format!(
            "SELECT COUNT(*), COUNT(g.id) FROM flights f LEFT JOIN {same_aircraft}"
        )),
        "212 208"
    );
    // @end
}

#[test]
fn three_tables() {
    // @listing three-test
    let (mut db, _) = fixture();
    // The four latest arrivals, with every code spelled out. `airports` is
    // joined twice, once for each end of the flight.
    assert_eq!(
        query(
            &mut db,
            "SELECT c.name, o.city, d.city, f.delay FROM flights f
             JOIN carriers c ON c.code = f.carrier
             JOIN airports o ON o.code = f.origin
             JOIN airports d ON d.code = f.dest
             ORDER BY f.delay DESC LIMIT 4"
        ),
        [
            "SkyWest Airlines Inc. Bend/Redmond San Francisco 315",
            "Southwest Airlines Co. Reno Las Vegas 292",
            "American Airlines Inc. Portland Phoenix 202",
            "SkyWest Airlines Inc. Great Falls Salt Lake City 178"
        ]
    );
    // SELECT * is every column of every table: 13 + 2 + 3 + 3.
    let Output::Rows(all) = db
        .execute(
            "SELECT * FROM flights f JOIN carriers c ON c.code = f.carrier
             JOIN airports o ON o.code = f.origin JOIN airports d ON d.code = f.dest",
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!((all.rows.len(), all.columns.len()), (200, 21));
    assert_eq!(
        all.columns[13..],
        ["code", "name", "code", "city", "state", "code", "city", "state"]
    );
    // @end
}

#[test]
fn joins_against_a_model() {
    // @listing model
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute("CREATE TABLE a (id INTEGER PRIMARY KEY, k INTEGER)").unwrap();
    db.execute("CREATE TABLE b (id INTEGER PRIMARY KEY, k INTEGER)").unwrap();
    // Keys from a small range, so that they repeat, and one in six is NULL.
    let mut state = 0x9e37_79b9_7f4a_7c15;
    let mut rows = |db: &mut Database, table: &str, count: i64| {
        let mut keys = Vec::new();
        for id in 1..=count {
            let n = next(&mut state);
            let k = if n.is_multiple_of(6) { None } else { Some((n % 40) as i64) };
            let sql = k.map_or("NULL".to_string(), |k| k.to_string());
            db.execute(&format!("INSERT INTO {table} VALUES ({id}, {sql})")).unwrap();
            keys.push((id, k));
        }
        keys
    };
    let (a, b) = (rows(&mut db, "a", 150), rows(&mut db, "b", 90));

    // @end
    // @listing model-check
    // The model is the definition of a join: every pair, and a test.
    let (mut inner, mut left) = (Vec::new(), Vec::new());
    for (a_id, a_k) in &a {
        let matches: Vec<&(i64, Option<i64>)> =
            b.iter().filter(|(_, b_k)| a_k.is_some() && a_k == b_k).collect();
        for (b_id, _) in &matches {
            inner.push(format!("{a_id} {b_id}"));
            left.push(format!("{a_id} {b_id}"));
        }
        if matches.is_empty() {
            left.push(format!("{a_id} NULL"));
        }
    }
    assert_eq!((inner.len(), left.len()), (244, 292));
    // Each join twice: as a single equality, which is hashed, and with a second
    // condition that is always true, which makes it a nested loop.
    for on in ["a.k = b.k", "a.k = b.k AND a.id > 0"] {
        let sql =
            format!("SELECT a.id, b.id FROM a JOIN b ON {on} ORDER BY a.id, b.id");
        assert_eq!(query(&mut db, &sql), inner);
        let sql = format!(
            "SELECT a.id, b.id FROM a LEFT JOIN b ON {on} ORDER BY a.id, b.id"
        );
        assert_eq!(query(&mut db, &sql), left);
    }
    // @end
}
