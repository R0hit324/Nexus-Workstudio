//! Terminal rendering.
//!
//! Provides terminal rendering with ANSI escape sequence parsing and
//! rendering to a cell grid.

use std::collections::VecDeque;

/// Terminal cell with character and attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub char: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub inverse: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            char: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            inverse: false,
        }
    }
}

impl Cell {
    pub fn new(char: char) -> Self {
        Self {
            char,
            ..Default::default()
        }
    }

    pub fn with_fg(mut self, fg: Color) -> Self {
        self.fg = fg;
        self
    }

    pub fn with_bg(mut self, bg: Color) -> Self {
        self.bg = bg;
        self
    }
}

/// Terminal colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    Default,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

impl Default for Color {
    fn default() -> Self {
        Self::Default
    }
}

/// Terminal cursor position and state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub visible: bool,
    pub shape: CursorShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

/// Terminal render configuration.
#[derive(Debug, Clone)]
pub struct RenderConfig {
    pub size: nexus_core::TerminalSize,
    pub scrollback_limit: usize,
    pub tab_width: usize,
    pub bell: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            size: nexus_core::TerminalSize::default(),
            scrollback_limit: 10000,
            tab_width: 8,
            bell: true,
        }
    }
}

/// Terminal renderer with cell grid and scrollback.
pub struct TerminalRenderer {
    config: RenderConfig,
    grid: Vec<Vec<Cell>>,
    scrollback: VecDeque<Vec<Cell>>,
    cursor: Cursor,
    alt_screen: bool,
    alt_grid: Option<Vec<Vec<Cell>>>,
    alt_cursor: Option<Cursor>,
    saved_cursor: Option<Cursor>,
    scroll_region: (usize, usize),
    attributes: CellAttributes,
}

#[derive(Debug, Clone, Copy, Default)]
struct CellAttributes {
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    inverse: bool,
    fg: Color,
    bg: Color,
}

impl TerminalRenderer {
    pub fn new(config: RenderConfig) -> Self {
        let size = config.size;
        let grid = vec![vec![Cell::default(); size.cols as usize]; size.rows as usize];

        Self {
            config,
            grid,
            scrollback: VecDeque::new(),
            cursor: Cursor::default(),
            alt_screen: false,
            alt_grid: None,
            alt_cursor: None,
            saved_cursor: None,
            scroll_region: (0, size.rows as usize - 1),
            attributes: CellAttributes::default(),
        }
    }

    /// Gets the current terminal size.
    pub fn size(&self) -> nexus_core::TerminalSize {
        self.config.size
    }

    /// Resizes the terminal.
    pub fn resize(&mut self, size: nexus_core::TerminalSize) {
        self.config.size = size;

        // Resize main grid
        let mut new_grid = vec![vec![Cell::default(); size.cols as usize]; size.rows as usize];

        // Copy existing content
        for row in 0..self.grid.len().min(size.rows as usize) {
            for col in 0..self.grid[row].len().min(size.cols as usize) {
                new_grid[row][col] = self.grid[row][col];
            }
        }
        self.grid = new_grid;

        // Resize alt screen if active
        if let Some(alt) = &mut self.alt_grid {
            let mut new_alt = vec![vec![Cell::default(); size.cols as usize]; size.rows as usize];
            for row in 0..alt.len().min(size.rows as usize) {
                for col in 0..alt[row].len().min(size.cols as usize) {
                    new_alt[row][col] = alt[row][col];
                }
            }
            *alt = new_alt;
        }

        // Clamp cursor
        self.cursor.row = self.cursor.row.min(size.rows as usize - 1);
        self.cursor.col = self.cursor.col.min(size.cols as usize - 1);

        self.scroll_region = (0, size.rows as usize - 1);
    }

    /// Gets a mutable reference to the current cell.
    fn current_cell(&mut self) -> &mut Cell {
        let row = self.cursor.row.min(self.grid.len() - 1);
        let col = self.cursor.col.min(self.grid[row].len() - 1);
        &mut self.grid[row][col]
    }

    /// Writes a character at the cursor position.
    pub fn write_char(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }

