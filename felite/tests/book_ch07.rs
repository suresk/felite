//! The examples in Chapter 7: `cargo test --test book_ch07`.
use felite::{Cursor, FREE, INTERIOR, LEAF, MemFile, Page, Pager, Tree};
use felite::{interior_cell, leaf_cell, row_key};
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

// @listing remove
/// Deletes a row that must be there.
fn remove(tree: &Tree, pager: &mut Pager, id: i64) {
    assert!(tree.delete(pager, &row_key(id)).unwrap());
}
// @end

#[test]
fn removing_keys() {
    // @listing delete-test
    let (mut pager, tree) = new_tree();
    for (code, city) in
        [("SLC", "Salt Lake City"), ("ATL", "Atlanta"), ("DEN", "Denver")]
    {
        tree.insert(&mut pager, code.as_bytes(), city.as_bytes()).unwrap();
    }
    assert!(tree.delete(&mut pager, b"DEN").unwrap());
    assert_eq!(tree.get(&mut pager, b"DEN").unwrap(), None);
    assert!(!tree.delete(&mut pager, b"DEN").unwrap()); // already gone
    assert!(!tree.delete(&mut pager, b"JFK").unwrap()); // never there
    assert_eq!(tree.check(&mut pager).unwrap().entries, 2);
    // The key can be inserted again.
    tree.insert(&mut pager, b"DEN", b"Denver, again").unwrap();
    assert_eq!(tree.get(&mut pager, b"DEN").unwrap().unwrap(), b"Denver, again");
    // @end
}

#[test]
fn the_free_list() {
    // @listing free-list-test
    let file = MemFile::new();
    let mut pager = Pager::open(Box::new(file.clone())).unwrap();
    for _ in 0..4 {
        pager.allocate(Page::new()).unwrap(); // pages 1 to 4
    }
    pager.free(2).unwrap();
    pager.free(4).unwrap();
    assert_eq!(pager.free_count().unwrap(), 2);
    assert_eq!(pager.page_count().unwrap(), 5); // freeing does not shorten the file

    // The header points at page 4, the last one freed, and page 4 points at page 2.
    assert_eq!(pager.page(0).unwrap().u32_at(24), 4);
    let page = pager.page(4).unwrap();
    assert_eq!((page.bytes()[0], page.u32_at(4)), (FREE, 2));
    assert_eq!(pager.page(2).unwrap().u32_at(4), 0); // the end of the list

    // Allocation takes from the list before it extends the file.
    assert_eq!(pager.allocate(Page::new()).unwrap(), 4);
    assert_eq!(pager.allocate(Page::new()).unwrap(), 2);
    assert_eq!(pager.allocate(Page::new()).unwrap(), 5);
    assert_eq!((pager.free_count().unwrap(), pager.page_count().unwrap()), (0, 6));

    // The list is in the file, so it is still there after a flush and a reopen.
    pager.free(3).unwrap();
    pager.flush().unwrap();
    let mut pager = Pager::open(Box::new(file)).unwrap();
    assert_eq!(pager.free_count().unwrap(), 1);
    assert_eq!(pager.allocate(Page::new()).unwrap(), 3);
    // @end
}

#[test]
fn what_free_refuses() {
    // @listing free-refuses
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    pager.allocate(Page::new()).unwrap();
    let message = |pager: &mut Pager, n| pager.free(n).unwrap_err().to_string();
    assert_eq!(message(&mut pager, 0), "corrupt database: page 0 cannot be freed");
    assert_eq!(
        message(&mut pager, 7),
        "corrupt database: page 7 is past the end of a 2-page file"
    );
    pager.free(1).unwrap();
    assert_eq!(message(&mut pager, 1), "corrupt database: page 1 cannot be freed");
    assert_eq!(pager.free_count().unwrap(), 1);
    // @end
}

