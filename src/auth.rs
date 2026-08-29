//! Hysteria 2 HTTP Authentication Webhook Server.
//!
//! Provides ultra-low latency HTTP endpoint (`POST /auth`) consumed by the native
//! Hysteria 2 core to authenticate incoming client connections against `UserManager`.

use crate::user::UserManager;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

/// Hysteria 2 auth.http request payload schema.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthRequest {
    /// Remote client IP:Port (e.g. "1.2.3.4:5678")
    #[serde(default)]
    pub addr: Option<String>,
    /// Client auth credential (UUID / Password)
    pub auth: String,
    /// Client requested Tx bandwidth in bytes/sec (optional)
    #[serde(default)]
    pub tx: Option<u64>,
}

/// Hysteria 2 auth.http response payload schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthResponse {
    /// Authentication result status
    pub ok: bool,
    /// Identifier to assign to this connection (Hysteria 2 uses this as key in traffic stats)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Error message if authentication failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

/// Handler for `POST /auth`.
pub async fn handle_auth(
    State(user_manager): State<Arc<UserManager>>,
    Json(payload): Json<AuthRequest>,
) -> impl IntoResponse {
    let auth_str = payload.auth.trim();
    if let Some(user) = user_manager.authenticate(auth_str) {
        tracing::debug!(
            user_id = user.id,
            uuid = %user.uuid,
            client_addr = ?payload.addr,
            req_tx = ?payload.tx,
            "Hysteria 2 authentication succeeded"
        );
        (
            StatusCode::OK,
            Json(AuthResponse {
                ok: true,
                id: Some(user.uuid),
                msg: None,
            }),
        )
    } else {
        tracing::warn!(
            auth_token = auth_str,
            client_addr = ?payload.addr,
            "Hysteria 2 authentication rejected: user not found or expired"
        );
        (
            StatusCode::OK,
            Json(AuthResponse {
                ok: false,
                id: None,
                msg: Some("user not found or expired".to_string()),
            }),
        )
    }
}

/// Handler for health check and status verification.
pub async fn handle_health(State(user_manager): State<Arc<UserManager>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "healthy",
            "service": "hyboard-bridge",
            "active_users": user_manager.user_count(),
        })),
    )
}

/// Construct the Axum Router for authentication service.
pub fn create_auth_router(user_manager: Arc<UserManager>) -> Router {
    Router::new()
        .route("/auth", post(handle_auth))
        .route("/health", get(handle_health))
        .route("/", get(handle_health))
        .with_state(user_manager)
}

/// Start the Axum HTTP authentication webhook server.
pub async fn run_auth_server(
    listen_addr: &str,
    user_manager: Arc<UserManager>,
) -> anyhow::Result<()> {
    let app = create_auth_router(user_manager);
    let socket_addr: SocketAddr = listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(socket_addr).await?;

    tracing::info!(
        addr = %socket_addr,
        "Hysteria 2 HTTP Auth Webhook server listening"
    );

    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user::UserInfo;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for oneshot

    #[tokio::test]
    async fn test_auth_webhook_success_and_failure() {
        let user_manager = Arc::new(UserManager::new());
        user_manager.update_users(vec![UserInfo {
            id: 10,
            uuid: "test-valid-uuid-1234".to_string(),
            speed_limit: 0,
        }]);

        let app = create_auth_router(user_manager);

        // 1. Test Valid Auth Request
        let req_valid = Request::builder()
            .uri("/auth")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(Body::from(
                r#"{"addr": "1.2.3.4:5678", "auth": "test-valid-uuid-1234", "tx": 1000000}"#,
            ))
            .unwrap();

        let resp_valid = app.clone().oneshot(req_valid).await.unwrap();
        assert_eq!(resp_valid.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(resp_valid.into_body(), usize::MAX)
            .await
            .unwrap();
        let auth_resp: AuthResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert!(auth_resp.ok);
        assert_eq!(auth_resp.id.as_deref(), Some("test-valid-uuid-1234"));

        // 2. Test Invalid Auth Request
        let req_invalid = Request::builder()
            .uri("/auth")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(Body::from(
                r#"{"addr": "1.2.3.4:5678", "auth": "non-existent-uuid"}"#,
            ))
            .unwrap();

        let resp_invalid = app.oneshot(req_invalid).await.unwrap();
        assert_eq!(resp_invalid.status(), StatusCode::OK);

        let body_bytes_inv = axum::body::to_bytes(resp_invalid.into_body(), usize::MAX)
            .await
            .unwrap();
        let auth_resp_inv: AuthResponse = serde_json::from_slice(&body_bytes_inv).unwrap();
        assert!(!auth_resp_inv.ok);
        assert_eq!(
            auth_resp_inv.msg.as_deref(),
            Some("user not found or expired")
        );
    }
}
