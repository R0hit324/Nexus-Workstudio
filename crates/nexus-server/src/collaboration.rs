use std::collections::HashMap;
use std::sync::Arc;

use nexus_core::collaboration::{
    ActivityEvent, ActivityEventType, BuildActivity, ChatMessage, Collaborator, CursorPosition,
    FollowState, PairSession, PresenceStatus, Selection, TerminalActivity, User,
};
use nexus_core::ids::{CubeId, FileId, SessionId, UserId};
use nexus_core::permission::Permission;
use nexus_core::timestamp::Timestamp;
use nexus_events::bus::EventBus;
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tracing::debug;

use crate::connection::ConnectionManager;
use crate::permissions::PermissionManager;
use crate::session::SessionManager;
use nexus_core::cube::CubeManager;

/// Manages collaboration state for a Cube.
pub struct CubeCollaborationState {
    /// Active collaborators in this cube.
    collaborators: RwLock<HashMap<UserId, Collaborator>>,
    /// User colors for deterministic UI identity.
    user_colors: RwLock<HashMap<UserId, u32>>,
    /// Active file awareness.
    active_files: RwLock<HashMap<UserId, Option<FileId>>>,
    /// Remote cursors.
    cursors: RwLock<HashMap<UserId, CursorPosition>>,
    /// Remote selections.
    selections: RwLock<HashMap<UserId, Selection>>,
    /// Follow mode state.
    follow_state: RwLock<HashMap<UserId, FollowState>>,
    /// Pair programming sessions.
    pair_sessions: RwLock<HashMap<u64, PairSession>>,
    /// Chat history (bounded).
    chat_history: RwLock<Vec<ChatMessage>>,
    /// Activity feed (bounded).
    activity_feed: RwLock<Vec<ActivityEvent>>,
    /// Build activity.
    build_activity: RwLock<HashMap<UserId, BuildActivity>>,
    /// Terminal activity.
    terminal_activity: RwLock<HashMap<u64, TerminalActivity>>,
    /// Broadcast channel for ephemeral events.
    ephemeral_tx: broadcast::Sender<CollabEphemeralEvent>,
    /// Maximum chat history size.
    max_chat_history: usize,
    /// Maximum activity feed size.
    max_activity_feed: usize,
}

/// Ephemeral collaboration events that don't need persistence.
#[derive(Debug, Clone)]
pub enum CollabEphemeralEvent {
    PresenceUpdate(Collaborator),
    CursorUpdate(CursorPosition),
    SelectionUpdate(Selection),
    ActiveFileChanged {
        user_id: UserId,
        file_id: Option<FileId>,
    },
    TypingIndicator {
        user_id: UserId,
        file_id: FileId,
        is_typing: bool,
    },
    BuildActivityUpdate(BuildActivity),
    TerminalActivityUpdate(TerminalActivity),
}

impl CubeCollaborationState {
    pub fn new() -> Self {
        let (ephemeral_tx, _ephemeral_rx) = broadcast::channel(1024);
        Self {
            collaborators: RwLock::new(HashMap::new()),
            user_colors: RwLock::new(HashMap::new()),
            active_files: RwLock::new(HashMap::new()),
            cursors: RwLock::new(HashMap::new()),
            selections: RwLock::new(HashMap::new()),
            follow_state: RwLock::new(HashMap::new()),
            pair_sessions: RwLock::new(HashMap::new()),
            chat_history: RwLock::new(Vec::new()),
            activity_feed: RwLock::new(Vec::new()),
            build_activity: RwLock::new(HashMap::new()),
            terminal_activity: RwLock::new(HashMap::new()),
            ephemeral_tx,
            max_chat_history: 1000,
            max_activity_feed: 500,
        }
    }

