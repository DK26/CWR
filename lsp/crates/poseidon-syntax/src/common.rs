//! Shared token / diagnostic vocabulary for both languages.

/// A byte range `[start, end)` into the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// Semantic classification of a token. Maps onto LSP semantic-token types in
/// the server, and onto TextMate scopes in the grammars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Comment,
    /// Double-quoted string (or `$STR...` localized ref in config).
    Str,
    Number,
    /// Core control-flow / literal word (`if`, `then`, `forEach`, `true`, ...).
    Keyword,
    /// A registered engine/core command used as such (not shadowed by a var).
    Command,
    /// `_`-prefixed local/script variable (SQF).
    LocalVar,
    /// Bare identifier that is not a known command — a global variable.
    GlobalVar,
    /// Symbolic operator (`+ - * / == && ...`) or assignment.
    Operator,
    /// Structural punctuation (`( ) [ ] { } , ;`).
    Punct,
    /// `class` / `enum` keyword (config).
    StructKeyword,
    /// A class name in a definition or base reference (config).
    TypeName,
    /// A member/property name on the left of `=` / `[]=` (config).
    Property,
    /// A preprocessor directive line (`#include`, `#define`, ...).
    Macro,
    /// SQS `#label`.
    Label,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub span: Span,
    pub tok: Tok,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    /// Short stable code, e.g. `unterminated-string`, mirroring the engine's
    /// `EvalError` family where applicable.
    pub code: &'static str,
    pub message: String,
}

/// Byte char-class helpers matching the engine's `isalphaext` / `isalnumext`.
pub fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}
pub fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}
