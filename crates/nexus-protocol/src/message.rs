use serde::{Deserialize, Serialize};

use nexus_core::collaboration::{
    ActivityEvent, BuildActivity, ChatMessage, Collaborator, CursorPosition, FollowState,
    PairSession, Selection, TerminalActivity,
};
use nexus_core::cube::CubeDescriptor;
use nexus_core::event::Event;
use nexus_core::ids::{
    BuildId, CubeId, FileId, RequestId, SessionId, SnapshotId, TerminalId, UserId,
};
use nexus_core::session::SessionDescriptor;
use nexus_core::timestamp::Timestamp;
use nexus_core::version::Version;
use nexus_core::{
    BuildArtifact, BuildConfiguration, BuildResult, BuildTarget, Diagnostic, TerminalSession,
    TerminalSize, TerminalState, TerminalVisibility,
};

/// A feature the client or server can support. The intersection of the two
/// sets determines the negotiated feature set for a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Real-time workspace synchronization.
    Synchronization,
    /// Message/event batch compression.
    Compression,
    /// Workspace snapshots.
    Snapshots,
    /// The plugin system.
    Plugins,
    /// The AI subsystem.
    Ai,
    /// Shared/private terminals.
    Terminals,
    /// The build system.
    Build,
    /// Isolated Cube runtimes.
    Runtime,
    /// Project chat.
    Chat,
    /// User presence (cursors, selection).
    Presence,
    /// Recovery and reconnection support.
    Recovery,
}

/// Stable alias for forward-compatibility wrappers.
pub type KnownCapability = Capability;

/// Error codes carried inside [`Message::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    AuthenticationFailed,
    PermissionDenied,
    NotFound,
    AlreadyExists,
    InvalidRequest,
    Unsupported,
    ProtocolViolation,
    /// Client and server protocol versions are incompatible.
    ProtocolMismatch,
    /// A file path was rejected (traversal, malformed, or out of range).
    InvalidPath,
    Timeout,
    RateLimited,
    InternalError,
    /// Recovery token is invalid or expired.
    RecoveryTokenInvalid,
    /// Session cannot be recovered.
    SessionNotRecoverable,
    /// Cube recovery failed.
    CubeRecoveryFailed,
    /// Snapshot is invalid or corrupted.
    SnapshotInvalid,
    /// Event gap detected during recovery.
    EventGapDetected,
    /// Operation reconciliation failed.
    ReconciliationFailed,
    /// Version compatibility check failed.
    VersionMismatch,
}

/// Session recovery status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoverySessionStatus {
    Active,
    Disconnected,
    Recoverable,
    Expired,
    Destroyed,
}

/// Synchronization strategy for recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncStrategy {
    NoSyncRequired,
    EventReplay,
    SnapshotPlusReplay,
    FullWorkspaceSync,
}

/// Event range for recovery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRange {
    pub from_sequence: nexus_core::event::EventSequence,
    pub to_sequence: nexus_core::event::EventSequence,
}

/// A pending operation awaiting reconciliation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingOperation {
    pub operation_id: RequestId,
    pub file_id: FileId,
    pub base_version: Version,
    pub payload: Vec<u8>,
    pub created_at: Timestamp,
    pub retry_count: u32,
}

/// Result of operation reconciliation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationReconciliationResult {
    pub operation_id: RequestId,
    pub status: OperationReconciliationStatus,
    pub transformed_operations: Option<Vec<crate::sync::TextOperation>>,
}

/// Operation reconciliation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationReconciliationStatus {
    Accepted,
    Rejected,
    Merged,
    NeedsManualResolution,
}

/// Recovery stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryStage {
    SessionRecovery,
    CubeRecovery,
    StateComparison,
    EventReplay,
    SnapshotTransfer,
    OperationReconciliation,
    Complete,
}

/// Recovery plan sent by server in response to RecoveryRequest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryPlan {
    pub request_id: RequestId,
    pub session_status: RecoverySessionStatus,
    pub sync_strategy: SyncStrategy,
    pub snapshot_id: Option<SnapshotId>,
    pub event_range: Option<EventRange>,
    pub requires_full_sync: bool,
}

/// Which output stream a [`Message::BuildOutput`] chunk belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildOutputStream {
    Stdout,
    Stderr,
}

/// Control signals deliverable to a terminal process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalSignalKind {
    /// Ctrl+C (SIGINT/CTRL_C_EVENT).
    Interrupt,
    /// Ctrl+\ (SIGQUIT).
    Quit,
    /// Graceful termination (SIGTERM).
    Terminate,
    /// Forceful kill (SIGKILL).
    Kill,
    /// Hangup (SIGHUP).
    Hangup,
}

