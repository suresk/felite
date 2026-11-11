//! Tests for the Chapter 4 challenges: `cargo test --test challenges_ch04`.
use felite::{
    INTERIOR, LEAF, MemFile, Page, PageNo, Pager, Tree, interior_cell, leaf_cell,
};
use felite::{contains, first_key, last_key, path, row_key, separator};

fn leaf(entries: &[(&[u8], &[u8])]) -> Page {
    let mut page = Page::new();
    page.init_slotted(LEAF);
    for (i, (key, payload)) in entries.iter().enumerate() {
        assert!(page.insert_cell(i, &leaf_cell(key, payload)).unwrap());
    }
    page
}

fn interior(entries: &[(PageNo, &[u8])]) -> Page {
    let mut page = Page::new();
    page.init_slotted(INTERIOR);
    for (i, (child, key)) in entries.iter().enumerate() {
        assert!(page.insert_cell(i, &interior_cell(*child, key)).unwrap());
    }
    page
}

/// The nine airports of Section 4.5: leaves on pages 1 to 3, the root on page 4.
fn airports(pager: &mut Pager) -> Tree {
    pager
        .allocate(leaf(&[
            (b"ATL", b"Atlanta"),
            (b"BOS", b"Boston"),
            (b"DEN", b"Denver"),
        ]))
        .unwrap();
    pager
        .allocate(leaf(&[
            (b"JFK", b"New York"),
            (b"LAX", b"LA"),
            (b"ORD", b"Chicago"),
        ]))
        .unwrap();
    pager
        .allocate(leaf(&[
            (b"SEA", b"Seattle"),
            (b"SFO", b"SF"),
            (b"SLC", b"Salt Lake"),
        ]))
        .unwrap();
    let root = interior(&[(1, b"ATL"), (2, b"JFK"), (3, b"SEA")]);
    Tree::open(pager.allocate(root).unwrap())
}

#[test]
fn predict_paths_and_heights() {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = airports(&mut pager);
    // (page, cell chosen) at the root and then at the leaf.
    assert_eq!(path(&tree, &mut pager, b"LAX").unwrap(), [(4, 1), (2, 1)]);
    assert_eq!(path(&tree, &mut pager, b"MIA").unwrap(), [(4, 1), (2, 2)]);
    assert_eq!(path(&tree, &mut pager, b"AAA").unwrap(), [(4, 0), (1, 0)]);
    assert_eq!(path(&tree, &mut pager, b"SEA").unwrap(), [(4, 2), (3, 0)]);
    assert_eq!(path(&tree, &mut pager, b"RDU").unwrap(), [(4, 1), (2, 3)]);
    assert_eq!(path(&tree, &mut pager, b"ZZZ").unwrap(), [(4, 2), (3, 3)]);

    // Rows with a 200-byte payload: 19 to a leaf.
    assert_eq!(4084 / (1 + 8 + 200 + 4), 19);
    assert_eq!(19 * 255, 4_845); // within reach at height 2
    assert_eq!(19 * 255 * 255, 1_235_475); // and at height 3
    // Interior cells with 30-byte keys: 107 children to a page.
    assert_eq!(4084 / (4 + 30 + 4), 107);
}

#[test]
fn contains_copies_nothing() {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = airports(&mut pager);
    for key in [b"ATL", b"DEN", b"JFK", b"ORD", b"SLC"] {
        assert!(contains(&tree, &mut pager, key).unwrap());
    }
    for key in [&b"AAA"[..], b"MIA", b"RDU", b"ZZZ", b"SL", b"SLCC"] {
        assert!(!contains(&tree, &mut pager, key).unwrap());
    }
}

#[test]
fn the_ends_of_a_tree() {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = airports(&mut pager);
    assert_eq!(first_key(&tree, &mut pager).unwrap().unwrap(), b"ATL");
    assert_eq!(last_key(&tree, &mut pager).unwrap().unwrap(), b"SLC");

    let empty = Tree::open(pager.allocate(leaf(&[])).unwrap());
    assert_eq!(first_key(&empty, &mut pager).unwrap(), None);
    assert_eq!(last_key(&empty, &mut pager).unwrap(), None);

    let one = Tree::open(pager.allocate(leaf(&[(&row_key(42), b"x")])).unwrap());
    assert_eq!(first_key(&one, &mut pager).unwrap().unwrap(), row_key(42));
    assert_eq!(last_key(&one, &mut pager).unwrap().unwrap(), row_key(42));
}

#[test]
fn short_separators() {
    assert_eq!(separator(b"DEN", b"JFK"), b"J");
    assert_eq!(separator(b"SEA", b"SFO"), b"SF");
    assert_eq!(separator(b"SL", b"SLC"), b"SLC");
    assert_eq!(separator(b"Salt Lake City", b"San Francisco"), b"San");
    // Row ids next to each other differ only in the last byte, so nothing is saved.
    assert_eq!(separator(&row_key(76), &row_key(77)), row_key(77));
    for (left, right) in
        [(&b"DEN"[..], &b"JFK"[..]), (b"SEA", b"SFO"), (b"SL", b"SLC")]
    {
        let s = separator(left, right);
        assert!(left < &s[..] && &s[..] <= right);
    }

    // The airports again, with separators in the root in place of whole keys.
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = airports(&mut pager);
    *pager.page_mut(4).unwrap() = interior(&[(1, b""), (2, b"J"), (3, b"S")]);
    for key in [b"ATL", b"DEN", b"JFK", b"ORD", b"SEA", b"SLC"] {
        assert!(contains(&tree, &mut pager, key).unwrap());
    }
    assert!(!contains(&tree, &mut pager, b"MIA").unwrap());
}
