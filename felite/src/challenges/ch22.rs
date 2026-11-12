//! Chapter 22 challenges: a join of two sorted inputs, and a hash join that
//! works on one part of its input at a time.

use std::collections::HashMap;

// @listing merge-join
/// Joins two lists that are each sorted by key, by walking down both at once.
/// Returns every pair of entries with equal keys.
pub fn merge_join<A: Clone, B: Clone>(
    left: &[(i64, A)],
    right: &[(i64, B)],
) -> Vec<(i64, A, B)> {
    let mut joined = Vec::new();
    let (mut l, mut r) = (0, 0);
    while l < left.len() && r < right.len() {
        let key = left[l].0;
        if key < right[r].0 {
            l += 1;
        } else if key > right[r].0 {
            r += 1;
        } else {
            // A run of equal keys on each side. Every entry of one pairs with
            // every entry of the other.
            let run = right[r..].iter().take_while(|(k, _)| *k == key).count();
            while l < left.len() && left[l].0 == key {
                for (_, b) in &right[r..r + run] {
                    joined.push((key, left[l].1.clone(), b.clone()));
                }
                l += 1;
            }
            r += run;
        }
    }
    joined
}
// @end

// @listing partitioned
/// Joins two lists on their keys with a hash table that never holds more than
/// one partition of `right`. Returns the pairs, and the size of the largest
/// partition that was built.
pub fn partitioned_join<A: Clone, B: Clone>(
    left: &[(i64, A)],
    right: &[(i64, B)],
    partitions: usize,
) -> (Vec<(i64, A, B)>, usize) {
    let partitions = partitions.max(1);
    // Equal keys land in the same partition on both sides, so a partition of
    // one side can only match the same partition of the other.
    let part = |key: i64| key.rem_euclid(partitions as i64) as usize;
    let mut left_parts = vec![Vec::new(); partitions];
    let mut right_parts = vec![Vec::new(); partitions];
    for entry in left {
        left_parts[part(entry.0)].push(entry);
    }
    for entry in right {
        right_parts[part(entry.0)].push(entry);
    }

    let (mut joined, mut largest) = (Vec::new(), 0);
    for (probe, build) in left_parts.iter().zip(&right_parts) {
        let mut table: HashMap<i64, Vec<&B>> = HashMap::new();
        for (key, b) in build {
            table.entry(*key).or_default().push(b);
        }
        largest = largest.max(build.len());
        for (key, a) in probe {
            for b in table.get(key).into_iter().flatten() {
                joined.push((*key, a.clone(), (*b).clone()));
            }
        }
    }
    (joined, largest)
}
// @end
