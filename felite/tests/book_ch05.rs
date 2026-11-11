//! The examples in Chapter 5: `cargo test --test book_ch05`.
use felite::{Error, INTERIOR, LEAF, MAX_CELL, MemFile, Page, Pager, Tree, TreeInfo};
use felite::{interior_cell, leaf_cell, parse_interior, parse_leaf, row_key};
use std::collections::BTreeMap;

// @listing helpers
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
// @end

#[test]
fn inserting_where_there_is_room() {
    // @listing first-inserts
    let (mut pager, tree) = new_tree();
    assert_eq!(tree.root(), 1);
    assert_eq!(tree.get(&mut pager, b"DEN").unwrap(), None);

    // In no particular order.
    for (code, city) in
        [("SLC", "Salt Lake City"), ("ATL", "Atlanta"), ("DEN", "Denver")]
    {
        tree.insert(&mut pager, code.as_bytes(), city.as_bytes()).unwrap();
    }
    assert_eq!(tree.get(&mut pager, b"DEN").unwrap().unwrap(), b"Denver");

    // The leaf keeps them sorted.
    let page = pager.page(1).unwrap();
    let keys: Vec<&[u8]> =
        (0..3).map(|i| parse_leaf(page.cell(i).unwrap()).unwrap().0).collect();
    assert_eq!(keys, [b"ATL", b"DEN", b"SLC"]);
    // @end
}

#[test]
fn what_insert_refuses() {
    // @listing refusals
    let (mut pager, tree) = new_tree();
    tree.insert(&mut pager, b"DEN", b"Denver").unwrap();
    let error = tree.insert(&mut pager, b"DEN", b"Denton").unwrap_err();
    assert!(matches!(error, Error::DuplicateKey));
    assert_eq!(error.to_string(), "that key is already present");
    assert_eq!(tree.get(&mut pager, b"DEN").unwrap().unwrap(), b"Denver");

    // A row id key is 8 bytes and its length takes 1, so 991 bytes of payload make
    // a cell of exactly 1,000.
    assert_eq!(MAX_CELL, 1000);
    tree.insert(&mut pager, &row_key(1), &[0; 991]).unwrap();
    let error = tree.insert(&mut pager, &row_key(2), &[0; 992]).unwrap_err();
    assert!(matches!(error, Error::TooLarge(1001)));
    assert_eq!(error.to_string(), "an entry of 1001 bytes is too large");
    // @end
}

#[test]
fn the_first_split() {
    // @listing first-split
    let (mut pager, tree) = new_tree();
    // The even ids from 0 to 152: 77 rows, which fill one leaf exactly.
    for id in (0..154).step_by(2) {
        add(&tree, &mut pager, id, 40);
    }
    assert_eq!(pager.page_count().unwrap(), 2); // the header and the root
    assert_eq!(pager.page(1).unwrap().kind(), LEAF);
    assert!(pager.page(1).unwrap().free_space().unwrap() < 53);

    add(&tree, &mut pager, 75, 40); // belongs in the middle, and there is no room

    assert_eq!(pager.page_count().unwrap(), 4);
    let root = pager.page(1).unwrap();
    assert_eq!((root.kind(), root.cell_count()), (INTERIOR, 2));
    // The root's children: page 3 with everything from the start, page 2 from 76.
    assert_eq!(root.cell(0).unwrap(), interior_cell(3, &row_key(0)));
    assert_eq!(root.cell(1).unwrap(), interior_cell(2, &row_key(76)));
    // The 78 rows are divided evenly, and the left leaf points at the right one.
    let (left, right) =
        (pager.page(3).unwrap().clone(), pager.page(2).unwrap().clone());
    assert_eq!((left.cell_count(), left.right()), (39, 2));
    assert_eq!((right.cell_count(), right.right()), (39, 0));
    for id in [0, 74, 75, 76, 152] {
        assert!(tree.get(&mut pager, &row_key(id)).unwrap().is_some());
    }
    // @end
}

#[test]
fn splitting_by_bytes() {
    // @listing by-bytes
    let (mut pager, tree) = new_tree();
    add(&tree, &mut pager, 0, 900);
    add(&tree, &mut pager, 1, 900);
    // Small rows, each going in directly after the two large ones, until the leaf
    // splits.
    let mut id = 1000;
    while pager.page_count().unwrap() == 2 {
        add(&tree, &mut pager, id, 40);
        id -= 1;
    }
    let (left, right) =
        (pager.page(3).unwrap().clone(), pager.page(2).unwrap().clone());
    assert_eq!(left.cell_count() + right.cell_count(), 45);
    // By count this would be 22 and 23. By bytes, the two large rows and four
    // small ones weigh as much as the other 39.
    assert_eq!((left.cell_count(), right.cell_count()), (6, 39));
    assert_eq!(
        (left.free_space().unwrap(), right.free_space().unwrap()),
        (2046, 2017)
    );
    // @end
}

