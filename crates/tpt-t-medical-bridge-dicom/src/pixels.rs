//! Pixel ingestion for CT slices and volumes (spec.txt §4.3, Phase 6).
//!
//! [`read_slice`] reads one uncompressed CT slice and converts it to Hounsfield
//! units. [`read_volume`] orders a series by position and stacks it into a
//! regular grid, which is the input the segmentation pipeline needs.
//!
//! Scope, stated plainly:
//!
//! * Only 16-bit `MONOCHROME2` data is read, as CT produces. Other bit depths
//!   and photometric interpretations are refused, not guessed at.
//! * `RescaleSlope` and `RescaleIntercept` are applied. Their defaults are 1 and
//!   0, as the standard says.
//! * A volume needs `ImagePositionPatient` on every slice, and slice spacing must
//!   be uniform. A non-uniform stack is refused, because the signed-distance grid
//!   assumes a regular lattice.
//! * The voxel count is bounded by [`MAX_VOXELS`], so a hostile file cannot
//!   force a large allocation.

use core::fmt;

use crate::{Element, MAX_FILE_BYTES, ParseError, read_text, read_u16, scan};

/// Largest volume accepted, in voxels. At 4 bytes per voxel this is 256 MiB.
pub const MAX_VOXELS: usize = 64 * 1024 * 1024;

/// Tolerance for comparing spacings, in millimetres.
const SPACING_TOLERANCE_MM: f64 = 1e-3;

const TAG_SAMPLES_PER_PIXEL: (u16, u16) = (0x0028, 0x0002);
const TAG_PHOTOMETRIC: (u16, u16) = (0x0028, 0x0004);
const TAG_ROWS: (u16, u16) = (0x0028, 0x0010);
const TAG_COLUMNS: (u16, u16) = (0x0028, 0x0011);
const TAG_PIXEL_SPACING: (u16, u16) = (0x0028, 0x0030);
const TAG_BITS_ALLOCATED: (u16, u16) = (0x0028, 0x0100);
const TAG_PIXEL_REPRESENTATION: (u16, u16) = (0x0028, 0x0103);
const TAG_RESCALE_INTERCEPT: (u16, u16) = (0x0028, 0x1052);
const TAG_RESCALE_SLOPE: (u16, u16) = (0x0028, 0x1053);
const TAG_IMAGE_POSITION: (u16, u16) = (0x0020, 0x0032);
const TAG_PIXEL_DATA: (u16, u16) = (0x7FE0, 0x0010);

/// One CT slice, converted to Hounsfield units.
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    /// Image rows.
    pub rows: usize,
    /// Image columns.
    pub columns: usize,
    /// Column spacing (x) and row spacing (y), in millimetres.
    pub spacing_mm: [f64; 2],
    /// Position of the first voxel, from `ImagePositionPatient`, in millimetres.
    pub position_mm: [f64; 3],
    /// Row-major values in Hounsfield units.
    pub hounsfield: Vec<f32>,
}

/// A regular stack of slices. Voxels are stored x-fastest: the index of
/// `(x, y, z)` is `x + nx * (y + ny * z)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    /// Voxels along x, y and z.
    pub dims: [usize; 3],
    /// Voxel size along x, y and z, in millimetres.
    pub spacing_mm: [f64; 3],
    /// Position of voxel `(0, 0, 0)`, in millimetres.
    pub origin_mm: [f64; 3],
    /// Values in Hounsfield units.
    pub hounsfield: Vec<f32>,
}

/// Why pixel data could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PixelError {
    /// The file or its metadata is malformed. See [`ParseError`].
    Parse(ParseError),
    /// The data uses a format this crate does not read.
    Unsupported(&'static str),
    /// The pixel data is not the size the image dimensions require.
    PixelDataSize {
        /// Bytes the dimensions require.
        expected: usize,
        /// Bytes present.
        found: usize,
    },
    /// The image or volume is above [`MAX_VOXELS`].
    TooLarge {
        /// The voxel count requested.
        voxels: usize,
    },
    /// No slices were given.
    NoSlices,
    /// The slices do not form one regular volume.
    Inconsistent(&'static str),
}

impl fmt::Display for PixelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::Unsupported(what) => write!(f, "unsupported: {what}"),
            Self::PixelDataSize { expected, found } => {
                write!(f, "pixel data is {found} bytes, expected {expected}")
            }
            Self::TooLarge { voxels } => write!(f, "{voxels} voxels is above the limit"),
            Self::NoSlices => f.write_str("no slices given"),
            Self::Inconsistent(what) => write!(f, "slices do not form a volume: {what}"),
        }
    }
}

