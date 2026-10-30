//! Exhaustive interleaving checks for the SPSC protocol, run under loom.
//!
//! Run with:
//! `RUSTFLAGS="--cfg loom" cargo test -p tpt-t-medical-ring --test loom --release`

#![cfg(loom)]

use loom::sync::Arc;
use loom::thread;
use tpt_t_medical_ring::SpscRing;

/// Two values pushed through a ring of capacity 1 arrive in order. The
/// producer retries when the ring is full, so this exercises the full path
/// and the slot reuse after a pop.
#[test]
fn capacity_one_forces_slot_reuse_in_order() {
    loom::model(|| {
        let ring = Arc::new(SpscRing::<u32>::with_capacity(1));
        let producer_ring = Arc::clone(&ring);

        let producer = thread::spawn(move || {
            for i in 0..2 {
                while producer_ring.push(i).is_err() {
                    thread::yield_now();
                }
            }
        });

        let mut expected = 0;
        while expected < 2 {
            match ring.pop() {
                Some(v) => {
                    assert_eq!(v, expected);
                    expected += 1;
                }
                None => thread::yield_now(),
            }
        }
        producer.join().expect("producer panicked");
        assert!(ring.is_empty());
    });
}

/// A value written before the producer's Release store must be visible to
/// the consumer after its Acquire load of `tail`. Here the payload is a
/// `String`, so a missing happens-before edge would show up as a use of
/// uninitialised or freed memory under loom's tracking.
#[test]
fn payload_is_visible_after_publish() {
    loom::model(|| {
        let ring = Arc::new(SpscRing::<String>::with_capacity(2));
        let producer_ring = Arc::clone(&ring);

        let producer = thread::spawn(move || {
            while producer_ring.push(String::from("scalpel")).is_err() {
                thread::yield_now();
            }
        });

        let value = loop {
            if let Some(v) = ring.pop() {
                break v;
            }
            thread::yield_now();
        };
        assert_eq!(value, "scalpel");
        producer.join().expect("producer panicked");
    });
}

/// Values still in the ring when it is dropped are dropped exactly once,
/// even when the consumer and producer are racing.
#[test]
fn drop_after_concurrent_use_is_clean() {
    loom::model(|| {
        let ring = Arc::new(SpscRing::<String>::with_capacity(2));
        let producer_ring = Arc::clone(&ring);

        let producer = thread::spawn(move || {
            let _ = producer_ring.push(String::from("a"));
            let _ = producer_ring.push(String::from("b"));
        });

        let _ = ring.pop();
        producer.join().expect("producer panicked");
        drop(ring);
    });
}
