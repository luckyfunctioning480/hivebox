#!/bin/bash
# Stress tests for HiveBox.
#
# Tests sandbox creation/destruction under load to verify resource cleanup
# and stability. Requires the HiveBox daemon to be running.
#
# Usage: bash tests/stress_test.sh [count]
#   count: number of sandboxes to create (default: 50)

set -euo pipefail

HIVEBOX="${HIVEBOX:-./target/release/hivebox}"
API_URL="${API_URL:-http://localhost:7070}"
API_KEY="${API_KEY:-testkey}"
COUNT="${1:-50}"
PASS=0
FAIL=0

pass() { echo "  PASS: $1"; ((PASS++)); }
fail() { echo "  FAIL: $1"; ((FAIL++)); }

auth_header="Authorization: Bearer $API_KEY"

echo "=== HiveBox Stress Tests ==="
echo "Creating $COUNT sandboxes..."
echo ""

# --- Test: rapid sequential creation and destruction ---
echo "[Sequential Create/Destroy]"

for i in $(seq 1 "$COUNT"); do
    id=$(curl -sf -X POST "$API_URL/api/v1/sandboxes" \
        -H "$auth_header" \
        -H "Content-Type: application/json" \
        -d '{"image":"base","timeout":60}' | \
        python3 -c "import sys,json; print(json.load(sys.stdin)['id'])" 2>/dev/null)

    if [ -z "$id" ]; then
        fail "failed to create sandbox $i"
        continue
    fi

    # Execute a simple command.
    result=$(curl -sf -X POST "$API_URL/api/v1/sandboxes/$id/exec" \
        -H "$auth_header" \
        -H "Content-Type: application/json" \
        -d "{\"command\":\"echo $i\"}" 2>/dev/null)

    # Destroy the sandbox.
    curl -sf -X DELETE "$API_URL/api/v1/sandboxes/$id" \
        -H "$auth_header" >/dev/null 2>&1

    if [ $((i % 10)) -eq 0 ]; then
        echo "  ... completed $i/$COUNT"
    fi
done

pass "sequential create/destroy ($COUNT sandboxes)"

# --- Test: verify no leaked sandboxes ---
echo ""
echo "[Cleanup Verification]"

remaining=$(curl -sf "$API_URL/api/v1/sandboxes" \
    -H "$auth_header" | \
    python3 -c "import sys,json; print(json.load(sys.stdin)['total'])" 2>/dev/null)

if [ "$remaining" = "0" ]; then
    pass "no leaked sandboxes (total: 0)"
else
    fail "leaked sandboxes remain (total: $remaining)"
fi

# --- Test: parallel creation ---
echo ""
echo "[Parallel Create]"

pids=()
parallel_count=10

for i in $(seq 1 $parallel_count); do
    curl -sf -X POST "$API_URL/api/v1/sandboxes" \
        -H "$auth_header" \
        -H "Content-Type: application/json" \
        -d "{\"image\":\"base\",\"timeout\":30,\"name\":\"stress-$i\"}" \
        >/dev/null 2>&1 &
    pids+=($!)
done

# Wait for all parallel creates.
failures=0
for pid in "${pids[@]}"; do
    if ! wait "$pid"; then
        ((failures++))
    fi
done

if [ "$failures" -eq 0 ]; then
    pass "parallel creation ($parallel_count simultaneous)"
else
    fail "parallel creation had $failures failures"
fi

# Clean up parallel sandboxes.
for i in $(seq 1 $parallel_count); do
    curl -sf -X DELETE "$API_URL/api/v1/sandboxes/stress-$i" \
        -H "$auth_header" >/dev/null 2>&1 || true
done

# --- Test: memory under load ---
echo ""
echo "[Memory Stability]"

# Check that daemon memory usage hasn't exploded.
# This is a rough heuristic — daemon should stay under 100 MB RSS.
daemon_pid=$(pgrep -f "hivebox daemon" 2>/dev/null | head -1 || true)
if [ -n "$daemon_pid" ]; then
    rss_kb=$(cat /proc/$daemon_pid/status 2>/dev/null | grep VmRSS | awk '{print $2}' || echo "0")
    rss_mb=$((rss_kb / 1024))
    if [ "$rss_mb" -lt 100 ]; then
        pass "daemon memory OK (${rss_mb} MB RSS)"
    else
        fail "daemon memory high (${rss_mb} MB RSS)"
    fi
else
    echo "  SKIP: cannot find daemon process"
fi

# --- Summary ---
echo ""
echo "=== Results ==="
echo "Passed: $PASS"
echo "Failed: $FAIL"
echo ""

if [ "$FAIL" -gt 0 ]; then
    echo "SOME TESTS FAILED"
    exit 1
else
    echo "ALL TESTS PASSED"
    exit 0
fi