/// The typed protocol envelope. Serialized with a compact binary format inside
/// length-prefixed frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// First message sent by a client on a fresh connection.
    ClientHello {
        client_name: String,
        protocol_version: Version,
        capabilities: Vec<Capability>,
    },
    /// Server's reply to a `ClientHello`.
    ServerHello {
        server_name: String,
        protocol_version: Version,
        capabilities: Vec<Capability>,
    },
    /// Client asks to authenticate a user.
    Authenticate {
        request_id: RequestId,
        username: String,
    },
    /// Server grants/denies authentication.
    AuthenticationResult {
        request_id: RequestId,
        session: SessionDescriptor,
    },
    /// Client asks to create a Cube.
    CreateCube { request_id: RequestId, name: String },
    /// Server confirms Cube creation.
    CubeCreated {
        request_id: RequestId,
        cube: CubeDescriptor,
    },
    /// Client asks for the visible Cube list.
    ListCubes { request_id: RequestId },
    /// Server responds with the Cube list.
    CubeList {
        request_id: RequestId,
        cubes: Vec<CubeDescriptor>,
    },
    /// Client joins a Cube.
    JoinCube {
        request_id: RequestId,
        cube_id: CubeId,
    },
    /// Server confirms the join.
    CubeJoined {
        request_id: RequestId,
        cube: CubeDescriptor,
    },
    /// Client leaves a Cube.
    LeaveCube {
        request_id: RequestId,
        cube_id: CubeId,
    },
    /// Server confirms the leave.
    CubeLeft {
        request_id: RequestId,
        cube_id: CubeId,
    },
    /// Client asks for the workspace snapshot of a Cube.
    ListWorkspace {
        request_id: RequestId,
        cube_id: CubeId,
    },
    /// Server responds with workspace metadata and the file listing.
    WorkspaceList {
        request_id: RequestId,
        cube_id: CubeId,
        workspace: nexus_core::workspace::WorkspaceMetadata,
        files: Vec<nexus_core::workspace::FileEntry>,
    },
    /// Client creates a file in a Cube workspace.
    CreateFile {
        request_id: RequestId,
        cube_id: CubeId,
        path: String,
    },
    /// Client reads a file from a Cube workspace.
    ReadFile {
        request_id: RequestId,
        cube_id: CubeId,
        path: String,
    },
    /// Server responds with the requested file contents.
    FileContent {
        request_id: RequestId,
        cube_id: CubeId,
        path: String,
        version: nexus_core::version::Version,
        content: Vec<u8>,
    },
    /// Client writes a file into a Cube workspace.
    WriteFile {
        request_id: RequestId,
        cube_id: CubeId,
        path: String,
        content: Vec<u8>,
    },
    /// Client deletes a file from a Cube workspace.
    DeleteFile {
        request_id: RequestId,
        cube_id: CubeId,
        path: String,
    },
    /// Client renames a file inside a Cube workspace.
    RenameFile {
        request_id: RequestId,
        cube_id: CubeId,
        from: String,
        to: String,
    },
    /// A single workspace/domain event. Broadcast fan-out arrives in Part 4.
    Event { event: Event },
    /// A batch of events.
    EventBatch { events: Vec<Event> },
    /// Generic acknowledgement for requests that need no richer reply.
    Ack {
        request_id: RequestId,
        ok: bool,
        message: Option<String>,
    },
    /// A protocol-level error.
    Error {
        /// Correlates the error to the originating request.
        request_id: Option<RequestId>,
        code: ErrorCode,
        message: String,
    },
    /// Client-originated keepalive.
    Heartbeat { at: Timestamp },
    /// Server acknowledgement of a heartbeat.
    HeartbeatAck { at: Timestamp },
    /// Clean connection termination.
    Disconnect { reason: String },

    // ========== Part 5: Real-Time Synchronization ==========
    /// Client requests to open a file for collaborative editing.
    OpenFile {
        request_id: RequestId,
        cube_id: CubeId,
        file_id: FileId,
        local_version: Version,
    },
    /// Server responds with file content and current version.
    OpenFileResponse {
        request_id: RequestId,
        file_id: FileId,
        version: Version,
        content: Vec<u8>,
        synchronized: bool,
    },

    /// Client submits an edit operation (CRDT-based).
    EditOperation {
        request_id: RequestId,
        operation: crate::sync::EditOperation,
    },
    /// Server acknowledges an edit operation.
    OperationAck {
        request_id: RequestId,
        operation_id: crate::sync::OperationId,
        version: Version,
        accepted: bool,
        rejection_reason: Option<String>,
        transformed_operations: Option<Vec<crate::sync::TextOperation>>,
    },

    /// Batch of edit operations for efficiency.
    OperationBatch {
        request_id: RequestId,
        cube_id: CubeId,
        operations: Vec<crate::sync::EditOperation>,
    },
    /// Batch acknowledgment.
    OperationBatchAck {
        request_id: RequestId,
        acks: Vec<crate::sync::OperationAck>,
    },

    /// Cursor position/update (transient, high-frequency).
    CursorUpdate {
        cube_id: CubeId,
        update: crate::sync::CursorUpdate,
    },

    /// Presence update (user status, active file).
    PresenceUpdate {
        cube_id: CubeId,
        update: crate::sync::PresenceUpdate,
    },

    /// Client requests delta synchronization from a version.
    RequestDelta {
        request_id: RequestId,
        file_id: FileId,
        from_version: Version,
    },
    /// Server responds with missing operations.
    DeltaResponse {
        request_id: RequestId,
        file_id: FileId,
        operations: Vec<crate::sync::EditOperation>,
    },

    /// Synchronization status notification.
    SyncStatus {
        cube_id: CubeId,
        file_id: Option<FileId>,
        status: crate::sync::SyncStatus,
        version: Version,
    },

    /// Client requests full resynchronization.
    ResyncRequest {
        request_id: RequestId,
        file_id: FileId,
    },

    // ========== Part 7: Reconnection and Recovery ==========
    /// Client requests recovery after reconnection.
    RecoveryRequest {
        request_id: RequestId,
        session_id: Option<SessionId>,
        cube_id: CubeId,
        client_sequence: nexus_core::event::EventSequence,
        workspace_version: Version,
        pending_operation_ids: Vec<RequestId>,
    },
    /// Server responds with the recovery plan.
    RecoveryPlan(RecoveryPlan),
    /// Client requests event replay.
    ReplayEvents {
        request_id: RequestId,
        cube_id: CubeId,
        from_sequence: nexus_core::event::EventSequence,
        to_sequence: Option<nexus_core::event::EventSequence>,
    },
    /// Server responds with events for replay.
    ReplayEventsResponse {
        request_id: RequestId,
        events: Vec<Event>,
        has_more: bool,
    },
    /// Client requests snapshot transfer.
    RequestSnapshot {
        request_id: RequestId,
        cube_id: CubeId,
        snapshot_id: SnapshotId,
        offset: u64,
        chunk_size: u32,
    },
    /// Server responds with snapshot chunk.
    SnapshotChunk {
        request_id: RequestId,
        snapshot_id: SnapshotId,
        offset: u64,
        data: Vec<u8>,
        is_final: bool,
    },
    /// Client acknowledges snapshot chunk receipt.
    SnapshotChunkAck {
        request_id: RequestId,
        snapshot_id: SnapshotId,
        offset: u64,
        received_size: u64,
    },
    /// Client submits pending operations for reconciliation.
    ReconcileOperations {
        request_id: RequestId,
        cube_id: CubeId,
        operations: Vec<PendingOperation>,
    },
    /// Server responds with reconciliation results.
    ReconcileOperationsResponse {
        request_id: RequestId,
        results: Vec<OperationReconciliationResult>,
    },
    /// Recovery progress notification.
    RecoveryProgress {
        cube_id: CubeId,
        stage: RecoveryStage,
        progress: u32,
        total: u32,
        message: String,
    },
    /// Recovery complete notification.
    RecoveryComplete {
        cube_id: CubeId,
        final_sequence: nexus_core::event::EventSequence,
        final_workspace_version: Version,
    },
    /// Recovery failed notification.
    RecoveryFailed {
        cube_id: CubeId,
        reason: String,
        recoverable: bool,
    },

    // ========== Part 8: Collaboration Engine ==========
    /// Presence update for a user in a Cube.
    CollabPresenceUpdate { cube_id: CubeId, user: Collaborator },
    /// Batch presence update for multiple users.
    CollabPresenceBatch {
        cube_id: CubeId,
        users: Vec<Collaborator>,
    },
    /// User joined a Cube.
    CollabUserJoinedCube { cube_id: CubeId, user: Collaborator },
    /// User left a Cube.
    CollabUserLeftCube { cube_id: CubeId, user_id: UserId },
    /// Cursor position update (high-frequency, ephemeral).
    CollabCursorUpdate {
        cube_id: CubeId,
        cursor: CursorPosition,
    },
    /// Selection update.
    CollabSelectionUpdate {
        cube_id: CubeId,
        selection: Selection,
    },
    /// Active file changed.
    CollabActiveFileChanged {
        cube_id: CubeId,
        user_id: UserId,
        file_id: Option<FileId>,
    },
    /// Chat message sent.
    CollabChatMessageSent {
        cube_id: CubeId,
        message: ChatMessage,
    },
    /// Chat history response.
    CollabChatHistory {
        cube_id: CubeId,
        messages: Vec<ChatMessage>,
    },
    /// Activity feed event.
    CollabActivityEvent {
        cube_id: CubeId,
        event: ActivityEvent,
    },
    /// Follow user mode change.
    CollabFollowModeChanged {
        cube_id: CubeId,
        follower_id: UserId,
        target_id: Option<UserId>,
        state: FollowState,
    },
    /// Pair programming session change.
    CollabPairSessionChanged {
        cube_id: CubeId,
        session: Option<PairSession>,
    },
    /// Build activity update.
    CollabBuildActivityUpdate {
        cube_id: CubeId,
        activity: BuildActivity,
    },
    /// Terminal activity update.
    CollabTerminalActivityUpdate {
        cube_id: CubeId,
        activity: TerminalActivity,
    },
    /// User permission changed.
    CollabUserPermissionChanged {
        cube_id: CubeId,
        user_id: UserId,
        permission: nexus_core::permission::Permission,
    },
    /// Request chat history.
    RequestChatHistory {
        request_id: RequestId,
        cube_id: CubeId,
        limit: u32,
    },
    /// Request activity feed.
    RequestActivityFeed {
        request_id: RequestId,
        cube_id: CubeId,
        limit: u32,
    },
    /// Request presence snapshot.
    CollabRequestPresenceSnapshot {
        request_id: RequestId,
        cube_id: CubeId,
    },
    /// Presence snapshot response.
    CollabPresenceSnapshot {
        request_id: RequestId,
        cube_id: CubeId,
        users: Vec<Collaborator>,
    },

    // ========== Part 10: Build System ==========
    /// Client submits a build to a Cube's queue.
    BuildSubmit {
        request_id: RequestId,
        cube_id: CubeId,
        target: BuildTarget,
        configuration: BuildConfiguration,
    },
    /// Server confirms the build was queued.
    BuildQueued {
        request_id: RequestId,
        build_id: BuildId,
        queue_position: u32,
    },
    /// Server notifies that a build started executing.
    BuildStarted { build_id: BuildId, cube_id: CubeId },
    /// Streaming build output.
    BuildOutput {
        build_id: BuildId,
        stream: BuildOutputStream,
        data: String,
    },
    /// Compiler diagnostics produced by a build.
    BuildDiagnostics {
        build_id: BuildId,
        diagnostics: Vec<Diagnostic>,
    },
    /// Server notifies that a build finished.
    BuildFinished {
        build_id: BuildId,
        result: BuildResult,
    },
    /// Client requests to cancel a running/queued build.
    BuildCancel {
        request_id: RequestId,
        build_id: BuildId,
    },
    /// Server confirms the cancellation.
    BuildCancelled {
        request_id: RequestId,
        build_id: BuildId,
    },
    /// Server publishes the artifacts produced by a build.
    BuildArtifacts {
        build_id: BuildId,
        cube_id: CubeId,
        artifacts: Vec<BuildArtifact>,
    },

    // ========== Part 10: Terminals ==========
    /// Client requests a new PTY-backed terminal in a Cube.
    TerminalCreate {
        request_id: RequestId,
        cube_id: CubeId,
        shell: Option<String>,
        working_directory: String,
        size: TerminalSize,
        visibility: TerminalVisibility,
    },
    /// Server confirms terminal creation.
    TerminalCreated {
        request_id: RequestId,
        session: TerminalSession,
    },
    /// Client writes bytes to the terminal stdin.
    TerminalWrite {
        terminal_id: TerminalId,
        data: Vec<u8>,
    },
    /// Server streams terminal output bytes.
    TerminalOutput {
        terminal_id: TerminalId,
        data: Vec<u8>,
    },
    /// Client resizes the terminal.
    TerminalResize {
        request_id: RequestId,
        terminal_id: TerminalId,
        size: TerminalSize,
    },
    /// Client sends a control signal to the terminal.
    TerminalSignal {
        request_id: RequestId,
        terminal_id: TerminalId,
        signal: TerminalSignalKind,
    },
    /// Client closes a terminal.
    TerminalClose {
        request_id: RequestId,
        terminal_id: TerminalId,
    },
    /// Server confirms the terminal closed.
    TerminalClosed {
        request_id: RequestId,
        terminal_id: TerminalId,
        exit_code: Option<i32>,
    },
    /// Server notifies a terminal state transition.
    TerminalStateChanged {
        terminal_id: TerminalId,
        state: TerminalState,
    },
    /// Client requests the terminal list of a Cube.
    ListTerminals {
        request_id: RequestId,
        cube_id: CubeId,
    },
    /// Server responds with the terminal list.
    TerminalList {
        request_id: RequestId,
        cube_id: CubeId,
        terminals: Vec<TerminalSession>,
    },
    /// Client asks for the plugin roster (Part 11).
    PluginStatusRequest { request_id: RequestId },
    /// Server responds with the per-plugin records.
    PluginStatus {
        request_id: RequestId,
        plugins: Vec<PluginRecordInfo>,
    },
}

