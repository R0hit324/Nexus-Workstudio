//! Lightweight, allocation-conscious syntax highlighting.
//!
//! Spans use *character* offsets (consistent with `GapBuffer`) and are produced
//! by a small hand-rolled tokenizer instead of a regex engine, so highlight
//! cost stays linear and files over `LARGE_FILE_THRESHOLD` can cheaply decline
//! highlighting (see `SyntaxProvider::is_large_file_safe`).

/// Languages recognized from file extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageId {
    PlainText,
    Rust,
    Cpp,
    Python,
    TypeScript,
    Json,
    Markdown,
    Shell,
    Go,
    Toml,
}

impl LanguageId {
    /// Maps a file extension to a language.
    #[must_use]
    pub fn from_path(path: &std::path::Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("rs") => Self::Rust,
            Some("c") | Some("h") | Some("cpp") | Some("hpp") | Some("cc") => Self::Cpp,
            Some("py") => Self::Python,
            Some("ts") | Some("tsx") | Some("js") | Some("jsx") | Some("mjs") => Self::TypeScript,
            Some("json") => Self::Json,
            Some("md") | Some("markdown") => Self::Markdown,
            Some("sh") | Some("bash") | Some("zsh") => Self::Shell,
            Some("go") => Self::Go,
            Some("toml") => Self::Toml,
            _ => Self::PlainText,
        }
    }
}

impl std::fmt::Display for LanguageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::PlainText => "plaintext",
            Self::Rust => "Rust",
            Self::Cpp => "C++",
            Self::Python => "Python",
            Self::TypeScript => "TypeScript",
            Self::Json => "JSON",
            Self::Markdown => "Markdown",
            Self::Shell => "Shell",
            Self::Go => "Go",
            Self::Toml => "TOML",
        };
        f.write_str(name)
    }
}

/// The highlight classes the theme can style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightKind {
    Keyword,
    String,
    Number,
    Comment,
    Type,
    Function,
    Operator,
    Punctuation,
}

/// A styled run within a line, in character offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: HighlightKind,
}

/// Produces highlight spans for a language.
pub trait SyntaxProvider {
    fn language(&self) -> LanguageId;
    /// Returns spans for one line of text (without newline).
    fn highlight_line(&self, line: &str) -> Vec<HighlightSpan>;
    /// Large files may opt out of highlighting to stay responsive.
    fn is_large_file_safe(&self) -> bool {
        false
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

const CPP_KEYWORDS: &[&str] = &[
    "auto",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "double",
    "else",
    "enum",
    "explicit",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "namespace",
    "new",
    "nullptr",
    "operator",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

const PYTHON_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield",
];

const TS_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

const SHELL_KEYWORDS: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if", "in",
    "local", "return", "set", "then", "true", "false", "while", "echo", "exit",
];

const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

/// Generic tokenizer configured per language.
#[derive(Debug, Clone, Copy)]
pub struct SimpleSyntax {
    language: LanguageId,
    keywords: &'static [&'static str],
    line_comment: Option<&'static str>,
    block_comment: Option<(&'static str, &'static str)>,
}

/// Returns a provider for `language` (or `None` for plain text).
#[must_use]
pub fn provider_for(language: LanguageId) -> Option<SimpleSyntax> {
    let (keywords, line_comment, block_comment): (&[&str], Option<&str>, Option<(&str, &str)>) =
        match language {
            LanguageId::PlainText => return None,
            LanguageId::Rust => (RUST_KEYWORDS, Some("//"), Some(("/*", "*/"))),
            LanguageId::Cpp => (CPP_KEYWORDS, Some("//"), Some(("/*", "*/"))),
            LanguageId::Python => (PYTHON_KEYWORDS, Some("#"), None),
            LanguageId::TypeScript => (TS_KEYWORDS, Some("//"), Some(("/*", "*/"))),
            LanguageId::Json => (&[], Some("//"), None),
            LanguageId::Markdown => (&[], None, None),
            LanguageId::Shell => (SHELL_KEYWORDS, Some("#"), None),
            LanguageId::Go => (GO_KEYWORDS, Some("//"), Some(("/*", "*/"))),
            LanguageId::Toml => (&[], Some("#"), None),
        };
    Some(SimpleSyntax {
        language,
        keywords,
        line_comment,
        block_comment,
    })
}

impl SyntaxProvider for SimpleSyntax {
    fn language(&self) -> LanguageId {
        self.language
    }

