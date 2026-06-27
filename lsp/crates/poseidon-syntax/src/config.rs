//! ParamFile (config) lexer + class-tree extraction.
//!
//! One grammar serves `.cfg`, `.ext`, `config.cpp` and `mission.sqm`
//! (`ParamFile.cpp`). Two layers: a C-style preprocessor (`Preproc.cpp`, only
//! `// /* */` comments and the eight `#`-directives) then a recursive-descent
//! tree parser. Statements: `class N {…};`, `class N : Base {…};`, `enum {…};`,
//! `name = value;`, `name[] = { … };`. Strings use `""` doubling (no backslash
//! escapes); numbers are int / `0x`-hex / float / `dbNN`.

use crate::common::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prev {
    None,
    Class,
    Colon,
}

/// Lex a ParamFile document. Returns `(tokens, diagnostics)`.
pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let b = src.as_bytes();
    let mut pos = 0usize;
    let mut tokens = Vec::new();
    let mut diags = Vec::new();
    let mut depth: Vec<usize> = Vec::new();
    let mut prev = Prev::None;

    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let line_start = |p: usize| -> bool {
        let mut i = p;
        while i > 0 {
            match b[i - 1] {
                b'\n' => return true,
                b' ' | b'\t' | b'\r' => i -= 1,
                _ => return false,
            }
        }
        true
    };

    while pos < b.len() {
        let c = at(pos);
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => pos += 1,
            b'#' if line_start(pos) => {
                let s = pos;
                while pos < b.len() && at(pos) != b'\n' {
                    // line continuation: `\` then newline keeps the directive going
                    if at(pos) == b'\\' && (at(pos + 1) == b'\n' || (at(pos + 1) == b'\r' && at(pos + 2) == b'\n')) {
                        pos += if at(pos + 1) == b'\r' { 3 } else { 2 };
                    } else {
                        pos += 1;
                    }
                }
                tokens.push(Token { span: Span::new(s, pos), tok: Tok::Macro });
            }
            b'/' if at(pos + 1) == b'/' => {
                let s = pos;
                while pos < b.len() && at(pos) != b'\n' {
                    pos += 1;
                }
                tokens.push(Token { span: Span::new(s, pos), tok: Tok::Comment });
            }
            b'/' if at(pos + 1) == b'*' => {
                let s = pos;
                pos += 2;
                let mut closed = false;
                while pos < b.len() {
                    if at(pos) == b'*' && at(pos + 1) == b'/' {
                        pos += 2;
                        closed = true;
                        break;
                    }
                    pos += 1;
                }
                tokens.push(Token { span: Span::new(s, pos), tok: Tok::Comment });
                if !closed {
                    diags.push(Diagnostic {
                        span: Span::new(s, pos),
                        severity: Severity::Error,
                        code: "unterminated-block-comment",
                        message: "unterminated `/* */` comment".into(),
                    });
                }
            }
            b'"' => {
                let s = pos;
                pos += 1;
                let mut closed = false;
                while pos < b.len() {
                    if at(pos) == b'"' {
                        if at(pos + 1) == b'"' {
                            pos += 2;
                        } else {
                            pos += 1;
                            closed = true;
                            break;
                        }
                    } else if at(pos) == b'\n' {
                        break; // a raw newline ends the (broken) string in config
                    } else {
                        pos += 1;
                    }
                }
                tokens.push(Token { span: Span::new(s, pos), tok: Tok::Str });
                if !closed {
                    diags.push(Diagnostic {
                        span: Span::new(s, pos),
                        severity: Severity::Error,
                        code: "unterminated-string",
                        message: "unterminated string literal".into(),
                    });
                }
                prev = Prev::None;
            }
            b'0' if matches!(at(pos + 1), b'x' | b'X') => {
                let s = pos;
                pos += 2;
                while at(pos).is_ascii_hexdigit() {
                    pos += 1;
                }
                tokens.push(Token { span: Span::new(s, pos), tok: Tok::Number });
            }
            c if c.is_ascii_digit() || (c == b'.' && at(pos + 1).is_ascii_digit()) => {
                let s = pos;
                while at(pos).is_ascii_digit() {
                    pos += 1;
                }
                if at(pos) == b'.' {
                    pos += 1;
                    while at(pos).is_ascii_digit() {
                        pos += 1;
                    }
                }
                if matches!(at(pos), b'e' | b'E') {
                    let mut j = pos + 1;
                    if matches!(at(j), b'+' | b'-') {
                        j += 1;
                    }
                    if at(j).is_ascii_digit() {
                        pos = j;
                        while at(pos).is_ascii_digit() {
                            pos += 1;
                        }
                    }
                }
                tokens.push(Token { span: Span::new(s, pos), tok: Tok::Number });
            }
            b'{' | b'(' | b'[' => {
                if c == b'{' {
                    depth.push(pos);
                }
                let s = pos;
                pos += 1;
                tokens.push(Token { span: Span::new(s, pos), tok: Tok::Punct });
            }
            b'}' | b')' | b']' => {
                if c == b'}' && depth.pop().is_none() {
                    diags.push(Diagnostic {
                        span: Span::new(pos, pos + 1),
                        severity: Severity::Error,
                        code: "unbalanced-brace",
                        message: "unmatched `}`".into(),
                    });
                }
                let s = pos;
                pos += 1;
                tokens.push(Token { span: Span::new(s, pos), tok: Tok::Punct });
            }
            b':' => {
                let s = pos;
                pos += 1;
                tokens.push(Token { span: Span::new(s, pos), tok: Tok::Operator });
                prev = Prev::Colon;
            }
            b'=' | b';' | b',' => {
                let s = pos;
                pos += 1;
                tokens.push(Token { span: Span::new(s, pos), tok: if c == b'=' { Tok::Operator } else { Tok::Punct } });
                prev = Prev::None;
            }
            c if is_ident_start(c) => {
                let s = pos;
                while is_ident_continue(at(pos)) {
                    pos += 1;
                }
                let word = std::str::from_utf8(&b[s..pos]).unwrap_or("");
                let lower = word.to_ascii_lowercase();
                let tok = if lower == "class" || lower == "enum" {
                    prev = Prev::Class;
                    Tok::StructKeyword
                } else if prev == Prev::Class || prev == Prev::Colon {
                    prev = Prev::None;
                    Tok::TypeName
                } else {
                    // member name if the next significant char is `=` or `[`
                    let mut j = pos;
                    while matches!(at(j), b' ' | b'\t') {
                        j += 1;
                    }
                    prev = Prev::None;
                    if at(j) == b'=' || at(j) == b'[' {
                        Tok::Property
                    } else {
                        Tok::GlobalVar
                    }
                };
                tokens.push(Token { span: Span::new(s, pos), tok });
            }
            _ => pos += 1,
        }
    }

    for &open in &depth {
        diags.push(Diagnostic {
            span: Span::new(open, open + 1),
            severity: Severity::Error,
            code: "unbalanced-brace",
            message: "unmatched `{`".into(),
        });
    }

    (tokens, diags)
}

