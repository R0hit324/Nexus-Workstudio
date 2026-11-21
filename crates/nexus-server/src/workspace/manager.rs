use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nexus_core::cube::CubeDescriptor;
use nexus_core::ids::{CubeId, FileId, WorkspaceId};
use nexus_core::timestamp::Timestamp;
use nexus_core::version::Version;
use nexus_core::workspace::{DirectoryEntry, FileEntry, WorkspaceMetadata};

use super::paths::validate_workspace_path;
use crate::storage::{list_all_files, EntryKind, Storage};
use nexus_core::error::Error;

/// Storage namespace for a Cube's workspace: `cubes/<cube-id>/workspace`.
fn storage_root_for(cube_id: CubeId) -> String {
    format!("cubes/{cube_id}/workspace")
}

/// A Cube's authoritative workspace, backed by the shared [`Storage`].
///
/// Every mutation bumps the workspace version so clients can detect
/// divergence (full CRDT/OT reconciliation comes in a later part). Paths are
/// validated here and defensively again at the storage layer.
pub struct CubeWorkspace {
    cube_id: CubeId,
    workspace_id: WorkspaceId,
    storage: Arc<dyn Storage>,
    root: String,
    version: AtomicU64,
    created_at: Timestamp,
    updated_at: Mutex<Timestamp>,
}

impl CubeWorkspace {
    pub fn new(cube_id: CubeId, workspace_id: WorkspaceId, storage: Arc<dyn Storage>) -> Self {
        Self {
            cube_id,
            workspace_id,
            storage,
            root: storage_root_for(cube_id),
            version: AtomicU64::new(1),
            created_at: Timestamp::now(),
            updated_at: Mutex::new(Timestamp::now()),
        }
    }

    /// Records a mutation: bump the version and refresh the updated timestamp.
    fn bumped(&self) {
        self.version.fetch_add(1, Ordering::Relaxed);
        *self.updated_at.lock().unwrap_or_else(|p| p.into_inner()) = Timestamp::now();
    }

    fn touched(&self) -> Timestamp {
        *self.updated_at.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Maps a workspace-relative path onto its storage key.
    fn key(&self, relative: &str) -> nexus_core::Result<String> {
        if relative.is_empty() {
            return Err(Error::invalid_path("path must be non-empty"));
        }
        Ok(format!("{}/{}", self.root, relative))
    }

    /// Current workspace version without recomputing counts.
    #[must_use]
    pub fn version(&self) -> Version {
        Version::new(self.version.load(Ordering::Relaxed) as u32)
    }

    fn strip(&self, key: &str) -> String {
        key.strip_prefix(&format!("{}/", self.root))
            .unwrap_or(key)
            .to_owned()
    }

    /// Walks the store to recompute counts without bumping the version.
    async fn recount(&self) -> nexus_core::Result<(u64, u64)> {
        let mut file_count = 0u64;
        let mut directory_count = 0u64;
        let mut frontier = vec![self.root.clone()];
        while let Some(dir) = frontier.pop() {
            for entry in self.storage.list_entries(&dir).await? {
                match entry.kind {
                    EntryKind::File => file_count += 1,
                    EntryKind::Directory => {
                        directory_count += 1;
                        frontier.push(entry.path);
                    }
                }
            }
        }
        Ok((file_count, directory_count))
    }
}

#[async_trait::async_trait]
impl nexus_core::workspace::Workspace for CubeWorkspace {
    async fn metadata(&self) -> nexus_core::Result<WorkspaceMetadata> {
        let (file_count, directory_count) = self.recount().await?;
        Ok(WorkspaceMetadata {
            id: self.workspace_id,
            cube_id: self.cube_id,
            name: format!("workspace-{}", self.cube_id),
            version: Version::new(self.version.load(Ordering::Relaxed) as u32),
            file_count,
            directory_count,
            created_at: self.created_at,
            updated_at: self.touched(),
        })
    }

