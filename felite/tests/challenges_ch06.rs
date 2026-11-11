//! Tests for the Chapter 6 challenges: `cargo test --test challenges_ch06`.
use felite::{Cursor, MemFile, Pager, Tree, row_id, row_key};
use felite::{ReverseCursor, StableCursor, count_range, prefix_end, prefix_scan};

fn new_tree() -> (Pager, Tree) {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    (pager, tree)
}

fn add(tree: &Tree, pager: &mut Pager, id: i64, size: usize) {
    tree.insert(pager, &row_key(id), &vec![id as u8; size]).unwrap();
}

/// 100,000 rows of 40 bytes with ids from 0, flushed. 77 rows to a leaf.
fn big_table() -> (MemFile, Tree) {
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    for id in 0..100_000 {
        add(&tree, &mut pager, id, 40);
    }
    pager.flush().unwrap();
    (file, tree)
}

#[test]
fn predict_pages_read() {
    let (file, tree) = big_table();
    let pages = |low: i64, high: i64| {
        let mut pager = Pager::open(Box::new(file.clone())).unwrap();
        let rows = tree.range(&mut pager, &row_key(low), &row_key(high)).unwrap();
        (rows.len(), pager.pages_in_memory() - 1)
    };
    assert_eq!(pages(0, 77), (77, 2 + 2)); // one leaf of rows, and a look at the next
    assert_eq!(pages(0, 76), (76, 2 + 1));
    assert_eq!(pages(100, 101), (1, 2 + 1));
    assert_eq!(pages(76, 78), (2, 2 + 2));
    assert_eq!(pages(50_000, 50_000), (0, 2 + 1));
    assert_eq!(pages(0, 10_000), (10_000, 2 + 130));
    // Leaves 254 and 255 are under different interior pages, but the scan goes
    // from one to the other by the link, so only one interior page is read.
    assert_eq!(pages(255 * 77 - 1, 255 * 77 + 1), (2, 2 + 2));
}

#[test]
fn counting_a_range() {
    let (file, tree) = big_table();
    let mut pager = Pager::open(Box::new(file)).unwrap();
    let mut count = |low: i64, high: i64| {
        count_range(&tree, &mut pager, &row_key(low), &row_key(high)).unwrap()
    };
    assert_eq!(count(0, 100_000), 100_000);
    assert_eq!(count(-10, 5), 5);
    assert_eq!(count(1234, 5678), 4444);
    assert_eq!(count(99_990, 200_000), 10);
    assert_eq!(count(500, 500), 0);
    assert_eq!(count(500, 400), 0); // a range that runs backward is empty
    assert_eq!(count(200_000, 300_000), 0);
    assert_eq!(count(76, 78), 2);
}

#[test]
fn a_cursor_that_survives_changes() {
    let (mut pager, tree) = new_tree();
    for id in (0..2000).step_by(2) {
        add(&tree, &mut pager, id, 40);
    }
    let mut cursor = StableCursor::new();
    let mut seen = Vec::new();
    let mut odd = 1;
    while let Some((key, _)) = cursor.next(&tree, &mut pager).unwrap() {
        let id = row_id(&key).unwrap();
        seen.push(id);
        // After every row, insert one behind the cursor. Leaves split under it.
        if id % 2 == 0 && odd < id {
            add(&tree, &mut pager, odd, 40);
            odd += 2;
        }
    }
    let evens: Vec<i64> = (0..2000).step_by(2).collect();
    assert_eq!(seen, evens); // every original row once, and none of the new ones
    tree.check(&mut pager).unwrap();

    // The plain cursor, under the same treatment, repeats itself.
    let (mut pager, tree) = new_tree();
    for id in [10, 20, 30] {
        add(&tree, &mut pager, id, 40);
    }
    let mut stable = StableCursor::new();
    let mut plain = Cursor::first(&tree, &mut pager).unwrap();
    stable.next(&tree, &mut pager).unwrap();
    plain.next(&mut pager).unwrap();
    add(&tree, &mut pager, 5, 40);
    let id = |entry: Option<(Vec<u8>, Vec<u8>)>| row_id(&entry.unwrap().0).unwrap();
    assert_eq!(id(plain.next(&mut pager).unwrap()), 10);
    assert_eq!(id(stable.next(&tree, &mut pager).unwrap()), 20);
    // A row inserted ahead of the cursor is seen.
    add(&tree, &mut pager, 25, 40);
    assert_eq!(id(stable.next(&tree, &mut pager).unwrap()), 25);
}

#[test]
fn scanning_by_prefix() {
    assert_eq!(prefix_end(b"SL").unwrap(), b"SM");
    assert_eq!(prefix_end(b"S\xff").unwrap(), b"T");
    assert_eq!(prefix_end(b"\xff\xff"), None);
    assert_eq!(prefix_end(b""), None);

    let (mut pager, tree) = new_tree();
    for code in ["ATL", "SAN", "SEA", "SFO", "SJC", "SLC", "SMF", "TPA", "S", "SL"] {
        tree.insert(&mut pager, code.as_bytes(), b"").unwrap();
    }
    let mut keys = |prefix: &[u8]| -> Vec<String> {
        let entries = prefix_scan(&tree, &mut pager, prefix).unwrap();
        entries.into_iter().map(|(key, _)| String::from_utf8(key).unwrap()).collect()
    };
    assert_eq!(keys(b"S"), ["S", "SAN", "SEA", "SFO", "SJC", "SL", "SLC", "SMF"]);
    assert_eq!(keys(b"SL"), ["SL", "SLC"]);
    assert_eq!(keys(b"SLC"), ["SLC"]);
    assert_eq!(keys(b"X"), Vec::<String>::new());
    assert_eq!(keys(b"").len(), 10);
}

#[test]
fn reading_backward() {
    let (mut pager, tree) = new_tree();
    let mut cursor = ReverseCursor::last(&tree, &mut pager).unwrap();
    assert_eq!(cursor.prev(&mut pager).unwrap(), None);

    for id in 0..30_000 {
        add(&tree, &mut pager, id * 7919 % 30_000, 40); // every id once, shuffled
    }
    assert_eq!(tree.check(&mut pager).unwrap().height, 3);
    let mut cursor = ReverseCursor::last(&tree, &mut pager).unwrap();
    let mut expected = 29_999;
    while let Some((key, payload)) = cursor.prev(&mut pager).unwrap() {
        assert_eq!(row_id(&key).unwrap(), expected);
        assert_eq!(payload, vec![expected as u8; 40]);
        expected -= 1;
    }
    assert_eq!(expected, -1);
    assert_eq!(cursor.prev(&mut pager).unwrap(), None);
}
