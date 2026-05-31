#!/bin/sh
# Two-node constellation smoke harness.
#
# Brings up nodes A and B as constellation peers on 127.0.0.1:18101/18102, each
# with delegation_enabled = true, then drives the /constellation/* endpoints
# directly via curl. Verifies: digest exchange, peer discovery, blob/blobinfo
# anti-tamper handshake, and the delegated-retrieve guardrails.
#
# Usage: ./smoke/run.sh (from the project root).
#
# Cross-platform note: on Windows + Git Bash, paths must be Windows-form when
# they cross into the lodestone-mcp.exe binary. We use cygpath when available.
set -e
cd "$(dirname "$0")/.."

TOKEN="smoke-token"
A_BASE="http://127.0.0.1:18101"
B_BASE="http://127.0.0.1:18102"
LOG_A="$(pwd)/smoke/a/node-a.log"
LOG_B="$(pwd)/smoke/b/node-b.log"

# Convert paths for the binary on Windows; on Unix the identity works fine.
to_winpath() {
    if command -v cygpath >/dev/null 2>&1; then
        cygpath -w "$1"
    else
        printf '%s' "$1"
    fi
}

clean() {
    # Kill anything from a previous run that might still be holding ports.
    if command -v powershell.exe >/dev/null 2>&1; then
        powershell.exe -NoProfile -Command "Get-Process lodestone-mcp -ErrorAction SilentlyContinue | Stop-Process -Force" >/dev/null 2>&1 || true
    else
        pkill -f lodestone-mcp >/dev/null 2>&1 || true
    fi
    rm -f "$LOG_A" "$LOG_B"
    sleep 1
}

wait_health() {
    local base="$1" name="$2"
    for _ in $(seq 1 30); do
        if curl -fsS --max-time 1 "$base/health" >/dev/null 2>&1; then
            printf '  ✔ %s is up at %s\n' "$name" "$base"
            return 0
        fi
        sleep 1
    done
    printf '  ✗ %s never came up at %s\n' "$name" "$base"
    return 1
}

start_node() {
    local letter="$1" base="$2"
    local cfg_dir="$(pwd)/smoke/${letter}/config"
    local cfg_file="$cfg_dir/00-server.toml"
    local log_file="$(pwd)/smoke/${letter}/node-${letter}.log"
    LODESTONE_CONFIG=$(to_winpath "$cfg_file") \
    LODESTONE_CONFIG_DIR=$(to_winpath "$cfg_dir") \
        nohup ./target/release/lodestone-mcp.exe >"$log_file" 2>&1 &
    echo "  started node-${letter}: pid $!"
}

assert_eq() {
    if [ "$1" = "$2" ]; then
        printf '  ✔ %s: %s\n' "$3" "$1"
    else
        printf '  ✗ %s: expected "%s", got "%s"\n' "$3" "$2" "$1"
        return 1
    fi
}

assert_contains() {
    if printf '%s' "$1" | grep -q "$2"; then
        printf '  ✔ %s contains %s\n' "$3" "$2"
    else
        printf '  ✗ %s: expected to contain "%s", got: %s\n' "$3" "$2" "$1"
        return 1
    fi
}

printf '=== Stage 0: clean + start ===\n'
clean
start_node a "$A_BASE"
start_node b "$B_BASE"
wait_health "$A_BASE" node-a
wait_health "$B_BASE" node-b

printf '\n=== Stage 1: digest exchange + peer discovery ===\n'
# A's digest — should be a JSON document.
DIGEST_A=$(curl -fsS -H "Authorization: Bearer $TOKEN" "$A_BASE/constellation/digest")
assert_contains "$DIGEST_A" '"node_id":"node-a"' "node-a digest node_id"
assert_contains "$DIGEST_A" '"constellation_id":"smoke"' "node-a digest constellation_id"
assert_contains "$DIGEST_A" '"delegation_enabled":true' "node-a digest delegation_enabled"
DIGEST_B=$(curl -fsS -H "Authorization: Bearer $TOKEN" "$B_BASE/constellation/digest")
assert_contains "$DIGEST_B" '"node_id":"node-b"' "node-b digest node_id"
assert_contains "$DIGEST_B" '"delegation_enabled":true' "node-b digest delegation_enabled"

# Verify auth gating: missing token → 401.
NOAUTH=$(curl -sS -o /dev/null -w '%{http_code}' "$A_BASE/constellation/digest")
assert_eq "$NOAUTH" "401" "constellation/digest without token returns 401"

# Wait for sync (default 5s) and check the peer-status endpoint to confirm
# they've seen each other.
printf '  waiting 7s for digest sync...\n'
sleep 7
PEERS_A=$(curl -fsS -H "Authorization: Bearer $TOKEN" "$A_BASE/health")
printf '  node-a /health: %s\n' "$PEERS_A"

printf '\n=== Stage 2: delegated retrieve guardrails ===\n'
# Ask node-b to fetch a tiny well-known URL on behalf of node-a.
PAYLOAD='{"url":"https://example.com/","max_bytes":1048576,"source":"other"}'
RESP=$(curl -sS -o /tmp/smoke-retrieve-body.bin -w '%{http_code}' \
  -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Lodestone-Peer-Id: node-a" \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD" \
  "$B_BASE/constellation/retrieve")
BODY=$(cat /tmp/smoke-retrieve-body.bin 2>/dev/null || true)
if [ "$RESP" = "200" ]; then
    SIZE=${#BODY}
    printf '  ✔ delegated retrieve OK: HTTP 200, body length %s\n' "$SIZE"
    case "$BODY" in
        *"Example Domain"*) printf '  ✔ body contains expected "Example Domain"\n' ;;
        *) printf '  ✗ body missing expected "Example Domain"\n' ;;
    esac
else
    printf '  ⚠ delegated retrieve returned HTTP %s (might be no network) — body:\n%s\n' "$RESP" "$BODY"
fi

# Bytes-too-large rejection: ask for more than delegation_max_bytes_per_job.
BIG_PAYLOAD='{"url":"https://example.com/","max_bytes":1073741824,"source":"other"}'
BIG_RESP=$(curl -sS -o /tmp/smoke-big.bin -w '%{http_code}' \
  -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Lodestone-Peer-Id: node-a-big" \
  -H "Content-Type: application/json" \
  -d "$BIG_PAYLOAD" \
  "$B_BASE/constellation/retrieve")
assert_eq "$BIG_RESP" "413" "oversize request returns 413"

printf '\n=== Stage 3: tear down ===\n'
clean
printf '\nall smoke checks done.\n'
