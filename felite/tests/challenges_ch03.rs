//! Tests for the Chapter 3 challenges: `cargo test --test challenges_ch03`.
use felite::Page;

fn page_of(cells: &[&[u8]]) -> Page {
    let mut page = Page::new();
    page.init_slotted(1);
    for (i, cell) in cells.iter().enumerate() {
        assert!(page.insert_cell(i, cell).unwrap());
    }
    page
}

// @challenge 2
use felite::replace_cell;

// @challenge 2 3 4 5
use felite::check_slotted;

// @challenge 4
use felite::truncate_cells;

// @challenge 5
use felite::insert_cell_reusing;

// @challenge 1
#[test]
fn predict_free_space() {
    let mut page = page_of(&[&[1; 100], &[2; 200], &[3; 300]]);
    assert_eq!(page.free_space().unwrap(), 3472);
    page.remove_cell(0).unwrap();
    assert_eq!(page.free_space().unwrap(), 3576);
    assert_eq!(page.u16_at(6), 100); // the hole
    page.remove_cell(1).unwrap(); // the 300-byte cell, the lowest in the page
    assert_eq!(page.free_space().unwrap(), 3880);
    assert_eq!(page.u16_at(6), 100);
    assert_eq!(page.u16_at(4), 3796); // the 200-byte cell starts here
    // 13-byte flight records: 240 to a page, so January 2024 needs 2,281 pages.
    assert_eq!(4084 / (13 + 4), 240);
    assert_eq!(547_271usize.div_ceil(240), 2281);
}

// @challenge 2
#[test]
fn replace_in_place_or_move() {
    let mut page = page_of(&[b"Atlanta", b"Boston", b"Chicago"]);
    let before = page.free_space().unwrap();
    let boston_at = page.u16_at(12 + 4);
    assert!(replace_cell(&mut page, 1, b"Bend").unwrap());
    assert_eq!(page.cell(1).unwrap(), b"Bend");
    assert_eq!(page.u16_at(12 + 4), boston_at); // it did not move
    assert_eq!(page.free_space().unwrap(), before + 2);
    check_slotted(&page).unwrap();

    assert!(replace_cell(&mut page, 1, b"Birmingham").unwrap());
    assert_eq!(page.cell(0).unwrap(), b"Atlanta");
    assert_eq!(page.cell(1).unwrap(), b"Birmingham");
    assert_eq!(page.cell(2).unwrap(), b"Chicago");
    assert_ne!(page.u16_at(12 + 4), boston_at);
    check_slotted(&page).unwrap();

    // No room: the page is left as it was.
    let mut full = page_of(&[&[1; 2000], &[2; 2000]]);
    let copy = full.clone();
    assert!(!replace_cell(&mut full, 0, &[3; 2100]).unwrap());
    assert_eq!(full.bytes(), copy.bytes());
    // Room only because the old cell's own space counts.
    assert!(replace_cell(&mut full, 0, &[3; 2076]).unwrap());
    assert_eq!(full.free_space().unwrap(), 0);
    check_slotted(&full).unwrap();
}

// @challenge 3
#[test]
fn check_finds_what_is_wrong() {
    let mut good = page_of(&[b"Atlanta", b"Boston", b"Chicago"]);
    check_slotted(&good).unwrap();
    good.remove_cell(1).unwrap();
    check_slotted(&good).unwrap();
    let message = |page: &Page| check_slotted(page).unwrap_err().to_string();

    let mut page = good.clone();
    page.put_u16(6, 0); // forget the hole
    assert_eq!(
        message(&page),
        "corrupt database: 6 bytes in holes, and the header says 0"
    );
    let mut page = good.clone();
    let chicago_at = page.u16_at(16);
    page.put_u16(12, chicago_at + 3); // cell 0 now starts inside cell 1
    assert_eq!(message(&page), "corrupt database: the cell at byte 4079 overlaps");
    let mut page = good.clone();
    page.put_u16(2, 3000);
    assert_eq!(message(&page), "corrupt database: a page claims 3000 cells");
}

// @challenge 4
#[test]
fn truncate_drops_the_tail() {
    let cells: Vec<Vec<u8>> = (0..100).map(|i| vec![i as u8; 30]).collect();
    let refs: Vec<&[u8]> = cells.iter().map(Vec::as_slice).collect();
    let mut page = page_of(&refs);
    truncate_cells(&mut page, 40).unwrap();
    assert_eq!(page.cell_count(), 40);
    assert_eq!(page.free_space().unwrap(), 4084 - 40 * 34);
    assert_eq!(page.u16_at(6), 0); // no holes
    for (i, cell) in cells.iter().take(40).enumerate() {
        assert_eq!(page.cell(i).unwrap(), cell);
    }
    check_slotted(&page).unwrap();
    truncate_cells(&mut page, 0).unwrap();
    let mut empty = Page::new();
    empty.init_slotted(1);
    assert_eq!(page.bytes(), empty.bytes());
}

// @challenge 5
#[test]
fn a_hole_of_the_right_size_is_reused() {
    let mut page = page_of(&[&[1; 1000], &[2; 1000], &[3; 1000], &[4; 1000]]);
    let second_at = page.u16_at(12 + 4);
    page.remove_cell(1).unwrap();
    // A cell too long for the gap goes into the hole, and nothing is compacted.
    assert!(insert_cell_reusing(&mut page, 0, &[5; 1000]).unwrap());
    assert_eq!(page.u16_at(12), second_at);
    assert_eq!(page.u16_at(6), 0);
    let firsts: Vec<u8> = (0..4).map(|i| page.cell(i).unwrap()[0]).collect();
    assert_eq!(firsts, [5, 1, 3, 4]);
    check_slotted(&page).unwrap();
    // With no hole of the right size it behaves as insert_cell does.
    page.remove_cell(2).unwrap();
    assert!(insert_cell_reusing(&mut page, 3, &[6; 10]).unwrap());
    assert_eq!(page.u16_at(6), 1000);
    assert!(!insert_cell_reusing(&mut page, 0, &[7; 2000]).unwrap());
    check_slotted(&page).unwrap();
}
