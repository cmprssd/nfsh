//! Memory entries are decoded straight into for the GPU to copy from: a ring handed out
//! in slots, each given back when dropped, so a decoded texture is never copied again
//! on the processor before its upload (docs/BASELINE.md, "Decoding into the staging
//! memory").

use std::any::Any;
use std::collections::VecDeque;
use std::ops::Deref;
use std::sync::{Arc, Mutex, PoisonError};

/// Where each slot starts: a multiple of every block size and of the copy alignment
/// devices prefer.
pub const ALIGN: usize = 256;

/// Memory handed out in slots, in order, around and around: a slot takes the bytes
/// after the last one given, or the ring's start when they don't fit before its end,
/// and only bytes no slot holds any more. Slots dropped out of order are given back
/// once those before them are.
pub struct Ring {
    base: *mut u8,
    len: usize,
    /// What holds the memory (a mapped GPU buffer), for whoever copies from a slot.
    owner: Box<dyn Any + Send + Sync>,
    state: Mutex<State>,
}

// SAFETY: `base` points into memory `owner` keeps alive, and each byte of it is
// reached only through the one slot holding it.
unsafe impl Send for Ring {}
unsafe impl Sync for Ring {}

/// Positions count bytes from the ring's first slot, never wrapping: a position's
/// place in the memory is it modulo the ring's length.
#[derive(Default)]
struct State {
    /// Where the next slot may start.
    head: u64,
    /// Each slot given and not yet given back, oldest first: its start and end, and
    /// whether it has been dropped (waiting for an older one).
    live: VecDeque<(u64, u64, bool)>,
}

impl Ring {
    /// A ring over the memory `memory` finds in `owner`, which keeps it; its length
    /// rounded down to a multiple of [`ALIGN`].
    ///
    /// # Safety
    ///
    /// The memory must stay where it is, and be used by nothing else, for as long as
    /// `owner` lives, wherever `owner` is moved: a GPU buffer's mapping or a `Vec`'s
    /// heap memory, not an array inside `owner`.
    pub unsafe fn new<M: Any + Send + Sync>(
        mut owner: M,
        memory: impl FnOnce(&mut M) -> &mut [u8],
    ) -> Arc<Self> {
        let memory = memory(&mut owner);
        let (base, len) = (memory.as_mut_ptr(), memory.len() / ALIGN * ALIGN);
        Arc::new(Self {
            base,
            len,
            owner: Box::new(owner),
            state: Mutex::default(),
        })
    }

    /// A slot of `len` bytes, or `None` while the bytes it needs are held by others
    /// (or for none, or more than the ring holds).
    pub fn slot(self: &Arc<Self>, len: usize) -> Option<Slot> {
        if len == 0 || len > self.len {
            return None;
        }
        let ring = self.len as u64;
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let tail = s.live.front().map_or(s.head, |l| l.0);
        let mut start = s.head.next_multiple_of(ALIGN as u64);
        let at = start % ring;
        if at + len as u64 > ring {
            start += ring - at;
        }
        let end = start + len as u64;
        if end - tail > ring {
            return None;
        }
        s.head = end;
        s.live.push_back((start, end, false));
        Some(Slot {
            ring: self.clone(),
            start,
            offset: (start % ring) as usize,
            len,
        })
    }

    /// What holds the ring's memory.
    pub fn owner(&self) -> &(dyn Any + Send + Sync) {
        &*self.owner
    }

    /// Bytes held by slots, including those dropped but waiting for an older one.
    pub fn held(&self) -> u64 {
        let s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        s.live.front().map_or(0, |l| s.head - l.0)
    }

    fn give_back(&self, start: u64) {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Ok(i) = s.live.binary_search_by_key(&start, |l| l.0) {
            s.live[i].2 = true;
        }
        while s.live.front().is_some_and(|l| l.2) {
            s.live.pop_front();
        }
    }
}

/// Bytes of a [`Ring`], given back when dropped.
pub struct Slot {
    ring: Arc<Ring>,
    start: u64,
    offset: usize,
    len: usize,
}

impl Slot {
    /// Where the slot starts in the ring's memory.
    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: the ring gives each byte to one live slot at a time, and this
        // borrows the slot mutably.
        unsafe { std::slice::from_raw_parts_mut(self.ring.base.add(self.offset), self.len) }
    }
}

impl Deref for Slot {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        // SAFETY: as `bytes_mut`, borrowed shared.
        unsafe { std::slice::from_raw_parts(self.ring.base.add(self.offset), self.len) }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.ring.give_back(self.start);
    }
}

/// An entry's decoded bytes: in memory of their own, or in a ring's slot.
pub enum Bytes {
    Heap(Vec<u8>),
    Staged(Slot),
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Bytes::Heap(v) => v,
            Bytes::Staged(s) => s,
        }
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(v: Vec<u8>) -> Self {
        Bytes::Heap(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(len: usize) -> Arc<Ring> {
        // SAFETY: a Vec's heap memory stays put when the Vec moves.
        unsafe { Ring::new(vec![0u8; len], |v| &mut v[..]) }
    }

    #[test]
    fn slots_follow_each_other_aligned() {
        let r = ring(4096);
        let a = r.slot(100).unwrap();
        let b = r.slot(300).unwrap();
        let c = r.slot(1).unwrap();
        assert_eq!([a.offset(), b.offset(), c.offset()], [0, 256, 768]);
        assert_eq!(r.held(), 769);
    }

    #[test]
    fn a_full_ring_gives_no_slot_until_the_oldest_is_dropped() {
        let r = ring(1024);
        let a = r.slot(512).unwrap();
        let b = r.slot(512).unwrap();
        assert!(r.slot(1).is_none());
        drop(b);
        // `a` still holds the start, and `b`'s bytes wait for it.
        assert!(r.slot(1).is_none());
        assert_eq!(r.held(), 1024);
        drop(a);
        assert_eq!(r.held(), 0);
        assert_eq!(r.slot(1024).unwrap().offset(), 0);
    }

    #[test]
    fn a_slot_that_would_pass_the_end_starts_over_at_the_start() {
        let r = ring(1024);
        let a = r.slot(600).unwrap();
        let b = r.slot(100).unwrap();
        assert_eq!(b.offset(), 768);
        // 512 bytes don't fit after `b`, and the start is `a`'s.
        assert!(r.slot(512).is_none());
        drop(a);
        let c = r.slot(512).unwrap();
        assert_eq!(c.offset(), 0);
        // Between `c` and `b` there are 256 bytes.
        assert_eq!(r.slot(256).unwrap().offset(), 512);
        assert!(r.slot(1).is_none());
    }

    #[test]
    fn no_slot_for_nothing_or_more_than_the_ring() {
        let r = ring(1000);
        assert!(r.slot(0).is_none());
        // Rounded down to 768 bytes.
        assert!(r.slot(769).is_none());
        assert!(r.slot(768).is_some());
    }

    #[test]
    fn each_slot_keeps_its_own_bytes() {
        let r = ring(1024);
        let mut a = r.slot(3).unwrap();
        let mut b = r.slot(2).unwrap();
        a.bytes_mut().copy_from_slice(&[1, 2, 3]);
        b.bytes_mut().copy_from_slice(&[9, 8]);
        assert_eq!((&a[..], &b[..]), (&[1, 2, 3][..], &[9, 8][..]));
        assert_eq!(&Bytes::Staged(b)[..], &[9, 8]);
        assert!(r.owner().downcast_ref::<Vec<u8>>().is_some());
    }
}
