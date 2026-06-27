//! SQF / SQS lexer + diagnostics.
//!
//! Per `express.cpp`: identifiers are `[A-Za-z_][A-Za-z0-9_]*`, fully
//! case-insensitive; numbers are decimal/scientific plus `0x` and `$` hex;
//! strings are double-quoted with `""` as the only escape. The engine's raw
//! evaluator has no comment syntax, but real scripts are run through the
//! preprocessor (`preprocessFile`), so we accept `//` and `/* */` like the
//! config preprocessor does. SQS adds line sigils (`#`, `~`, `@`, `&`, `?`,
//! and `;` line comments) handled in [`Dialect::Sqs`].

use crate::common::*;
use poseidon_catalog as cat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqf,
    Sqs,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    dialect: Dialect,
    tokens: Vec<Token>,
    diags: Vec<Diagnostic>,
    /// open delimiter byte + its position, for balance checking.
    stack: Vec<(u8, usize)>,
}

/// Lex `src`. Returns `(tokens, diagnostics)`.
pub fn lex(src: &str, dialect: Dialect) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lx = Lexer {
        src: src.as_bytes(),
        pos: 0,
        dialect,
        tokens: Vec::new(),
        diags: Vec::new(),
        stack: Vec::new(),
    };
    lx.run();
    (lx.tokens, lx.diags)
}

/// Convenience: just the diagnostics.
pub fn diagnostics(src: &str, dialect: Dialect) -> Vec<Diagnostic> {
    lex(src, dialect).1
}

