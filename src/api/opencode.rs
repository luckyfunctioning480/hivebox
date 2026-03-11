//! Reverse proxy for opencode serve instances.
//!
//! Each hivebox gets its own `opencode serve` process on an internal port.
//! This module proxies external requests from
//! `POST /api/v1/hiveboxes/{id}/opencode/{path}` to the corresponding
//! internal opencode serve instance, so everything is accessible on the
//! single daemon port (7070).

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use reqwest::Client;
use tracing::error;

use super::handlers::AppState;
use super::types::ErrorResponse;

/// Shared reqwest client (connection pooling).
static PROXY_CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();

fn client() -> &'static Client {
    PROXY_CLIENT.get_or_init(|| {
        Client::builder()
            .no_proxy()
            .build()
            .expect("failed to build reqwest client")
    })
}

/// Proxy handler for `ANY /api/v1/hiveboxes/{id}/opencode/{*rest}`.
///
/// Forwards the full request (method, headers, body) to the internal
/// opencode serve instance and streams the response back.
pub async fn opencode_proxy(
    State(state): State<AppState>,
    Path((id, rest)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, (StatusCode, axum::Json<ErrorResponse>)> {
    let port = state.manager.get_opencode_port(&id).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            axum::Json(ErrorResponse {
                error: format!("no opencode instance for sandbox '{id}'"),
            }),
        )
    })?;

    // Strip leading slash from rest path if present.
    let rest = rest.trim_start_matches('/');
    let target_url = format!("http://127.0.0.1:{port}/{rest}");

    // Read the incoming body.
    let body_bytes = axum::body::to_bytes(body, 64 * 1024 * 1024)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to read proxy request body");
            (
                StatusCode::BAD_REQUEST,
                axum::Json(ErrorResponse {
                    error: "failed to read request body".to_string(),
                }),
            )
        })?;

    // Build the proxied request.
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);

    let mut proxy_req = client().request(reqwest_method, &target_url);

    // Forward relevant headers (skip hop-by-hop headers).
    for (name, value) in headers.iter() {
        let n = name.as_str();
        if n == "host" || n == "connection" || n == "transfer-encoding" {
            continue;
        }
        proxy_req = proxy_req.header(name.clone(), value.clone());
    }

    if !body_bytes.is_empty() {
        proxy_req = proxy_req.body(body_bytes);
    }

    // Send to internal opencode serve.
    let resp = proxy_req.send().await.map_err(|e| {
        error!(sandbox = id, url = target_url, error = %e, "opencode proxy request failed");
        (
            StatusCode::BAD_GATEWAY,
            axum::Json(ErrorResponse {
                error: format!("opencode serve unreachable: {e}"),
            }),
        )
    })?;

    // Build the axum response, streaming the body through.
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let resp_headers = resp.headers().clone();
    let stream = resp.bytes_stream();
    let body = Body::from_stream(stream);

    let mut response = Response::new(body);
    *response.status_mut() = status;

    // Copy response headers.
    for (name, value) in resp_headers.iter() {
        let n = name.as_str();
        if n == "transfer-encoding" || n == "connection" {
            continue;
        }
        response.headers_mut().insert(name.clone(), value.clone());
    }

    Ok(response)
}
