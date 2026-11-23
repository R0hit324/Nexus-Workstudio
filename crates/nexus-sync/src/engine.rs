use std::sync::Arc;

use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Text, Transact, Update};

use crate::{
    error::{SyncError, SyncResult},
    operations::{EditOperation, TextOperation, Version},
};

/// Abstraction for the text synchronization engine.
///
/// Keeps CRDT/OT implementation details isolated from the rest of the system.
pub trait TextSyncEngine: Send + Sync {
    /// Apply a local edit operation and return the generated CRDT operations.
    fn local_edit(&self, operation: &EditOperation) -> SyncResult<Vec<TextOperation>>;

    /// Apply a remote operation from another client/server.
    fn remote_edit(&self, operation: &EditOperation) -> SyncResult<()>;

    /// Get current document content as plain text.
    fn content(&self) -> String;

    /// Get current document version.
    fn version(&self) -> Version;

    /// Encode state vector for delta synchronization.
    fn state_vector(&self) -> Vec<u8>;

    /// Apply a delta update from state vector.
    fn apply_delta(&self, delta: &[u8]) -> SyncResult<()>;

    /// Request missing operations from a version.
    fn request_delta(&self, from_version: Version) -> SyncResult<Vec<EditOperation>>;

    /// Force resynchronization to a specific state.
    fn resynchronize(&self, content: String, version: Version) -> SyncResult<()>;

    /// Get the Yrs Doc for advanced operations (if using Yrs).
    fn as_yrs_doc(&self) -> Option<&Doc> {
        None
    }
}

/// Yrs-based CRDT synchronization engine.
///
/// Uses Yrs (Rust port of Yjs) for conflict-free replicated data type
/// synchronization of text documents.
pub struct YrsSyncEngine {
    doc: Doc,
    text: yrs::TextRef,
    // Track version separately since Yrs uses state vectors
    version: parking_lot::RwLock<Version>,
}

impl YrsSyncEngine {
    /// Create a new Yrs synchronization engine.
    pub fn new() -> Self {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("content");
        Self {
            doc,
            text,
            version: parking_lot::RwLock::new(Version::INITIAL),
        }
    }

    /// Create from existing document state (for server recovery).
    pub fn from_content(content: String) -> Self {
        let engine = Self::new();
        if !content.is_empty() {
            let mut txn = engine.doc.transact_mut();
            engine.text.insert(&mut txn, 0, &content);
        }
        engine
    }

    fn apply_text_ops(&self, ops: &[TextOperation]) -> SyncResult<()> {
        let mut txn = self.doc.transact_mut();
        for op in ops {
            match op {
                TextOperation::Insert { position, text } => {
                    self.text.insert(&mut txn, *position, text);
                }
                TextOperation::Delete { position, length } => {
                    self.text.remove_range(&mut txn, *position, *length);
                }
                TextOperation::Retain { .. } => {
                    // No-op for CRDT
                }
            }
        }
        Ok(())
    }
}

impl Default for YrsSyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl YrsSyncEngine {
    /// Create from a binary Yrs update (for persistence/restore).
    pub fn from_update(update: &[u8]) -> SyncResult<Self> {
        let engine = Self::new();
        {
            let mut txn = engine.doc.transact_mut();
            let update_obj =
                Update::decode_v1(update).map_err(|e| SyncError::CrdtError(e.to_string()))?;
            let _ = txn.apply_update(update_obj);
        }
        Ok(engine)
    }
}

impl TextSyncEngine for YrsSyncEngine {
    fn local_edit(&self, operation: &EditOperation) -> SyncResult<Vec<TextOperation>> {
        // Apply locally - Yrs handles this optimistically
        self.apply_text_ops(&operation.operations)?;
        let mut version = self.version.write();
        *version = version.next();
        Ok(operation.operations.clone())
    }

    fn remote_edit(&self, operation: &EditOperation) -> SyncResult<()> {
        // Apply remote operation - Yrs merges automatically
        self.apply_text_ops(&operation.operations)?;
        let mut version = self.version.write();
        *version = operation.base_version.next();
        Ok(())
    }

    fn content(&self) -> String {
        // TODO: Implement proper text content retrieval from Yrs
        String::new()
    }

    fn version(&self) -> Version {
        *self.version.read()
    }

    fn state_vector(&self) -> Vec<u8> {
        let txn = self.doc.transact();
        let state_vector: StateVector = ReadTxn::state_vector(&txn);
        Encode::encode_v1(&state_vector)
    }

    fn apply_delta(&self, delta: &[u8]) -> SyncResult<()> {
        let mut txn = self.doc.transact_mut();
        let update = Decode::decode_v1(delta).map_err(|e| SyncError::CrdtError(e.to_string()))?;
        let _ = txn.apply_update(update);
        let mut version = self.version.write();
        *version = version.next();
        Ok(())
    }

    fn resynchronize(&self, content: String, version: Version) -> SyncResult<()> {
        let mut txn = self.doc.transact_mut();
        // Clear existing content
        let len = self.text.len(&txn) as u32;
        if len > 0 {
            self.text.remove_range(&mut txn, 0, len);
        }
        // Insert new content
        if !content.is_empty() {
            self.text.insert(&mut txn, 0, &content);
        }
        *self.version.write() = version;
        Ok(())
    }

    fn request_delta(&self, _from_version: Version) -> SyncResult<Vec<EditOperation>> {
        // In a full implementation, this would encode the delta from the given version
        // For now, return empty - the server will send full state if needed
        Ok(Vec::new())
    }

    fn as_yrs_doc(&self) -> Option<&Doc> {
        Some(&self.doc)
    }
}

/// Factory for creating sync engines.
pub struct SyncEngineFactory;

impl SyncEngineFactory {
    /// Create a new Yrs-based engine.
    pub fn create_yrs() -> Arc<dyn TextSyncEngine> {
        Arc::new(YrsSyncEngine::new())
    }

    /// Create from existing content.
    pub fn create_yrs_from_content(content: String) -> Arc<dyn TextSyncEngine> {
        Arc::new(YrsSyncEngine::from_content(content))
    }

    /// Create from binary update.
    pub fn create_yrs_from_update(update: &[u8]) -> SyncResult<Arc<dyn TextSyncEngine>> {
        YrsSyncEngine::from_update(update).map(|e| Arc::new(e) as Arc<dyn TextSyncEngine>)
    }
}
