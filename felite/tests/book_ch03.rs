//! The examples in Chapter 3: `cargo test --test book_ch03`.
use felite::{Page, SLOT_SIZE, SLOTTED_HEADER, Value, encode, hex_dump};

#[test]
fn an_empty_slotted_page() {
    // @listing empty-page
    let mut page = Page::new();
    page.init_slotted(1);
    assert_eq!(page.kind(), 1);
    assert_eq!(page.cell_count(), 0);
    assert_eq!(page.free_space().unwrap(), 4084); // 4,096 less the 12-byte header
    assert_eq!(page.bytes()[..12], [1, 0, 0, 0, 0x10, 0, 0, 0, 0, 0, 0, 0]);
    // @end
}

#[test]
fn three_cells() {
    // @listing three-cells
    let mut page = Page::new();
    page.init_slotted(1);
    assert!(page.insert_cell(0, b"Salt Lake City").unwrap());
    assert!(page.insert_cell(1, b"Atlanta").unwrap());
    assert!(page.insert_cell(0, b"Denver").unwrap()); // in front of the other two

    assert_eq!(page.cell(0).unwrap(), b"Denver");
    assert_eq!(page.cell(1).unwrap(), b"Salt Lake City");
    assert_eq!(page.cell(2).unwrap(), b"Atlanta");
    // 27 bytes of cells and three 4-byte slots are gone from the 4,084.
    assert_eq!(page.free_space().unwrap(), 4084 - 27 - 12);
    assert_eq!(
        hex_dump(page.bytes()),
        "0000  01 00 00 03 0f e5 00 00 00 00 00 00 0f e5 00 06  |................|\n\
         0010  0f f2 00 0e 0f eb 00 07 00 00 00 00 00 00 00 00  |................|\n\
         0020  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00  |................|\n\
         *\n\
         0fe0  00 00 00 00 00 44 65 6e 76 65 72 41 74 6c 61 6e  |.....DenverAtlan|\n\
         0ff0  74 61 53 61 6c 74 20 4c 61 6b 65 20 43 69 74 79  |taSalt Lake City|\n"
    );
    // @end
}

#[test]
fn how_many_cells_fit() {
    // @listing capacity
    let mut page = Page::new();
    page.init_slotted(1);
    let cell = [7u8; 20];
    let mut count = 0;
    while page.insert_cell(count, &cell).unwrap() {
        count += 1;
    }
    assert_eq!(count, 170); // 4,084 / (20 + 4), rounded down
    assert_eq!(page.free_space().unwrap(), 4); // not enough for another slot and cell
    assert_eq!(page.cell_count(), 170);

    // The largest cell an empty page can take leaves nothing over.
    let mut page = Page::new();
    page.init_slotted(1);
    assert!(!page.insert_cell(0, &[0; 4081]).unwrap());
    assert!(page.insert_cell(0, &[0; 4080]).unwrap());
    assert_eq!(page.free_space().unwrap(), 0);
    assert_eq!(4080, 4096 - SLOTTED_HEADER - SLOT_SIZE);
    // @end
}

#[test]
fn removing_a_cell() {
    // @listing remove-test
    let mut page = Page::new();
    page.init_slotted(1);
    for (i, city) in ["Atlanta", "Boston", "Chicago", "Denver"].iter().enumerate() {
        page.insert_cell(i, city.as_bytes()).unwrap();
    }
    let before = page.free_space().unwrap();

    page.remove_cell(1).unwrap(); // Boston
    assert_eq!(page.cell_count(), 3);
    assert_eq!(page.cell(0).unwrap(), b"Atlanta");
    assert_eq!(page.cell(1).unwrap(), b"Chicago"); // what was cell 2
    assert_eq!(page.cell(2).unwrap(), b"Denver");
    // Boston's six bytes and its slot are free again.
    assert_eq!(page.free_space().unwrap(), before + 6 + 4);
    // But the six bytes are a hole between two cells, not part of the gap.
    assert_eq!(page.u16_at(6), 6);
    assert_eq!(
        hex_dump(&page.bytes()[0xfe0..]),
        "0000  00 00 00 00 00 00 44 65 6e 76 65 72 43 68 69 63  |......DenverChic|\n\
         0010  61 67 6f 00 00 00 00 00 00 41 74 6c 61 6e 74 61  |ago......Atlanta|\n"
    );
    // @end
}

