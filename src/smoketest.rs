//! End-to-end smoketest for the batch 1/2/3 skill modules — constructs a
//! real `Lodestone`, then invokes every new tool through `Skill::call` with
//! realistic args, asserting each returns a success result.
//!
//! Test-only; behind `#[cfg(test)]`.

#![cfg(test)]

use std::sync::Arc;

use rmcp::model::{CallToolResult, JsonObject};
use serde_json::Value;

use crate::config::Config;
use crate::provider::Registry;
use crate::skills::{all_skills, Skill, SkillCtx};
use crate::Lodestone;

async fn make_server() -> Lodestone {
    let mut cfg = Config::default();
    cfg.memory.enabled = false;
    cfg.bind = "127.0.0.1:0".into();
    let cfg = Arc::new(cfg);
    let registry = Arc::new(Registry::from_config(&cfg, None, None));
    let memory = crate::skills::memory::Memory::new(cfg.memory.clone())
        .await
        .expect("memory init");
    Lodestone::new(
        registry,
        cfg.stackexchange.default_site.clone(),
        cfg.stackexchange.key.clone(),
        cfg.stackexchange.allowed_sites.clone(),
        cfg.github.token.clone(),
        cfg.nasa.key.clone(),
        cfg.eia.key.clone(),
        cfg.serial.clone(),
        cfg.search.timeout_secs,
        None,
        cfg.retrieval.default_chars,
        cfg.retrieval.max_chars,
        cfg.docker.clone(),
        cfg.kubernetes.clone(),
        cfg.filesystem.clone(),
        cfg.shell.clone(),
        cfg.git.clone(),
        cfg.databases.clone(),
        None,
        memory,
        cfg.python.clone(),
        cfg.systemd.clone(),
        None,
        cfg.clone(),
        &cfg.tools.enabled,
        &[],
    )
}

fn obj(v: Value) -> JsonObject {
    match v {
        Value::Object(m) => m,
        _ => panic!("expected JSON object"),
    }
}

async fn call(server: &Lodestone, skill: &dyn Skill, args: Value) -> CallToolResult {
    let ctx = SkillCtx {
        server,
        args: obj(args),
        peer: None,
        meta: None,
    };
    skill.call(ctx).await.unwrap_or_else(|e| {
        panic!("skill `{}` failed: {}", skill.name(), e.message);
    })
}

