//! Quaternion + reference-frame math skill. Pure math; on by default.
//!
//! Tools: `quat_from_euler`, `quat_to_euler`, `quat_compose`, `quat_rotate`,
//! `quat_conjugate`, `quat_normalize`, `quat_slerp`, `frame_dcm_from_euler`.
//!
//! Convention: Hamilton quaternion (w, x, y, z) with w as scalar; Euler
//! angles are (roll, pitch, yaw) about (X, Y, Z) intrinsic rotations.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use nalgebra::{Matrix3, UnitQuaternion, Vector3};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EulerArgs {
    /// Roll (rotation about X) in radians.
    roll: f64,
    /// Pitch (rotation about Y) in radians.
    pitch: f64,
    /// Yaw (rotation about Z) in radians.
    yaw: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QuatArgs {
    /// Quaternion (w, x, y, z). w is the scalar part.
    q: [f64; 4],
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TwoQuatArgs {
    /// Left quaternion.
    a: [f64; 4],
    /// Right quaternion.
    b: [f64; 4],
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RotateArgs {
    /// Rotation as quaternion (w, x, y, z).
    q: [f64; 4],
    /// 3-vector to rotate.
    v: [f64; 3],
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SlerpArgs {
    /// Start quaternion (w, x, y, z).
    a: [f64; 4],
    /// End quaternion (w, x, y, z).
    b: [f64; 4],
    /// Interpolation parameter [0, 1].
    t: f64,
}

fn quat_from_array(a: [f64; 4]) -> UnitQuaternion<f64> {
    UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(a[0], a[1], a[2], a[3]))
}

fn quat_to_array(q: &UnitQuaternion<f64>) -> [f64; 4] {
    let qi = q.as_ref();
    [qi.w, qi.i, qi.j, qi.k]
}

pub struct QuatFromEuler;
impl Skill for QuatFromEuler {
    fn name(&self) -> &'static str {
        "quat_from_euler"
    }
    fn description(&self) -> &'static str {
        "Build a unit quaternion (w, x, y, z) from intrinsic Euler angles \
        (roll about X, pitch about Y, yaw about Z), in radians. Hamilton \
        convention."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EulerArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EulerArgs>()?;
            let q = UnitQuaternion::from_euler_angles(a.roll, a.pitch, a.yaw);
            Ok(text_result(json!({ "q": quat_to_array(&q) }).to_string()))
        })
    }
}

pub struct QuatToEuler;
impl Skill for QuatToEuler {
    fn name(&self) -> &'static str {
        "quat_to_euler"
    }
    fn description(&self) -> &'static str {
        "Decompose a unit quaternion into (roll, pitch, yaw) intrinsic Euler \
        angles in radians."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<QuatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<QuatArgs>()?;
            let q = quat_from_array(a.q);
            let (r, p, y) = q.euler_angles();
            Ok(text_result(
                json!({ "roll": r, "pitch": p, "yaw": y }).to_string(),
            ))
        })
    }
}

pub struct QuatCompose;
impl Skill for QuatCompose {
    fn name(&self) -> &'static str {
        "quat_compose"
    }
    fn description(&self) -> &'static str {
        "Compose two rotations: `q_result = a * b`. Applies `b` first, then `a`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TwoQuatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<TwoQuatArgs>()?;
            let qa = quat_from_array(args.a);
            let qb = quat_from_array(args.b);
            let q = qa * qb;
            Ok(text_result(json!({ "q": quat_to_array(&q) }).to_string()))
        })
    }
}

pub struct QuatRotate;
impl Skill for QuatRotate {
    fn name(&self) -> &'static str {
        "quat_rotate"
    }
    fn description(&self) -> &'static str {
        "Rotate a 3-vector by a unit quaternion. Returns the rotated vector."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RotateArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<RotateArgs>()?;
            let q = quat_from_array(args.q);
            let v = Vector3::new(args.v[0], args.v[1], args.v[2]);
            let r = q * v;
            Ok(text_result(json!({ "v": [r.x, r.y, r.z] }).to_string()))
        })
    }
}

