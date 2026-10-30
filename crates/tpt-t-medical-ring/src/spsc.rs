//! Wait-free SPSC ring buffer.
//!
//! # Algorithm
//!
//! Two monotonic cursors, each written by exactly one endpoint:
//!
//! * `tail` is written only by the producer. It is stored with Release after
//!   the slot is filled, which publishes the slot's contents.
//! * `head` is written only by the consumer. It is stored with Release after
//!   the slot is emptied, which hands the slot back to the producer.
//!
//! Each side loads the other side's cursor with Acquire, which establishes
//! the happens-before edge for the slot contents. `push` and `pop` never
//! loop, lock or allocate. A full ring makes `push` return the value at once,
//! and an empty ring makes `pop` return `None` at once.
//!
//! Cursors are `usize` values that only increase. Wrap-around uses wrapping
//! subtraction, which stays correct across the whole `usize` range because
//! the capacity is a power of two.
//!
//! # Concurrency contract
//!
//! Exactly one thread may call `push`, and exactly one thread may call `pop`.
//! Any other use is a caller bug. The model checker in `tests/loom.rs` covers
//! the interleavings of this protocol.

#[cfg(not(loom))]
use core::cell::UnsafeCell;
#[cfg(loom)]
use loom::cell::UnsafeCell;

#[cfg(not(loom))]
use core::sync::atomic::{AtomicUsize, Ordering};
#[cfg(loom)]
use loom::sync::atomic::{AtomicUsize, Ordering};

use core::fmt;
use core::mem::MaybeUninit;

/// A cursor on its own cache line, so the producer and consumer do not
/// invalidate each other's line on every operation.
#[repr(align(64))]
struct Cursor(AtomicUsize);

/// Stores `value` into a slot.
///
/// # Safety
///
/// The caller must be the producer, and the slot must not be readable by the
/// consumer. In the ring, that means the slot index is in `[tail, head + cap)`.
#[inline]
unsafe fn write_slot<T>(slot: &UnsafeCell<MaybeUninit<T>>, value: T) {
    #[cfg(loom)]
    // SAFETY: upheld by the caller; loom checks the access for races.
    slot.with_mut(|p| unsafe {
        (*p).write(value);
    });
    #[cfg(not(loom))]
    // SAFETY: upheld by the caller.
    unsafe {
        (*slot.get()).write(value);
    }
}

/// Moves a value out of a slot.
///
/// # Safety
///
/// The caller must be the consumer, and the slot must hold a value that was
/// published by a Release store of `tail` that the caller has observed with
/// Acquire. After this call the slot is logically uninitialised.
#[inline]
unsafe fn read_slot<T>(slot: &UnsafeCell<MaybeUninit<T>>) -> T {
    #[cfg(loom)]
    // SAFETY: upheld by the caller; loom checks the access for races.
    return slot.with(|p| unsafe { (*p).assume_init_read() });
    #[cfg(not(loom))]
    // SAFETY: upheld by the caller.
    return unsafe { (*slot.get()).assume_init_read() };
}

/// A heap-allocated, bounded, wait-free SPSC ring.
///
/// Share it as `Arc<SpscRing<T>>`, with one thread holding the producer side
/// and one thread holding the consumer side.
pub struct SpscRing<T> {
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
    /// Consumer cursor: the next index to pop.
    head: Cursor,
    /// Producer cursor: the next index to push.
    tail: Cursor,
}

// SAFETY: a value crosses threads only when the producer hands it to the
// consumer, so this is sound when `T: Send`. Each slot is owned by one side
// at a time, and the cursors enforce that with Acquire/Release ordering.
unsafe impl<T: Send> Send for SpscRing<T> {}
// SAFETY: `&SpscRing` is the sharing mode. Mutation goes only through the
// slot protocol and the atomic cursors, as described in the Send impl above.
unsafe impl<T: Send> Sync for SpscRing<T> {}

