//! Configuration management for hyboard-bridge.
//!
//! Supports multi-node and multi-panel configuration via `config.toml`,
//! with seamless backward-compatible fallback to environment variables (`.env`).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

fn default_listen_port() -> u16 {
    9999
}

fn default_rust_log() -> String {
    "info".to_string()
}

fn default_node_type() -> String {
    "hysteria".to_string()
}

fn default_sync_interval() -> u64 {
    15
}

fn default_push_interval() -> u64 {
    60
}

/// Global settings across all managed nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// Webhook server listen port (default: 9999)
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,

    /// Log level filter (default: "info")
    #[serde(default = "default_rust_log")]
    pub rust_log: String,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            listen_port: default_listen_port(),
            rust_log: default_rust_log(),
        }
    }
}

/// Configuration for a specific node connecting to an X-board / V2board panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Unique tag/identifier for this node (e.g. "hk_panel_a" or "1")
    #[serde(default)]
    pub tag: Option<String>,

    /// Panel API host, e.g. `https://panel.example.com`
    pub api_host: String,

    /// UniProxy communication key / token for this panel
    pub api_key: String,

    /// Node ID registered in this panel
    pub node_id: u32,

    /// Node type (default: "hysteria")
    #[serde(default = "default_node_type")]
    pub node_type: String,

    /// Interval in seconds for pulling user whitelist (default: 15s)
    #[serde(default = "default_sync_interval", rename = "sync_interval")]
    pub sync_interval_secs: u64,

    /// Interval in seconds for pushing traffic metrics and heartbeat (default: 60s)
    #[serde(default = "default_push_interval", rename = "push_interval")]
    pub push_interval_secs: u64,

    /// Base URL of the Hysteria 2 core trafficStats API (e.g. `http://127.0.0.1:7654`)
    pub hysteria_base_url: String,

    /// Normalized full URL for traffic statistics (computed automatically)
    #[serde(skip)]
    pub hysteria_traffic_url: String,
}

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub global: GlobalConfig,

    #[serde(default)]
    pub nodes: Vec<NodeConfig>,
}

impl Config {
    /// Load configuration from `config.toml` (if present) or environment variables (`.env`).
    pub fn load() -> Result<Self> {
        let _ = dotenvy::dotenv();

        // 1. Try loading from file (custom path via CONFIG_FILE or default "config.toml")
        let config_path =
            std::env::var("CONFIG_FILE").unwrap_or_else(|_| "config.toml".to_string());
        if Path::new(&config_path).exists() {
            let content = std::fs::read_to_string(&config_path).with_context(|| {
                format!("Failed to read configuration file at '{}'", config_path)
            })?;
            let mut cfg: Config = toml::from_str(&content).with_context(|| {
                format!("Failed to parse TOML configuration from '{}'", config_path)
            })?;

            cfg.validate_and_normalize()?;
            tracing::info!(path = %config_path, node_count = cfg.nodes.len(), "Loaded configuration from TOML file");
            return Ok(cfg);
        }

        // 2. Fallback: Load single-node configuration from environment variables
        let api_host = std::env::var("API_HOST")
            .context("No config.toml found. Environment variable API_HOST is required")?
            .trim_end_matches('/')
            .to_string();

        let api_key = std::env::var("API_KEY")
            .or_else(|_| std::env::var("TOKEN"))
            .context("Environment variable API_KEY (or TOKEN) is required")?;

        let node_id: u32 = std::env::var("NODE_ID")
            .context("Environment variable NODE_ID is required (e.g. 1)")?
            .parse()
            .context("Failed to parse NODE_ID as u32")?;

        let node_type = std::env::var("NODE_TYPE").unwrap_or_else(|_| default_node_type());

        let sync_interval_secs = std::env::var("SYNC_INTERVAL")
            .unwrap_or_else(|_| default_sync_interval().to_string())
            .parse::<u64>()
            .unwrap_or(default_sync_interval());

        let push_interval_secs = std::env::var("PUSH_INTERVAL")
            .unwrap_or_else(|_| default_push_interval().to_string())
            .parse::<u64>()
            .unwrap_or(default_push_interval());

        let listen_port = std::env::var("LISTEN_PORT")
            .or_else(|_| std::env::var("PORT"))
            .unwrap_or_else(|_| default_listen_port().to_string())
            .parse::<u16>()
            .unwrap_or(default_listen_port());

        let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| default_rust_log());

