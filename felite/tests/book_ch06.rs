//! The examples in Chapter 6: `cargo test --test book_ch06`.
use felite::{Cursor, Error, MemFile, Pager, Tree, row_id, row_key};
use std::collections::BTreeMap;

/// A new database in memory with one empty tree in it.
fn new_tree() -> (Pager, Tree) {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    (pager, tree)
}

/// Inserts a row whose payload is `size` copies of the low byte of its id.
fn add(tree: &Tree, pager: &mut Pager, id: i64, size: usize) {
    tree.insert(pager, &row_key(id), &vec![id as u8; size]).unwrap();
}

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// @listing ids
/// The row ids a cursor returns from where it stands to the end of the tree.
fn ids_from(mut cursor: Cursor, pager: &mut Pager) -> Vec<i64> {
    let mut ids = Vec::new();
    while let Some((key, _)) = cursor.next(pager).unwrap() {
        ids.push(row_id(&key).unwrap());
    }
    ids
}
// @end

#[test]
fn a_scan_in_key_order() {
    // @listing first-scan
    let (mut pager, tree) = new_tree();
    for (code, city) in
        [("SLC", "Salt Lake City"), ("ATL", "Atlanta"), ("DEN", "Denver")]
    {
        tree.insert(&mut pager, code.as_bytes(), city.as_bytes()).unwrap();
    }
    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    let mut seen = Vec::new();
    while let Some((key, payload)) = cursor.next(&mut pager).unwrap() {
        seen.push((
            String::from_utf8(key).unwrap(),
            String::from_utf8(payload).unwrap(),
        ));
    }
    let pairs: Vec<(&str, &str)> =
        seen.iter().map(|(k, p)| (k.as_str(), p.as_str())).collect();
    assert_eq!(
        pairs,
        [("ATL", "Atlanta"), ("DEN", "Denver"), ("SLC", "Salt Lake City")]
    );
    // At the end, the cursor goes on answering None.
    assert_eq!(cursor.next(&mut pager).unwrap(), None);

    // An empty tree has nothing to return.
    let (mut pager, tree) = new_tree();
    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    assert_eq!(cursor.next(&mut pager).unwrap(), None);
    // @end
}

#[test]
fn a_scan_across_many_leaves() {
    // @listing long-scan
    let (mut pager, tree) = new_tree();
    let mut model = BTreeMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..20_000 {
        let id = (next(&mut state) >> 1) as i64;
        add(&tree, &mut pager, id, 40);
        model.insert(id, vec![id as u8; 40]);
    }
    assert_eq!(tree.check(&mut pager).unwrap().leaves, 371);

    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    let mut expected = model.iter();
    while let Some((key, payload)) = cursor.next(&mut pager).unwrap() {
        let (id, model_payload) = expected.next().unwrap();
        assert_eq!((row_id(&key).unwrap(), &payload), (*id, model_payload));
    }
    assert!(expected.next().is_none()); // the cursor returned every row
    // @end
}

#[test]
fn seeking() {
    // @listing seek-test
    let (mut pager, tree) = new_tree();
    for id in (10..=1000).step_by(10) {
        add(&tree, &mut pager, id, 40); // 10, 20, …, 1000, on two leaves
    }
    let mut from = |id: i64| {
        let mut cursor = Cursor::seek(&tree, &mut pager, &row_key(id)).unwrap();
        let entry = cursor.next(&mut pager).unwrap();
        entry.map(|(key, _)| row_id(&key).unwrap())
    };
    assert_eq!(from(500), Some(500)); // a key that is there
    assert_eq!(from(501), Some(510)); // one that is not: the next key up
    assert_eq!(from(-7), Some(10)); // before the first key
    assert_eq!(from(770), Some(770)); // the last key on the first leaf
    assert_eq!(from(771), Some(780)); // past it: the first key on the second
    assert_eq!(from(1000), Some(1000));
    assert_eq!(from(1001), None); // past the last key
    // @end
}

