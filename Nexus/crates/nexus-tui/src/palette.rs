//! Command palette with fuzzy matching.
//!
//! Matching is pure so it is fully unit-testable without a terminal.

use crate::commands::{Command, CommandRegistry};

/// A scored fuzzy match of `query` inside `candidate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Indices of matched characters within the candidate (0-based).
    pub indices: Vec<usize>,
    /// Lower score is better.
    pub score: u32,
}

/// A subsequence fuzzy matcher with a light penalty scheme.
#[must_use]
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let query: Vec<_> = query.chars().collect();
    if query.is_empty() {
        return None;
    }
    let cand: Vec<_> = candidate.chars().collect();
    let ql = query.len();
    let cl = cand.len();
    if ql > cl {
        return None;
    }

    // Dynamic programming over (query prefix, candidate position) minimizing
    // the total gap cost. O(n*m) worst case; queries are tiny.
    let mut dp = vec![u32::MAX; cl + 1];
    let mut prev = vec![u32::MAX; cl + 1];
    prev[0] = 0;
    let mut matches: Vec<Vec<usize>> = vec![Vec::new(); cl + 1];

    for (qi, qc) in query.iter().enumerate() {
        for (ci, c) in cand.iter().enumerate() {
            if qc.eq_ignore_ascii_case(c) && prev[ci] != u32::MAX {
                let mut cost = prev[ci];
                if qi > 0 {
                    cost += gap_penalty(matches[ci].last().copied(), ci);
                }
                if cost < dp[ci + 1] {
                    dp[ci + 1] = cost;
                    let mut path = matches[ci].clone();
                    path.push(ci);
                    matches[ci + 1] = path;
                }
            }
            if dp[ci] < dp[ci + 1] {
                dp[ci + 1] = dp[ci];
                matches[ci + 1] = matches[ci].clone();
            }
        }
        // Roll the DP window: next round reads from the previous row.
        std::mem::swap(&mut dp, &mut prev);
        dp.fill(u32::MAX);
    }

    if prev[cl] == u32::MAX {
        return None;
    }
    let score = prev[cl] + match_bonus(&matches[cl]);
    Some(FuzzyMatch {
        indices: matches[cl].clone(),
        score,
    })
}

fn gap_penalty(previous: Option<usize>, current: usize) -> u32 {
    match previous {
        None => 0,
        Some(p) if current == p + 1 => 0, // contiguous run: free
        Some(p) => (current - p - 1) as u32 * 2,
    }
}

/// Prefer matches that land on word/capital boundaries: lower score is better.
fn match_bonus(indices: &[usize]) -> u32 {
    indices.iter().filter(|&&i| i == 0).count() as u32
}

/// Palette over the command registry. Tracks the query and the selected row.
#[derive(Debug, Clone)]
pub struct CommandPalette {
    pub query: String,
    pub cursor: usize,
    /// Cache of matched commands, refreshed on query change.
    pub results: Vec<PaletteResult>,
}

#[derive(Debug, Clone)]
pub struct PaletteResult {
    pub command: Command,
    pub title: &'static str,
    pub match_: Option<FuzzyMatch>,
}

impl CommandPalette {
    /// An empty palette.
    #[must_use]
    pub fn new() -> Self {
        Self {
            query: String::new(),
            cursor: 0,
            results: Vec::new(),
        }
    }

    /// Recomputes fuzzy results from `registry`, keeping the cursor in range.
    pub fn refresh(&mut self, registry: &CommandRegistry) {
        let query = self.query.trim();
        let mut results: Vec<PaletteResult> = registry
            .all()
            .filter_map(|(command, spec)| {
                let match_ = if query.is_empty() {
                    None
                } else {
                    Some(fuzzy_match(query, spec.title)?)
                };
                Some(PaletteResult {
                    command: *command,
                    title: spec.title,
                    match_,
                })
            })
            .collect();
        results.sort_by_key(|r| r.match_.as_ref().map(|m| m.score).unwrap_or(u32::MAX));
        self.results = results;
        self.cursor = self.cursor.min(self.results.len().saturating_sub(1));
    }

    /// Moves the selection.
    pub fn move_selection(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        let len = self.results.len() as isize;
        self.cursor = ((self.cursor as isize + delta).rem_euclid(len)) as usize;
    }

    /// The currently selected command, if any.
    #[must_use]
    pub fn selected(&self) -> Option<Command> {
        self.results.get(self.cursor).map(|r| r.command)
    }
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matches_subsequences() {
        let m = fuzzy_match("sv", "Save").unwrap();
        assert!(!m.indices.is_empty());
    }

    #[test]
    fn fuzzy_rejects_non_matches() {
        assert!(fuzzy_match("zzz", "Save").is_none());
        assert!(fuzzy_match("", "Save").is_none());
    }

    #[test]
    fn contiguous_matches_score_better_than_gappy() {
        let tight = fuzzy_match("sv", "SaveFile").unwrap();
        let loose = fuzzy_match("sf", "SearchFocus").unwrap();
        assert!(tight.score <= loose.score);
    }

    #[test]
    fn palette_filters_and_selects() {
        let reg = CommandRegistry::default();
        let mut palette = CommandPalette::new();
        palette.query = "toggle".into();
        palette.refresh(&reg);
        assert!(!palette.results.is_empty());
        assert!(palette
            .results
            .iter()
            .all(|r| r.title.to_ascii_lowercase().contains("toggle")));

        palette.move_selection(1);
        assert!(palette.selected().is_some());
        palette.move_selection(-5);
        assert!(palette.selected().is_some()); // wraps without panic
    }
}
