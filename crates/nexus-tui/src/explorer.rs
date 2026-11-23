//! File explorer: tree/list state and filesystem navigation.
//!
//! The explorer is a *view* over the workspace. Authority over what the user
//! may modify lives server-side (later parts); here we only track listing,
//! expansion, selection, and translate user actions into requests.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A file or directory entry in the current directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub is_dir: bool,
}

impl Entry {
    #[must_use]
    pub fn name(&self) -> &str {
        self.path.file_name().and_then(|n| n.to_str()).unwrap_or("")
    }
}

/// Browse/crawl state for the explorer panel.
#[derive(Debug, Clone)]
pub struct Explorer {
    pub root: PathBuf,
    /// Entries of the current directory (flat list).
    pub entries: Vec<Entry>,
    /// Directories currently expanded in tree mode.
    pub expanded: HashSet<PathBuf>,
    /// Currently selected entry.
    pub selection: PathBuf,
    /// Scroll offset within `entries`.
    pub offset: usize,
    /// True when the selection moved by scrolling (dirty tracking).
    pub scrolled: bool,
}

impl Explorer {
    /// Creates an explorer rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            entries: Vec::new(),
            expanded: HashSet::new(),
            selection: root.clone(),
            offset: 0,
            root,
            scrolled: true,
        }
    }

    /// Replaces the directory listing.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        let selected = self.selection.clone();
        self.entries = entries;
        self.offset = 0;
        self.scrolled = true;
        if !self
            .entries
            .iter()
            .any(|e| e.path.as_path() == selected.as_path())
        {
            self.selection = self
                .entries
                .first()
                .map(|e| e.path.clone())
                .unwrap_or_else(|| self.root.clone());
        }
    }

    /// Lists the directory at `dir` using the real filesystem.
    #[cfg(not(target_os = "windows"))]
    pub fn read_dir(dir: &Path) -> std::io::Result<Vec<Entry>> {
        Ok(std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|de| Entry {
                path: de.path(),
                is_dir: de.file_type().map(|t| t.is_dir()).unwrap_or(false),
            })
            .collect())
    }

    /// Windows variant: hide hidden/system-flagged files by default.
    #[cfg(target_os = "windows")]
    pub fn read_dir(dir: &Path) -> std::io::Result<Vec<Entry>> {
        use std::os::windows::fs::MetadataExt;
        Ok(std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .filter(|de| {
                de.metadata()
                    .map(|m| m.file_attributes() & 0x6 == 0) // hidden/system bits
                    .unwrap_or(true)
            })
            .map(|de| Entry {
                path: de.path(),
                is_dir: de.file_type().map(|t| t.is_dir()).unwrap_or(false),
            })
            .collect())
    }

    /// Whether any entry is selected.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        !self.entries.is_empty() && self.entries.iter().any(|e| e.path == self.selection)
    }

    /// The selected entry, if it is in the listing.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == self.selection)
    }

    /// Moves the selection by `delta` within `visible` lines, scrolling.
    pub fn move_selection(&mut self, delta: isize, visible: usize) {
        if self.entries.is_empty() {
            return;
        }
        let len = self.entries.len() as isize;
        let mut idx = self
            .entries
            .iter()
            .position(|e| e.path == self.selection)
            .unwrap_or(0) as isize;
        idx = (idx + delta).clamp(0, len - 1);
        self.selection = self.entries[idx as usize].path.clone();
        let visible = visible.max(1);
        if self.offset > idx as usize {
            self.offset = idx as usize;
        } else if self.offset + visible <= idx as usize {
            self.offset = idx as usize + 1 - visible;
        }
        self.scrolled = idx as usize != self.offset;
    }

    /// Toggles directory expansion for `path`.
    pub fn toggle_expand(&mut self, path: &Path) {
        if self.expanded.contains(path) {
            self.expanded.remove(path);
        } else {
            self.expanded.insert(path.to_path_buf());
        }
    }

    #[must_use]
    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(names: &[(bool, &str)]) -> Vec<Entry> {
        names
            .iter()
            .map(|(is_dir, n)| Entry {
                path: PathBuf::from(n),
                is_dir: *is_dir,
            })
            .collect()
    }

    #[test]
    fn listing_replaces_and_selects_first() {
        let mut explorer = Explorer::new("workspace");
        explorer.set_entries(entries(&[(false, "a.rs"), (false, "b.rs"), (true, "src")]));
        assert_eq!(explorer.entries.len(), 3);
        assert_eq!(explorer.selection, PathBuf::from("a.rs"));
    }

    #[test]
    fn selection_stays_if_still_present() {
        let mut explorer = Explorer::new("workspace");
        explorer.set_entries(entries(&[(false, "a.rs"), (false, "b.rs")]));
        explorer.move_selection(1, 5);
        assert!(explorer.selected_entry().is_some());
        // Refresh with a listing that still contains b.rs.
        explorer.set_entries(entries(&[(false, "b.rs"), (false, "c.rs")]));
        assert_eq!(explorer.selection, PathBuf::from("b.rs"));
    }

    #[test]
    fn movement_scrolls_and_clamps() {
        let mut explorer = Explorer::new("workspace");
        explorer.set_entries(entries(&[
            (false, "0"),
            (false, "1"),
            (false, "2"),
            (false, "3"),
        ]));
        explorer.move_selection(3, 2);
        assert_eq!(explorer.selection, PathBuf::from("3"));
        assert_eq!(explorer.offset, 2); // scrolled down within 4 items / view 2
        explorer.move_selection(100, 2);
        assert_eq!(explorer.selection, PathBuf::from("3"));
        explorer.move_selection(-100, 2);
        assert_eq!(explorer.selection, PathBuf::from("0"));
    }

    #[test]
    fn expansion_toggles() {
        let mut explorer = Explorer::new("workspace");
        assert!(!explorer.is_expanded(Path::new("src")));
        explorer.toggle_expand(Path::new("src"));
        assert!(explorer.is_expanded(Path::new("src")));
        explorer.toggle_expand(Path::new("src"));
        assert!(!explorer.is_expanded(Path::new("src")));
    }

    #[test]
    fn read_dir_lists_real_directory() {
        let scratch =
            std::env::temp_dir().join(format!("nexus-explorer-test-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("hello.txt"), b"hi").unwrap();
        std::fs::create_dir_all(scratch.join("nested")).unwrap();
        let entries = Explorer::read_dir(&scratch).unwrap();
        assert_eq!(entries.len(), 2);
        let has_file = entries.iter().any(|e| !e.is_dir && e.name() == "hello.txt");
        let has_dir = entries.iter().any(|e| e.is_dir && e.name() == "nested");
        assert!(has_file);
        assert!(has_dir);
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