impl std::error::Error for PixelError {}

impl From<ParseError> for PixelError {
    fn from(e: ParseError) -> Self {
        Self::Parse(e)
    }
}

/// Reads one CT slice from a DICOM Part 10 file.
///
/// # Errors
///
/// Returns an error if the file is malformed, uses an unsupported format, or
/// its pixel data does not match its dimensions.
#[allow(clippy::cast_possible_truncation)]
pub fn read_slice(file: &[u8]) -> Result<Slice, PixelError> {
    if file.len() > MAX_FILE_BYTES {
        return Err(ParseError::TooLarge { len: file.len() }.into());
    }
    let (_, elements) = scan(file)?;

    let mut rows = None;
    let mut columns = None;
    let mut samples = None;
    let mut bits = None;
    let mut signed = None;
    let mut photometric = None;
    let mut slope = None;
    let mut intercept = None;
    let mut spacing = None;
    let mut position = None;
    let mut pixels: Option<Element> = None;
    for el in &elements {
        match el.tag {
            TAG_ROWS => rows = Some(read_u16(file, el, "Rows")?),
            TAG_COLUMNS => columns = Some(read_u16(file, el, "Columns")?),
            TAG_SAMPLES_PER_PIXEL => samples = Some(read_u16(file, el, "SamplesPerPixel")?),
            TAG_BITS_ALLOCATED => bits = Some(read_u16(file, el, "BitsAllocated")?),
            TAG_PIXEL_REPRESENTATION => {
                signed = Some(read_u16(file, el, "PixelRepresentation")? == 1);
            }
            TAG_PHOTOMETRIC => {
                photometric = Some(read_text(file, el, "PhotometricInterpretation")?);
            }
            TAG_RESCALE_SLOPE => slope = Some(read_decimals(file, el, "RescaleSlope")?),
            TAG_RESCALE_INTERCEPT => {
                intercept = Some(read_decimals(file, el, "RescaleIntercept")?);
            }
            TAG_PIXEL_SPACING => spacing = Some(read_decimals(file, el, "PixelSpacing")?),
            TAG_IMAGE_POSITION => position = Some(read_decimals(file, el, "ImagePositionPatient")?),
            TAG_PIXEL_DATA => pixels = Some(*el),
            _ => {}
        }
    }

    let rows = required(rows, "Rows")?;
    let columns = required(columns, "Columns")?;
    let samples = required(samples, "SamplesPerPixel")?;
    let bits = required(bits, "BitsAllocated")?;
    let signed = required(signed, "PixelRepresentation")?;
    let photometric = required(photometric, "PhotometricInterpretation")?;
    let pixels = required(pixels, "PixelData")?;
    let spacing = required(spacing, "PixelSpacing")?;
    let position = required(position, "ImagePositionPatient")?;

    if samples != 1 {
        return Err(PixelError::Unsupported("SamplesPerPixel other than 1"));
    }
    if photometric != "MONOCHROME2" {
        return Err(PixelError::Unsupported(
            "PhotometricInterpretation other than MONOCHROME2",
        ));
    }
    if bits != 16 {
        return Err(PixelError::Unsupported("BitsAllocated other than 16"));
    }
    if rows == 0 || columns == 0 {
        return Err(ParseError::BadValue("Rows").into());
    }
    let voxels = usize::from(rows)
        .checked_mul(usize::from(columns))
        .ok_or(PixelError::TooLarge { voxels: usize::MAX })?;
    if voxels > MAX_VOXELS {
        return Err(PixelError::TooLarge { voxels });
    }
    let expected = voxels * 2;
    if pixels.value_len != expected {
        return Err(PixelError::PixelDataSize {
            expected,
            found: pixels.value_len,
        });
    }

    let slope = slope.and_then(|v| v.first().copied()).unwrap_or(1.0);
    let intercept = intercept.and_then(|v| v.first().copied()).unwrap_or(0.0);
    let bytes = &file[pixels.value_start..pixels.value_start + pixels.value_len];
    let hounsfield = bytes
        .chunks_exact(2)
        .map(|b| {
            let raw = if signed {
                f64::from(i16::from_le_bytes([b[0], b[1]]))
            } else {
                f64::from(u16::from_le_bytes([b[0], b[1]]))
            };
            (raw * slope + intercept) as f32
        })
        .collect();

    let [row_mm, column_mm] = pair(&spacing, "PixelSpacing")?;
    let position_mm = triple(&position, "ImagePositionPatient")?;
    Ok(Slice {
        rows: usize::from(rows),
        columns: usize::from(columns),
        spacing_mm: [column_mm, row_mm],
        position_mm,
        hounsfield,
    })
}

