# tpt-t-medical-track

Tool and anatomy tracking for tpt-teleop-medical (spec.txt §4.4, Phase 3). It
fuses position reports from several sensors and registers the tracker's frame to
the pre-operative scan.

## Status

Logic done. The filter uses a linear model and is checked on simulated data only.
Nonlinear sensor models and the portable SIMD the spec asks for are not
implemented.

## Overview

- **Filter** ([`filter`](src/filter.rs)): a constant-velocity Kalman filter that
  fuses position reports, such as optical and electromagnetic. Reports far from
  the prediction are rejected by an outlier gate (`DEFAULT_GATE`).
- **Registration** ([`register`](src/register.rs)): finds the rigid transform
  (`Rigid`) from the tracker's frame to the pre-op scan's frame, from paired
  fiducials (Horn's method).

### Main types

| Item | Purpose |
|---|---|
| `Filter` | `new`, `start`, `predict`, `update`, `apply`. Exposes `position`, `position_sigma`, `velocity`, `is_started`. |
| `Outcome` | The result of applying one position report, including whether it was gated out |
| `register` | Builds a `Rigid` transform from paired points |
| `Rigid` | The rigid transform. `rms_error` measures the fit. |
| `DEFAULT_GATE` | Default outlier threshold |

The filter is the extended form with an identity Jacobian, because the
measurement models are linear.

## Testing

```sh
cargo test -p tpt-t-medical-track
```

## Limitations

- Linear measurement models only.
- The sub-millimetre accuracy and latency targets are checked on simulated data.
  They need the target sensors.

## Dependencies

None.

## Related crates

- [`tpt-t-medical-vision`](../tpt-t-medical-vision) depends on this crate to
  place pre-operative points into the live image.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
