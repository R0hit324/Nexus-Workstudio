//! Pluggable byte storage for workspaces.
//!
//! Keys are relative, POSIX-style paths (e.g. `cubes/<cube-id>/workspace/
//! docs/readme.md`). The trait is deliberately small: existence, read, write,
//! delete, dir creation, entry listing, and tree removal. Versioning and
//! collaborative semantics live one layer up in the workspace manager.

pub mod filesystem;
pub mod memory;

pub use filesystem::FilesystemStorage;
pub use memory::InMemoryStorage;

use nexus_core::timestamp::Timestamp;

/// Kind of a storage entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

/// A single entry returned by [`Storage::list_entries`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageEntry {
    /// Fully-qualified key, relative to the storage root.
    pub path: String,
    pub kind: EntryKind,
    /// Byte size (`0` for directories).
    pub size: u64,
    pub modified_at: Timestamp,
}

impl StorageEntry {
    #[must_use]
    pub const fn file(path: String, size: u64, modified_at: Timestamp) -> Self {
        Self {
            path,
            kind: EntryKind::File,
            size,
            modified_at,
        }
    }

    #[must_use]
    pub const fn directory(path: String, modified_at: Timestamp) -> Self {
        Self {
            path,
            kind: EntryKind::Directory,
            size: 0,
            modified_at,
        }
    }
}

/// Validates that a storage key is non-empty and does not escape its root with
/// `..` segments. Backslashes are normalized onto forward slashes first.
///
/// Also rejects Windows drive letters (`C:`), NUL, and control characters so
/// a key can never be interpreted as an absolute or verbatim path on the
/// host filesystem.
#[must_use]
pub fn is_safe_relative_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') {
        return false;
    }
    normalized.split('/').all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && !segment.contains(':')
            && !segment.chars().any(char::is_control)
            && !segment.contains('\u{0000}')
    })
}

#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    /// Recursively creates a directory.
    async fn create_dir_all(&self, path: &str) -> nexus_core::Result<()>;

    /// Overwrites (or creates) a file at `path`.
    async fn write_file(&self, path: &str, data: &[u8]) -> nexus_core::Result<()>;

    /// Reads a file, or `NotFound` when it does not exist.
    async fn read_file(&self, path: &str) -> nexus_core::Result<Vec<u8>>;

    /// Removes a file, or `NotFound` when it does not exist.
    async fn delete_file(&self, path: &str) -> nexus_core::Result<()>;

    async fn file_exists(&self, path: &str) -> nexus_core::Result<bool>;

    /// Lists every entry directly under `dir`.
    async fn list_entries(&self, dir: &str) -> nexus_core::Result<Vec<StorageEntry>>;

    /// Removes an entire directory tree (used when a cube is destroyed).
    async fn remove_dir_all(&self, path: &str) -> nexus_core::Result<()>;
}

/// Recursively lists every file under `dir`, walking subdirectories.
pub async fn list_all_files(storage: &dyn Storage, dir: &str) -> nexus_core::Result<Vec<String>> {
    let mut files = Vec::new();
    let mut frontier = vec![dir.to_owned()];
    while let Some(directory) = frontier.pop() {
        for entry in storage.list_entries(&directory).await? {
            match entry.kind {
                EntryKind::File => files.push(entry.path),
                EntryKind::Directory => frontier.push(entry.path),
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_paths() {
        assert!(is_safe_relative_path("docs/readme.md"));
        assert!(is_safe_relative_path("a"));
        assert!(is_safe_relative_path("a/b/c"));
    }

    #[test]
    fn rejects_escapes() {
        assert!(!is_safe_relative_path(""));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path("../secret"));
        assert!(!is_safe_relative_path("a/../../secret"));
        assert!(!is_safe_relative_path("a/./b"));
        assert!(!is_safe_relative_path("C:/windows/system32"));
        assert!(!is_safe_relative_path("C:\\windows"));
        assert!(!is_safe_relative_path("a\nb"));
    }
}
