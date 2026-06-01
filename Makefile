# lodestone-mcp — developer-task automation.
#
# Mirrors the pre-commit triad described in CONTRIBUTING.md (golden rule 10):
#   cargo fmt --all
#   cargo build
#   cargo clippy --all-targets -- -D warnings
#   cargo test
# and the Docker build/smoke steps the CI job runs.
#
# Usage:
#   make            show this help
#   make check      pre-commit triad — fmt + build + clippy + test
#   make ci         exactly what CI runs (fmt --check, not fmt)
#   make docker     build the image + run the same /health smoke test as CI
#
# Run `make help` to see every target with one-line descriptions.
#
# Cross-platform note: targets use POSIX `sh` and standard Unix tools (printf,
# rm, mkdir, curl, sleep). On Windows, run from Git Bash, WSL, or invoke the
# cargo commands directly from PowerShell.

CARGO         ?= cargo
DOCKER        ?= docker
NPM           ?= npm
NODE_IMAGE    ?= node:22-bookworm
IMAGE         ?= lodestone-mcp:dev
CONTAINER     ?= lodestone-dev
PORT          ?= 8000
HEALTH_TIMEOUT_SECS ?= 30

# Color helpers for the help banner (NO_COLOR=1 disables).
ifdef NO_COLOR
	BOLD :=
	DIM  :=
	RST  :=
