//! Multi-node service registry and Hysteria 2 core hub management.
//!
//! Provides isolated user whitelists, multi-panel traffic disaggregation,
//! and unified node routing for multiple Hysteria 2 instances.

use crate::config::{Config, NodeConfig};
use crate::sync::PanelClient;
use crate::traffic::{TrafficCollector, TrafficRecord};
use crate::user::{UserInfo, UserManager};
use anyhow::{Context, Result};
use dashmap::DashMap;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// An active node instance representing an X-board / V2board node connection.
#[derive(Debug)]
pub struct NodeInstance {
    pub tag: String,
    pub node_id: u32,
    #[allow(dead_code)]
    pub node_type: String,
    pub api_host: String,
    #[allow(dead_code)]
    pub api_key: String,
    pub sync_interval_secs: u64,
    pub push_interval_secs: u64,
    #[allow(dead_code)]
    pub hysteria_base_url: String,
    pub hysteria_traffic_url: String,
    pub user_manager: Arc<UserManager>,
    pub panel_client: Arc<PanelClient>,
    pub traffic_collector: Arc<TrafficCollector>,
}

impl NodeInstance {
    pub fn new(cfg: &NodeConfig) -> Self {
        let tag = cfg
            .tag
            .clone()
            .unwrap_or_else(|| format!("node_{}", cfg.node_id));
        let user_manager = Arc::new(UserManager::new());
        let panel_client = Arc::new(PanelClient::new(
            &cfg.api_host,
            &cfg.api_key,
            cfg.node_id,
            &cfg.node_type,
        ));
        let traffic_collector = Arc::new(TrafficCollector::new(
            cfg.hysteria_traffic_url.clone(),
            user_manager.clone(),
        ));

        Self {
            tag,
            node_id: cfg.node_id,
            node_type: cfg.node_type.clone(),
            api_host: cfg.api_host.clone(),
            api_key: cfg.api_key.clone(),
            sync_interval_secs: cfg.sync_interval_secs,
            push_interval_secs: cfg.push_interval_secs,
            hysteria_base_url: cfg.hysteria_base_url.clone(),
            hysteria_traffic_url: cfg.hysteria_traffic_url.clone(),
            user_manager,
            panel_client,
            traffic_collector,
        }
    }
}

/// Shared Hysteria 2 core hub that aggregates multiple nodes sharing the same Hysteria instance.
#[derive(Debug)]
#[allow(dead_code)]
pub struct HysteriaCoreHub {
    pub traffic_url: String,
    pub nodes: Vec<Arc<NodeInstance>>,
    http_client: Client,
    last_snapshot: Mutex<HashMap<String, TrafficRecord>>,
}

#[allow(dead_code)]
impl HysteriaCoreHub {
    pub fn new(traffic_url: String, nodes: Vec<Arc<NodeInstance>>) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        Self {
            traffic_url,
            nodes,
            http_client,
            last_snapshot: Mutex::new(HashMap::new()),
        }
    }

    /// Fetch raw traffic from Hysteria 2 core and route delta metrics to the respective node.
    pub async fn poll_and_dispatch_traffic(&self) -> Result<usize> {
        let resp = self
            .http_client
            .get(&self.traffic_url)
            .send()
            .await
            .with_context(|| format!("Failed to request traffic from {}", self.traffic_url))?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Hysteria 2 traffic endpoint returned HTTP {}",
                resp.status()
            );
        }

        let raw_stats: HashMap<String, TrafficRecord> = resp
            .json()
            .await
            .with_context(|| "Failed to parse traffic JSON response")?;

        let active_count = self.dispatch_raw_stats(raw_stats);
        Ok(active_count)
    }

    /// Calculate delta metrics and route to the correct NodeInstance based on user ownership.
    pub fn dispatch_raw_stats(&self, current_stats: HashMap<String, TrafficRecord>) -> usize {
        let mut last_snap = self.last_snapshot.lock().unwrap();
        let mut active_users = 0;

        for (user_key, current_record) in &current_stats {
            let last_record = last_snap.get(user_key).copied().unwrap_or_default();

            let delta_tx = if current_record.tx >= last_record.tx {
                current_record.tx - last_record.tx
            } else {
                current_record.tx
            };

            let delta_rx = if current_record.rx >= last_record.rx {
                current_record.rx - last_record.rx
            } else {
                current_record.rx
            };

            if delta_tx > 0 || delta_rx > 0 {
                // Find which node owns this user UUID
                let mut routed = false;
                for node in &self.nodes {
                    if let Some(user) = node.user_manager.authenticate(user_key) {
                        // Deliver delta to this node's TrafficCollector
                        let mut single_stat = HashMap::new();
                        single_stat.insert(user.uuid.clone(), *current_record);
                        node.traffic_collector.process_raw_stats(single_stat);
                        routed = true;
                        active_users += 1;
                        break;
                    }
                }

                if !routed {
                    tracing::trace!(
                        user_key = %user_key,
                        "Traffic observed for user not found in any bound node whitelist"
                    );
                }
            }
        }

        *last_snap = current_stats;
        active_users
    }
}

