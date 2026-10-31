# tpt-t-medical-edge

The 1 kHz control-law pipeline for tpt-teleop-medical (spec.txt §4.3, Phase 3).
It turns a tool force into joint torques, adds gravity and friction compensation,
and clamps the result to the torque limits.

## Status

Torque pipeline done. The virtual-work identities are checked by tests. The
real-time core (pinned thread, scheduler policy) is not implemented.

## Overview

Each cycle, `control::Loop::step` runs four stages:

1. The tool force becomes joint torques through the Jacobian
   (`joint_torque_from_force`, `position_jacobian`).
2. Gravity compensation is added (`gravity_torque`, `GRAVITY_M_S2`).
3. Friction is added (`friction_torque`, `Friction`).
4. Each joint is clamped to its limit (`saturate`).

If a cycle arrives after its deadline, the loop holds the last command rather
than computing one from a stale state. The result carries flags for this:
`Command::saturated` and `Command::held`.

### Main types

| Item | Purpose |
|---|---|
| `Model` | Arm geometry, masses, friction and torque limits |
| `State` | Joint angles and rates, tool force, and the current time in µs |
| `Command` | `torque_nm: [f64; 7]`, plus `saturated` and `held` flags |
| `Loop` | Holds the last command and runs `step` each cycle |
| `Friction` | Viscous and Coulomb friction parameters |
| `PERIOD_US`, `DEADLINE_US` | 1 000 µs each |

## Testing

```sh
cargo test -p tpt-t-medical-edge
```

## Limitations

- Timing is checked against the clock the caller supplies. Nothing measures
  real-time behaviour, pins a thread or sets a scheduler policy.
- The model is an approximation. It has not been identified on the hardware.

## Dependencies

- [`tpt-t-medical-surgical`](../tpt-t-medical-surgical) for the arm kinematics.

## Related crates

- [`tpt-t-medical-teleop`](../tpt-t-medical-teleop) calls this loop from its tick.
- [`tpt-t-medical-safety`](../tpt-t-medical-safety) decides when a stop must
  override the torque output.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
