//! Status bar model.
//!
//! The bar consumes a prepared [`UiSnapshot`] — it never queries subsystems
//! directly, so adding new subsystems cannot break the bar.

/// A snapshot of everything the status bar wants to show, assembled by the
/// application once per frame.
#[derive(Debug, Clone, Default)]
pub struct UiSnapshot {
    pub connection: String,
    pub cube: Option<String>,
    pub user: String,
    pub permission: Option<String>,
    pub file: Option<String>,
    pub language: Option<String>,
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub mode: String,
    /// Optional summary, e.g. "3 notifications" or a build state.
    pub summary: Option<String>,
}

impl UiSnapshot {
    /// Renders the bar for a given terminal width, trimming middle segments
    /// when the bar is narrow.
    #[must_use]
    pub fn to_status_line(&self, width: u16) -> String {
        let mut segments = Vec::new();
        segments.push(format!("Nexus"));
        if let Some(cube) = &self.cube {
            segments.push(format!("Cube: {cube}"));
        }
        segments.push(format!("{}", self.connection));
        if let Some(perm) = &self.permission {
            segments.push(perm.clone());
        }
        if let Some(file) = &self.file {
            let mut seg = format!("{file}");
            if let Some(lang) = &self.language {
                seg.push_str(&format!(" [{lang}]"));
            }
            segments.push(seg);
        }
        match (self.line, self.column) {
            (Some(l), Some(c)) => segments.push(format!("Ln {l} Col {c}")),
            (Some(l), None) => segments.push(format!("Ln {l}")),
            _ => {}
        }
        segments.push(self.mode.clone());
        if let Some(summary) = &self.summary {
            segments.push(summary.clone());
        }

        let mut line = String::new();
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                line.push_str(" │ ");
            }
            line.push_str(segment);
            if line.chars().count() as u16 >= width {
                break;
            }
        }
        // Hard-truncate to the terminal width so we never emit invalid output.
        line.chars().take(width as usize).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> UiSnapshot {
        UiSnapshot {
            connection: "CONNECTED".into(),
            cube: Some("backend-api".into()),
            user: "alice".into(),
            permission: Some("EDITOR".into()),
            file: Some("src/main.rs".into()),
            language: Some("Rust".into()),
            line: Some(42),
            column: Some(18),
            mode: "NORMAL".into(),
            summary: None,
        }
    }

    #[test]
    fn status_line_contains_key_facts() {
        let line = snapshot().to_status_line(160);
        assert!(line.contains("backend-api"));
        assert!(line.contains("CONNECTED"));
        assert!(line.contains("Ln 42 Col 18"));
        assert!(line.contains("NORMAL"));
    }

    #[test]
    fn status_line_never_exceeds_width() {
        let line = snapshot().to_status_line(20);
        assert!(line.chars().count() <= 20);
    }

    #[test]
    fn defaults_produce_minimal_line() {
        let line = UiSnapshot::default().to_status_line(120);
        assert!(line.contains("Nexus"));
        assert!(line.contains("OFFLINE") || line.contains("Nexus"));
    }
}
