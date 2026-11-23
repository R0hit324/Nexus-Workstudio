//! Layout engine.
//!
//! All geometry is computed here from the terminal size and panel visibility.
//! Widgets receive pre-computed rectangles; nothing hardcodes coordinates.
//! The math is pure (no terminal dependencies) so it is unit-tested directly.

use std::collections::HashMap;

/// A rectangle in terminal cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    #[must_use]
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[must_use]
    pub const fn area(self) -> u32 {
        self.width as u32 * self.height as u32
    }

    /// Restricts `rect` to lie inside this rectangle.
    #[must_use]
    pub fn clip(self, rect: Rect) -> Rect {
        let ax = rect.x.max(self.x);
        let ay = rect.y.max(self.y);
        let ar = (rect.x + rect.width).min(self.x + self.width);
        let ab = (rect.y + rect.height).min(self.y + self.height);
        let width = ar.saturating_sub(ax);
        let height = ab.saturating_sub(ay);
        Rect::new(ax, ay, width, height)
    }

    /// A horizontally-split slice starting at `top` of `height` rows.
    #[must_use]
    pub fn slice(self, top: u16, height: u16) -> Rect {
        Rect::new(
            self.x,
            self.y + top,
            self.width,
            height.min(self.height.saturating_sub(top)),
        )
    }

    /// A left slice of `width` columns.
    #[must_use]
    pub fn slice_left(self, width: u16) -> Rect {
        Rect::new(self.x, self.y, width.min(self.width), self.height)
    }

    /// The remaining area to the right of a `left` slice of this rect.
    #[must_use]
    pub fn remainder_right(self, left_width: u16) -> Rect {
        let x = self.x.saturating_add(left_width);
        Rect::new(
            x,
            self.y,
            self.width.saturating_sub(left_width),
            self.height,
        )
    }

    /// The top part of `height` rows, leaving the rest as bottom remainder.
    #[must_use]
    pub fn remainder_bottom(self, top_height: u16) -> Rect {
        let y = self.y.saturating_add(top_height);
        Rect::new(
            self.x,
            y,
            self.width,
            self.height.saturating_sub(top_height),
        )
    }

    /// Checks the rectangle is fully inside `outer` with positive size.
    #[must_use]
    pub fn contained_in(self, outer: Rect) -> bool {
        self.x >= outer.x
            && self.y >= outer.y
            && self.x + self.width <= outer.x + outer.width
            && self.y + self.height <= outer.y + outer.height
            && self.width > 0
            && self.height > 0
    }
}

/// Identifyer of a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    Explorer,
    Editor,
    Terminal,
    Users,
    Chat,
    BuildOutput,
}

impl PanelId {
    pub const ALL: [PanelId; 6] = [
        Self::Explorer,
        Self::Editor,
        Self::Terminal,
        Self::Users,
        Self::Chat,
        Self::BuildOutput,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Editor => "editor",
            Self::Terminal => "terminal",
            Self::Users => "users",
            Self::Chat => "chat",
            Self::BuildOutput => "build",
        }
    }
}

/// Which side panels are currently visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PanelToggles {
    pub explorer: bool,
    pub terminal: bool,
    pub users: bool,
    pub chat: bool,
    pub build: bool,
}

impl PanelToggles {
    /// Bottom panel visible if any bottom slot is toggled.
    #[must_use]
    fn has_bottom(self) -> bool {
        self.terminal || self.chat || self.build
    }

    /// Count of visible bottom panels.
    #[must_use]
    fn bottom_count(self) -> u16 {
        let mut n = 0;
        if self.chat {
            n += 1;
        }
        if self.terminal {
            n += 1;
        }
        if self.build {
            n += 1;
        }
        n
    }
}

/// The complete set of rectangles handed to widgets for one frame.
#[derive(Debug, Clone, Copy)]
pub struct LayoutMap {
    pub header: Rect,
    pub explorer: Option<Rect>,
    pub editor: Rect,
    pub users: Option<Rect>,
    pub terminal: Option<Rect>,
    pub chat: Option<Rect>,
    pub build: Option<Rect>,
    pub status: Rect,
    /// Overlay centered above everything while a palette is open.
    pub palette: Option<Rect>,
}

/// Whether every non-editor panel is hidden (terminal too small).
#[must_use]
pub fn all_optional_hidden(map: &LayoutMap) -> bool {
    map.explorer.is_none()
        && map.users.is_none()
        && map.chat.is_none()
        && map.terminal.is_none()
        && map.build.is_none()
}

