//! The examples in Chapter 4: `cargo test --test book_ch04`.
use felite::{INTERIOR, LEAF, MemFile, Page, PageNo, Pager, Tree};
use felite::{interior_cell, leaf_cell, parse_interior, parse_leaf, row_id, row_key};

// @listing helpers
/// A leaf page holding the given keys and payloads, which must be in key order.
fn leaf(entries: &[(&[u8], &[u8])]) -> Page {
    let mut page = Page::new();
    page.init_slotted(LEAF);
    for (i, (key, payload)) in entries.iter().enumerate() {
        assert!(page.insert_cell(i, &leaf_cell(key, payload)).unwrap());
    }
    page
}

/// An interior page holding the given children and their lowest keys.
fn interior(entries: &[(PageNo, &[u8])]) -> Page {
    let mut page = Page::new();
    page.init_slotted(INTERIOR);
    for (i, (child, key)) in entries.iter().enumerate() {
        assert!(page.insert_cell(i, &interior_cell(*child, key)).unwrap());
    }
    page
}
// @end

// @listing airports
/// Nine airports in three leaves under one root. Returns the tree.
fn airports(pager: &mut Pager) -> Tree {
    let first =
        leaf(&[(b"ATL", b"Atlanta"), (b"BOS", b"Boston"), (b"DEN", b"Denver")]);
    let second = leaf(&[
        (b"JFK", b"New York"),
        (b"LAX", b"Los Angeles"),
        (b"ORD", b"Chicago"),
    ]);
    let third = leaf(&[
        (b"SEA", b"Seattle"),
        (b"SFO", b"San Francisco"),
        (b"SLC", b"Salt Lake City"),
    ]);
    assert_eq!(pager.allocate(first).unwrap(), 1);
    assert_eq!(pager.allocate(second).unwrap(), 2);
    assert_eq!(pager.allocate(third).unwrap(), 3);
    let root = interior(&[(1, b"ATL"), (2, b"JFK"), (3, b"SEA")]);
    Tree::open(pager.allocate(root).unwrap())
}
// @end

// @listing pack
/// Packs cells into as many pages of one kind as they need. Each cell comes with
/// its key. Returns the lowest key and the page number of every page made.
fn pack(
    pager: &mut Pager,
    kind: u8,
    cells: &[(Vec<u8>, Vec<u8>)],
) -> Vec<(Vec<u8>, PageNo)> {
    let mut made = Vec::new();
    let mut page = Page::new();
    page.init_slotted(kind);
    let mut lowest = &cells[0].0;
    for (key, cell) in cells {
        if !page.insert_cell(page.cell_count(), cell).unwrap() {
            made.push((lowest.clone(), pager.allocate(page).unwrap()));
            page = Page::new();
            page.init_slotted(kind);
            assert!(page.insert_cell(0, cell).unwrap());
            lowest = key;
        }
    }
    made.push((lowest.clone(), pager.allocate(page).unwrap()));
    made
}
// @end

// @listing build
/// A tree of rows numbered 0 to `rows - 1`, each with a payload of `size` bytes,
/// built from the leaves up. Returns the tree and the number of leaves.
fn build(pager: &mut Pager, rows: i64, size: usize) -> (Tree, usize) {
    let cells: Vec<(Vec<u8>, Vec<u8>)> = (0..rows)
        .map(|id| {
            (row_key(id).to_vec(), leaf_cell(&row_key(id), &vec![id as u8; size]))
        })
        .collect();
    let mut level = pack(pager, LEAF, &cells);
    let leaves = level.len();
    while level.len() > 1 {
        let cells: Vec<(Vec<u8>, Vec<u8>)> = level
            .iter()
            .map(|(key, child)| (key.clone(), interior_cell(*child, key)))
            .collect();
        level = pack(pager, INTERIOR, &cells);
    }
    (Tree::open(level[0].1), leaves)
}
// @end

#[test]
fn cells_hold_keys() {
    // @listing cells-test
    let cell = leaf_cell(b"SLC", b"Salt Lake City");
    assert_eq!(cell, b"\x03SLCSalt Lake City");
    assert_eq!(parse_leaf(&cell).unwrap(), (&b"SLC"[..], &b"Salt Lake City"[..]));

    let cell = interior_cell(258, b"SEA");
    assert_eq!(cell, [0, 0, 1, 2, b'S', b'E', b'A']);
    assert_eq!(parse_interior(&cell).unwrap(), (258, &b"SEA"[..]));

    // A key length that runs past the end of the cell, and a cell too short to
    // hold a page number.
    assert!(parse_leaf(b"\x09SLC").is_err());
    assert!(parse_interior(&[0, 0, 1]).is_err());
    // @end
}

#[test]
fn looking_up_airports() {
    // @listing airports-test
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = airports(&mut pager);
    assert_eq!(tree.root(), 4);
    assert_eq!(tree.height(&mut pager).unwrap(), 2);

    let get = |pager: &mut Pager, key: &[u8]| tree.get(pager, key).unwrap();
    assert_eq!(get(&mut pager, b"DEN").unwrap(), b"Denver");
    assert_eq!(get(&mut pager, b"JFK").unwrap(), b"New York");
    assert_eq!(get(&mut pager, b"SLC").unwrap(), b"Salt Lake City");
    assert_eq!(get(&mut pager, b"MIA"), None); // would be in the second leaf
    assert_eq!(get(&mut pager, b"AAA"), None); // before every key
    assert_eq!(get(&mut pager, b"ZZZ"), None); // after every key
    assert_eq!(get(&mut pager, b"SL"), None); // a prefix of a key is another key
    // @end
}

