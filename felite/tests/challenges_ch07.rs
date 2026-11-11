//! Tests for the Chapter 7 challenges: `cargo test --test challenges_ch07`.
use felite::{MemFile, Page, Pager, Tree, row_key};
use felite::{account, delete_range, free_pages, rebuild, tree_pages};

fn new_tree() -> (Pager, Tree) {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let tree = Tree::create(&mut pager).unwrap();
    (pager, tree)
}

fn add(tree: &Tree, pager: &mut Pager, id: i64, size: usize) {
    tree.insert(pager, &row_key(id), &vec![id as u8; size]).unwrap();
}

#[test]
fn predict_the_free_list() {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    for _ in 0..6 {
        pager.allocate(Page::new()).unwrap(); // pages 1 to 6
    }
    for n in [3, 5, 1] {
        pager.free(n).unwrap();
    }
    assert_eq!(free_pages(&mut pager).unwrap(), [1, 5, 3]);
    assert_eq!(pager.allocate(Page::new()).unwrap(), 1);
    pager.free(6).unwrap();
    assert_eq!(free_pages(&mut pager).unwrap(), [6, 5, 3]);
    let got: Vec<u32> =
        (0..4).map(|_| pager.allocate(Page::new()).unwrap()).collect();
    assert_eq!(got, [6, 5, 3, 7]);
    assert_eq!(pager.page_count().unwrap(), 8);

    // 77 rows to a leaf, and a leaf merges when it is under a quarter full: 1,021
    // bytes, which is 19 rows.
    assert_eq!((4084 / 4, 1021 / 53), (1021, 19));
    // Two full leaves, emptied from the far end of the second.
    let (mut pager, tree) = new_tree();
    for id in 0..154 {
        add(&tree, &mut pager, id, 40);
    }
    let mut deleted = 0;
    while pager.free_count().unwrap() == 0 {
        tree.delete(&mut pager, &row_key(153 - deleted)).unwrap();
        deleted += 1;
    }
    // At 19 rows the second leaf is small, but 77 + 19 rows do not fit in a page.
    // It has to be empty before it can go.
    assert_eq!(deleted, 77);
}

#[test]
fn deleting_a_range() {
    let (mut pager, tree) = new_tree();
    for id in 0..10_000 {
        add(&tree, &mut pager, id, 40);
    }
    let mut delete = |low: i64, high: i64| {
        delete_range(&tree, &mut pager, &row_key(low), &row_key(high)).unwrap()
    };
    assert_eq!(delete(2000, 7000), 5000);
    assert_eq!(delete(2000, 7000), 0);
    assert_eq!(delete(-5, 10), 10);
    assert_eq!(delete(9990, 20_000), 10);
    assert_eq!(delete(500, 400), 0);
    let info = tree.check(&mut pager).unwrap();
    assert_eq!(info.entries, 4980);
    assert_eq!(
        tree.get(&mut pager, &row_key(1999)).unwrap().unwrap(),
        [1999u32 as u8; 40]
    );
    assert_eq!(tree.get(&mut pager, &row_key(2000)).unwrap(), None);
    assert!(tree.get(&mut pager, &row_key(7000)).unwrap().is_some());
    account(&mut pager, &[tree]).unwrap();
}

#[test]
fn where_every_page_is() {
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let a = Tree::create(&mut pager).unwrap();
    let b = Tree::create(&mut pager).unwrap();
    for id in 0..3000 {
        add(&a, &mut pager, id, 40);
        add(&b, &mut pager, id, 100);
    }
    let pages_a = tree_pages(&a, &mut pager).unwrap();
    let info = a.check(&mut pager).unwrap();
    assert_eq!(pages_a.len(), info.leaves + info.interior);
    assert_eq!(pages_a[0], a.root());
    account(&mut pager, &[a, b]).unwrap();

    for id in 0..3000 {
        a.delete(&mut pager, &row_key(id)).unwrap();
    }
    assert_eq!(tree_pages(&a, &mut pager).unwrap(), [a.root()]);
    assert_eq!(free_pages(&mut pager).unwrap().len(), pages_a.len() - 1);
    account(&mut pager, &[a, b]).unwrap();

    // Leave a tree out, and its pages belong to nothing.
    let error = account(&mut pager, &[a]).unwrap_err().to_string();
    assert!(error.ends_with("pages belong to nothing"), "{error}");
    // Name a tree twice, and its pages are in use twice.
    let error = account(&mut pager, &[a, b, b]).unwrap_err().to_string();
    assert_eq!(error, format!("corrupt database: page {} is in use twice", b.root()));
    // A free list that loops.
    let first = free_pages(&mut pager).unwrap()[0];
    pager.page_mut(first).unwrap().put_u32(4, first);
    assert!(free_pages(&mut pager).is_err());
}

#[test]
fn rebuilding_a_tree() {
    let (mut pager, tree) = new_tree();
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut ids = Vec::new();
    for _ in 0..20_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        ids.push((state >> 1) as i64);
        add(&tree, &mut pager, *ids.last().unwrap(), 40);
    }
    for id in ids.iter().step_by(2) {
        tree.delete(&mut pager, &row_key(*id)).unwrap(); // every other row
    }
    let before = tree.check(&mut pager).unwrap();
    assert_eq!(before.entries, 10_000);
    assert_eq!((before.fill() * 100.0).round(), 39.0);

    rebuild(&tree, &mut pager).unwrap();
    let after = tree.check(&mut pager).unwrap();
    assert_eq!(after.entries, 10_000);
    assert_eq!((after.fill() * 100.0).round(), 99.0);
    assert_eq!((before.leaves, after.leaves), (337, 130));
    assert_eq!(tree.root(), 1);
    for id in ids.iter().skip(1).step_by(2) {
        assert!(tree.get(&mut pager, &row_key(*id)).unwrap().is_some());
    }
    account(&mut pager, &[tree]).unwrap();
    assert_eq!(pager.free_count().unwrap(), 243); // pages the rebuilt tree does not need
}
