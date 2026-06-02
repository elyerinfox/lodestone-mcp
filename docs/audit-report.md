# Skill correctness audit (0.1.6)

A cross-codebase audit of every skill that makes factual or mathematical
claims. Findings were collected by six parallel research agents, each
reading the relevant Rust source and cross-checking against canonical
sources (IEEE/ISO/IUPAC/NIST/CODATA, vendor handbooks, original papers).

This document captures the **findings** (✓ verified, ⚠ caveat surfaced,
✗ bug fixed) and the **actions taken** in 0.1.6.

## Already validated upstream

The following 0.1.4 / 0.1.5 modules landed with citation-first development
and are not re-audited here — see their per-skill docs for sources:

- `chemistry`, `biology`, `bio_data`, `nuclear`, `radiology`, `machinist`,
  `cnc`.

## Tier 1 — confirmed wrong-answer bugs (fixed in 0.1.6)

| File | Item | Source | Fix |
| --- | --- | --- | --- |
| `astro.rs` | `equ_to_topo` returned **south-referenced** azimuth but passed it straight to `compass()` (a body due south reported as **N**). Affected every astro tool (sun, moon, star, visible_stars). | Meeus 13.6 vs north-referenced compass. | Added 180° to the Meeus-form azimuth before normalizing. |
| `astro.rs` | Lunar β third-largest coefficient was `0.278 sin(M′ − F)`; Meeus Table 47.B specifies `+0.173 sin(M′ − F)`. ~0.1° declination error. | Meeus *Astronomical Algorithms* 2nd ed., Table 47.B. | Replaced coefficient. |
| `astro.rs` | Moon rise/set threshold was `0.567°` (refraction only); standard h₀ = +0.125° accounts for parallax and semi-diameter. | Meeus §15. | Changed to +0.125°. |
| `satellite.rs` | `sat_passes` set-time bisection passed `(t, t − step)` as `(lo, hi)`, inverting the `lo < hi` contract — the bisection walked backward and could return the wrong endpoint by up to one step. | `refine_horizon` contract. | Reordered to `(t − step, t)`. |
| `dsp_advanced.rs` | M-QAM BER erfc argument was missing the `1/√2` factor (`(M−1)` rather than `2(M−1)` in the denominator). Understated BER by ~3 dB effective SNR. | Proakis 5e §5.2.9. | Replaced denominator with `2·(M−1)`. |
| `rf_link.rs` | Hata large-city threshold was `f ≥ 200 MHz`; Hata's published spec is `f ≤ 200` and `f ≥ 400` with the 200-400 band undefined. | Hata 1980 IEEE TVT 29(3). | Changed to `f ≥ 400 MHz`. |
| `rf_link.rs` | Knife-edge docstring claimed "h positive = clear" but the formula `v = h·√(…)` follows the standard "h positive = obstructing" convention. | Rappaport 2002 §4.7; ITU-R P.526. | Rewrote field and tool descriptions to match the formula (obstructing convention). |
| `geodesy.rs` | MGRS row-letter set selection used `(set / 3) % 2`, flipping the scheme every 3 zones (so zones 1–3 shared a row set, then 4–6 the other). NGA TM 8358.1 alternates the row pattern **every zone** (odd vs even). MGRS strings were wrong for ~2/3 of UTM zones in both `mgrs_forward` and `mgrs_inverse`. | NGA TM 8358.1 §3.2.2.3. | Replaced selector with `(zone − 1) % 2`. |

## Tier 2 — guard rails / docs (also fixed in 0.1.6)

