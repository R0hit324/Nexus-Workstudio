//! Reusable modal dialogs.

use std::collections::VecDeque;

/// Kind of dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Confirm,
    Input,
    Error,
    Info,
}

/// A queued modal dialog. The `action` fires if the user confirms.
#[derive(Debug, Clone)]
pub struct Dialog {
    pub kind: DialogKind,
    pub title: String,
    pub message: String,
    /// Pre-filled value for [`DialogKind::Input`].
    pub default_value: String,
    /// Action executed on confirm.
    pub action: crate::actions::Action,
    /// Optional destructive flag used by themes (Confirm/Delete).
    pub destructive: bool,
}

impl Dialog {
    #[must_use]
    pub fn confirm(
        title: impl Into<String>,
        message: impl Into<String>,
        action: crate::actions::Action,
    ) -> Self {
        Self {
            kind: DialogKind::Confirm,
            title: title.into(),
            message: message.into(),
            default_value: String::new(),
            action,
            destructive: false,
        }
    }

    #[must_use]
    pub fn input(
        title: impl Into<String>,
        prompt: impl Into<String>,
        default_value: impl Into<String>,
        action: crate::actions::Action,
    ) -> Self {
        Self {
            kind: DialogKind::Input,
            title: title.into(),
            message: prompt.into(),
            default_value: default_value.into(),
            action,
            destructive: false,
        }
    }

    #[must_use]
    pub fn error(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: DialogKind::Error,
            title: title.into(),
            message: message.into(),
            default_value: String::new(),
            action: crate::actions::Action::RefreshWorkspace,
            destructive: false,
        }
    }
}

/// Manages the modal stack. Only the top dialog accepts input.
#[derive(Debug, Default)]
pub struct DialogQueue {
    stack: VecDeque<Dialog>,
}

impl DialogQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes a dialog. Confirmation/input revert to the previous dialog.
    pub fn push(&mut self, dialog: Dialog) {
        self.stack.push_back(dialog);
    }

    /// The active dialog, if any.
    #[must_use]
    pub fn active(&self) -> Option<&Dialog> {
        self.stack.back()
    }

    /// Dismisses the top dialog without confirming.
    pub fn cancel(&mut self) {
        self.stack.pop_back();
    }

    /// Pops the top dialog, returning its action when the user confirmed.
    #[must_use]
    pub fn confirm_top(&mut self) -> Option<crate::actions::Action> {
        self.stack.pop_back().map(|d| d.action)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.stack.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Action;
    use nexus_core::ids::FileId;

    #[test]
    fn confirm_pops_and_returns_action() {
        let mut queue = DialogQueue::new();
        let action = Action::DeleteFile(FileId::new());
        queue.push(Dialog::confirm("Delete", "Delete file?", action.clone()));
        assert_eq!(queue.confirm_top(), Some(action));
        assert!(queue.is_empty());
    }

    #[test]
    fn cancel_keeps_previous_dialog() {
        let mut queue = DialogQueue::new();
        queue.push(Dialog::error("first", "top"));
        queue.push(Dialog::error("second", "under"));
        queue.cancel(); // dismiss the top dialog
        assert_eq!(queue.active().unwrap().title, "first");
    }

    #[test]
    fn input_dialog_holds_default() {
        let dialog = Dialog::input("Rename", "New name:", "main.rs", Action::RefreshWorkspace);
        assert_eq!(dialog.default_value, "main.rs");
    }
}
