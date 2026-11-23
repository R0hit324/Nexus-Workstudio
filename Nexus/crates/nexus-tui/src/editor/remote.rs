//! Remote cursor registry.
//!
//! Tracks where other session members have their caret and selection in each
//! document. Only positions are kept (never content), so memory stays
//! proportional to the number of online users.

use nexus_core::ids::{FileId, UserId};

/// Another user's position inside one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCursor {
    pub user_id: UserId,
    pub file_id: FileId,
    pub position: usize,
    pub selection: Option<(usize, usize)>,
}

/// A bounded registry of remote cursors (one per user per document).
#[derive(Debug, Clone, Default)]
pub struct RemoteCursorStore {
    cursors: Vec<RemoteCursor>,
}

/// Which remote cursor a renderer should show at a given line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorLineRect {
    pub user_id: UserId,
    pub line: usize,
    pub col: usize,
    pub select_to: Option<usize>,
}

impl RemoteCursorStore {
    /// The maximum number of entries that can be shown at once.
    pub const LIMIT: usize = 512;

    /// Inserts or replaces a user's cursor in `file`.
    pub fn upsert(
        &mut self,
        user_id: UserId,
        file_id: FileId,
        position: usize,
        selection: Option<(usize, usize)>,
    ) {
        if let Some(existing) = self
            .cursors
            .iter_mut()
            .find(|c| c.user_id == user_id && c.file_id == file_id)
        {
            existing.position = position;
            existing.selection = selection;
            return;
        }
        if self.cursors.len() >= Self::LIMIT {
            self.cursors.remove(0);
        }
        self.cursors.push(RemoteCursor {
            user_id,
            file_id,
            position,
            selection,
        });
    }

    /// Removes all cursors for a user (on disconnect).
    pub fn remove_user(&mut self, user_id: UserId) {
        self.cursors.retain(|c| c.user_id != user_id);
    }

    /// The remote cursors visible in `file`.
    #[must_use]
    pub fn for_file(&self, file_id: FileId) -> Vec<&RemoteCursor> {
        self.cursors
            .iter()
            .filter(|c| c.file_id == file_id)
            .collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.cursors.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_replaces_same_user_same_file() {
        let mut store = RemoteCursorStore::default();
        let u = UserId::from_u64(1);
        let f = FileId::from_u64(1);
        store.upsert(u, f, 4, None);
        store.upsert(u, f, 9, Some((2, 9)));
        assert_eq!(store.len(), 1);
        assert_eq!(store.for_file(f)[0].position, 9);
        assert_eq!(store.for_file(f)[0].selection, Some((2, 9)));
    }

    #[test]
    fn different_files_are_kept_apart() {
        let mut store = RemoteCursorStore::default();
        store.upsert(UserId::from_u64(1), FileId::from_u64(1), 1, None);
        store.upsert(UserId::from_u64(1), FileId::from_u64(2), 2, None);
        assert_eq!(store.len(), 2);
        assert_eq!(store.for_file(FileId::from_u64(1)).len(), 1);
    }

    #[test]
    fn remove_user_drops_all_their_cursors() {
        let mut store = RemoteCursorStore::default();
        store.upsert(UserId::from_u64(1), FileId::from_u64(1), 1, None);
        store.upsert(UserId::from_u64(1), FileId::from_u64(2), 1, None);
        store.upsert(UserId::from_u64(2), FileId::from_u64(1), 1, None);
        store.remove_user(UserId::from_u64(1));
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.for_file(FileId::from_u64(1))[0].user_id,
            UserId::from_u64(2)
        );
    }

    #[test]
    fn bounded_at_limit_drops_oldest() {
        let mut store = RemoteCursorStore::default();
        for i in 0..(RemoteCursorStore::LIMIT + 10) as u64 {
            store.upsert(UserId::from_u64(i), FileId::from_u64(1), 0, None);
        }
        assert!(store.len() <= RemoteCursorStore::LIMIT);
        let remaining = store.for_file(FileId::from_u64(1));
        let first = remaining.first().unwrap().user_id;
        assert!(first.to_u64() >= Some(10));
    }
}