impl<T> SpscRing<T> {
    /// Creates a ring that holds at least `capacity` elements.
    ///
    /// The capacity is rounded up to a power of two, with a minimum of 1.
    /// All allocation happens here; `push` and `pop` never allocate.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.max(1).next_power_of_two();
        let slots = (0..cap)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect();
        Self {
            slots,
            head: Cursor(AtomicUsize::new(0)),
            tail: Cursor(AtomicUsize::new(0)),
        }
    }

    /// Returns the maximum number of elements in flight. This is a power of two.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Returns the current element count.
    ///
    /// The count is exact when called from one endpoint. Called from both
    /// endpoints at once, it is only a bounded estimate.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        let tail = self.tail.0.load(Ordering::Acquire);
        let head = self.head.0.load(Ordering::Acquire);
        tail.wrapping_sub(head)
    }

    /// Returns `true` when no elements are pending.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` when the next `push` would fail.
    #[inline]
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    /// Enqueues `value`. Returns `Err(value)` if the ring is full.
    ///
    /// Must be called only from the producer thread. It is wait-free: the
    /// instruction sequence is fixed and it never spins.
    ///
    /// # Errors
    ///
    /// Returns `Err` with the rejected value when the ring is full.
    #[inline]
    pub fn push(&self, value: T) -> Result<(), T> {
        // Relaxed is enough for our own cursor, since only this thread writes it.
        let tail = self.tail.0.load(Ordering::Relaxed);
        // Acquire pairs with the consumer's Release store of `head`, so we see
        // which slots have been freed before we claim one.
        let head = self.head.0.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= self.capacity() {
            return Err(value);
        }
        let slot = &self.slots[tail & (self.capacity() - 1)];
        // SAFETY: we are the producer. `tail - head < cap` proves the slot at
        // `tail & mask` was consumed before, so it is free for us to write.
        unsafe { write_slot(slot, value) };
        // Release publishes the slot contents before the new tail is visible.
        self.tail.0.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Dequeues the oldest element, or returns `None` if the ring is empty.
    ///
    /// Must be called only from the consumer thread. It is wait-free.
    #[inline]
    pub fn pop(&self) -> Option<T> {
        // Relaxed is enough for our own cursor, since only this thread writes it.
        let head = self.head.0.load(Ordering::Relaxed);
        // Acquire pairs with the producer's Release store of `tail`, so the
        // slot contents are visible once we see the new tail.
        if head == self.tail.0.load(Ordering::Acquire) {
            return None;
        }
        let slot = &self.slots[head & (self.capacity() - 1)];
        // SAFETY: we are the consumer. `head != tail` means the producer has
        // published this slot with a Release store that we have just observed.
        let value = unsafe { read_slot(slot) };
        // Release hands the slot back to the producer after the read.
        self.head.0.store(head.wrapping_add(1), Ordering::Release);
        Some(value)
    }
}

impl<T> Drop for SpscRing<T> {
    fn drop(&mut self) {
        // Drain the ring so each unconsumed value is dropped exactly once.
        while self.pop().is_some() {}
    }
}

impl<T> fmt::Debug for SpscRing<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpscRing")
            .field("capacity", &self.capacity())
            .field("len", &self.len())
            .finish()
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize as Counter, Ordering as CounterOrdering};
    use std::thread;

    #[test]
    fn fifo_order_is_preserved_across_wraparound() {
        let ring: SpscRing<u64> = SpscRing::with_capacity(4);
        let mut next_push = 0u64;
        let mut next_pop = 0u64;
        // Keep the ring between one and three elements full so the cursors
        // wrap the mask many times.
        for _ in 0..1_000 {
            for _ in 0..3 {
                ring.push(next_push).expect("ring has room");
                next_push += 1;
            }
            for _ in 0..3 {
                assert_eq!(ring.pop(), Some(next_pop));
                next_pop += 1;
            }
        }
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn full_and_empty_semantics() {
        let ring: SpscRing<u32> = SpscRing::with_capacity(4);
        assert_eq!(ring.capacity(), 4);
        assert_eq!(ring.len(), 0);
        for i in 0..4 {
            assert!(ring.push(i).is_ok());
        }
        assert!(ring.is_full());
        assert_eq!(ring.push(99), Err(99));
        assert_eq!(ring.len(), 4);
        assert_eq!(ring.pop(), Some(0));
        assert!(!ring.is_full());
        assert!(ring.push(99).is_ok());
    }

    #[test]
    fn capacity_rounds_up_to_power_of_two() {
        let ring: SpscRing<u8> = SpscRing::with_capacity(5);
        assert_eq!(ring.capacity(), 8);
        let minimum: SpscRing<u8> = SpscRing::with_capacity(0);
        assert_eq!(minimum.capacity(), 1);
    }

    #[test]
    fn cross_thread_transfer_preserves_every_value() {
        const N: u64 = 200_000;
        let ring = Arc::new(SpscRing::<u64>::with_capacity(1024));
        let producer_ring = Arc::clone(&ring);

        let producer = thread::spawn(move || {
            for i in 0..N {
                while producer_ring.push(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        });
        let consumer = thread::spawn(move || {
            let mut expected = 0u64;
            while expected < N {
                match ring.pop() {
                    Some(v) => {
                        assert_eq!(v, expected);
                        expected += 1;
                    }
                    None => std::hint::spin_loop(),
                }
            }
        });

        producer.join().expect("producer panicked");
        consumer.join().expect("consumer panicked");
    }

    #[test]
    fn unconsumed_values_drop_exactly_once() {
        static DROPS: Counter = Counter::new(0);
        struct Counted(#[allow(dead_code)] u32);
        impl Drop for Counted {
            fn drop(&mut self) {
                DROPS.fetch_add(1, CounterOrdering::SeqCst);
            }
        }

        {
            let ring: SpscRing<Counted> = SpscRing::with_capacity(8);
            for i in 0..5 {
                assert!(ring.push(Counted(i)).is_ok());
            }
            let first = ring.pop().expect("ring holds five values");
            drop(first);
            assert_eq!(DROPS.load(CounterOrdering::SeqCst), 1);
            // Four values remain in the ring. Dropping the ring must drop them.
        }
        assert_eq!(DROPS.load(CounterOrdering::SeqCst), 5);
    }
}
