use std::collections::HashMap;
use std::time::{Duration, Instant};

use nexus_core::ids::{CubeId, FileId, UserId};
use nexus_protocol::sync::{
    CursorUpdate as ProtoCursorUpdate, PresenceStatus as ProtoPresenceStatus,
    PresenceUpdate as ProtoPresenceUpdate, SyncStatus as ProtoSyncStatus,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// User cursor position and selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorUpdate {
    pub user_id: UserId,
    pub file_id: FileId,
    pub position: u32,
    pub selection_start: Option<u32>,
    pub selection_end: Option<u32>,
    pub timestamp: u64,
}

impl CursorUpdate {
    pub fn new(user_id: UserId, file_id: crate::FileId, position: u32) -> Self {
        Self {
            user_id,
            file_id,
            position,
            selection_start: None,
            selection_end: None,
            timestamp: nexus_core::timestamp::Timestamp::now().as_millis(),
        }
    }

    pub fn with_selection(mut self, selection_start: u32, selection_end: u32) -> Self {
        self.selection_start = Some(selection_start);
        self.selection_end = Some(selection_end);
        self
    }

    pub fn has_selection(&self) -> bool {
        self.selection_start.is_some() && self.selection_end.is_some()
    }

    pub fn selection_range(&self) -> Option<(u32, u32)> {
        match (self.selection_start, self.selection_end) {
            (Some(start), Some(end)) => Some((start.min(end), start.max(end))),
            _ => None,
        }
    }
}

impl From<CursorUpdate> for ProtoCursorUpdate {
    fn from(update: CursorUpdate) -> Self {
        ProtoCursorUpdate {
            user_id: update.user_id,
            file_id: update.file_id,
            position: update.position,
            selection_start: update.selection_start,
            selection_end: update.selection_end,
            timestamp: update.timestamp,
        }
    }
}

impl From<ProtoCursorUpdate> for CursorUpdate {
    fn from(update: ProtoCursorUpdate) -> Self {
        CursorUpdate {
            user_id: update.user_id,
            file_id: update.file_id,
            position: update.position,
            selection_start: update.selection_start,
            selection_end: update.selection_end,
            timestamp: update.timestamp,
        }
    }
}

/// Presence state for a user in a Cube.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceState {
    pub user_id: UserId,
    pub username: String,
    pub color: UserColor,
    pub status: PresenceStatus,
    pub current_file: Option<FileId>,
    pub cursor: Option<CursorUpdate>,
    pub last_active: u64,
}

impl PresenceState {
    pub fn new(user_id: UserId, username: String, color: UserColor) -> Self {
        Self {
            user_id,
            username,
            color,
            status: PresenceStatus::Active,
            current_file: None,
            cursor: None,
            last_active: nexus_core::timestamp::Timestamp::now().as_millis(),
        }
    }

    pub fn update_cursor(&mut self, cursor: CursorUpdate) {
        self.cursor = Some(cursor);
        self.last_active = nexus_core::timestamp::Timestamp::now().as_millis();
    }

    pub fn set_file(&mut self, file_id: Option<crate::FileId>) {
        self.current_file = file_id;
        self.last_active = nexus_core::timestamp::Timestamp::now().as_millis();
    }

    pub fn set_status(&mut self, status: PresenceStatus) {
        self.status = status;
        self.last_active = nexus_core::timestamp::Timestamp::now().as_millis();
    }

    pub fn is_idle(&self, threshold: Duration) -> bool {
        let now = nexus_core::timestamp::Timestamp::now().as_millis();
        now.saturating_sub(self.last_active) > threshold.as_millis() as u64
    }
}

/// User presence status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    Active,
    Idle,
    Away,
    Offline,
}

impl From<PresenceStatus> for ProtoPresenceStatus {
    fn from(s: PresenceStatus) -> Self {
        match s {
            PresenceStatus::Active => ProtoPresenceStatus::Active,
            PresenceStatus::Idle => ProtoPresenceStatus::Idle,
            PresenceStatus::Away => ProtoPresenceStatus::Away,
            PresenceStatus::Offline => ProtoPresenceStatus::Offline,
        }
    }
}

