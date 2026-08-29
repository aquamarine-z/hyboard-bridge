//! Traffic collection, delta calculation, and reliable buffering for Hysteria 2.
//!
//! Interacts with Hysteria 2's native `GET /traffic` endpoint, calculates incremental
//! upload/download bytes for each user, and maintains a resilient buffer that guarantees
//! zero traffic loss during temporary panel network disconnections.

use crate::user::UserManager;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Raw traffic counters reported by Hysteria 2 core.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TrafficRecord {
    /// Transmitted by server to client (Download traffic for the user)
    #[serde(default)]
    pub tx: u64,
    /// Received by server from client (Upload traffic for the user)
    #[serde(default)]
    pub rx: u64,
}

/// Thread-safe traffic collector and delta accumulator.
#[derive(Debug)]
pub struct TrafficCollector {
    /// URL of Hysteria 2 traffic statistics API (e.g. `http://127.0.0.1:7654/traffic`)
    hysteria_url: String,
    /// HTTP Client with timeout
    http_client: Client,
    /// Reference to UserManager for UUID -> User ID translation
    user_manager: Arc<UserManager>,
    /// Last seen snapshot of raw traffic counters from Hysteria { Identifier -> TrafficRecord }
    last_snapshot: Mutex<HashMap<String, TrafficRecord>>,
    /// Accumulated unpushed traffic deltas { user_id -> (upload_bytes, download_bytes) }
    pending_deltas: Mutex<HashMap<u32, (u64, u64)>>,
}

