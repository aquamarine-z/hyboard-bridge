//! Configuration management for hyboard-bridge.
//!
//! Handles loading configuration from environment variables / `.env` file.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default internal port for the Axum authentication webhook server.
const DEFAULT_AUTH_LISTEN_ADDR: &str = "0.0.0.0:9999";

/// Application configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // --- X-board / V2board Panel API Settings ---
    /// Panel API host, e.g. `https://panel.example.com` or `http://127.0.0.1:8000`
    pub api_host: String,
    /// UniProxy communication key / token
    pub api_key: String,
    /// Node ID registered in the panel
    pub node_id: u32,
    /// Node type, defaults to `hysteria`
    pub node_type: String,
    /// Interval in seconds for pulling user whitelist from panel (default: 15s)
    pub sync_interval_secs: u64,
    /// Interval in seconds for pushing traffic metrics to panel (default: 60s)
    pub push_interval_secs: u64,

    // --- Hysteria 2 Integration Settings ---
    /// Address for the Axum HTTP authentication webhook server (fixed to `0.0.0.0:9999`)
    pub auth_listen_addr: String,
    /// URL of Hysteria 2 traffic statistics API (e.g. `http://127.0.0.1:7654/traffic`)
    pub hysteria_traffic_url: String,
}

impl Config {
    /// Load configuration from environment variables and `.env` file.
    pub fn from_env() -> Result<Self> {
        // Load .env file if present
        let _ = dotenvy::dotenv();

        let api_host = std::env::var("API_HOST")
            .context("Environment variable API_HOST is required (e.g. https://panel.example.com)")?
            .trim_end_matches('/')
            .to_string();

        let api_key = std::env::var("API_KEY")
            .or_else(|_| std::env::var("TOKEN"))
            .context("Environment variable API_KEY (or TOKEN) is required")?;

        let node_id: u32 = std::env::var("NODE_ID")
            .context("Environment variable NODE_ID is required (e.g. 1)")?
            .parse()
            .context("Failed to parse NODE_ID as u32")?;

        let node_type = std::env::var("NODE_TYPE").unwrap_or_else(|_| "hysteria".to_string());

        let sync_interval_secs = std::env::var("SYNC_INTERVAL")
            .unwrap_or_else(|_| "15".to_string())
            .parse::<u64>()
            .unwrap_or(15);

        let push_interval_secs = std::env::var("PUSH_INTERVAL")
            .unwrap_or_else(|_| "60".to_string())
            .parse::<u64>()
            .unwrap_or(60);

        // Required HYSTERIA_API parameter: e.g. "http://127.0.0.1:7654" or "http://hysteria:7654"
        let raw_hysteria_api = std::env::var("HYSTERIA_API")
            .context("Environment variable HYSTERIA_API is required (e.g. http://127.0.0.1:7654 or http://hysteria:7654)")?;

        let hysteria_traffic_url = Self::normalize_traffic_url(&raw_hysteria_api)?;
        let auth_listen_addr = DEFAULT_AUTH_LISTEN_ADDR.to_string();

        Ok(Self {
            api_host,
            api_key,
            node_id,
            node_type,
            sync_interval_secs,
            push_interval_secs,
            auth_listen_addr,
            hysteria_traffic_url,
        })
    }

    /// Normalize the user-supplied HYSTERIA_API URL into the full traffic endpoint.
    ///
    /// Examples:
    /// - `"http://127.0.0.1:7654"` -> `"http://127.0.0.1:7654/traffic"`
    /// - `"http://hysteria:7654"` -> `"http://hysteria:7654/traffic"`
    /// - `"http://127.0.0.1:7654/traffic"` -> `"http://127.0.0.1:7654/traffic"`
    pub fn normalize_traffic_url(input: &str) -> Result<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            anyhow::bail!("HYSTERIA_API cannot be empty");
        }

        let mut url = trimmed.to_string();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            url = format!("http://{}", url);
        }

        if !url.ends_with("/traffic") {
            url = format!("{}/traffic", url.trim_end_matches('/'));
        }

        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_traffic_url() {
        assert_eq!(
            Config::normalize_traffic_url("http://127.0.0.1:7654").unwrap(),
            "http://127.0.0.1:7654/traffic"
        );
        assert_eq!(
            Config::normalize_traffic_url("http://hysteria:7654").unwrap(),
            "http://hysteria:7654/traffic"
        );
        assert_eq!(
            Config::normalize_traffic_url("http://127.0.0.1:7654/traffic").unwrap(),
            "http://127.0.0.1:7654/traffic"
        );
        assert_eq!(
            Config::normalize_traffic_url("127.0.0.1:7654").unwrap(),
            "http://127.0.0.1:7654/traffic"
        );
    }
}