impl From<ProtoPresenceStatus> for PresenceStatus {
    fn from(s: ProtoPresenceStatus) -> Self {
        match s {
            ProtoPresenceStatus::Active => PresenceStatus::Active,
            ProtoPresenceStatus::Idle => PresenceStatus::Idle,
            ProtoPresenceStatus::Away => PresenceStatus::Away,
            ProtoPresenceStatus::Offline => PresenceStatus::Offline,
        }
    }
}

/// Presence update message (transient, high-frequency).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceUpdate {
    pub user_id: UserId,
    pub cube_id: CubeId,
    pub status: Option<PresenceStatus>,
    pub current_file: Option<FileId>,
    pub cursor: Option<CursorUpdate>,
    pub timestamp: u64,
}

impl PresenceUpdate {
    pub fn new(user_id: UserId, cube_id: CubeId) -> Self {
        Self {
            user_id,
            cube_id,
            status: None,
            current_file: None,
            cursor: None,
            timestamp: nexus_core::timestamp::Timestamp::now().as_millis(),
        }
    }
}

impl From<PresenceUpdate> for ProtoPresenceUpdate {
    fn from(update: PresenceUpdate) -> Self {
        ProtoPresenceUpdate {
            user_id: update.user_id,
            cube_id: update.cube_id,
            status: update.status.map(Into::into),
            current_file: update.current_file,
            cursor: update.cursor.map(Into::into),
            timestamp: update.timestamp,
        }
    }
}

impl From<ProtoPresenceUpdate> for PresenceUpdate {
    fn from(update: ProtoPresenceUpdate) -> Self {
        PresenceUpdate {
            user_id: update.user_id,
            cube_id: update.cube_id,
            status: update.status.map(Into::into),
            current_file: update.current_file,
            cursor: update.cursor.map(Into::into),
            timestamp: update.timestamp,
        }
    }
}

/// Stable visual color assignment for users in a Cube.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserColor(pub u8);

impl UserColor {
    // Predefined theme colors (indices into theme palette)
    pub const PALETTE: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

    pub fn from_index(index: usize) -> Self {
        Self(Self::PALETTE[index % Self::PALETTE.len()])
    }

    pub fn index(&self) -> usize {
        self.0 as usize
    }
}

/// Synchronization status for a document or file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncStatus {
    Unknown,
    Loading,
    Synchronizing,
    Synchronized,
    Desynchronized,
    Recovering,
    Offline,
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

impl From<SyncStatus> for ProtoSyncStatus {
    fn from(s: SyncStatus) -> Self {
        match s {
            SyncStatus::Unknown => ProtoSyncStatus::Unknown,
            SyncStatus::Loading => ProtoSyncStatus::Loading,
            SyncStatus::Synchronizing => ProtoSyncStatus::Synchronizing,
            SyncStatus::Synchronized => ProtoSyncStatus::Synchronized,
            SyncStatus::Desynchronized => ProtoSyncStatus::Desynchronized,
            SyncStatus::Recovering => ProtoSyncStatus::Recovering,
            SyncStatus::Offline => ProtoSyncStatus::Offline,
        }
    }
}

impl From<ProtoSyncStatus> for SyncStatus {
    fn from(s: ProtoSyncStatus) -> Self {
        match s {
            ProtoSyncStatus::Unknown => SyncStatus::Unknown,
            ProtoSyncStatus::Loading => SyncStatus::Loading,
            ProtoSyncStatus::Synchronizing => SyncStatus::Synchronizing,
            ProtoSyncStatus::Synchronized => SyncStatus::Synchronized,
            ProtoSyncStatus::Desynchronized => SyncStatus::Desynchronized,
            ProtoSyncStatus::Recovering => SyncStatus::Recovering,
            ProtoSyncStatus::Offline => SyncStatus::Offline,
        }
    }
}

/// Manages presence and cursor state for a Cube.
pub struct PresenceManager {
    #[allow(dead_code)]
    cube_id: CubeId,
    users: RwLock<HashMap<UserId, PresenceState>>,
    color_allocator: RwLock<ColorAllocator>,
    cursor_throttle: Duration,
    last_cursor_send: RwLock<HashMap<UserId, Instant>>,
}

