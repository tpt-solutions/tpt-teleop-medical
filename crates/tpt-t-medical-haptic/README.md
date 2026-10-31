# tpt-t-medical-haptic

The 1 kHz haptic rendering loop for tpt-teleop-medical (spec.txt §4.2). It turns
the newest tool sample into a contact force that the surgeon feels, and it holds
the last force rather than dropping to zero when something goes wrong.

## Status

Contact model and hold logic done. Pinned-core timing is not implemented, and
the timing claims (under 50 µs execution, 1 ms deadline) are not verified here.

## Overview

`HapticLoop::step` runs once per tick:

1. It reads the newest sample from the lock-free ring
   ([`tpt-t-medical-ring`](../tpt-t-medical-ring)).
2. It renders the contact force against the surface model.
3. It returns an `Output` whose `Status` says what happened.

The loop reads the newest sample, not the oldest. Stale samples are dropped, so
the surgeon never feels a force that is out of date.

### Hold-last-value cases

Each of these holds the previous force. None of them zeroes it, because a sudden
drop would be felt as a jolt.

| `Status` | When |
|---|---|
| `Rendered` | A fresh sample was rendered. This is the normal case. |
| `HeldDeadlineMissed` | The tick took longer than `DEADLINE_US` (1 000 µs) |
| `HeldNoInput` | No sample was waiting |
| `HeldStale` | The newest sample is older than `STALE_US` (2 000 µs) |
| `HeldInvalid` | The sample contained a non-finite value |

### Contact model

`contact_force` applies an impedance model against a plane. `Surface::new`
normalises the normal and returns `None` for non-finite input or a zero normal.
`Tissue::new` takes a stiffness and a damping constant, both finite and
non-negative.

### Main types

| Item | Purpose |
|---|---|
| `HapticLoop` | The 1 kHz loop. Created with `new`, driven with `step`. |
| `HapticInput` | Timestamp, position and velocity of the tool |
| `Output` | The force and the `Status` for this tick |
| `Surface`, `Tissue` | Contact geometry and tissue parameters |
| `contact_force` | The impedance model on its own |

## Testing

```sh
cargo test -p tpt-t-medical-haptic
```

## Limitations

- No haptic device model. The spec does not define one.
- No haptic rendering against a real device or a pinned core.
- The tests cover the logic that decides when to hold, not the timing.

## Dependencies

- [`tpt-t-medical-ring`](../tpt-t-medical-ring) for the sample handoff.

## Related crates

- [`tpt-t-medical-sim`](../tpt-t-medical-sim) does not model haptic rendering.
- [`tpt-t-medical-teleop`](../tpt-t-medical-teleop) is the adapter that connects
  the surgical arm to the tpt-teleop core, which carries the force back to the
  surgeon.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
