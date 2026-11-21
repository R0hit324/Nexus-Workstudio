//! Namespaced, quota-controlled plugin storage.
//!
//! Each plugin owns an isolated namespace (`plugins/<plugin-id>/`); through
//! the public API a plugin can never read another plugin's keys, and total
//! on-disk usage is bounded by configured quotas so no plugin can fill the
//! disk.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{PluginError, PluginResult};
use crate::id::PluginId;

/// Storage quotas for one plugin namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageQuota {
    /// Maximum total serialized bytes across all keys.
    pub max_total_bytes: u64,
    /// Maximum serialized bytes per stored object.
    pub max_object_bytes: u64,
    /// Maximum number of keys.
    pub max_entries: usize,
}

impl StorageQuota {
    /// Defaults for trusted plugins (1 MiB per object, 16 MiB total,
    /// 4096 entries).
    #[must_use]
    pub const fn default_plugin() -> Self {
        Self {
            max_total_bytes: 16 * 1024 * 1024,
            max_object_bytes: 1024 * 1024,
            max_entries: 4096,
        }
    }

    /// Restrictive defaults for restricted/untrusted plugins.
    #[must_use]
    pub const fn restricted() -> Self {
        Self {
            max_total_bytes: 1024 * 1024,
            max_object_bytes: 64 * 1024,
            max_entries: 256,
        }
    }
}

/// The storage backend the host provides. Values are opaque byte blobs the
/// plugin defines its own format for; the API never surfaces Nexus internals.
#[async_trait]
pub trait PluginStorageBackend: Send + Sync {
    /// Reads one key.
    async fn get(&self, plugin: &PluginId, key: &str) -> PluginResult<Option<Vec<u8>>>;

    /// Writes one key, enforcing the namespace quota.
    async fn set(
        &self,
        plugin: &PluginId,
        key: &str,
        value: Vec<u8>,
        quota: StorageQuota,
    ) -> PluginResult<()>;

    /// Deletes one key.
    async fn delete(&self, plugin: &PluginId, key: &str) -> PluginResult<()>;

    /// Lists keys (never values) of one namespace.
    async fn keys(&self, plugin: &PluginId) -> PluginResult<Vec<String>>;
}

