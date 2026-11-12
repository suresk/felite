//! Tests for the Chapter 20 challenges: `cargo test --test challenges_ch20`.
use felite::{Database, Limit, MemFile, Operator, Output, Row, Scan, Scope, Value};
use felite::{Sort, SortKey, TempSpace, bind, parse_expr};
use std::cmp::Ordering;

fn fixture() -> Database {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    for statement in include_str!("../fixtures/flights.sql").lines() {
        db.execute(statement).unwrap();
    }
    db
}

fn lines(output: Output) -> Vec<String> {
    let Output::Rows(result) = output else { panic!("not a query") };
    let line =
        |row: &Row| row.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
    result.rows.iter().map(line).collect()
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// A scan of the flights and keys to sort them by, each as SQL with a direction.
fn scan_and_keys(
    db: &mut Database,
    keys: &[(&str, bool)],
) -> (Box<dyn Operator>, Vec<SortKey>) {
    let table = db.table("flights").unwrap().clone();
    let scope = Scope::of(&table);
    let key = |(sql, descending): &(&str, bool)| SortKey {
        expr: bind(&parse_expr(sql).unwrap(), &scope).unwrap(),
        descending: *descending,
    };
    let scan = Scan::new(&table, db.pager()).unwrap();
    (Box::new(scan), keys.iter().map(key).collect())
}

fn drain(mut operator: impl Operator, db: &mut Database) -> Vec<Row> {
    let mut rows = Vec::new();
    while let Some(row) = operator.next(db.pager()).unwrap() {
        rows.push(row);
    }
    rows
}

// @challenge 2
use felite::TopN;

// @challenge 3
use felite::order_with;

// @challenge 4
use felite::merge_passes;

// @challenge 5
use felite::replacement_runs;

// @challenge 6
use felite::query_by_position;

// @challenge 1
#[test]
fn predict_runs() {
    let mut db = fixture();
    for (budget, runs) in [(2048, 7), (3000, 5), (12_290, 0), (12_289, 2)] {
        let (scan, keys) = scan_and_keys(&mut db, &[("delay", false)]);
        let mut sort = Sort::new(scan, keys, budget, TempSpace::Memory);
        while sort.next(db.pager()).unwrap().is_some() {}
        assert_eq!(sort.stats().runs, runs, "a budget of {budget}");
    }
}

// @challenge 2
#[test]
fn the_first_few_without_the_sort() {
    let mut db = fixture();
    for keys in [
        vec![("delay", true)],
        vec![("carrier", false), ("miles", true)],
        vec![("cancelled", true)],
    ] {
        for n in [0, 1, 5, 40, 200, 500] {
            let (scan, sort_keys) = scan_and_keys(&mut db, &keys);
            let sort = Sort::new(scan, sort_keys, 1 << 20, TempSpace::Memory);
            let limit = Limit { input: Box::new(sort), skip: 0, take: Some(n) };
            let expected = drain(limit, &mut db);

            let (scan, sort_keys) = scan_and_keys(&mut db, &keys);
            let mut top = TopN::new(scan, sort_keys, n);
            let mut rows = Vec::new();
            while let Some(row) = top.next(db.pager()).unwrap() {
                rows.push(row);
            }
            assert_eq!(rows, expected);
            assert_eq!(rows.len(), n.min(200));
            assert_eq!(top.held, (n + 1).min(200));
        }
    }
}

// @challenge 3
#[test]
fn nulls_last() {
    let (null, one, two) = (Value::Null, Value::Integer(1), Value::Integer(2));
    assert_eq!(order_with(&one, &two, false, false), Ordering::Less);
    assert_eq!(order_with(&one, &two, true, false), Ordering::Greater);
    assert_eq!(order_with(&null, &one, false, false), Ordering::Less);
    assert_eq!(order_with(&null, &one, false, true), Ordering::Greater);
    assert_eq!(order_with(&null, &one, true, true), Ordering::Greater);
    assert_eq!(order_with(&null, &one, true, false), Ordering::Less);
    assert_eq!(order_with(&one, &null, true, true), Ordering::Less);
    assert_eq!(order_with(&null, &null, true, true), Ordering::Equal);

    let mut values = vec![two.clone(), null.clone(), one.clone(), null.clone()];
    values.sort_by(|a, b| order_with(a, b, true, true));
    assert_eq!(values, [two, one, null.clone(), null]);
}

// @challenge 4
#[test]
fn merging_in_passes() {
    let mut state = 20;
    let mut runs: Vec<Vec<i64>> = Vec::new();
    let mut all = Vec::new();
    for _ in 0..26 {
        let mut run: Vec<i64> =
            (0..40).map(|_| (next(&mut state) % 1000) as i64).collect();
        run.sort();
        all.extend(&run);
        runs.push(run);
    }
    all.sort();
    assert_eq!(merge_passes(runs.clone(), 26), (all.clone(), 1));
    assert_eq!(merge_passes(runs.clone(), 100).1, 1);
    assert_eq!(merge_passes(runs.clone(), 5), (all.clone(), 3));
    assert_eq!(merge_passes(runs[..25].to_vec(), 5).1, 2);
    assert_eq!(merge_passes(runs.clone(), 2), (all, 5));
    assert_eq!(merge_passes(runs[..1].to_vec(), 5), (runs[0].clone(), 0));
    assert_eq!(merge_passes(Vec::new(), 5), (Vec::new(), 0));
}

// @challenge 5
#[test]
fn longer_runs() {
    let sorted: Vec<i64> = (0..1000).collect();
    assert_eq!(replacement_runs(&sorted, 10), std::slice::from_ref(&sorted));
    let backwards: Vec<i64> = (0..1000).rev().collect();
    let runs = replacement_runs(&backwards, 10);
    assert_eq!(runs.len(), 100);
    assert!(runs.iter().all(|run| run.len() == 10));

    let mut state = 20;
    let random: Vec<i64> =
        (0..10_000).map(|_| (next(&mut state) >> 40) as i64).collect();
    let runs = replacement_runs(&random, 100);
    assert!(runs.iter().all(|run| run.is_sorted()));
    let mut all: Vec<i64> = runs.iter().flatten().copied().collect();
    let mut expected = random.clone();
    all.sort();
    expected.sort();
    assert_eq!(all, expected);
    // Runs of 100 would make 100 of them. These average close to 200.
    assert_eq!(runs.len(), 51);
    assert_eq!(replacement_runs(&[], 10), Vec::<Vec<i64>>::new());
}

// @challenge 6
#[test]
fn sorting_by_position() {
    let mut db = fixture();
    let by_number = query_by_position(
        &mut db,
        "SELECT carrier, miles, id FROM flights WHERE miles > 2000.0
         ORDER BY 1, 2 DESC",
    );
    let by_name = db.execute(
        "SELECT carrier, miles, id FROM flights WHERE miles > 2000.0
         ORDER BY carrier, miles DESC",
    );
    let rows = lines(by_number.unwrap());
    assert_eq!(rows, lines(by_name.unwrap()));
    assert_eq!(rows.len(), 16);
    for sql in [
        "SELECT carrier FROM flights ORDER BY 2",
        "SELECT carrier FROM flights ORDER BY 0",
        "SELECT * FROM flights ORDER BY 1",
        "DELETE FROM flights",
    ] {
        assert!(query_by_position(&mut db, sql).is_err(), "{sql}");
    }
}
