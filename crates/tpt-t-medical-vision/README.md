# tpt-t-medical-vision

Endoscope vision for tpt-teleop-medical (spec.txt §4.4, Phase 3): rectified
stereo depth, a point cloud, augmented-reality overlay of pre-operative points,
and segmentation to the distance grid that the virtual fixtures use.

## Status

Algorithms done and tested on synthetic images. Real-time performance and
clinical validation are not done. DICOM pixel ingestion lives in `tpt-t-medical-bridge-dicom`; the segmentation module takes its volumes once masks are reviewed.

## Overview

| Module | Main types | Purpose |
|---|---|---|
| `stereo` | `Matcher`, `Disparity`, `Camera`, `Intrinsics`, `StereoError` | Block matching on a rectified pair with a left-right consistency check. Gives disparity, depth and a point cloud, and back-projects pixels. |
| `overlay` | `Placed`, `overlay`, `project` | Projects pre-operative scan points into the live image through the registration, for AR guidance |
| `volume` | `Volume`, `signed_distance`, `MAX_VOXELS` | Turns a segmented structure into a signed-distance grid, using an exact separable distance transform |
| `segmentation` | `Mask`, `fixture_from_mask`, `window`, `SegmentError` | Resamples a reviewed mask to isotropic spacing, then calls `volume`. Output is in metres |

Images are represented as `Gray`. `Gray::new` takes the pixel buffer, and
`Gray::from_rgb` converts an interleaved RGB buffer. Both return `None` when the
buffer does not match the stated size.

## Testing

```sh
cargo test -p tpt-t-medical-vision
```

The tests use synthetic images only.

## Limitations

- The algorithms are simple and not tuned for speed. The spec's latency targets
  need the target hardware.
- Not validated on clinical images.

## Dependencies

- [`tpt-t-medical-surgical`](../tpt-t-medical-surgical) (the SDF type used by
  fixtures)
- [`tpt-t-medical-track`](../tpt-t-medical-track) (the registration used by the
  overlay)

## Related crates

- [`tpt-t-medical-bridge-dicom`](../tpt-t-medical-bridge-dicom) reads the metadata
  that matches a scan to a patient. Its pixel reader (`pixels`) feeds CT
  slices and volumes into this crate's segmentation path, and
  `tpt-t-medical-conformance/tests/vision_fixture.rs` runs that path end to end.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