/// Validates a storage key: non-empty, traversal-proof, and safe for
/// path- and JSON-shaped backends. Backslash, `..`, and control characters
/// are rejected outright.
#[must_use]
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 256
        && !key.contains('\\')
        && !key.contains("..")
        && !key.starts_with('/')
        && key.chars().all(|c| {
            !c.is_control() && (c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct NamespaceFile {
    entries: HashMap<String, Vec<u8>>,
}

/// Filesystem-backed namespace store: one JSON file per plugin.
///
/// Writes are atomic (write-then-rename), so a crash leaves either the old
/// or the new state, never a torn file.
pub struct FilesystemStorage {
    root: PathBuf,
}

impl FilesystemStorage {
    /// Creates a storage rooted at the given directory.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The on-disk file for one plugin namespace.
    ///
    /// The id's dots are replaced so the path segments never traverse out of
    /// the store root.
    #[must_use]
    pub fn namespace_file(&self, plugin: &PluginId) -> PathBuf {
        self.root
            .join(format!("{}.json", plugin.as_str().replace('.', "-")))
    }

    fn load(&self, plugin: &PluginId) -> NamespaceFile {
        std::fs::read(self.namespace_file(plugin))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, plugin: &PluginId, file: &NamespaceFile) -> PluginResult<()> {
        let path = self.namespace_file(plugin);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                PluginError::new(
                    Some(plugin.clone()),
                    "storage.persist",
                    format!("cannot create storage directory {}: {e}", parent.display()),
                )
            })?;
        }
        let bytes = serde_json::to_vec(file).map_err(|e| {
            PluginError::new(Some(plugin.clone()), "storage.persist", e.to_string())
        })?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| {
                PluginError::new(
                    Some(plugin.clone()),
                    "storage.persist",
                    format!("cannot persist namespace {}: {e}", path.display()),
                )
            })
    }

    fn enforce_quota(
        &self,
        plugin: &PluginId,
        key: &str,
        value: &[u8],
        file: &NamespaceFile,
        quota: &StorageQuota,
    ) -> PluginResult<()> {
        let replacing = file.entries.contains_key(key);
        if !replacing && file.entries.len() >= quota.max_entries {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.quota",
                format!("namespace limit of {} keys exceeded", quota.max_entries),
            ));
        }
        if value.len() as u64 > quota.max_object_bytes {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.quota",
                format!(
                    "object size {} exceeds per-object limit of {} bytes",
                    value.len(),
                    quota.max_object_bytes
                ),
            ));
        }
        let other: u64 = file
            .entries
            .iter()
            .filter(|(k, _)| k.as_str() != key)
            .map(|(_, v)| v.len() as u64)
            .sum();
        if other + value.len() as u64 > quota.max_total_bytes {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.quota",
                format!(
                    "total namespace size would exceed limit of {} bytes",
                    quota.max_total_bytes
                ),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl PluginStorageBackend for FilesystemStorage {
    async fn get(&self, plugin: &PluginId, key: &str) -> PluginResult<Option<Vec<u8>>> {
        if !valid_key(key) {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.read",
                format!("invalid storage key `{key}`"),
            ));
        }
        Ok(self.load(plugin).entries.get(key).cloned())
    }

    async fn set(
        &self,
        plugin: &PluginId,
        key: &str,
        value: Vec<u8>,
        quota: StorageQuota,
    ) -> PluginResult<()> {
        if !valid_key(key) {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.write",
                format!("invalid storage key `{key}`"),
            ));
        }
        let mut file = self.load(plugin);
        self.enforce_quota(plugin, key, &value, &file, &quota)
            .map(|()| ())?;
        file.entries.insert(String::from(key), value);
        self.save(plugin, &file)
    }

    async fn delete(&self, plugin: &PluginId, key: &str) -> PluginResult<()> {
        if !valid_key(key) {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.delete",
                format!("invalid storage key `{key}`"),
            ));
        }
        let mut file = self.load(plugin);
        file.entries.remove(key);
        self.save(plugin, &file)
    }

    async fn keys(&self, plugin: &PluginId) -> PluginResult<Vec<String>> {
        let mut keys: Vec<String> = self.load(plugin).entries.keys().cloned().collect();
        keys.sort();
        Ok(keys)
    }
}

/// In-memory backend used by tests and the TUI client.
#[derive(Default)]
pub struct InMemoryStorage {
    namespaces: std::sync::RwLock<HashMap<PluginId, NamespaceFile>>,
}

