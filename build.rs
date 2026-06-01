//! Build script — DECOUPLED from the dashboard.
//!
//! Earlier versions of this script ran `npm install` + `npm run
//! generate` automatically whenever Node was on PATH. That coupled
//! every `cargo build` to the state of the frontend tree, which made
//! the matrix (Node present? lockfile drifted? embed needed?)
//! surprising for contributors and CI alike.
//!
//! The new contract is small: `include_dir!()` needs the target
//! directory to exist, so we create it. Everything else — installing
//! npm deps, running Nuxt, picking host-Node vs Docker-Node — is
//! orchestrated explicitly from the Makefile (`make frontend`,
//! `make frontend-docker`, `make build-with-dashboard`,
//! `make build-with-dashboard-docker`). Whatever's already in
//! `frontend/.output/public/` at compile time is what the binary
//! embeds. An empty directory → the dashboard route serves a small
//! "not built" page; a populated directory → the SPA ships.
//!
//! See docs/building.md for the workflow.

use std::path::Path;

fn main() {
    // Re-run only when the embed directory's *content* changes. Cargo
    // hashes the directory tree on subsequent builds and re-fires the
    // script when files inside it change — so after `make frontend`
    // / `make frontend-docker` writes a new SPA, the next
    // `cargo build` re-embeds it without a manual `cargo clean`.
    println!("cargo:rerun-if-changed=frontend/.output/public");

    let output_dir = Path::new("frontend").join(".output").join("public");
    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        println!(
            "cargo:warning=could not create {}: {e} \
             (the binary will refuse to embed the dashboard)",
            output_dir.display()
        );
    }
}