pub struct QuatConjugate;
impl Skill for QuatConjugate {
    fn name(&self) -> &'static str {
        "quat_conjugate"
    }
    fn description(&self) -> &'static str {
        "Quaternion conjugate (negate the vector part). For a unit quaternion \
        this equals the inverse rotation."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<QuatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<QuatArgs>()?;
            let q = quat_from_array(args.q);
            let c = q.conjugate();
            Ok(text_result(json!({ "q": quat_to_array(&c) }).to_string()))
        })
    }
}

pub struct QuatNormalize;
impl Skill for QuatNormalize {
    fn name(&self) -> &'static str {
        "quat_normalize"
    }
    fn description(&self) -> &'static str {
        "Normalize a (possibly non-unit) quaternion to unit length."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<QuatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<QuatArgs>()?;
            let raw = nalgebra::Quaternion::new(args.q[0], args.q[1], args.q[2], args.q[3]);
            let n = raw.norm();
            if n == 0.0 {
                return Err(invalid("zero quaternion can't be normalized"));
            }
            let unit = raw / n;
            Ok(text_result(
                json!({ "q": [unit.w, unit.i, unit.j, unit.k] }).to_string(),
            ))
        })
    }
}

pub struct QuatSlerp;
impl Skill for QuatSlerp {
    fn name(&self) -> &'static str {
        "quat_slerp"
    }
    fn description(&self) -> &'static str {
        "Spherical linear interpolation between two unit quaternions at \
        parameter `t ∈ [0, 1]`. Use for smooth orientation interpolation."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SlerpArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<SlerpArgs>()?;
            if !(0.0..=1.0).contains(&args.t) {
                return Err(invalid("t must be in [0, 1]"));
            }
            let a = quat_from_array(args.a);
            let b = quat_from_array(args.b);
            let q = a.slerp(&b, args.t);
            Ok(text_result(json!({ "q": quat_to_array(&q) }).to_string()))
        })
    }
}

pub struct FrameDcmFromEuler;
impl Skill for FrameDcmFromEuler {
    fn name(&self) -> &'static str {
        "frame_dcm_from_euler"
    }
    fn description(&self) -> &'static str {
        "Direction-cosine matrix (DCM) from (roll, pitch, yaw) intrinsic \
        Euler angles in radians. Returns the 3×3 rotation matrix."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EulerArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EulerArgs>()?;
            let q = UnitQuaternion::from_euler_angles(a.roll, a.pitch, a.yaw);
            let m: Matrix3<f64> = q.to_rotation_matrix().into_inner();
            let rows: Vec<Vec<f64>> = (0..3)
                .map(|i| (0..3).map(|j| m[(i, j)]).collect())
                .collect();
            Ok(text_result(json!({ "dcm": rows }).to_string()))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(QuatFromEuler),
        Box::new(QuatToEuler),
        Box::new(QuatCompose),
        Box::new(QuatRotate),
        Box::new(QuatConjugate),
        Box::new(QuatNormalize),
        Box::new(QuatSlerp),
        Box::new(FrameDcmFromEuler),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_euler() {
        let q = UnitQuaternion::from_euler_angles(0.1_f64, 0.2_f64, 0.3_f64);
        let (r, p, y) = q.euler_angles();
        assert!((r - 0.1_f64).abs() < 1e-9);
        assert!((p - 0.2_f64).abs() < 1e-9);
        assert!((y - 0.3_f64).abs() < 1e-9);
    }

    #[test]
    fn quarter_turn_about_z() {
        let q = UnitQuaternion::from_euler_angles(0.0, 0.0, std::f64::consts::FRAC_PI_2);
        let v = Vector3::new(1.0, 0.0, 0.0);
        let r = q * v;
        assert!((r.x - 0.0).abs() < 1e-9);
        assert!((r.y - 1.0).abs() < 1e-9);
        assert!(r.z.abs() < 1e-9);
    }
}