| File | Item | Fix |
| --- | --- | --- |
| `linalg.rs` | `linalg_eigen` silently symmetrized any input via `(A + Aᵀ)/2`, hiding caller bugs. | Added `‖A − Aᵀ‖ > 1e-9·‖A‖` rejection before the symmetric solve. |
| `optimization.rs` | `opt_tsp_2opt` assumes a symmetric distance matrix (2-opt's delta is only valid then), but didn't check. | Added explicit symmetry check with clear error pointing at the violating cell. |
| `nav_aiding.rs` | Saastamoinen `height_m` arg silently unused; full model needs `B(h)` and `δR(h, z)`. | Documented the omission inline; height stays on the schema for forward-compat. |
| `geodesy.rs` | `geodetic_from_ecef` doc claimed "converges in 2–3 iterations" but executes Bowring's closed-form single pass. | Rewrote description to call out closed-form Bowring 1976 + the polar-singularity caveat. |
| `crypto_math.rs` | PBKDF2 description recommended "≥ 100 000 today"; current OWASP/NIST guidance is **600 000**. | Updated. |
| `crypto_math.rs` | Argon2 description labeled defaults as "OWASP 2023 low-end"; OWASP's actual floor is t=2 / m=19 MiB / p=1. | Reframed as "conservative middle, between OWASP floor and high-end". |
| `info_theory.rs` | `crc16-ccitt` resolved to KERMIT parameters without saying which of the three "CCITT-16" variants. | Description now spells out KERMIT vs XMODEM vs CCITT-FALSE and which is used. |
| `trigonometry.rs` | Summaries for `arc_length` and `sector_area` showed `s = r·θ` next to a "degrees" annotation; the formula is only valid for θ in radians (the code does convert internally). | Updated summaries to show `s = r·θ·π/180`. |
| `units.rs` | Volume units were US customary but description didn't say so; data prefixes had no kb/kib disambiguation note. | Description now spells out **US customary** vs imperial and **decimal vs binary** prefixes (IEC 80000-13). |

## Tier 3 — caveats noted, no code change

Found by the audit and considered acceptable as-is given each tool's
documented scope. They live in the table below in case a follow-up
release wants to address them.

### Physics constants (`physics.rs`)

Six constants are CODATA 2018 (not CODATA 2022): `MU0`, `EPSILON0`,
`M_E`, `M_P`, `K_E`, `R_RYDBERG`. All discrepancies are at <1e-8
relative — functionally negligible — but if the project ever advertises
CODATA 2022 currency, these should be refreshed.

### Geometry / arithmetic (`geometry.rs`, `arithmetic.rs`)

- Haversine becomes numerically unstable at antipodes (sqrt(a) > 1 →
  asin NaN). Robust alternative: `2·R·atan2(√a, √(1−a))` (Sinnott
  1984). Practical impact only for >19 000 km separations.
- `arithmetic_eval` uses radians; `trig_formula` uses degrees. Both
  docstrings now lead with the unit, but the asymmetry remains by
  design.

### Radar / DSP (`radar.rs`, `dsp_advanced.rs`)

- CA-CFAR docstring is inconsistent between "one side" and "2N
  reference cells"; formula uses `n` as total. Pick one — out of
  scope for this audit.
- Non-coherent integration loss approximation `5·log N/(N+1)` is
  reasonable only for small N (≤ ~10).
- OS-CFAR α is a CA-style approximation; full Rohling solution requires
  inverting the beta function.
- Rayleigh-fading BER fallback `ber_awgn × 4` is unmotivated; closed
  forms exist (Simon & Alouini Ch. 9).
- ITU-R P.676 and P.838 implementations are simplified fits; the
  official Recommendations use multi-term line sums.

### Forecast (`forecast.rs`)

- Holt-Winters seasonal seed uses only the first season and isn't
  centered to ΣS = 0; γ-updates wash this out after a few periods.
- 95 % band uses the random-walk `1.96·σ·√h` form, which understates
  h-step variance for Holt and Holt-Winters. Documented as "rough" in
  the description.

### Navigation aiding (`nav_aiding.rs`)

- Saastamoinen `B(h)` and `δR(h, z)` corrections are omitted (now
  documented).

### Quaternion (`quaternion.rs`)

- Uses `nalgebra::UnitQuaternion::from_euler_angles` which applies
  Rz·Ry·Rx (intrinsic XYZ). The choice is documented in the module
  preamble; users mixing it with a different convention (e.g. ZYX) will
  get rotated results.

### Tracking / optimization

- KF uses standard `(I − KH)P` rather than Joseph form (numerically
  asymmetric P over many steps). Documented in the description.

### MGRS inverse (`geodesy.rs`)

- Northing reconstruction snaps to the 2 000 000 m band closest to the
  band center; near band edges and at very high latitudes this can pick
  the wrong "ladder". Robust fix: iterate the candidate northing until
  the recovered lat sits inside the input band.

## Methodology

For each cluster, an agent received the module list, a list of known
canonical sources to check against, and the audit goal. Each agent
returned a per-tool table with status flags. The audit consolidated the
findings into Tier 1 (wrong answers, fixed in 0.1.6), Tier 2 (guard
rails / docs, also fixed in 0.1.6), and Tier 3 (caveats accepted as-is
for this release). All tier-1 and tier-2 fixes ship under a single
commit; the test suite stays green (347/347 unit tests + 163/163
end-to-end skill smoketest).

## Sources cited in the audit

- IUGG mean Earth radius / WGS84 datum definitions.
- NIST SP 811 (unit conversions, exact factors); IEC 80000-13 (binary
  prefixes).
- Bronshtein & Semendyayev *Handbook of Mathematics* (algebra,
  geometry, trig).
- ISO 80000-2 (mathematical notation).
- CODATA 2018 / 2022 (physical constants).
- Vallado, *Fundamentals of Astrodynamics*, AIAA 2006-6753 (SGP4),
  Bate-Mueller-White §6.
- Meeus, *Astronomical Algorithms* 2nd ed. (Sun / Moon / sidereal /
  rise-set conventions).
- USSA 1976 (atmospheric model).
- Stull 2011 (wet-bulb), Magnus-Tetens / WMO (dewpoint).
- Holt 1957 ONR memo; Winters 1960 *Management Sci.*; Hyndman &
  Athanasopoulos *FPP3*.
- Brealey/Myers/Allen *Principles of Corporate Finance*; Hull
  *Options, Futures and Other Derivatives*.
- Frankfurter API (FX convention check).
- Rappaport, *Wireless Communications*; Hata 1980; COST 231 Final
  Report; ITU-R P.676, P.838, P.526; Friis 1946; Lee, *Mobile
  Communications Engineering*; Balanis, *Antenna Theory*; Proakis,
  *Digital Communications*; Simon & Alouini, *Digital Communication
  over Fading Channels*; Skolnik, *Introduction to Radar Systems*;
  Richards, *Fundamentals of Radar Signal Processing*; Rohling 1983
  (OS-CFAR); Oppenheim & Schafer, *Discrete-Time Signal Processing*;
  Marple 1999 *IEEE TSP* (Hilbert via FFT); A&S Handbook 7.1.26
  (erfc).
- Mackenzie 1981 *JASA* (sea-water sound speed); Thorp 1967 (absorption);
  Urick, *Principles of Underwater Sound*.
- Sutton & Graves 1971 NASA TR R-376 (reentry heating); Hoerner
  *Fluid-Dynamic Drag*.
- Karney 2013 *J. Geod.* 87:43 (geodesy); Vincenty 1975 *Survey
  Review*; NIMA TR8350.2 (WGS84); Hofmann-Wellenhof *GNSS*; ICD-GPS-200L
  (Klobuchar); Saastamoinen 1972 *Bull. Géod.* 105; Bowring 1976
  *Survey Review*; Misra & Enge *GPS*; Kaplan & Hegarty *GPS/GNSS*;
  NGA TM 8358.1 (UTM/MGRS).
- Bar-Shalom *Estimation with Applications*; Titterton & Weston
  *Strapdown Inertial Navigation*; Fischler & Bolles 1981 *CACM*
  (RANSAC); OGC 06-103r4 (WKT); RFC 7946 (GeoJSON); NMEA-0183 v4.10;
  MITRE CoT schema.
- Cover & Thomas *Elements of Information Theory*; MacWilliams &
  Sloane *Theory of Error-Correcting Codes*; CCSDS 131.0-B-4
  (convolutional encoding).
- crc-catalog (Rust `crc` crate); Modbus over Serial v1.02; ITU-T
  X.25 / ISO 13239; IEEE 802.3 / RFC 1952 (CRC-32); RFC 3720
  (CRC-32C); ECMA-182 / ISO 3309 (CRC-64).
- RFC 5869 (HKDF); RFC 8018 (PBKDF2); RFC 9106 (Argon2); RFC 2104,
  FIPS 198-1 (HMAC); RFC 7519, RFC 7515 (JWT); Knuth Vol 2 §4.5
  (Miller-Rabin, mod inverse, CRT); OWASP Password Storage Cheat Sheet
  2023; NIST SP 800-132 / SP 800-90B.
