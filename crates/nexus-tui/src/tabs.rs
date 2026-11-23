//! Tab bar over open editor documents.
//!
//! Tabs reference document ids; they never duplicate document contents, so
//! memory stays bounded by the document store, not the number of tabs.

use nexus_core::ids::FileId;

/// A single tab pointing at a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub file_id: FileId,
    /// Display label (usually the file name).
    pub label: String,
}

/// Ordered set of open tabs.
#[derive(Debug, Clone, Default)]
pub struct Tabs {
    pub items: Vec<Tab>,
    pub active: usize,
}

impl Tabs {
    /// Opens `file_id`. Reuses an existing tab and activates it when already
    /// open; otherwise appends a new tab.
    pub fn open(&mut self, file_id: FileId, label: impl Into<String>) {
        if let Some(idx) = self.items.iter().position(|t| t.file_id == file_id) {
            self.active = idx;
            return;
        }
        self.items.push(Tab {
            file_id,
            label: label.into(),
        });
        self.active = self.items.len() - 1;
    }

    /// Closes the active tab; active moves to its left neighbor.
    pub fn close_active(&mut self) -> Option<FileId> {
        if self.items.is_empty() {
            return None;
        }
        let idx = self.active.min(self.items.len() - 1);
        let removed = self.items.remove(idx).file_id;
        self.active = self.active.min(self.items.len().saturating_sub(1));
        Some(removed)
    }

    /// Closes a specific tab.
    pub fn close(&mut self, file_id: FileId) -> Option<FileId> {
        if let Some(idx) = self.items.iter().position(|t| t.file_id == file_id) {
            self.items.remove(idx);
            if self.items.is_empty() {
                self.active = 0;
            } else if idx <= self.active {
                self.active = self.active.saturating_sub(1);
            }
            return Some(file_id);
        }
        None
    }

    /// Switches to the next/previous tab, wrapping around.
    pub fn cycle(&mut self, delta: isize) {
        match self.items.len() {
            0 | 1 => {}
            n => {
                let n = n as isize;
                self.active = ((self.active as isize + delta).rem_euclid(n)) as usize;
            }
        }
    }

    /// The active tab, if any.
    #[must_use]
    pub fn active_tab(&self) -> Option<&Tab> {
        self.items
            .get(self.active.min(self.items.len().saturating_sub(1)))
    }

    /// The ids of all open tabs, newest first.
    #[must_use]
    pub fn file_ids(&self) -> impl Iterator<Item = FileId> + '_ {
        self.items.iter().map(|t| t.file_id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fid(n: u8) -> FileId {
        // Deterministic ids from counters.
        FileId::from_u64(n as u64)
    }

    #[test]
    fn opening_is_idempotent_and_activates() {
        let mut tabs = Tabs::default();
        tabs.open(fid(1), "a.rs");
        tabs.open(fid(2), "b.rs");
        tabs.open(fid(3), "c.rs");
        assert_eq!(tabs.items.len(), 3);
        assert_eq!(tabs.active_tab().unwrap().label, "c.rs");

        tabs.open(fid(1), "a.rs"); // reopen: focus, no duplicate
        assert_eq!(tabs.items.len(), 3);
        assert_eq!(tabs.active_tab().unwrap().file_id, fid(1));
    }

    #[test]
    fn closing_active_selects_left_neighbor() {
        let mut tabs = Tabs::default();
        tabs.open(fid(1), "a.rs");
        tabs.open(fid(2), "b.rs");
        tabs.open(fid(3), "c.rs");
        assert_eq!(tabs.close_active(), Some(fid(3)));
        assert_eq!(tabs.active_tab().unwrap().file_id, fid(2));
    }

    #[test]
    fn closing_specific_tab_keeps_index_sane() {
        let mut tabs = Tabs::default();
        tabs.open(fid(1), "a.rs");
        tabs.open(fid(2), "b.rs");
        tabs.open(fid(3), "c.rs");
        tabs.close(fid(2)).unwrap();
        assert_eq!(tabs.file_ids().collect::<Vec<_>>(), vec![fid(1), fid(3)]);
        assert!(tabs.close(fid(9)).is_none());
    }

    #[test]
    fn cycling_wraps() {
        let mut tabs = Tabs::default();
        tabs.open(fid(1), "a.rs");
        tabs.open(fid(2), "b.rs");
        tabs.cycle(1);
        assert_eq!(tabs.active_tab().unwrap().file_id, fid(1));
        tabs.cycle(1);
        assert_eq!(tabs.active_tab().unwrap().file_id, fid(2));
        tabs.cycle(-1);
        assert_eq!(tabs.active_tab().unwrap().file_id, fid(1));
    }

    #[test]
    fn closing_last_file_empties() {
        let mut tabs = Tabs::default();
        tabs.open(fid(1), "a.rs");
        tabs.close_active().unwrap();
        assert!(tabs.is_empty());
        assert_eq!(tabs.close_active(), None);
    }
}