/// Computes the workspace layout for a terminal of `cols` x `rows`.
#[must_use]
pub fn layout_main(cols: u16, rows: u16, toggles: PanelToggles) -> LayoutMap {
    let total = Rect::new(0, 0, cols, rows);
    let header = Rect::new(0, 0, cols, 1.min(rows));
    let status = Rect::new(0, rows.saturating_sub(1), cols, 1.min(rows));
    let content = Rect::new(0, 1.min(rows), cols, rows.saturating_sub(2));

    let palette = palette_overlay(total);
    let compact = rows < 8 || cols < 40;
    if compact {
        return LayoutMap {
            header,
            explorer: None,
            editor: content,
            users: None,
            terminal: None,
            chat: None,
            build: None,
            status,
            palette,
        };
    }

    // Reserve a bottom strip when any bottom panel is visible.
    let bottom_h = if toggles.has_bottom() {
        content.height.min(12).max(6)
    } else {
        0
    };
    let top_area = content.slice(0, content.height.saturating_sub(bottom_h));
    let bottom_area = content.remainder_bottom(top_area.height);

    // Top band: explorer | editor | users.
    let (explorer, editor, users) = top_split(top_area, toggles);

    // Bottom band: chat | terminal | build.
    let (chat, terminal, build) = bottom_split(bottom_area, toggles);

    LayoutMap {
        header,
        explorer,
        editor,
        users,
        terminal,
        chat,
        build,
        status,
        palette,
    }
}

fn top_split(area: Rect, toggles: PanelToggles) -> (Option<Rect>, Rect, Option<Rect>) {
    let left = if toggles.explorer {
        let w = (area.width * 22 / 100).max(18).min(area.width * 40 / 100);
        Some(area.slice_left(w))
    } else {
        None
    };
    let middle = left.map(|r| area.remainder_right(r.width)).unwrap_or(area);
    let right = if toggles.users {
        let w = (area.width / 5).max(15).min(22).min(middle.width / 2);
        let right_rect = Rect::new(
            middle.x + middle.width.saturating_sub(w),
            middle.y,
            w,
            middle.height,
        );
        Some(right_rect)
    } else {
        None
    };
    let editor = right
        .map(|r| {
            // Editor takes whatever is ~left-of the users panel.
            Rect::new(
                middle.x,
                middle.y,
                r.x.saturating_sub(middle.x),
                middle.height,
            )
        })
        .unwrap_or(middle);
    (left, editor, right)
}

fn bottom_split(area: Rect, toggles: PanelToggles) -> (Option<Rect>, Option<Rect>, Option<Rect>) {
    let count = toggles.bottom_count();
    if count == 0 {
        return (None, None, None);
    }
    let slot = area.width / count;
    let mut x = area.x;
    let mut take = |want: bool| -> Option<Rect> {
        if !want {
            return None;
        }
        let rect = Rect::new(x, area.y, slot, area.height);
        x += slot;
        Some(rect)
    };
    let chat = take(toggles.chat);
    let terminal = take(toggles.terminal);
    let build = take(toggles.build);
    (chat, terminal, build)
}

fn palette_overlay(total: Rect) -> Option<Rect> {
    if total.width < 30 || total.height < 8 {
        return None;
    }
    let width = (total.width * 60 / 100).min(80);
    let height = (total.height * 40 / 100).min(20).max(6);
    let x = total.x + (total.width.saturating_sub(width)) / 2;
    let y = total.y + (total.height.saturating_sub(height)) / 3;
    Some(Rect::new(x, y, width, height))
}

/// Focus order over visible panels; wraps around.
#[derive(Debug, Clone)]
pub struct FocusGraph {
    order: Vec<PanelId>,
}

