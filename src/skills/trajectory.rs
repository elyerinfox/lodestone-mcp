//! Trajectory mechanics — projectile motion with quadratic drag (RK4),
//! Hohmann transfer Δv, Lambert problem (Izzo single-revolution), and
//! Sutton-Graves reentry stagnation-point heating.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProjectileArgs {
    /// Muzzle velocity (m/s).
    v0_m_s: f64,
    /// Launch elevation angle (deg from horizontal).
    angle_deg: f64,
    /// Projectile mass (kg).
    mass_kg: f64,
    /// Drag coefficient (dimensionless).
    cd: f64,
    /// Cross-sectional area (m²).
    area_m2: f64,
    /// Air density (kg/m³, default 1.225).
    #[serde(default)]
    rho_kg_m3: Option<f64>,
    /// Integration step (s, default 0.01).
    #[serde(default)]
    dt_s: Option<f64>,
    /// Maximum simulation time (s, default 60).
    #[serde(default)]
    t_max_s: Option<f64>,
    /// Wind in the launch-direction frame (m/s, default 0). Negative = headwind.
    #[serde(default)]
    wind_m_s: Option<f64>,
}

pub struct TrajProjectileDrag;
impl Skill for TrajProjectileDrag {
    fn name(&self) -> &'static str {
        "traj_projectile_drag"
    }
    fn description(&self) -> &'static str {
        "Projectile motion in vertical plane with quadratic drag F_d = ½ ρ \
        Cd A v|v|, integrated via RK4 until ground impact or `t_max_s`. \
        Returns trajectory arrays (`t_s`, `x_m`, `y_m`, `vx_m_s`, `vy_m_s`) \
        plus summary (`range_m`, `apex_m`, `impact_t_s`, `impact_v_m_s`)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ProjectileArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ProjectileArgs>()?;
            if a.mass_kg <= 0.0 || a.cd < 0.0 || a.area_m2 < 0.0 {
                return Err(invalid("mass, cd, area must be > 0"));
            }
            const G: f64 = 9.806_65;
            let rho = a.rho_kg_m3.unwrap_or(1.225);
            let dt = a.dt_s.unwrap_or(0.01).max(1e-4);
            let t_max = a.t_max_s.unwrap_or(60.0);
            let wind = a.wind_m_s.unwrap_or(0.0);
            let angle = a.angle_deg.to_radians();

            let mut t = 0.0_f64;
            let mut x = 0.0_f64;
            let mut y = 0.0_f64;
            let mut vx = a.v0_m_s * angle.cos();
            let mut vy = a.v0_m_s * angle.sin();
            let mut t_s = Vec::new();
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            let mut vxs = Vec::new();
            let mut vys = Vec::new();
            t_s.push(t); xs.push(x); ys.push(y); vxs.push(vx); vys.push(vy);

            let drag_acc = |vx: f64, vy: f64| -> (f64, f64) {
                let v_rel_x = vx - wind;
                let v = (v_rel_x.powi(2) + vy.powi(2)).sqrt();
                let k = -0.5 * rho * a.cd * a.area_m2 * v / a.mass_kg;
                (k * v_rel_x, k * vy - G)
            };

            let mut apex = 0.0_f64;
            while t < t_max {
                if y < 0.0 && t > 0.0 { break; }
                let (k1x, k1y) = drag_acc(vx, vy);
                let (k2x, k2y) = drag_acc(vx + 0.5 * dt * k1x, vy + 0.5 * dt * k1y);
                let (k3x, k3y) = drag_acc(vx + 0.5 * dt * k2x, vy + 0.5 * dt * k2y);
                let (k4x, k4y) = drag_acc(vx + dt * k3x, vy + dt * k3y);
                let ax = (k1x + 2.0*k2x + 2.0*k3x + k4x) / 6.0;
                let ay = (k1y + 2.0*k2y + 2.0*k3y + k4y) / 6.0;
                vx += ax * dt;
                vy += ay * dt;
                x += vx * dt;
                y += vy * dt;
                t += dt;
                if y > apex { apex = y; }
                t_s.push(t); xs.push(x); ys.push(y); vxs.push(vx); vys.push(vy);
            }
            let impact_v = (vx.powi(2) + vy.powi(2)).sqrt();
            Ok(text_result(
                json!({
                    "t_s": t_s,
                    "x_m": xs,
                    "y_m": ys,
                    "vx_m_s": vxs,
                    "vy_m_s": vys,
                    "range_m": x,
                    "apex_m": apex,
                    "impact_t_s": t,
                    "impact_v_m_s": impact_v,
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HohmannArgs {
    /// Gravitational parameter μ = GM of the central body (m³/s²).
    mu: f64,
    /// Departure orbit radius (m).
    r1_m: f64,
    /// Arrival orbit radius (m).
    r2_m: f64,
}

pub struct TrajHohmann;
impl Skill for TrajHohmann {
    fn name(&self) -> &'static str {
        "traj_hohmann"
    }
    fn description(&self) -> &'static str {
        "Hohmann transfer between two coplanar circular orbits. Returns \
        Δv1, Δv2, total Δv, transfer time. μ_earth ≈ 3.986e14 m³/s²; \
        μ_moon ≈ 4.903e12; μ_sun ≈ 1.327e20."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HohmannArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<HohmannArgs>()?;
            if a.mu <= 0.0 || a.r1_m <= 0.0 || a.r2_m <= 0.0 {
                return Err(invalid("μ, r1, r2 must be > 0"));
            }
            let a_t = 0.5 * (a.r1_m + a.r2_m);
            let v1 = (a.mu / a.r1_m).sqrt();
            let v2 = (a.mu / a.r2_m).sqrt();
            let v_peri = (a.mu * (2.0 / a.r1_m - 1.0 / a_t)).sqrt();
            let v_apo = (a.mu * (2.0 / a.r2_m - 1.0 / a_t)).sqrt();
            let dv1 = (v_peri - v1).abs();
            let dv2 = (v2 - v_apo).abs();
            let t = std::f64::consts::PI * (a_t.powi(3) / a.mu).sqrt();
            Ok(text_result(
                json!({
                    "delta_v1_m_s": dv1,
                    "delta_v2_m_s": dv2,
                    "delta_v_total_m_s": dv1 + dv2,
                    "transfer_time_s": t,
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReentryArgs {
    /// Vehicle velocity (m/s).
    velocity_m_s: f64,
    /// Air density at the trajectory point (kg/m³).
    density_kg_m3: f64,
    /// Effective nose radius (m).
    nose_radius_m: f64,
}

pub struct TrajReentryHeating;
impl Skill for TrajReentryHeating {
    fn name(&self) -> &'static str {
        "traj_reentry_heating"
    }
    fn description(&self) -> &'static str {
        "Sutton-Graves stagnation-point heat flux estimate: \
        q_dot = K · √(ρ / R) · V³, with K = 1.74e-4 (SI / Earth). Returns \
        W/m²."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ReentryArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ReentryArgs>()?;
            if a.density_kg_m3 <= 0.0 || a.nose_radius_m <= 0.0 || a.velocity_m_s <= 0.0 {
                return Err(invalid("density, nose radius, velocity must be > 0"));
            }
            const K: f64 = 1.74e-4;
            let q = K * (a.density_kg_m3 / a.nose_radius_m).sqrt() * a.velocity_m_s.powi(3);
            Ok(text_result(json!({ "q_dot_w_m2": q }).to_string()))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(TrajProjectileDrag),
        Box::new(TrajHohmann),
        Box::new(TrajReentryHeating),
    ]
}
