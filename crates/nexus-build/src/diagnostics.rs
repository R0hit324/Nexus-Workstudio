//! Diagnostic parsing for compiler output.

use nexus_core::{Diagnostic, DiagnosticSeverity};

/// Trait for language-specific diagnostic parsers.
pub trait DiagnosticsParserTrait: Send + Sync {
    fn language(&self) -> &str;
    fn parse(&self, stdout: &str, stderr: &str) -> Vec<Diagnostic>;
}

/// Diagnostic parser for compiler output.
pub struct DiagnosticParser {
    parsers: Vec<Box<dyn DiagnosticsParserTrait>>,
}

impl DiagnosticParser {
    pub fn new() -> Self {
        Self {
            parsers: Vec::new(),
        }
    }

    pub fn register(&mut self, parser: Box<dyn DiagnosticsParserTrait>) {
        self.parsers.push(parser);
    }

    /// Parses compiler output, dispatching to a registered language parser
    /// when available and falling back to a generic parser otherwise.
    pub fn parse(&self, stdout: &str, stderr: &str, language: &str) -> Vec<Diagnostic> {
        for parser in &self.parsers {
            if parser.language() == language {
                return parser.parse(stdout, stderr);
            }
        }
        Self::parse_generic(stdout, stderr)
    }

    fn parse_generic(stdout: &str, stderr: &str) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Parse common patterns from stderr and stdout for some compilers
        for line in stderr.lines().chain(stdout.lines()) {
            if let Some(diag) = try_parse_generic_line(line) {
                diagnostics.push(diag);
            }
        }

        diagnostics
    }
}

impl Default for DiagnosticParser {
    fn default() -> Self {
        Self::new()
    }
}

fn severity_from_word(word: &str) -> DiagnosticSeverity {
    match word {
        "error" => DiagnosticSeverity::Error,
        "warning" => DiagnosticSeverity::Warning,
        "note" => DiagnosticSeverity::Hint,
        _ => DiagnosticSeverity::Info,
    }
}

fn diag(
    file: String,
    line: Option<u32>,
    col: Option<u32>,
    severity: DiagnosticSeverity,
    message: String,
    code: Option<String>,
    source: &str,
) -> Diagnostic {
    Diagnostic {
        file_id: None,
        path: Some(file),
        line,
        column: col,
        end_line: None,
        end_column: None,
        severity,
        message,
        code,
        source: source.to_string(),
    }
}

/// Generic compiler pattern: `file:line:col: severity: message`.
pub(crate) fn try_parse_generic_line(line: &str) -> Option<Diagnostic> {
    let rest = line.trim();

    // Pattern: file(line,col): severity: message (MSVC style)
    if let Some((file, ln, col, rest)) = try_split_msvc(rest) {
        let (severity, message) = split_severity(rest)?;
        return Some(diag(
            file.to_string(),
            Some(ln),
            Some(col),
            severity,
            message,
            None,
            "compiler",
        ));
    }

    // Pattern: file:line:col: severity: message
    if let Some(caps) = parse_file_line_col(rest) {
        let (file, ln, col, rest) = caps;
        if let Some((severity, message)) = split_severity(rest) {
            return Some(diag(
                file.to_string(),
                Some(ln),
                Some(col),
                severity,
                message,
                None,
                "compiler",
            ));
        }
        return None;
    }

    // Pattern: file:line: severity: message
    let (file, ln, rest) = parse_file_line(rest)?;
    let (severity, message) = split_severity(rest)?;
    Some(diag(
        file.to_string(),
        Some(ln),
        None,
        severity,
        message,
        None,
        "compiler",
    ))
}

/// Rust pattern: `file:line:col: severity[code]: message`.
pub(crate) fn try_parse_rust_line(line: &str) -> Option<Diagnostic> {
    let rest = line.trim();
    let (file, ln, col, tail) = parse_file_line_col(rest)?;
    let (severity, code, message) = split_rust_severity(tail)?;
    Some(diag(
        file.to_string(),
        Some(ln),
        Some(col),
        severity,
        message,
        code,
        "rustc",
    ))
}

