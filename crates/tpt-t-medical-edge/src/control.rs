//! Torque terms for the 7-DOF arm: force to joints, gravity, friction, and
//! limits (spec.txt §4.3).
//!
//! Each term is a plain function, so each can be checked on its own.
//!
//! * [`joint_torque_from_force`] maps a Cartesian force on the tool to joint
//!   torques, through the transpose of the position Jacobian. The Jacobian is
//!   taken by central differences of the forward kinematics.
//! * [`gravity_torque`] holds the arm up against gravity. It uses one point mass
//!   per link, placed at the link's joint frame. That is a model, not a measured
//!   inertia.
//! * [`friction_torque`] models Coulomb and viscous friction in each joint.
//! * [`saturate`] clamps each joint to its torque limit, and reports whether it
//!   did.

use tpt_t_medical_surgical::kinematics::Arm;

/// Standard gravity, in metres per second squared.
pub const GRAVITY_M_S2: f64 = 9.81;

/// Step used for the central difference, in radians.
const JACOBIAN_STEP_RAD: f64 = 1e-6;

/// Friction parameters, applied to every joint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Friction {
    /// Coulomb friction magnitude, in newton-metres.
    pub coulomb_nm: f64,
    /// Viscous friction coefficient, in newton-metre-seconds per radian.
    pub viscous_nms: f64,
    /// Speed below which Coulomb friction is not applied, in radians per second.
    /// This keeps the torque from chattering around zero velocity.
    pub deadband_rad_s: f64,
}

impl Friction {
    /// Creates friction parameters. Returns `None` if any value is negative or not
    /// finite.
    #[must_use]
    pub fn new(coulomb_nm: f64, viscous_nms: f64, deadband_rad_s: f64) -> Option<Self> {
        let ok = [coulomb_nm, viscous_nms, deadband_rad_s]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0);
        ok.then_some(Self {
            coulomb_nm,
            viscous_nms,
            deadband_rad_s,
        })
    }
}

/// The position Jacobian, 3×7, at joint angles `q`. Row `k` is the derivative of
/// position component `k` with respect to each joint.
///
/// Returns `None` if the angles are not finite.
#[must_use]
pub fn position_jacobian(arm: &Arm, q: [f64; 7]) -> Option<[[f64; 7]; 3]> {
    let mut jac = [[0.0; 7]; 3];
    for j in 0..7 {
        let mut plus = q;
        let mut minus = q;
        plus[j] += JACOBIAN_STEP_RAD;
        minus[j] -= JACOBIAN_STEP_RAD;
        let p_plus = arm.forward(plus)?.position_m;
        let p_minus = arm.forward(minus)?.position_m;
        for k in 0..3 {
            jac[k][j] = (p_plus[k] - p_minus[k]) / (2.0 * JACOBIAN_STEP_RAD);
        }
    }
    Some(jac)
}

/// Joint torques that produce the Cartesian force `force_n` on the tool: `tau =
/// Jᵀ F`. Returns `None` if the angles or the force are not finite.
#[must_use]
pub fn joint_torque_from_force(arm: &Arm, q: [f64; 7], force_n: [f64; 3]) -> Option<[f64; 7]> {
    if !force_n.iter().all(|v| v.is_finite()) {
        return None;
    }
    let jac = position_jacobian(arm, q)?;
    Some(core::array::from_fn(|j| {
        (0..3).map(|k| jac[k][j] * force_n[k]).sum()
    }))
}

/// Torques that hold the arm against gravity, with `link_masses_kg[i]` at the
/// frame of joint `i + 1`.
///
/// Returns `None` if the angles are not finite, or any mass is negative or not
/// finite.
#[must_use]
pub fn gravity_torque(arm: &Arm, q: [f64; 7], link_masses_kg: [f64; 7]) -> Option<[f64; 7]> {
    if !link_masses_kg.iter().all(|m| m.is_finite() && *m >= 0.0) {
        return None;
    }
    let frames = arm.joint_frames(q)?;
    let weight = [0.0, 0.0, -GRAVITY_M_S2];
    let mut tau = [0.0; 7];
    for (j, joint) in frames.iter().enumerate() {
        // A joint carries every link that is distal to it, so links j and beyond.
        for (i, mass) in link_masses_kg.iter().enumerate().skip(j) {
            let point = frames[i].origin_m;
            let r = sub(point, joint.origin_m);
            let force = scale(weight, *mass);
            tau[j] += dot(joint.axis, cross(r, force));
        }
    }
    Some(tau)
}

/// Friction torque for each joint at speeds `qdot`.
///
/// Returns `None` if a speed is not finite.
#[must_use]
pub fn friction_torque(qdot: [f64; 7], friction: Friction) -> Option<[f64; 7]> {
    if !qdot.iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(qdot.map(|v| {
        let coulomb = if v.abs() > friction.deadband_rad_s {
            friction.coulomb_nm * v.signum()
        } else {
            0.0
        };
        coulomb + friction.viscous_nms * v
    }))
}