impl<'a> Lexer<'a> {
    fn at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }
    fn cur(&self) -> u8 {
        self.at(self.pos)
    }
    fn at_line_start(&self) -> bool {
        let mut i = self.pos;
        while i > 0 {
            let c = self.src[i - 1];
            if c == b'\n' {
                return true;
            }
            if c == b' ' || c == b'\t' || c == b'\r' {
                i -= 1;
            } else {
                return false;
            }
        }
        true
    }

    fn push(&mut self, start: usize, end: usize, tok: Tok) {
        self.tokens.push(Token { span: Span::new(start, end), tok });
    }

    /// `true` if `self.pos` is at the `#` of a preprocessor directive line
    /// (`#define`, `# include`, `#ifdef`, …). Allows optional spaces after `#`.
    fn is_preproc_directive(&self) -> bool {
        const PREPROC: &[&str] = &[
            "define", "include", "undef", "ifdef", "ifndef", "if", "else", "elif",
            "endif", "pragma", "line", "error", "warning",
        ];
        let mut i = self.pos + 1;
        while matches!(self.at(i), b' ' | b'\t') {
            i += 1;
        }
        let ws = i;
        while self.at(i).is_ascii_alphabetic() {
            i += 1;
        }
        let word = std::str::from_utf8(&self.src[ws..i]).unwrap_or("").to_ascii_lowercase();
        PREPROC.contains(&word.as_str())
    }

    /// Consume a preprocessor directive to end of its logical line, honouring
    /// `\`-newline continuations, as one [`Tok::Macro`].
    fn macro_line(&mut self) {
        let s = self.pos;
        while self.pos < self.src.len() {
            let c = self.cur();
            if c == b'\n' {
                break;
            }
            if c == b'\\' {
                // line continuation: backslash followed by (optional CR) LF
                let mut j = self.pos + 1;
                if self.at(j) == b'\r' {
                    j += 1;
                }
                if self.at(j) == b'\n' {
                    self.pos = j + 1; // jump onto the continued line
                    continue;
                }
            }
            self.pos += 1;
        }
        self.push(s, self.pos, Tok::Macro);
    }

    fn run(&mut self) {
        while self.pos < self.src.len() {
            let c = self.cur();
            // Preprocessor directive lines (#define / #include / #ifdef / …): the
            // engine runs its preprocessor BEFORE the evaluator, so these never
            // reach the parser. Lex the whole logical line as one Macro token (in
            // both dialects) so the checker skips it instead of flagging the body.
            if c == b'#' && self.at_line_start() && self.is_preproc_directive() {
                self.macro_line();
                continue;
            }
            // SQS line-leading sigils
            if self.dialect == Dialect::Sqs && self.at_line_start() {
                match c {
                    b';' => {
                        self.line_comment();
                        continue;
                    }
                    b'#' => {
                        self.rest_of_line(Tok::Label);
                        continue;
                    }
                    b'~' | b'@' | b'&' | b'?' => {
                        let s = self.pos;
                        self.pos += 1;
                        self.push(s, self.pos, Tok::Keyword);
                        continue;
                    }
                    _ => {}
                }
            }
            match c {
                b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => self.pos += 1,
                b'/' if self.at(self.pos + 1) == b'/' => self.line_comment(),
                b'/' if self.at(self.pos + 1) == b'*' => self.block_comment(),
                b'"' => self.string(),
                b'$' if self.at(self.pos + 1).is_ascii_hexdigit() => self.hex_dollar(),
                b'0' if matches!(self.at(self.pos + 1), b'x' | b'X') => self.hex_0x(),
                c if c.is_ascii_digit() => self.number(),
                b'.' if self.at(self.pos + 1).is_ascii_digit() => self.number(),
                c if is_ident_start(c) => self.ident(),
                b'(' | b'[' | b'{' => {
                    let s = self.pos;
                    self.stack.push((c, s));
                    self.pos += 1;
                    self.push(s, self.pos, Tok::Punct);
                }
                b')' | b']' | b'}' => self.close_delim(c),
                b',' | b';' => {
                    let s = self.pos;
                    self.pos += 1;
                    self.push(s, self.pos, Tok::Punct);
                }
                _ => self.operator(),
            }
        }
        // unclosed delimiters
        let leftovers: Vec<(u8, usize)> = self.stack.drain(..).collect();
        for (b, at) in leftovers {
            self.diags.push(Diagnostic {
                span: Span::new(at, at + 1),
                severity: Severity::Error,
                code: "EvalOpenB",
                message: format!("unmatched `{}`", b as char),
            });
        }
    }

    fn line_comment(&mut self) {
        let s = self.pos;
        while self.pos < self.src.len() && self.cur() != b'\n' {
            self.pos += 1;
        }
        self.push(s, self.pos, Tok::Comment);
    }

    fn rest_of_line(&mut self, tok: Tok) {
        let s = self.pos;
        while self.pos < self.src.len() && self.cur() != b'\n' {
            self.pos += 1;
        }
        self.push(s, self.pos, tok);
    }

    fn block_comment(&mut self) {
        let s = self.pos;
        self.pos += 2;
        while self.pos < self.src.len() {
            if self.cur() == b'*' && self.at(self.pos + 1) == b'/' {
                self.pos += 2;
                self.push(s, self.pos, Tok::Comment);
                return;
            }
            self.pos += 1;
        }
        self.push(s, self.pos, Tok::Comment);
        self.diags.push(Diagnostic {
            span: Span::new(s, self.pos),
            severity: Severity::Error,
            code: "unterminated-block-comment",
            message: "unterminated `/* */` comment".into(),
        });
    }

    fn string(&mut self) {
        let s = self.pos;
        self.pos += 1; // opening quote
        loop {
            if self.pos >= self.src.len() {
                self.push(s, self.pos, Tok::Str);
                self.diags.push(Diagnostic {
                    span: Span::new(s, self.pos),
                    severity: Severity::Error,
                    code: "EvalNum", // engine raises EvalNum for unterminated strings too
                    message: "unterminated string literal".into(),
                });
                return;
            }
            match self.cur() {
                b'"' => {
                    if self.at(self.pos + 1) == b'"' {
                        self.pos += 2; // doubled quote = literal quote
                    } else {
                        self.pos += 1; // closing quote
                        self.push(s, self.pos, Tok::Str);
                        return;
                    }
                }
                b'\n' => {
                    // The engine tolerates newlines inside strings, but for editor
                    // recovery we terminate an unterminated string at end-of-line so a
                    // single missing quote doesn't swallow the rest of the file.
                    self.push(s, self.pos, Tok::Str);
                    self.diags.push(Diagnostic {
                        span: Span::new(s, self.pos),
                        severity: Severity::Error,
                        code: "EvalNum",
                        message: "unterminated string literal".into(),
                    });
                    return;
                }
                _ => self.pos += 1,
            }
        }
    }

    fn hex_dollar(&mut self) {
        let s = self.pos;
        self.pos += 1;
        while self.at(self.pos).is_ascii_hexdigit() {
            self.pos += 1;
        }
        self.push(s, self.pos, Tok::Number);
    }

    fn hex_0x(&mut self) {
        let s = self.pos;
        self.pos += 2;
        while self.at(self.pos).is_ascii_hexdigit() {
            self.pos += 1;
        }
        self.push(s, self.pos, Tok::Number);
    }

    fn number(&mut self) {
        let s = self.pos;
        while self.at(self.pos).is_ascii_digit() {
            self.pos += 1;
        }
        if self.cur() == b'.' {
            self.pos += 1;
            while self.at(self.pos).is_ascii_digit() {
                self.pos += 1;
            }
        }
        if matches!(self.cur(), b'e' | b'E') {
            let mut j = self.pos + 1;
            if matches!(self.at(j), b'+' | b'-') {
                j += 1;
            }
            if self.at(j).is_ascii_digit() {
                self.pos = j;
                while self.at(self.pos).is_ascii_digit() {
                    self.pos += 1;
                }
            }
        }
        self.push(s, self.pos, Tok::Number);
    }

    fn ident(&mut self) {
        let s = self.pos;
        while is_ident_continue(self.at(self.pos)) {
            self.pos += 1;
        }
        let name = std::str::from_utf8(&self.src[s..self.pos]).unwrap_or("");
        let tok = if name.starts_with('_') {
            Tok::LocalVar
        } else if cat::is_keyword(name) {
            Tok::Keyword
        } else if cat::is_command(name) {
            Tok::Command
        } else {
            Tok::GlobalVar
        };
        self.push(s, self.pos, tok);
    }

    fn close_delim(&mut self, c: u8) {
        let s = self.pos;
        self.pos += 1;
        let want = match c {
            b')' => b'(',
            b']' => b'[',
            _ => b'{',
        };
        match self.stack.last() {
            Some(&(open, _)) if open == want => {
                self.stack.pop();
            }
            _ => self.diags.push(Diagnostic {
                span: Span::new(s, self.pos),
                severity: Severity::Error,
                code: "EvalCloseB",
                message: format!("unmatched `{}`", c as char),
            }),
        }
        self.push(s, self.pos, Tok::Punct);
    }

    fn operator(&mut self) {
        let s = self.pos;
        // try 2-char operators first (matches Vyhod's 2-then-1 lookup)
        let two = [self.cur(), self.at(self.pos + 1)];
        const TWO: &[&[u8; 2]] = &[b"==", b"!=", b"<=", b">=", b"&&", b"||", b">>"];
        if TWO.iter().any(|t| t.as_slice() == two) {
            self.pos += 2;
        } else if matches!(self.cur(), b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'<' | b'>' | b'=' | b'!') {
            self.pos += 1;
        } else {
            // unknown byte: consume one so we always make progress
            self.pos += 1;
            return;
        }
        self.push(s, self.pos, Tok::Operator);
    }
}

