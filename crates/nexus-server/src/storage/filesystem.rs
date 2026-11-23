use std::path::{Path, PathBuf};
use std::sync::Arc;

use nexus_core::error::Error;
use tokio::sync::Mutex;

use super::{is_safe_relative_path, Storage};

/// Stores files under a base directory on the host filesystem.
///
/// Every key is contained within `base`; a canonicalization check re-verifies
/// containment after directory creation so a symlink planted inside the tree
/// cannot smuggle reads or writes outside the root.
pub struct FilesystemStorage {
    base: PathBuf,
    // Serializes structural mutations (create_dir_all / remove_dir_all) to
    // avoid racing directory creation from concurrent workspace operations.
    lock: Arc<Mutex<()>>,
}

impl FilesystemStorage {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self {
            base: base.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    fn key_within_base(&self, key: &str) -> nexus_core::Result<PathBuf> {
        if !is_safe_relative_path(key) {
            return Err(Error::invalid_path(format!("unsafe storage key: {key:?}")));
        }
        Ok(self.base.join(key))
    }

    /// Verifies `candidate` stays inside the canonicalized base, resolving any
    /// symlinks put in between. A not-yet-existing candidate is validated
    /// against its nearest existing ancestor, and a not-yet-created base walks
    /// up to its nearest existing ancestor as its canonical anchor, so a fresh
    /// store can be used before its root directory physically exists (the
    /// first `create_dir_all`/`write_file` materializes it).
    fn ensure_contained(&self, candidate: &Path) -> nexus_core::Result<PathBuf> {
        let base_canonical = self.base_canonical()?;
        let Ok(candidate_canonical) = std::fs::canonicalize(candidate) else {
            let mut probe = candidate.to_path_buf();
            while !probe.exists() {
                if !probe.pop() {
                    return Err(Error::invalid_path("storage key escapes its root"));
                }
            }
            let probe_canonical = std::fs::canonicalize(&probe)
                .map_err(|err| Error::storage("storage path unavailable").with_source(err))?;
            if probe_canonical.starts_with(&base_canonical) {
                return Ok(candidate.to_path_buf());
            }
            return Err(Error::invalid_path("storage key escapes its root"));
        };
        if candidate_canonical.starts_with(&base_canonical) {
            Ok(candidate_canonical)
        } else {
            Err(Error::invalid_path("storage key escapes its root"))
        }
    }

    /// Canonical anchor of the storage root, resolving any not-yet-created
    /// tail by walking up to the nearest existing ancestor.
    fn base_canonical(&self) -> nexus_core::Result<PathBuf> {
        let mut probe = self.base.clone();
        loop {
            match std::fs::canonicalize(&probe) {
                Ok(canonical) => return Ok(canonical),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    if !probe.pop() {
                        return Err(Error::storage("storage root unavailable").with_source(err));
                    }
                }
                Err(err) => return Err(Error::storage("storage root unavailable").with_source(err)),
            }
        }
    }

    async fn write_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.lock.lock().await
    }
}