        let attrs = self.attributes;
        let cell = self.current_cell();
        *cell = Cell::new(ch).with_fg(attrs.fg).with_bg(attrs.bg);

        cell.bold = attrs.bold;
        cell.italic = attrs.italic;
        cell.underline = attrs.underline;
        cell.strikethrough = attrs.strikethrough;
        cell.inverse = attrs.inverse;

        self.move_cursor_right(1);
    }

    /// Writes a string at the cursor position.
    pub fn write_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.write_char(ch);
        }
    }

    /// Moves cursor right by n columns.
    fn move_cursor_right(&mut self, n: usize) {
        let max_col = self.config.size.cols as usize - 1;
        self.cursor.col = (self.cursor.col + n).min(max_col);

        if self.cursor.col > max_col {
            self.cursor.col = 0;
            self.move_cursor_down(1);
        }
    }

    /// Moves cursor down by n rows.
    fn move_cursor_down(&mut self, n: usize) {
        let max_row = self.config.size.rows as usize - 1;
        self.cursor.row = (self.cursor.row + n).min(max_row);

        if self.cursor.row > max_row {
            self.scroll_up(1);
        }
    }

    /// Moves cursor up by n rows.
    #[allow(dead_code)]
    fn move_cursor_up(&mut self, n: usize) {
        self.cursor.row = self.cursor.row.saturating_sub(n);
    }

    /// Moves cursor to absolute position.
    pub fn move_cursor_to(&mut self, row: usize, col: usize) {
        let max_row = self.config.size.rows as usize - 1;
        let max_col = self.config.size.cols as usize - 1;
        self.cursor.row = row.min(max_row);
        self.cursor.col = col.min(max_col);
    }

    /// Scrolls the screen up by n lines.
    fn scroll_up(&mut self, n: usize) {
        if n == 0 {
            return;
        }

        let top = self.scroll_region.0;
        let bottom = self.scroll_region.1;

        for _ in 0..n {
            // Save top line to scrollback
            let top_line = self.grid[top].clone();
            if self.scrollback.len() >= self.config.scrollback_limit {
                self.scrollback.pop_front();
            }
            self.scrollback.push_back(top_line);

            // Shift lines up
            for row in top..bottom {
                self.grid[row] = self.grid[row + 1].clone();
            }

            // Clear bottom line
            self.grid[bottom] = vec![Cell::default(); self.config.size.cols as usize];
        }
    }

    /// Scrolls the screen down by n lines.
    #[allow(dead_code)]
    fn scroll_down(&mut self, n: usize) {
        if n == 0 {
            return;
        }

        let top = self.scroll_region.0;
        let bottom = self.scroll_region.1;

        for _ in 0..n {
            let _bottom_line = self.grid[bottom].clone();

            for row in (top + 1..=bottom).rev() {
                self.grid[row] = self.grid[row - 1].clone();
            }

            self.grid[top] = vec![Cell::default(); self.config.size.cols as usize];
        }
    }

    /// Clears the screen.
    pub fn clear_screen(&mut self) {
        for row in 0..self.config.size.rows as usize {
            self.grid[row] = vec![Cell::default(); self.config.size.cols as usize];
        }
        self.cursor = Cursor::default();
    }

    /// Clears from cursor to end of screen.
    pub fn clear_to_end_of_screen(&mut self) {
        let row = self.cursor.row;
        let col = self.cursor.col;

        // Clear current line from cursor
        for c in col..self.config.size.cols as usize {
            self.grid[row][c] = Cell::default();
        }

        // Clear remaining lines
        for r in (row + 1)..self.config.size.rows as usize {
            self.grid[r] = vec![Cell::default(); self.config.size.cols as usize];
        }
    }

    /// Clears from cursor to beginning of screen.
    pub fn clear_to_beginning_of_screen(&mut self) {
        let row = self.cursor.row;
        let col = self.cursor.col;

        // Clear current line up to cursor
        for c in 0..=col {
            self.grid[row][c] = Cell::default();
        }

        // Clear previous lines
        for r in 0..row {
            self.grid[r] = vec![Cell::default(); self.config.size.cols as usize];
        }
    }

    /// Clears current line.
    pub fn clear_line(&mut self) {
        self.grid[self.cursor.row] = vec![Cell::default(); self.config.size.cols as usize];
    }

    /// Clears from cursor to end of line.
    pub fn clear_to_end_of_line(&mut self) {
        let col = self.cursor.col;
        for c in col..self.config.size.cols as usize {
            self.grid[self.cursor.row][c] = Cell::default();
        }
    }

    /// Clears from cursor to beginning of line.
    pub fn clear_to_beginning_of_line(&mut self) {
        let col = self.cursor.col;
        for c in 0..=col {
            self.grid[self.cursor.row][c] = Cell::default();
        }
    }

    /// Sets the scroll region.
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        self.scroll_region = (
            top.min(self.config.size.rows as usize - 1),
            bottom.min(self.config.size.rows as usize - 1),
        );
    }

    /// Enters alternate screen mode.
    pub fn enter_alt_screen(&mut self) {
        if !self.alt_screen {
            self.alt_grid = Some(self.grid.clone());
            self.alt_cursor = Some(self.cursor);
            self.grid = vec![
                vec![Cell::default(); self.config.size.cols as usize];
                self.config.size.rows as usize
            ];
            self.cursor = Cursor::default();
            self.alt_screen = true;
        }
    }

    /// Exits alternate screen mode.
    pub fn exit_alt_screen(&mut self) {
        if self.alt_screen {
            if let Some(alt_grid) = self.alt_grid.take() {
                self.grid = alt_grid;
            }
            if let Some(alt_cursor) = self.alt_cursor.take() {
                self.cursor = alt_cursor;
            }
            self.alt_screen = false;
        }
    }

    /// Saves cursor position.
    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some(self.cursor);
    }

    /// Restores cursor position.
    pub fn restore_cursor(&mut self) {
        if let Some(saved) = self.saved_cursor {
            self.cursor = saved;
        }
    }

    /// Sets a rendering attribute.
    pub fn set_attribute(&mut self, attr: Attribute) {
        match attr {
            Attribute::Bold => self.attributes.bold = true,
            Attribute::Italic => self.attributes.italic = true,
            Attribute::Underline => self.attributes.underline = true,
            Attribute::Strikethrough => self.attributes.strikethrough = true,
            Attribute::Inverse => self.attributes.inverse = true,
            Attribute::Reset => {
                self.attributes = CellAttributes::default();
            }
            Attribute::FgColor(color) => self.attributes.fg = color,
            Attribute::BgColor(color) => self.attributes.bg = color,
            Attribute::ResetFg => self.attributes.fg = Color::Default,
            Attribute::ResetBg => self.attributes.bg = Color::Default,
        }
    }

    /// Gets the current grid for rendering.
    pub fn grid(&self) -> &Vec<Vec<Cell>> {
        &self.grid
    }

    /// Gets the cursor position.
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Gets the scrollback buffer.
    pub fn scrollback(&self) -> &VecDeque<Vec<Cell>> {
        &self.scrollback
    }
}

/// Terminal attributes for SGR sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attribute {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Inverse,
    Reset,
    FgColor(Color),
    BgColor(Color),
    ResetFg,
    ResetBg,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_renderer_creation() {
        let renderer = TerminalRenderer::new(RenderConfig::default());
        assert_eq!(renderer.size(), nexus_core::TerminalSize::default());
    }

    #[test]
    fn test_write_char() {
        let mut renderer = TerminalRenderer::new(RenderConfig::default());
        renderer.write_char('A');
        assert_eq!(renderer.cursor.col, 1);
        assert_eq!(renderer.grid[0][0].char, 'A');
    }

    #[test]
    fn test_resize() {
        let mut renderer = TerminalRenderer::new(RenderConfig::default());
        renderer.resize(nexus_core::TerminalSize {
            rows: 40,
            cols: 120,
        });
        assert_eq!(renderer.size().rows, 40);
        assert_eq!(renderer.size().cols, 120);
    }
}