#[test]
fn the_first_key_is_never_compared() {
    // @listing first-key
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = airports(&mut pager);
    // Replace the root with one whose first key is nonsense.
    *pager.page_mut(4).unwrap() =
        interior(&[(1, b"ZZZZZZ"), (2, b"JFK"), (3, b"SEA")]);
    for (key, city) in [(b"ATL", "Atlanta"), (b"DEN", "Denver"), (b"ORD", "Chicago")]
    {
        assert_eq!(tree.get(&mut pager, key).unwrap().unwrap(), city.as_bytes());
    }
    assert_eq!(tree.get(&mut pager, b"AAA").unwrap(), None);
    // @end
}

#[test]
fn row_ids_sort_as_bytes() {
    // @listing row-key-test
    assert_eq!(row_key(0), [0x80, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(row_key(1), [0x80, 0, 0, 0, 0, 0, 0, 1]);
    assert_eq!(row_key(-1), [0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    assert_eq!(row_id(&row_key(-12345)).unwrap(), -12345);
    assert!(row_id(b"SLC").is_err());

    let ids = [i64::MIN, -256, -1, 0, 1, 255, 256, 65_536, i64::MAX];
    for pair in ids.windows(2) {
        assert!(row_key(pair[0]) < row_key(pair[1]));
    }
    // Little-endian bytes would not do: 256 would sort before 1.
    assert!(256i64.to_le_bytes() < 1i64.to_le_bytes());
    // @end
}

#[test]
fn what_fits_in_a_page() {
    // @listing fan-out
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    // A row id and a 40-byte payload: a 49-byte cell and a 4-byte slot.
    assert_eq!(leaf_cell(&row_key(7), &[0; 40]).len(), 49);
    let (_, leaves) = build(&mut pager, 77 * 3, 40);
    assert_eq!(leaves, 3); // 77 rows to a leaf: 4,084 / 53, rounded down
    assert_eq!(pager.page(1).unwrap().cell_count(), 77);

    // A page number and a row id: a 12-byte cell and a 4-byte slot.
    assert_eq!(interior_cell(1, &row_key(7)).len(), 12);
    assert_eq!(4084 / 16, 255); // children of one interior page

    // Rows within reach of a tree of each height.
    assert_eq!(77 * 255, 19_635);
    assert_eq!(77 * 255 * 255, 5_006_925);
    assert_eq!(77u64 * 255 * 255 * 255, 1_276_765_875);
    // @end
}

#[test]
fn a_hundred_thousand_rows() {
    // @listing big-tree
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let (tree, leaves) = build(&mut pager, 100_000, 40);
    assert_eq!(leaves, 1299);
    // The header, the leaves, six interior pages and the root.
    assert_eq!(pager.page_count().unwrap(), 1 + 1299 + 6 + 1);
    assert_eq!(tree.height(&mut pager).unwrap(), 3);

    for id in [0, 1, 76, 77, 50_000, 99_999] {
        let payload = tree.get(&mut pager, &row_key(id)).unwrap().unwrap();
        assert_eq!(payload, vec![id as u8; 40]);
    }
    assert_eq!(tree.get(&mut pager, &row_key(-1)).unwrap(), None);
    assert_eq!(tree.get(&mut pager, &row_key(100_000)).unwrap(), None);
    // @end
}

#[test]
fn three_pages_or_six_hundred_and_fifty() {
    // @listing pages-read
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    let (tree, leaves) = build(&mut pager, 100_000, 40);
    pager.flush().unwrap();
    let target = row_key(50_000);

    // A fresh pager has read only the header. Scan the leaves in order for the row.
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    assert_eq!(pager.pages_in_memory(), 1);
    let mut found = false;
    for n in 1..=leaves as PageNo {
        let page = pager.page(n).unwrap();
        for i in 0..page.cell_count() {
            found |= parse_leaf(page.cell(i).unwrap()).unwrap().0 == target;
        }
        if found {
            break;
        }
    }
    assert_eq!(pager.pages_in_memory() - 1, 650);

    // Another fresh pager, and the tree.
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    assert!(tree.get(&mut pager, &target).unwrap().is_some());
    assert_eq!(pager.pages_in_memory() - 1, 3);
    // A neighboring row is in the same leaf, and a distant one shares only the root.
    tree.get(&mut pager, &row_key(50_001)).unwrap();
    assert_eq!(pager.pages_in_memory() - 1, 3);
    tree.get(&mut pager, &row_key(5)).unwrap();
    assert_eq!(pager.pages_in_memory() - 1, 5);
    // @end
}

#[test]
fn a_damaged_tree_is_an_error() {
    // @listing damaged-tree
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = airports(&mut pager);
    let message =
        |pager: &mut Pager| tree.get(pager, b"DEN").unwrap_err().to_string();

    // The root's first child is the root itself.
    *pager.page_mut(4).unwrap() = interior(&[(4, b"ATL"), (2, b"JFK"), (3, b"SEA")]);
    assert_eq!(
        message(&mut pager),
        "corrupt database: a tree more than 40 levels deep"
    );

    // The first child is the file's header page.
    *pager.page_mut(4).unwrap() = interior(&[(0, b"ATL"), (2, b"JFK"), (3, b"SEA")]);
    assert_eq!(
        message(&mut pager),
        "corrupt database: page 0 has kind 102 and is not part of a tree"
    );

    // The first child is a page the file does not have.
    *pager.page_mut(4).unwrap() = interior(&[(9, b"ATL"), (2, b"JFK"), (3, b"SEA")]);
    assert_eq!(
        message(&mut pager),
        "corrupt database: page 9 is past the end of a 5-page file"
    );

    // An interior page with no cells.
    *pager.page_mut(4).unwrap() = interior(&[]);
    assert_eq!(
        message(&mut pager),
        "corrupt database: an interior page with no cells"
    );
    // @end
}
