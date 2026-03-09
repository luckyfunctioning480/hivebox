# REST API Reference

Base URL: `http://<host>:7070`

## Authentication

All `/api/` endpoints require a Bearer token in the `Authorization` header:

```
Authorization: Bearer <your-api-key>
```

The `/healthz` and `/dashboard` endpoints do not require authentication.

Set the API key when starting the daemon:
```bash
hivebox daemon --api-key your-secret-key
# or
HIVEBOX_API_KEY=your-secret-key hivebox daemon
```

## Web Dashboard

```
GET /dashboard
```

A built-in management UI served directly by the HiveBox daemon. Provides sandbox creation, listing, terminal access, and destruction — all from the browser. Login with your API key.

No authentication required to load the page (authentication happens client-side via API calls).

## Endpoints

### Health Check

```
GET /healthz
```

Returns `ok` if the server is running. No authentication required.

**Response**: `200 OK` with body `ok`

---

### Create Sandbox

```
POST /api/v1/hiveboxes
```

Creates a new persistent sandbox.

**Request body**:
```json
{
  "name": "my-sandbox",
  "memory": "512m",
  "cpus": 2.0,
  "pids": 128,
  "network": "isolated",
  "timeout": 3600
}
```

All fields are optional. Defaults:
| Field | Default | Description |
|-------|---------|-------------|
| `name` | random ID | Unique sandbox name |
| `memory` | `"512m"` | Memory limit |
| `cpus` | `1.0` | CPU fraction |
| `pids` | `128` | Max processes |
| `network` | `"none"` | Network mode: `none`, `isolated`, `shared` |
| `timeout` | `3600` | Auto-destroy timeout in seconds (max 86400) |

**Response** (`201 Created`):
```json
{
  "id": "my-sandbox",
  "status": "running",
  "image": "base",
  "created_at": "2025-01-15T10:30:00Z",
  "network": {
    "mode": "isolated",
    "ip": "10.10.0.2"
  },
  "limits": {
    "memory": "512m",
    "cpus": 2.0,
    "pids": 128
  },
  "expires_at": "2025-01-15T11:30:00Z"
}
```

---

### List Sandboxes

```
GET /api/v1/hiveboxes
```

Returns all active sandboxes.

**Response** (`200 OK`):
```json
{
  "sandboxes": [
    {
      "id": "my-sandbox",
      "status": "running",
      "image": "base",
      "uptime_seconds": 120,
      "ttl_seconds": 3480,
      "memory": "512m",
      "cpus": 2.0,
      "commands_executed": 5,
      "network": "isolated"
    }
  ],
  "total": 1
}
```

---

### Get Sandbox Details

```
GET /api/v1/hiveboxes/:id
```

Returns detailed information about a sandbox.

**Response** (`200 OK`):
```json
{
  "id": "my-sandbox",
  "status": "running",
  "image": "base",
  "created_at": "2025-01-15T10:30:00Z",
  "uptime_seconds": 120,
  "network": {
    "mode": "isolated",
    "ip": "10.10.0.2"
  },
  "limits": {
    "memory": "512m",
    "cpus": 2.0,
    "pids": 128
  }
}
```

**Errors**: `404 Not Found` if sandbox doesn't exist.

---

### Execute Command

```
POST /api/v1/hiveboxes/:id/exec
```

Executes a command inside a running sandbox.