#[async_trait::async_trait]
impl Storage for FilesystemStorage {
    async fn create_dir_all(&self, path: &str) -> nexus_core::Result<()> {
        let target = self.key_within_base(path)?;
        let _guard = self.write_guard().await;
        if target == self.base {
            return Ok(());
        }
        self.ensure_contained(&target)?;
        tokio::fs::create_dir_all(&target).await.map_err(|err| {
            Error::storage(format!("cannot create directory: {path}")).with_source(err)
        })
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> nexus_core::Result<()> {
        let target = self.key_within_base(path)?;
        let parent = target
            .parent()
            .ok_or_else(|| Error::invalid_path("file has no parent"))?;
        let _guard = self.write_guard().await;
        self.ensure_contained(parent)?;
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            Error::storage(format!("cannot create parent dirs for: {path}")).with_source(err)
        })?;
        tokio::fs::write(&target, data)
            .await
            .map_err(|err| Error::storage(format!("cannot write file: {path}")).with_source(err))
    }

    async fn read_file(&self, path: &str) -> nexus_core::Result<Vec<u8>> {
        let target = self.key_within_base(path)?;
        let _guard = self.write_guard().await;
        self.ensure_contained(&target)?;
        tokio::fs::read(&target).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Error::not_found(format!("no such file: {path}"))
            } else {
                Error::storage(format!("cannot read file: {path}")).with_source(err)
            }
        })
    }

    async fn delete_file(&self, path: &str) -> nexus_core::Result<()> {
        let target = self.key_within_base(path)?;
        let _guard = self.write_guard().await;
        self.ensure_contained(&target)?;
        tokio::fs::remove_file(&target).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Error::not_found(format!("no such file: {path}"))
            } else {
                Error::storage(format!("cannot delete file: {path}")).with_source(err)
            }
        })
    }

    async fn file_exists(&self, path: &str) -> nexus_core::Result<bool> {
        let target = self.key_within_base(path)?;
        let _guard = self.write_guard().await;
        self.ensure_contained(&target).map_err(|err| {
            // A dangling symlink is treated as "does not exist".
            if err.code() == nexus_core::error::ErrorCode::InvalidPath {
                Error::not_found(format!("no such file: {path}"))
            } else {
                err
            }
        })?;
        Ok(tokio::fs::try_exists(&target).await.unwrap_or(false))
    }

    async fn list_entries(
        &self,
        dir: &str,
    ) -> nexus_core::Result<Vec<crate::storage::StorageEntry>> {
        use crate::storage::StorageEntry;

        let target = self.key_within_base(dir)?;
        let base_prefix = {
            let prefix = self.base.to_string_lossy().into_owned();
            if prefix.ends_with(std::path::MAIN_SEPARATOR) {
                prefix
            } else {
                format!("{prefix}{}", std::path::MAIN_SEPARATOR)
            }
        };
        let _guard = self.write_guard().await;
        self.ensure_contained(&target)?;
        let mut entries = Vec::new();
        let mut read_dir = match tokio::fs::read_dir(&target).await {
            Ok(read_dir) => read_dir,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(err) => return Err(Error::storage(format!("cannot list: {dir}")).with_source(err)),
        };
        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|err| Error::storage(format!("cannot list: {dir}")).with_source(err))?
        {
            let path = entry.path();
            let relative = path
                .to_string_lossy()
                .strip_prefix(&base_prefix)
                .map(|s| s.to_owned());
            let Some(relative) = relative else {
                continue;
            };
            let relative = relative.replace('\\', "/");
            let metadata = tokio::fs::metadata(&path)
                .await
                .map_err(|err| Error::storage(format!("cannot stat: {path:?}")).with_source(err))?;
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| nexus_core::timestamp::Timestamp::from_millis(d.as_millis() as u64))
                .unwrap_or_else(nexus_core::timestamp::Timestamp::now);
            if metadata.is_file() {
                entries.push(StorageEntry::file(relative, metadata.len(), modified_at));
            } else if metadata.is_dir() {
                entries.push(StorageEntry::directory(relative, modified_at));
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn remove_dir_all(&self, path: &str) -> nexus_core::Result<()> {
        let target = self.key_within_base(path)?;
        let _guard = self.write_guard().await;
        self.ensure_contained(&target)?;
        tokio::fs::remove_dir_all(&target).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Error::not_found(format!("no such directory: {path}"))
            } else {
                Error::storage(format!("cannot remove: {path}")).with_source(err)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().expect("tempdir")
    }

    fn unique() -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string()
    }

    #[tokio::test]
    async fn roundtrip_filesystem() {
        let temp = temp_dir();
        let store = FilesystemStorage::new(temp.path().join(unique()));
        store.write_file("docs/readme.md", b"hello").await.unwrap();
        assert_eq!(store.read_file("docs/readme.md").await.unwrap(), b"hello");
        assert!(store.file_exists("docs/readme.md").await.unwrap());
        let entries = store.list_entries("docs").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "docs/readme.md");
        assert_eq!(entries[0].size, 5);
        store.delete_file("docs/readme.md").await.unwrap();
        assert!(!store.file_exists("docs/readme.md").await.unwrap());
    }

    #[tokio::test]
    async fn rejects_traversal() {
        let temp = temp_dir();
        let store = FilesystemStorage::new(temp.path());
        let err = store
            .write_file("../../outside.txt", b"x")
            .await
            .unwrap_err();
        assert_eq!(err.code(), nexus_core::error::ErrorCode::InvalidPath);
        let err2 = store.read_file("/etc/hosts").await.unwrap_err();
        assert_eq!(err2.code(), nexus_core::error::ErrorCode::InvalidPath);
    }

    #[tokio::test]
    async fn listing_a_missing_subtree_is_empty() {
        let temp = temp_dir();
        let store = FilesystemStorage::new(temp.path().join(unique()));
        let entries = store.list_entries("cubes/never/workspace").await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn create_dir_all_makes_target_and_ancestors() {
        let temp = temp_dir();
        let base = temp.path().join(unique());
        let store = FilesystemStorage::new(&base);
        store.create_dir_all("a/b/c").await.unwrap();
        assert!(
            base.join("a/b/c").is_dir(),
            "target directory should be created"
        );
        let entries = store.list_entries("a").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "a/b");
    }
}
