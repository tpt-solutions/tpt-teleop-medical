// Test-only builder: the coordinates are small, so the lossy casts and by-ref
// byte pairs are intended.
#![allow(
    clippy::cast_precision_loss,
    clippy::trivially_copy_pass_by_ref,
    clippy::cast_possible_truncation
)]

//! Cross-crate wiring for critical structures (spec.txt §4.3). A DICOM series of
//! a structure is read as a volume, windowed to its tissue, converted to a
//! fixture by the vision crate, and then enforced by the surgical guard. The
//! guard must block a motion that would enter the structure.

use tpt_t_medical_bridge_dicom::pixels::read_volume;
use tpt_t_medical_surgical::guard::{Action, Guard};
use tpt_t_medical_vision::segmentation::{Mask, fixture_from_mask, window};

const SIDE: usize = 16;
/// The structure's centre, in voxels, and its radius, in millimetres.
const CENTRE: f64 = 8.0;
const RADIUS_MM: f64 = 4.0;
/// Raw stored values carry this intercept, so air is raw 24 and tissue raw 1324.
const INTERCEPT: i32 = -1024;
/// One millimetre, in metres. The fixture is in metres.
const MM: f64 = 1.0e-3;

fn text(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    if v.len() % 2 == 1 {
        v.push(b' ');
    }
    v
}

fn element(group: u16, tag: u16, vr: &[u8; 2], value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&group.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(vr);
    if vr == b"OW" {
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&u32::try_from(value.len()).expect("small").to_le_bytes());
    } else {
        out.extend_from_slice(&u16::try_from(value.len()).expect("small").to_le_bytes());
    }
    out.extend_from_slice(value);
    out
}

/// One slice of a sphere at height `z_mm`. Tissue inside the sphere is 300 HU,
/// and everything else is air at -1000 HU.
fn slice(z_mm: usize) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(SIDE * SIDE * 2);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let d2 = (x as f64 - CENTRE).powi(2)
                + (y as f64 - CENTRE).powi(2)
                + (z_mm as f64 - CENTRE).powi(2);
            let hu: i32 = if d2.sqrt() <= RADIUS_MM { 300 } else { -1000 };
            let raw = i16::try_from(hu - INTERCEPT).expect("in range");
            pixels.extend_from_slice(&raw.to_le_bytes());
        }
    }
    let mut file = vec![0_u8; 128];
    file.extend_from_slice(b"DICM");
    file.extend(element(0x0002, 0x0010, b"UI", b"1.2.840.10008.1.2.1\0"));
    file.extend(element(0x0028, 0x0002, b"US", &1_u16.to_le_bytes()));
    file.extend(element(0x0028, 0x0004, b"CS", &text("MONOCHROME2")));
    file.extend(element(0x0028, 0x0010, b"US", &(SIDE as u16).to_le_bytes()));
    file.extend(element(0x0028, 0x0011, b"US", &(SIDE as u16).to_le_bytes()));
    file.extend(element(0x0028, 0x0030, b"DS", &text("1\\1")));
    file.extend(element(0x0028, 0x0100, b"US", &16_u16.to_le_bytes()));
    file.extend(element(0x0028, 0x0103, b"US", &1_u16.to_le_bytes()));
    file.extend(element(
        0x0028,
        0x1052,
        b"DS",
        &text(&INTERCEPT.to_string()),
    ));
    file.extend(element(0x0028, 0x1053, b"DS", &text("1")));
    file.extend(element(
        0x0020,
        0x0032,
        b"DS",
        &text(&format!("0\\0\\{z_mm}")),
    ));
    file.extend(element(0x7FE0, 0x0010, b"OW", &pixels));
    file
}

#[test]
fn a_dicom_structure_becomes_a_fixture_the_guard_enforces() {
    // Presented out of order, to show the volume is put into position order.
    let files: Vec<Vec<u8>> = [7, 0, 15, 3, 11, 1, 14, 8, 2, 12, 5, 9, 4, 13, 6, 10]
        .into_iter()
        .map(slice)
        .collect();
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let volume = read_volume(&refs).expect("series reads");
    assert_eq!(volume.dims, [SIDE, SIDE, SIDE]);

    let inside = window(&volume.hounsfield, 200.0, 1000.0);
    let mask = Mask {
        dims: volume.dims,
        spacing_mm: volume.spacing_mm,
        origin_mm: volume.origin_mm,
        inside: &inside,
    };
    let fixture = fixture_from_mask(&mask).expect("fixture");

    // A tool 2 mm outside the sphere. A 4 mm step toward its centre would end
    // 2 mm inside, so the guard must block it. The fixture is in metres.
    let tool = [CENTRE + RADIUS_MM + 2.0, CENTRE, CENTRE].map(|v| v * MM);
    let mut guard = Guard::new(0.5 * MM, 3.0, 200_000, MM).expect("guard");
    let toward_centre = [-4.0 * MM, 0.0, 0.0];
    let action = guard.evaluate(0, &fixture, tool, toward_centre);
    assert!(
        matches!(action, Action::Blocked { .. }),
        "the guard must block motion into the structure: {action:?}"
    );

    // A 1 mm step away from it is followed.
    let away = guard.evaluate(1, &fixture, tool, [MM, 0.0, 0.0]);
    assert!(matches!(away, Action::Follow { .. }), "{away:?}");
}
