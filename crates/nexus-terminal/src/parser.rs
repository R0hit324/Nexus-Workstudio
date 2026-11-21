//! ANSI escape sequence parser.
//!
//! Parses terminal escape sequences and converts them into structured events.

/// ANSI parser for terminal escape sequences.
pub struct AnsiParser {
    state: ParserState,
    params: Vec<i64>,
    intermediate: Vec<u8>,
    #[allow(dead_code)]
    in_escape: bool,
    #[allow(dead_code)]
    in_csi: bool,
    #[allow(dead_code)]
    in_osc: bool,
    osc_buffer: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserState {
    Ground,
    Escape,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    #[allow(dead_code)]
    CsiIgnore,
    OscString,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    #[allow(dead_code)]
    DcsIgnore,
    SosPmApcString,
}

impl Default for AnsiParser {
    fn default() -> Self {
        Self::new()
    }
}

impl AnsiParser {
    pub fn new() -> Self {
        Self {
            state: ParserState::Ground,
            params: Vec::new(),
            intermediate: Vec::new(),
            in_escape: false,
            in_csi: false,
            in_osc: false,
            osc_buffer: Vec::new(),
        }
    }

    /// Parses a byte and returns any complete ANSI events.
    pub fn parse(&mut self, byte: u8) -> Vec<AnsiEvent> {
        let mut events = Vec::new();

        match self.state {
            ParserState::Ground => {
                if byte == 0x1B {
                    // ESC
                    self.state = ParserState::Escape;
                } else if byte <= 0x1F || byte == 0x7F {
                    events.push(AnsiEvent::Control(byte));
                } else {
                    events.push(AnsiEvent::Print(byte as char));
                }
            }
            ParserState::Escape => match byte {
                b'[' => {
                    self.state = ParserState::CsiEntry;
                    self.params.clear();
                    self.intermediate.clear();
                }
                b']' => {
                    self.state = ParserState::OscString;
                    self.osc_buffer.clear();
                }
                b'P' => {
                    self.state = ParserState::DcsEntry;
                    self.params.clear();
                    self.intermediate.clear();
                }
                b'_' => self.state = ParserState::SosPmApcString,
                b'^' => self.state = ParserState::SosPmApcString,
                b'X' => self.state = ParserState::SosPmApcString,
                b'c' => events.push(AnsiEvent::Reset),
                b'7' => events.push(AnsiEvent::SaveCursor),
                b'8' => events.push(AnsiEvent::RestoreCursor),
                b'H' => events.push(AnsiEvent::TabSet),
                b'D' => events.push(AnsiEvent::Index),
                b'E' => events.push(AnsiEvent::NextLine),
                b'M' => events.push(AnsiEvent::ReverseIndex),
                _ if (0x30..=0x4F).contains(&byte) => {
                    self.state = ParserState::Ground;
                }
                _ if (0x51..=0x57).contains(&byte) => {
                    self.state = ParserState::Ground;
                }
                _ if (0x58..=0x5F).contains(&byte) => {
                    self.state = ParserState::Ground;
                }
                _ if (0x60..=0x7E).contains(&byte) => {
                    self.handle_esc_final(byte, &mut events);
                    self.state = ParserState::Ground;
                }
                _ => {
                    self.state = ParserState::Ground;
                }
            },
            ParserState::CsiEntry => {
                if (0x30..=0x3F).contains(&byte) {
                    self.state = ParserState::CsiParam;
                    self.params.push((byte - b'0') as i64);
                } else if (0x20..=0x2F).contains(&byte) {
                    self.state = ParserState::CsiIntermediate;
                    self.intermediate.push(byte);
                } else if (0x40..=0x7E).contains(&byte) {
                    self.handle_csi_final(byte, &mut events);
                    self.state = ParserState::Ground;
                } else {
                    self.state = ParserState::Ground;
                }
            }
            ParserState::CsiParam => {
                if (0x30..=0x39).contains(&byte) {
                    let last = self.params.last_mut().unwrap();
                    *last = *last * 10 + (byte - b'0') as i64;
                } else if byte == b';' {
                    self.params.push(0);
                } else if (0x3A..=0x3F).contains(&byte) {
                    self.intermediate.push(byte);
                    self.state = ParserState::CsiIntermediate;
                } else if (0x20..=0x2F).contains(&byte) {
                    self.intermediate.push(byte);
                    self.state = ParserState::CsiIntermediate;
                } else if (0x40..=0x7E).contains(&byte) {
                    self.handle_csi_final(byte, &mut events);
                    self.state = ParserState::Ground;
                } else {
                    self.state = ParserState::Ground;
                }
            }
            ParserState::CsiIntermediate => {
                if (0x20..=0x2F).contains(&byte) {
                    self.intermediate.push(byte);
                } else if (0x40..=0x7E).contains(&byte) {
                    self.handle_csi_final(byte, &mut events);
                    self.state = ParserState::Ground;
                } else {
                    self.state = ParserState::Ground;
                }
            }
            ParserState::CsiIgnore => {
                if (0x40..=0x7E).contains(&byte) {
                    self.state = ParserState::Ground;
                }
            }
            ParserState::OscString => {
                if byte == 0x07 || byte == 0x1B {
                    let osc_data = String::from_utf8_lossy(&self.osc_buffer).to_string();
                    self.handle_osc(&osc_data, &mut events);
                    self.state = ParserState::Ground;
                    self.osc_buffer.clear();
                } else {
                    self.osc_buffer.push(byte);
                }
            }
            ParserState::DcsEntry => {
                if (0x30..=0x3F).contains(&byte) {
                    self.state = ParserState::DcsParam;
                    self.params.push((byte - b'0') as i64);
                } else if (0x20..=0x2F).contains(&byte) {
                    self.state = ParserState::DcsIntermediate;
                    self.intermediate.push(byte);
                } else if (0x40..=0x7E).contains(&byte) {
                    self.state = ParserState::DcsPassthrough;
                } else {
                    self.state = ParserState::Ground;
                }
            }
            ParserState::DcsParam | ParserState::DcsIntermediate => {
                if (0x20..=0x7E).contains(&byte) {
                    if (0x40..=0x7E).contains(&byte) {
                        self.state = ParserState::DcsPassthrough;
                    }
                } else {
                    self.state = ParserState::Ground;
                }
            }
            ParserState::DcsPassthrough => {
                if byte == 0x1B {
                    self.state = ParserState::Ground;
                }
            }
            ParserState::DcsIgnore => {
                if byte == 0x1B {
                    self.state = ParserState::Ground;
                }
            }
            ParserState::SosPmApcString => {
                if byte == 0x07 || byte == 0x1B {
                    self.state = ParserState::Ground;
                }
            }
        }
        events
    }