/// Registry managing all active nodes and Hysteria core hubs.
#[derive(Debug, Default)]
pub struct NodeRegistry {
    nodes: Vec<Arc<NodeInstance>>,
    nodes_by_tag: DashMap<String, Arc<NodeInstance>>,
    nodes_by_id: DashMap<u32, Arc<NodeInstance>>,
    #[allow(dead_code)]
    hubs: Vec<Arc<HysteriaCoreHub>>,
}

impl NodeRegistry {
    /// Initialize registry from top-level configuration.
    pub fn from_config(config: &Config) -> Self {
        let mut nodes = Vec::with_capacity(config.nodes.len());
        let nodes_by_tag = DashMap::new();
        let nodes_by_id = DashMap::new();

        // Group nodes by normalized hysteria_traffic_url
        let mut nodes_by_traffic_url: HashMap<String, Vec<Arc<NodeInstance>>> = HashMap::new();

        for node_cfg in &config.nodes {
            let instance = Arc::new(NodeInstance::new(node_cfg));
            nodes.push(instance.clone());

            nodes_by_tag.insert(instance.tag.clone(), instance.clone());
            nodes_by_id.insert(instance.node_id, instance.clone());

            nodes_by_traffic_url
                .entry(instance.hysteria_traffic_url.clone())
                .or_default()
                .push(instance.clone());
        }

        let mut hubs = Vec::with_capacity(nodes_by_traffic_url.len());
        for (traffic_url, hub_nodes) in nodes_by_traffic_url {
            hubs.push(Arc::new(HysteriaCoreHub::new(traffic_url, hub_nodes)));
        }

        Self {
            nodes,
            nodes_by_tag,
            nodes_by_id,
            hubs,
        }
    }

    /// Authenticate a user token against a specific node (by tag or node_id string)
    /// or across all active nodes (if node_key is None).
    pub fn authenticate(&self, node_key: Option<&str>, auth_token: &str) -> Option<UserInfo> {
        let auth_str = auth_token.trim();

        // 1. Target specific node if node_key is provided
        if let Some(key) = node_key {
            let trimmed_key = key.trim();
            if let Some(node) = self.nodes_by_tag.get(trimmed_key) {
                return node.user_manager.authenticate(auth_str);
            }
            if let Ok(id) = trimmed_key.parse::<u32>()
                && let Some(node) = self.nodes_by_id.get(&id)
            {
                return node.user_manager.authenticate(auth_str);
            }
        }

        // 2. Global search across all nodes (first match priority)
        for node in &self.nodes {
            if let Some(user) = node.user_manager.authenticate(auth_str) {
                return Some(user);
            }
        }

        None
    }

    /// Retrieve all managed nodes.
    pub fn all_nodes(&self) -> &[Arc<NodeInstance>] {
        &self.nodes
    }

    /// Retrieve all Hysteria core hubs.
    #[allow(dead_code)]
    pub fn all_hubs(&self) -> &[Arc<HysteriaCoreHub>] {
        &self.hubs
    }

    /// Total number of configured nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}
