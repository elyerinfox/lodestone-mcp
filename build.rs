//! Build script: compiles the Nuxt dashboard into static files so
//! `include_dir!` can embed them into the binary at compile time.
//!
//! Runs **conditionally** — if `npm` isn't on PATH, or `frontend/`
//! doesn't exist, or the contributor explicitly opts out via
//! `LODESTONE_SKIP_FRONTEND=1`, we print a warning and skip the build.
//! Cargo build still succeeds; the dashboard route at runtime serves a
//! "dashboard not built" page instead of the SPA. This keeps the
//! backend buildable for contributors without Node.
//!
//! Triggers re-run only when the frontend SOURCE changes (not on every
//! cargo build) via `cargo:rerun-if-changed` directives scoped to the
//! files the Nuxt build actually consumes.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=LODESTONE_SKIP_FRONTEND");
    println!("cargo:rerun-if-changed=frontend/package.json");
    println!("cargo:rerun-if-changed=frontend/nuxt.config.ts");
    println!("cargo:rerun-if-changed=frontend/tailwind.config.ts");
    println!("cargo:rerun-if-changed=frontend/tsconfig.json");
    println!("cargo:rerun-if-changed=frontend/app.vue");
    println!("cargo:rerun-if-changed=frontend/types");
    println!("cargo:rerun-if-changed=frontend/composables");
    println!("cargo:rerun-if-changed=frontend/layouts");
    println!("cargo:rerun-if-changed=frontend/pages");
    println!("cargo:rerun-if-changed=frontend/components");

    let frontend_dir = Path::new("frontend");
    let output_dir = frontend_dir.join(".output").join("public");

    // include_dir!() fails on a missing directory — always create the
    // target so the embed step has something to read even when the
    // Nuxt build doesn't run.
    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        println!(
            "cargo:warning=could not create {}: {e}",
            output_dir.display()
        );
        return;
    }

    if std::env::var("LODESTONE_SKIP_FRONTEND")
        .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
    {
        println!("cargo:warning=LODESTONE_SKIP_FRONTEND set — dashboard build skipped");
        return;
    }

    if !frontend_dir.join("package.json").is_file() {
        println!(
            "cargo:warning=frontend/package.json missing — dashboard build skipped \
             (route at runtime will say 'not built')"
        );
        return;
    }

    if which("npm").is_none() {
        println!(
            "cargo:warning=npm not on PATH — dashboard build skipped. Install Node.js \
             and run `cargo clean && cargo build` to build the dashboard."
        );
        return;
    }

    // Try `npm ci` first if a lockfile exists (reproducible builds);
    // fall back to `npm install` on failure (lockfile drift,
    // version mismatch with the local Node, or a first-build state
    // where the lockfile is stale relative to package.json).
    let lock_exists = frontend_dir.join("package-lock.json").is_file();
    let install_ok = if lock_exists {
        run_npm(frontend_dir, &["ci"]) || {
            println!("cargo:warning=npm ci failed, falling back to npm install");
            run_npm(frontend_dir, &["install"])
        }
    } else {
        run_npm(frontend_dir, &["install"])
    };
    if !install_ok {
        println!("cargo:warning=npm install failed — dashboard not built");
        return;
    }
    if !run_npm(frontend_dir, &["run", "generate"]) {
        println!("cargo:warning=npm run generate failed — dashboard not built");
        return;
    }

    println!(
        "cargo:warning=dashboard built into {} — will be embedded into the binary",
        output_dir.display()
    );
}

fn run_npm(cwd: &Path, args: &[&str]) -> bool {
    // On Windows the executable is `npm.cmd`; on Unix it's `npm`. Try
    // both so the build script works cross-platform without
    // os-specific branches in the caller.
    for bin in ["npm", "npm.cmd"] {
        match Command::new(bin).args(args).current_dir(cwd).status() {
            Ok(status) if status.success() => return true,
            Ok(status) => {
                println!(
                    "cargo:warning={bin} {} exited with {status}",
                    args.join(" ")
                );
                return false;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                println!("cargo:warning=could not spawn {bin}: {e}");
                return false;
            }
        }
    }
    false
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    for candidate in [bin, &format!("{bin}.cmd")] {
        let probe = Command::new(candidate).arg("--version").output();
        if probe.is_ok() {
            return Some(std::path::PathBuf::from(candidate));
        }
    }
    None
}
