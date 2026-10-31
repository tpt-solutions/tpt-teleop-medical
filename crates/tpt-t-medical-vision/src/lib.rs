//! Vision for the endoscope (spec.txt §4.4, Phase 3).
//!
//! * [`stereo`] matches a rectified stereo pair, gives depth and a point cloud.
//! * [`overlay`] places pre-operative points into the live image, for AR guidance.
//! * [`volume`] turns a segmented critical structure into the signed-distance
//!   grid that the surgical virtual fixtures use.
//!
//! The algorithms are exact and simple. They are not tuned for speed, so the
//! spec's latency targets need the target hardware. The tests use synthetic
//! images, not clinical data.

#![forbid(unsafe_code)]

pub mod overlay;
pub mod segmentation;
pub mod stereo;
pub mod volume;

/// A grayscale image, row-major, 8 bits per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gray {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Pixel values, row-major.
    pub data: Vec<u8>,
}

impl Gray {
    /// Creates an image. Returns `None` if the data length is not `width · height`
    /// or a dimension is zero.
    #[must_use]
    pub fn new(width: usize, height: usize, data: Vec<u8>) -> Option<Self> {
        let ok = width > 0 && height > 0 && width.checked_mul(height) == Some(data.len());
        ok.then_some(Self {
            width,
            height,
            data,
        })
    }

    /// Converts packed RGB to luminance, using the Rec. 601 weights. Returns
    /// `None` if the data length is not `3 · width · height`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn from_rgb(width: usize, height: usize, rgb: &[u8]) -> Option<Self> {
        let pixels = width.checked_mul(height)?;
        if pixels == 0 || rgb.len() != pixels.checked_mul(3)? {
            return None;
        }
        let data = rgb
            .chunks_exact(3)
            .map(|p| {
                let y = 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]);
                // The weights sum to one, so the result is within 0 to 255.
                // Clamped to the byte range above, so the cast cannot wrap.
                y.round().clamp(0.0, 255.0) as u8
            })
            .collect();
        Self::new(width, height, data)
    }
}