else
	BOLD := \033[1m
	DIM  := \033[2m
	RST  := \033[0m
endif

.DEFAULT_GOAL := help
.PHONY: help fmt fmt-check build build-release clippy test test-live check ci \
        run run-galaxy clean docker docker-build docker-run docker-stop \
        docker-smoke install-hooks doc deps-check \
        frontend frontend-docker frontend-clean \
        build-with-dashboard build-with-dashboard-docker

## ─── Help ───────────────────────────────────────────────────────────────────

help: ## Show this help.
	@printf "$(BOLD)lodestone-mcp$(RST) — make targets\n\n"
	@printf "$(BOLD)Pre-commit (golden rule 10):$(RST)\n"
	@printf "  $(BOLD)make check$(RST)                   fmt + build + clippy + test (run before every commit)\n"
	@printf "  $(BOLD)make ci$(RST)                      exactly what CI runs (fmt --check, not fmt)\n\n"
	@printf "$(BOLD)Backend:$(RST)\n"
	@printf "  $(BOLD)make build$(RST) / $(BOLD)make build-release$(RST)\n"
	@printf "    Never touches Node. The dashboard is built separately (or not at all).\n\n"
	@printf "$(BOLD)Dashboard (opt-in):$(RST)\n"
	@printf "  $(BOLD)make frontend$(RST)                Build the SPA with host Node into frontend/.output/public/.\n"
	@printf "  $(BOLD)make frontend-docker$(RST)         Build the SPA in node:22-bookworm (no host Node required).\n"
	@printf "  $(BOLD)make build-with-dashboard$(RST)    frontend + build-release in one command.\n"
	@printf "  $(BOLD)make build-with-dashboard-docker$(RST)  frontend-docker + build-release.\n\n"
	@printf "$(BOLD)All targets:$(RST)\n"
	@awk 'BEGIN {FS = ":.*##"} /^[a-zA-Z_-]+:.*##/ { printf "  $(BOLD)%-30s$(RST) %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@printf "\n$(DIM)Overrides: CARGO=…  DOCKER=…  NODE_IMAGE=…  IMAGE=…  PORT=…  NO_COLOR=1$(RST)\n"

## ─── Build & verify (pre-commit triad) ─────────────────────────────────────

fmt: ## Reformat the workspace (cargo fmt --all).
	$(CARGO) fmt --all

fmt-check: ## Check formatting without rewriting (CI-style; fails on diff).
	$(CARGO) fmt --all -- --check

build: ## cargo build.
	$(CARGO) build

build-release: ## cargo build --release (used by the Dockerfile).
	$(CARGO) build --release

clippy: ## cargo clippy --all-targets -- -D warnings (deny on warnings).
	$(CARGO) clippy --all-targets -- -D warnings

test: ## cargo test (skips #[ignore] live tests).
	$(CARGO) test

test-live: ## cargo test including #[ignore] live network tests.
	$(CARGO) test -- --include-ignored

check: fmt build clippy test ## The pre-commit triad — golden rule 10.
	@printf "$(BOLD)✔ pre-commit triad clean — safe to git commit.$(RST)\n"

ci: fmt-check clippy build test ## What CI runs (.github/workflows/ci.yml).
	@printf "$(BOLD)✔ CI gate would pass.$(RST)\n"

## ─── Frontend (dashboard SPA) ──────────────────────────────────────────────
#
# The Rust binary embeds the dashboard via `include_dir!()` reading
# `frontend/.output/public/`. The two targets here populate that
# directory; one needs a host Node, the other shells the build out
# into a temporary Node Docker container so the host stays Node-less.
# Either way the output is on the host filesystem and the next
# `cargo build` embeds it.

frontend: ## Build the Nuxt dashboard locally (requires Node + npm on PATH).
	@printf "$(BOLD)Building dashboard with host Node…$(RST)\n"
	cd frontend && $(NPM) ci && $(NPM) run generate
	@printf "$(BOLD)✔ Dashboard built into frontend/.output/public/.$(RST)\n"
	@printf "  $(DIM)Next: 'make build-release' to embed it into the binary.$(RST)\n"

frontend-docker: ## Build the Nuxt dashboard inside a Node Docker container (no host Node required).
	@printf "$(BOLD)Building dashboard with $(NODE_IMAGE) (no host Node)…$(RST)\n"
	# MSYS_NO_PATHCONV=1 stops Git Bash on Windows from rewriting /work
	# into C:/Program Files/Git/work before the path reaches Docker.
	# On macOS / Linux / WSL it's a no-op.
	MSYS_NO_PATHCONV=1 $(DOCKER) run --rm \
		-v "$(CURDIR):/work" -w /work/frontend \
		$(NODE_IMAGE) sh -c "npm ci --no-audit --no-fund && npm run generate"
	@printf "$(BOLD)✔ Dashboard built into frontend/.output/public/ on the host.$(RST)\n"
	@printf "  $(DIM)Next: 'make build-release' to embed it into the binary.$(RST)\n"

frontend-clean: ## Remove the Nuxt build output + node_modules.
	rm -rf frontend/.output frontend/node_modules
	@printf "$(BOLD)✔ Cleaned frontend/.output and frontend/node_modules.$(RST)\n"

build-with-dashboard: frontend build-release ## Build the dashboard (host Node) then a release binary.
	@printf "$(BOLD)✔ Release binary with embedded dashboard at target/release/lodestone-mcp.$(RST)\n"

build-with-dashboard-docker: frontend-docker build-release ## Build the dashboard (Docker Node) then a release binary on the host.
	@printf "$(BOLD)✔ Release binary with embedded dashboard at target/release/lodestone-mcp.$(RST)\n"

## ─── Run ───────────────────────────────────────────────────────────────────

run: ## Run the dev server (cargo run, debug profile, bind 0.0.0.0:$(PORT) via [server].bind in config).
	$(CARGO) run

run-galaxy: ## Run the lodestone-galaxy broker binary (cargo run --bin lodestone-galaxy).
	$(CARGO) run --bin lodestone-galaxy

## ─── Docker ────────────────────────────────────────────────────────────────

docker: docker-build docker-smoke ## Build the image then run the CI /health smoke test.

docker-build: ## Build the Docker image (tagged $(IMAGE)).
	$(DOCKER) build -t $(IMAGE) .

docker-run: ## Run the image in the foreground on host port $(PORT).
	$(DOCKER) run --rm --name $(CONTAINER) -p $(PORT):8000 $(IMAGE)

docker-stop: ## Stop and remove the dev container if it's running.
	-$(DOCKER) rm -f $(CONTAINER) >/dev/null 2>&1
	@true

docker-smoke: ## Replicate the CI smoke test — boot the image, poll /health, tear down.
	@$(DOCKER) rm -f $(CONTAINER) >/dev/null 2>&1 || true
	$(DOCKER) run -d --name $(CONTAINER) -p $(PORT):8000 $(IMAGE)
	@ok=; \
	for _ in $$(seq 1 $(HEALTH_TIMEOUT_SECS)); do \
	  if curl -fsS http://localhost:$(PORT)/health >/dev/null 2>&1; then ok=1; break; fi; \
	  sleep 1; \
	done; \
	echo; \
	$(DOCKER) logs $(CONTAINER) || true; \
	$(DOCKER) rm -f $(CONTAINER) >/dev/null 2>&1 || true; \
	if [ "$$ok" = "1" ]; then printf "$(BOLD)✔ /health responded.$(RST)\n"; else printf "$(BOLD)✗ /health never responded within $(HEALTH_TIMEOUT_SECS)s$(RST)\n"; exit 1; fi

## ─── Misc ──────────────────────────────────────────────────────────────────

clean: ## cargo clean.
	$(CARGO) clean

doc: ## Build the rustdoc tree (cargo doc --no-deps --document-private-items).
	$(CARGO) doc --no-deps --document-private-items

deps-check: ## Print the resolved toolchain + tool versions used by `check`.
	@printf "$(BOLD)Toolchain$(RST)\n"
	@$(CARGO) --version
	@rustc --version
	@$(CARGO) fmt --version 2>/dev/null || true
	@$(CARGO) clippy --version 2>/dev/null || true
	@printf "\n$(BOLD)Docker$(RST)\n"
	@$(DOCKER) --version 2>/dev/null || printf "  (docker not installed — docker targets unavailable)\n"

install-hooks: ## Install a git pre-commit hook that runs `make ci`.
	@mkdir -p .git/hooks
	@printf '#!/bin/sh\nset -e\nexec make ci\n' > .git/hooks/pre-commit
	@chmod +x .git/hooks/pre-commit
	@printf "$(BOLD)✔ .git/hooks/pre-commit installed (runs 'make ci' on every commit).$(RST)\n"
	@printf "  $(DIM)Bypass an individual commit with 'git commit --no-verify' — but golden rule 10 says don't.$(RST)\n"