/// One plugin's status as surfaced to clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginRecordInfo {
    /// Plugin id (reverse-DNS).
    pub id: String,
    /// Manifest name.
    pub name: String,
    /// Plugin version string.
    pub version: String,
    /// Lifecycle state name (snake case).
    pub state: String,
    /// Trust level name.
    pub trust: String,
    /// Last failure message, if any.
    pub error: Option<String>,
}

impl Message {
    /// Builds a `ClientHello` with the given name and capabilities.
    #[must_use]
    pub fn client_hello(client_name: &str, capabilities: Vec<Capability>) -> Self {
        Self::ClientHello {
            client_name: client_name.into(),
            protocol_version: crate::PROTOCOL_VERSION,
            capabilities,
        }
    }

    /// Builds a `ServerHello` with the given name and capabilities.
    #[must_use]
    pub fn server_hello(server_name: &str, capabilities: Vec<Capability>) -> Self {
        Self::ServerHello {
            server_name: server_name.into(),
            protocol_version: crate::PROTOCOL_VERSION,
            capabilities,
        }
    }

    /// Builds an `Error` correlated to a request (or `None` when no request
    /// exists, e.g. a handshake failure).
    #[must_use]
    pub fn error(
        request_id: Option<RequestId>,
        code: ErrorCode,
        message: impl Into<String>,
    ) -> Self {
        Self::Error {
            request_id,
            code,
            message: message.into(),
        }
    }

