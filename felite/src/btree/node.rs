//! What is inside the cells of a tree page.

use crate::error::{Error, Result};
use crate::page::PageNo;
use crate::record::{read_varint, take};
use crate::varint::put_varint;

// @listing kinds
/// A page whose cells hold keys and their payloads.
pub const LEAF: u8 = 1;
/// A page whose cells hold keys and the numbers of child pages.
pub const INTERIOR: u8 = 2;
// @end

// @listing leaf-cell
/// A leaf cell: the key's length, the key, then the payload.
pub fn leaf_cell(key: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut cell = Vec::with_capacity(key.len() + payload.len() + 2);
    put_varint(&mut cell, key.len() as u64);
    cell.extend_from_slice(key);
    cell.extend_from_slice(payload);
    cell
}

/// The key and the payload of a leaf cell.
pub fn parse_leaf(cell: &[u8]) -> Result<(&[u8], &[u8])> {
    let mut rest = cell;
    let key_len = read_varint(&mut rest)?;
    let key = take(&mut rest, key_len)?;
    Ok((key, rest))
}
// @end

// @listing interior-cell
/// An interior cell: a child's page number, then the lowest key in that child.
pub fn interior_cell(child: PageNo, key: &[u8]) -> Vec<u8> {
    let mut cell = Vec::with_capacity(4 + key.len());
    cell.extend_from_slice(&child.to_be_bytes());
    cell.extend_from_slice(key);
    cell
}

/// The child and the key of an interior cell.
pub fn parse_interior(cell: &[u8]) -> Result<(PageNo, &[u8])> {
    if cell.len() < 4 {
        let len = cell.len();
        return Err(Error::Corrupt(format!("an interior cell of {len} bytes")));
    }
    let child = PageNo::from_be_bytes([cell[0], cell[1], cell[2], cell[3]]);
    Ok((child, &cell[4..]))
}
// @end

// @listing row-key
/// A row id as a key: eight bytes that sort, as bytes, in numeric order.
pub fn row_key(id: i64) -> [u8; 8] {
    (id as u64 ^ (1 << 63)).to_be_bytes()
}

/// The row id that a key stands for.
pub fn row_id(key: &[u8]) -> Result<i64> {
    match <[u8; 8]>::try_from(key) {
        Ok(bytes) => Ok((u64::from_be_bytes(bytes) ^ (1 << 63)) as i64),
        Err(_) => Err(Error::Corrupt(format!("a row id key of {} bytes", key.len()))),
    }
}
// @end