    fn highlight_line(&self, line: &str) -> Vec<HighlightSpan> {
        let mut spans = Vec::new();
        let chars: Vec<char> = line.chars().collect();
        let n = chars.len();
        let mut i = 0usize;

        while i < n {
            let c = chars[i];

            // Block comments.
            if let Some((open, close)) = self.block_comment {
                if starts_with(&chars, i, open) {
                    let close_len = close.chars().count();
                    let end = find_sub(&chars, i + open.chars().count(), close).unwrap_or(n);
                    spans.push(span(
                        i,
                        end.saturating_add(close_len).min(n),
                        HighlightKind::Comment,
                    ));
                    i = end + close_len;
                    continue;
                }
            }

            // Line comments.
            if let Some(marker) = self.line_comment {
                if starts_with(&chars, i, marker) {
                    spans.push(span(i, n, HighlightKind::Comment));
                    break;
                }
            }

            // Strings.
            if c == '"' || c == '\'' {
                let quote = c;
                let mut j = i + 1;
                while j < n {
                    if chars[j] == '\\' {
                        j = (j + 2).min(n);
                    } else if chars[j] == quote {
                        j += 1;
                        break;
                    } else {
                        j += 1;
                    }
                }
                spans.push(span(i, j.min(n), HighlightKind::String));
                i = j;
                continue;
            }

            // Numbers.
            if c.is_ascii_digit() {
                let mut j = i + 1;
                while j < n
                    && (chars[j].is_ascii_digit()
                        || matches!(chars[j], '.' | 'x' | 'b' | 'o' | '_'))
                {
                    j += 1;
                }
                spans.push(span(i, j, HighlightKind::Number));
                i = j;
                continue;
            }

            // Identifiers / keywords / types.
            if c.is_alphanumeric() || c == '_' {
                let mut j = i + 1;
                while j < n && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                if self.keywords.contains(&word.as_str()) {
                    spans.push(span(i, j, HighlightKind::Keyword));
                } else if word
                    .chars()
                    .next()
                    .map(|w| w.is_uppercase())
                    .unwrap_or(false)
                {
                    spans.push(span(i, j, HighlightKind::Type));
                } else {
                    // Function call heuristic: identifier followed by '('.
                    if chars.get(j) == Some(&'(') {
                        spans.push(span(i, j, HighlightKind::Function));
                    }
                }
                i = j;
                continue;
            }

            // Operators vs punctuation.
            let kind = if matches!(
                c,
                '+' | '-' | '*' | '/' | '%' | '=' | '!' | '<' | '>' | '&' | '|' | '^' | '~' | '?'
            ) {
                HighlightKind::Operator
            } else {
                HighlightKind::Punctuation
            };
            spans.push(span(i, i + 1, kind));
            i += 1;
        }

        spans
    }
}

fn span(start: usize, end: usize, kind: HighlightKind) -> HighlightSpan {
    HighlightSpan { start, end, kind }
}

fn starts_with(chars: &[char], at: usize, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    p.len() <= chars.len() - at && chars[at..at + p.len()] == *p
}

fn find_sub(chars: &[char], mut at: usize, pat: &str) -> Option<usize> {
    let p: Vec<char> = pat.chars().collect();
    if p.is_empty() {
        return Some(at);
    }
    while at + p.len() <= chars.len() {
        if chars[at..at + p.len()] == *p {
            return Some(at);
        }
        at += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn extension_mapping() {
        assert_eq!(LanguageId::from_path(Path::new("a.rs")), LanguageId::Rust);
        assert_eq!(LanguageId::from_path(Path::new("x.CPP")), LanguageId::Cpp);
        assert_eq!(
            LanguageId::from_path(Path::new("config.json")),
            LanguageId::Json
        );
        assert_eq!(
            LanguageId::from_path(Path::new("README.md")),
            LanguageId::Markdown
        );
        assert_eq!(
            LanguageId::from_path(Path::new("noext")),
            LanguageId::PlainText
        );
    }

    #[test]
    fn rust_line_highlights_keywords_strings_numbers_comments() {
        let p = provider_for(LanguageId::Rust).unwrap();
        let spans = p.highlight_line("let x: u32 = foo(\"hi /* fake */\") * 42; // real");
        let kinds: Vec<HighlightKind> = spans.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&HighlightKind::Keyword));
        assert!(kinds.contains(&HighlightKind::String));
        assert!(kinds.contains(&HighlightKind::Number));
        assert!(kinds.contains(&HighlightKind::Comment));
        assert!(
            kinds
                .iter()
                .filter(|k| **k == HighlightKind::String)
                .count()
                == 1
        );
        // String swallowed the fake block comment.
        for s in &spans {
            if s.kind == HighlightKind::String {
                assert!(s.start < 22 && 22 < s.end);
            }
        }
    }

    #[test]
    fn block_comments_span_properly() {
        let p = provider_for(LanguageId::Rust).unwrap();
        let spans = p.highlight_line("/* a */ let b = 1;");
        let first = spans.first().unwrap();
        assert_eq!(first.kind, HighlightKind::Comment);
        assert_eq!(first.start, 0);
        assert_eq!(first.end, 7);
    }

    #[test]
    fn function_heuristic_and_upper_word_types() {
        let p = provider_for(LanguageId::Rust).unwrap();
        let spans = p.highlight_line("Foo::new()");
        let has_fn = spans.iter().any(|s| s.kind == HighlightKind::Function);
        let has_type = spans.iter().any(|s| s.kind == HighlightKind::Type);
        assert!(has_type, "Foo marked as Type");
        assert!(has_fn, "new() marked as Function");
    }

    #[test]
    fn plain_text_has_no_provider() {
        assert!(provider_for(LanguageId::PlainText).is_none());
    }

    #[test]
    fn spans_never_exceed_line_length() {
        let p = provider_for(LanguageId::Python).unwrap();
        for line in ["def f(a):", "x = 'str' # note", "42 >= 7 and False"] {
            for s in p.highlight_line(line) {
                assert!(
                    s.end <= line.chars().count(),
                    "span {} out of bounds",
                    s.end
                );
                assert!(s.start < s.end);
            }
        }
    }
}