/// Clamps each joint torque to `±limit_nm[j]`. Returns the clamped torques, and
/// `true` if any joint was clamped.
///
/// A non-finite torque or limit is not clamped to a usable value. It makes the
/// function return `None`.
#[must_use]
pub fn saturate(torque: [f64; 7], limit_nm: [f64; 7]) -> Option<([f64; 7], bool)> {
    if !torque.iter().chain(limit_nm.iter()).all(|v| v.is_finite()) {
        return None;
    }
    if limit_nm.iter().any(|l| *l < 0.0) {
        return None;
    }
    let mut clamped = false;
    let out = core::array::from_fn(|j| {
        let t = torque[j].clamp(-limit_nm[j], limit_nm[j]);
        clamped |= torque[j].abs() > limit_nm[j];
        t
    });
    Some((out, clamped))
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: [f64; 7] = [0.4, -0.5, 0.2, -1.2, 0.3, 0.9, -0.4];

    fn dq() -> [f64; 7] {
        [1e-6, -2e-6, 3e-6, 1e-6, -1e-6, 2e-6, -3e-6]
    }

    fn step(q: [f64; 7], d: [f64; 7]) -> [f64; 7] {
        core::array::from_fn(|j| q[j] + d[j])
    }

    #[test]
    fn force_to_torque_obeys_virtual_work() {
        // For a small joint step, the work done by the joints equals the work
        // done by the force at the tool: tau . dq = F . dp.
        let arm = Arm::reference();
        let f = [3.0, -1.0, 2.0];
        let tau = joint_torque_from_force(&arm, Q, f).expect("tau");
        let dp = {
            let a = arm.forward(Q).expect("a").position_m;
            let b = arm.forward(step(Q, dq())).expect("b").position_m;
            sub(b, a)
        };
        let joint_work: f64 = tau.iter().zip(dq()).map(|(t, d)| t * d).sum();
        let tool_work = dot(f, dp);
        assert!(
            (joint_work - tool_work).abs() < 1e-9,
            "{joint_work} vs {tool_work}"
        );
    }

    #[test]
    fn a_zero_force_needs_no_torque() {
        let arm = Arm::reference();
        let tau = joint_torque_from_force(&arm, Q, [0.0; 3]).expect("tau");
        assert!(tau.iter().all(|t| *t == 0.0));
    }

    #[test]
    fn gravity_torque_obeys_virtual_work() {
        // A small joint step changes each link's point by dp_i, and gravity does
        // -sum(m_i g . dp_i). The joint torques must do the same work.
        let arm = Arm::reference();
        let masses = [4.0, 3.0, 3.0, 2.5, 2.0, 1.5, 1.0];
        let tau = gravity_torque(&arm, Q, masses).expect("tau");
        let before = arm.joint_frames(Q).expect("before");
        let after = arm.joint_frames(step(Q, dq())).expect("after");
        let mut gravity_work = 0.0;
        for (i, m) in masses.iter().enumerate() {
            let dp = sub(after[i].origin_m, before[i].origin_m);
            gravity_work -= m * GRAVITY_M_S2 * dp[2];
        }
        let joint_work: f64 = tau.iter().zip(dq()).map(|(t, d)| t * d).sum();
        assert!(
            (joint_work - gravity_work).abs() < 1e-9,
            "{joint_work} vs {gravity_work}"
        );
    }

    #[test]
    fn gravity_is_zero_with_no_mass() {
        let arm = Arm::reference();
        let tau = gravity_torque(&arm, Q, [0.0; 7]).expect("tau");
        assert!(tau.iter().all(|t| *t == 0.0));
    }

    #[test]
    fn friction_opposes_motion_and_is_not_applied_in_the_deadband() {
        let f = Friction::new(0.5, 0.1, 0.01).expect("friction");
        let fast = friction_torque([1.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0], f).expect("fast");
        assert!((fast[0] - 0.6).abs() < 1e-12);
        assert!((fast[1] + 0.6).abs() < 1e-12);
        // Inside the deadband only the viscous term acts.
        let slow = friction_torque([0.005, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], f).expect("slow");
        assert!((slow[0] - 0.0005).abs() < 1e-12);
    }

    #[test]
    fn friction_parameters_must_be_non_negative() {
        assert!(Friction::new(-1.0, 0.0, 0.0).is_none());
        assert!(Friction::new(0.0, f64::NAN, 0.0).is_none());
    }

    #[test]
    fn saturation_clamps_and_reports() {
        let limits = [2.0; 7];
        let (out, clamped) =
            saturate([3.0, -2.5, 1.0, 0.0, 0.0, 0.0, 0.0], limits).expect("saturate");
        assert!((out[0] - 2.0).abs() < 1e-12);
        assert!((out[1] + 2.0).abs() < 1e-12);
        assert!((out[2] - 1.0).abs() < 1e-12);
        assert!(clamped);
        let (_, clamped) = saturate([1.0; 7], limits).expect("saturate");
        assert!(!clamped);
    }

    #[test]
    fn non_finite_values_are_refused_not_clamped() {
        assert!(saturate([f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], [1.0; 7]).is_none());
        assert!(saturate([0.0; 7], [-1.0; 7]).is_none());
        assert!(joint_torque_from_force(&Arm::reference(), Q, [f64::INFINITY, 0.0, 0.0]).is_none());
    }
}