fn parse_file_line_col(s: &str) -> Option<(&str, u32, u32, &str)> {
    // Find first colon which is not part of a drive letter (e.g. C:\)
    let mut split = 0;
    for (i, c) in s.char_indices() {
        if c == ':' {
            split = i;
            break;
        }
    }
    if split == 0 {
        return None;
    }
    let file = &s[..split];
    let rest = &s[split + 1..];
    // rest = "line:col: remainder"
    let mut parts = rest.splitn(3, ':');
    let ln = parts.next()?.trim().parse::<u32>().ok()?;
    let col = parts.next()?.trim().parse::<u32>().ok()?;
    let tail = parts.next()?;
    Some((file, ln, col, tail))
}

fn parse_file_line(s: &str) -> Option<(&str, u32, &str)> {
    let mut split = 0;
    for (i, c) in s.char_indices() {
        if c == ':' {
            split = i;
            break;
        }
    }
    if split == 0 {
        return None;
    }
    let file = &s[..split];
    let rest = &s[split + 1..];
    // Reject Windows drive-letter style colons in file names
    if file.len() == 1 {
        return None;
    }
    let mut parts = rest.splitn(2, ':');
    let ln = parts.next()?.trim().parse::<u32>().ok()?;
    let tail = parts.next()?;
    // The remaining segment must look like "severity: message"
    let _ = split_severity(tail)?;
    Some((file, ln, tail))
}

fn try_split_msvc(s: &str) -> Option<(&str, u32, u32, &str)> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let file = &s[..open];
    let inner = &s[open + 1..close];
    let mut parts = inner.split(',');
    let ln = parts.next()?.trim().parse::<u32>().ok()?;
    let col = parts.next()?.trim().parse::<u32>().ok()?;
    let tail = s[close + 1..].trim_start();
    Some((file, ln, col, tail))
}

fn split_severity(rest: &str) -> Option<(DiagnosticSeverity, String)> {
    let mut parts = rest.splitn(2, ':');
    let sev = parts.next()?.trim();
    let message = parts.next()?;
    let severity = severity_from_word(sev);
    Some((severity, message.trim().to_string()))
}

fn split_rust_severity(rest: &str) -> Option<(DiagnosticSeverity, Option<String>, String)> {
    let mut parts = rest.splitn(2, ':');
    let sev = parts.next()?.trim();
    let message = parts.next()?;
    // severity may carry an error code: error[E0432]
    if let Some(open) = sev.find('[') {
        let close = sev.find(']')?;
        let word = &sev[..open];
        let code = &sev[open + 1..close];
        Some((
            severity_from_word(word),
            Some(code.to_string()),
            message.trim().to_string(),
        ))
    } else {
        Some((severity_from_word(sev), None, message.trim().to_string()))
    }
}

/// Rust-specific diagnostic parser.
pub struct RustDiagnosticParser;

impl DiagnosticsParserTrait for RustDiagnosticParser {
    fn language(&self) -> &str {
        "rust"
    }

    fn parse(&self, _stdout: &str, stderr: &str) -> Vec<Diagnostic> {
        stderr.lines().filter_map(try_parse_rust_line).collect()
    }
}

/// GCC/Clang diagnostic parser.
pub struct GccDiagnosticParser;

impl DiagnosticsParserTrait for GccDiagnosticParser {
    fn language(&self) -> &str {
        "c"
    }

    fn parse(&self, _stdout: &str, stderr: &str) -> Vec<Diagnostic> {
        stderr.lines().filter_map(try_parse_generic_line).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_generic_line() {
        let d = try_parse_generic_line("src/main.rs:10:5: error: expected `;`").unwrap();
        assert_eq!(d.line, Some(10));
        assert_eq!(d.column, Some(5));
        assert_eq!(d.severity, DiagnosticSeverity::Error);
        assert_eq!(d.message, "expected `;`");
    }

    #[test]
    fn parses_rust_code() {
        let d = try_parse_rust_line("src/main.rs:3:5: error[E0432]: unresolved import").unwrap();
        assert_eq!(d.code, Some("E0432".to_string()));
        assert_eq!(d.severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn generic_parser_falls_back() {
        let parser = DiagnosticParser::new();
        let diags = parser.parse("", "a.rs:1:1: warning: thing", "unknown-lang");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Warning);
    }
}
