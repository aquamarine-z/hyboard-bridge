//! Panel synchronization client and asynchronous background task loops.
//!
//! Implements X-board / V2board UniProxy protocol for periodic user whitelist
//! synchronization and incremental traffic reporting / heartbeat maintenance.

use crate::config::Config;
use crate::traffic::TrafficCollector;
use crate::user::{UserInfo, UserManager};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// UniProxy API client for X-board and V2board panels.
#[derive(Debug, Clone)]
pub struct PanelClient {
    api_host: String,
    api_key: String,
    node_id: u32,
    node_type: String,
    http_client: Client,
}

/// Helper struct to deserialize various panel response formats flexibly.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum UserApiResponse {
    DataArray { data: Vec<UserInfo> },
    UsersArray { users: Vec<UserInfo> },
    NestedData { data: NestedUserData },
    DirectArray(Vec<UserInfo>),
}

#[derive(Debug, Deserialize)]
struct NestedUserData {
    #[serde(default)]
    users: Vec<UserInfo>,
}

impl PanelClient {
    /// Create a new `PanelClient` from configuration.
    pub fn new(config: &Config) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        Self {
            api_host: config.api_host.clone(),
            api_key: config.api_key.clone(),
            node_id: config.node_id,
            node_type: config.node_type.clone(),
            http_client,
        }
    }

    /// Fetch the latest user whitelist from the panel.
    pub async fn fetch_users(&self) -> Result<Vec<UserInfo>> {
        let url = format!(
            "{}/api/v1/server/UniProxy/user?node_id={}&node_type={}&token={}",
            self.api_host, self.node_id, self.node_type, self.api_key
        );

        let resp = self
            .http_client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to connect to panel at {}", self.api_host))?;

        if !resp.status().is_success() {
            anyhow::bail!("Panel user sync API returned HTTP status {}", resp.status());
        }

        let body_text = resp
            .text()
            .await
            .with_context(|| "Failed to read panel response body")?;

        let parsed: UserApiResponse = serde_json::from_str(&body_text).with_context(|| {
            format!(
                "Failed to deserialize user list from panel response: {}",
                body_text.chars().take(200).collect::<String>()
            )
        })?;

        let users = match parsed {
            UserApiResponse::DataArray { data } => data,
            UserApiResponse::UsersArray { users } => users,
            UserApiResponse::NestedData { data } => data.users,
            UserApiResponse::DirectArray(users) => users,
        };

        Ok(users)
    }

    /// Push accumulated traffic deltas and send heartbeat to the panel.
    ///
    /// Even if `payload` is empty `{}` (no traffic generated), it is pushed to keep the
    /// node online status active in X-board / V2board.
    pub async fn push_traffic(&self, payload: &HashMap<String, [u64; 2]>) -> Result<()> {
        let url = format!(
            "{}/api/v1/server/UniProxy/push?node_id={}&node_type={}&token={}",
            self.api_host, self.node_id, self.node_type, self.api_key
        );

        let resp = self
            .http_client
            .post(&url)
            .json(payload)
            .send()
            .await
            .with_context(|| "Failed to send traffic push request to panel")?;

        if !resp.status().is_success() {
            anyhow::bail!("Panel traffic push API returned HTTP status {}", resp.status());
        }

        Ok(())
    }
}

/// Periodic user whitelist synchronization loop.
pub async fn run_user_sync_loop(
    client: Arc<PanelClient>,
    user_manager: Arc<UserManager>,
    interval: Duration,
) {
    let mut interval_timer = tokio::time::interval(interval);
    // Don't burst if a tick was delayed
    interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval_timer.tick().await;

        match client.fetch_users().await {
            Ok(users) => {
                let user_count = users.len();
                let (total_active, changed) = user_manager.update_users(users);
                tracing::info!(
                    fetched = user_count,
                    active = total_active,
                    updated = changed,
                    "User whitelist synchronized from panel"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    retained_users = user_manager.user_count(),
                    "Failed to fetch users from panel; retaining previous local cache"
                );
            }
        }
    }
}

/// Periodic traffic collection, delta calculation, and push loop.
pub async fn run_traffic_push_loop(
    client: Arc<PanelClient>,
    traffic_collector: Arc<TrafficCollector>,
    interval: Duration,
) {
    let mut interval_timer = tokio::time::interval(interval);
    interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval_timer.tick().await;

        // 1. Collect latest traffic from Hysteria 2 core
        if let Err(e) = traffic_collector.collect_and_accumulate().await {
            tracing::debug!(
                error = %e,
                "Hysteria 2 traffic poll tick (core may be idle or initializing)"
            );
        }

        // 2. Prepare payload
        let payload = traffic_collector.get_pending_payload();
        let user_count = payload.len();
        let (total_u, total_d) = traffic_collector.total_pending_bytes();

        // 3. Push to panel (payload may be empty `{}` for heartbeat)
        match client.push_traffic(&payload).await {
            Ok(_) => {
                traffic_collector.commit_pushed_payload(&payload);
                if user_count > 0 {
                    tracing::info!(
                        active_users = user_count,
                        upload_bytes = total_u,
                        download_bytes = total_d,
                        "Traffic metrics pushed to panel successfully"
                    );
                } else {
                    tracing::debug!("Heartbeat ping pushed to panel successfully");
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    buffered_users = user_count,
                    buffered_upload = total_u,
                    buffered_download = total_d,
                    "Failed to push traffic to panel; metrics retained in buffer for next cycle"
                );
            }
        }
    }
}
