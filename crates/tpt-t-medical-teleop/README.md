# tpt-t-medical-teleop

The telesurgery adapter for tpt-teleop-medical. It implements the tpt-teleop
`DomainTeleopInterface` for the surgical arm, so the shared tpt-teleop core can
drive it (spec §4.6).

## Status

Adapter done against the Domain Teleoperation Interface (DTI). It covers the
lifecycle, the guard, safety and the sensor feed. Multi-site session handling is
not started.

## Overview

[`Adapter`](src/adapter.rs) implements `tpt-t-domain-bridge`'s
`DomainTeleopInterface`. Wrap it in `DomainSession` to get the handover
lifecycle, which rejects out-of-order calls before they reach the adapter.

Each command passes through, in order:

1. A 10 Hz tremor notch (`tpt-t-medical-surgical`).
2. Motion scaling (`tpt-t-medical-surgical`).
3. The virtual-fixture guard (`tpt-t-medical-surgical`), which can block the
   motion or override the master.
4. The tool target. The tool is tracked by a spring and a damper through the edge
   torque pipeline (`tpt-t-medical-edge`).

The safety monitor (`tpt-t-medical-safety`) runs on every tick. A stop zeroes the
torque and latches. Engage starts at the current tool position. Disengage ramps
the tracking force away while gravity compensation holds the arm.

### Main types

| Item | Purpose |
|---|---|
| `Adapter<A: Actuators>` | The DTI implementation. `tick` runs once per 1 kHz cycle. |
| `Actuators` | The trait the caller implements to receive torques |
| `Config` | Arm model, workspace envelope, force limit, watchdog, motion scaler, step size and tracking stiffness |
| `JointReading` | One joint's measured position and velocity |

Other methods: `push_frame`, `update_joints`, `update_force`, `set_battery`,
`tool_position`, `target`, `last_torque`, `is_latched`, `actuators`.

## Network and sibling checkout

The adapter does not talk to the network. The caller reads commands from
tpt-teleop and calls `Adapter::tick` at 1 kHz.

This crate depends on the tpt-teleop core by a relative path:

```
../../../tpt-teleop/crates/tpt-t-core
../../../tpt-teleop/crates/tpt-t-domain-bridge
```

So the workspace needs a sibling checkout of `tpt-teleop` next to this
repository. See the [workspace README](../../README.md#building).

## Testing

```sh
cargo test -p tpt-t-medical-teleop
```

The integration tests in `tests/adapter.rs` drive the adapter through
`DomainSession`.

## Limitations

- No multi-site teleoperation sessions.
- The link to the surgeon is provided by tpt-teleop. Latency is not measured here.

## Dependencies

- `tpt-t-core` and `tpt-t-domain-bridge` (tpt-teleop, by path)
- [`tpt-t-medical-edge`](../tpt-t-medical-edge)
- [`tpt-t-medical-safety`](../tpt-t-medical-safety)
- [`tpt-t-medical-surgical`](../tpt-t-medical-surgical)

## Related crates

- [`tpt-t-medical-core`](../tpt-t-medical-core) is the medical-side state machine.
- [`tpt-t-medical-haptic`](../tpt-t-medical-haptic) renders the force that goes
  back to the surgeon.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
