//! Configuration management for hyboard-bridge.
//!
//! Handles loading configuration from environment variables / `.env` file.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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

    // --- Bridge & Hysteria 2 Endpoint Settings ---
    /// Listen port for this bridge program's Auth Webhook server (default: 9999)
    pub listen_port: u16,
    /// Derived address to bind the Axum Auth server on (e.g. `0.0.0.0:9999`)
    pub auth_listen_addr: String,
    /// Base URL of Hysteria 2 core without subpath (e.g. `http://127.0.0.1:7654`)
    pub hysteria_base_url: String,
    /// Full URL of Hysteria 2 traffic statistics API (e.g. `http://127.0.0.1:7654/traffic`)
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

        // 1. Bridge program listen port (e.g. 9999)
        let listen_port = std::env::var("LISTEN_PORT")
            .or_else(|_| std::env::var("PORT"))
            .unwrap_or_else(|_| "9999".to_string())
            .parse::<u16>()
            .context("Failed to parse LISTEN_PORT as valid u16 port number")?;

        let auth_listen_addr = format!("0.0.0.0:{}", listen_port);

        // 2. Hysteria 2 Base URL without subpaths (e.g. "http://127.0.0.1:7654" or "http://hysteria:7654")
        let raw_base_url = std::env::var("HYSTERIA_BASE_URL")
            .or_else(|_| std::env::var("HYSTERIA_URL"))
            .or_else(|_| std::env::var("HYSTERIA_API"))
            .context("Environment variable HYSTERIA_BASE_URL is required (e.g. http://127.0.0.1:7654 or http://hysteria:7654)")?;

        let (hysteria_base_url, hysteria_traffic_url) = Self::normalize_hysteria_urls(&raw_base_url)?;

        Ok(Self {
            api_host,
            api_key,
            node_id,
            node_type,
            sync_interval_secs,
            push_interval_secs,
            listen_port,
            auth_listen_addr,
            hysteria_base_url,
            hysteria_traffic_url,
        })
    }

    /// Normalize Hysteria 2 base URL and derive the full traffic endpoint.
    ///
    /// Accepts:
    /// - `"http://127.0.0.1:7654"` -> Base: `"http://127.0.0.1:7654"`, Traffic: `"http://127.0.0.1:7654/traffic"`
    /// - `"http://hysteria:7654"` -> Base: `"http://hysteria:7654"`, Traffic: `"http://hysteria:7654/traffic"`
    /// - `"127.0.0.1:7654"` -> Base: `"http://127.0.0.1:7654"`, Traffic: `"http://127.0.0.1:7654/traffic"`
    pub fn normalize_hysteria_urls(input: &str) -> Result<(String, String)> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            anyhow::bail!("HYSTERIA_BASE_URL cannot be empty");
        }

        let with_proto = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            trimmed.to_string()
        } else {
            format!("http://{}", trimmed)
        };

        // Strip subpaths if user accidentally passed /traffic
        let base_url = with_proto
            .trim_end_matches('/')
            .strip_suffix("/traffic")
            .unwrap_or(with_proto.trim_end_matches('/'))
            .to_string();

        let traffic_url = format!("{}/traffic", base_url);

        Ok((base_url, traffic_url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_hysteria_urls() {
        let (base, traffic) = Config::normalize_hysteria_urls("http://127.0.0.1:7654").unwrap();
        assert_eq!(base, "http://127.0.0.1:7654");
        assert_eq!(traffic, "http://127.0.0.1:7654/traffic");

        let (base, traffic) = Config::normalize_hysteria_urls("http://hysteria:7654/").unwrap();
        assert_eq!(base, "http://hysteria:7654");
        assert_eq!(traffic, "http://hysteria:7654/traffic");

        let (base, traffic) = Config::normalize_hysteria_urls("127.0.0.1:7654").unwrap();
        assert_eq!(base, "http://127.0.0.1:7654");
        assert_eq!(traffic, "http://127.0.0.1:7654/traffic");

        let (base, traffic) = Config::normalize_hysteria_urls("http://127.0.0.1:7654/traffic").unwrap();
        assert_eq!(base, "http://127.0.0.1:7654");
        assert_eq!(traffic, "http://127.0.0.1:7654/traffic");
    }
}
