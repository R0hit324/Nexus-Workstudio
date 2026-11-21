use serde::{Deserialize, Serialize};

use crate::ids::{CubeId, RuntimeId, TerminalId, UserId};
use crate::timestamp::Timestamp;

/// Terminal visibility scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalVisibility {
    /// Only the owner can see this terminal.
    Private,
    /// All authorized Cube members can view.
    Shared,
    /// Build/output terminal.
    Build,
    /// Ephemeral task process.
    Task,
}

/// Terminal lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalState {
    Created,
    Starting,
    Running,
    Stopping,
    Stopped,
    Exited,
    Failed,
}

/// Terminal session metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalSession {
    pub terminal_id: TerminalId,
    pub cube_id: CubeId,
    pub owner: UserId,
    pub visibility: TerminalVisibility,
    pub runtime_id: RuntimeId,
    pub process_id: u32,
    pub shell: String,
    pub working_directory: String,
    pub size: TerminalSize,
    pub state: TerminalState,
    pub created_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub exited_at: Option<Timestamp>,
    pub exit_code: Option<i32>,
}

/// Terminal dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

/// Shared terminal input ownership state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputOwnership {
    Owner,
    Requested,
    Waiting,
}

/// Shared terminal control request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalControlRequest {
    pub terminal_id: TerminalId,
    pub requester: UserId,
    pub timestamp: Timestamp,
}

/// Terminal permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalPermission {
    ViewTerminal,
    CreateTerminal,
    SendInput,
    KillProcess,
    ShareTerminal,
    ManageSharedTerminal,
}

/// Terminal event types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TerminalEvent {
    Created {
        terminal_id: TerminalId,
        cube_id: CubeId,
        owner: UserId,
        visibility: TerminalVisibility,
        timestamp: Timestamp,
    },
    Started {
        terminal_id: TerminalId,
        timestamp: Timestamp,
    },
    Output {
        terminal_id: TerminalId,
        data: Vec<u8>,
        timestamp: Timestamp,
    },
    Input {
        terminal_id: TerminalId,
        data: Vec<u8>,
        user_id: UserId,
        timestamp: Timestamp,
    },
    Resized {
        terminal_id: TerminalId,
        size: TerminalSize,
        timestamp: Timestamp,
    },
    ControlChanged {
        terminal_id: TerminalId,
        new_owner: UserId,
        timestamp: Timestamp,
    },
    Exited {
        terminal_id: TerminalId,
        exit_code: i32,
        timestamp: Timestamp,
    },
    Closed {
        terminal_id: TerminalId,
        timestamp: Timestamp,
    },
}

impl TerminalEvent {
    pub fn terminal_id(&self) -> TerminalId {
        match self {
            Self::Created { terminal_id, .. } => *terminal_id,
            Self::Started { terminal_id, .. } => *terminal_id,
            Self::Output { terminal_id, .. } => *terminal_id,
            Self::Input { terminal_id, .. } => *terminal_id,
            Self::Resized { terminal_id, .. } => *terminal_id,
            Self::ControlChanged { terminal_id, .. } => *terminal_id,
            Self::Exited { terminal_id, .. } => *terminal_id,
            Self::Closed { terminal_id, .. } => *terminal_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_size_default() {
        let size = TerminalSize::default();
        assert_eq!(size.rows, 24);
        assert_eq!(size.cols, 80);
    }
}