    /// Builds a protocol-mismatch error for handshake failures.
    #[must_use]
    pub fn protocol_mismatch(client_version: Version, server_version: Version) -> Self {
        Self::error(
            None,
            ErrorCode::ProtocolMismatch,
            format!(
                "incompatible protocol versions: client={client_version} server={server_version}"
            ),
        )
    }

    /// The `request_id` carried by this message, if any. Matches both reply
    /// variants (correlated to a request) and request variants that carry an
    /// id inside a larger operation.
    #[must_use]
    pub fn response_request_id(&self) -> Option<RequestId> {
        match self {
            Self::Error { request_id, .. } => *request_id,
            Self::Authenticate { request_id, .. } => Some(*request_id),
            Self::CreateCube { request_id, .. } => Some(*request_id),
            Self::ListCubes { request_id, .. } => Some(*request_id),
            Self::JoinCube { request_id, .. } => Some(*request_id),
            Self::LeaveCube { request_id, .. } => Some(*request_id),
            Self::ListWorkspace { request_id, .. } => Some(*request_id),
            Self::CreateFile { request_id, .. } => Some(*request_id),
            Self::ReadFile { request_id, .. } => Some(*request_id),
            Self::WriteFile { request_id, .. } => Some(*request_id),
            Self::DeleteFile { request_id, .. } => Some(*request_id),
            Self::RenameFile { request_id, .. } => Some(*request_id),
            Self::AuthenticationResult { request_id, .. } => Some(*request_id),
            Self::CubeCreated { request_id, .. } => Some(*request_id),
            Self::CubeList { request_id, .. } => Some(*request_id),
            Self::CubeJoined { request_id, .. } => Some(*request_id),
            Self::CubeLeft { request_id, .. } => Some(*request_id),
            Self::WorkspaceList { request_id, .. } => Some(*request_id),
            Self::FileContent { request_id, .. } => Some(*request_id),
            Self::Ack { request_id, .. } => Some(*request_id),
            Self::OpenFile { request_id, .. } => Some(*request_id),
            Self::OpenFileResponse { request_id, .. } => Some(*request_id),
            Self::EditOperation { request_id, .. } => Some(*request_id),
            Self::OperationAck { request_id, .. } => Some(*request_id),
            Self::OperationBatch { request_id, .. } => Some(*request_id),
            Self::OperationBatchAck { request_id, .. } => Some(*request_id),
            Self::RequestDelta { request_id, .. } => Some(*request_id),
            Self::DeltaResponse { request_id, .. } => Some(*request_id),
            Self::ResyncRequest { request_id, .. } => Some(*request_id),
            Self::RecoveryRequest { request_id, .. } => Some(*request_id),
            Self::RecoveryPlan(ref plan) => Some(plan.request_id),
            Self::ReplayEvents { request_id, .. } => Some(*request_id),
            Self::ReplayEventsResponse { request_id, .. } => Some(*request_id),
            Self::RequestSnapshot { request_id, .. } => Some(*request_id),
            Self::SnapshotChunk { request_id, .. } => Some(*request_id),
            Self::SnapshotChunkAck { request_id, .. } => Some(*request_id),
            Self::ReconcileOperations { request_id, .. } => Some(*request_id),
            Self::ReconcileOperationsResponse { request_id, .. } => Some(*request_id),
            Self::RequestChatHistory { request_id, .. } => Some(*request_id),
            Self::RequestActivityFeed { request_id, .. } => Some(*request_id),
            Self::CollabRequestPresenceSnapshot { request_id, .. } => Some(*request_id),
            Self::CollabPresenceSnapshot { request_id, .. } => Some(*request_id),
            Self::BuildSubmit { request_id, .. } => Some(*request_id),
            Self::BuildQueued { request_id, .. } => Some(*request_id),
            Self::BuildCancel { request_id, .. } => Some(*request_id),
            Self::BuildCancelled { request_id, .. } => Some(*request_id),
            Self::TerminalCreate { request_id, .. } => Some(*request_id),
            Self::TerminalCreated { request_id, .. } => Some(*request_id),
            Self::TerminalResize { request_id, .. } => Some(*request_id),
            Self::TerminalSignal { request_id, .. } => Some(*request_id),
            Self::TerminalClose { request_id, .. } => Some(*request_id),
            Self::TerminalClosed { request_id, .. } => Some(*request_id),
            Self::ListTerminals { request_id, .. } => Some(*request_id),
            Self::TerminalList { request_id, .. } => Some(*request_id),
            Self::PluginStatusRequest { request_id } => Some(*request_id),
            Self::PluginStatus { request_id, .. } => Some(*request_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::workspace::{FileEntry, WorkspaceMetadata};

    /// Returns a representative instance of every message variant.
    fn sample_messages() -> Vec<Message> {
        let cube = CubeDescriptor::new(CubeId::new(), "demo", nexus_core::ids::UserId::new());
        vec![
            Message::client_hello("test-client", vec![Capability::Presence, Capability::Chat]),
            Message::server_hello("test-server", vec![Capability::Snapshots]),
            Message::Authenticate {
                request_id: RequestId::new(),
                username: "alice".into(),
            },
            Message::AuthenticationResult {
                request_id: RequestId::new(),
                session: SessionDescriptor::new(nexus_core::ids::UserId::new(), "alice"),
            },
            Message::CreateCube {
                request_id: RequestId::new(),
                name: "demo".into(),
            },
            Message::CubeCreated {
                request_id: RequestId::new(),
                cube: cube.clone(),
            },
            Message::ListCubes {
                request_id: RequestId::new(),
            },
            Message::CubeList {
                request_id: RequestId::new(),
                cubes: vec![cube.clone()],
            },
            Message::JoinCube {
                request_id: RequestId::new(),
                cube_id: cube.id,
            },
            Message::CubeJoined {
                request_id: RequestId::new(),
                cube: cube.clone(),
            },
            Message::LeaveCube {
                request_id: RequestId::new(),
                cube_id: cube.id,
            },
            Message::CubeLeft {
                request_id: RequestId::new(),
                cube_id: cube.id,
            },
            Message::ListWorkspace {
                request_id: RequestId::new(),
                cube_id: cube.id,
            },
            Message::WorkspaceList {
                request_id: RequestId::new(),
                cube_id: cube.id,
                workspace: WorkspaceMetadata {
                    id: nexus_core::ids::WorkspaceId::new(),
                    cube_id: cube.id,
                    name: "demo".into(),
                    version: Version::new(1),
                    file_count: 1,
                    directory_count: 0,
                    created_at: Timestamp::now(),
                    updated_at: Timestamp::now(),
                },
                files: vec![FileEntry {
                    id: nexus_core::ids::FileId::new(),
                    path: "src/main.rs".into(),
                    size: 128,
                    modified_at: Timestamp::now(),
                }],
            },
            Message::CreateFile {
                request_id: RequestId::new(),
                cube_id: cube.id,
                path: "src/new.rs".into(),
            },
            Message::ReadFile {
                request_id: RequestId::new(),
                cube_id: cube.id,
                path: "src/main.rs".into(),
            },
            Message::FileContent {
                request_id: RequestId::new(),
                cube_id: cube.id,
                path: "src/main.rs".into(),
                version: Version::new(1),
                content: b"fn main() {}".to_vec(),
            },
            Message::WriteFile {
                request_id: RequestId::new(),
                cube_id: cube.id,
                path: "src/main.rs".into(),
                content: b"fn main() { println!(\"hi\"); }".to_vec(),
            },
            Message::DeleteFile {
                request_id: RequestId::new(),
                cube_id: cube.id,
                path: "src/new.rs".into(),
            },
            Message::RenameFile {
                request_id: RequestId::new(),
                cube_id: cube.id,
                from: "a.rs".into(),
                to: "b.rs".into(),
            },
            Message::Event {
                event: Event::new(
                    cube.id,
                    nexus_core::ids::UserId::new(),
                    nexus_core::event::EventPayload::ChatMessageSent { text: "hi".into() },
                ),
            },
            Message::EventBatch { events: vec![] },
            Message::Ack {
                request_id: RequestId::new(),
                ok: true,
                message: None,
            },
            Message::Error {
                request_id: Some(RequestId::new()),
                code: ErrorCode::NotFound,
                message: "missing".into(),
            },
            Message::Heartbeat {
                at: Timestamp::now(),
            },
            Message::HeartbeatAck {
                at: Timestamp::now(),
            },
            Message::Disconnect {
                reason: "bye".into(),
            },
            // ========== Part 10: Build ==========
            Message::BuildSubmit {
                request_id: RequestId::new(),
                cube_id: cube.id,
                target: nexus_core::BuildTarget {
                    target_type: nexus_core::BuildTargetType::Build,
                    profile: nexus_core::BuildProfile::Debug,
                    target_name: None,
                    working_directory: String::new(),
                    environment: vec![],
                    timeout_seconds: 600,
                    resource_limits: nexus_core::ResourceLimits::default(),
                },
                configuration: nexus_core::BuildConfiguration {
                    language: "rust".into(),
                    toolchain: None,
                    build_command: None,
                    test_command: None,
                    check_command: None,
                    lint_command: None,
                    format_command: None,
                    environment: vec![],
                    working_directory: String::new(),
                    timeout_seconds: 600,
                    resource_limits: nexus_core::ResourceLimits::default(),
                },
            },
            Message::BuildQueued {
                request_id: RequestId::new(),
                build_id: nexus_core::ids::BuildId::new(),
                queue_position: 0,
            },
            Message::BuildStarted {
                build_id: nexus_core::ids::BuildId::new(),
                cube_id: cube.id,
            },
            Message::BuildOutput {
                build_id: nexus_core::ids::BuildId::new(),
                stream: crate::message::BuildOutputStream::Stderr,
                data: "warning: unused variable".into(),
            },
            Message::BuildDiagnostics {
                build_id: nexus_core::ids::BuildId::new(),
                diagnostics: vec![nexus_core::Diagnostic {
                    file_id: None,
                    path: Some("src/main.rs".into()),
                    line: Some(10),
                    column: Some(5),
                    end_line: None,
                    end_column: None,
                    severity: nexus_core::DiagnosticSeverity::Error,
                    message: "expected `;`".into(),
                    code: Some("E0266".into()),
                    source: "rustc".into(),
                }],
            },
            Message::BuildFinished {
                build_id: nexus_core::ids::BuildId::new(),
                result: nexus_core::BuildResult {
                    build_id: nexus_core::ids::BuildId::new(),
                    state: nexus_core::BuildState::Succeeded,
                    exit_code: Some(0),
                    duration_ms: 1234,
                    started_at: Timestamp::now(),
                    finished_at: Some(Timestamp::now()),
                    diagnostics: vec![],
                    artifacts: vec![nexus_core::BuildArtifact {
                        artifact_id: nexus_core::ids::ArtifactId::new(),
                        path: "target/debug/app".into(),
                        size: 2048,
                        kind: nexus_core::ArtifactKind::Binary,
                        created_at: Timestamp::now(),
                    }],
                    resource_usage: nexus_core::ResourceUsage {
                        peak_memory_bytes: 4096,
                        cpu_time_ms: 900,
                        wall_time_ms: 1234,
                    },
                },
            },
            Message::BuildCancel {
                request_id: RequestId::new(),
                build_id: nexus_core::ids::BuildId::new(),
            },
            Message::BuildCancelled {
                request_id: RequestId::new(),
                build_id: nexus_core::ids::BuildId::new(),
            },
            Message::BuildArtifacts {
                build_id: nexus_core::ids::BuildId::new(),
                cube_id: cube.id,
                artifacts: vec![],
            },
            // ========== Part 10: Terminals ==========
            Message::TerminalCreate {
                request_id: RequestId::new(),
                cube_id: cube.id,
                shell: None,
                working_directory: "/".into(),
                size: nexus_core::TerminalSize::default(),
                visibility: nexus_core::TerminalVisibility::Shared,
            },
            Message::TerminalCreated {
                request_id: RequestId::new(),
                session: nexus_core::TerminalSession {
                    terminal_id: nexus_core::ids::TerminalId::new(),
                    cube_id: cube.id,
                    owner: nexus_core::ids::UserId::new(),
                    visibility: nexus_core::TerminalVisibility::Shared,
                    runtime_id: nexus_core::ids::RuntimeId::new(),
                    process_id: 42,
                    shell: "/bin/bash".into(),
                    working_directory: "/".into(),
                    size: nexus_core::TerminalSize::default(),
                    state: nexus_core::TerminalState::Running,
                    created_at: Timestamp::now(),
                    started_at: Some(Timestamp::now()),
                    exited_at: None,
                    exit_code: None,
                },
            },
            Message::TerminalWrite {
                terminal_id: nexus_core::ids::TerminalId::new(),
                data: b"ls\n".to_vec(),
            },
            Message::TerminalOutput {
                terminal_id: nexus_core::ids::TerminalId::new(),
                data: b"hello\n".to_vec(),
            },
            Message::TerminalResize {
                request_id: RequestId::new(),
                terminal_id: nexus_core::ids::TerminalId::new(),
                size: nexus_core::TerminalSize {
                    rows: 40,
                    cols: 120,
                },
            },
            Message::TerminalSignal {
                request_id: RequestId::new(),
                terminal_id: nexus_core::ids::TerminalId::new(),
                signal: crate::message::TerminalSignalKind::Interrupt,
            },
            Message::TerminalClose {
                request_id: RequestId::new(),
                terminal_id: nexus_core::ids::TerminalId::new(),
            },
            Message::TerminalClosed {
                request_id: RequestId::new(),
                terminal_id: nexus_core::ids::TerminalId::new(),
                exit_code: Some(0),
            },
            Message::TerminalStateChanged {
                terminal_id: nexus_core::ids::TerminalId::new(),
                state: nexus_core::TerminalState::Exited,
            },
            Message::ListTerminals {
                request_id: RequestId::new(),
                cube_id: cube.id,
            },
            Message::TerminalList {
                request_id: RequestId::new(),
                cube_id: cube.id,
                terminals: vec![],
            },
            Message::PluginStatusRequest {
                request_id: RequestId::new(),
            },
            Message::PluginStatus {
                request_id: RequestId::new(),
                plugins: vec![PluginRecordInfo {
                    id: String::from("org.nexus.git"),
                    name: String::from("Git Integration"),
                    version: String::from("1.0.0"),
                    state: String::from("enabled"),
                    trust: String::from("trusted"),
                    error: None,
                }],
            },
        ]
    }

    #[test]
    fn every_message_roundtrips_through_postcard() {
        for message in sample_messages() {
            let bytes = postcard::to_stdvec(&message).expect("encode");
            let decoded: Message = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(message, decoded);
        }
    }

    #[test]
    fn hello_messages_carry_the_current_version() {
        let hello = Message::client_hello("c", vec![]);
        match hello {
            Message::ClientHello {
                protocol_version, ..
            } => {
                assert_eq!(protocol_version, crate::PROTOCOL_VERSION);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