#[test]
fn two_leaves_become_one() {
    // @listing merge-test
    // The tree from Section 5.6: two leaves of 39 rows under a root.
    let (mut pager, tree) = new_tree();
    for id in (0..154).step_by(2) {
        add(&tree, &mut pager, id, 40);
    }
    add(&tree, &mut pager, 75, 40);
    assert_eq!(tree.check(&mut pager).unwrap().height, 2);

    // Take rows from the right-hand leaf. At 19 rows it is under a quarter full.
    let mut id = 152;
    for _ in 0..19 {
        remove(&tree, &mut pager, id);
        id -= 2;
    }
    assert_eq!(pager.page(2).unwrap().cell_count(), 20);
    assert_eq!(pager.free_count().unwrap(), 0);
    remove(&tree, &mut pager, id);

    // The 19 rows joined the 39 on the left, and the root, left with one child,
    // took that child's place. The tree is one leaf again, still on page 1.
    let root = pager.page(1).unwrap();
    assert_eq!((root.kind(), root.cell_count()), (LEAF, 58));
    assert_eq!(pager.free_count().unwrap(), 2);
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.height, info.leaves, info.entries), (1, 1, 58));
    // @end
}

#[test]
fn a_small_page_that_cannot_merge() {
    // @listing no-merge
    let (mut pager, tree) = new_tree();
    for id in 0..231 {
        add(&tree, &mut pager, id, 40); // three full leaves: pages 3, 2 and 4
    }
    // Empty the middle leaf down to one row. Its neighbor is full, so there is
    // nowhere for even one row to go, and the leaf stays.
    for id in 77..153 {
        remove(&tree, &mut pager, id);
    }
    assert_eq!(pager.page(2).unwrap().cell_count(), 1);
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.leaves, pager.free_count().unwrap()), (3, 0));

    // An empty page fits with anything. Removing the last row frees the leaf, and
    // the first leaf now links straight to the third.
    remove(&tree, &mut pager, 153);
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.leaves, pager.free_count().unwrap()), (2, 1));
    assert_eq!(pager.page(2).unwrap().kind(), FREE);
    assert_eq!(pager.page(3).unwrap().right(), 4);
    // @end
}

#[test]
fn inserts_and_deletes_against_a_model() {
    // @listing mixed
    let (mut pager, tree) = new_tree();
    let mut model = BTreeMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for step in 1..=100_000 {
        let random = next(&mut state);
        let id = (random >> 8) as i64 % 20_000;
        if !random.is_multiple_of(3) {
            // Two times in three, insert the row if it is not there.
            if model.insert(id, ()).is_none() {
                add(&tree, &mut pager, id, 40);
            }
        } else {
            // Otherwise delete it, and agree with the model about whether it was.
            let was_there = model.remove(&id).is_some();
            assert_eq!(tree.delete(&mut pager, &row_key(id)).unwrap(), was_there);
        }
        if step % 10_000 == 0 {
            assert_eq!(tree.check(&mut pager).unwrap().entries, model.len());
        }
    }
    let mut cursor = Cursor::first(&tree, &mut pager).unwrap();
    for id in model.keys() {
        let (key, _) = cursor.next(&mut pager).unwrap().unwrap();
        assert_eq!(key, row_key(*id));
    }
    assert_eq!(cursor.next(&mut pager).unwrap(), None);
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.entries, info.height, info.leaves), (13_326, 3, 269));
    // @end
}

#[test]
fn deleting_everything() {
    // @listing delete-all
    let (mut pager, tree) = new_tree();
    for id in 0..30_000 {
        add(&tree, &mut pager, id, 40);
    }
    let info = tree.check(&mut pager).unwrap();
    assert_eq!((info.height, info.leaves, info.interior), (3, 390, 3));
    assert_eq!(pager.page_count().unwrap(), 394);

    // (rows deleted so far, height, leaves, free pages)
    let mut history = Vec::new();
    for id in 0..29_999 {
        remove(&tree, &mut pager, id);
        if [10_000, 20_000, 29_000, 29_999].contains(&(id + 1)) {
            let info = tree.check(&mut pager).unwrap();
            history.push((
                id + 1,
                info.height,
                info.leaves,
                pager.free_count().unwrap(),
            ));
        }
    }
    assert_eq!(
        history,
        [
            (10_000, 3, 261, 129),
            (20_000, 2, 131, 261),
            (29_000, 2, 14, 378),
            (29_999, 1, 1, 392)
        ]
    );
    remove(&tree, &mut pager, 29_999);
    // One empty leaf, which is the root. Every other page is on the free list,
    // and the file is as long as it ever was.
    assert_eq!(tree.root(), 1);
    assert_eq!(pager.page(1).unwrap().kind(), LEAF);
    assert_eq!(
        (pager.page_count().unwrap(), pager.free_count().unwrap()),
        (394, 392)
    );

    // Loading the rows again uses those pages, and the file does not grow.
    for id in 0..30_000 {
        add(&tree, &mut pager, id, 40);
    }
    assert_eq!((pager.page_count().unwrap(), pager.free_count().unwrap()), (394, 0));
    tree.check(&mut pager).unwrap();
    // @end
}