    async fn list_files(&self) -> nexus_core::Result<Vec<FileEntry>> {
        let mut entries = Vec::new();
        for path in list_all_files(self.storage.as_ref(), &self.root).await? {
            let data = self.storage.read_file(&path).await?;
            entries.push(FileEntry {
                id: FileId::new(),
                path: self.strip(&path),
                size: data.len() as u64,
                modified_at: self.touched(),
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn list_directories(&self) -> nexus_core::Result<Vec<DirectoryEntry>> {
        let mut directories = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut frontier = vec![self.root.clone()];
        while let Some(dir) = frontier.pop() {
            for entry in self.storage.list_entries(&dir).await? {
                if entry.kind != EntryKind::Directory || !seen.insert(entry.path.clone()) {
                    continue;
                }
                let relative = self.strip(&entry.path);
                if !relative.is_empty() {
                    directories.push(DirectoryEntry { path: relative });
                }
                frontier.push(entry.path);
            }
        }
        directories.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(directories)
    }

    async fn create_file(&self, path: &str) -> nexus_core::Result<()> {
        validate_workspace_path(path)?;
        let key = self.key(path)?;
        if self.storage.file_exists(&key).await? {
            return Err(Error::already_exists(format!(
                "file already exists: {path}"
            )));
        }
        self.storage.write_file(&key, b"").await?;
        self.bumped();
        Ok(())
    }

    async fn delete_file(&self, path: &str) -> nexus_core::Result<()> {
        validate_workspace_path(path)?;
        let key = self.key(path)?;
        self.storage.delete_file(&key).await?;
        self.bumped();
        Ok(())
    }

    async fn rename_file(&self, from: &str, to: &str) -> nexus_core::Result<()> {
        validate_workspace_path(from)?;
        validate_workspace_path(to)?;
        let from_key = self.key(from)?;
        let to_key = self.key(to)?;
        if !self.storage.file_exists(&from_key).await? {
            return Err(Error::not_found(format!("no such file: {from}")));
        }
        if self.storage.file_exists(&to_key).await? {
            return Err(Error::already_exists(format!(
                "destination already exists: {to}"
            )));
        }
        let data = self.storage.read_file(&from_key).await?;
        self.storage.write_file(&to_key, &data).await?;
        self.storage.delete_file(&from_key).await?;
        self.bumped();
        Ok(())
    }

    async fn create_directory(&self, path: &str) -> nexus_core::Result<()> {
        validate_workspace_path(path)?;
        let key = self.key(path)?;
        self.storage.create_dir_all(&key).await?;
        self.bumped();
        Ok(())
    }

    async fn delete_directory(&self, path: &str) -> nexus_core::Result<()> {
        validate_workspace_path(path)?;
        let key = self.key(path)?;
        let children = self.storage.list_entries(&key).await?;
        if !children.is_empty() {
            return Err(Error::workspace(format!(
                "directory not empty ({} entries)",
                children.len()
            )));
        }
        self.storage.remove_dir_all(&key).await?;
        self.bumped();
        Ok(())
    }

    async fn read_file(&self, path: &str) -> nexus_core::Result<Vec<u8>> {
        validate_workspace_path(path)?;
        let key = self.key(path)?;
        self.storage.read_file(&key).await
    }

    async fn write_file(&self, path: &str, content: &[u8]) -> nexus_core::Result<()> {
        validate_workspace_path(path)?;
        let key = self.key(path)?;
        self.storage.write_file(&key, content).await?;
        self.bumped();
        Ok(())
    }
}

/// Registry of per-Cube workspaces. Workspaces are attached once when a Cube
/// is created and shared across all of its sessions.
pub struct WorkspaceManager {
    workspaces: Mutex<HashMap<CubeId, Arc<CubeWorkspace>>>,
    storage: Arc<dyn Storage>,
}

impl WorkspaceManager {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            workspaces: Mutex::new(HashMap::new()),
            storage,
        }
    }

    /// Attaches a workspace for `cube`, creating the backing layout lazily on
    /// first mutation. Idempotent.
    pub fn attach(&self, cube: &CubeDescriptor) -> Arc<CubeWorkspace> {
        let workspace_id = cube.workspace_id.unwrap_or_else(WorkspaceId::new);
        self.workspaces
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(cube.id)
            .or_insert_with(|| {
                Arc::new(CubeWorkspace::new(
                    cube.id,
                    workspace_id,
                    self.storage.clone(),
                ))
            })
            .clone()
    }

    /// Resolves the workspace for a Cube if one has been attached.
    pub fn get(&self, cube_id: CubeId) -> Option<Arc<CubeWorkspace>> {
        self.workspaces
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&cube_id)
            .cloned()
    }

    /// Detaches and forgets a Cube's workspace (used when a Cube is destroyed).
    pub fn detach(&self, cube_id: CubeId) {
        self.workspaces
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&cube_id);
    }

