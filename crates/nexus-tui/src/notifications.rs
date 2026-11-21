//! Bounded notification center.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::MAX_NOTIFICATIONS;

/// Severity of a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    Info,
    Success,
    Warning,
    Error,
    Progress,
}

impl NotificationKind {
    /// Stable machine name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Progress => "progress",
        }
    }
}

/// A single notifiable message.
#[derive(Debug, Clone)]
pub struct Notification {
    pub seq: u64,
    pub kind: NotificationKind,
    pub text: String,
    pub created_at: Instant,
    pub expires_after: Option<Duration>,
}

impl Notification {
    #[must_use]
    pub fn expired(&self, now: Instant) -> bool {
        matches!(self.expires_after, Some(t) if now.duration_since(self.created_at) >= t)
    }
}

/// Queues notifications, expiring them automatically and keeping a bounded
/// history.
#[derive(Debug)]
pub struct NotificationCenter {
    pub(crate) items: VecDeque<Notification>,
    next_seq: u64,
    /// Default lifetime for notifications without an explicit one.
    pub ttl: Duration,
}

impl Default for NotificationCenter {
    fn default() -> Self {
        Self {
            items: VecDeque::new(),
            next_seq: 1,
            ttl: Duration::from_secs(8),
        }
    }
}

impl NotificationCenter {
    /// Emits a notification with the default TTL.
    pub fn push(&mut self, kind: NotificationKind, text: impl Into<String>) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.items.push_back(Notification {
            seq,
            kind,
            text: text.into(),
            created_at: Instant::now(),
            expires_after: Some(self.ttl),
        });
        self.trim();
    }

    /// Emits a sticky (never-expiring) notification; replaces same-kind sticky.
    pub fn push_sticky(&mut self, kind: NotificationKind, text: impl Into<String>) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.items.push_back(Notification {
            seq,
            kind,
            text: text.into(),
            created_at: Instant::now(),
            expires_after: None,
        });
        self.trim();
    }

    /// Removes expired notifications, keeping the newest `MAX_NOTIFICATIONS`.
    pub fn sweep(&mut self) {
        let now = Instant::now();
        self.items.retain(|n| !n.expired(now));
        self.trim();
    }

    /// Dismisses an explicit notification by sequence number.
    pub fn dismiss(&mut self, seq: u64) {
        self.items.retain(|n| n.seq != seq);
    }

    fn trim(&mut self) {
        while self.items.len() > MAX_NOTIFICATIONS {
            self.items.pop_front();
        }
    }

    /// Iterates newest-first (for rendering).
    pub fn newest_first(&self) -> impl Iterator<Item = &Notification> {
        self.items.iter().rev()
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

    #[test]
    fn pushes_and_bounds_history() {
        let mut center = NotificationCenter::default();
        center.ttl = Duration::from_secs(60);
        for i in 0..(MAX_NOTIFICATIONS + 20) {
            center.push(NotificationKind::Info, format!("m{i}"));
        }
        assert_eq!(center.len(), MAX_NOTIFICATIONS);
        // Oldest were dropped; newest (last push) is present.
        assert!(
            center.newest_first().next().unwrap().text == format!("m{}", MAX_NOTIFICATIONS + 19)
        );
    }

    #[test]
    fn sticky_and_expiry_lifecycle() {
        let mut center = NotificationCenter::default();
        center.push_sticky(NotificationKind::Error, "connection lost");
        assert_eq!(center.len(), 1);
        center.sweep(); // does not expire stickies
        assert_eq!(center.len(), 1);
        let seq = center.newest_first().next().unwrap().seq;
        center.dismiss(seq);
        assert_eq!(center.len(), 0);
    }

    #[test]
    fn expired_notifications_are_cleaned() {
        let mut center = NotificationCenter::default();
        center.push(NotificationKind::Info, "x");
        // Force expiry by lowering the deadline created in the past.
        let mut n = center.items.pop_front().unwrap();
        n.created_at = Instant::now() - Duration::from_secs(60);
        n.expires_after = Some(Duration::from_millis(1));
        center.items.push_back(n);
        center.sweep();
        assert_eq!(center.len(), 0);
    }
}
