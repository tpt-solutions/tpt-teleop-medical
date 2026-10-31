# tpt-t-medical-teleop

The telesurgery adapter for tpt-teleop-medical. It implements the tpt-teleop
`DomainTeleopInterface` for the surgical arm, so the shared tpt-teleop core can
drive it (spec §4.6).

## Status

Adapter done against the Domain Teleoperation Interface (DTI). It covers the
lifecycle, the guard, safety, the sensor feed and link quality (`link`). Two-site
session handling covers both paths and the link state. The network transport
(`transport`) carries surgeon commands and telemetry over sealed UDP, with a signed
handshake and a trust check on every peer.

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

The adapter itself does not talk to the network. The caller reads commands from
tpt-teleop and calls `Adapter::tick` at 1 kHz.

The transport in [`src/transport.rs`](src/transport.rs) is separate from the
adapter. It runs on two sockets, one for commands and one for telemetry, and it
uses `tpt-t-link` for framing and `tpt-t-sec` for the handshake and sealing:

- The operator (`Operator`) connects to the arm (`Arm`). Each path has its own
  signed handshake on its own socket. The operator accepts only the enrolled arm
  it expects.
- The arm accepts only enrolled operators with a role that may command it
  (`Operator` or `AiAgent`). A live session cannot be taken over until it has
  been silent for the takeover window.
- Every datagram after the handshake is sealed under the session. The arm refuses a
  command whose sequence number does not rise, and the operator refuses telemetry
  the same way.
- `Arm::recv_command` returns a `ControlCommand`, which the caller passes to
  `Adapter::on_control_command`. `Operator::recv_feed` returns a `Feed`.
- `Adapter::on_autonomous_motion` is the entry point for displacements the AI
  session has accepted. Only the surgeon's commands travel through `Arm::recv_command`.

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

- The transport runs over loopback in the tests. It has not been measured over a WAN.
- One-way latency assumes both units share a disciplined clock (PTP or GPS). The
  link monitors treat a clock fault as a lost link, but nothing here checks the clock
  itself.
- Video and point-cloud frames are not carried yet. Telemetry is a fixed set of eight
  values.
- Keys are not rotated. A session lasts until the operator goes quiet and another
  takes over.
- AI proposals are not sent over the network. Only surgeon commands are.

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
