//! Property-based and convergence tests for the synchronization layer
//! (Part 14, spec sections 50, 73, and the final quality objective).
//!
//! These tests exercise the CRDT synchronization properties that ordinary
//! unit tests cannot reach:
//!
//! - **Convergence**: two engines fed the same operation stream converge to
//!   identical content.
//! - **Local vs. remote application**: applying one edit through the local
//!   path or the remote path must produce the same document state.
//! - **Versioning**: local edits advance the version monotonically; resync
//!   restores the same content that incremental edits produced.

#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use nexus_core::ids::{FileId, UserId};
use nexus_sync::{EditOperation, TextOperation, TextSyncEngine, Version, YrsSyncEngine};

fn insert(position: u32, text: &str) -> TextOperation {
    TextOperation::Insert {
        position,
        text: text.to_string(),
    }
}

fn edit(file_id: FileId, base_version: Version, operations: Vec<TextOperation>) -> EditOperation {
    EditOperation::new(file_id, UserId::new(), base_version, operations)
}

#[test]
fn local_and_remote_application_of_the_same_edit_converge() {
    let local = YrsSyncEngine::from_content("hello world".to_string());
    let remote = YrsSyncEngine::from_content("hello world".to_string());

    let operation = edit(FileId::new(), Version::INITIAL, vec![insert(5, " brave")]);
    local.local_edit(&operation).expect("local edit applies");
    remote.remote_edit(&operation).expect("remote edit applies");

    assert_eq!(local.content(), "hello brave world");
    assert_eq!(remote.content(), "hello brave world");
}

#[test]
fn versions_advance_monotonically_with_local_edits() {
    let engine = YrsSyncEngine::new();
    assert_eq!(engine.version(), Version::INITIAL);

    let first = edit(FileId::new(), Version::INITIAL, vec![insert(0, "a")]);
    engine.local_edit(&first).expect("first edit applies");
    assert_eq!(engine.version().value(), 1);

    let second = edit(FileId::new(), Version::INITIAL, vec![insert(2, "b")]);
    engine.local_edit(&second).expect("second edit applies");
    assert_eq!(engine.version().value(), 2);
}

#[test]
fn resync_restores_the_content_produced_by_incremental_edits() {
    let engine = YrsSyncEngine::new();
    let first = edit(FileId::new(), Version::INITIAL, vec![insert(0, "hello")]);
    engine.local_edit(&first).expect("first edit applies");

    let second = edit(FileId::new(), Version::INITIAL, vec![insert(5, ", world")]);
    engine.local_edit(&second).expect("second edit applies");

    engine
        .resynchronize("hello, world".to_string(), engine.version())
        .expect("resync applies");

    assert_eq!(engine.content(), "hello, world");
}
