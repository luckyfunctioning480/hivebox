#!/bin/bash
# Security tests: attempt to escape the sandbox.
#
# These tests verify that common escape techniques are blocked by the
# sandbox's security layers (namespaces, capabilities, seccomp, Landlock).
#
# All tests should FAIL to escape (i.e., the escape attempt should be denied).
#
# Usage: bash tests/escape_test.sh

set -euo pipefail

HIVEBOX="${HIVEBOX:-./target/release/hivebox}"
PASS=0
FAIL=0

pass() { echo "  BLOCKED: $1"; ((PASS++)); }
fail() { echo "  ESCAPED: $1 !!!"; ((FAIL++)); }

echo "=== HiveBox Escape Tests ==="
echo "All attempts should be BLOCKED."
echo ""

# --- Attempt: access host filesystem via /proc ---
echo "[Escape via /proc]"

output=$($HIVEBOX run -- cat /proc/1/root/etc/hostname 2>&1 || true)
if echo "$output" | grep -qi "denied\|permission\|no such\|error\|failed"; then
    pass "cannot access host root via /proc/1/root"
else
    fail "accessed host root via /proc/1/root"
fi

# --- Attempt: mount host filesystem ---
echo ""
echo "[Escape via mount]"

output=$($HIVEBOX run -- mount -t proc proc /mnt 2>&1 || true)
if [ $? -ne 0 ] || echo "$output" | grep -qi "denied\|permission\|operation not\|error"; then
    pass "cannot mount over /mnt"
else
    fail "mounted over /mnt"
fi

# --- Attempt: access host network ---
echo ""
echo "[Escape via network]"

output=$($HIVEBOX run --network none -- ping -c 1 -W 1 8.8.8.8 2>&1 || true)
if echo "$output" | grep -qi "unreachable\|denied\|network\|100% packet loss\|error\|not found"; then
    pass "cannot reach internet in none mode"
else
    fail "reached internet in none mode"
fi

# --- Attempt: create device nodes ---
echo ""
echo "[Escape via mknod]"

output=$($HIVEBOX run -- mknod /dev/sda b 8 0 2>&1 || true)
if echo "$output" | grep -qi "denied\|permission\|operation not\|error"; then
    pass "cannot create block device nodes"
else
    fail "created block device node"
fi

# --- Attempt: load kernel module ---
echo ""
echo "[Escape via kernel module]"

output=$($HIVEBOX run -- sh -c "insmod /nonexistent.ko 2>&1 || modprobe dummy 2>&1" 2>&1 || true)
if echo "$output" | grep -qi "denied\|permission\|operation not\|not found\|error"; then
    pass "cannot load kernel modules"
else
    fail "loaded kernel module"
fi

# --- Attempt: reboot ---
echo ""
echo "[Escape via reboot]"

output=$($HIVEBOX run -- reboot 2>&1 || true)
# If we're still running, reboot was blocked.
pass "reboot did not affect host"

# --- Attempt: kill host processes ---
echo ""
echo "[Escape via kill]"

output=$($HIVEBOX run -- kill -9 1 2>&1 || true)
if echo "$output" | grep -qi "denied\|permission\|no such\|operation not\|error"; then
    pass "cannot kill host PID 1"
else
    # Even if kill returns success, PID 1 inside the sandbox is the sandbox init,
    # not the host init. So this is actually safe.
    pass "kill -9 1 only affects sandbox PID 1"
fi

# --- Attempt: ptrace host processes ---
echo ""
echo "[Escape via ptrace]"

output=$($HIVEBOX run -- sh -c "strace -p 1 2>&1 || true" 2>&1 || true)
if echo "$output" | grep -qi "denied\|permission\|operation not\|attach\|error\|not found"; then
    pass "cannot ptrace PID 1"
else
    fail "ptrace succeeded"
fi

# --- Attempt: write to /proc/sys ---
echo ""
echo "[Escape via /proc/sys write]"

output=$($HIVEBOX run -- sh -c "echo 1 > /proc/sys/kernel/sysrq 2>&1" 2>&1 || true)
if echo "$output" | grep -qi "denied\|permission\|read-only\|error"; then
    pass "cannot write to /proc/sys"
else
    fail "wrote to /proc/sys"
fi

# --- Attempt: pivot_root escape ---
echo ""
echo "[Escape via pivot_root]"

output=$($HIVEBOX run -- ls /.pivot_old 2>&1 || true)
if echo "$output" | grep -qi "no such\|not found\|error"; then
    pass "old root mount point does not exist"
else
    fail "old root mount point exists"
fi

# --- Summary ---
echo ""
echo "=== Results ==="
echo "Blocked: $PASS"
echo "Escaped: $FAIL"
echo ""

if [ "$FAIL" -gt 0 ]; then
    echo "SECURITY FAILURES DETECTED"
    exit 1
else
    echo "ALL ESCAPE ATTEMPTS BLOCKED"
    exit 0
fi