    /// Gets or generates a deterministic color for a user.
    pub fn get_user_color(&self, user_id: UserId) -> u32 {
        let mut colors = self.user_colors.write();
        *colors.entry(user_id).or_insert_with(|| {
            let hash = user_id.as_uuid().as_u128();
            let r = ((hash >> 16) & 0xFF) as u32;
            let g = ((hash >> 8) & 0xFF) as u32;
            let b = (hash & 0xFF) as u32;
            // Ensure good contrast
            ((r.saturating_add(128) % 256) << 16)
                | ((g.saturating_add(128) % 256) << 8)
                | (b.saturating_add(128) % 256)
        })
    }

    /// Adds a user to the cube.
    pub fn add_user(&self, user: User, _session_id: SessionId) -> Collaborator {
        let user_id = user.id;
        let color = self.get_user_color(user_id);
        let mut collab = Collaborator::from_user(user);
        collab.color = color;

        self.collaborators.write().insert(user_id, collab.clone());
        self.active_files.write().insert(user_id, None);
        self.follow_state
            .write()
            .insert(user_id, FollowState::Normal);

        debug!(user_id = %user_id, name = %collab.display_name, "User joined cube");

        // Emit presence update
        let _ = self
            .ephemeral_tx
            .send(CollabEphemeralEvent::PresenceUpdate(collab.clone()));

        collab
    }

    /// Removes a user from the cube.
    pub fn remove_user(&self, user_id: UserId) {
        self.collaborators.write().remove(&user_id);
        self.active_files.write().remove(&user_id);
        self.cursors.write().remove(&user_id);
        self.selections.write().remove(&user_id);
        self.follow_state.write().remove(&user_id);
        self.build_activity.write().remove(&user_id);

        debug!(%user_id, "User left cube");
    }

    /// Updates a user's presence status.
    pub fn update_presence(&self, user_id: UserId, status: PresenceStatus) {
        if let Some(collab) = self.collaborators.write().get_mut(&user_id) {
            collab.status = status;
            let _ = self
                .ephemeral_tx
                .send(CollabEphemeralEvent::PresenceUpdate(collab.clone()));
        }
    }

    /// Updates a user's active file.
    pub fn update_active_file(&self, user_id: UserId, file_id: Option<FileId>) {
        self.active_files.write().insert(user_id, file_id);
        if let Some(collab) = self.collaborators.write().get_mut(&user_id) {
            collab.active_file = file_id;
            let _ = self
                .ephemeral_tx
                .send(CollabEphemeralEvent::ActiveFileChanged { user_id, file_id });
        }
    }

    /// Updates a user's cursor position.
    pub fn update_cursor(&self, cursor: CursorPosition) {
        self.cursors.write().insert(cursor.user_id, cursor.clone());
        if let Some(collab) = self.collaborators.write().get_mut(&cursor.user_id) {
            collab.cursor = Some(cursor.clone());
        }
        let _ = self
            .ephemeral_tx
            .send(CollabEphemeralEvent::CursorUpdate(cursor));
    }

    /// Updates a user's selection.
    pub fn update_selection(&self, selection: Selection) {
        self.selections
            .write()
            .insert(selection.user_id, selection.clone());
        if let Some(collab) = self.collaborators.write().get_mut(&selection.user_id) {
            collab.selection = Some(selection.clone());
        }
        let _ = self
            .ephemeral_tx
            .send(CollabEphemeralEvent::SelectionUpdate(selection));
    }

    /// Gets all collaborators.
    pub fn get_collaborators(&self) -> Vec<Collaborator> {
        self.collaborators.read().values().cloned().collect()
    }

    /// Gets a specific collaborator.
    pub fn get_collaborator(&self, user_id: UserId) -> Option<Collaborator> {
        self.collaborators.read().get(&user_id).cloned()
    }

    /// Sets follow mode state.
    pub fn set_follow_state(&self, user_id: UserId, state: FollowState) {
        self.follow_state.write().insert(user_id, state);
    }

    /// Gets follow mode state.
    pub fn get_follow_state(&self, user_id: UserId) -> FollowState {
        self.follow_state
            .read()
            .get(&user_id)
            .copied()
            .unwrap_or(FollowState::Normal)
    }