/// A `class` node in the config tree.
#[derive(Debug, Clone)]
pub struct ClassSymbol {
    pub name: String,
    pub base: Option<String>,
    /// Byte span of the name identifier (for the symbol's selection range).
    pub name_span: Span,
    /// Byte span of the whole `class … { … }` (or forward decl).
    pub full_span: Span,
    pub children: Vec<ClassSymbol>,
}

/// Extract the nested class tree for document symbols / outline.
pub fn symbols(src: &str) -> Vec<ClassSymbol> {
    let b = src.as_bytes();
    let (roots, _) = parse_classes(b, 0, b.len());
    roots
}

/// Parse `class` definitions in `b[start..end]`, returning `(classes, _)`.
fn parse_classes(b: &[u8], start: usize, end: usize) -> (Vec<ClassSymbol>, usize) {
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut i = start;
    let mut out = Vec::new();
    while i < end {
        // find the next `class` keyword at a token boundary
        if matches_kw(b, i, b"class") {
            let kw_start = i;
            i += 5;
            i = skip_ws(b, i, end);
            // class name
            let ns = i;
            while i < end && is_ident_continue(at(i)) {
                i += 1;
            }
            if i == ns {
                continue;
            }
            let name = String::from_utf8_lossy(&b[ns..i]).into_owned();
            let name_span = Span::new(ns, i);
            i = skip_ws(b, i, end);
            // optional base
            let mut base = None;
            if at(i) == b':' {
                i += 1;
                i = skip_ws(b, i, end);
                let bs = i;
                while i < end && is_ident_continue(at(i)) {
                    i += 1;
                }
                if i > bs {
                    base = Some(String::from_utf8_lossy(&b[bs..i]).into_owned());
                }
                i = skip_ws(b, i, end);
            }
            if at(i) == b'{' {
                let body_start = i + 1;
                let body_end = match_brace(b, i, end);
                let (children, _) = parse_classes(b, body_start, body_end);
                let mut full_end = body_end + 1;
                let semi = skip_ws(b, full_end, end);
                if at(semi) == b';' {
                    full_end = semi + 1;
                }
                out.push(ClassSymbol {
                    name,
                    base,
                    name_span,
                    full_span: Span::new(kw_start, full_end.min(end)),
                    children,
                });
                i = full_end;
            } else {
                // forward declaration `class Name;`
                out.push(ClassSymbol {
                    name,
                    base,
                    name_span,
                    full_span: Span::new(kw_start, i),
                    children: Vec::new(),
                });
            }
        } else {
            i += 1;
        }
    }
    (out, i)
}