#[test]
fn a_tree_that_passes() {
    // @listing check-test
    let (mut pager, tree) = new_tree();
    let empty = TreeInfo { height: 1, leaves: 1, interior: 0, entries: 0, used: 0 };
    assert_eq!(tree.check(&mut pager).unwrap(), empty);

    for id in 1..=1000 {
        add(&tree, &mut pager, id, 40);
    }
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.height, info.leaves, info.interior), (2, 13, 1));
    assert_eq!(info.entries, 1000);
    assert_eq!(info.used, 1000 * 53 + 13 * 16); // the rows, and the root's 13 cells
    // @end
}

#[test]
fn check_finds_damage() {
    // @listing check-damage
    let (mut pager, tree) = new_tree();
    for id in 1..=1000 {
        add(&tree, &mut pager, id, 40);
    }
    let good: Vec<Page> = (0..15).map(|n| pager.page(n).unwrap().clone()).collect();
    // Damages one page, runs the check, and puts the page back.
    let mut damaged = |n: u32, damage: &dyn Fn(&mut Page)| {
        damage(pager.page_mut(n).unwrap());
        let message = tree.check(&mut pager).unwrap_err().to_string();
        *pager.page_mut(n).unwrap() = good[n as usize].clone();
        message
    };

    // Page 2 is the second leaf, after page 3. Replace its last key with one that
    // sorts after every key in the tree.
    let cell = leaf_cell(&row_key(5000), b"");
    let message = damaged(2, &|page| {
        page.remove_cell(76).unwrap();
        assert!(page.insert_cell(76, &cell).unwrap());
    });
    assert_eq!(
        message,
        "corrupt database: page 2: the key in cell 76 belongs on another page"
    );

    let message = damaged(2, &|page| page.set_right(9));
    assert_eq!(message, "corrupt database: page 2: its right link is 9, not 4");

    // Swap two of the root's children.
    let message = damaged(1, &|page| {
        let (a, b) = (page.cell(4).unwrap().to_vec(), page.cell(5).unwrap().to_vec());
        page.remove_cell(4).unwrap();
        page.insert_cell(5, &a).unwrap();
        assert_eq!(page.cell(4).unwrap(), b);
    });
    assert_eq!(
        message,
        "corrupt database: page 1: the key in cell 4 is out of order"
    );

    // Make the root's last child the root itself.
    let message = damaged(1, &|page| {
        let (_, key) = parse_interior(good[1].cell(12).unwrap()).unwrap();
        page.remove_cell(12).unwrap();
        page.insert_cell(12, &interior_cell(1, key)).unwrap();
    });
    assert_eq!(message, "corrupt database: page 1: reached twice");
    tree.check(&mut pager).unwrap(); // everything was put back
    // @end
}

#[test]
fn random_inserts_against_a_model() {
    // @listing random
    let (mut pager, tree) = new_tree();
    let mut model = BTreeMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    let mut heights = Vec::new();
    for count in 1..=100_000 {
        let id = (next(&mut state) >> 1) as i64;
        add(&tree, &mut pager, id, 40);
        model.insert(id, vec![id as u8; 40]);
        if [10, 100, 1000, 10_000, 100_000].contains(&count) {
            heights.push(tree.check(&mut pager).unwrap().height);
        }
    }
    assert_eq!(heights, [1, 2, 2, 2, 3]);
    assert_eq!(tree.root(), 1);
    for (id, payload) in &model {
        assert_eq!(
            tree.get(&mut pager, &row_key(*id)).unwrap().as_ref(),
            Some(payload)
        );
    }
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.entries, info.leaves, info.interior), (100_000, 1861, 9));
    assert_eq!((info.fill() * 1000.0).round(), 698.0); // 69.8% full
    // @end
}

#[test]
fn how_full_the_pages_end_up() {
    // @listing fill
    // Row ids in increasing order, the way a table hands them out.
    let (mut pager, tree) = new_tree();
    for id in 0..100_000 {
        add(&tree, &mut pager, id, 40);
    }
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.height, info.leaves, info.interior), (3, 1299, 7));
    assert_eq!((info.fill() * 1000.0).round(), 998.0);

    // The same rows in decreasing order.
    let (mut pager, tree) = new_tree();
    for id in (0..100_000).rev() {
        add(&tree, &mut pager, id, 40);
    }
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.height, info.leaves, info.interior), (3, 2564, 21));
    assert_eq!((info.fill() * 1000.0).round(), 506.0);
    // @end
}

#[test]
fn the_root_of_a_thousand_rows() {
    // @listing describe-root
    let (mut pager, tree) = new_tree();
    for id in 1..=1000 {
        add(&tree, &mut pager, id, 40);
    }
    let text = pager.page(tree.root()).unwrap().describe().unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 14);
    assert_eq!(lines[0], "kind 2, 13 cells, 3876 bytes free, right 0");
    assert_eq!(
        lines[1],
        "  cell 0:   12 bytes at 4084  00 00 00 03 80 00 00 00 00 00 00 01"
    );
    assert_eq!(
        lines[2],
        "  cell 1:   12 bytes at 4072  00 00 00 02 80 00 00 00 00 00 00 4e"
    );
    assert_eq!(
        lines[3],
        "  cell 2:   12 bytes at 4060  00 00 00 04 80 00 00 00 00 00 00 9b"
    );
    // @end
}