    /// Creates a pair programming session.
    pub fn create_pair_session(
        &self,
        cube_id: CubeId,
        driver: UserId,
        navigator: UserId,
    ) -> PairSession {
        let session = PairSession {
            id: rand::random::<u64>(),
            cube_id,
            driver,
            navigator,
            created_at: Timestamp::now(),
        };
        self.pair_sessions
            .write()
            .insert(session.id, session.clone());
        session
    }

    /// Ends a pair programming session.
    pub fn end_pair_session(&self, session_id: u64) {
        self.pair_sessions.write().remove(&session_id);
    }

    /// Gets all pair sessions.
    pub fn get_pair_sessions(&self) -> Vec<PairSession> {
        self.pair_sessions.read().values().cloned().collect()
    }

    /// Adds a chat message.
    pub fn add_chat_message(&self, message: ChatMessage) {
        let mut history = self.chat_history.write();
        history.push(message);
        if history.len() > self.max_chat_history {
            let drain_end = history.len();
            let drain_start = drain_end - self.max_chat_history;
            history.drain(0..drain_start);
        }
    }

    /// Gets recent chat history.
    pub fn get_chat_history(&self, limit: usize) -> Vec<ChatMessage> {
        let history = self.chat_history.read();
        history.iter().rev().take(limit).cloned().collect()
    }

    /// Adds an activity event.
    pub fn add_activity_event(&self, event: ActivityEvent) {
        let mut feed = self.activity_feed.write();
        feed.push(event);
        if feed.len() > self.max_activity_feed {
            let drain_end = feed.len();
            let drain_start = drain_end - self.max_activity_feed;
            feed.drain(0..drain_start);
        }
    }

    /// Gets recent activity feed.
    pub fn get_activity_feed(&self, limit: usize) -> Vec<ActivityEvent> {
        let feed = self.activity_feed.read();
        feed.iter().rev().take(limit).cloned().collect()
    }

    /// Updates build activity.
    pub fn update_build_activity(&self, activity: BuildActivity) {
        self.build_activity
            .write()
            .insert(activity.user_id, activity.clone());
        let _ = self
            .ephemeral_tx
            .send(CollabEphemeralEvent::BuildActivityUpdate(activity));
    }

    /// Gets build activity for a user.
    pub fn get_build_activity(&self, user_id: UserId) -> Option<BuildActivity> {
        self.build_activity.read().get(&user_id).cloned()
    }

    /// Updates terminal activity.
    pub fn update_terminal_activity(&self, activity: TerminalActivity) {
        self.terminal_activity
            .write()
            .insert(activity.terminal_id, activity.clone());
        let _ = self
            .ephemeral_tx
            .send(CollabEphemeralEvent::TerminalActivityUpdate(activity));
    }

    /// Subscribes to ephemeral events.
    pub fn subscribe_ephemeral(&self) -> broadcast::Receiver<CollabEphemeralEvent> {
        self.ephemeral_tx.subscribe()
    }
}

/// Server-side collaboration manager.
pub struct CollaborationManager {
    /// Collaboration state per cube.
    cubes: RwLock<HashMap<CubeId, Arc<CubeCollaborationState>>>,
    /// Session manager for user lookups.
    session_manager: Arc<SessionManager>,
    /// Connection manager for broadcasting.
    #[allow(dead_code)]
    connection_manager: Arc<ConnectionManager>,
    /// Permission manager for authorization.
    permission_manager: Arc<PermissionManager>,
    /// Cube manager for permission lookups.
    cube_manager: Arc<dyn CubeManager>,
    /// Event bus for durable events.
    #[allow(dead_code)]
    event_bus: Arc<EventBus>,
}

impl CollaborationManager {
    pub fn new(
        session_manager: Arc<SessionManager>,
        connection_manager: Arc<ConnectionManager>,
        permission_manager: Arc<PermissionManager>,
        cube_manager: Arc<dyn CubeManager>,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self {
            cubes: RwLock::new(HashMap::new()),
            session_manager,
            connection_manager,
            permission_manager,
            cube_manager,
            event_bus,
        }
    }