impl InMemoryStorage {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PluginStorageBackend for InMemoryStorage {
    async fn get(&self, plugin: &PluginId, key: &str) -> PluginResult<Option<Vec<u8>>> {
        if !valid_key(key) {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.read",
                format!("invalid storage key `{key}`"),
            ));
        }
        Ok(self
            .namespaces
            .read()
            .expect("storage lock")
            .get(plugin)
            .and_then(|f| f.entries.get(key).cloned()))
    }

    async fn set(
        &self,
        plugin: &PluginId,
        key: &str,
        value: Vec<u8>,
        quota: StorageQuota,
    ) -> PluginResult<()> {
        if !valid_key(key) {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.write",
                format!("invalid storage key `{key}`"),
            ));
        }
        let mut namespaces = self.namespaces.write().expect("storage lock");
        let file = namespaces.entry(plugin.clone()).or_default();
        if !file.entries.contains_key(key) && file.entries.len() >= quota.max_entries {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.quota",
                format!("namespace limit of {} keys exceeded", quota.max_entries),
            ));
        }
        if value.len() as u64 > quota.max_object_bytes {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.quota",
                format!(
                    "object size {} exceeds per-object limit of {} bytes",
                    value.len(),
                    quota.max_object_bytes
                ),
            ));
        }
        let other: u64 = file
            .entries
            .iter()
            .filter(|(k, _)| k.as_str() != key)
            .map(|(_, v)| v.len() as u64)
            .sum();
        if other + value.len() as u64 > quota.max_total_bytes {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "storage.quota",
                format!(
                    "total namespace size would exceed limit of {} bytes",
                    quota.max_total_bytes
                ),
            ));
        }
        file.entries.insert(String::from(key), value);
        Ok(())
    }

    async fn delete(&self, plugin: &PluginId, key: &str) -> PluginResult<()> {
        if let Some(file) = self
            .namespaces
            .write()
            .expect("storage lock")
            .get_mut(plugin)
        {
            file.entries.remove(key);
        }
        Ok(())
    }

    async fn keys(&self, plugin: &PluginId) -> PluginResult<Vec<String>> {
        Ok(self
            .namespaces
            .read()
            .expect("storage lock")
            .get(plugin)
            .map(|f| {
                let mut keys: Vec<String> = f.entries.keys().cloned().collect();
                keys.sort();
                keys
            })
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn namespaces_are_isolated() {
        let a = PluginId::new("org.nexus.a.b").unwrap();
        let b = PluginId::new("org.nexus.a.c").unwrap();
        let storage = FilesystemStorage::new(std::env::temp_dir().join("nexus-plugin-test"));
        storage
            .set(&a, "config", b"local".to_vec(), StorageQuota::restricted())
            .await
            .unwrap();
        assert_eq!(
            storage.get(&a, "config").await.unwrap(),
            Some(b"local".to_vec())
        );
        assert_eq!(storage.get(&b, "config").await.unwrap(), None);
    }

    #[tokio::test]
    async fn quota_limits_object_size() {
        let id = PluginId::new("org.nexus.quota.a").unwrap();
        let storage = FilesystemStorage::new(std::env::temp_dir().join("nexus-plugin-test"));
        let err = storage
            .set(
                &id,
                "blob",
                vec![0u8; (StorageQuota::restricted().max_object_bytes + 1) as usize],
                StorageQuota::restricted(),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("per-object limit"));
    }

    #[tokio::test]
    async fn quota_limits_total_namespace() {
        let id = PluginId::new("org.nexus.quota.b").unwrap();
        let storage = FilesystemStorage::new(std::env::temp_dir().join("nexus-plugin-test"));
        let quota = StorageQuota {
            max_total_bytes: 100,
            max_object_bytes: 64,
            max_entries: 10,
        };
        storage.set(&id, "one", vec![0u8; 60], quota).await.unwrap();
        let err = storage
            .set(&id, "two", vec![0u8; 60], quota)
            .await
            .unwrap_err();
        assert!(err.message.contains("total namespace size"));
    }

    #[tokio::test]
    async fn delete_removes_keys() {
        let id = PluginId::new("org.nexus.quota.c").unwrap();
        let storage = FilesystemStorage::new(std::env::temp_dir().join("nexus-plugin-test"));
        storage
            .set(&id, "k", b"v".to_vec(), StorageQuota::restricted())
            .await
            .unwrap();
        storage.delete(&id, "k").await.unwrap();
        assert_eq!(storage.get(&id, "k").await.unwrap(), None);
        assert_eq!(storage.keys(&id).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn invalid_keys_rejected() {
        let id = PluginId::new("org.nexus.quota.d").unwrap();
        let storage = InMemoryStorage::new();
        let err = storage
            .set(&id, "/../escape", b"x".to_vec(), StorageQuota::restricted())
            .await
            .unwrap_err();
        assert!(err.message.contains("invalid storage key"));
        assert!(!valid_key("../up"));
        assert!(valid_key("settings/general.theme"));
    }
}

/// Secret storage abstraction (spec �52): secrets never mix into the
/// regular storage namespace and are never printable in logs; hosts are
/// expected to back the trait with the OS keyring, the in-process store
/// keeps plugin secrets write-only readable.
#[async_trait]
pub trait PluginSecretStore: Send + Sync {
    /// Stores one secret, replacing any previous value atomically.
    ///
    /// # Errors
    /// Backend failure.
    async fn set_secret(&self, plugin: &PluginId, key: &str, value: &[u8]) -> PluginResult<()>;

    /// Reads one secret; returns None when not stored.
    ///
    /// # Errors
    /// Backend failure or invalid key.
    async fn get_secret(&self, plugin: &PluginId, key: &str) -> PluginResult<Option<Vec<u8>>>;

    /// Removes one secret.
    ///
    /// # Errors
    /// Backend failure.
    async fn delete_secret(&self, plugin: &PluginId, key: &str) -> PluginResult<()>;

    /// Lists secret keys (never values).
    ///
    /// # Errors
    /// Backend failure.
    async fn secret_keys(&self, plugin: &PluginId) -> PluginResult<Vec<String>>;
}

fn secret_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// In-process secret store (reference backend; hosts may wrap the OS
/// keyring behind the same trait).
#[derive(Default)]
pub struct InMemorySecretStore {
    by_plugin: RwLock<HashMap<PluginId, HashMap<String, Vec<u8>>>>,
}

impl InMemorySecretStore {
    /// Builds an empty secret store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PluginSecretStore for InMemorySecretStore {
    async fn set_secret(&self, plugin: &PluginId, key: &str, value: &[u8]) -> PluginResult<()> {
        if !secret_valid_key(key) {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "secrets.set",
                format!("invalid secret key `{key}`"),
            ));
        }
        let mut by_plugin = self.by_plugin.write().expect("secret store lock");
        by_plugin
            .entry(plugin.clone())
            .or_default()
            .insert(String::from(key), value.to_vec());
        Ok(())
    }

    async fn get_secret(&self, plugin: &PluginId, key: &str) -> PluginResult<Option<Vec<u8>>> {
        if !secret_valid_key(key) {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "secrets.get",
                format!("invalid secret key `{key}`"),
            ));
        }
        let by_plugin = self.by_plugin.read().expect("secret store lock");
        Ok(by_plugin.get(plugin).and_then(|map| map.get(key)).cloned())
    }

    async fn delete_secret(&self, plugin: &PluginId, key: &str) -> PluginResult<()> {
        if !secret_valid_key(key) {
            return Err(PluginError::new(
                Some(plugin.clone()),
                "secrets.delete",
                format!("invalid secret key `{key}`"),
            ));
        }
        let mut by_plugin = self.by_plugin.write().expect("secret store lock");
        if let Some(map) = by_plugin.get_mut(plugin) {
            map.remove(key);
        }
        Ok(())
    }

    async fn secret_keys(&self, plugin: &PluginId) -> PluginResult<Vec<String>> {
        Ok(self
            .by_plugin
            .read()
            .expect("secret store lock")
            .get(plugin)
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod secrets_tests {
    use super::*;

    fn pid(name: &str) -> PluginId {
        PluginId::new(name).unwrap()
    }

    #[tokio::test]
    async fn secret_round_trip_and_listing() {
        let store = InMemorySecretStore::new();
        let a = pid("org.nexus.secret.alpha");
        let b = pid("org.nexus.secret.beta");
        store
            .set_secret(&a, "api-token", b"tok-1234")
            .await
            .unwrap();
        assert_eq!(
            store.get_secret(&a, "api-token").await.unwrap(),
            Some(b"tok-1234".to_vec())
        );
        store
            .set_secret(&b, "api-token", b"tok-9999")
            .await
            .unwrap();
        assert_eq!(
            store.get_secret(&a, "api-token").await.unwrap(),
            Some(b"tok-1234".to_vec()),
            "plugin namespaces are isolated"
        );
        store.delete_secret(&a, "api-token").await.unwrap();
        assert_eq!(store.get_secret(&a, "api-token").await.unwrap(), None);
        assert!(store
            .secret_keys(&b)
            .await
            .unwrap()
            .contains(&String::from("api-token")));
    }

    #[tokio::test]
    async fn invalid_secret_keys_rejected() {
        let store = InMemorySecretStore::new();
        let id = pid("org.nexus.secret.gamma");
        assert!(store.set_secret(&id, "", b"x").await.is_err());
        assert!(store.set_secret(&id, "../escape", b"x").await.is_err());
        assert!(store.set_secret(&id, "bad key!", b"x").await.is_err());
        let long = "a".repeat(129);
        assert!(store.set_secret(&id, &long, b"x").await.is_err());
        assert!(store.set_secret(&id, "clean_key-1.x", b"x").await.is_ok());
    }
}
