//! Real-time room presence tracking (Phoenix Presence style) with RAII cleanup.
//!
//! Tracks connected keys (e.g. user IDs) per room, ref-counting multiple
//! concurrent connections/tabs for the same key so a user is only marked
//! offline when their last connection drops.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::groups::Groups;

/// Metadata for a currently present member in a room.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresenceEntry {
    /// Unique member key (typically a user ID or session ID).
    pub key: String,
    /// Arbitrary JSON metadata associated with this member (e.g. display name, status).
    pub meta: Value,
    /// Unix epoch timestamp (seconds) when the first active connection joined.
    pub joined_at: u64,
}

/// Broadcast event emitted on `"presence:{room}"` when a [`Groups`] registry is attached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PresenceEvent {
    /// Emitted when a key transitions from 0 to 1 active connections in `room`.
    Join {
        /// Room identifier.
        room: String,
        /// Newly joined presence entry.
        entry: PresenceEntry,
    },
    /// Emitted when a key's last active connection leaves `room`.
    Leave {
        /// Room identifier.
        room: String,
        /// Departed member key.
        key: String,
    },
}

#[derive(Debug, Clone)]
struct TrackedMember {
    entry: PresenceEntry,
    connections: usize,
}

#[derive(Default)]
struct PresenceState {
    rooms: HashMap<String, HashMap<String, TrackedMember>>,
}

/// Thread-safe, cheaply cloneable real-time presence tracker.
#[derive(Clone, Default)]
pub struct Presence {
    state: Arc<Mutex<PresenceState>>,
    groups: Option<Groups>,
}

impl Presence {
    /// Create a standalone in-memory [`Presence`] tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a [`Presence`] tracker that publishes join/leave [`PresenceEvent`]s
    /// to `"presence:{room}"` on the supplied [`Groups`] pub/sub registry.
    pub fn with_groups(groups: Groups) -> Self {
        Self {
            state: Arc::new(Mutex::new(PresenceState::default())),
            groups: Some(groups),
        }
    }

    /// Track `key` in `room` with `meta`, returning an RAII [`PresenceGuard`].
    ///
    /// Dropping the returned guard automatically untrack-decrements the connection
    /// and emits a [`PresenceEvent::Leave`] when the last connection for `key` closes.
    pub fn track(&self, room: &str, key: &str, meta: Value) -> PresenceGuard {
        let mut join_event = None;
        {
            let mut guard = self.state.lock().expect("lock presence state");
            let members = guard.rooms.entry(room.to_string()).or_default();
            if let Some(existing) = members.get_mut(key) {
                existing.connections += 1;
                existing.entry.meta = meta;
            } else {
                let joined_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let entry = PresenceEntry {
                    key: key.to_string(),
                    meta,
                    joined_at,
                };
                members.insert(
                    key.to_string(),
                    TrackedMember {
                        entry: entry.clone(),
                        connections: 1,
                    },
                );
                join_event = Some(PresenceEvent::Join {
                    room: room.to_string(),
                    entry,
                });
            }
        }

        if let (Some(groups), Some(ev)) = (&self.groups, join_event) {
            if let Ok(json) = serde_json::to_string(&ev) {
                groups.group(&format!("presence:{room}")).send(json);
            }
        }

        PresenceGuard {
            presence: self.clone(),
            room: room.to_string(),
            key: key.to_string(),
        }
    }

    fn untrack(&self, room: &str, key: &str) {
        let mut leave_event = None;
        {
            let mut guard = self.state.lock().expect("lock presence state");
            if let Some(members) = guard.rooms.get_mut(room) {
                if let Some(member) = members.get_mut(key) {
                    if member.connections > 1 {
                        member.connections -= 1;
                    } else {
                        members.remove(key);
                        leave_event = Some(PresenceEvent::Leave {
                            room: room.to_string(),
                            key: key.to_string(),
                        });
                    }
                }
                if members.is_empty() {
                    guard.rooms.remove(room);
                }
            }
        }

        if let (Some(groups), Some(ev)) = (&self.groups, leave_event) {
            if let Ok(json) = serde_json::to_string(&ev) {
                groups.group(&format!("presence:{room}")).send(json);
            }
        }
    }

    /// List all currently present members in `room`, sorted by key.
    pub fn list(&self, room: &str) -> Vec<PresenceEntry> {
        let guard = self.state.lock().expect("lock presence state");
        let mut entries: Vec<PresenceEntry> = guard
            .rooms
            .get(room)
            .map(|members| members.values().map(|m| m.entry.clone()).collect())
            .unwrap_or_default();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        entries
    }

    /// Return the number of distinct keys currently present in `room`.
    pub fn count(&self, room: &str) -> usize {
        let guard = self.state.lock().expect("lock presence state");
        guard.rooms.get(room).map(|m| m.len()).unwrap_or(0)
    }

    /// Return whether `key` currently has at least one active connection in `room`.
    pub fn is_present(&self, room: &str, key: &str) -> bool {
        let guard = self.state.lock().expect("lock presence state");
        guard
            .rooms
            .get(room)
            .is_some_and(|members| members.contains_key(key))
    }

    /// Return all active room names, sorted alphabetically.
    pub fn rooms(&self) -> Vec<String> {
        let guard = self.state.lock().expect("lock presence state");
        let mut names: Vec<String> = guard.rooms.keys().cloned().collect();
        names.sort();
        names
    }
}

/// RAII guard returned by [`Presence::track`]; untracks the member on [`Drop`].
pub struct PresenceGuard {
    presence: Presence,
    room: String,
    key: String,
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        self.presence.untrack(&self.room, &self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn presence_tracks_multitab_refcount_and_broadcasts_join_leave() {
        let groups = Groups::new();
        let mut sub = groups.group("presence:doc-1").subscribe();
        let presence = Presence::with_groups(groups);

        let g1 = presence.track("doc-1", "user-7", serde_json::json!({"name": "Ada"}));
        assert_eq!(presence.count("doc-1"), 1);
        assert!(presence.is_present("doc-1", "user-7"));

        let raw_join = sub.next().await.unwrap();
        let ev: PresenceEvent = serde_json::from_str(&raw_join).unwrap();
        assert!(matches!(ev, PresenceEvent::Join { room, .. } if room == "doc-1"));

        // Second tab for the same user should not drop presence when first closes.
        let g2 = presence.track(
            "doc-1",
            "user-7",
            serde_json::json!({"name": "Ada", "tab": 2}),
        );
        assert_eq!(presence.count("doc-1"), 1);
        drop(g1);
        assert!(presence.is_present("doc-1", "user-7"));

        drop(g2);
        assert!(!presence.is_present("doc-1", "user-7"));
        assert_eq!(presence.count("doc-1"), 0);
        assert!(presence.rooms().is_empty());

        let raw_leave = sub.next().await.unwrap();
        let ev2: PresenceEvent = serde_json::from_str(&raw_leave).unwrap();
        assert_eq!(
            ev2,
            PresenceEvent::Leave {
                room: "doc-1".to_string(),
                key: "user-7".to_string()
            }
        );
    }
}