#[test]
fn range_scans() {
    // @listing range-test
    let (mut pager, tree) = new_tree();
    let mut model = BTreeMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..5000 {
        let id = (next(&mut state) % 100_000) as i64;
        if model.insert(id, ()).is_none() {
            add(&tree, &mut pager, id, 40);
        }
    }
    let mut check = |low: i64, high: i64| {
        let entries = tree.range(&mut pager, &row_key(low), &row_key(high)).unwrap();
        let got: Vec<i64> =
            entries.iter().map(|(key, _)| row_id(key).unwrap()).collect();
        let expected: Vec<i64> = model.range(low..high).map(|(id, _)| *id).collect();
        assert_eq!(got, expected);
        got.len()
    };
    assert_eq!(check(20_000, 30_000), 475);
    assert_eq!(check(-50, 1000), 58); // starts before the first key
    assert_eq!(check(99_000, 200_000), 63); // ends after the last
    assert_eq!(check(0, 100_000), model.len()); // everything
    assert_eq!(check(40_000, 40_000), 0); // an empty range
    assert_eq!(check(200_000, 300_000), 0); // a range past the end
    for _ in 0..200 {
        let low = (next(&mut state) % 100_000) as i64;
        check(low, low + (next(&mut state) % 3000) as i64);
    }
    // @end
}

#[test]
fn pages_read_by_a_scan() {
    // @listing scan-pages
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    for id in 0..100_000 {
        add(&tree, &mut pager, id, 40); // 77 rows to a leaf, height 3
    }
    pager.flush().unwrap();

    // Rows 7,700 to 8,699. Leaves hold 77 rows, so that is leaves 100 to 112: 13
    // of them, the last short by one row.
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let rows = tree.range(&mut pager, &row_key(7700), &row_key(8700)).unwrap();
    assert_eq!(rows.len(), 1000);
    // The root and one interior page to get there, then the 13 leaves.
    assert_eq!(pager.pages_in_memory() - 1, 2 + 13);

    // The whole table: every leaf once, and one interior page on the way down.
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    let mut count = 0;
    while cursor.next(&mut pager).unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 100_000);
    assert_eq!(pager.pages_in_memory() - 1, 2 + 1299);
    // @end
}

#[test]
fn the_next_row_id() {
    // @listing row-ids
    let (mut pager, tree) = new_tree();
    assert_eq!(tree.last_key(&mut pager).unwrap(), None);
    assert_eq!(tree.next_row_id(&mut pager).unwrap(), 1);

    for _ in 0..500 {
        let id = tree.next_row_id(&mut pager).unwrap();
        add(&tree, &mut pager, id, 40);
    }
    assert_eq!(tree.last_key(&mut pager).unwrap().unwrap(), row_key(500));
    assert_eq!(
        ids_from(Cursor::first(&tree, &mut pager).unwrap(), &mut pager).len(),
        500
    );

    // A row id chosen by hand moves the high-water mark.
    add(&tree, &mut pager, 9000, 40);
    assert_eq!(tree.next_row_id(&mut pager).unwrap(), 9001);

    add(&tree, &mut pager, i64::MAX, 40);
    let error = tree.next_row_id(&mut pager).unwrap_err();
    assert!(matches!(error, Error::OutOfRowIds));
    // @end
}

#[test]
fn a_tree_that_changes_under_a_cursor() {
    // @listing moving-tree
    let (mut pager, tree) = new_tree();
    for id in [10, 20, 30] {
        add(&tree, &mut pager, id, 40);
    }
    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    let read = |cursor: &mut Cursor, pager: &mut Pager| {
        cursor.next(pager).unwrap().map(|(key, _)| row_id(&key).unwrap())
    };
    assert_eq!(read(&mut cursor, &mut pager), Some(10));

    // A row inserted behind the cursor pushes the cells along by one.
    add(&tree, &mut pager, 5, 40);
    assert_eq!(read(&mut cursor, &mut pager), Some(10)); // row 10, a second time
    assert_eq!(read(&mut cursor, &mut pager), Some(20));
    // @end
}

#[test]
fn leaves_in_a_circle() {
    // @listing circle
    let (mut pager, tree) = new_tree();
    for id in 0..200 {
        add(&tree, &mut pager, id, 40); // three leaves: pages 3, 2 and 4
    }
    pager.page_mut(4).unwrap().set_right(3); // the last leaf points back at the first

    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    let mut rows = 0;
    let error = loop {
        match cursor.next(&mut pager) {
            Ok(Some(_)) => rows += 1,
            Ok(None) => panic!("a circle has no end"),
            Err(error) => break error,
        }
    };
    assert_eq!(error.to_string(), "corrupt database: the leaves link in a circle");
    // The file has five pages, so the cursor gave up on its sixth step to a new
    // leaf, having read the 200 rows twice over.
    assert_eq!(rows, 400);
    // @end
}
