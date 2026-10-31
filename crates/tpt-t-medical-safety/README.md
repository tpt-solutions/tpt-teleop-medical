# tpt-t-medical-safety

The safety monitor for tpt-teleop-medical (spec.txt §4.1, §5.5): workspace
envelope, force limit, watchdog, latched emergency stop and a triple modular
redundancy (TMR) voter.

## Status

Pure logic done. The timing targets in the spec are not verified here, and
the requirement for the three copies to run on isolated cores is not met.

## Overview

The monitor takes a `Sample` of the tool's state and returns a `Decision`:

- `Decision::Run`: motion may continue.
- `Decision::Stop(StopReason)`: the system must go to its safe state.

### Rules

1. The tool must stay inside the workspace `Envelope`.
2. The force on the tool must not exceed the limit. A NaN or infinite reading
   counts as a violation.
3. The monitor must be evaluated at least once per watchdog timeout. A longer
   gap, or a clock that moves backwards, is a violation.
4. Any stop is latched. Only an explicit `Monitor::reset` clears it.

### Stop reasons

| `StopReason` | Cause |
|---|---|
| `Disagreement` | The three TMR copies disagreed |
| `OutsideEnvelope` | The tool left the workspace envelope |
| `ForceExceeded` | Force over the limit, or not a finite number |
| `WatchdogExpired` | The monitor was not evaluated in time, or the clock moved backwards |
| `EStop(EStopSource)` | An emergency stop, from `FootPedal`, `HandSwitch`, `Voice` or `Automatic` |

### Main types

| Item | Purpose |
|---|---|
| `Limits` | Envelope, force limit and watchdog timeout. `Limits::new` returns `None` for invalid values. |
| `Envelope` | The workspace the tool may not leave. `contains` tests a point. |
| `Monitor` | One copy of the monitor. `evaluate`, `emergency_stop`, `reset`, `latched`. |
| `Tmr` | Three monitors voting together. `Tmr::from_monitors` builds one. |
| `vote` | Returns the value on which all three copies agree. Any disagreement is a `Disagreement` stop. |

The monitor allocates nothing and reads no clock. Time comes in on each sample,
which keeps the logic deterministic and easy to test.

## Testing

```sh
cargo test -p tpt-t-medical-safety
```

## Limitations

- The spec's timing targets (under 100 µs disagreement response, under 10 ms
  E-stop) are not verified. They need the real-time target.
- Core isolation for the three copies is not implemented.

## Dependencies

None at runtime.

## Related crates

- [`tpt-t-medical-core`](../tpt-t-medical-core) turns a stop into an
  `EmergencyStop` event.
- [`tpt-t-medical-ai-session`](../tpt-t-medical-ai-session) latches an automatic
  stop when an AI proposal breaks the envelope or force limit.
- [`tpt-t-medical-edge`](../tpt-t-medical-edge) runs the torque pipeline that a
  safe state must stop.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