/// Semantic diagnostics for SQF/SQS — unknown/misspelled commands, arity, and
/// type mismatches. Delegates to the precedence-correct checker in
/// [`crate::check`], a faithful port of `express.cpp`'s `_checkOnly` evaluator.
pub fn semantic_diagnostics(src: &str, dialect: Dialect) -> Vec<Diagnostic> {
    crate::check::check(src, dialect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_commands_vars_keywords() {
        let (toks, diags) = lex(r#"_x = player; if (true) then { hint "hi" }"#, Dialect::Sqf);
        assert!(diags.is_empty(), "{diags:?}");
        let kinds: Vec<Tok> = toks.iter().map(|t| t.tok).collect();
        assert!(kinds.contains(&Tok::LocalVar)); // _x
        assert!(kinds.contains(&Tok::Command)); // player / hint
        assert!(kinds.contains(&Tok::Keyword)); // if/then/true
        assert!(kinds.contains(&Tok::Str)); // "hi"
    }

    #[test]
    fn flags_unterminated_string() {
        let d = diagnostics(r#"hint "oops"#, Dialect::Sqf);
        assert!(d.iter().any(|d| d.code == "EvalNum"));
    }

    #[test]
    fn flags_unbalanced_braces() {
        let d = diagnostics("if (x) then { foo", Dialect::Sqf);
        assert!(d.iter().any(|d| d.code == "EvalOpenB"));
    }

    #[test]
    fn doubled_quote_escape() {
        let (toks, diags) = lex(r#""say ""hi""""#, Dialect::Sqf);
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(toks.iter().filter(|t| t.tok == Tok::Str).count(), 1);
    }

    // Helper: the kind of the single token covering the whole input.
    fn sole_kind(src: &str) -> Tok {
        let (toks, _) = lex(src, Dialect::Sqf);
        let t: Vec<&Token> = toks.iter().filter(|t| t.tok != Tok::Comment).collect();
        assert_eq!(t.len(), 1, "expected one token in `{src}`, got {:?}", t);
        t[0].tok
    }

    #[test]
    fn identifier_charset_and_classification() {
        // express.cpp Const(): ident = isalpha|'_' then (isalnum|'_'); '_'-prefix = local
        assert_eq!(sole_kind("_x"), Tok::LocalVar);
        assert_eq!(sole_kind("_unit1"), Tok::LocalVar);
        assert_eq!(sole_kind("player"), Tok::Command); // registered command
        assert_eq!(sole_kind("myGlobalVar"), Tok::GlobalVar); // unknown ident = global
        assert_eq!(sole_kind("if"), Tok::Keyword); // cosmetic keyword
    }

    #[test]
    fn case_insensitive_lookup() {
        // express.cpp lowercases every identifier before lookup (strlwr:182)
        assert_eq!(sole_kind("PLAYER"), Tok::Command);
        assert_eq!(sole_kind("SetPos"), Tok::Command);
        assert_eq!(sole_kind("THEN"), Tok::Keyword);
    }

    #[test]
    fn number_forms() {
        // express.cpp sejmid (strtod): decimal, fraction, scientific; plus 0x and $ hex
        for n in ["1", "3.14", ".5", "2.5e-4", "1.0e+10", "0xFF", "$1a2b"] {
            assert_eq!(sole_kind(n), Tok::Number, "`{n}` should be a number");
        }
    }

    #[test]
    fn string_double_quote_is_only_escape() {
        // express.cpp:222-256 — "" is the only escape; backslash is literal
        let (toks, d) = lex(r#""a\nb""#, Dialect::Sqf); // backslash NOT an escape
        assert!(d.is_empty());
        assert_eq!(toks.iter().filter(|t| t.tok == Tok::Str).count(), 1);
    }

    #[test]
    fn two_char_operators_tokenise_whole() {
        // Vyhod tries a 2-char operator before a 1-char one (express.cpp:1631)
        for op in ["==", "!=", "<=", ">=", "&&", "||"] {
            let src = format!("a {op} b");
            let (toks, _) = lex(&src, Dialect::Sqf);
            assert!(
                toks.iter().any(|t| t.tok == Tok::Operator && &src[t.span.start..t.span.end] == op),
                "`{op}` should be one operator token"
            );
        }
    }

    #[test]
    fn sqs_line_sigils() {
        // Scripts.cpp ProcessLine: `#`=label, `;`=comment, `~ @ &` line sigils
        let (toks, _) = lex("#loop\n; a comment\n~3\n@ alive player", Dialect::Sqs);
        assert!(toks.iter().any(|t| t.tok == Tok::Label));
        assert!(toks.iter().any(|t| t.tok == Tok::Comment));
        assert!(toks.iter().any(|t| t.tok == Tok::Keyword)); // ~ and @ sigils
    }
}