/// Reads a series and stacks it into a regular volume.
///
/// The slices may be given in any order. They are sorted by their position
/// along z.
///
/// # Errors
///
/// Returns an error if any slice fails [`read_slice`], if the slices differ in
/// size or in-plane spacing, if there are fewer than two, if their z spacing is
/// not uniform, or if the volume is above [`MAX_VOXELS`].
pub fn read_volume(files: &[&[u8]]) -> Result<Volume, PixelError> {
    if files.is_empty() {
        return Err(PixelError::NoSlices);
    }
    let mut slices = files
        .iter()
        .map(|f| read_slice(f))
        .collect::<Result<Vec<_>, _>>()?;
    if slices.len() < 2 {
        return Err(PixelError::Inconsistent(
            "a volume needs at least two slices",
        ));
    }

    let first = &slices[0];
    let (rows, columns, spacing) = (first.rows, first.columns, first.spacing_mm);
    for s in &slices {
        if s.rows != rows || s.columns != columns {
            return Err(PixelError::Inconsistent("slice sizes differ"));
        }
        if (s.spacing_mm[0] - spacing[0]).abs() > SPACING_TOLERANCE_MM
            || (s.spacing_mm[1] - spacing[1]).abs() > SPACING_TOLERANCE_MM
        {
            return Err(PixelError::Inconsistent("in-plane spacing differs"));
        }
    }

    slices.sort_by(|a, b| a.position_mm[2].total_cmp(&b.position_mm[2]));
    let step = slices[1].position_mm[2] - slices[0].position_mm[2];
    if step <= 0.0 {
        return Err(PixelError::Inconsistent("two slices share a position"));
    }
    for pair in slices.windows(2) {
        let gap = pair[1].position_mm[2] - pair[0].position_mm[2];
        if (gap - step).abs() > SPACING_TOLERANCE_MM {
            return Err(PixelError::Inconsistent("non-uniform slice spacing"));
        }
    }

    let voxels = rows
        .checked_mul(columns)
        .and_then(|p| p.checked_mul(slices.len()))
        .ok_or(PixelError::TooLarge { voxels: usize::MAX })?;
    if voxels > MAX_VOXELS {
        return Err(PixelError::TooLarge { voxels });
    }

    let origin = slices[0].position_mm;
    let mut hounsfield = Vec::with_capacity(voxels);
    for s in &slices {
        hounsfield.extend_from_slice(&s.hounsfield);
    }
    Ok(Volume {
        dims: [columns, rows, slices.len()],
        spacing_mm: [spacing[0], spacing[1], step],
        origin_mm: origin,
        hounsfield,
    })
}

fn required<T>(value: Option<T>, name: &'static str) -> Result<T, PixelError> {
    value.ok_or_else(|| ParseError::MissingElement(name).into())
}

/// Reads a decimal-string (DS) value: one or more numbers separated by `\`.
fn read_decimals(file: &[u8], el: &Element, name: &'static str) -> Result<Vec<f64>, PixelError> {
    let text = read_text(file, el, name)?;
    text.split('\\')
        .map(|part| {
            part.trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| ParseError::BadValue(name).into())
        })
        .collect()
}

fn pair(values: &[f64], name: &'static str) -> Result<[f64; 2], PixelError> {
    match values {
        [a, b] => Ok([*a, *b]),
        _ => Err(ParseError::BadValue(name).into()),
    }
}

fn triple(values: &[f64], name: &'static str) -> Result<[f64; 3], PixelError> {
    match values {
        [a, b, c] => Ok([*a, *b, *c]),
        _ => Err(ParseError::BadValue(name).into()),
    }
}

#[cfg(test)]
// Expected values are exact by construction, so exact comparison is intended.
#[allow(
    clippy::float_cmp,
    clippy::cast_possible_truncation,
    clippy::trivially_copy_pass_by_ref
)]
mod tests {
    use super::*;

    const EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";

    fn text(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        if v.len() % 2 == 1 {
            v.push(b' ');
        }
        v
    }

