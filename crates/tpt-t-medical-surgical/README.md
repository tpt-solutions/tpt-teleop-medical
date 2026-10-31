# tpt-t-medical-surgical

Surgical signal and geometry processing for tpt-teleop-medical (spec.txt §4.3,
§4.5): motion scaling, tremor removal, signed-distance fixtures, arm kinematics,
a predictive display and a forbidden-zone guard.

## Status

Signal, geometry, kinematics and guard logic done. The timing targets (under
5 µs for the filters, under 20 µs for the SDF lookup) are not verified on this
target.

## Overview

Everything here is plain arithmetic on `f64`.

| Module | Main types | Purpose |
|---|---|---|
| `scaling` | `MotionScaler`, `MIN_RATIO`, `MAX_RATIO` | Divides the surgeon's hand motion down to the tool's motion, from 1:1 (microsurgery) to 10:1 (gross dissection) |
| `notch` | `NotchFilter` | Band-stop filter for the 8–12 Hz physiological tremor band. RBJ design, centred at 10 Hz, direct form II transposed. |
| `sdf` | `SdfGrid` | Signed distance to a critical structure, by trilinear interpolation. Returns "unknown" outside the grid. |
| `fixture` | `repulsive_force` | Progressive force that rises as the tool nears a forbidden zone. A NaN distance gives the maximum force. |
| `kinematics` | `Arm`, `Pose`, `Link`, `JointFrame`, `Rotation` | Seven-joint forward kinematics from a reference Denavit–Hartenberg table |
| `predict` | `Predictor`, `DEFAULT_HORIZON_US` | Predictive display: shows where the tool will be, hiding network delay. The horizon is capped at 50 ms. |
| `guard` | `Guard`, `Action` | Keeps the tool out of a forbidden structure. Returns `Follow`, `Blocked` or `Override`. |

### Guard actions

- `Follow { delta_m, force_n }`: the motion is safe. Apply it, and feed back the
  force.
- `Blocked { force_n }`: the motion would enter the structure. Do not apply it.
- `Override { … }`: the master is overridden after persistent intrusion. The
  tool retreats.

## Testing

```sh
cargo test -p tpt-t-medical-surgical
```

## Limitations

- Kinematics use a reference DH table. It has not been validated on hardware.
- Timing targets are not measured.
- The SDF grid is supplied by the caller. Segmentation is in
  [`tpt-t-medical-vision`](../tpt-t-medical-vision).

## Dependencies

None.

## Related crates

- [`tpt-t-medical-vision`](../tpt-t-medical-vision) produces the SDF grid from a
  segmented structure.
- [`tpt-t-medical-edge`](../tpt-t-medical-edge) uses the arm model for its
  torque pipeline.
- [`tpt-t-medical-teleop`](../tpt-t-medical-teleop) composes the scaler, notch
  filter and guard into the telesurgery adapter.
- [`tpt-t-medical-visualizer`](../tpt-t-medical-visualizer) shows the fixture and
  trajectory.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
