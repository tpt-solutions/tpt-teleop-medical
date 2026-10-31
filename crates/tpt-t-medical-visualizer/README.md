# tpt-t-medical-visualizer

The operator visualizer model for tpt-teleop-medical (spec.txt §6.5, Phase 7). It
holds the data an operator would see, and it can write that data as a
self-contained HTML page.

## Status

Scene model and view projection done. HTML export done. The interactive window is
not part of this crate.

## Overview

The crate is pure data and can be tested without a display.

| Module | Main items | Purpose |
|---|---|---|
| `scene` | `Scene`, `Event`, `Force`, `View`, `Copy3`, `DEMO_SAMPLES`, `demo`, `fit`, `project`, `bounds`, `box_edges` | The scene: the tool's trajectory against its workspace box, the fixture boundary, force arrows, the audit timeline, and the state of the three TMR copies. Also a view projection and fitting. |
| `html` | `render` | Writes the scene as one HTML page |

The HTML page has inline SVG and a small script that rotates the view when the
user drags it. It has no external resources, and its projection matches the Rust
model.

### Export example

```sh
cargo run -p tpt-t-medical-visualizer --example export
```

The example writes the demonstration scene (a reference arm swept through a slow
motion) to an HTML file.

## Testing

```sh
cargo test -p tpt-t-medical-visualizer
```

## Limitations

- The live operator window is not in this crate. A live view would take the scene
  from the running system instead of the demonstration scene.
- The HTML export is for review. It is not a clinical display.

## Dependencies

- [`tpt-t-medical-surgical`](../tpt-t-medical-surgical) (fixture and kinematics
  types)

## Related crates

- [`tpt-t-medical-sim`](../tpt-t-medical-sim) produces simulated runs. A live
  scene would be fed from the running system instead of the demonstration scene.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