#[test]
fn the_lowest_cell_leaves_no_hole() {
    // @listing remove-lowest
    let mut page = Page::new();
    page.init_slotted(1);
    page.insert_cell(0, b"Atlanta").unwrap();
    page.insert_cell(1, b"Boston").unwrap();
    page.remove_cell(1).unwrap(); // the cell written last, so the lowest in the page
    assert_eq!(page.u16_at(6), 0); // no fragmented bytes
    assert_eq!(page.u16_at(4), 4096 - 7); // cell content starts at Atlanta again
    // @end
}

#[test]
fn compaction_makes_room() {
    // @listing compact-test
    let mut page = Page::new();
    page.init_slotted(1);
    for i in 0..4 {
        assert!(page.insert_cell(i, &[b'a' + i as u8; 1000]).unwrap());
    }
    assert_eq!(page.free_space().unwrap(), 4084 - 4 * 1004); // 68 bytes

    page.remove_cell(1).unwrap(); // the thousand b's
    assert_eq!(page.free_space().unwrap(), 1072);
    assert_eq!(page.u16_at(6), 1000); // nearly all of it is one hole

    // 1,050 bytes fit in the free space, but in no single stretch of it.
    assert!(page.insert_cell(3, &[b'e'; 1050]).unwrap());
    assert_eq!(page.u16_at(6), 0); // the page was compacted to make room
    assert_eq!(page.free_space().unwrap(), 1072 - 1054);
    let firsts: Vec<u8> = (0..4).map(|i| page.cell(i).unwrap()[0]).collect();
    assert_eq!(firsts, b"acde");
    // @end
}

#[test]
fn records_in_a_page() {
    // @listing records
    let flights =
        [(1432, "DL", "SLC", -7), (89, "UA", "DEN", 12), (2210, "WN", "LAS", 0)];
    let mut page = Page::new();
    page.init_slotted(1);
    for (i, (number, carrier, origin, delay)) in flights.into_iter().enumerate() {
        let record = encode(&[
            Value::Integer(number),
            Value::Text(carrier.to_string()),
            Value::Text(origin.to_string()),
            Value::Integer(delay),
        ]);
        assert!(page.insert_cell(i, &record).unwrap());
    }
    assert_eq!(
        page.describe().unwrap(),
        "kind 1, 3 cells, 4033 bytes free, right 0\n  \
           cell 0:   13 bytes at 4083  04 01 05 06 01 96 30 44 4c 53 4c 43 …\n  \
           cell 1:   13 bytes at 4070  04 01 05 06 01 81 32 55 41 44 45 4e …\n  \
           cell 2:   13 bytes at 4057  04 01 05 06 01 a2 44 57 4e 4c 41 53 …\n"
    );
    // @end
}

#[test]
fn a_damaged_page_is_an_error() {
    // @listing damaged-page
    let mut good = Page::new();
    good.init_slotted(1);
    good.insert_cell(0, b"Atlanta").unwrap();
    let message = |page: &Page| page.cell(0).unwrap_err().to_string();

    let mut page = good.clone();
    page.put_u16(2, 2000); // the cell count
    assert_eq!(message(&page), "corrupt database: a page claims 2000 cells");

    let mut page = good.clone();
    page.put_u16(12, 8); // cell 0's offset, now inside the header
    assert_eq!(message(&page), "corrupt database: cell 0 claims bytes 8 to 15");

    let mut page = good.clone();
    page.put_u16(14, 500); // cell 0's length
    assert_eq!(message(&page), "corrupt database: cell 0 claims bytes 4089 to 4589");

    let mut page = good.clone();
    page.put_u16(4, 5); // the start of cell content
    let error = page.insert_cell(1, b"Boston").unwrap_err().to_string();
    assert_eq!(error, "corrupt database: cell content starts at byte 5");
    // @end
}
