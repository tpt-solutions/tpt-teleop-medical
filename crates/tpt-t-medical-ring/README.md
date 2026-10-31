# tpt-t-medical-ring

A wait-free single-producer/single-consumer (SPSC) ring buffer for the
haptic and tracking handoff in tpt-teleop-medical (spec.txt §4.2).

## Status

Done for the SPSC core. Verified by unit tests, loom interleaving tests and
Miri. A multi-producer variant (MPSC) is deferred.

## Overview

The crate's only public type is [`SpscRing<T>`](src/spsc.rs). A producer thread
calls `push`, a consumer thread calls `pop`, and neither waits on the other.
The buffer is fixed-size: `with_capacity` allocates once, and no operation
allocates afterwards, which is what the real-time paths need.

| Item | Purpose |
|---|---|
| `SpscRing::with_capacity` | Create a ring with a fixed capacity |
| `push` / `pop` | Hand one item from the producer to the consumer |
| `len`, `is_empty`, `is_full`, `capacity` | Occupancy queries |

## Safety

`unsafe` code is confined to the `spsc` module. Every other module in the crate
forbids it. The workspace lint `unsafe_code = "deny"` applies everywhere else.

The ring is adapted from the `tpt-t-ring` crate in the tpt-teleop repository.
Modules that this project does not use (shared memory, pointer rings, byte
casting, layout) were left out.

## Testing

```sh
# Unit tests, including the threaded stress test
cargo test -p tpt-t-medical-ring

# Loom interleaving tests (exhaustively explores thread schedules)
cargo test -p tpt-t-medical-ring --test loom --release

# Miri on the single-threaded unit tests (the 200k-item stress test is skipped)
cargo +nightly miri test -p tpt-t-medical-ring --lib -- --skip cross_thread
```

The loom tests include a deliberately weakened memory ordering, which the
tests are expected to catch. That check confirms the tests have teeth.

The loom and Miri jobs run in CI on pushes to `master` and on pull requests.

## Dependencies

None at runtime. `loom` 0.7 is a dev-time dependency, and it is only compiled
for `cfg(loom)` builds.

## Related crates

- [`tpt-t-medical-haptic`](../tpt-t-medical-haptic) reads tool samples from the
  ring at 1 kHz.
- [`tpt-t-medical-core`](../tpt-t-medical-core) uses the ring for its per-producer
  event lanes.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