    /// Gets or creates the collaboration state for a cube.
    fn get_or_create_cube_state(&self, cube_id: CubeId) -> Arc<CubeCollaborationState> {
        let mut cubes = self.cubes.write();
        cubes
            .entry(cube_id)
            .or_insert_with(|| Arc::new(CubeCollaborationState::new()))
            .clone()
    }

    /// Handles a user joining a cube.
    pub async fn handle_user_joined(
        &self,
        cube_id: CubeId,
        user_id: UserId,
        session_id: SessionId,
    ) -> Collaborator {
        // Get user info from session
        let session = self
            .session_manager
            .get(session_id)
            .expect("session should exist");

        // Get permission from permission manager
        let permission = self
            .permission_manager
            .get_user_permission(&self.cube_manager, cube_id, user_id)
            .await;

        let user = User::new(user_id, session.username().to_string(), permission);
        let cube_state = self.get_or_create_cube_state(cube_id);
        let collaborator = cube_state.add_user(user, session_id);

        // Broadcast to other users in the cube
        self.broadcast_to_cube(cube_id, Some(session_id)).await;

        // Add activity event
        cube_state.add_activity_event(ActivityEvent {
            id: rand::random::<u64>(),
            cube_id,
            user_id,
            user_name: collaborator.display_name.clone(),
            event_type: ActivityEventType::UserJoined,
            timestamp: Timestamp::now(),
            details: format!("{} joined the cube", collaborator.display_name),
        });

        collaborator
    }

    /// Handles a user leaving a cube.
    pub async fn handle_user_left(&self, cube_id: CubeId, user_id: UserId, session_id: SessionId) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            if let Some(collab) = cube_state.get_collaborator(user_id) {
                let display_name = collab.display_name.clone();
                cube_state.remove_user(user_id);

                // Broadcast to other users
                self.broadcast_to_cube(cube_id, Some(session_id)).await;

                // Add activity event
                cube_state.add_activity_event(ActivityEvent {
                    id: rand::random::<u64>(),
                    cube_id,
                    user_id,
                    user_name: display_name.clone(),
                    event_type: ActivityEventType::UserLeft,
                    timestamp: Timestamp::now(),
                    details: format!("{} left the cube", display_name),
                });
            }
        }
    }

    /// Handles cursor update from a user.
    pub fn handle_cursor_update(&self, cube_id: CubeId, cursor: CursorPosition) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.update_cursor(cursor);
        }
    }

    /// Handles selection update from a user.
    pub fn handle_selection_update(&self, cube_id: CubeId, selection: Selection) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.update_selection(selection);
        }
    }

    /// Handles active file change.
    pub fn handle_active_file_changed(
        &self,
        cube_id: CubeId,
        user_id: UserId,
        file_id: Option<FileId>,
    ) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.update_active_file(user_id, file_id);
        }
    }
    /// Handles chat message.
    pub async fn handle_chat_message(&self, cube_id: CubeId, message: ChatMessage) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.add_chat_message(message.clone());

            // Broadcast to all users in cube
            self.broadcast_to_cube(cube_id, None).await;
        }
    }

    /// Handles follow mode change.
    pub fn handle_follow_mode_changed(
        &self,
        cube_id: CubeId,
        follower_id: UserId,
        _target_id: Option<UserId>,
        state: FollowState,
    ) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.set_follow_state(follower_id, state);

            // Broadcast to cube
            let _ = self.broadcast_to_cube(cube_id, None);
        }
    }

    /// Handles pair programming session.
    pub async fn handle_pair_session(&self, cube_id: CubeId, driver: UserId, navigator: UserId) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            let _session = cube_state.create_pair_session(cube_id, driver, navigator);

            // Broadcast to cube
            self.broadcast_to_cube(cube_id, None).await;
        }
    }

    /// Handles build activity update.
    pub fn handle_build_activity(&self, cube_id: CubeId, activity: BuildActivity) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.update_build_activity(activity.clone());

            // Broadcast to cube
            let _ = self.broadcast_to_cube(cube_id, None);
        }
    }

    /// Handles terminal activity update.
    pub fn handle_terminal_activity(&self, cube_id: CubeId, activity: TerminalActivity) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.update_terminal_activity(activity.clone());

            // Broadcast to cube
            let _ = self.broadcast_to_cube(cube_id, None);
        }
    }

    /// Handles user permission change.
    pub async fn handle_permission_changed(
        &self,
        cube_id: CubeId,
        user_id: UserId,
        _permission: Permission,
    ) {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            if let Some(_collab) = cube_state.collaborators.write().get_mut(&user_id) {
                // Note: Collaborator doesn't have permissions field, would need to add
            }

            // Broadcast to cube
            self.broadcast_to_cube(cube_id, None).await;
        }
    }

    /// Gets presence snapshot for a cube.
    pub fn get_presence_snapshot(&self, cube_id: CubeId) -> Vec<Collaborator> {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.get_collaborators()
        } else {
            Vec::new()
        }
    }

    /// Gets chat history for a cube.
    pub fn get_chat_history(&self, cube_id: CubeId, limit: usize) -> Vec<ChatMessage> {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.get_chat_history(limit)
        } else {
            Vec::new()
        }
    }

    /// Gets activity feed for a cube.
    pub fn get_activity_feed(&self, cube_id: CubeId, limit: usize) -> Vec<ActivityEvent> {
        if let Some(cube_state) = self.cubes.read().get(&cube_id) {
            cube_state.get_activity_feed(limit)
        } else {
            Vec::new()
        }
    }

    /// Broadcasts a message to all users in a cube (except excluded session).
    async fn broadcast_to_cube(&self, cube_id: CubeId, _exclude_session: Option<SessionId>) {
        // Implementation would iterate over connections bound to this cube
        // and send the message to each
        debug!(%cube_id, "Broadcasting to cube members");
    }
}

