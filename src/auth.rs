//! Hysteria 2 HTTP Authentication Webhook Server with Multi-Node Support.
//!
//! Provides ultra-low latency HTTP endpoints consumed by Hysteria 2 core instances:
//! - `POST /auth/:node_key`: Direct routing to a specific node (by tag or node_id)
//! - `POST /auth`: Universal authentication across all bound nodes
//! - `GET /health`: Multi-node health and active user status

use crate::node::NodeRegistry;
use axum::{
    Json, Router,
    extract::{Path, State},
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
    /// Identifier to assign to this connection (UUID)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Error message if authentication failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

/// Handler for `POST /auth` (Universal authentication across all nodes).
pub async fn handle_auth_universal(
    State(registry): State<Arc<NodeRegistry>>,
    Json(payload): Json<AuthRequest>,
) -> impl IntoResponse {
    authenticate_and_respond(&registry, None, payload)
}

/// Handler for `POST /auth/:node_key` (Node-targeted authentication by tag or ID).
pub async fn handle_auth_targeted(
    Path(node_key): Path<String>,
    State(registry): State<Arc<NodeRegistry>>,
    Json(payload): Json<AuthRequest>,
) -> impl IntoResponse {
    authenticate_and_respond(&registry, Some(&node_key), payload)
}

fn authenticate_and_respond(
    registry: &NodeRegistry,
    node_key: Option<&str>,
    payload: AuthRequest,
) -> (StatusCode, Json<AuthResponse>) {
    let auth_str = payload.auth.trim();

    if let Some(user) = registry.authenticate(node_key, auth_str) {
        tracing::debug!(
            user_id = user.id,
            uuid = %user.uuid,
            node_key = ?node_key,
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
            node_key = ?node_key,
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

/// Handler for health check and multi-node status verification.
pub async fn handle_health(State(registry): State<Arc<NodeRegistry>>) -> impl IntoResponse {
    let node_details: Vec<serde_json::Value> = registry
        .all_nodes()
        .iter()
        .map(|node| {
            serde_json::json!({
                "tag": node.tag,
                "node_id": node.node_id,
                "api_host": node.api_host,
                "active_users": node.user_manager.user_count(),
                "traffic_url": node.hysteria_traffic_url,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "healthy",
            "service": "hyboard-bridge",
            "nodes_count": registry.node_count(),
            "nodes": node_details,
        })),
    )
}

/// Construct the Axum Router for authentication service.
pub fn create_auth_router(registry: Arc<NodeRegistry>) -> Router {
    Router::new()
        .route("/auth", post(handle_auth_universal))
        .route("/auth/{node_key}", post(handle_auth_targeted))
        .route("/health", get(handle_health))
        .route("/", get(handle_health).post(handle_auth_universal))
        .with_state(registry)
}

/// Start the Axum HTTP authentication webhook server.
pub async fn run_auth_server(listen_addr: &str, registry: Arc<NodeRegistry>) -> anyhow::Result<()> {
    let app = create_auth_router(registry);
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
    use crate::config::{Config, GlobalConfig, NodeConfig};
    use crate::user::UserInfo;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_multi_node_auth_routing() {
        let config = Config {
            global: GlobalConfig::default(),
            nodes: vec![
                NodeConfig {
                    tag: Some("hk_1".to_string()),
                    api_host: "https://panel-a.com".to_string(),
                    api_key: "key1".to_string(),
                    node_id: 1,
                    node_type: "hysteria".to_string(),
                    sync_interval_secs: 15,
                    push_interval_secs: 60,
                    hysteria_base_url: "http://127.0.0.1:7654".to_string(),
                    hysteria_traffic_url: "http://127.0.0.1:7654/traffic".to_string(),
                },
                NodeConfig {
                    tag: Some("us_2".to_string()),
                    api_host: "https://panel-b.com".to_string(),
                    api_key: "key2".to_string(),
                    node_id: 2,
                    node_type: "hysteria".to_string(),
                    sync_interval_secs: 15,
                    push_interval_secs: 60,
                    hysteria_base_url: "http://127.0.0.1:7655".to_string(),
                    hysteria_traffic_url: "http://127.0.0.1:7655/traffic".to_string(),
                },
            ],
        };

        let registry = Arc::new(NodeRegistry::from_config(&config));

        // Add user to node 1
        registry.all_nodes()[0]
            .user_manager
            .update_users(vec![UserInfo {
                id: 10,
                uuid: "user-hk-uuid".to_string(),
                speed_limit: 0,
                device_limit: None,
            }]);

        // Add user to node 2
        registry.all_nodes()[1]
            .user_manager
            .update_users(vec![UserInfo {
                id: 20,
                uuid: "user-us-uuid".to_string(),
                speed_limit: 0,
                device_limit: None,
            }]);

        let app = create_auth_router(registry);

        // 1. Test targeted auth to /auth/hk_1
        let req_hk = Request::builder()
            .uri("/auth/hk_1")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"auth": "user-hk-uuid"}"#))
            .unwrap();

        let resp = app.clone().oneshot(req_hk).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: AuthResponse = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(body.ok);

        // 2. Test targeted auth to /auth/hk_1 with user from US (should fail)
        let req_fail = Request::builder()
            .uri("/auth/hk_1")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"auth": "user-us-uuid"}"#))
            .unwrap();

        let resp_fail = app.clone().oneshot(req_fail).await.unwrap();
        let body_fail: AuthResponse = serde_json::from_slice(
            &axum::body::to_bytes(resp_fail.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(!body_fail.ok);

        // 3. Test universal auth to /auth (should find both)
        let req_univ = Request::builder()
            .uri("/auth")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"auth": "user-us-uuid"}"#))
            .unwrap();

        let resp_univ = app.oneshot(req_univ).await.unwrap();
        let body_univ: AuthResponse = serde_json::from_slice(
            &axum::body::to_bytes(resp_univ.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(body_univ.ok);
    }
}
