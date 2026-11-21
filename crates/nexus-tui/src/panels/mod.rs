//! Panel abstraction.
//!
//! Panels are the smallest renderable units. They render into a rectangle and
//! consume application events, but must never talk to the network layer.

use ratatui::layout::Rect as RRect;
use ratatui::Frame;

use crate::input::UiEvent;
use crate::layout::PanelId;
use crate::theme::Theme;

/// A modular, independently renderable UI panel.
pub trait Panel {
    /// Stable identity.
    fn id(&self) -> PanelId;
    /// Title shown in the panel border and on the status bar.
    fn title(&self) -> &str;

    /// Draws the panel into `area`. Widgets must render inside `area`.
    fn render(&self, frame: &mut Frame, area: RRect, theme: &Theme, focused: bool);

    /// Consumes an application event. Only focused panels normally receive
    /// key events; mouse events are routed by hit-testing in the application.
    fn handle_event(&mut self, _event: &UiEvent) {}
}

/// Type alias for stored panels.
pub type BoxPanel = Box<dyn Panel>;
