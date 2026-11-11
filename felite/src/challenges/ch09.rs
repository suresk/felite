//! Chapter 9 challenges: replacement policies compared on traces, and a pool that
//! hands out shared pages and pins them.

use crate::error::{Error, Result};
use crate::page::{PAGE_SIZE, Page, PageNo};
use crate::storage::Storage;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

// @listing sim-clock
/// How many of the page requests in `trace` miss, for a pool of `capacity` frames
/// run by the clock. If `new_is_recent`, a page counts as recently used from the
/// moment it is loaded. The pager's clock does not do that.
pub fn clock_misses(capacity: usize, trace: &[PageNo], new_is_recent: bool) -> usize {
    let mut clock: VecDeque<PageNo> = VecDeque::new();
    let mut recent: HashSet<PageNo> = HashSet::new();
    let mut misses = 0;
    for &n in trace {
        if clock.contains(&n) {
            recent.insert(n);
            continue;
        }
        misses += 1;
        while clock.len() >= capacity {
            let Some(victim) = clock.pop_front() else { break };
            if recent.remove(&victim) {
                clock.push_back(victim);
            }
        }
        clock.push_back(n);
        if new_is_recent {
            recent.insert(n);
        }
    }
    misses
}
// @end

// @listing sim-lru
/// The same count for a pool that always drops the page used longest ago.
pub fn lru_misses(capacity: usize, trace: &[PageNo]) -> usize {
    // The pages in memory, least recently used first.
    let mut order: VecDeque<PageNo> = VecDeque::new();
    let mut misses = 0;
    for &n in trace {
        if let Some(at) = order.iter().position(|&p| p == n) {
            order.remove(at);
        } else {
            misses += 1;
            if order.len() >= capacity {
                order.pop_front();
            }
        }
        order.push_back(n);
    }
    misses
}
// @end

// @listing shared-pool
/// A page that several holders can share. Holding one keeps it in memory.
pub type Shared = Rc<RefCell<Page>>;

/// A pool that hands out shared pages. A page with a holder is pinned: it cannot
/// be dropped until every handle to it has gone.
pub struct SharedPool {
    storage: Box<dyn Storage>,
    capacity: usize,
    frames: HashMap<PageNo, Shared>,
}

impl SharedPool {
    pub fn new(storage: Box<dyn Storage>, capacity: usize) -> SharedPool {
        SharedPool { storage, capacity, frames: HashMap::new() }
    }

    /// Page `n`, shared. Fails if the pool is full and every page in it is held.
    pub fn get(&mut self, n: PageNo) -> Result<Shared> {
        if let Some(page) = self.frames.get(&n) {
            return Ok(Rc::clone(page));
        }
        if self.frames.len() >= self.capacity {
            // A count of 1 means the pool's own handle is the only one.
            let mut unpinned: Vec<PageNo> = self
                .frames
                .iter()
                .filter(|(_, p)| Rc::strong_count(p) == 1)
                .map(|(n, _)| *n)
                .collect();
            unpinned.sort();
            let Some(victim) = unpinned.first() else {
                return Err(Error::Corrupt(
                    "every page in the pool is pinned".into(),
                ));
            };
            self.frames.remove(victim);
        }
        let mut page = Page::new();
        self.storage.read_at(n as u64 * PAGE_SIZE as u64, page.bytes_mut())?;
        let page = Rc::new(RefCell::new(page));
        self.frames.insert(n, Rc::clone(&page));
        Ok(page)
    }
}
// @end
