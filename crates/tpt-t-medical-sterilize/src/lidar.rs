//! `LiDAR` room mapping for dose planning (spec.txt §5.4).
//!
//! `LiDAR` returns are binned into a voxel grid. A voxel that holds a return is
//! occupied. Each face of an occupied voxel that borders free space is a surface
//! the lamp can see, if the lamp is on the free side of it. The face's normal is
//! the direction of its free neighbour, so no surface fitting is needed.
//!
//! A face is shadowed when an occupied voxel lies on the straight line from the
//! lamp to the face. The sampled line is a conservative approximation, so a
//! shadow found here is real, and a face reported as lit may in rare cases be
//! partly shadowed. Those faces are then over-dosed in the plan, not under-dosed.
//!
//! The reflectance of a face is its `LiDAR` intensity, scaled to 0–1. It is
//! recorded only, as with every other reflectance in this crate.

use crate::Surface;

/// Most voxels in one grid. About 4 million, which is 4 MB of storage.
pub const MAX_CELLS: usize = 4_000_000;

/// One `LiDAR` return, in the room frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Return {
    /// Position of the hit, in metres.
    pub position_m: [f64; 3],
    /// Intensity, from 0 (none) to 255 (strongest).
    pub intensity: u8,
}

/// A voxel grid of the room.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    origin_m: [f64; 3],
    voxel_m: f64,
    dims: [usize; 3],
    /// Zero is free. Anything else is an occupied voxel and holds the strongest
    /// intensity seen in it.
    cells: Vec<u8>,
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
impl Grid {
    /// Creates an empty grid. Returns `None` if the voxel size is not positive
    /// and finite, a dimension is zero, or the grid has more than [`MAX_CELLS`]
    /// voxels.
    #[must_use]
    pub fn new(origin_m: [f64; 3], voxel_m: f64, dims: [usize; 3]) -> Option<Self> {
        let cells = dims[0].checked_mul(dims[1])?.checked_mul(dims[2])?;
        let ok = voxel_m.is_finite()
            && voxel_m > 0.0
            && dims.iter().all(|&d| d > 0)
            && cells <= MAX_CELLS
            && origin_m.iter().all(|v| v.is_finite());
        ok.then(|| Self {
            origin_m,
            voxel_m,
            dims,
            cells: vec![0; cells],
        })
    }

    /// Records one return. Returns `false` if it falls outside the grid.
    pub fn insert(&mut self, point: Return) -> bool {
        match self.index_of(point.position_m) {
            Some(i) => {
                self.cells[i] = self.cells[i].max(point.intensity.max(1));
                true
            }
            None => false,
        }
    }

    /// Returns the number of occupied voxels.
    #[must_use]
    pub fn occupied(&self) -> usize {
        self.cells.iter().filter(|&&c| c != 0).count()
    }

    /// Builds the lit surfaces the lamp at `lamp_m` can see, with shadowing.
    #[must_use]
    pub fn surfaces(&self, lamp_m: [f64; 3]) -> Vec<Surface> {
        let mut out = Vec::new();
        for z in 0..self.dims[2] {
            for y in 0..self.dims[1] {
                for x in 0..self.dims[0] {
                    let cell = [x, y, z];
                    if self.at(cell) == 0 {
                        continue;
                    }
                    for axis in 0..3 {
                        for positive in [false, true] {
                            self.face(cell, lamp_m, axis, positive, &mut out);
                        }
                    }
                }
            }
        }
        out
    }

    fn face(
        &self,
        cell: [usize; 3],
        lamp_m: [f64; 3],
        axis: usize,
        positive: bool,
        out: &mut Vec<Surface>,
    ) {
        // Outside the grid counts as free space, so the room's edge is a face.
        let neighbour = self.step(cell, axis, positive);
        if neighbour.is_some_and(|n| self.at(n) != 0) {
            return;
        }
        let sign = if positive { 1.0 } else { -1.0 };
        let mut face_centre = self.centre(cell);
        face_centre[axis] += sign * self.voxel_m / 2.0;
        let mut normal = [0.0; 3];
        normal[axis] = sign;
        let to_lamp = [0, 1, 2].map(|i| lamp_m[i] - face_centre[i]);
        let distance = to_lamp.iter().map(|v| v * v).sum::<f64>().sqrt();
        if distance <= 0.0 {
            return;
        }
        let cos: f64 = (0..3).map(|i| to_lamp[i] * normal[i]).sum::<f64>() / distance;
        if cos <= 0.0 {
            return;
        }
        let shadowed = self.blocked(face_centre, lamp_m, cell, neighbour);
        let reflectance = f64::from(self.at(cell)) / 255.0;
        if let Some(s) = Surface::new(distance, cos.min(1.0), reflectance, shadowed) {
            out.push(s);
        }
    }

    /// The cell next to `cell` along `axis`, or `None` at the grid's edge.
    fn step(&self, cell: [usize; 3], axis: usize, positive: bool) -> Option<[usize; 3]> {
        let mut next = cell;
        next[axis] = if positive {
            cell[axis].checked_add(1).filter(|&v| v < self.dims[axis])?
        } else {
            cell[axis].checked_sub(1)?
        };
        Some(next)
    }

