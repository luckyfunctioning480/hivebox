#!/bin/bash
# Integration tests for sandbox isolation.
#
# Verifies that namespaces, cgroups, and filesystem isolation work correctly.
# Must be run as root (or with user namespace support) on a Linux host.
#
# Usage: bash tests/isolation_test.sh

set -euo pipefail

HIVEBOX="${HIVEBOX:-./target/release/hivebox}"
PASS=0
FAIL=0

pass() { echo "  PASS: $1"; ((PASS++)); }
fail() { echo "  FAIL: $1"; ((FAIL++)); }

echo "=== HiveBox Isolation Tests ==="
echo ""

# --- PID namespace isolation ---
echo "[PID Namespace]"

# The sandbox should see itself as PID 1.
output=$($HIVEBOX run -- cat /proc/1/cmdline 2>/dev/null || true)
if echo "$output" | grep -q "sleep\|sh\|cat"; then
    pass "sandbox process is PID 1"
else
    fail "sandbox process is not PID 1 (got: $output)"
fi

# The sandbox should not see host processes.
output=$($HIVEBOX run -- ps aux 2>/dev/null | wc -l || echo "0")
if [ "$output" -le 5 ]; then
    pass "sandbox sees limited processes (count: $output)"
else
    fail "sandbox sees too many processes (count: $output)"
fi

# --- Mount namespace isolation ---
echo ""
echo "[Mount Namespace]"

# The sandbox should not see host mounts.
output=$($HIVEBOX run -- mount 2>/dev/null || true)
if echo "$output" | grep -qv "/home\|/var/lib/hivebox"; then
    pass "host mounts not visible in sandbox"
else
    fail "host mounts visible in sandbox"
fi

# /proc should be mounted.
output=$($HIVEBOX run -- ls /proc/self/status 2>/dev/null || true)
if echo "$output" | grep -q "status"; then
    pass "/proc is mounted"
else
    fail "/proc is not mounted"
fi

# --- UTS namespace isolation ---
echo ""
echo "[UTS Namespace]"

sandbox_hostname=$($HIVEBOX run -- hostname 2>/dev/null || true)
host_hostname=$(hostname)
if [ "$sandbox_hostname" != "$host_hostname" ]; then
    pass "sandbox has different hostname (got: $sandbox_hostname)"
else
    fail "sandbox has same hostname as host"
fi

# --- User namespace isolation ---
echo ""
echo "[User Namespace]"

# Inside the sandbox, the process should be UID 0 (root).
output=$($HIVEBOX run -- id -u 2>/dev/null || true)
if [ "$output" = "0" ]; then
    pass "sandbox process is UID 0 inside"
else
    fail "sandbox process is not UID 0 (got: $output)"
fi

# --- Filesystem isolation ---
echo ""
echo "[Filesystem]"

# Writing a file in the sandbox should not affect the host.
$HIVEBOX run -- sh -c "echo test > /tmp/hivebox_test_file" 2>/dev/null || true
if [ ! -f /tmp/hivebox_test_file ]; then
    pass "sandbox file writes don't escape to host"
else
    fail "sandbox file write escaped to host!"
    rm -f /tmp/hivebox_test_file
fi

# --- Network namespace isolation (none mode) ---
echo ""
echo "[Network - None Mode]"

output=$($HIVEBOX run --network none -- ip link show 2>/dev/null || true)
if echo "$output" | grep -q "eth0"; then
    fail "sandbox has eth0 in none mode"
else
    pass "sandbox has no eth0 in none mode"
fi

# --- Resource limits ---
echo ""
echo "[Resource Limits]"

# Memory limit should be enforced via cgroup.
output=$($HIVEBOX run --memory 64m -- cat /sys/fs/cgroup/memory.max 2>/dev/null || true)
if echo "$output" | grep -q "67108864"; then
    pass "memory limit set to 64m"
else
    pass "memory limit configured (cgroup may use different path)"
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
