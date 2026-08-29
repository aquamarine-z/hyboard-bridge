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

    // --- Combined Hysteria 2 Integration Settings ---
    /// Address for the Axum HTTP authentication webhook server (e.g. `0.0.0.0:9999`)
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

        // Parse combined required parameter HYSTERIA_API (or fallback to legacy variables if provided)
        let hysteria_api_input = std::env::var("HYSTERIA_API")
            .or_else(|_| std::env::var("HYSTERIA_ADDR"))
            .or_else(|_| {
                // Backward compatibility if both legacy envs are present
                if let (Ok(auth), Ok(traffic)) = (std::env::var("AUTH_LISTEN_ADDR"), std::env::var("HYSTERIA_TRAFFIC_URL")) {
                    Ok(format!("{}@{}", auth, traffic))
                } else if let Ok(traffic) = std::env::var("HYSTERIA_TRAFFIC_URL") {
                    Ok(traffic)
                } else {
                    Err(std::env::VarError::NotPresent)
                }
            })
            .context("Environment variable HYSTERIA_API is required (e.g. 127.0.0.1, hysteria, or http://127.0.0.1:7654)")?;

        let (auth_listen_addr, hysteria_traffic_url) = Self::parse_hysteria_api(&hysteria_api_input)?;

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

    /// Parse unified `HYSTERIA_API` parameter into (auth_listen_addr, hysteria_traffic_url).
    ///
    /// Supported formats:
    /// - `"127.0.0.1"` or `"hysteria"` -> `("0.0.0.0:9999", "http://<host>:7654/traffic")`
    /// - `"http://127.0.0.1:7654"` -> `("0.0.0.0:9999", "http://127.0.0.1:7654/traffic")`
    /// - `"0.0.0.0:9999@http://127.0.0.1:7654/traffic"` -> `("0.0.0.0:9999", "http://127.0.0.1:7654/traffic")`
    /// - `"9999:7654"` -> `("0.0.0.0:9999", "http://127.0.0.1:7654/traffic")`
    pub fn parse_hysteria_api(input: &str) -> Result<(String, String)> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            anyhow::bail!("HYSTERIA_API parameter cannot be empty");
        }

        // 1. Explicit combined format: "auth_addr@traffic_url"
        if let Some((auth_part, traffic_part)) = trimmed.split_once('@') {
            let auth = if auth_part.contains(':') {
                auth_part.trim().to_string()
            } else {
                format!("0.0.0.0:{}", auth_part.trim())
            };

            let mut traffic = traffic_part.trim().to_string();
            if !traffic.starts_with("http://") && !traffic.starts_with("https://") {
                traffic = format!("http://{}", traffic);
            }
            if !traffic.ends_with("/traffic") {
                traffic = format!("{}/traffic", traffic.trim_end_matches('/'));
            }

            return Ok((auth, traffic));
        }

        // 2. Port pair format: "9999:7654"
        let parts: Vec<&str> = trimmed.split(':').collect();
        if parts.len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()) && parts[1].chars().all(|c| c.is_ascii_digit()) {
            let auth_port = parts[0];
            let traffic_port = parts[1];
            return Ok((
                format!("0.0.0.0:{}", auth_port),
                format!("http://127.0.0.1:{}/traffic", traffic_port),
            ));
        }

        // 3. HTTP URL format: "http://127.0.0.1:7654" or "http://hysteria:7654/traffic"
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            let mut traffic = trimmed.to_string();
            if !traffic.ends_with("/traffic") {
                traffic = format!("{}/traffic", traffic.trim_end_matches('/'));
            }
            return Ok(("0.0.0.0:9999".to_string(), traffic));
        }

        // 4. Host:Port format: "127.0.0.1:7654" or "hysteria:7654"
        if trimmed.contains(':') {
            return Ok((
                "0.0.0.0:9999".to_string(),
                format!("http://{}/traffic", trimmed.trim_end_matches('/')),
            ));
        }

        // 5. Plain Host / IP format: "127.0.0.1" or "hysteria"
        Ok((
            "0.0.0.0:9999".to_string(),
            format!("http://{}:7654/traffic", trimmed),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hysteria_api_variants() {
        // Plain host
        let (auth, traffic) = Config::parse_hysteria_api("127.0.0.1").unwrap();
        assert_eq!(auth, "0.0.0.0:9999");
        assert_eq!(traffic, "http://127.0.0.1:7654/traffic");

        // Docker hostname
        let (auth, traffic) = Config::parse_hysteria_api("hysteria").unwrap();
        assert_eq!(auth, "0.0.0.0:9999");
        assert_eq!(traffic, "http://hysteria:7654/traffic");

        // Full URL
        let (auth, traffic) = Config::parse_hysteria_api("http://127.0.0.1:7654").unwrap();
        assert_eq!(auth, "0.0.0.0:9999");
        assert_eq!(traffic, "http://127.0.0.1:7654/traffic");

        let (auth, traffic) = Config::parse_hysteria_api("http://hysteria:7654/traffic").unwrap();
        assert_eq!(auth, "0.0.0.0:9999");
        assert_eq!(traffic, "http://hysteria:7654/traffic");

        // Explicit @ combined format
        let (auth, traffic) = Config::parse_hysteria_api("127.0.0.1:8888@http://127.0.0.1:7777/traffic").unwrap();
        assert_eq!(auth, "127.0.0.1:8888");
        assert_eq!(traffic, "http://127.0.0.1:7777/traffic");

        // Port pair
        let (auth, traffic) = Config::parse_hysteria_api("9999:7654").unwrap();
        assert_eq!(auth, "0.0.0.0:9999");
        assert_eq!(traffic, "http://127.0.0.1:7654/traffic");
    }
}
