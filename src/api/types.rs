//! Request and response types for the HiveBox REST API.

use serde::{Deserialize, Serialize};

use crate::sandbox::network::NetworkMode;

/// Request body for `POST /api/v1/hiveboxes` (create a new sandbox).
#[derive(Debug, Deserialize)]
pub struct CreateSandboxRequest {
    /// Optional sandbox name. If not provided, a random ID is generated.
    #[serde(default)]
    pub name: Option<String>,

    /// Memory limit (e.g., "256m", "1g").
    #[serde(default = "default_memory")]
    pub memory: String,

    /// CPU limit as fraction of one core.
    #[serde(default = "default_cpus")]
    pub cpus: f64,

    /// Maximum number of processes.
    #[serde(default = "default_pids")]
    pub pids: u64,

    /// Network mode.
    #[serde(default)]
    pub network: NetworkMode,

    /// Timeout in seconds (sandbox is destroyed after this).
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_memory() -> String { "512m".to_string() }
fn default_cpus() -> f64 { 1.0 }
fn default_pids() -> u64 { 128 }
fn default_timeout() -> u64 { 3600 }

/// Response body for `POST /api/v1/hiveboxes`.
#[derive(Debug, Serialize)]
pub struct CreateSandboxResponse {
    pub id: String,
    pub status: String,
    pub image: String,
    pub created_at: String,
    pub network: NetworkInfoResponse,
    pub limits: LimitsResponse,
    pub expires_at: String,
}

/// Network information in API responses.
#[derive(Debug, Serialize)]
pub struct NetworkInfoResponse {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

/// Resource limits in API responses.
#[derive(Debug, Serialize)]
pub struct LimitsResponse {
    pub memory: String,
    pub cpus: f64,
    pub pids: u64,
}

/// Request body for `POST /api/v1/hiveboxes/:id/exec`.
#[derive(Debug, Deserialize)]
pub struct ExecRequest {
    /// Command to execute (passed to /bin/sh -c).
    pub command: String,

    /// Command timeout in seconds (0 = use sandbox default).
    #[serde(default)]
    pub timeout: u64,
}

/// Response body for synchronous exec.
#[derive(Debug, Serialize)]
pub struct ExecResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

/// Sandbox status response for `GET /api/v1/hiveboxes/:id`.
#[derive(Debug, Serialize)]
pub struct SandboxStatusResponse {
    pub id: String,
    pub status: String,
    pub image: String,
    pub created_at: String,
    pub uptime_seconds: u64,
    pub network: NetworkInfoResponse,
    pub limits: LimitsResponse,
}

/// Response for `GET /api/v1/hiveboxes` (list all sandboxes).
#[derive(Debug, Serialize)]
pub struct ListSandboxesResponse {
    pub sandboxes: Vec<SandboxSummary>,
    pub total: usize,
}

/// Summary of a sandbox in list responses.
#[derive(Debug, Serialize)]
pub struct SandboxSummary {
    pub id: String,
    pub status: String,
    pub image: String,
    pub uptime_seconds: u64,
    pub ttl_seconds: u64,
    pub memory: String,
    pub cpus: f64,
    pub commands_executed: u64,
    pub network: String,
    pub memory_usage_bytes: u64,
    pub pid_current: u64,
    pub cpu_usage_usec: u64,
}

/// Response for `DELETE /api/v1/hiveboxes/:id`.
#[derive(Debug, Serialize)]
pub struct DestroySandboxResponse {
    pub id: String,
    pub status: String,
}

/// Generic error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}