        let hysteria_base_url = std::env::var("HYSTERIA_BASE_URL")
            .or_else(|_| std::env::var("HYSTERIA_URL"))
            .or_else(|_| std::env::var("HYSTERIA_API"))
            .context(
                "Environment variable HYSTERIA_BASE_URL is required (e.g. http://127.0.0.1:7654)",
            )?;

        let mut node = NodeConfig {
            tag: Some(format!("node_{}", node_id)),
            api_host,
            api_key,
            node_id,
            node_type,
            sync_interval_secs,
            push_interval_secs,
            hysteria_base_url,
            hysteria_traffic_url: String::new(),
        };
        node.hysteria_traffic_url = Self::normalize_traffic_url(&node.hysteria_base_url);

        let cfg = Config {
            global: GlobalConfig {
                listen_port,
                rust_log,
            },
            nodes: vec![node],
        };

        tracing::info!(
            node_id = node_id,
            "Loaded single-node configuration from environment variables"
        );
        Ok(cfg)
    }

    /// Validate configuration and normalize URLs / tags.
    pub fn validate_and_normalize(&mut self) -> Result<()> {
        if self.nodes.is_empty() {
            anyhow::bail!("Configuration error: At least one [[nodes]] definition is required");
        }

        for (idx, node) in self.nodes.iter_mut().enumerate() {
            if node.api_host.trim().is_empty() {
                anyhow::bail!("Node [{}] is missing 'api_host'", idx);
            }
            node.api_host = node.api_host.trim_end_matches('/').to_string();

            if node.api_key.trim().is_empty() {
                anyhow::bail!("Node [{}] is missing 'api_key'", idx);
            }

            if node.hysteria_base_url.trim().is_empty() {
                anyhow::bail!("Node [{}] is missing 'hysteria_base_url'", idx);
            }

            node.hysteria_traffic_url = Self::normalize_traffic_url(&node.hysteria_base_url);

            // Assign default tag if missing
            if node.tag.as_ref().is_none_or(|t| t.trim().is_empty()) {
                node.tag = Some(format!("node_{}", node.node_id));
            }
        }

        Ok(())
    }

    /// Normalize Hysteria 2 base URL to full `/traffic` endpoint.
    pub fn normalize_traffic_url(input: &str) -> String {
        let trimmed = input.trim();
        let with_proto = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            trimmed.to_string()
        } else {
            format!("http://{}", trimmed)
        };

        let base = with_proto
            .trim_end_matches('/')
            .strip_suffix("/traffic")
            .unwrap_or(with_proto.trim_end_matches('/'));

        format!("{}/traffic", base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_multi_node_toml() {
        let toml_str = r#"
            [global]
            listen_port = 9999
            rust_log = "debug"

            [[nodes]]
            tag = "hk_panel_a"
            api_host = "https://xboard-a.example.com"
            api_key = "token_a"
            node_id = 1
            hysteria_base_url = "http://127.0.0.1:7654"

            [[nodes]]
            tag = "hk_panel_b"
            api_host = "https://xboard-b.example.com"
            api_key = "token_b"
            node_id = 101
            hysteria_base_url = "http://127.0.0.1:7654"

            [[nodes]]
            tag = "us_node"
            api_host = "https://xboard-a.example.com"
            api_key = "token_a"
            node_id = 2
            hysteria_base_url = "http://127.0.0.1:7655"
        "#;

        let mut config: Config = toml::from_str(toml_str).unwrap();
        config.validate_and_normalize().unwrap();

        assert_eq!(config.global.listen_port, 9999);
        assert_eq!(config.nodes.len(), 3);

        assert_eq!(config.nodes[0].tag.as_deref(), Some("hk_panel_a"));
        assert_eq!(config.nodes[0].node_id, 1);
        assert_eq!(
            config.nodes[0].hysteria_traffic_url,
            "http://127.0.0.1:7654/traffic"
        );

        assert_eq!(config.nodes[1].tag.as_deref(), Some("hk_panel_b"));
        assert_eq!(config.nodes[1].node_id, 101);
        assert_eq!(
            config.nodes[1].hysteria_traffic_url,
            "http://127.0.0.1:7654/traffic"
        );

        assert_eq!(config.nodes[2].tag.as_deref(), Some("us_node"));
        assert_eq!(config.nodes[2].node_id, 2);
        assert_eq!(
            config.nodes[2].hysteria_traffic_url,
            "http://127.0.0.1:7655/traffic"
        );
    }
}