    pub fn len(&self) -> usize {
        self.workspaces
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::InMemoryStorage;
    use nexus_core::error::ErrorCode;
    use nexus_core::workspace::Workspace;

    fn cube(cube_id: CubeId) -> CubeDescriptor {
        CubeDescriptor::new(cube_id, "demo", nexus_core::ids::UserId::new())
    }

    #[tokio::test]
    async fn workspace_crud_and_versioning() {
        let storage = Arc::new(InMemoryStorage::new());
        let managers = WorkspaceManager::new(storage);
        let workspace = managers.attach(&cube(CubeId::new()));

        workspace.create_file("readme.md").await.unwrap();
        workspace.write_file("readme.md", b"hello").await.unwrap();
        assert_eq!(workspace.read_file("readme.md").await.unwrap(), b"hello");
        assert_eq!(workspace.metadata().await.unwrap().file_count, 1);
        assert_eq!(workspace.metadata().await.unwrap().version.get(), 3);

        workspace
            .rename_file("readme.md", "README.md")
            .await
            .unwrap();
        assert!(workspace.read_file("readme.md").await.is_err());
        assert_eq!(workspace.read_file("README.md").await.unwrap(), b"hello");
        assert_eq!(workspace.metadata().await.unwrap().file_count, 1);

        workspace.delete_file("README.md").await.unwrap();
        assert_eq!(workspace.metadata().await.unwrap().file_count, 0);
    }

    #[tokio::test]
    async fn duplicate_creation_is_rejected() {
        let storage = Arc::new(InMemoryStorage::new());
        let managers = WorkspaceManager::new(storage);
        let workspace = managers.attach(&cube(CubeId::new()));
        workspace.create_file("a.txt").await.unwrap();
        let err = workspace.create_file("a.txt").await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::AlreadyExists);
    }

    #[tokio::test]
    async fn path_escape_is_rejected() {
        let storage = Arc::new(InMemoryStorage::new());
        let managers = WorkspaceManager::new(storage);
        let workspace = managers.attach(&cube(CubeId::new()));
        let err = workspace.write_file("../escape", b"x").await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidPath);
    }

    #[tokio::test]
    async fn registry_attach_is_idempotent() {
        let storage = Arc::new(InMemoryStorage::new());
        let managers = WorkspaceManager::new(storage);
        let cube_id = CubeId::new();
        let first = managers.attach(&cube(cube_id));
        let second = managers.attach(&cube(cube_id));
        assert_eq!(Arc::as_ptr(&first), Arc::as_ptr(&second));
        assert_eq!(managers.len(), 1);
    }

    #[tokio::test]
    async fn metadata_counts_directories() {
        let storage = Arc::new(InMemoryStorage::new());
        let managers = WorkspaceManager::new(storage);
        let workspace = managers.attach(&cube(CubeId::new()));
        workspace.create_file("src/lib.rs").await.unwrap();
        workspace.create_file("src/main.rs").await.unwrap();
        workspace.create_directory("docs").await.unwrap();
        let meta = workspace.metadata().await.unwrap();
        assert_eq!(meta.file_count, 2);
        assert_eq!(meta.directory_count, 2); // src and docs
    }
}