    fn handle_esc_final(&self, byte: u8, events: &mut Vec<AnsiEvent>) {
        match byte {
            b'c' => events.push(AnsiEvent::Reset),
            b'7' => events.push(AnsiEvent::SaveCursor),
            b'8' => events.push(AnsiEvent::RestoreCursor),
            b'H' => events.push(AnsiEvent::TabSet),
            b'D' => events.push(AnsiEvent::Index),
            b'E' => events.push(AnsiEvent::NextLine),
            b'M' => events.push(AnsiEvent::ReverseIndex),
            _ => {}
        }
    }

    fn handle_csi_final(&mut self, byte: u8, events: &mut Vec<AnsiEvent>) {
        let param =
            |i: usize, default: i64| -> i64 { self.params.get(i).copied().unwrap_or(default) };

        match byte {
            b'A' => events.push(AnsiEvent::CursorUp(param(0, 1) as usize)),
            b'B' => events.push(AnsiEvent::CursorDown(param(0, 1) as usize)),
            b'C' => events.push(AnsiEvent::CursorForward(param(0, 1) as usize)),
            b'D' => events.push(AnsiEvent::CursorBackward(param(0, 1) as usize)),
            b'E' => events.push(AnsiEvent::CursorNextLine(param(0, 1) as usize)),
            b'F' => events.push(AnsiEvent::CursorPrevLine(param(0, 1) as usize)),
            b'G' => events.push(AnsiEvent::CursorHorizontalAbsolute(param(0, 1) as usize)),
            b'H' => events.push(AnsiEvent::CursorPosition(
                param(0, 1) as usize,
                param(1, 1) as usize,
            )),
            b'J' => events.push(AnsiEvent::EraseInDisplay(
                EraseMode::from(param(0, 0) as u8),
            )),
            b'K' => events.push(AnsiEvent::EraseInLine(EraseMode::from(param(0, 0) as u8))),
            b'S' => events.push(AnsiEvent::ScrollUp(param(0, 1) as usize)),
            b'T' => events.push(AnsiEvent::ScrollDown(param(0, 1) as usize)),
            b'r' => {
                let top = param(0, 0) as usize;
                let bottom = param(1, 0) as usize;
                if self.params.len() >= 2 {
                    events.push(AnsiEvent::SetScrollRegion(top, bottom));
                } else {
                    events.push(AnsiEvent::SetScrollRegion(0, 0));
                }
            }
            b'c' => events.push(AnsiEvent::DeviceAttributes),
            b'm' => {
                for p in &self.params {
                    events.push(AnsiEvent::Sgr(*p));
                }
            }
            b'h' => events.push(AnsiEvent::SetMode(true, self.params.clone())),
            b'l' => events.push(AnsiEvent::SetMode(false, self.params.clone())),
            _ => {}
        }
        self.params.clear();
        self.intermediate.clear();
    }