impl TrafficCollector {
    /// Create a new `TrafficCollector`.
    pub fn new(hysteria_url: String, user_manager: Arc<UserManager>) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        Self {
            hysteria_url,
            http_client,
            user_manager,
            last_snapshot: Mutex::new(HashMap::new()),
            pending_deltas: Mutex::new(HashMap::new()),
        }
    }

    /// Query Hysteria 2 core API, calculate delta traffic, and accumulate into pending deltas.
    ///
    /// Returns the number of active users with traffic changes.
    pub async fn collect_and_accumulate(&self) -> Result<usize> {
        let resp = self
            .http_client
            .get(&self.hysteria_url)
            .send()
            .await
            .with_context(|| {
                format!(
                    "Failed to request Hysteria 2 traffic stats from {}",
                    self.hysteria_url
                )
            })?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Hysteria 2 traffic stats endpoint returned HTTP {}",
                resp.status()
            );
        }

        let raw_stats: HashMap<String, TrafficRecord> = resp
            .json()
            .await
            .with_context(|| "Failed to parse Hysteria 2 traffic JSON response")?;

        let active_count = self.process_raw_stats(raw_stats);
        Ok(active_count)
    }

    /// Process a map of raw traffic stats, calculate incremental deltas, and accumulate.
    pub fn process_raw_stats(&self, current_stats: HashMap<String, TrafficRecord>) -> usize {
        let mut last_snap = self.last_snapshot.lock().unwrap();
        let mut pending = self.pending_deltas.lock().unwrap();
        let mut active_users = 0;

        for (user_key, current_record) in &current_stats {
            // Resolve user_id: first try parsing as integer, then look up via UUID
            let user_id = if let Ok(id) = user_key.parse::<u32>() {
                Some(id)
            } else {
                self.user_manager.get_user_id_by_uuid(user_key)
            };

            let Some(id) = user_id else {
                tracing::trace!(
                    user_key = %user_key,
                    "Traffic reported for unknown or removed user, skipping"
                );
                continue;
            };

            let last_record = last_snap.get(user_key).copied().unwrap_or_default();

            // Calculate incremental delta with overflow / restart protection
            let delta_tx = if current_record.tx >= last_record.tx {
                current_record.tx - last_record.tx
            } else {
                // Hysteria restarted or counter was reset
                current_record.tx
            };

            let delta_rx = if current_record.rx >= last_record.rx {
                current_record.rx - last_record.rx
            } else {
                // Hysteria restarted or counter was reset
                current_record.rx
            };

            if delta_tx > 0 || delta_rx > 0 {
                // In Hysteria: tx = download to user, rx = upload from user
                let upload_bytes = delta_rx;
                let download_bytes = delta_tx;

                let entry = pending.entry(id).or_insert((0, 0));
                entry.0 = entry.0.saturating_add(upload_bytes);
                entry.1 = entry.1.saturating_add(download_bytes);
                active_users += 1;

                tracing::trace!(
                    user_id = id,
                    upload = upload_bytes,
                    download = download_bytes,
                    "Accumulated traffic delta"
                );
            }
        }

        // Update the snapshot
        *last_snap = current_stats;
        active_users
    }

    /// Extract a snapshot of pending traffic formatted for the X-board UniProxy push API.
    ///
    /// Format: `{ "<user_id>": [upload_bytes, download_bytes] }`
    pub fn get_pending_payload(&self) -> HashMap<String, [u64; 2]> {
        let pending = self.pending_deltas.lock().unwrap();
        let mut payload = HashMap::with_capacity(pending.len());
        for (&user_id, &(u, d)) in pending.iter() {
            if u > 0 || d > 0 {
                payload.insert(user_id.to_string(), [u, d]);
            }
        }
        payload
    }

    /// Acknowledge that the specified payload was successfully pushed to the panel.
    ///
    /// Safely subtracts the pushed quantities from `pending_deltas`, ensuring that
    /// any new traffic accumulated concurrently during the network push is preserved.
    pub fn commit_pushed_payload(&self, pushed: &HashMap<String, [u64; 2]>) {
        let mut pending = self.pending_deltas.lock().unwrap();
        for (id_str, &[pushed_u, pushed_d]) in pushed {
            if let Ok(user_id) = id_str.parse::<u32>()
                && let Some(entry) = pending.get_mut(&user_id)
            {
                entry.0 = entry.0.saturating_sub(pushed_u);
                entry.1 = entry.1.saturating_sub(pushed_d);
                if entry.0 == 0 && entry.1 == 0 {
                    pending.remove(&user_id);
                }
            }
        }
    }

    /// Get total pending upload and download bytes across all users.
    pub fn total_pending_bytes(&self) -> (u64, u64) {
        let pending = self.pending_deltas.lock().unwrap();
        let mut total_u: u64 = 0;
        let mut total_d: u64 = 0;
        for &(u, d) in pending.values() {
            total_u = total_u.saturating_add(u);
            total_d = total_d.saturating_add(d);
        }
        (total_u, total_d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user::UserInfo;

    #[test]
    fn test_traffic_delta_and_commit() {
        let user_manager = Arc::new(UserManager::new());
        user_manager.update_users(vec![
            UserInfo {
                id: 101,
                uuid: "user-uuid-1".to_string(),
                speed_limit: 0,
            },
            UserInfo {
                id: 102,
                uuid: "user-uuid-2".to_string(),
                speed_limit: 0,
            },
        ]);

        let collector = TrafficCollector::new(
            "http://127.0.0.1:7654/traffic".to_string(),
            user_manager.clone(),
        );

        // 1. First collection tick
        let mut tick1 = HashMap::new();
        tick1.insert(
            "user-uuid-1".to_string(),
            TrafficRecord { tx: 1000, rx: 500 },
        );
        tick1.insert(
            "user-uuid-2".to_string(),
            TrafficRecord { tx: 2000, rx: 800 },
        );

        let active = collector.process_raw_stats(tick1);
        assert_eq!(active, 2);

        let payload1 = collector.get_pending_payload();
        // In Hysteria: tx = download, rx = upload. UniProxy expects [upload, download]
        assert_eq!(payload1.get("101"), Some(&[500, 1000]));
        assert_eq!(payload1.get("102"), Some(&[800, 2000]));

        // Commit push
        collector.commit_pushed_payload(&payload1);
        assert_eq!(collector.get_pending_payload().len(), 0);

        // 2. Second collection tick (incremental)
        let mut tick2 = HashMap::new();
        tick2.insert(
            "user-uuid-1".to_string(),
            TrafficRecord { tx: 1200, rx: 600 },
        ); // +200 tx, +100 rx
        tick2.insert(
            "user-uuid-2".to_string(),
            TrafficRecord { tx: 2000, rx: 800 },
        ); // unchanged

        let active2 = collector.process_raw_stats(tick2);
        assert_eq!(active2, 1);

        let payload2 = collector.get_pending_payload();
        assert_eq!(payload2.get("101"), Some(&[100, 200]));
        assert_eq!(payload2.get("102"), None);

        // 3. Test Hysteria core restart (counters reset to lower values)
        let mut tick_restart = HashMap::new();
        tick_restart.insert("user-uuid-1".to_string(), TrafficRecord { tx: 50, rx: 30 });

        collector.process_raw_stats(tick_restart);
        let payload3 = collector.get_pending_payload();
        // Uncommitted from tick2 (100, 200) + new tick (30, 50) = (130, 250)
        assert_eq!(payload3.get("101"), Some(&[130, 250]));
    }
}
