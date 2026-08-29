//! hyboard-bridge: High-performance daemon bridging X-board/V2board to Hysteria 2.
//!
//! Provides ultra-low latency authentication, resilient incremental traffic metering,
//! and automated orchestration.

mod auth;
mod config;
mod sync;
mod traffic;
mod user;

use anyhow::Result;
use config::Config;
use std::sync::Arc;
use std::time::Duration;
use sync::PanelClient;
use traffic::TrafficCollector;
use user::UserManager;

/// Main bridge daemon orchestrator.
pub struct HyboardBridge {
    config: Arc<Config>,
    user_manager: Arc<UserManager>,
    traffic_collector: Arc<TrafficCollector>,
    panel_client: Arc<PanelClient>,
}

impl HyboardBridge {
    /// Initialize a new instance of `HyboardBridge`.
    pub fn new(config: Config) -> Self {
        let config = Arc::new(config);
        let user_manager = Arc::new(UserManager::new());
        let traffic_collector = Arc::new(TrafficCollector::new(
            config.hysteria_traffic_url.clone(),
            user_manager.clone(),
        ));
        let panel_client = Arc::new(PanelClient::new(&config));

        Self {
            config,
            user_manager,
            traffic_collector,
            panel_client,
        }
    }

    /// Start and run the bridge daemon services.
    pub async fn run(&self) -> Result<()> {
        // 1. Perform initial user pull from panel
        tracing::info!("Executing initial user synchronization from panel...");
        match self.panel_client.fetch_users().await {
            Ok(users) => {
                let (total, _) = self.user_manager.update_users(users);
                tracing::info!(active_users = total, "Initial user synchronization complete");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Initial user synchronization failed; will retry in background loop"
                );
            }
        }

        // 3. Spawn Axum Auth Webhook Server
        let auth_listen_addr = self.config.auth_listen_addr.clone();
        let user_mgr_clone = self.user_manager.clone();
        let auth_server_handle = tokio::spawn(async move {
            if let Err(e) = auth::run_auth_server(&auth_listen_addr, user_mgr_clone).await {
                tracing::error!(error = %e, "Auth webhook server terminated unexpectedly");
            }
        });

        // 4. Spawn User Sync Loop
        let sync_client = self.panel_client.clone();
        let sync_mgr = self.user_manager.clone();
        let sync_interval = Duration::from_secs(self.config.sync_interval_secs);
        let sync_loop_handle = tokio::spawn(async move {
            sync::run_user_sync_loop(sync_client, sync_mgr, sync_interval).await;
        });

        // 5. Spawn Traffic Push Loop
        let push_client = self.panel_client.clone();
        let push_collector = self.traffic_collector.clone();
        let push_interval = Duration::from_secs(self.config.push_interval_secs);
        let push_loop_handle = tokio::spawn(async move {
            sync::run_traffic_push_loop(push_client, push_collector, push_interval).await;
        });

        tracing::info!(
            node_id = self.config.node_id,
            auth_endpoint = %self.config.auth_listen_addr,
            hysteria_traffic_api = %self.config.hysteria_traffic_url,
            sync_interval_s = self.config.sync_interval_secs,
            push_interval_s = self.config.push_interval_secs,
            "hyboard-bridge daemon started successfully"
        );

        // 6. Wait for termination signals (SIGINT / SIGTERM)
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received Ctrl+C / SIGINT signal, initiating graceful shutdown...");
            }
            _ = auth_server_handle => {
                tracing::error!("Auth server task ended unexpectedly");
            }
        }

        // Abort background worker loops
        sync_loop_handle.abort();
        push_loop_handle.abort();

        // 7. Flush final traffic metrics before exiting
        self.shutdown_flush().await;

        tracing::info!("hyboard-bridge daemon terminated gracefully");
        Ok(())
    }

    /// Perform a final traffic poll and push to the panel upon graceful shutdown.
    async fn shutdown_flush(&self) {
        tracing::info!("Flushing pending traffic metrics to panel before shutdown...");
        let _ = self.traffic_collector.collect_and_accumulate().await;
        let payload = self.traffic_collector.get_pending_payload();
        if !payload.is_empty() {
            match self.panel_client.push_traffic(&payload).await {
                Ok(_) => {
                    self.traffic_collector.commit_pushed_payload(&payload);
                    tracing::info!("Final traffic metrics flushed successfully");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to flush final traffic metrics to panel");
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize structured logging
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,hyboard_bridge=debug"));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_thread_ids(false)
        .init();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "Initializing hyboard-bridge"
    );

    // Load configuration
    let config = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "Configuration validation failed");
            std::process::exit(1);
        }
    };

    let bridge = HyboardBridge::new(config);
    bridge.run().await
}
