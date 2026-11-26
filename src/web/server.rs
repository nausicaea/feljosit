//! The HTTP server (axum on tokio).
//!
//! The server reads the complete request body with a size limit. Then a worker thread
//! handles the request with [`App::handle`]. Thus a long import does not stop the
//! other connections.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};

use super::{App, Request, Response};
use crate::db::log;

async fn handle(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let (parts, body) = request.into_parts();
    let mut req = Request {
        method: parts.method.as_str().to_string(),
        path: parts.uri.path().to_string(),
        query: parts.uri.query().unwrap_or("").to_string(),
        headers: parts
            .headers
            .iter()
            .map(|(name, value)| {
                (name.as_str().to_string(), String::from_utf8_lossy(value.as_bytes()).into_owned())
            })
            .collect(),
        body: Vec::new(),
        peer_is_loopback: peer.ip().is_loopback(),
    };
    let limit = app.max_body_bytes();
    let declared_too_large =
        req.header("content-length").is_some_and(|v| v.trim().parse::<usize>().map_or(true, |n| n > limit));
    let response = if declared_too_large {
        // App::handle refuses the request from the header. Do not read the body.
        let app = app.clone();
        let req = req.clone();
        run(app, req).await
    } else {
        match axum::body::to_bytes(body, limit).await {
            Ok(bytes) => {
                req.body = bytes.to_vec();
                run(app.clone(), req.clone()).await
            }
            Err(_) => app.finish(&req, Response::text(413, "Request body too large")),
        }
    };
    log::info(&format!("{} \"{} {}\" {}", peer.ip(), req.method, req.path, response.status));
    convert(response)
}

async fn run(app: Arc<App>, req: Request) -> Response {
    let finish_app = app.clone();
    let finish_req = req.clone();
    match tokio::task::spawn_blocking(move || app.handle(&req)).await {
        Ok(response) => response,
        Err(error) => {
            log::warning(&format!("A request handler stopped: {error}"));
            finish_app.finish(&finish_req, Response::text(500, "Internal Server Error"))
        }
    }
}

fn convert(response: Response) -> axum::response::Response {
    let mut out = axum::response::Response::new(Body::from(response.body));
    *out.status_mut() = StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    for (name, value) in response.headers {
        if let (Ok(name), Ok(value)) = (HeaderName::try_from(name), HeaderValue::try_from(value)) {
            out.headers_mut().append(name, value);
        }
    }
    out
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    log::info("The server stops.");
}

/// Serve the application on the listener until the user presses Ctrl+C.
pub async fn serve(app: Arc<App>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    let router = axum::Router::new().fallback(handle).with_state(app);
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
}