    fn handle_osc(&mut self, data: &str, events: &mut Vec<AnsiEvent>) {
        if let Some((cmd, data)) = data.split_once(';') {
            if let Ok(cmd) = cmd.parse::<u8>() {
                events.push(AnsiEvent::Osc {
                    command: cmd,
                    data: data.to_string(),
                });
            }
        }
    }

    #[allow(dead_code)]
    fn param(&self, index: usize, default: i64) -> i64 {
        self.params.get(index).copied().unwrap_or(default)
    }
}

/// ANSI parser events.
#[derive(PartialEq, Eq, Debug)]
pub enum AnsiEvent {
    /// Printable character.
    Print(char),
    /// Control character.
    Control(u8),
    /// Reset terminal.
    Reset,
    /// Cursor up.
    CursorUp(usize),
    /// Cursor down.
    CursorDown(usize),
    /// Cursor forward.
    CursorForward(usize),
    /// Cursor backward.
    CursorBackward(usize),
    /// Cursor next line.
    CursorNextLine(usize),
    /// Cursor previous line.
    CursorPrevLine(usize),
    /// Cursor horizontal absolute.
    CursorHorizontalAbsolute(usize),
    /// Cursor position.
    CursorPosition(usize, usize),
    /// Erase in display.
    EraseInDisplay(EraseMode),
    /// Erase in line.
    EraseInLine(EraseMode),
    /// Scroll up.
    ScrollUp(usize),
    /// Scroll down.
    ScrollDown(usize),
    /// Set scroll region.
    SetScrollRegion(usize, usize),
    /// Device attributes.
    DeviceAttributes,
    /// Save cursor.
    SaveCursor,
    /// Restore cursor.
    RestoreCursor,
    /// Tab set.
    TabSet,
    /// SGR (Select Graphic Rendition).
    Sgr(i64),
    /// Set mode.
    SetMode(bool, Vec<i64>),
    /// OSC sequence.
    Osc { command: u8, data: String },
    /// DCS sequence.
    Dcs {
        params: Vec<i64>,
        intermediate: Vec<u8>,
        data: Vec<u8>,
    },
    /// Index.
    Index,
    /// Next line.
    NextLine,
    /// Reverse index.
    ReverseIndex,
    /// Device attributes response.
    DeviceAttributesResponse(String),
    /// Window title.
    WindowTitle(String),
    /// Window icon title.
    WindowIconTitle(String),
}

/// Erase mode for CSI J/K.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EraseMode {
    ToEnd = 0,
    ToBeginning = 1,
    All = 2,
    Scrollback = 3,
}

impl EraseMode {
    pub fn from(value: u8) -> Self {
        match value {
            0 => EraseMode::ToEnd,
            1 => EraseMode::ToBeginning,
            2 => EraseMode::All,
            3 => EraseMode::Scrollback,
            _ => EraseMode::ToEnd,
        }
    }
}

impl Default for EraseMode {
    fn default() -> Self {
        Self::ToEnd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_creation() {
        let parser = AnsiParser::new();
        assert_eq!(parser.state, ParserState::Ground);
    }

    #[test]
    fn test_parse_print() {
        let mut parser = AnsiParser::new();
        let events = parser.parse(b'A');
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], AnsiEvent::Print('A'));
    }

    #[test]
    fn test_parse_esc() {
        let mut parser = AnsiParser::new();
        parser.parse(0x1B);
        let events = parser.parse(b'c');
        assert!(events.iter().any(|e| *e == AnsiEvent::Reset));
    }

    #[test]
    fn test_parse_csi() {
        let mut parser = AnsiParser::new();
        parser.parse(0x1B);
        parser.parse(b'[');
        let _events = parser.parse(b'2');
        parser.parse(b'J');
    }
}
