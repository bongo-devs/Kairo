//! Request logging middleware for the REST API.
//!
//! Each request is logged once it completes, as `METHOD /path?query, client=..., headers=[...],
//! payload=...`, with every segment gated by its own
//! [`RequestLoggingConfig`](crate::config::RequestLoggingConfig) flag. `before_request` adds a
//! second line, prefixed `>> `, before the handler runs. Sensitive headers are redacted.

use std::net::SocketAddr;

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, Method};
use axum::middleware::Next;
use axum::response::Response;

use crate::node::AppState;

// Ceiling on how much of a request body is buffered to log it, set far above any real payload so a
// legitimate request is never truncated on the wire. The logged copy is cut to `max_payload_length`.
const MAX_BODY_BUFFER: usize = 16 * 1024 * 1024;

// Headers whose values are replaced with `[REDACTED]` in the `headers=` segment.
const REDACTED_HEADERS: [header::HeaderName; 4] = [
    header::AUTHORIZATION,
    header::PROXY_AUTHORIZATION,
    header::COOKIE,
    header::SET_COOKIE,
];

/// Log each REST request after it completes (and optionally before), per `logging.request.*`.
pub async fn request_logging(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let cfg = state.config().logging.request.clone();
    // Two gates: the flag is the operator asking for request logs, the level filter decides whether
    // anything would print them. Without the second, a filtered-out INFO still buffers every body
    // and formats every header for a line nobody reads.
    if !cfg.enabled || !tracing::enabled!(tracing::Level::INFO) {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    let method = parts.method.clone();
    let path = parts.uri.path().to_string();
    let query = cfg
        .include_query_string
        .then(|| parts.uri.query().map(str::to_owned))
        .flatten();
    let client = cfg
        .include_client_info
        .then(|| {
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.to_string())
        })
        .flatten();
    let headers = cfg.include_headers.then(|| format_headers(&parts.headers));

    // Buffer the body of a body-bearing method so it can be reported as the payload. The handler
    // still receives all of it; only the logged copy is cut short.
    let has_body = matches!(method, Method::POST | Method::PUT | Method::PATCH);
    let (body, payload) = if cfg.include_payload && has_body {
        match axum::body::to_bytes(body, MAX_BODY_BUFFER).await {
            Ok(bytes) => {
                let payload = render_payload(&bytes, cfg.max_payload_length);
                (Body::from(bytes), payload)
            }
            // Too large, or a read error: drop the body rather than replay part of it.
            Err(_) => (Body::empty(), None),
        }
    } else {
        (body, None)
    };

    if cfg.before_request {
        tracing::info!(
            "{}",
            message(">> ", &method, &path, &query, &client, &headers, &None)
        );
    }

    let response = next.run(Request::from_parts(parts, body)).await;

    tracing::info!(
        "{}",
        message("", &method, &path, &query, &client, &headers, &payload)
    );

    response
}

// Assemble one log line from the segments that survived their flags.
fn message(
    prefix: &str,
    method: &Method,
    path: &str,
    query: &Option<String>,
    client: &Option<String>,
    headers: &Option<String>,
    payload: &Option<String>,
) -> String {
    let mut msg = String::with_capacity(64);
    msg.push_str(prefix);
    msg.push_str(method.as_str());
    msg.push(' ');
    msg.push_str(path);
    if let Some(query) = query {
        msg.push('?');
        msg.push_str(query);
    }
    if let Some(client) = client {
        msg.push_str(", client=");
        msg.push_str(client);
    }
    if let Some(headers) = headers {
        msg.push_str(", headers=");
        msg.push_str(headers);
    }
    if let Some(payload) = payload {
        msg.push_str(", payload=");
        msg.push_str(payload);
    }
    msg
}

// Render a body as lossy UTF-8, truncated to `max_len`, or `None` when it is empty.
fn render_payload(bytes: &Bytes, max_len: usize) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let end = bytes.len().min(max_len);
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

// Format the headers as `[name:"value", ...]`, redacting the sensitive ones.
fn format_headers(headers: &HeaderMap) -> String {
    let rendered: Vec<String> = headers
        .iter()
        .map(|(name, value)| {
            let value = if REDACTED_HEADERS.contains(name) {
                "[REDACTED]"
            } else {
                value.to_str().unwrap_or("<binary>")
            };
            format!("{name}:\"{value}\"")
        })
        .collect();
    format!("[{}]", rendered.join(", "))
}