fn matches_kw(b: &[u8], i: usize, kw: &[u8]) -> bool {
    if i + kw.len() > b.len() {
        return false;
    }
    // token boundary before
    if i > 0 && is_ident_continue(b[i - 1]) {
        return false;
    }
    for (k, &kc) in kw.iter().enumerate() {
        if b[i + k].to_ascii_lowercase() != kc {
            return false;
        }
    }
    // boundary after
    !b.get(i + kw.len()).copied().map_or(false, is_ident_continue)
}

fn skip_ws(b: &[u8], mut i: usize, end: usize) -> usize {
    while i < end && matches!(b.get(i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        i += 1;
    }
    i
}

/// Given `b[open]==b'{'`, return the index of the matching `}` (or `end`).
fn match_brace(b: &[u8], open: usize, end: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open;
    let mut in_str = false;
    while i < end {
        let c = b[i];
        if in_str {
            if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_class_and_member() {
        let (toks, diags) = lex("class Car : Vehicle { maxSpeed = 80; magazines[] = {\"x\"}; };");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(toks.iter().any(|t| t.tok == Tok::StructKeyword));
        assert!(toks.iter().any(|t| t.tok == Tok::TypeName)); // Car, Vehicle
        assert!(toks.iter().any(|t| t.tok == Tok::Property)); // maxSpeed, magazines
    }

    #[test]
    fn builds_class_tree() {
        let src = "class CfgVehicles { class Car {}; class Tank : Car {}; };";
        let syms = symbols(src);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "CfgVehicles");
        assert_eq!(syms[0].children.len(), 2);
        assert_eq!(syms[0].children[1].name, "Tank");
        assert_eq!(syms[0].children[1].base.as_deref(), Some("Car"));
    }

    #[test]
    fn flags_unbalanced_brace() {
        let (_t, d) = lex("class X { y = 1;");
        assert!(d.iter().any(|d| d.code == "unbalanced-brace"));
    }

    fn kinds(src: &str) -> Vec<Tok> {
        lex(src).0.iter().map(|t| t.tok).collect()
    }

    #[test]
    fn preprocessor_directives_tagged() {
        // Preproc.cpp recognises exactly these eight directives at line start
        for d in [
            "#include \"x.hpp\"",
            "#include <x.hpp>",
            "#define FOO 1",
            "#define BAR(a,b) a##b",
            "#undef FOO",
            "#ifdef FOO",
            "#ifndef FOO",
            "#else",
            "#endif",
        ] {
            assert!(kinds(d).contains(&Tok::Macro), "`{d}` should be a preprocessor line");
        }
    }

    #[test]
    fn comments_are_stage1_only() {
        // Preproc.cpp:379-388 — `//` line and `/* */` block comments
        assert!(kinds("// a comment").contains(&Tok::Comment));
        assert!(kinds("/* block\n comment */").contains(&Tok::Comment));
    }

    #[test]
    fn string_double_quote_escape() {
        // ParamFile.cpp GetWord:39-74 — "" is the only escape, no backslash escapes
        let (toks, d) = lex(r#"name = "say ""hi""";"#);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(toks.iter().filter(|t| t.tok == Tok::Str).count(), 1);
    }

    #[test]
    fn number_forms() {
        // ParamFile.cpp: decimal int, 0x hex, decimal/scientific float
        assert!(kinds("a = 42;").contains(&Tok::Number));
        assert!(kinds("a = 0xFF;").contains(&Tok::Number));
        assert!(kinds("a = 3.16e-5;").contains(&Tok::Number));
    }

    #[test]
    fn class_inheritance_and_members() {
        // class Name : Base { member = v; arr[] = {..}; }
        let toks = lex("class Car : Vehicle { maxSpeed = 80; magazines[] = {\"x\"}; };").0;
        // `class` keyword, `Car`/`Vehicle` type names, `maxSpeed`/`magazines` properties
        assert!(toks.iter().any(|t| t.tok == Tok::StructKeyword));
        assert_eq!(toks.iter().filter(|t| t.tok == Tok::TypeName).count(), 2); // Car, Vehicle
        assert_eq!(toks.iter().filter(|t| t.tok == Tok::Property).count(), 2); // maxSpeed, magazines
    }

    #[test]
    fn enum_keyword_recognised() {
        assert!(kinds("enum { ONE, TWO = 2 };").contains(&Tok::StructKeyword));
    }

    #[test]
    fn forward_declaration_and_nested_tree() {
        // `class Base;` forward decl + nested children with base links
        let syms = symbols("class Base; class CfgVehicles { class Car : Base {}; };");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "Base");
        assert!(syms[0].children.is_empty()); // forward declaration
        assert_eq!(syms[1].children[0].base.as_deref(), Some("Base"));
    }
}
