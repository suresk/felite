//! Tests for the Chapter 5 challenges: `cargo test --test challenges_ch05`.
use felite::{MemFile, Pager, Tree, interior_cell, row_key};
use felite::{bulk_load, count, put};

fn new_tree() -> (Pager, Tree) {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    (pager, tree)
}

fn add(tree: &Tree, pager: &mut Pager, id: i64, size: usize) {
    tree.insert(pager, &row_key(id), &vec![id as u8; size]).unwrap();
}

#[test]
fn predict_a_small_tree() {
    // Rows of 900 bytes: a cell of 909 and its slot, so four to a leaf.
    let (mut pager, tree) = new_tree();
    for id in [10, 20, 30, 40] {
        add(&tree, &mut pager, id, 900);
    }
    assert_eq!(pager.page_count().unwrap(), 2);
    add(&tree, &mut pager, 25, 900); // the first split, down the middle
    assert_eq!(pager.page_count().unwrap(), 4);
    let ids = |pager: &mut Pager, n| {
        let page = pager.page(n).unwrap().clone();
        let mut ids = Vec::new();
        for i in 0..page.cell_count() {
            let (key, _) = felite::parse_leaf(page.cell(i).unwrap()).unwrap();
            ids.push(felite::row_id(key).unwrap());
        }
        ids
    };
    assert_eq!(ids(&mut pager, 3), [10, 20]);
    assert_eq!(ids(&mut pager, 2), [25, 30, 40]);
    add(&tree, &mut pager, 50, 900); // room in the right leaf
    assert_eq!(ids(&mut pager, 2), [25, 30, 40, 50]);
    add(&tree, &mut pager, 60, 900); // at the end of the last leaf: a page of its own
    assert_eq!(ids(&mut pager, 2), [25, 30, 40, 50]);
    assert_eq!(ids(&mut pager, 4), [60]);
    add(&tree, &mut pager, 15, 900);
    add(&tree, &mut pager, 5, 900);
    add(&tree, &mut pager, 12, 900); // the left leaf is full: [5, 10, 12, 15, 20] splits
    assert_eq!(ids(&mut pager, 3), [5, 10]);
    assert_eq!(ids(&mut pager, 5), [12, 15, 20]);
    let root = pager.page(1).unwrap();
    assert_eq!(root.cell_count(), 4);
    assert_eq!(root.cell(0).unwrap(), interior_cell(3, &row_key(10)));
    assert_eq!(root.cell(1).unwrap(), interior_cell(5, &row_key(12)));
    assert_eq!(root.cell(2).unwrap(), interior_cell(2, &row_key(25)));
    assert_eq!(root.cell(3).unwrap(), interior_cell(4, &row_key(60)));
    tree.check(&mut pager).unwrap();
}

#[test]
fn put_inserts_or_replaces() {
    let (mut pager, tree) = new_tree();
    assert!(!put(&tree, &mut pager, b"DEN", b"Denver").unwrap());
    assert!(put(&tree, &mut pager, b"DEN", b"Denver, Colorado").unwrap());
    assert_eq!(tree.get(&mut pager, b"DEN").unwrap().unwrap(), b"Denver, Colorado");
    assert!(!put(&tree, &mut pager, b"DENX", b"not Denver").unwrap());
    assert_eq!(tree.check(&mut pager).unwrap().entries, 2);

    // Replacing a row with a longer one in a full leaf has to split it.
    let (mut pager, tree) = new_tree();
    for id in 0..77 {
        add(&tree, &mut pager, id, 40);
    }
    assert!(put(&tree, &mut pager, &row_key(30), &[9; 500]).unwrap());
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.entries, info.leaves), (77, 2));
    assert_eq!(tree.get(&mut pager, &row_key(30)).unwrap().unwrap(), [9; 500]);
    // Too large: refused, and the old value is still there.
    assert!(put(&tree, &mut pager, &row_key(30), &[9; 2000]).is_err());
    assert_eq!(tree.get(&mut pager, &row_key(30)).unwrap().unwrap(), [9; 500]);
}

#[test]
fn counting_splits() {
    // Every split allocates one page, and a root split allocates one more.
    let (mut pager, tree) = new_tree();
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..100_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        add(&tree, &mut pager, (state >> 1) as i64, 40);
    }
    let info = tree.check(&mut pager).unwrap();
    let pages = pager.page_count().unwrap() as usize - 1; // without the file header
    assert_eq!(pages, info.leaves + info.interior);
    assert_eq!(pages, 1870);
    let root_splits = info.height - 1;
    let splits = pages - 1 - root_splits;
    assert_eq!(splits, 1867); // one insert in 54 splits a page
    assert_eq!(count(&tree, &mut pager).unwrap(), 100_000);
}

#[test]
fn count_walks_the_leaves() {
    let (mut pager, tree) = new_tree();
    assert_eq!(count(&tree, &mut pager).unwrap(), 0);
    for id in 0..5000 {
        add(&tree, &mut pager, id * 7 % 5000, 40);
        if id % 500 == 499 {
            assert_eq!(count(&tree, &mut pager).unwrap(), id as usize + 1);
        }
    }
    // A leaf that links back to itself is an error, not a hang.
    let first = pager.page(1).unwrap().clone();
    let (child, _) = felite::parse_interior(first.cell(0).unwrap()).unwrap();
    pager.page_mut(child).unwrap().set_right(child);
    assert!(count(&tree, &mut pager).is_err());
}

#[test]
fn bulk_load_builds_full_pages() {
    let entries: Vec<(Vec<u8>, Vec<u8>)> = (0..100_000i64)
        .map(|id| (row_key(id).to_vec(), vec![id as u8; 40]))
        .collect();
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = bulk_load(&mut pager, &entries).unwrap();
    assert_eq!(tree.root(), 1);
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.height, info.leaves, info.interior), (3, 1299, 7));
    assert_eq!(info.entries, 100_000);
    for id in [0, 76, 77, 54_321, 99_999] {
        assert_eq!(
            tree.get(&mut pager, &row_key(id)).unwrap().unwrap(),
            [id as u8; 40]
        );
    }
    // More rows can be inserted afterwards.
    tree.insert(&mut pager, &row_key(-5), b"new").unwrap();
    tree.insert(&mut pager, &row_key(100_000), b"new").unwrap();
    assert_eq!(tree.check(&mut pager).unwrap().entries, 100_002);

    // Small inputs: nothing, and one leaf's worth.
    let empty = bulk_load(&mut pager, &[]).unwrap();
    assert_eq!(empty.check(&mut pager).unwrap().entries, 0);
    let small = bulk_load(&mut pager, &entries[..50]).unwrap();
    assert_eq!(small.check(&mut pager).unwrap().height, 1);
}
