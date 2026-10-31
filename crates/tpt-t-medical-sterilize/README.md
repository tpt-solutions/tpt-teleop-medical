# tpt-t-medical-sterilize

UV-C disinfection planning and coverage validation for tpt-teleop-medical
(spec.txt §5.4). It plans a dose from room geometry, finds which surfaces are lit
after shadowing, and checks a re-scan to confirm every surface was treated.

## Status

LiDAR room mapping, dose planning and coverage validation are done. Hardware
dose calibration is not.

## Model

- Direct irradiance falls with the square of distance and with the cosine of the
  angle of incidence (`direct_irradiance`, `distance_m`, `cos_incidence`).
- A surface in shadow receives nothing (`is_shadowed`).
- The dose delivered in a cycle is irradiance × time, in mJ/cm².
- **Reflection is not credited.** Reflectance is recorded but never used in the
  dose. Crediting an unreliable reflected estimate could hide an under-treated
  surface.

## Main types

| Item | Purpose |
|---|---|
| `UvSource` | A UV-C source, given by irradiance at 1 m in mW/cm² |
| `Surface` | A measured surface with its position, normal and reflectance |
| `plan_cycle` | One cycle time for the whole room |
| `verify_coverage` | Compares a re-scan with the requirement. Returns `Coverage::Complete` or `Coverage::Incomplete { under_dosed, first_index }`. |
| `lidar` module: `Grid`, `Return` | Bins LiDAR returns into a voxel grid (`MAX_CELLS` bounds it), finds lit faces with shadowing, and feeds them to the planner |

The planner and the validator share the same model. The validator trusts only
the dose that was measured, not the model's prediction.

## Testing

```sh
cargo test -p tpt-t-medical-sterilize
```

## Limitations

- The model is geometric and uses only the parameters it is given.
- Dose values are not calibrated against a biological standard.

## Dependencies

None.

## Related crates

- [`tpt-t-medical-logistics`](../tpt-t-medical-logistics) requires a complete UV
  cycle before a robot re-enters clean space. It records these cycles, and this
  crate is where the dose is planned and checked.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
