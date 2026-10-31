# tpt-t-medical-core

The central event loop and surgical state machine for tpt-teleop-medical
(spec.txt §3, Phase 1). This is the one place that decides whether a command
is allowed.

## Status

Logic done. The state machine, tick loop, per-producer lanes and the audited
wrapper are implemented and tested. There is no real-time scheduling here.

## Overview

The state machine has five states:

```
Idle → Registered → Active ⇄ Paused
   (any state) ──EmergencyStop──▶ Emergency ──ClearEmergency──▶ Idle
```

| State | Meaning |
|---|---|
| `Idle` | No surgeon registered. No motion commands are accepted. |
| `Registered` | A surgeon is registered but has not activated control. |
| `Active` | Commands are accepted and the arm follows the master. |
| `Paused` | Control is held. The arm stays where it is. |
| `Emergency` | Latched stop. Only `ClearEmergency` leaves it, and only to `Idle`. |

Events are the inputs to the machine: `Register`, `Deregister`, `Activate`,
`Pause`, `Resume`, `EmergencyStop` and `ClearEmergency`.

### Main types

| Item | Module | Purpose |
|---|---|---|
| `State`, `Event` | `state` | The machine's states and inputs |
| `Machine`, `transition` | `state` | Pure transition logic; returns `InvalidTransition` for a refused move |
| `EventLoop<const LANES>` | `event_loop` | Drains one SPSC lane per producer on each tick |
| `TickReport` | `event_loop` | What one tick did |
| `EMERGENCY_LANE` | `event_loop` | Lane 0, drained first |
| `AuditedLoop<LANES, S, G>` | `audit` | The loop plus a compliance journal |

### Ordering and emergency handling

Producers push `Event`s into their own lanes. Each tick drains lane 0 first, so
an emergency stop is never queued behind routine traffic.

`AuditedLoop` writes every event to the journal before it takes effect. A safety
stop is journalled as a reason record followed by the transition. A stop takes
effect even if the journal write fails, because the alternative is to keep
moving while the system cannot record that it should not.

## Design constraints

- The state machine is pure logic. It does not allocate, perform I/O or read a
  clock.
- Only the event lanes allocate, and only at construction.
- Event and state codes are fixed, because the audit format depends on them.

## Dependencies

- [`tpt-t-medical-compliance`](../tpt-t-medical-compliance): the audit journal.
- [`tpt-t-medical-ring`](../tpt-t-medical-ring): the SPSC lanes.
- [`tpt-t-medical-safety`](../tpt-t-medical-safety): stop sources.

## Testing

```sh
cargo test -p tpt-t-medical-core
```

## Limitations

- No real-time pinning, scheduling or timing measurement. Those need the target
  hardware.
- The loop is driven by the caller. Nothing here runs a thread.

## Related crates

- [`tpt-t-medical-safety`](../tpt-t-medical-safety) decides when a stop is
  needed. Its output becomes an `EmergencyStop` event.
- [`tpt-t-medical-teleop`](../tpt-t-medical-teleop) is the telesurgery adapter
  that connects this system to the tpt-teleop core.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
