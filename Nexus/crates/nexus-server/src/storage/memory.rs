use std::collections::HashMap;
use std::sync::Mutex;

use nexus_core::error::Error;
use nexus_core::timestamp::Timestamp;

use super::{is_safe_relative_path, Storage, StorageEntry};

type FileValue = (Vec<u8>, Timestamp);

/// An in-memory byte store used by tests and the default dev mode.
///
/// Directories are explicit (for listing fidelity) and auto-created whenever a
/// nested file is written.
pub struct InMemoryStorage {
    files: Mutex<HashMap<String, FileValue>>,
    dirs: Mutex<HashMap<String, Timestamp>>,
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            dirs: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
    }

    /// Data-only snapshot of every stored file.
    pub fn snapshot(&self) -> HashMap<String, Vec<u8>> {
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(path, (data, _))| (path.clone(), data.clone()))
            .collect()
    }

    fn checked(path: &str) -> nexus_core::Result<String> {
        if is_safe_relative_path(path) {
            Ok(path.to_owned())
        } else {
            Err(Error::invalid_path("path escapes storage root"))
        }
    }
}

#[async_trait::async_trait]
impl Storage for InMemoryStorage {
    async fn create_dir_all(&self, path: &str) -> nexus_core::Result<()> {
        let key = Self::checked(path)?;
        let mut dirs = self.dirs.lock().unwrap_or_else(|p| p.into_inner());
        let mut current = String::new();
        for segment in key.split('/') {
            if segment.is_empty() {
                continue;
            }
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(segment);
            dirs.insert(current.clone(), Timestamp::now());
        }
        Ok(())
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> nexus_core::Result<()> {
        let key = Self::checked(path)?;
        let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        let mut dirs = self.dirs.lock().unwrap_or_else(|p| p.into_inner());
        let parent = key.rsplit_once('/').map(|(p, _)| p.to_owned());
        if let Some(parent) = parent {
            let mut current = String::new();
            for segment in parent.split('/') {
                if !current.is_empty() {
                    current.push('/');
                }
                current.push_str(segment);
                dirs.insert(current.clone(), Timestamp::now());
            }
        }
        files.insert(key, (data.to_vec(), Timestamp::now()));
        Ok(())
    }

    async fn read_file(&self, path: &str) -> nexus_core::Result<Vec<u8>> {
        let key = Self::checked(path)?;
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .map(|(data, _)| data.clone())
            .ok_or_else(|| Error::not_found(format!("no such file: {path}")))
    }

    async fn delete_file(&self, path: &str) -> nexus_core::Result<()> {
        let key = Self::checked(path)?;
        let removed = self
            .files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key);
        if removed.is_none() {
            return Err(Error::not_found(format!("no such file: {path}")));
        }
        Ok(())
    }

    async fn file_exists(&self, path: &str) -> nexus_core::Result<bool> {
        let key = Self::checked(path)?;
        Ok(self
            .files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&key))
    }

    async fn list_entries(&self, dir: &str) -> nexus_core::Result<Vec<StorageEntry>> {
        let key = if dir.is_empty() {
            String::new()
        } else {
            Self::checked(dir)?
        };
        let prefix = if key.is_empty() {
            String::new()
        } else {
            format!("{key}/")
        };
        let files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        let dirs = self.dirs.lock().unwrap_or_else(|p| p.into_inner());

        let mut children: HashMap<String, StorageEntry> = HashMap::new();
        // Directories first so a nested file "a/b/c.txt" records "a/b" as a
        // directory rather than a zero-size file.
        for (path, modified_at) in dirs.iter() {
            if *path == key {
                continue;
            }
            if let Some(rest) = path.strip_prefix(&prefix) {
                let child = child_of(&prefix, rest);
                children
                    .entry(child.clone())
                    .or_insert_with(|| StorageEntry::directory(child, *modified_at));
            }
        }
        for (path, (_, modified_at)) in files.iter() {
            if let Some(rest) = path.strip_prefix(&prefix) {
                let child = child_of(&prefix, rest);
                children.entry(child.clone()).or_insert_with(|| {
                    StorageEntry::file(child.clone(), size_of(files.get(&child)), *modified_at)
                });
            }
        }
        let mut entries: Vec<StorageEntry> = children.into_values().collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn remove_dir_all(&self, path: &str) -> nexus_core::Result<()> {
        let key = if path.is_empty() {
            String::new()
        } else {
            Self::checked(path)?
        };
        let prefix = if key.is_empty() {
            String::new()
        } else {
            format!("{key}/")
        };
        let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        files.retain(|p, _| !p.starts_with(&prefix));
        let mut dirs = self.dirs.lock().unwrap_or_else(|p| p.into_inner());
        dirs.retain(|p, _| !p.starts_with(&prefix));
        dirs.remove(&key);
        Ok(())
    }
}

fn child_of(prefix: &str, rest: &str) -> String {
    let child_name = rest.split('/').next().unwrap_or(rest);
    if prefix.is_empty() {
        child_name.to_owned()
    } else {
        format!("{prefix}{child_name}")
    }
}

fn size_of(entry: Option<&FileValue>) -> u64 {
    entry.map(|(data, _)| data.len() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::EntryKind;

    #[tokio::test]
    async fn roundtrip_and_listing() {
        let store = InMemoryStorage::new();
        store.write_file("docs/readme.md", b"hello").await.unwrap();
        store.write_file("docs/guide.md", b"guide").await.unwrap();
        store.write_file("other.txt", b"x").await.unwrap();

        assert_eq!(store.read_file("docs/readme.md").await.unwrap(), b"hello");
        let entries = store.list_entries("docs").await.unwrap();
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["docs/guide.md", "docs/readme.md"]);
        assert!(store.file_exists("docs/readme.md").await.unwrap());

        store.delete_file("docs/readme.md").await.unwrap();
        assert!(!store.file_exists("docs/readme.md").await.unwrap());
        assert!(matches!(
            store.read_file("docs/readme.md").await,
            Err(ref err) if err.code() == nexus_core::error::ErrorCode::NotFound
        ));
    }

    #[tokio::test]
    async fn directories_are_listed() {
        let store = InMemoryStorage::new();
        store.write_file("a/b/c.txt", b"x").await.unwrap();
        store.write_file("a/d.txt", b"y").await.unwrap();
        let root = store.list_entries("").await.unwrap();
        let kinds: Vec<(String, EntryKind)> = root.into_iter().map(|e| (e.path, e.kind)).collect();
        assert!(kinds.contains(&("a".to_string(), EntryKind::Directory)));
        let a = store.list_entries("a").await.unwrap();
        let paths: Vec<&str> = a.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a/b", "a/d.txt"]);
    }

    #[tokio::test]
    async fn rejects_escape_keys() {
        let store = InMemoryStorage::new();
        assert!(matches!(
            store.write_file("../etc/passwd", b"x").await,
            Err(ref err) if err.code() == nexus_core::error::ErrorCode::InvalidPath
        ));
    }
}
