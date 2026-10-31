# tpt-t-medical-logistics

Hospital delivery-robot logic for tpt-teleop-medical (spec.txt §5.2): infection
control zones, single-occupancy elevator arbitration, UV-aware route planning,
transport profiles and an MQTT elevator service.

## Status

Logic done. There is no door model, no traffic model between robots and no
broker connection. Those are left to the deployment.

## Overview

| Module | Main types | Purpose |
|---|---|---|
| `zones` | `Zone`, `Robot`, `ZoneError` | Clean, dirty and isolation zones. A robot that has left a non-clean zone must complete a UV-C cycle before it may re-enter clean space (`NeedsSterilization`). |
| `elevator` | `Elevator`, `Grant`, `NotOccupant` | One occupant per cab. Emergency transports are served first. |
| `route` | `Map`, `Node`, `NodeId`, `Route`, `RouteError`, `MAX_NODES` | Corridor route planning. The infection-control rule is part of the search, so a forbidden route is never returned. |
| `profile` | `Profile`, `ProfileError` | What each load may enter and where it may end |
| `mqtt` | `Message`, `ElevatorService`, `FrameError`, `ElevatorMessageError` | Elevator requests, grants and releases as MQTT 3.1.1 QoS 0 packets. The framing is written in-house, so no client dependency is added. |

### Transport profiles

| `Profile` | Meaning |
|---|---|
| `Linens` | Clean and soiled linen |
| `Meals` | Patient meals. Stays in clean space. |
| `Medication` | Stays in clean space. |
| `LabSample` | Stays in clean space. |
| `Biohazard` | Must end outside clean space. Never left in clean space. |

### Limits

Messages are bounded: `MAX_TOPIC_BYTES`, `MAX_PAYLOAD_BYTES` and `MAX_NODES`.

## Testing

```sh
cargo test -p tpt-t-medical-logistics
```

## Limitations

- No door timing model.
- No traffic model between robots.
- No MQTT broker connection. The crate produces and parses frames only.

## Dependencies

- [`tpt-t-medical-sterilize`](../tpt-t-medical-sterilize)

## Related crates

- [`tpt-t-medical-bridge-hl7fhir`](../tpt-t-medical-bridge-hl7fhir) maps an
  admit or transfer to a transport destination, and a discharge to a clear.
- [`tpt-t-medical-pharmacy`](../tpt-t-medical-pharmacy) supplies medication
  loads.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
