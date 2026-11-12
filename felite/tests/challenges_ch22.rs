//! Tests for the Chapter 22 challenges: `cargo test --test challenges_ch22`.
use felite::{Database, MemFile, Output, Row};
use felite::{merge_join, partitioned_join};

fn fixture() -> Database {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    for statement in include_str!("../fixtures/flights.sql").lines() {
        db.execute(statement).unwrap();
    }
    db
}

fn query(db: &mut Database, sql: &str) -> Vec<String> {
    let Output::Rows(result) = db.execute(sql).unwrap() else {
        panic!("not a query")
    };
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

type List = Vec<(i64, String)>;

/// Two lists of (key, label) with repeated keys, sorted by key.
fn two_lists() -> (List, List) {
    let mut state = 22;
    let mut list = |count: usize, tag: &str| {
        let mut entries: Vec<(i64, String)> = (0..count)
            .map(|i| ((next(&mut state) % 60) as i64 - 10, format!("{tag}{i}")))
            .collect();
        entries.sort();
        entries
    };
    (list(200, "a"), list(120, "b"))
}

/// The join by definition: every pair, tested.
fn every_pair(
    left: &[(i64, String)],
    right: &[(i64, String)],
) -> Vec<(i64, String, String)> {
    let mut joined = Vec::new();
    for (k, a) in left {
        for (j, b) in right {
            if k == j {
                joined.push((*k, a.clone(), b.clone()));
            }
        }
    }
    joined
}

#[test]
fn predict_a_join() {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute("CREATE TABLE x (k INTEGER)").unwrap();
    db.execute("CREATE TABLE y (k INTEGER)").unwrap();
    for k in ["1", "2", "2", "NULL"] {
        db.execute(&format!("INSERT INTO x VALUES ({k})")).unwrap();
    }
    for k in ["2", "2", "3", "NULL"] {
        db.execute(&format!("INSERT INTO y VALUES ({k})")).unwrap();
    }
    let mut count = |sql: &str| query(&mut db, sql).len();
    assert_eq!(count("SELECT * FROM x JOIN y ON x.k = y.k"), 4);
    assert_eq!(count("SELECT * FROM x LEFT JOIN y ON x.k = y.k"), 6);
    assert_eq!(count("SELECT * FROM y LEFT JOIN x ON x.k = y.k"), 6);
    assert_eq!(count("SELECT * FROM x JOIN y ON x.k < y.k"), 5);
    assert_eq!(
        count("SELECT * FROM x LEFT JOIN y ON x.k = y.k WHERE y.k IS NULL"),
        2
    );
}

#[test]
fn every_pair_of_rows() {
    let mut db = fixture();
    assert_eq!(
        query(&mut db, "SELECT COUNT(*) FROM carriers c JOIN airports a ON 1 = 1"),
        ["5010"]
    );
}

#[test]
fn rows_with_no_match() {
    let mut db = fixture();
    // Airports that no flight of the sample arrived at.
    assert_eq!(
        query(
            &mut db,
            "SELECT COUNT(*) FROM airports a LEFT JOIN flights f ON f.dest = a.code
             WHERE f.id IS NULL"
        ),
        ["256"]
    );
    // Carriers with no cancelled flight.
    assert_eq!(
        query(
            &mut db,
            "SELECT c.code FROM carriers c LEFT JOIN flights f
             ON f.carrier = c.code AND f.cancelled = 1 WHERE f.id IS NULL"
        ),
        ["9E", "B6", "DL", "F9", "G4", "HA", "MQ", "NK", "WN", "YX"]
    );
}

#[test]
fn joining_sorted_inputs() {
    let (left, right) = two_lists();
    let joined = merge_join(&left, &right);
    assert_eq!(joined, every_pair(&left, &right));
    assert_eq!(joined.len(), 418);
    assert!(merge_join(&left, &[] as &[(i64, String)]).is_empty());
    assert!(merge_join(&[] as &[(i64, String)], &right).is_empty());
    let ones = vec![(1, 'a'), (1, 'b')];
    assert_eq!(merge_join(&ones, &ones).len(), 4);
}

#[test]
fn a_hash_join_in_parts() {
    let (left, right) = two_lists();
    let mut expected = every_pair(&left, &right);
    expected.sort();
    for partitions in [1, 2, 7, 16, 1000] {
        let (mut joined, largest) = partitioned_join(&left, &right, partitions);
        joined.sort();
        assert_eq!(joined, expected);
        if partitions == 1 {
            assert_eq!(largest, 120);
        }
        if partitions == 7 {
            assert!(largest < 40, "{largest}");
        }
    }
}