#[test]
fn a_queue_stops_growing() {
    // @listing queue
    let (mut pager, tree) = new_tree();
    for id in 0..10_000 {
        add(&tree, &mut pager, id, 40);
    }
    let mut pages = vec![pager.page_count().unwrap()];
    // 50,000 times: add a row at the end, remove the oldest row from the front.
    for id in 10_000..60_000 {
        add(&tree, &mut pager, id, 40);
        remove(&tree, &mut pager, id - 10_000);
        if (id + 1) % 10_000 == 0 {
            pages.push(pager.page_count().unwrap());
        }
    }
    assert_eq!(pages, [132, 133, 133, 133, 133, 133]);
    assert_eq!(tree.check(&mut pager).unwrap().entries, 10_000);
    // @end
}

#[test]
fn random_churn_levels_off() {
    // @listing churn
    let (mut pager, tree) = new_tree();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    let mut live = Vec::new();
    for _ in 0..10_000 {
        let id = (next(&mut state) >> 1) as i64;
        add(&tree, &mut pager, id, 40);
        live.push(id);
    }
    let mut pages = vec![pager.page_count().unwrap()];
    // 100,000 times: replace a randomly chosen row with a new random one.
    for step in 1..=100_000 {
        let which = (next(&mut state) % 10_000) as usize;
        remove(&tree, &mut pager, live[which]);
        live[which] = (next(&mut state) >> 1) as i64;
        add(&tree, &mut pager, live[which], 40);
        if step % 20_000 == 0 {
            pages.push(pager.page_count().unwrap());
        }
    }
    assert_eq!(pages, [189, 216, 227, 230, 233, 233]);
    let info = tree.check(&mut pager).unwrap();
    assert_eq!(info.entries, 10_000);
    assert_eq!((info.fill() * 100.0).round(), 57.0);
    // @end
}

#[test]
fn an_empty_leaf_at_the_end() {
    // @listing empty-last-leaf
    // By hand: a root with two leaves, the second of them empty.
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let mut root = Page::new();
    root.init_slotted(INTERIOR);
    root.insert_cell(0, &interior_cell(2, &row_key(1))).unwrap();
    root.insert_cell(1, &interior_cell(3, &row_key(100))).unwrap();
    let mut first = Page::new();
    first.init_slotted(LEAF);
    for (i, id) in [1, 2, 3].into_iter().enumerate() {
        first.insert_cell(i, &leaf_cell(&row_key(id), b"row")).unwrap();
    }
    first.set_right(3);
    let mut last = Page::new();
    last.init_slotted(LEAF);
    for page in [root, first, last] {
        pager.allocate(page).unwrap(); // pages 1, 2 and 3
    }
    let tree = Tree::open(1);
    tree.check(&mut pager).unwrap();

    assert_eq!(tree.last_key(&mut pager).unwrap().unwrap(), row_key(3));
    assert_eq!(tree.next_row_id(&mut pager).unwrap(), 4);
    // @end
}

#[test]
fn destroying_a_tree() {
    // @listing destroy-test
    let mut pager = Pager::open(Box::new(MemFile::new())).unwrap();
    let keep = Tree::create(&mut pager).unwrap();
    let doomed = Tree::create(&mut pager).unwrap();
    for id in 0..5000 {
        add(&keep, &mut pager, id, 40);
        add(&doomed, &mut pager, id, 40);
    }
    let pages = doomed.check(&mut pager).unwrap();
    let pages = (pages.leaves + pages.interior) as u32;
    assert_eq!(pages, 66);

    doomed.destroy(&mut pager).unwrap();
    assert_eq!(pager.free_count().unwrap(), 66);
    // The other tree, whose pages were interleaved with it in the file, is intact.
    assert_eq!(keep.check(&mut pager).unwrap().entries, 5000);
    // @end
}
