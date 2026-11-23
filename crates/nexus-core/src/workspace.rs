use serde::{Deserialize, Serialize};

use crate::ids::{CubeId, FileId, WorkspaceId};
use crate::timestamp::Timestamp;
use crate::version::Version;

/// Descriptive state of a workspace: the file tree owned by a Cube.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    pub id: WorkspaceId,
    pub cube_id: CubeId,
    pub name: String,
    pub version: Version,
    pub file_count: u64,
    pub directory_count: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A file entry as listed in a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: FileId,
    pub path: String,
    pub size: u64,
    pub modified_at: Timestamp,
}

/// A directory entry as listed in a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub path: String,
}

/// The workspace abstraction. Collaborative synchronization is implemented in
/// a later part; Part 1 defines the operations future engines will implement.
#[async_trait::async_trait]
pub trait Workspace: Send + Sync {
    async fn metadata(&self) -> crate::Result<WorkspaceMetadata>;
    async fn list_files(&self) -> crate::Result<Vec<FileEntry>>;
    async fn list_directories(&self) -> crate::Result<Vec<DirectoryEntry>>;
    async fn create_file(&self, path: &str) -> crate::Result<()>;
    async fn delete_file(&self, path: &str) -> crate::Result<()>;
    async fn rename_file(&self, from: &str, to: &str) -> crate::Result<()>;
    async fn create_directory(&self, path: &str) -> crate::Result<()>;
    async fn delete_directory(&self, path: &str) -> crate::Result<()>;
    async fn read_file(&self, path: &str) -> crate::Result<Vec<u8>>;
    async fn write_file(&self, path: &str, content: &[u8]) -> crate::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_serializes_through_json() {
        let meta = WorkspaceMetadata {
            id: WorkspaceId::new(),
            cube_id: CubeId::new(),
            name: "project".into(),
            version: Version::new(1),
            file_count: 3,
            directory_count: 1,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        };
        let encoded = serde_json::to_string(&meta).unwrap();
        let decoded: WorkspaceMetadata = serde_json::from_str(&encoded).unwrap();
        assert_eq!(meta, decoded);
        assert_eq!(decoded.file_count, 3);
    }

    #[test]
    fn file_entry_fields_roundtrip() {
        let entry = FileEntry {
            id: FileId::new(),
            path: "src/main.rs".into(),
            size: 128,
            modified_at: Timestamp::now(),
        };
        let encoded = serde_json::to_string(&entry).unwrap();
        let decoded: FileEntry = serde_json::from_str(&encoded).unwrap();
        assert_eq!(entry, decoded);
    }
}