    /// Writes one Explicit VR element. OW and UN take the long header.
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

    fn us(v: u16) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    /// Builds a 16-bit CT slice. `pixels` must hold rows * columns values.
    struct SliceSpec<'a> {
        rows: u16,
        columns: u16,
        pixels: &'a [i16],
        signed: bool,
        photometric: &'a str,
        bits: u16,
        slope: &'a str,
        intercept: &'a str,
        position: &'a str,
    }

    impl<'a> SliceSpec<'a> {
        fn new(rows: u16, columns: u16, pixels: &'a [i16]) -> Self {
            Self {
                rows,
                columns,
                pixels,
                signed: true,
                photometric: "MONOCHROME2",
                bits: 16,
                slope: "1",
                intercept: "-1024",
                position: "0\\0\\0",
            }
        }

        fn build(&self) -> Vec<u8> {
            let mut file = vec![0u8; 128];
            file.extend_from_slice(b"DICM");
            file.extend(element(0x0002, 0x0010, b"UI", &text(EXPLICIT_LE)));
            file.extend(element(0x0028, 0x0002, b"US", &us(1)));
            file.extend(element(0x0028, 0x0004, b"CS", &text(self.photometric)));
            file.extend(element(0x0028, 0x0010, b"US", &us(self.rows)));
            file.extend(element(0x0028, 0x0011, b"US", &us(self.columns)));
            file.extend(element(0x0028, 0x0030, b"DS", &text("0.5\\0.25")));
            file.extend(element(0x0028, 0x0100, b"US", &us(self.bits)));
            file.extend(element(0x0028, 0x0103, b"US", &us(u16::from(self.signed))));
            file.extend(element(0x0028, 0x1052, b"DS", &text(self.intercept)));
            file.extend(element(0x0028, 0x1053, b"DS", &text(self.slope)));
            file.extend(element(0x0020, 0x0032, b"DS", &text(self.position)));
            let data: Vec<u8> = self.pixels.iter().flat_map(|v| v.to_le_bytes()).collect();
            file.extend(element(0x7FE0, 0x0010, b"OW", &data));
            file
        }
    }

    #[test]
    fn pixels_become_hounsfield_units() {
        // Stored values are raw; the intercept of -1024 moves air to -1024 HU.
        let raw = [0_i16, 1024, 2048, 3071];
        let file = SliceSpec::new(2, 2, &raw).build();
        let slice = read_slice(&file).expect("reads");
        assert_eq!(slice.hounsfield, vec![-1024.0, 0.0, 1024.0, 2047.0]);
        assert_eq!((slice.rows, slice.columns), (2, 2));
    }

    #[test]
    fn spacing_is_column_then_row() {
        // PixelSpacing is "row\column", but the slice stores x then y.
        let raw = [0_i16; 4];
        let slice = read_slice(&SliceSpec::new(2, 2, &raw).build()).expect("reads");
        assert_eq!(slice.spacing_mm, [0.25, 0.5]);
    }

    #[test]
    fn unsigned_representation_reads_large_values_as_positive() {
        let raw = [-5_i16, 0, 0, 0];
        let mut spec = SliceSpec::new(2, 2, &raw);
        spec.signed = false;
        spec.intercept = "0";
        let slice = read_slice(&spec.build()).expect("reads");
        assert_eq!(slice.hounsfield[0], 65531.0);
    }

    #[test]
    fn zero_intercept_leaves_values_unchanged() {
        let raw = [7_i16, 8, 9, 10];
        let mut spec = SliceSpec::new(2, 2, &raw);
        spec.slope = "1";
        spec.intercept = "0";
        let slice = read_slice(&spec.build()).expect("reads");
        assert_eq!(slice.hounsfield, vec![7.0, 8.0, 9.0, 10.0]);
    }

    #[test]
    fn non_ct_photometric_interpretation_is_refused() {
        let raw = [0_i16; 4];
        let mut spec = SliceSpec::new(2, 2, &raw);
        spec.photometric = "MONOCHROME1";
        assert_eq!(
            read_slice(&spec.build()),
            Err(PixelError::Unsupported(
                "PhotometricInterpretation other than MONOCHROME2"
            ))
        );
    }

    #[test]
    fn eight_bit_data_is_refused() {
        let raw = [0_i16; 4];
        let mut spec = SliceSpec::new(2, 2, &raw);
        spec.bits = 8;
        assert_eq!(
            read_slice(&spec.build()),
            Err(PixelError::Unsupported("BitsAllocated other than 16"))
        );
    }

    #[test]
    fn pixel_data_of_the_wrong_size_is_refused() {
        let raw = [0_i16; 3];
        let file = SliceSpec::new(2, 2, &raw).build();
        assert_eq!(
            read_slice(&file),
            Err(PixelError::PixelDataSize {
                expected: 8,
                found: 6
            })
        );
    }

    #[test]
    fn a_slice_without_position_is_refused() {
        let raw = [0_i16; 4];
        let file = SliceSpec::new(2, 2, &raw).build();
        // Remove the ImagePositionPatient element by rebuilding without it.
        let needle = element(0x0020, 0x0032, b"DS", &text("0\\0\\0"));
        let at = file
            .windows(needle.len())
            .position(|w| w == needle.as_slice())
            .expect("present");
        let mut stripped = file.clone();
        stripped.drain(at..at + needle.len());
        assert_eq!(
            read_slice(&stripped),
            Err(PixelError::Parse(ParseError::MissingElement(
                "ImagePositionPatient"
            )))
        );
    }

    fn stack(z_values: &[&str]) -> Vec<Vec<u8>> {
        z_values
            .iter()
            .enumerate()
            .map(|(k, z)| {
                let raw: Vec<i16> = (0..4)
                    .map(|v| i16::try_from(k * 10 + v).unwrap_or(0))
                    .collect();
                let mut spec = SliceSpec::new(2, 2, &raw);
                spec.intercept = "0";
                spec.position = *z;
                spec.build()
            })
            .collect()
    }

    #[test]
    fn volume_is_ordered_by_position_whatever_the_file_order() {
        // `stack` gives file k the raw values 10k..10k+3. The files are listed
        // as z = 20, 0, 10, so file 1 (raw 10..13) is the lowest slice.
        let files = stack(&["0\\0\\20", "0\\0\\0", "0\\0\\10"]);
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        let volume = read_volume(&refs).expect("reads");
        assert_eq!(volume.dims, [2, 2, 3]);
        assert_eq!(volume.origin_mm, [0.0, 0.0, 0.0]);
        assert!((volume.spacing_mm[2] - 10.0).abs() < 1e-9);
        assert_eq!(&volume.hounsfield[0..4], &[10.0, 11.0, 12.0, 13.0]);
        // Voxel (1, 1, 2) is the last value of the top slice, file 0 (raw 0..3).
        let top_right = 1 + 2 * (1 + 2 * 2);
        assert_eq!(volume.hounsfield[top_right], 3.0);
    }

    #[test]
    fn non_uniform_slice_spacing_is_refused() {
        let files = stack(&["0\\0\\0", "0\\0\\10", "0\\0\\25"]);
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        assert_eq!(
            read_volume(&refs),
            Err(PixelError::Inconsistent("non-uniform slice spacing"))
        );
    }

    #[test]
    fn duplicate_positions_are_refused() {
        let files = stack(&["0\\0\\0", "0\\0\\0"]);
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        assert_eq!(
            read_volume(&refs),
            Err(PixelError::Inconsistent("two slices share a position"))
        );
    }

    #[test]
    fn a_single_slice_is_not_a_volume() {
        let files = stack(&["0\\0\\0"]);
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        assert!(matches!(
            read_volume(&refs),
            Err(PixelError::Inconsistent(_))
        ));
    }

    #[test]
    fn no_slices_is_refused() {
        assert_eq!(read_volume(&[]), Err(PixelError::NoSlices));
    }

    #[test]
    fn mismatched_slice_sizes_are_refused() {
        let a = SliceSpec::new(2, 2, &[0_i16; 4]).build();
        let b = SliceSpec::new(2, 4, &[0_i16; 8]).build();
        let files = [a.as_slice(), b.as_slice()];
        assert_eq!(
            read_volume(&files),
            Err(PixelError::Inconsistent("slice sizes differ"))
        );
    }

    #[test]
    fn mutated_slices_never_panic() {
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        let base = SliceSpec::new(2, 2, &[1_i16, 2, 3, 4]).build();
        for _ in 0..256 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let mut file = base.clone();
            let at = usize::try_from(seed % u64::try_from(file.len()).unwrap_or(1)).unwrap_or(0);
            file[at] = u8::try_from((seed >> 32) & 0xFF).unwrap_or(0);
            let _ = read_slice(&file);
        }
    }
}
