# CLI Reference

## Global Flags

| Flag | Description |
|------|-------------|
| `-v, --verbose` | Enable debug logging |
| `--version` | Show version |
| `--help` | Show help |

## Commands

### `hivebox run`

One-shot execution: creates a sandbox, runs a command, and destroys the sandbox.

```bash
hivebox run [OPTIONS] -- <COMMAND>...
```

**Options**:
| Flag | Default | Description |
|------|---------|-------------|
| `--memory <SIZE>` | `256m` | Memory limit (e.g., `256m`, `1g`) |
| `--cpus <FLOAT>` | `1.0` | CPU limit (fraction of one core) |
| `--pids <NUM>` | `64` | Max number of processes |
| `--network <MODE>` | `none` | Network mode: `none`, `isolated`, `shared:group` |

**Examples**:
```bash
# Run a simple command
hivebox run -- echo "hello from the sandbox"

# Run with more memory
hivebox run --memory 512m -- python3 -c "print('hello')"

# Run with internet access
hivebox run --network isolated -- wget -qO- https://example.com

# Run with shared networking between sandboxes
hivebox run --network shared:mygroup -- hostname -I
```

---

### `hivebox create`

Creates a persistent sandbox that stays alive until explicitly destroyed or timeout.

```bash
hivebox create [OPTIONS]
```

**Options**:
| Flag | Default | Description |
|------|---------|-------------|
| `--name <NAME>` | random | Assign a name to the sandbox |
| `--memory <SIZE>` | `256m` | Memory limit |
| `--cpus <FLOAT>` | `1.0` | CPU limit |
| `--pids <NUM>` | `64` | Max processes |
| `--network <MODE>` | `none` | Network mode |
| `--timeout <SECS>` | `3600` | Auto-destroy timeout (max 86400s / 24h) |

**Output**: Prints the sandbox ID.

**Examples**:
```bash
# Create with auto-generated ID
hivebox create
# Output: hb-7f3a9b

# Create with a name and custom resources
hivebox create --name myagent --memory 1g --timeout 7200

# Create with internet access
hivebox create --name webworker --network isolated
```

---

### `hivebox exec`

Executes a command in an existing sandbox.

```bash
hivebox exec <SANDBOX> -- <COMMAND>...
```

**Arguments**:
- `SANDBOX`: sandbox name or ID
- `COMMAND`: command to execute

**Examples**:
```bash
# Install a package
hivebox exec myagent -- pip install requests

# Run a script
hivebox exec myagent -- python3 /script.py

# Check disk usage
hivebox exec myagent -- df -h
```

---

### `hivebox destroy`

Destroys a sandbox and cleans up all resources.

```bash
hivebox destroy <SANDBOX>
```

**Examples**:
```bash
hivebox destroy myagent
hivebox destroy hb-7f3a9b
```

---

### `hivebox list` (alias: `ls`)

Lists all active sandboxes with status and resource information.

```bash
hivebox list
```

**Output**:
```
ID           STATUS     UPTIME     TTL        CMDS     NETWORK
------------------------------------------------------------
myagent      running    15m30s     44m30s     3        none
hb-7f3a9b    running    2h10m      49m50s     12       isolated
```

Columns:
| Column | Description |
|--------|-------------|
| ID | Sandbox name or generated ID |
| STATUS | Current state (running, stopped) |
| UPTIME | Time since creation |
| TTL | Time remaining before auto-destroy |
| CMDS | Number of commands executed |
| NETWORK | Network mode |

---

### `hivebox daemon`

Starts the HiveBox API server.

```bash
hivebox daemon [OPTIONS]
```

**Options**:
| Flag | Default | Description |
|------|---------|-------------|
| `--port <PORT>` | `7070` | TCP port to listen on |
| `--api-key <KEY>` | none | API key for authentication |

The API key can also be set via the `HIVEBOX_API_KEY` environment variable.

**Examples**:
```bash
# Start with authentication
hivebox daemon --port 7070 --api-key mysecretkey

# Start via environment variable
HIVEBOX_API_KEY=mysecretkey hivebox daemon

# Start without authentication (not recommended)
hivebox daemon
```

### `hivebox mcp`

Runs as an MCP (Model Context Protocol) server over stdin/stdout for a specific sandbox. Designed to be spawned by MCP-compatible AI clients (OpenCode, Claude Code, etc.).

```bash
hivebox mcp --sandbox <SANDBOX> [OPTIONS]
```

**Options**:
| Flag | Default | Description |
|------|---------|-------------|
| `--sandbox <NAME>` | required | Sandbox name or ID to expose via MCP |
| `--api-url <URL>` | `http://localhost:7070` | HiveBox daemon API URL |
| `--api-key <KEY>` | none | API key for authentication |

The API key can also be set via `HIVEBOX_API_KEY` and the API URL via `HIVEBOX_API_URL`.

**Examples**:
```bash
# Start MCP server for a sandbox
hivebox mcp --sandbox myagent --api-url http://localhost:7070

# With authentication
hivebox mcp --sandbox myagent --api-key mysecretkey
```

**MCP client configuration** (e.g., in OpenCode or Claude Code):
```json
{
  "mcpServers": {
    "sandbox": {
      "command": "hivebox",
      "args": ["mcp", "--sandbox", "myagent", "--api-url", "http://localhost:7070"],
      "env": { "HIVEBOX_API_KEY": "mysecretkey" }
    }
  }
}
```

---

## Typical Workflow

```bash
# 1. Start the daemon (in production)
hivebox daemon --api-key secret &

# 2. Create a sandbox
hivebox create --name worker --memory 512m

# 3. Set up the environment
hivebox exec worker -- pip install numpy pandas

# 4. Run your workload
hivebox exec worker -- python3 -c "import numpy; print(numpy.__version__)"

# 5. Clean up
hivebox destroy worker
```