impl FocusGraph {
    /// Builds the traversal order from visibility toggles.
    #[must_use]
    pub fn new(toggles: PanelToggles) -> Self {
        let mut order = Vec::new();
        if toggles.explorer {
            order.push(PanelId::Explorer);
        }
        order.push(PanelId::Editor);
        if toggles.users {
            order.push(PanelId::Users);
        }
        if toggles.chat {
            order.push(PanelId::Chat);
        }
        if toggles.terminal {
            order.push(PanelId::Terminal);
        }
        if toggles.build {
            order.push(PanelId::BuildOutput);
        }
        Self { order }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// The panel after `current` in focus order (wraps).
    #[must_use]
    pub fn next(&self, current: Option<PanelId>) -> Option<PanelId> {
        if self.order.is_empty() {
            return None;
        }
        let idx = current
            .and_then(|c| self.order.iter().position(|p| *p == c))
            .map_or(0, |i| (i + 1) % self.order.len());
        Some(self.order[idx])
    }

    /// The panel before `current` in focus order (wraps).
    #[must_use]
    pub fn prev(&self, current: Option<PanelId>) -> Option<PanelId> {
        if self.order.is_empty() {
            return None;
        }
        let idx = current
            .and_then(|c| self.order.iter().position(|p| *p == c))
            .map_or(0, |i| (i + self.order.len() - 1) % self.order.len());
        Some(self.order[idx])
    }
}

/// Per-panel tracking used by the app: visibility + focus + last layout rect.
#[derive(Debug, Default)]
pub struct PanelRuntime {
    pub toggles: PanelToggles,
    pub focus: Option<PanelId>,
    pub rects: HashMap<PanelId, Rect>,
}

impl PanelRuntime {
    /// Advances focus by one panel.
    pub fn focus_next(&mut self) {
        let graph = FocusGraph::new(self.toggles);
        self.focus = graph.next(self.focus);
    }

    /// Moves focus explicitly.
    pub fn set_focus(&mut self, id: PanelId) {
        self.focus = Some(id);
    }

    /// Recomputes the layout for the current size and stores rects.
    pub fn layout(&mut self, cols: u16, rows: u16) -> LayoutMap {
        let map = layout_main(cols, rows, self.toggles);
        for id in PanelId::ALL {
            let rect = match id {
                PanelId::Explorer => map.explorer,
                PanelId::Terminal => map.terminal,
                PanelId::Users => map.users,
                PanelId::Chat => map.chat,
                PanelId::BuildOutput => map.build,
                PanelId::Editor => Some(map.editor),
            };
            if let Some(rect) = rect {
                self.rects.insert(id, rect);
            }
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_clip_and_slices_are_exact() {
        let outer = Rect::new(0, 0, 80, 24);
        let left = outer.slice_left(20);
        assert_eq!(left, Rect::new(0, 0, 20, 24));
        let right = outer.remainder_right(20);
        assert_eq!(right, Rect::new(20, 0, 60, 24));
        let mid = outer.slice(2, 5);
        assert_eq!(mid, Rect::new(0, 2, 80, 5));
        assert!(left.contained_in(outer));
    }

    #[test]
    fn compact_terminals_hide_optional_panels() {
        let map = layout_main(
            30,
            5,
            PanelToggles {
                explorer: true,
                users: true,
                chat: true,
                ..Default::default()
            },
        );
        assert!(all_optional_hidden(&map));
        assert!(map.editor.area() > 0);
    }

    #[test]
    fn standard_layout_has_no_overlap() {
        let map = layout_main(
            120,
            40,
            PanelToggles {
                explorer: true,
                terminal: true,
                users: true,
                chat: true,
                ..Default::default()
            },
        );
        let all: Vec<Rect> = [map.explorer, map.users, map.chat, map.terminal, map.build]
            .into_iter()
            .flatten()
            .chain(std::iter::once(map.editor))
            .collect();
        for a in &all {
            assert!(
                a.contained_in(Rect::new(0, 0, 120, 40)),
                "panel offscreen: {a:?}"
            );
            for b in &all {
                if a != b {
                    assert!(
                        a.x >= b.x + b.width
                            || b.x >= a.x + a.width
                            || a.y >= b.y + b.height
                            || b.y >= a.y + a.height,
                        "overlap {a:?} with {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn focus_graph_traverses_visible_panels_only() {
        let graph = FocusGraph::new(PanelToggles {
            explorer: true,
            terminal: true,
            ..Default::default()
        });
        assert_eq!(graph.next(None), Some(PanelId::Explorer));
        assert_eq!(graph.next(Some(PanelId::Explorer)), Some(PanelId::Editor));
        assert_eq!(graph.next(Some(PanelId::Terminal)), Some(PanelId::Explorer)); // wraps
        assert_eq!(graph.prev(Some(PanelId::Explorer)), Some(PanelId::Terminal));
    }

    #[test]
    fn empty_focus_graph_is_safe() {
        let graph = FocusGraph::new(PanelToggles::default());
        assert_eq!(graph.len(), 1); // editor always present
        assert_eq!(graph.next(None), Some(PanelId::Editor));
    }
}
