//! REST API server for HiveBox.
//!
//! Provides HTTP endpoints for managing sandboxes remotely. The API follows
//! REST conventions with JSON request/response bodies.
//!
//! # Endpoints
//!
//! | Method | Path                          | Description              |
//! |--------|-------------------------------|--------------------------|
//! | POST   | /api/v1/hiveboxes             | Create a new sandbox     |
//! | GET    | /api/v1/hiveboxes             | List all sandboxes       |
//! | GET    | /api/v1/hiveboxes/:id         | Get sandbox details      |
//! | POST   | /api/v1/hiveboxes/:id/exec    | Execute command          |
//! | POST   | /api/v1/hiveboxes/:id/mcp     | MCP over HTTP            |
//! | PUT    | /api/v1/hiveboxes/:id/files   | Upload file              |
//! | GET    | /api/v1/hiveboxes/:id/files   | Download file            |
//! | DELETE | /api/v1/hiveboxes/:id         | Destroy sandbox          |
//!
//! # Server configuration
//!
//! The API server listens on TCP port 7070 (configurable).

pub mod dashboard;
pub mod handlers;
pub mod mcp;
pub mod types;

use std::sync::Arc;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use axum::routing::{delete, get, post, put};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::sandbox::manager::SandboxManager;
use handlers::{AppState, SharedState};

/// Default TCP port for the REST API.
pub const DEFAULT_PORT: u16 = 7070;

/// Builds the axum router with all API routes.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/hiveboxes", post(handlers::create_sandbox))
        .route("/api/v1/hiveboxes", get(handlers::list_sandboxes))
        .route("/api/v1/hiveboxes/{id}", get(handlers::get_sandbox))
        .route("/api/v1/hiveboxes/{id}", delete(handlers::destroy_sandbox))
        .route("/api/v1/hiveboxes/{id}/exec", post(handlers::exec_in_sandbox))
        .route("/api/v1/hiveboxes/{id}/mcp", post(mcp::mcp_handler))
        .route("/api/v1/hiveboxes/{id}/files", put(handlers::upload_file))
        .route("/api/v1/hiveboxes/{id}/files", get(handlers::download_file))
        .route("/api/v1/analytics", get(handlers::get_analytics))
        .route("/healthz", get(|| async { "ok" }))
        .route("/dashboard", get(dashboard::dashboard_page))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Starts the API server without authentication.
pub async fn start_server(port: u16) -> anyhow::Result<()> {
    let manager = Arc::new(SandboxManager::new());

    let reaper_manager = manager.clone();
    tokio::spawn(async move {
        reaper_manager.run_reaper().await;
    });

    let metrics_manager = manager.clone();
    tokio::spawn(async move {
        metrics_manager.run_metrics_collector().await;
    });

    let state = SharedState { manager };
    let router = build_router(state);
    let addr = format!("0.0.0.0:{port}");

    info!(addr, "starting HiveBox API server (no auth)");

    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}

/// Starts the API server with Bearer token authentication.
pub async fn start_server_with_auth(port: u16, api_key: String) -> anyhow::Result<()> {
    let manager = Arc::new(SandboxManager::new());

    let reaper_manager = manager.clone();
    tokio::spawn(async move {
        reaper_manager.run_reaper().await;
    });

    let metrics_manager = manager.clone();
    tokio::spawn(async move {
        metrics_manager.run_metrics_collector().await;
    });

    let state = SharedState { manager };
    let router = build_router_with_auth(state, api_key);
    let addr = format!("0.0.0.0:{port}");

    info!(addr, "starting HiveBox API server (auth enabled)");

    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}

/// Builds the router with API key authentication middleware.
fn build_router_with_auth(state: AppState, api_key: String) -> Router {
    let api_routes = Router::new()
        .route("/api/v1/hiveboxes", post(handlers::create_sandbox))
        .route("/api/v1/hiveboxes", get(handlers::list_sandboxes))
        .route("/api/v1/hiveboxes/{id}", get(handlers::get_sandbox))
        .route("/api/v1/hiveboxes/{id}", delete(handlers::destroy_sandbox))
        .route("/api/v1/hiveboxes/{id}/exec", post(handlers::exec_in_sandbox))
        .route("/api/v1/hiveboxes/{id}/mcp", post(mcp::mcp_handler))
        .route("/api/v1/hiveboxes/{id}/files", put(handlers::upload_file))
        .route("/api/v1/hiveboxes/{id}/files", get(handlers::download_file))
        .route("/api/v1/analytics", get(handlers::get_analytics))
        .layer(middleware::from_fn(move |req, next| {
            let key = api_key.clone();
            auth_middleware(req, next, key)
        }))
        .with_state(state);

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/dashboard", get(dashboard::dashboard_page))
        .merge(api_routes)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}

/// Middleware that validates the `Authorization: Bearer <key>` header.
async fn auth_middleware(
    req: Request,
    next: Next,
    expected_key: String,
) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = &header[7..];
            if token == expected_key {
                Ok(next.run(req).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