    /// True if an occupied voxel other than the face's own and its free
    /// neighbour lies on the line from the face to the lamp. The line is sampled
    /// at a quarter-voxel step.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn blocked(
        &self,
        from: [f64; 3],
        to: [f64; 3],
        own: [usize; 3],
        front: Option<[usize; 3]>,
    ) -> bool {
        let delta = [0, 1, 2].map(|i| to[i] - from[i]);
        let length = delta.iter().map(|v| v * v).sum::<f64>().sqrt();
        let steps = (length / (self.voxel_m / 4.0)).ceil() as usize;
        for k in 1..steps {
            let t = k as f64 / steps as f64;
            let p = [0, 1, 2].map(|i| from[i] + delta[i] * t);
            let Some(cell) = self.index_of_cell(p) else {
                continue;
            };
            if cell != own && Some(cell) != front && self.at(cell) != 0 {
                return true;
            }
        }
        false
    }

    fn centre(&self, cell: [usize; 3]) -> [f64; 3] {
        [0, 1, 2].map(|i| self.origin_m[i] + (cell[i] as f64 + 0.5) * self.voxel_m)
    }

    fn index_of(&self, p: [f64; 3]) -> Option<usize> {
        let cell = self.index_of_cell(p)?;
        Some(self.flat(cell))
    }

    fn index_of_cell(&self, p: [f64; 3]) -> Option<[usize; 3]> {
        if !p.iter().all(|v| v.is_finite()) {
            return None;
        }
        let mut cell = [0_usize; 3];
        for i in 0..3 {
            let rel = (p[i] - self.origin_m[i]) / self.voxel_m;
            if rel < 0.0 || rel >= self.dims[i] as f64 {
                return None;
            }
            cell[i] = rel as usize;
        }
        Some(cell)
    }

    fn flat(&self, cell: [usize; 3]) -> usize {
        cell[0] + self.dims[0] * (cell[1] + self.dims[1] * cell[2])
    }

    fn at(&self, cell: [usize; 3]) -> u8 {
        self.cells[self.flat(cell)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UvSource, plan_cycle};

    fn grid() -> Grid {
        // A 4 m cube at 10 cm voxels: 40 voxels a side, 64 000 cells.
        Grid::new([0.0; 3], 0.1, [40, 40, 40]).expect("grid")
    }

    fn hit(x: f64, y: f64, z: f64, intensity: u8) -> Return {
        Return {
            position_m: [x, y, z],
            intensity,
        }
    }

    #[test]
    fn a_single_voxel_shows_one_lit_face_toward_the_lamp() {
        let mut g = grid();
        assert!(g.insert(hit(2.05, 2.05, 2.05, 255)));
        let lamp = [0.05, 2.05, 2.05];
        let s = g.surfaces(lamp);
        assert_eq!(s.len(), 1, "{s:?}");
        assert!((s[0].distance_m() - 1.95).abs() < 0.06);
        assert!(s[0].cos_incidence() > 0.99);
        assert!(!s[0].is_shadowed());
    }

    #[test]
    fn a_voxel_behind_another_is_shadowed() {
        let mut g = grid();
        g.insert(hit(1.05, 2.05, 2.05, 200));
        g.insert(hit(2.05, 2.05, 2.05, 200));
        let lamp = [0.05, 2.05, 2.05];
        let s = g.surfaces(lamp);
        let shadowed = s.iter().filter(|x| x.is_shadowed()).count();
        let lit = s.iter().filter(|x| !x.is_shadowed()).count();
        assert_eq!(lit, 1, "only the near face is lit");
        assert_eq!(shadowed, 1, "the far face is in the shadow");
    }

    #[test]
    fn reflectance_follows_intensity() {
        let mut g = grid();
        g.insert(hit(2.05, 2.05, 2.05, 255));
        let s = g.surfaces([0.05, 2.05, 2.05]);
        assert!((s[0].reflectance() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn returns_outside_the_grid_are_not_recorded() {
        let mut g = grid();
        assert!(!g.insert(hit(-0.5, 1.0, 1.0, 10)));
        assert!(!g.insert(hit(f64::NAN, 1.0, 1.0, 10)));
        assert_eq!(g.occupied(), 0);
    }

    #[test]
    fn oversized_and_degenerate_grids_are_refused() {
        assert!(Grid::new([0.0; 3], 0.1, [400, 400, 400]).is_none());
        assert!(Grid::new([0.0; 3], 0.0, [4, 4, 4]).is_none());
        assert!(Grid::new([0.0; 3], 0.1, [0, 4, 4]).is_none());
        assert!(Grid::new([f64::NAN, 0.0, 0.0], 0.1, [4, 4, 4]).is_none());
    }

    #[test]
    fn a_mapped_room_feeds_the_dose_planner() {
        // A wall two metres from the lamp, at 10 cm voxels.
        let mut g = grid();
        for j in 0..40_i32 {
            for k in 0..40_i32 {
                g.insert(hit(
                    2.05,
                    0.05 + f64::from(j) * 0.1,
                    0.05 + f64::from(k) * 0.1,
                    128,
                ));
            }
        }
        let surfaces = g.surfaces([0.05, 2.05, 2.05]);
        assert_ne!(surfaces.len(), 0);
        let source = UvSource::new(1.0).expect("source");
        // The planner returns a cycle time, and a longer one for a higher dose.
        let short = plan_cycle(&source, &surfaces, 1.0).expect("plan");
        let long = plan_cycle(&source, &surfaces, 2.0).expect("plan");
        assert!(long > short);
    }
}