**Request body**:
```json
{
  "command": "python3 -c 'print(1+1)'",
  "timeout": 30
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `command` | required | Shell command to execute |
| `timeout` | `0` | Command timeout in seconds |

**Response** (`200 OK`):
```json
{
  "exit_code": 0,
  "stdout": "2\n",
  "stderr": "",
  "duration_ms": 45
}
```

**Errors**: `404 Not Found` if sandbox doesn't exist.

---

### Upload File

```
PUT /api/v1/hiveboxes/:id/files?path=/path/in/sandbox
```

Uploads a file into the sandbox filesystem.

**Query parameters**:
- `path`: absolute path where the file should be created inside the sandbox

**Request body**: raw file content (binary)

**Response**: `200 OK`

---

### Download File

```
GET /api/v1/hiveboxes/:id/files?path=/path/in/sandbox
```

Downloads a file from the sandbox filesystem.

**Query parameters**:
- `path`: absolute path to the file inside the sandbox

**Response** (`200 OK`): raw file content

**Errors**: `404 Not Found` if file doesn't exist.

---

### Destroy Sandbox

```
DELETE /api/v1/hiveboxes/:id
```

Destroys a sandbox and cleans up all resources.

**Response** (`200 OK`):
```json
{
  "id": "my-sandbox",
  "status": "destroyed"
}
```

**Errors**: `404 Not Found` if sandbox doesn't exist.

---

### Analytics

```
GET /api/v1/analytics
```

Returns metrics history for all sandboxes and host resource usage.

**Query parameters**:
- `range` (optional): time range in seconds (e.g., `300` = 5 min, `3600` = 1 hour). Default: all available history.

**Response** (`200 OK`):
```json
{
  "samples": [
    {
      "timestamp": 1705312200,
      "sandbox_count": 2,
      "sandboxes": [],
      "host_memory_total": 8388608000,
      "host_memory_used": 4194304000,
      "host_cpu_percent": 12.5
    }
  ],
  "interval_secs": 10
}
```

---

## MCP Bridge

HiveBox exposes sandbox operations as MCP (Model Context Protocol) tools, allowing any MCP-compatible AI client (OpenCode, Claude Code, etc.) to operate inside sandboxes.

### MCP over HTTP (recommended)

```
POST /api/v1/hiveboxes/:id/mcp
```

The daemon serves MCP directly over HTTP. No local binary needed — just a URL.

Accepts JSON-RPC 2.0 requests in the body, returns JSON-RPC responses. This is the **Streamable HTTP** MCP transport.

Configure in your AI client:
```json
{
  "mcpServers": {
    "sandbox": {
      "url": "http://your-server:7070/api/v1/hiveboxes/my-sandbox/mcp",
      "headers": { "Authorization": "Bearer your-secret-key" }
    }
  }
}
```

### MCP over stdio (local)

For local setups, you can also use the `hivebox mcp` CLI command which communicates over stdin/stdout:

```json
{
  "mcpServers": {
    "sandbox": {
      "command": "hivebox",
      "args": ["mcp", "--sandbox", "my-sandbox", "--api-url", "http://localhost:7070"],
      "env": { "HIVEBOX_API_KEY": "your-secret-key" }
    }
  }
}
```

### Available tools

Both transports expose 15 tools:

| Tool | Description |
|------|-------------|
| `exec` | Execute a shell command |
| `read_file` | Read file contents (with optional head/tail) |
| `read_multiple_files` | Read multiple files at once |
| `write_file` | Create or overwrite a file |
| `edit_file` | Targeted text replacement |
| `list_directory` | List directory contents |
| `directory_tree` | Recursive tree view |
| `search_files` | Search for text patterns (grep) |
| `get_file_info` | File metadata (stat) |
| `create_directory` | Create directories |
| `move_file` | Move or rename files |
| `upload_file` | Upload file (supports base64 for binary) |
| `download_file` | Download file (supports base64 for binary) |
| `read_media_file` | Read images/media as base64 with MIME type |
| `list_directory_with_sizes` | List directory with human-readable sizes |

---

## Error Responses

All errors return JSON:

```json
{
  "error": "sandbox 'xyz' not found"
}
```

Common HTTP status codes:
| Code | Meaning |
|------|---------|
| 200 | Success |
| 201 | Created |
| 400 | Bad request (invalid input) |
| 401 | Unauthorized (missing/wrong API key) |
| 404 | Not found (sandbox/file doesn't exist) |
| 500 | Internal server error |