impl Default for CubeCollaborationState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::ids::FileId;

    #[test]
    fn test_user_color_deterministic() {
        let state = CubeCollaborationState::new();
        let user_id = UserId::from_u64(42);

        let color1 = state.get_user_color(user_id);
        let color2 = state.get_user_color(user_id);

        assert_eq!(color1, color2);
    }

    #[test]
    fn test_add_remove_user() {
        let state = CubeCollaborationState::new();
        let user = User::new(UserId::from_u64(1), "Alice".into(), Permission::Editor);
        let session_id = SessionId::from_u64(1);

        let collab = state.add_user(user, session_id);
        assert_eq!(collab.display_name, "Alice");

        let collaborators = state.get_collaborators();
        assert_eq!(collaborators.len(), 1);

        state.remove_user(UserId::from_u64(1));
        let collaborators = state.get_collaborators();
        assert_eq!(collaborators.len(), 0);
    }

    #[test]
    fn test_cursor_update() {
        let state = CubeCollaborationState::new();
        let user = User::new(UserId::from_u64(1), "Alice".into(), Permission::Editor);
        let session_id = SessionId::from_u64(1);

        state.add_user(user, session_id);

        let cursor = CursorPosition::new(FileId::from_u64(1), 100, UserId::from_u64(1));
        state.update_cursor(cursor.clone());

        let collab = state.get_collaborator(UserId::from_u64(1)).unwrap();
        assert_eq!(collab.cursor, Some(cursor));
    }

    #[test]
    fn test_chat_history() {
        let state = CubeCollaborationState::new();

        for i in 0..5 {
            state.add_chat_message(ChatMessage {
                id: i,
                cube_id: CubeId::from_u64(1),
                sender_id: UserId::from_u64(1),
                sender_name: "Alice".into(),
                content: format!("Message {}", i),
                timestamp: Timestamp::now(),
            });
        }

        let history = state.get_chat_history(3);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].content, "Message 4");
    }
}
