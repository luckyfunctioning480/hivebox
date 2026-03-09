#!/usr/bin/env python3
"""End-to-end API tests for HiveBox.

Tests the REST API by making real HTTP requests to a running HiveBox daemon.
The daemon must be started before running these tests:

    hivebox daemon --port 7070 --api-key testkey &
    python3 tests/api_test.py

Environment variables:
    API_URL:  Base URL of the HiveBox API (default: http://localhost:7070)
    API_KEY:  API key for authentication (default: testkey)
"""

import json
import os
import sys
import time
import urllib.request
import urllib.error

API_URL = os.environ.get("API_URL", "http://localhost:7070")
API_KEY = os.environ.get("API_KEY", "testkey")

passed = 0
failed = 0


def api(method, path, body=None):
    """Make an API request and return (status_code, response_body)."""
    url = f"{API_URL}{path}"
    data = json.dumps(body).encode() if body else None
    headers = {
        "Authorization": f"Bearer {API_KEY}",
        "Content-Type": "application/json",
    }
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req) as resp:
            return resp.status, json.loads(resp.read())
    except urllib.error.HTTPError as e:
        body = e.read().decode() if e.fp else ""
        try:
            body = json.loads(body)
        except (json.JSONDecodeError, ValueError):
            pass
        return e.code, body


def test(name, condition):
    """Record a test result."""
    global passed, failed
    if condition:
        print(f"  PASS: {name}")
        passed += 1
    else:
        print(f"  FAIL: {name}")
        failed += 1


# ─── Health check ───

print("[Health Check]")
try:
    req = urllib.request.Request(f"{API_URL}/healthz")
    with urllib.request.urlopen(req) as resp:
        test("GET /healthz returns 200", resp.status == 200)
        test("GET /healthz returns 'ok'", resp.read().decode().strip() == "ok")
except Exception as e:
    print(f"  FAIL: Cannot reach API at {API_URL}: {e}")
    sys.exit(1)

# ─── Authentication ───

print("\n[Authentication]")

# Request without auth header should fail.
try:
    req = urllib.request.Request(f"{API_URL}/api/v1/sandboxes")
    urllib.request.urlopen(req)
    test("unauthenticated request rejected", False)
except urllib.error.HTTPError as e:
    test("unauthenticated request rejected", e.code == 401)

# Request with wrong key should fail.
try:
    req = urllib.request.Request(f"{API_URL}/api/v1/sandboxes")
    req.add_header("Authorization", "Bearer wrongkey")
    urllib.request.urlopen(req)
    test("wrong API key rejected", False)
except urllib.error.HTTPError as e:
    test("wrong API key rejected", e.code == 401)

# ─── Sandbox lifecycle ───

print("\n[Sandbox Lifecycle]")

# Create a sandbox.
status, body = api("POST", "/api/v1/sandboxes", {
    "name": "test-sandbox",
    "image": "base",
    "timeout": 120,
})
test("POST /api/v1/sandboxes returns 201 or 200", status in (200, 201))
test("response contains sandbox ID", "id" in body)
sandbox_id = body.get("id", "")
print(f"    Created sandbox: {sandbox_id}")

# List sandboxes.
status, body = api("GET", "/api/v1/sandboxes")
test("GET /api/v1/sandboxes returns 200", status == 200)
test("response contains sandboxes list", "sandboxes" in body)
test("list contains our sandbox", any(s["id"] == sandbox_id for s in body.get("sandboxes", [])))

# Get sandbox details.
status, body = api("GET", f"/api/v1/sandboxes/{sandbox_id}")
test("GET /api/v1/sandboxes/:id returns 200", status == 200)
test("response contains sandbox ID", body.get("id") == sandbox_id)
test("response contains status", "status" in body)
test("response contains uptime", "uptime_seconds" in body)

# Execute a command.
status, body = api("POST", f"/api/v1/sandboxes/{sandbox_id}/exec", {
    "command": "echo hello-from-hivebox",
})
test("POST .../exec returns 200", status == 200)
test("stdout contains expected output", "hello-from-hivebox" in body.get("stdout", ""))
test("exit code is 0", body.get("exit_code") == 0)
test("response contains duration_ms", "duration_ms" in body)

# Execute a failing command.
status, body = api("POST", f"/api/v1/sandboxes/{sandbox_id}/exec", {
    "command": "exit 42",
})
test("failing command returns 200", status == 200)
test("failing command has non-zero exit code", body.get("exit_code") == 42)

# ─── File operations ───

print("\n[File Operations]")

# Upload a file.
status, body = api("PUT", f"/api/v1/sandboxes/{sandbox_id}/files?path=/tmp/test.txt", {
    "content": "aGVsbG8gd29ybGQ=",  # base64 of "hello world"
})
test("PUT .../files returns 200", status == 200)

# Download the file.
status, body = api("GET", f"/api/v1/sandboxes/{sandbox_id}/files?path=/tmp/test.txt")
test("GET .../files returns 200", status == 200)

# ─── Sandbox destruction ───

print("\n[Sandbox Destruction]")

status, body = api("DELETE", f"/api/v1/sandboxes/{sandbox_id}")
test("DELETE /api/v1/sandboxes/:id returns 200", status == 200)
test("response confirms destruction", body.get("status") in ("destroyed", "deleted"))

# Verify it's gone.
status, body = api("GET", f"/api/v1/sandboxes/{sandbox_id}")
test("destroyed sandbox returns 404", status == 404)

# ─── Edge cases ───

print("\n[Edge Cases]")

# Get non-existent sandbox.
status, body = api("GET", "/api/v1/sandboxes/nonexistent")
test("non-existent sandbox returns 404", status == 404)

# Exec on non-existent sandbox.
status, body = api("POST", "/api/v1/sandboxes/nonexistent/exec", {"command": "echo hi"})
test("exec on non-existent sandbox returns 404", status == 404)

# Create with duplicate name.
api("POST", "/api/v1/sandboxes", {"name": "dup-test"})
status, body = api("POST", "/api/v1/sandboxes", {"name": "dup-test"})
test("duplicate name returns 409 or 400", status in (400, 409))
api("DELETE", "/api/v1/sandboxes/dup-test")  # cleanup

# ─── Summary ───

print(f"\n=== Results ===")
print(f"Passed: {passed}")
print(f"Failed: {failed}")

if failed > 0:
    print("\nSOME TESTS FAILED")
    sys.exit(1)
else:
    print("\nALL TESTS PASSED")
    sys.exit(0)