/// (tool_name, args_json) pairs to smoketest. Network-only feeds and tools
/// that require a remote endpoint (open_data_*, atm_space_weather_kp,
/// crypto_jwt_decode of a remote token) are deliberately omitted — the
/// smoketest exercises algorithmic correctness, not network reachability.
fn smoketests() -> Vec<(&'static str, Value)> {
    use serde_json::json;
    vec![
        // ── batch 1: linalg ────────────────────────────────────────────────
        ("linalg_solve", json!({
            "a": [[2.0, 1.0], [1.0, 3.0]],
            "b": [5.0, 10.0]
        })),
        ("linalg_lstsq", json!({
            "a": [[1.0, 1.0], [1.0, 2.0], [1.0, 3.0]],
            "b": [1.0, 2.0, 2.0]
        })),
        ("linalg_svd", json!({"matrix": [[1.0, 0.0], [0.0, 2.0]]})),
        ("linalg_eigen", json!({"matrix": [[2.0, 0.0], [0.0, 3.0]]})),
        ("linalg_qr", json!({"matrix": [[1.0, 0.0], [0.0, 1.0]]})),
        ("linalg_inv", json!({"matrix": [[1.0, 2.0], [3.0, 4.0]]})),
        ("linalg_det", json!({"matrix": [[1.0, 2.0], [3.0, 4.0]]})),
        ("linalg_rank", json!({"matrix": [[1.0, 0.0], [0.0, 1.0]]})),
        ("linalg_norm", json!({"vector": [3.0, 4.0]})),
        ("linalg_matmul", json!({
            "a": [[1.0, 2.0], [3.0, 4.0]],
            "b": [[5.0, 6.0], [7.0, 8.0]]
        })),
        // ── batch 1: quaternion ───────────────────────────────────────────
        ("quat_from_euler", json!({"roll": 0.1, "pitch": 0.2, "yaw": 0.3})),
        ("quat_to_euler", json!({"q": [1.0, 0.0, 0.0, 0.0]})),
        ("quat_compose", json!({
            "a": [1.0, 0.0, 0.0, 0.0],
            "b": [0.7071, 0.7071, 0.0, 0.0]
        })),
        ("quat_rotate", json!({
            "q": [1.0, 0.0, 0.0, 0.0],
            "v": [1.0, 0.0, 0.0]
        })),
        ("quat_conjugate", json!({"q": [0.5, 0.5, 0.5, 0.5]})),
        ("quat_normalize", json!({"q": [2.0, 0.0, 0.0, 0.0]})),
        ("quat_slerp", json!({
            "a": [1.0, 0.0, 0.0, 0.0],
            "b": [0.7071, 0.7071, 0.0, 0.0],
            "t": 0.5
        })),
        ("frame_dcm_from_euler", json!({"roll": 0.0, "pitch": 0.0, "yaw": 1.5708})),
        // ── batch 1: ode ──────────────────────────────────────────────────
        ("ode_rk4", json!({
            "rhs": ["-y0"],
            "y0": [1.0],
            "t_start": 0.0,
            "t_end": 1.0,
            "steps": 100
        })),
        // ── batch 1: geodesy ──────────────────────────────────────────────
        ("geo_vincenty_inverse", json!({
            "lat1": 40.7128, "lon1": -74.0060,
            "lat2": 51.5074, "lon2": -0.1278
        })),
        ("geo_vincenty_direct", json!({
            "lat": 40.7128, "lon": -74.0060,
            "azimuth_deg": 90.0, "distance_m": 100000.0
        })),
        ("geo_great_circle_polyline", json!({
            "lat1": 40.0, "lon1": -74.0,
            "lat2": 51.0, "lon2": 0.0,
            "n": 5
        })),
        ("geo_cross_track", json!({
            "lat": 40.5, "lon": -37.0,
            "lat1": 40.0, "lon1": -74.0,
            "lat2": 51.0, "lon2": 0.0
        })),
        ("geo_polygon_area_geodesic", json!({
            "vertices": [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        })),
        ("geo_utm_from_latlon", json!({
            "lat": 40.7128, "lon": -74.0060
        })),
        ("geo_latlon_from_utm", json!({
            "zone": 18, "hemisphere": "N",
            "easting": 583960.0, "northing": 4507523.0
        })),
        ("geo_mgrs_from_latlon", json!({
            "lat": 40.7128, "lon": -74.0060
        })),
        ("geo_ecef_from_latlon", json!({
            "lat": 40.7128, "lon": -74.0060, "alt_m": 10.0
        })),
        ("geo_latlon_from_ecef", json!({
            "x": 1334000.0, "y": -4654000.0, "z": 4138000.0
        })),
        ("geo_helmert", json!({
            "x": 1334000.0, "y": -4654000.0, "z": 4138000.0,
            "tx": 0.0, "ty": 0.0, "tz": 0.0,
            "rx_arcsec": 0.0, "ry_arcsec": 0.0, "rz_arcsec": 0.0,
            "scale_ppm": 0.0
        })),
        // ── batch 1: atmospheric ──────────────────────────────────────────
        ("atm_isa", json!({"altitude_m": 5000.0})),
        ("atm_density_altitude", json!({
            "pressure_pa": 90000.0, "temp_c": 25.0
        })),
        ("atm_dewpoint", json!({"temp_c": 25.0, "rh_pct": 60.0})),
        ("atm_wbgt", json!({"temp_c": 30.0, "rh_pct": 70.0})),
        // ── batch 1: info_theory ──────────────────────────────────────────
        ("it_shannon_capacity", json!({
            "bandwidth_hz": 1.0e6, "snr_linear": 100.0
        })),
        ("it_entropy", json!({"p": [0.5, 0.5]})),
        ("it_kl_divergence", json!({"p": [0.5, 0.5], "q": [0.25, 0.75]})),
        ("it_js_divergence", json!({"p": [0.5, 0.5], "q": [0.25, 0.75]})),
        ("it_mutual_information", json!({
            "joint": [[0.25, 0.25], [0.25, 0.25]]
        })),
        ("code_hamming_distance", json!({"a": "aa", "b": "55"})),
        ("code_crc", json!({"data": "deadbeef", "algorithm": "crc32"})),
        ("code_rs_encode", json!({
            "data_shards": [[1, 2, 3, 4], [5, 6, 7, 8]],
            "parity_shards": 2
        })),
        ("code_convolutional_encode", json!({"data": "ab"})),
        // ── batch 1: crypto_math ──────────────────────────────────────────
        ("crypto_miller_rabin", json!({"n": "1000000007", "rounds": 8})),
        ("crypto_modexp", json!({
            "base": "2", "exponent": "10", "modulus": "1000"
        })),
        ("crypto_mod_inverse", json!({"a": "3", "modulus": "11"})),
        ("crypto_crt", json!({
            "residues": ["2", "3", "2"],
            "moduli": ["3", "5", "7"]
        })),
        ("crypto_hkdf", json!({
            "ikm": "0b0b0b0b0b0b0b0b0b0b",
            "salt": "",
            "info": "",
            "length": 32
        })),
        ("crypto_pbkdf2", json!({
            "password": "password",
            "salt_hex": "73616c74",
            "iterations": 1000,
            "length": 32
        })),
        ("crypto_argon2", json!({
            "password": "password",
            "salt_hex": "73616c7473616c74",
            "memory": 8192,
            "time": 1,
            "parallelism": 1,
            "length": 16
        })),
        ("crypto_hmac", json!({
            "key_hex": "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
            "message_hex": "4869205468657265",
            "algorithm": "sha256"
        })),
        ("crypto_jwt_decode", json!({
            "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
        })),
        // ── batch 2: rf_link ──────────────────────────────────────────────
        ("rf_two_ray_path_loss", json!({
            "frequency_hz": 900.0e6,
            "tx_height_m": 30.0, "rx_height_m": 2.0, "distance_m": 1000.0
        })),
        ("rf_hata_path_loss", json!({
            "frequency_mhz": 900.0, "bs_height_m": 30.0, "mobile_height_m": 1.5,
            "distance_km": 5.0, "environment": "urban_large"
        })),
        ("rf_cost231_path_loss", json!({
            "frequency_mhz": 1800.0, "bs_height_m": 30.0, "mobile_height_m": 1.5,
            "distance_km": 5.0, "environment": "medium_small_cities"
        })),
        ("rf_egli_path_loss", json!({
            "frequency_mhz": 900.0, "tx_height_m": 30.0, "rx_height_m": 1.5,
            "distance_km": 5.0
        })),
        ("rf_itu_p676_absorption", json!({
            "frequency_ghz": 10.0
        })),
        ("rf_itu_p838_rain", json!({
            "frequency_ghz": 12.0, "rain_rate_mm_h": 25.0, "polarization": "horizontal"
        })),
        ("rf_doppler_shift", json!({
            "frequency_hz": 1.5e9, "velocity_m_s": 100.0
        })),
        ("rf_polarization_loss", json!({
            "tx": "linear_h", "rx": "linear_v"
        })),
        ("rf_fresnel_zone_radius", json!({
            "frequency_hz": 2.4e9, "distance_m": 2000.0,
            "distance_to_obstruction_m": 1000.0
        })),
        ("rf_knife_edge_diffraction", json!({
            "frequency_hz": 2.4e9, "d1_m": 1000.0, "d2_m": 1000.0, "h_m": 5.0
        })),
        ("rf_friis_with_noise", json!({
            "frequency_hz": 2.4e9, "distance_m": 1000.0,
            "tx_power_dbm": 30.0, "tx_gain_dbi": 10.0, "rx_gain_dbi": 10.0,
            "bandwidth_hz": 1.0e6
        })),
        // ── batch 2: radar ────────────────────────────────────────────────
        ("radar_monostatic", json!({
            "pt_w": 1000.0, "gain": 1000.0,
            "wavelength_m": 0.03, "rcs_m2": 1.0,
            "range_m": 10000.0, "bandwidth_hz": 1.0e6
        })),
        ("radar_bistatic", json!({
            "pt_w": 1000.0, "gt": 1000.0, "gr": 1000.0,
            "wavelength_m": 0.03, "sigma_b_m2": 1.0,
            "rt_m": 10000.0, "rr_m": 10000.0, "bandwidth_hz": 1.0e6
        })),
        ("radar_integration_gain", json!({"n": 16, "method": "coherent"})),
        ("radar_pulse_compression_gain", json!({
            "pulse_width_s": 1.0e-6, "bandwidth_hz": 1.0e7
        })),
        ("radar_cfar_threshold", json!({
            "n_cells": 16, "pfa": 1.0e-6, "method": "ca"
        })),
        ("radar_clutter_threshold", json!({
            "distribution": "rayleigh", "pfa": 1.0e-6
        })),
        ("radar_doppler_shift", json!({
            "frequency_hz": 10.0e9, "radial_velocity_m_s": 30.0
        })),
        // ── batch 2: dsp_advanced (signal_ family) ────────────────────────
        ("signal_spectrogram", json!({
            "samples": (0..256).map(|i| (i as f64 * 0.1).sin()).collect::<Vec<_>>(),
            "sample_rate_hz": 1000.0,
            "window_size": 64,
            "overlap": 0.5
        })),
        ("signal_cross_correlation", json!({
            "a": [1.0, 2.0, 3.0, 4.0, 5.0],
            "b": [3.0, 4.0, 5.0, 0.0, 0.0],
            "sample_rate_hz": 1000.0
        })),
        ("signal_hilbert", json!({
            "samples": (0..128).map(|i| (i as f64 * 0.1).sin()).collect::<Vec<_>>(),
            "sample_rate_hz": 1000.0
        })),
        ("signal_cepstrum", json!({
            "samples": (0..128).map(|i| (i as f64 * 0.1).sin() + 0.5 * (i as f64 * 0.05).sin()).collect::<Vec<_>>()
        })),
        ("signal_ber_curve", json!({
            "modulation": "bpsk",
            "ebn0_db": 7.0
        })),
        ("signal_iq_demod", json!({
            "i": (0..128).map(|i| (i as f64 * 0.1).cos()).collect::<Vec<_>>(),
            "q": (0..128).map(|i| (i as f64 * 0.1).sin()).collect::<Vec<_>>()
        })),
        // ── batch 2: tracking ─────────────────────────────────────────────
        ("track_kalman_step", json!({
            "x": [0.0, 0.0],
            "p": [[1.0, 0.0], [0.0, 1.0]],
            "f": [[1.0, 1.0], [0.0, 1.0]],
            "q": [[0.01, 0.0], [0.0, 0.01]],
            "h": [[1.0, 0.0]],
            "r": [[0.1]],
            "z": [1.0]
        })),
        ("track_hungarian", json!({
            "cost": [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]]
        })),
        ("track_ransac_line", json!({
            "points": [[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0], [4.0, 9.0]],
            "threshold": 0.2,
            "iterations": 100
        })),
        // ── batch 2: acoustic ─────────────────────────────────────────────
        ("acoustic_sound_speed_water", json!({
            "temp_c": 15.0, "salinity_psu": 35.0, "depth_m": 100.0
        })),
        ("acoustic_sound_speed_air", json!({"temp_c": 20.0})),
        ("acoustic_snell", json!({
            "incident_deg": 30.0, "c1": 1500.0, "c2": 1530.0
        })),
        ("acoustic_transmission_loss", json!({
            "range_m": 1000.0, "frequency_khz": 10.0
        })),
        ("acoustic_sonar_equation", json!({
            "sl_db": 220.0, "tl_db": 60.0, "ts_db": 10.0,
            "nl_db": 70.0, "dt_db": 10.0
        })),
        // ── batch 2: nav_aiding ───────────────────────────────────────────
        ("nav_dop", json!({
            "los_enu": [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.5, 0.5, 0.7]
            ]
        })),
        ("nav_klobuchar", json!({
            "gps_tow_s": 100000.0,
            "lat_deg": 40.0,
            "lon_deg": -75.0,
            "elevation_deg": 30.0,
            "azimuth_deg": 90.0,
            "alpha": [1.0e-8, 1.5e-8, -5.0e-8, -5.0e-8],
            "beta": [88000.0, 49000.0, -131000.0, -262000.0]
        })),
        ("nav_saastamoinen", json!({
            "height_m": 100.0, "elevation_deg": 30.0
        })),
        ("nav_ecef_to_enu", json!({
            "ref_lat": 40.7128, "ref_lon": -74.0060, "ref_alt_m": 0.0,
            "x": 1334000.0, "y": -4654000.0, "z": 4138000.0
        })),
        ("nav_imu_drift", json!({
            "gyro_random_walk_deg_sqrt_hr": 0.1,
            "bias_instability_deg_per_hr": 1.0,
            "scale_factor_ppm": 100.0,
            "time_s": 600.0
        })),
        // ── batch 2: trajectory ───────────────────────────────────────────
        ("traj_projectile_drag", json!({
            "v0_m_s": 100.0, "angle_deg": 45.0, "mass_kg": 1.0,
            "cd": 0.3, "area_m2": 0.01
        })),
        ("traj_hohmann", json!({
            "mu": 3.986e14, "r1_m": 7000.0e3, "r2_m": 42000.0e3
        })),
        ("traj_reentry_heating", json!({
            "velocity_m_s": 7800.0, "density_kg_m3": 0.001, "nose_radius_m": 0.5
        })),
        // ── batch 2: earth_models ─────────────────────────────────────────
        ("earth_sidereal_time", json!({"longitude_deg": 0.0})),
        ("earth_magnetic_declination", json!({
            "lat_deg": 40.7128, "lon_deg": -74.0060, "year": 2026.0
        })),
        // ── batch 2: optimization ─────────────────────────────────────────
        ("opt_tsp_2opt", json!({
            "distances": [
                [0.0, 1.0, 1.41, 1.0],
                [1.0, 0.0, 1.0, 1.41],
                [1.41, 1.0, 0.0, 1.0],
                [1.0, 1.41, 1.0, 0.0]
            ]
        })),
        ("opt_shortest_path", json!({
            "edges": [[0, 1, 1.0], [1, 2, 2.0], [0, 2, 4.0]],
            "start": 0,
            "goal": 2
        })),
        // ── batch 3: geo_convert ──────────────────────────────────────────
        ("convert_nmea_decode", json!({
            "sentence": "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47"
        })),
        ("convert_cot_encode", json!({
            "uid": "smoke-1",
            "cot_type": "a-f-G-U-C",
            "lat": 40.0,
            "lon": -75.0,
            "hae_m": 100.0
        })),
        ("convert_geojson_to_wkt", json!({
            "geojson": {
                "type": "Point",
                "coordinates": [40.0, -75.0]
            }
        })),
        // ── batch 3: interchange ──────────────────────────────────────────
        ("interchange_stl_info", json!({
            "data_ascii": "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t"
        })),
        // ── batch 3: new_charts ───────────────────────────────────────────
        ("chart_polar", json!({
            "magnitudes": [1.0, 0.7, 0.3, 0.5, 0.9, 0.6, 0.2, 0.4]
        })),
        ("chart_smith", json!({
            "impedances": [[50.0, 0.0], [25.0, 25.0], [75.0, -50.0]]
        })),
        ("chart_waterfall", json!({
            "power": [
                [-80.0, -70.0, -60.0, -65.0],
                [-78.0, -68.0, -55.0, -62.0],
                [-76.0, -66.0, -50.0, -60.0]
            ]
        })),
        ("chart_compass_rose", json!({
            "magnitudes_by_bearing": [
                0.2, 0.3, 0.5, 0.8, 1.0, 0.7, 0.4, 0.3,
                0.2, 0.1, 0.2, 0.3, 0.4, 0.5, 0.4, 0.3
            ]
        })),
        ("chart_skyplot", json!({
            "az_el": [[45.0, 30.0], [120.0, 60.0], [270.0, 15.0]],
            "labels": ["sat1", "sat2", "sat3"]
        })),
        ("chart_density_map", json!({
            "points": (0..200).map(|i| {
                let t = i as f64 * 0.1;
                [t.sin(), t.cos()]
            }).collect::<Vec<_>>()
        })),
    ]
}

#[tokio::test]
async fn smoketest_all_new_skills() {
    let server = make_server().await;
    let registry: std::collections::HashMap<&'static str, Box<dyn Skill>> = all_skills()
        .into_iter()
        .map(|s| (s.name(), s))
        .collect();
    let cases = smoketests();
    let mut missing = Vec::new();
    let mut ran = 0_usize;
    for (name, args) in &cases {
        let Some(skill) = registry.get(name) else {
            missing.push(*name);
            continue;
        };
        let res = call(&server, skill.as_ref(), args.clone()).await;
        assert!(
            !res.content.is_empty(),
            "skill `{name}` returned empty content"
        );
        ran += 1;
    }
    if !missing.is_empty() {
        panic!("skills missing from registry: {missing:?}");
    }
    println!("smoketest_all_new_skills: {ran}/{} tools exercised", cases.len());
}
