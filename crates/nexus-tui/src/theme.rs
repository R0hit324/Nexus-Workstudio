//! Theme system: named color/style tables.
//!
//! Widgets reference [`StyleKey`]s only; never literal colors. This allows
//! future custom themes and ensures the whole UI shares one palette. Rendering
//! adapters translate [`StyleSpec`] into the terminal framework's styles.

use std::collections::HashMap;

/// A small ANSI/RGB color vocabulary shared across the codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpec {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    Rgb(u8, u8, u8),
}

/// Decoration flags applied to themed text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoration {
    Bold,
    Italic,
    Underlined,
    Dim,
}

/// A complete text style, independent of the rendering backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleSpec {
    pub fg: Option<ColorSpec>,
    pub bg: Option<ColorSpec>,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
    pub dim: bool,
}

impl StyleSpec {
    #[must_use]
    pub const fn plain() -> Self {
        Self {
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underlined: false,
            dim: false,
        }
    }

    #[must_use]
    pub const fn fg(color: ColorSpec) -> Self {
        Self {
            fg: Some(color),
            ..Self::plain()
        }
    }

    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    #[must_use]
    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    #[must_use]
    pub const fn on(mut self, color: ColorSpec) -> Self {
        self.bg = Some(color);
        self
    }
}

/// Semantic style slot referenced by widgets and the syntax highlighter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleKey {
    Background,
    Foreground,
    Keyword,
    String,
    Number,
    Comment,
    Error,
    Warning,
    Cursor,
    Selection,
    RemoteCursor,
    StatusBar,
    PanelBorder,
    PanelTitle,
    PanelFocused,
    ExplorerFile,
    ExplorerDirectory,
    TabActive,
    TabInactive,
    NotificationInfo,
    NotificationSuccess,
    NotificationWarning,
    NotificationError,
    ChatName,
    ChatText,
    BuildOk,
    BuildFail,
    LineNumber,
    Dimmed,
}

/// A named theme mapping every [`StyleKey`] to a [`StyleSpec`].
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,
    pub styles: HashMap<StyleKey, StyleSpec>,
}

impl Theme {
    /// Looks up a style, falling back to the foreground style when absent.
    #[must_use]
    pub fn style(&self, key: StyleKey) -> StyleSpec {
        self.styles
            .get(&key)
            .copied()
            .unwrap_or_else(|| self.styles[&StyleKey::Foreground])
    }
}

/// The default "Nexus Dark" theme.
#[must_use]
pub fn default_theme() -> Theme {
    use ColorSpec as C;
    use StyleKey as K;
    let mut styles = HashMap::new();
    styles.insert(K::Background, StyleSpec::fg(C::Rgb(22, 24, 30)));
    styles.insert(K::Foreground, StyleSpec::fg(C::Rgb(220, 220, 225)));
    styles.insert(K::Keyword, StyleSpec::fg(C::LightMagenta).bold());
    styles.insert(K::String, StyleSpec::fg(C::Green));
    styles.insert(K::Number, StyleSpec::fg(C::Cyan));
    styles.insert(K::Comment, StyleSpec::fg(C::Gray).dim().italic());
    styles.insert(K::Error, StyleSpec::fg(C::LightRed).bold());
    styles.insert(K::Warning, StyleSpec::fg(C::Yellow));
    styles.insert(K::Cursor, StyleSpec::plain().on(C::Rgb(80, 160, 255)));
    styles.insert(K::Selection, StyleSpec::plain().on(C::Rgb(60, 80, 130)));
    styles.insert(K::RemoteCursor, StyleSpec::plain().on(C::Rgb(140, 90, 200)));
    styles.insert(K::StatusBar, StyleSpec::fg(C::Black).on(C::LightBlue));
    styles.insert(K::PanelBorder, StyleSpec::fg(C::DarkGray));
    styles.insert(K::PanelTitle, StyleSpec::fg(C::LightCyan).bold());
    styles.insert(K::PanelFocused, StyleSpec::fg(C::White).bold());
    styles.insert(K::ExplorerFile, StyleSpec::fg(C::Rgb(220, 220, 225)));
    styles.insert(K::ExplorerDirectory, StyleSpec::fg(C::LightBlue).bold());
    styles.insert(K::TabActive, StyleSpec::fg(C::Black).on(C::Cyan));
    styles.insert(K::TabInactive, StyleSpec::fg(C::Gray));
    styles.insert(K::NotificationInfo, StyleSpec::fg(C::Cyan));
    styles.insert(K::NotificationSuccess, StyleSpec::fg(C::Green).bold());
    styles.insert(K::NotificationWarning, StyleSpec::fg(C::Yellow));
    styles.insert(K::NotificationError, StyleSpec::fg(C::LightRed).bold());
    styles.insert(K::ChatName, StyleSpec::fg(C::LightMagenta).bold());
    styles.insert(K::ChatText, StyleSpec::fg(C::Rgb(220, 220, 225)));
    styles.insert(K::BuildOk, StyleSpec::fg(C::Green).bold());
    styles.insert(K::BuildFail, StyleSpec::fg(C::LightRed).bold());
    styles.insert(K::LineNumber, StyleSpec::fg(C::DarkGray));
    styles.insert(K::Dimmed, StyleSpec::fg(C::Gray));
    Theme {
        name: "nexus-dark",
        styles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::StyleKey as K;

    #[test]
    fn default_theme_has_every_style_key() {
        let theme = default_theme();
        for key in [
            K::Background,
            K::Foreground,
            K::Keyword,
            K::String,
            K::Number,
            K::Comment,
            K::Error,
            K::Cursor,
            K::StatusBar,
            K::RemoteCursor,
            K::PanelBorder,
        ] {
            assert!(theme.styles.contains_key(&key), "missing {key:?}");
        }
    }

    #[test]
    fn unknown_styles_fall_back_to_foreground() {
        let theme = default_theme();
        assert_eq!(
            theme.style(K::Background).fg,
            Some(ColorSpec::Rgb(22, 24, 30))
        );
    }

    #[test]
    fn keyword_is_bold_and_comment_italic() {
        let theme = default_theme();
        assert!(theme.style(K::Keyword).bold);
        assert!(theme.style(K::Comment).italic);
    }
}
