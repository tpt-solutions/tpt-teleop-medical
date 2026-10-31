# tpt-t-medical-sim

A headless simulator core for tpt-teleop-medical (spec.txt §8). It models a
seven-joint arm at a fixed 1 kHz step, with fault injection, command latency and
jitter, a mass-spring tissue, and the tool's position from the joint readings.

## Status

Headless core and haptic force rendering (`haptic`) done. The visualizer is not modelled.

## Overview

The simulator is deterministic. Nothing reads the wall clock and nothing draws
random numbers, so a run can be repeated exactly. Each step is `STEP_US`
(1 000 µs), and the arm has `JOINTS` (7) joints.

| Module | Main types | Purpose |
|---|---|---|
| root | `Simulator`, `Readings`, `Fault`, `FaultError` | The arm, its readings, and fault injection and clearing |
| `joint` | `Joint`, `JointParams`, `backlash` | Per-joint dynamics: gravity, viscous and Coulomb friction, and a backlash operator on the reported position |
| `link` | `Link` | Command latency and deterministic jitter (`send`, `receive`, `in_flight`, `MAX_IN_FLIGHT`) |
| `tissue` | `Tissue`, `Spring` | A mass-spring tissue that the tool pushes on (`MAX_NODES`, `MAX_SPRINGS`) |

Tool position is computed from the joint readings using the arm's kinematics from
[`tpt-t-medical-surgical`](../tpt-t-medical-surgical).

### Faults

| `Fault` | Effect |
|---|---|
| `SensorFailure(index)` | The joint stops reporting its position |
| `MotorStall(index)` | The joint's motor stops producing torque |
| `NetworkDrop` | Commands stop arriving. Joints get zero torque until the link returns. |

`FaultError::NoSuchJoint` is returned for an index outside `0..JOINTS`. Non-finite
commands are refused.

## Testing

```sh
cargo test -p tpt-t-medical-sim
```

The integration test `tests/ai_session_faults.rs` runs five AI fault scenarios
against [`tpt-t-medical-ai-session`](../tpt-t-medical-ai-session). These are
development-only dependencies.

## Limitations

- The haptic device is modelled by the impedance contact of `tpt-t-medical-haptic`. The spec defines no device, so this is a stand-in.
- No visualizer. The GUI is in a separate crate that is not started.
- The model is a simplified joint model. It has not been identified against a
  physical arm.

## Dependencies

- [`tpt-t-medical-surgical`](../tpt-t-medical-surgical) (kinematics)

Dev-dependencies: [`tpt-t-medical-ai-session`](../tpt-t-medical-ai-session),
[`tpt-t-medical-compliance`](../tpt-t-medical-compliance),
[`tpt-t-medical-safety`](../tpt-t-medical-safety).

## Related crates

- [`tpt-t-medical-ai-session`](../tpt-t-medical-ai-session) is the system under
  test in the fault scenarios.
- [`tpt-t-medical-visualizer`](../tpt-t-medical-visualizer) is the planned viewer
  for simulated runs.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
