use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub file_id: usize,
    pub line: usize,
    pub col: usize,
    pub len: usize,
}

impl Span {
    pub fn new(file_id: usize, line: usize, col: usize, len: usize) -> Self {
        Self { file_id, line, col, len: len.max(1) }
    }

    pub fn merge(self, other: Span) -> Span {
        if self.file_id == other.file_id && self.line == other.line {
            let start = self.col.min(other.col);
            let end = (self.col + self.len).max(other.col + other.len);
            Span {
                file_id: self.file_id,
                line: self.line,
                col: start,
                len: (end - start).max(1),
            }
        } else {
            self
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub source: String,
    pub lines: Vec<String>,
}

impl SourceFile {
    pub fn new(path: PathBuf, source: String) -> Self {
        let lines = source.lines().map(|s| s.to_string()).collect();
        Self { path, source, lines }
    }

    pub fn line_text(&self, line_1_based: usize) -> &str {
        if line_1_based == 0 || line_1_based > self.lines.len() {
            ""
        } else {
            &self.lines[line_1_based - 1]
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span,
            hint: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn render(&self, files: &[SourceFile]) -> String {
        let mut out = String::new();
        let file = files.get(self.span.file_id);
        let path_str = file
            .map(|f| f.path.display().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());

        out.push_str(&format!(
            "error: {}\n  --> {}:{}:{}\n",
            self.message, path_str, self.span.line, self.span.col
        ));

        if let Some(f) = file {
            let line_str = f.line_text(self.span.line);
            if !line_str.is_empty() || self.span.line <= f.lines.len() {
                let line_num = format!("{}", self.span.line);
                let pad = " ".repeat(line_num.len());
                let caret_offset = self.span.col.saturating_sub(1);
                let spaces = " ".repeat(caret_offset);
                let carets = "^".repeat(self.span.len.max(1));
                out.push_str(&format!(
                    "   {pad} |\n   {line_num} | {line_str}\n   {pad} | {spaces}{carets}\n"
                ));
            }
        }
        if let Some(hint) = &self.hint {
            out.push_str(&format!("   = hint: {hint}\n"));
        }
        out
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

#[allow(dead_code)]
impl Diagnostics {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, diag: Diagnostic) {
        self.items.push(diag);
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn render_all(&self, files: &[SourceFile]) -> String {
        let mut s = String::new();
        for d in &self.items {
            s.push_str(&d.render(files));
        }
        s
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for d in &self.items {
            writeln!(f, "{}:{}: {}", d.span.line, d.span.col, d.message)?;
        }
        Ok(())
    }
}
