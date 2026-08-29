//! hyboard-bridge: High-performance daemon bridging X-board/V2board to Hysteria 2.
//!
//! Supports multi-node and multi-panel orchestration with ultra-low latency authentication
//! and resilient incremental traffic metering.

mod auth;
mod config;
mod node;
mod sync;
mod traffic;
mod user;

use anyhow::Result;
use config::Config;
use node::NodeRegistry;
use std::sync::Arc;
use std::time::Duration;

/// Main bridge daemon orchestrator.
pub struct HyboardBridge {
    config: Arc<Config>,
    registry: Arc<NodeRegistry>,
}

impl HyboardBridge {
    /// Initialize a new instance of `HyboardBridge`.
    pub fn new(config: Config) -> Self {
        let registry = Arc::new(NodeRegistry::from_config(&config));
        let config = Arc::new(config);

        Self { config, registry }
    }

    /// Start and run the multi-node bridge daemon services.
    pub async fn run(&self) -> Result<()> {
        let nodes = self.registry.all_nodes();
        tracing::info!(
            node_count = nodes.len(),
            "Executing initial user synchronization across all nodes..."
        );

        // 1. Perform initial user sync across all nodes concurrently
        for node in nodes {
            let client = node.panel_client.clone();
            let user_mgr = node.user_manager.clone();
            let tag = node.tag.clone();
            let node_id = node.node_id;

            match client.fetch_users().await {
                Ok(users) => {
                    let (total, _) = user_mgr.update_users(users);
                    tracing::info!(
                        tag = %tag,
                        node_id = node_id,
                        active_users = total,
                        "Initial user synchronization complete"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        tag = %tag,
                        node_id = node_id,
                        error = %e,
                        "Initial user synchronization failed; will retry in background loop"
                    );
                }
            }
        }

        // 2. Spawn Axum Auth Webhook Server
        let listen_addr = format!("0.0.0.0:{}", self.config.global.listen_port);
        let registry_clone = self.registry.clone();
        let auth_server_handle = tokio::spawn(async move {
            if let Err(e) = auth::run_auth_server(&listen_addr, registry_clone).await {
                tracing::error!(error = %e, "Auth webhook server terminated unexpectedly");
            }
        });

        // 3. Spawn background workers for each node
        let mut worker_handles = Vec::new();

        for node in nodes {
            // User Sync Loop
            let sync_client = node.panel_client.clone();
            let sync_mgr = node.user_manager.clone();
            let sync_interval = Duration::from_secs(node.sync_interval_secs);
            let sync_handle = tokio::spawn(async move {
                sync::run_user_sync_loop(sync_client, sync_mgr, sync_interval).await;
            });
            worker_handles.push(sync_handle);

            // Traffic Push Loop
            let push_client = node.panel_client.clone();
            let push_collector = node.traffic_collector.clone();
            let push_interval = Duration::from_secs(node.push_interval_secs);
            let push_handle = tokio::spawn(async move {
                sync::run_traffic_push_loop(push_client, push_collector, push_interval).await;
            });
            worker_handles.push(push_handle);
        }

        tracing::info!(
            listen_port = self.config.global.listen_port,
            managed_nodes = nodes.len(),
            "hyboard-bridge daemon started successfully"
        );

        // 4. Wait for termination signals (SIGINT / SIGTERM)
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received Ctrl+C / SIGINT signal, initiating graceful shutdown...");
            }
            _ = auth_server_handle => {
                tracing::error!("Auth server task ended unexpectedly");
            }
        }

        // Abort background worker loops
        for handle in worker_handles {
            handle.abort();
        }

        // 5. Flush final traffic metrics before exiting
        self.shutdown_flush().await;

        tracing::info!("hyboard-bridge daemon terminated gracefully");
        Ok(())
    }

    /// Perform a final traffic poll and push to panels upon graceful shutdown.
    async fn shutdown_flush(&self) {
        tracing::info!("Flushing pending traffic metrics to panels before shutdown...");
        for node in self.registry.all_nodes() {
            let _ = node.traffic_collector.collect_and_accumulate().await;
            let payload = node.traffic_collector.get_pending_payload();
            if !payload.is_empty() {
                match node.panel_client.push_traffic(&payload).await {
                    Ok(_) => {
                        node.traffic_collector.commit_pushed_payload(&payload);
                        tracing::info!(tag = %node.tag, "Final traffic metrics flushed successfully");
                    }
                    Err(e) => {
                        tracing::warn!(
                            tag = %node.tag,
                            error = %e,
                            "Failed to flush final traffic metrics to panel"
                        );
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Configuration error: {:#}", e);
            std::process::exit(1);
        }
    };

    // Initialize structured logging
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.global.rust_log));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_thread_ids(false)
        .init();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "Initializing hyboard-bridge"
    );

    let bridge = HyboardBridge::new(config);
    bridge.run().await
}