impl PresenceManager {
    pub fn new(cube_id: CubeId) -> Self {
        Self {
            cube_id,
            users: RwLock::new(HashMap::new()),
            color_allocator: RwLock::new(ColorAllocator::new()),
            cursor_throttle: Duration::from_millis(50), // 20 Hz max
            last_cursor_send: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_throttle(mut self, throttle: Duration) -> Self {
        self.cursor_throttle = throttle;
        self
    }

    /// Add or update a user's presence.
    pub fn upsert_user(&self, user_id: UserId, username: String) -> UserColor {
        let mut users = self.users.write();
        let mut allocator = self.color_allocator.write();

        if let Some(existing) = users.get(&user_id) {
            return existing.color;
        }

        let color = allocator.allocate();
        let presence = PresenceState::new(user_id, username, color);
        users.insert(user_id, presence);
        color
    }

    /// Remove a user.
    pub fn remove_user(&self, user_id: UserId) {
        let mut users = self.users.write();
        let mut allocator = self.color_allocator.write();
        if let Some(presence) = users.remove(&user_id) {
            allocator.release(presence.color);
        }
    }

    /// Update user's cursor (with throttling).
    pub fn update_cursor(&self, update: CursorUpdate) -> bool {
        let user_id = update.user_id;

        // Throttle check
        let should_send = {
            let mut last_send = self.last_cursor_send.write();
            match last_send.get(&user_id) {
                Some(last) if last.elapsed() < self.cursor_throttle => false,
                _ => {
                    last_send.insert(user_id, Instant::now());
                    true
                }
            }
        };

        if !should_send {
            return false;
        }

        let mut users = self.users.write();
        if let Some(presence) = users.get_mut(&user_id) {
            presence.update_cursor(update);
            true
        } else {
            false
        }
    }

    /// Update user's presence status.
    pub fn update_presence(&self, update: PresenceUpdate) {
        let mut users = self.users.write();
        if let Some(presence) = users.get_mut(&update.user_id) {
            if let Some(status) = update.status {
                presence.set_status(status);
            }
            if let Some(file_id) = update.current_file {
                presence.set_file(Some(file_id));
            }
            if let Some(cursor) = update.cursor {
                presence.update_cursor(cursor);
            }
        }
    }

    /// Get user's presence state.
    pub fn get_user(&self, user_id: UserId) -> Option<PresenceState> {
        self.users.read().get(&user_id).cloned()
    }

    /// Get all users in the cube.
    pub fn all_users(&self) -> Vec<PresenceState> {
        self.users.read().values().cloned().collect()
    }

    /// Get users currently viewing a file.
    pub fn users_in_file(&self, file_id: crate::FileId) -> Vec<PresenceState> {
        self.users
            .read()
            .values()
            .filter(|p| p.current_file == Some(file_id))
            .cloned()
            .collect()
    }

    /// Get active user count.
    pub fn active_count(&self) -> usize {
        self.users
            .read()
            .values()
            .filter(|p| p.status == PresenceStatus::Active)
            .count()
    }

    /// Set user's current file (for open/close file).
    pub fn set_user_file(&self, user_id: UserId, file_id: Option<crate::FileId>) {
        let mut users = self.users.write();
        if let Some(presence) = users.get_mut(&user_id) {
            presence.set_file(file_id);
        }
    }

    /// Update user's presence status.
    pub fn set_user_status(&self, user_id: UserId, status: PresenceStatus) {
        let mut users = self.users.write();
        if let Some(presence) = users.get_mut(&user_id) {
            presence.set_status(status);
        }
    }
}

/// Simple color allocator for user colors.
struct ColorAllocator {
    available: Vec<UserColor>,
    allocated: HashMap<UserColor, u8>,
}

impl ColorAllocator {
    fn new() -> Self {
        Self {
            available: UserColor::PALETTE.iter().map(|&c| UserColor(c)).collect(),
            allocated: HashMap::new(),
        }
    }

    fn allocate(&mut self) -> UserColor {
        if let Some(color) = self.available.pop() {
            *self.allocated.entry(color).or_insert(0) += 1;
            color
        } else {
            // Fallback: reuse first color
            UserColor::from_index(0)
        }
    }

    fn release(&mut self, color: UserColor) {
        if let Some(count) = self.allocated.get_mut(&color) {
            *count -= 1;
            if *count == 0 {
                self.allocated.remove(&color);
                self.available.push(color);
            }
        }
    }
}
