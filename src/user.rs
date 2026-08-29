//! User management and high-concurrency lock-free authentication cache.
//!
//! Provides $O(1)$ microsecond-level lookups for incoming Hysteria 2 connection
//! authentication requests using `DashMap`.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// User metadata retrieved from X-board / V2board UniProxy API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInfo {
    /// Panel User ID
    pub id: u32,
    /// User UUID / Password for Hysteria 2 connection
    pub uuid: String,
    /// Speed limit in bytes/sec or Mbps (0 for unlimited)
    #[serde(default)]
    pub speed_limit: u32,
}

/// Thread-safe user cache for high-throughput authentication and ID mapping.
#[derive(Debug, Default)]
pub struct UserManager {
    /// Mapping: UUID -> UserInfo (Primary lookup key for auth)
    users_by_uuid: DashMap<String, UserInfo>,
    /// Mapping: User ID -> UUID (Reverse lookup for quick resolution)
    uuid_by_id: DashMap<u32, String>,
}

impl UserManager {
    /// Create a new empty `UserManager`.
    pub fn new() -> Self {
        Self {
            users_by_uuid: DashMap::new(),
            uuid_by_id: DashMap::new(),
        }
    }

    /// Authenticate a user token/UUID against the in-memory whitelist.
    ///
    /// Performs an $O(1)$ lock-free lookup with microsecond-level latency.
    pub fn authenticate(&self, auth_token: &str) -> Option<UserInfo> {
        let trimmed = auth_token.trim();
        // Look up by exact match
        if let Some(user) = self.users_by_uuid.get(trimmed) {
            return Some(user.clone());
        }
        // Fallback: look up in lowercase if UUID was sent in mixed case
        let lower = trimmed.to_lowercase();
        if lower != trimmed
            && let Some(user) = self.users_by_uuid.get(&lower)
        {
            return Some(user.clone());
        }
        None
    }

    /// Get user ID corresponding to a given UUID.
    pub fn get_user_id_by_uuid(&self, uuid: &str) -> Option<u32> {
        let trimmed = uuid.trim();
        if let Some(user) = self.users_by_uuid.get(trimmed) {
            return Some(user.id);
        }
        let lower = trimmed.to_lowercase();
        if lower != trimmed
            && let Some(user) = self.users_by_uuid.get(&lower)
        {
            return Some(user.id);
        }
        None
    }

    /// Retrieve UserInfo by UUID.
    #[allow(dead_code)]
    pub fn get_user_by_uuid(&self, uuid: &str) -> Option<UserInfo> {
        self.authenticate(uuid)
    }

    /// Retrieve UserInfo by User ID.
    #[allow(dead_code)]
    pub fn get_user_by_id(&self, id: u32) -> Option<UserInfo> {
        if let Some(uuid_ref) = self.uuid_by_id.get(&id) {
            self.users_by_uuid
                .get(uuid_ref.as_str())
                .map(|u| u.value().clone())
        } else {
            None
        }
    }

    /// Atomically synchronize in-memory user whitelist with the new user list from panel.
    ///
    /// - Adds new users
    /// - Updates modified users
    /// - Removes users that are no longer in the panel whitelist
    ///
    /// Returns `(total_active_users, updated_or_added_count)`.
    pub fn update_users(&self, new_users: Vec<UserInfo>) -> (usize, usize) {
        let mut active_uuids = HashSet::with_capacity(new_users.len());
        let mut active_ids = HashSet::with_capacity(new_users.len());
        let mut updated_or_added = 0;

        for user in new_users {
            let normalized_uuid = user.uuid.trim().to_lowercase();
            active_uuids.insert(normalized_uuid.clone());
            active_ids.insert(user.id);

            // Update uuid_by_id map
            self.uuid_by_id.insert(user.id, normalized_uuid.clone());

            // Check if user changed or is new
            let needs_insert = match self.users_by_uuid.get(&normalized_uuid) {
                Some(existing) => *existing != user,
                None => true,
            };

            if needs_insert {
                self.users_by_uuid.insert(normalized_uuid, user);
                updated_or_added += 1;
            }
        }

        // Evict expired or removed users
        self.users_by_uuid
            .retain(|uuid, _| active_uuids.contains(uuid));
        self.uuid_by_id.retain(|id, _| active_ids.contains(id));

        (self.users_by_uuid.len(), updated_or_added)
    }

    /// Total count of active authorized users in memory.
    pub fn user_count(&self) -> usize {
        self.users_by_uuid.len()
    }

    /// Returns a snapshot of all active users.
    #[allow(dead_code)]
    pub fn all_users(&self) -> Vec<UserInfo> {
        self.users_by_uuid
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_manager_crud_and_auth() {
        let manager = UserManager::new();
        assert_eq!(manager.user_count(), 0);

        let users = vec![
            UserInfo {
                id: 1,
                uuid: "6BA7B810-9DAD-11D1-80B4-00C04FD430C8".to_string(),
                speed_limit: 1000,
            },
            UserInfo {
                id: 2,
                uuid: "d3b07384-d113-40e1-bb97-b2f7f9859f9a".to_string(),
                speed_limit: 0,
            },
        ];

        let (total, changed) = manager.update_users(users);
        assert_eq!(total, 2);
        assert_eq!(changed, 2);
        assert_eq!(manager.user_count(), 2);

        // Case-insensitive authentication check
        let auth1 = manager.authenticate("6ba7b810-9dad-11d1-80b4-00c04fd430c8");
        assert!(auth1.is_some());
        assert_eq!(auth1.unwrap().id, 1);

        let auth2 = manager.authenticate("d3b07384-d113-40e1-bb97-b2f7f9859f9a");
        assert!(auth2.is_some());
        assert_eq!(auth2.unwrap().id, 2);

        // Invalid user auth
        let auth_invalid = manager.authenticate("invalid-uuid-token");
        assert!(auth_invalid.is_none());

        // Reverse lookup
        assert_eq!(
            manager.get_user_id_by_uuid("d3b07384-d113-40e1-bb97-b2f7f9859f9a"),
            Some(2)
        );
        assert_eq!(manager.get_user_by_id(1).map(|u| u.id), Some(1));

        // Sync with removal of user 1
        let new_users = vec![UserInfo {
            id: 2,
            uuid: "d3b07384-d113-40e1-bb97-b2f7f9859f9a".to_string(),
            speed_limit: 5000, // speed limit updated
        }];

        let (total2, changed2) = manager.update_users(new_users);
        assert_eq!(total2, 1);
        assert_eq!(changed2, 1);
        assert_eq!(
            manager.authenticate("6ba7b810-9dad-11d1-80b4-00c04fd430c8"),
            None
        );
        assert_eq!(manager.get_user_by_id(2).unwrap().speed_limit, 5000);
    }
}
