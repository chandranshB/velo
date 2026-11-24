use crate::error::{Diagnostic, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum FStrTokenPart {
    Literal(String),
    Expr { src: String, prec: Option<i32>, col_offset: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Indentation & structure
    Newline,
    Indent,
    Dedent,
    Eof,

    // Literals
    Int(i128),
    Float(f64),
    Str(String),
    CStr(String),
    WStr(String),
    FStr(Vec<FStrTokenPart>),
    Ident(String),

    // Keywords
    Def,
    Struct,
    Enum,
    Extern,
    Let,
    Var,
    Const,
    If,
    Elif,
    Else,
    While,
    For,
    In,
    Match,
    Case,
    Return,
    Break,
    Continue,
    Defer,
    Pass,
    Import,
    From,
    As,
    And,
    Or,
    Not,
    True,
    False,
    Null,
    Link,
    Subsystem,
    Inline,
    SizeOf,
    Zeroed,
    Fn,
    PtrKw,

    // Punctuation
    Colon,
    Comma,
    Dot,
    DotDot,
    DotDotEq,
    DotDotDot,
    Semi,
    Arrow,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    Eq,
    EqEq,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn tokenize(file_id: usize, src: &str) -> Result<Vec<Token>, Diagnostic> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut indent_stack: Vec<usize> = vec![0];
    let mut pos = 0;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut at_line_start = true;
    let mut paren_depth = 0usize;

    while pos < chars.len() {
        if at_line_start && paren_depth == 0 {
            let mut indent = 0usize;
            let start_col = 1usize;
            while pos < chars.len() {
                match chars[pos] {
                    ' ' => {
                        indent += 1;
                        pos += 1;
                        col += 1;
                    }
                    '\t' => {
                        indent += 4;
                        pos += 1;
                        col += 1;
                    }
                    '\r' => {
                        pos += 1;
                    }
                    '\n' => {
                        pos += 1;
                        line += 1;
                        col = 1;
                        indent = 0;
                    }
                    '#' => {
                        while pos < chars.len() && chars[pos] != '\n' {
                            pos += 1;
                        }
                        if pos < chars.len() && chars[pos] == '\n' {
                            pos += 1;
                            line += 1;
                            col = 1;
                            indent = 0;
                        }
                    }
                    _ => break,
                }
            }
            if pos >= chars.len() {
                break;
            }
            at_line_start = false;
            let cur = *indent_stack.last().unwrap();
            if indent > cur {
                indent_stack.push(indent);
                tokens.push(Token {
                    kind: TokenKind::Indent,
                    span: Span::new(file_id, line, start_col, indent.max(1)),
                });
            } else if indent < cur {
                while let Some(&top) = indent_stack.last() {
                    if indent < top {
                        indent_stack.pop();
                        tokens.push(Token {
                            kind: TokenKind::Dedent,
                            span: Span::new(file_id, line, start_col, 1),
                        });
                    } else {
                        break;
                    }
                }
                if *indent_stack.last().unwrap() != indent {
                    return Err(Diagnostic::error(
                        Span::new(file_id, line, start_col, 1),
                        format!("inconsistent indentation level ({indent} spaces)"),
                    ));
                }
            }
            continue;
        }

        let ch = chars[pos];
        match ch {
            ' ' | '\t' | '\r' => {
                pos += 1;
                col += 1;
            }
            '\n' => {
                let sp = Span::new(file_id, line, col, 1);
                pos += 1;
                line += 1;
                col = 1;
                if paren_depth == 0 {
                    if !matches!(
                        tokens.last().map(|t| &t.kind),
                        None | Some(TokenKind::Newline) | Some(TokenKind::Indent) | Some(TokenKind::Dedent)
                    ) {
                        tokens.push(Token {
                            kind: TokenKind::Newline,
                            span: sp,
                        });
                    }
                    at_line_start = true;
                }
            }
            '#' => {
                while pos < chars.len() && chars[pos] != '\n' {
                    pos += 1;
                    col += 1;
                }
            }
            '(' => {
                paren_depth += 1;
                tokens.push(Token { kind: TokenKind::LParen, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                tokens.push(Token { kind: TokenKind::RParen, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            '[' => {
                paren_depth += 1;
                tokens.push(Token { kind: TokenKind::LBracket, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            ']' => {
                paren_depth = paren_depth.saturating_sub(1);
                tokens.push(Token { kind: TokenKind::RBracket, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            '{' => {
                paren_depth += 1;
                tokens.push(Token { kind: TokenKind::LBrace, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            '}' => {
                paren_depth = paren_depth.saturating_sub(1);
                tokens.push(Token { kind: TokenKind::RBrace, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            ':' => {
                tokens.push(Token { kind: TokenKind::Colon, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            ',' => {
                tokens.push(Token { kind: TokenKind::Comma, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            ';' => {
                tokens.push(Token { kind: TokenKind::Semi, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            '~' => {
                tokens.push(Token { kind: TokenKind::Tilde, span: Span::new(file_id, line, col, 1) });
                pos += 1;
                col += 1;
            }
            '.' => {
                let start_col = col;
                if pos + 2 < chars.len() && chars[pos + 1] == '.' && chars[pos + 2] == '.' {
                    tokens.push(Token { kind: TokenKind::DotDotDot, span: Span::new(file_id, line, start_col, 3) });
                    pos += 3;
                    col += 3;
                } else if pos + 2 < chars.len() && chars[pos + 1] == '.' && chars[pos + 2] == '=' {
                    tokens.push(Token { kind: TokenKind::DotDotEq, span: Span::new(file_id, line, start_col, 3) });
                    pos += 3;
                    col += 3;
                } else if pos + 1 < chars.len() && chars[pos + 1] == '.' {
                    tokens.push(Token { kind: TokenKind::DotDot, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Dot, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '+' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::PlusEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Plus, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '-' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '>' {
                    tokens.push(Token { kind: TokenKind::Arrow, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::MinusEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Minus, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '*' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::StarEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Star, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '/' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::SlashEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Slash, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '%' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::PercentEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Percent, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '&' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::AmpEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Amp, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '|' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::PipeEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Pipe, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '^' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::CaretEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Caret, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '=' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::EqEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Eq, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '!' => {
                let start_col = col;
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::BangEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    return Err(Diagnostic::error(
                        Span::new(file_id, line, start_col, 1),
                        "unexpected '!'; use 'not' for logical negation",
                    ));
                }
            }
            '<' => {
                let start_col = col;
                if pos + 2 < chars.len() && chars[pos + 1] == '<' && chars[pos + 2] == '=' {
                    tokens.push(Token { kind: TokenKind::ShlEq, span: Span::new(file_id, line, start_col, 3) });
                    pos += 3;
                    col += 3;
                } else if pos + 1 < chars.len() && chars[pos + 1] == '<' {
                    tokens.push(Token { kind: TokenKind::Shl, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::LtEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Lt, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '>' => {
                let start_col = col;
                if pos + 2 < chars.len() && chars[pos + 1] == '>' && chars[pos + 2] == '=' {
                    tokens.push(Token { kind: TokenKind::ShrEq, span: Span::new(file_id, line, start_col, 3) });
                    pos += 3;
                    col += 3;
                } else if pos + 1 < chars.len() && chars[pos + 1] == '>' {
                    tokens.push(Token { kind: TokenKind::Shr, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::GtEq, span: Span::new(file_id, line, start_col, 2) });
                    pos += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Gt, span: Span::new(file_id, line, start_col, 1) });
                    pos += 1;
                    col += 1;
                }
            }
            '\'' => {
                let start_col = col;
                pos += 1;
                col += 1;
                if pos >= chars.len() {
                    return Err(Diagnostic::error(Span::new(file_id, line, start_col, 1), "unterminated character literal"));
                }
                let val = if chars[pos] == '\\' {
                    let esc = lex_escape(&chars, &mut pos, &mut col, file_id, line)?;
                    esc as i128
                } else {
                    let c = chars[pos];
                    pos += 1;
                    col += 1;
                    c as u32 as i128
                };
                if pos >= chars.len() || chars[pos] != '\'' {
                    return Err(Diagnostic::error(Span::new(file_id, line, start_col, 2), "unterminated character literal"));
                }
                pos += 1;
                col += 1;
                tokens.push(Token {
                    kind: TokenKind::Int(val),
                    span: Span::new(file_id, line, start_col, col - start_col),
                });
            }
            '"' => {
                let start_col = col;
                let s = lex_plain_string(&chars, &mut pos, &mut line, &mut col, file_id, false)?;
                tokens.push(Token {
                    kind: TokenKind::Str(s),
                    span: Span::new(file_id, line, start_col, (col - start_col).max(2)),
                });
            }
            c if (c == 'f' || c == 'c' || c == 'w' || c == 'r') && pos + 1 < chars.len() && chars[pos + 1] == '"' => {
                let prefix = c;
                let start_col = col;
                pos += 1;
                col += 1;
                match prefix {
                    'r' => {
                        let s = lex_plain_string(&chars, &mut pos, &mut line, &mut col, file_id, true)?;
                        tokens.push(Token {
                            kind: TokenKind::Str(s),
                            span: Span::new(file_id, line, start_col, (col - start_col).max(3)),
                        });
                    }
                    'c' => {
                        let s = lex_plain_string(&chars, &mut pos, &mut line, &mut col, file_id, false)?;
                        tokens.push(Token {
                            kind: TokenKind::CStr(s),
                            span: Span::new(file_id, line, start_col, (col - start_col).max(3)),
                        });
                    }
                    'w' => {
                        let s = lex_plain_string(&chars, &mut pos, &mut line, &mut col, file_id, false)?;
                        tokens.push(Token {
                            kind: TokenKind::WStr(s),
                            span: Span::new(file_id, line, start_col, (col - start_col).max(3)),
                        });
                    }
                    'f' => {
                        let parts = lex_fstring(&chars, &mut pos, &mut line, &mut col, file_id)?;
                        tokens.push(Token {
                            kind: TokenKind::FStr(parts),
                            span: Span::new(file_id, line, start_col, (col - start_col).max(3)),
                        });
                    }
                    _ => unreachable!(),
                }
            }
            c if c.is_ascii_digit() => {
                let start_col = col;
                let tok = lex_number(&chars, &mut pos, &mut col, file_id, line)?;
                tokens.push(Token {
                    kind: tok,
                    span: Span::new(file_id, line, start_col, col - start_col),
                });
            }
            c if is_ident_start(c) => {
                let start_col = col;
                let start = pos;
                while pos < chars.len() && is_ident_continue(chars[pos]) {
                    pos += 1;
                    col += 1;
                }
                let word: String = chars[start..pos].iter().collect();
                let kind = keyword_or_ident(word);
                tokens.push(Token {
                    kind,
                    span: Span::new(file_id, line, start_col, col - start_col),
                });
            }
            other => {
                return Err(Diagnostic::error(
                    Span::new(file_id, line, col, 1),
                    format!("unexpected character '{other}'"),
                ));
            }
        }
    }

    if !matches!(
        tokens.last().map(|t| &t.kind),
        None | Some(TokenKind::Newline) | Some(TokenKind::Dedent)
    ) {
        tokens.push(Token {
            kind: TokenKind::Newline,
            span: Span::new(file_id, line, col, 1),
        });
    }
    while indent_stack.len() > 1 {
        indent_stack.pop();
        tokens.push(Token {
            kind: TokenKind::Dedent,
            span: Span::new(file_id, line, 1, 1),
        });
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(file_id, line, col, 1),
    });
    Ok(tokens)
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn keyword_or_ident(word: String) -> TokenKind {
    match word.as_str() {
        "def" => TokenKind::Def,
        "struct" => TokenKind::Struct,
        "enum" => TokenKind::Enum,
        "extern" => TokenKind::Extern,
        "let" => TokenKind::Let,
        "var" => TokenKind::Var,
        "const" => TokenKind::Const,
        "if" => TokenKind::If,
        "elif" => TokenKind::Elif,
        "else" => TokenKind::Else,
        "while" => TokenKind::While,
        "for" => TokenKind::For,
        "in" => TokenKind::In,
        "match" => TokenKind::Match,
        "case" => TokenKind::Case,
        "return" => TokenKind::Return,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "defer" => TokenKind::Defer,
        "pass" => TokenKind::Pass,
        "import" => TokenKind::Import,
        "from" => TokenKind::From,
        "as" => TokenKind::As,
        "and" => TokenKind::And,
        "or" => TokenKind::Or,
        "not" => TokenKind::Not,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        "null" => TokenKind::Null,
        "link" => TokenKind::Link,
        "subsystem" => TokenKind::Subsystem,
        "inline" => TokenKind::Inline,
        "sizeof" => TokenKind::SizeOf,
        "zeroed" => TokenKind::Zeroed,
        "fn" => TokenKind::Fn,
        "ptr" => TokenKind::PtrKw,
        _ => TokenKind::Ident(word),
    }
}

fn lex_escape(
    chars: &[char],
    pos: &mut usize,
    col: &mut usize,
    file_id: usize,
    line: usize,
) -> Result<char, Diagnostic> {
    let esc_col = *col;
    *pos += 1;
    *col += 1;
    if *pos >= chars.len() {
        return Err(Diagnostic::error(
            Span::new(file_id, line, esc_col, 1),
            "unexpected end of string after '\\'",
        ));
    }
    let ch = chars[*pos];
    *pos += 1;
    *col += 1;
    match ch {
        'n' => Ok('\n'),
        'r' => Ok('\r'),
        't' => Ok('\t'),
        '\\' => Ok('\\'),
        '"' => Ok('"'),
        '\'' => Ok('\''),
        '0' => Ok('\0'),
        'x' => {
            if *pos + 1 >= chars.len() {
                return Err(Diagnostic::error(
                    Span::new(file_id, line, esc_col, 2),
                    "incomplete \\x hex escape",
                ));
            }
            let h: String = chars[*pos..*pos + 2].iter().collect();
            let b = u8::from_str_radix(&h, 16).map_err(|_| {
                Diagnostic::error(Span::new(file_id, line, esc_col, 4), "invalid \\x hex escape")
            })?;
            *pos += 2;
            *col += 2;
            Ok(b as char)
        }
        other => Err(Diagnostic::error(
            Span::new(file_id, line, esc_col, 2),
            format!("unknown escape sequence '\\{other}'"),
        )),
    }
}

fn lex_plain_string(
    chars: &[char],
    pos: &mut usize,
    line: &mut usize,
    col: &mut usize,
    file_id: usize,
    raw: bool,
) -> Result<String, Diagnostic> {
    let start_line = *line;
    let start_col = *col;
    *pos += 1; // opening '"'
    *col += 1;
    let mut out = String::new();
    while *pos < chars.len() {
        let ch = chars[*pos];
        if ch == '"' {
            *pos += 1;
            *col += 1;
            return Ok(out);
        }
        if ch == '\n' {
            return Err(Diagnostic::error(
                Span::new(file_id, start_line, start_col, 1),
                "unterminated string literal",
            ));
        }
        if !raw && ch == '\\' {
            let esc = lex_escape(chars, pos, col, file_id, *line)?;
            out.push(esc);
        } else {
            out.push(ch);
            *pos += 1;
            *col += 1;
        }
    }
    Err(Diagnostic::error(
        Span::new(file_id, start_line, start_col, 1),
        "unterminated string literal",
    ))
}

fn lex_fstring(
    chars: &[char],
    pos: &mut usize,
    line: &mut usize,
    col: &mut usize,
    file_id: usize,
) -> Result<Vec<FStrTokenPart>, Diagnostic> {
    let start_line = *line;
    let start_col = *col;
    *pos += 1; // opening '"'
    *col += 1;
    let mut parts = Vec::new();
    let mut lit = String::new();

    while *pos < chars.len() {
        let ch = chars[*pos];
        if ch == '"' {
            *pos += 1;
            *col += 1;
            if !lit.is_empty() {
                parts.push(FStrTokenPart::Literal(lit));
            }
            return Ok(parts);
        }
        if ch == '\n' {
            return Err(Diagnostic::error(
                Span::new(file_id, start_line, start_col, 1),
                "unterminated f-string literal",
            ));
        }
        if ch == '\\' {
            let esc = lex_escape(chars, pos, col, file_id, *line)?;
            lit.push(esc);
            continue;
        }
        if ch == '{' {
            if *pos + 1 < chars.len() && chars[*pos + 1] == '{' {
                lit.push('{');
                *pos += 2;
                *col += 2;
                continue;
            }
            if !lit.is_empty() {
                parts.push(FStrTokenPart::Literal(std::mem::take(&mut lit)));
            }
            *pos += 1;
            *col += 1;
            let expr_col = *col;
            let mut depth = 1usize;
            let mut raw_inner = String::new();
            let mut in_quote = false;
            while *pos < chars.len() && depth > 0 {
                let c = chars[*pos];
                if c == '\n' {
                    return Err(Diagnostic::error(
                        Span::new(file_id, *line, expr_col, 1),
                        "unterminated '{...}' inside f-string",
                    ));
                }
                if c == '"' {
                    in_quote = !in_quote;
                } else if !in_quote {
                    if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            *pos += 1;
                            *col += 1;
                            break;
                        }
                    }
                }
                raw_inner.push(c);
                *pos += 1;
                *col += 1;
            }
            if depth != 0 {
                return Err(Diagnostic::error(
                    Span::new(file_id, *line, expr_col, 1),
                    "unclosed '{' in f-string",
                ));
            }
            // Check for optional format specifier like :.2 or :.2f
            let (expr_src, prec) = split_fstr_format(&raw_inner, file_id, *line, expr_col)?;
            parts.push(FStrTokenPart::Expr {
                src: expr_src,
                prec,
                col_offset: expr_col,
            });
            continue;
        }
        if ch == '}' {
            if *pos + 1 < chars.len() && chars[*pos + 1] == '}' {
                lit.push('}');
                *pos += 2;
                *col += 2;
                continue;
            }
            return Err(Diagnostic::error(
                Span::new(file_id, *line, *col, 1),
                "single '}' inside f-string; escape with '}}'",
            ));
        }
        lit.push(ch);
        *pos += 1;
        *col += 1;
    }

    Err(Diagnostic::error(
        Span::new(file_id, start_line, start_col, 1),
        "unterminated f-string literal",
    ))
}

fn split_fstr_format(
    inner: &str,
    file_id: usize,
    line: usize,
    col: usize,
) -> Result<(String, Option<i32>), Diagnostic> {
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut in_str = false;
    let mut colon_idx = None;
    for (i, ch) in inner.char_indices() {
        if ch == '"' {
            in_str = !in_str;
        } else if !in_str {
            match ch {
                '(' => paren += 1,
                ')' => paren = paren.saturating_sub(1),
                '[' => bracket += 1,
                ']' => bracket = bracket.saturating_sub(1),
                ':' if paren == 0 && bracket == 0 => {
                    colon_idx = Some(i);
                    break;
                }
                _ => {}
            }
        }
    }
    if let Some(idx) = colon_idx {
        let expr_part = inner[..idx].trim().to_string();
        let fmt_part = inner[idx + 1..].trim();
        let trimmed = fmt_part.trim_end_matches('f');
        if let Some(after_dot) = trimmed.strip_prefix('.') {
            if let Ok(p) = after_dot.parse::<i32>() {
                return Ok((expr_part, Some(p)));
            }
        }
        return Err(Diagnostic::error(
            Span::new(file_id, line, col + idx, fmt_part.len().max(1)),
            format!("unsupported f-string format specifier ':{fmt_part}'; expected e.g. ':.2'"),
        ));
    }
    Ok((inner.trim().to_string(), None))
}

fn lex_number(
    chars: &[char],
    pos: &mut usize,
    col: &mut usize,
    file_id: usize,
    line: usize,
) -> Result<TokenKind, Diagnostic> {
    let start_col = *col;
    if chars[*pos] == '0' && *pos + 1 < chars.len() {
        let next = chars[*pos + 1];
        if next == 'x' || next == 'X' || next == 'b' || next == 'B' || next == 'o' || next == 'O' {
            let radix = match next {
                'x' | 'X' => 16,
                'b' | 'B' => 2,
                _ => 8,
            };
            *pos += 2;
            *col += 2;
            let mut digits = String::new();
            while *pos < chars.len() && (chars[*pos].is_digit(radix) || chars[*pos] == '_') {
                if chars[*pos] != '_' {
                    digits.push(chars[*pos]);
                }
                *pos += 1;
                *col += 1;
            }
            if digits.is_empty() {
                return Err(Diagnostic::error(
                    Span::new(file_id, line, start_col, *col - start_col),
                    "expected digits after integer base prefix",
                ));
            }
            let val = i128::from_str_radix(&digits, radix).map_err(|_| {
                Diagnostic::error(
                    Span::new(file_id, line, start_col, *col - start_col),
                    "integer literal overflows 128-bit range",
                )
            })?;
            return Ok(TokenKind::Int(val));
        }
    }

    let mut raw = String::new();
    let mut is_float = false;
    while *pos < chars.len() && (chars[*pos].is_ascii_digit() || chars[*pos] == '_') {
        if chars[*pos] != '_' {
            raw.push(chars[*pos]);
        }
        *pos += 1;
        *col += 1;
    }

    if *pos + 1 < chars.len() && chars[*pos] == '.' && chars[*pos + 1].is_ascii_digit() {
        is_float = true;
        raw.push('.');
        *pos += 1;
        *col += 1;
        while *pos < chars.len() && (chars[*pos].is_ascii_digit() || chars[*pos] == '_') {
            if chars[*pos] != '_' {
                raw.push(chars[*pos]);
            }
            *pos += 1;
            *col += 1;
        }
    }

    if *pos < chars.len() && (chars[*pos] == 'e' || chars[*pos] == 'E') {
        is_float = true;
        raw.push('e');
        *pos += 1;
        *col += 1;
        if *pos < chars.len() && (chars[*pos] == '+' || chars[*pos] == '-') {
            raw.push(chars[*pos]);
            *pos += 1;
            *col += 1;
        }
        let mut exp_digits = 0;
        while *pos < chars.len() && (chars[*pos].is_ascii_digit() || chars[*pos] == '_') {
            if chars[*pos] != '_' {
                raw.push(chars[*pos]);
                exp_digits += 1;
            }
            *pos += 1;
            *col += 1;
        }
        if exp_digits == 0 {
            return Err(Diagnostic::error(
                Span::new(file_id, line, start_col, *col - start_col),
                "expected digits in float exponent",
            ));
        }
    }

    if is_float {
        let v: f64 = raw.parse().map_err(|_| {
            Diagnostic::error(
                Span::new(file_id, line, start_col, *col - start_col),
                "invalid floating-point literal",
            )
        })?;
        Ok(TokenKind::Float(v))
    } else {
        let v: i128 = raw.parse().map_err(|_| {
            Diagnostic::error(
                Span::new(file_id, line, start_col, *col - start_col),
                "integer literal overflows 128-bit range",
            )
        })?;
        Ok(TokenKind::Int(v))
    }
}
