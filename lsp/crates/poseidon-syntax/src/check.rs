//! Precedence-correct semantic checker for SQF — a faithful port of the
//! `_checkOnly` path of `engine/Evaluator/express.cpp` (`Vyhod` / `VyhCast`).
//!
//! Instead of evaluating *values* we evaluate *types* (`GameType` bitmasks),
//! using the engine's own overload-resolution rule: an operand of mask `T`
//! satisfies an argument of type `A` iff `A & T != 0`; the result is the union
//! of the return types of all matching overloads, and an empty union is a type
//! error (`possible == 0` in `VyhCast`). Variables, whose type we can't know
//! statically, get the `VALUE_MASK` ("any value"), so they satisfy every
//! argument — which is exactly why this raises **zero false positives** on real
//! scripts while still catching concrete type mismatches like `player setDamage "x"`.

use crate::common::*;
use crate::sqf::{lex, Dialect};
use poseidon_catalog as cat;
use std::collections::HashSet;

#[derive(Clone, Copy)]
struct Tk<'a> {
    tok: Tok,
    span: Span,
    text: &'a str,
    line: u32,
}

struct Checker<'a> {
    t: Vec<Tk<'a>>,
    pos: usize,
    diags: Vec<Diagnostic>,
    /// Names introduced by in-file `#define` directives. We can't expand them
    /// (that needs a preprocessor), but knowing the names lets us treat their
    /// invocations as opaque values instead of flagging them as unknown commands.
    macros: HashSet<String>,
}

/// Collect the names defined by `#define NAME …` directives in the lexed source.
fn collect_macros(toks: &[Token], src: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for t in toks {
        if t.tok != Tok::Macro {
            continue;
        }
        let text = &src[t.span.start..t.span.end];
        let rest = text.trim_start_matches('#').trim_start();
        if let Some(after) = rest.strip_prefix("define") {
            let name: String = after
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                set.insert(name);
            }
        }
    }
    set
}

/// Run the semantic checker over `src`.
pub fn check(src: &str, dialect: Dialect) -> Vec<Diagnostic> {
    let (toks, _) = lex(src, dialect);
    // line number per token (SQS is line-structured)
    let mut newlines: Vec<usize> = vec![];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            newlines.push(i);
        }
    }
    let line_of = |off: usize| newlines.partition_point(|&n| n < off) as u32;
    let macros = collect_macros(&toks, src);
    let all: Vec<Tk> = toks
        .iter()
        // Comments and preprocessor-directive lines are not code the evaluator
        // sees (the engine preprocesses first), so they never participate in the
        // type check.
        .filter(|t| t.tok != Tok::Comment && t.tok != Tok::Macro)
        .map(|t| Tk { tok: t.tok, span: t.span, text: &src[t.span.start..t.span.end], line: line_of(t.span.start) })
        .collect();

    let mut diags = Vec::new();
    match dialect {
        Dialect::Sqf => {
            let mut c = Checker { t: all, pos: 0, diags: Vec::new(), macros: macros.clone() };
            c.block(None);
            diags = c.diags;
        }
        Dialect::Sqs => {
            // SQS is line-oriented: each physical line is a statement. Strip the
            // leading sigil, skip `#labels` and `?cond:stmt` lines (the `:`
            // separator isn't tokenised), then check the rest as an expression.
            let mut i = 0;
            while i < all.len() {
                let line = all[i].line;
                let start = i;
                while i < all.len() && all[i].line == line {
                    i += 1;
                }
                let mut slice = &all[start..i];
                match slice.first().map(|t| (t.tok, t.text)) {
                    Some((Tok::Label, _)) => continue,
                    Some((Tok::Keyword, "?")) => continue,
                    Some((Tok::Keyword, "~")) | Some((Tok::Keyword, "@")) | Some((Tok::Keyword, "&")) => {
                        slice = &slice[1..];
                    }
                    _ => {}
                }
                if slice.is_empty() {
                    continue;
                }
                let mut c =
                    Checker { t: slice.to_vec(), pos: 0, diags: Vec::new(), macros: macros.clone() };
                c.block(None);
                diags.append(&mut c.diags);
            }
        }
    }
    diags
}

impl<'a> Checker<'a> {
    fn peek(&self) -> Option<Tk<'a>> {
        self.t.get(self.pos).copied()
    }
    fn bump(&mut self) -> Option<Tk<'a>> {
        let t = self.t.get(self.pos).copied();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn at_text(&self, s: &str) -> bool {
        self.peek().map_or(false, |t| t.text == s)
    }
    fn at_end(&self) -> bool {
        self.pos >= self.t.len()
    }
    /// Consume a balanced `( … )` group starting at the current `(` token.
    fn skip_balanced(&mut self) {
        let mut depth = 0i32;
        while let Some(t) = self.peek() {
            match t.text {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth -= 1;
                    self.bump();
                    if depth <= 0 {
                        return;
                    }
                    continue;
                }
                _ => {}
            }
            self.bump();
        }
    }
    fn err(&mut self, span: Span, code: &'static str, message: String) {
        self.diags.push(Diagnostic { span, severity: Severity::Warning, code, message });
    }

    /// Parse a `;`-separated statement list until `closer` (or EOF).
    fn block(&mut self, closer: Option<&str>) {
        loop {
            while self.at_text(";") {
                self.bump();
            }
            if self.at_end() || closer.map_or(false, |c| self.at_text(c)) {
                break;
            }
            let before = self.pos;
            self.statement(closer);
            if self.pos == before {
                self.bump(); // guarantee progress
            }
        }
    }

    fn statement(&mut self, closer: Option<&str>) {
        // assignment `lhs = rhs` (a single top-level `=` token)?
        if let Some(eq) = self.find_top_level(closer, "=") {
            // skip the lhs (a variable / `private _x`) and the `=`
            self.pos = eq + 1;
        }
        self.expr(0);
        // anything left before the statement terminator is unexpected
        if let Some(t) = self.peek() {
            if t.text != ";" && Some(t.text) != closer {
                self.err(t.span, "EvalSemicolon",
                    format!("unexpected `{}` (expected operator or end of statement)", t.text));
                // recover: skip to the next `;`/closer
                while let Some(t) = self.peek() {
                    if t.text == ";" || Some(t.text) == closer {
                        break;
                    }
                    self.bump();
                }
            }
        }
    }

    /// Index of the next top-level (depth-0) token with text `what`, before the
    /// statement terminator, or `None`.
    fn find_top_level(&self, closer: Option<&str>, what: &str) -> Option<usize> {
        let mut depth = 0i32;
        let mut i = self.pos;
        while let Some(t) = self.t.get(i) {
            match t.text {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                ";" if depth == 0 => return None,
                _ if depth == 0 && Some(t.text) == closer => return None,
                _ if depth == 0 && t.text == what => return Some(i),
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// Parse an expression with the given minimum binding power; return its
    /// result-type mask.
    fn expr(&mut self, min_bp: u8) -> u32 {
        let mut left = self.nud();
        loop {
            let Some(op) = self.peek() else { break };
            let Some((bp, name, unknown)) = self.infix(op) else { break };
            if bp <= min_bp {
                break;
            }
            self.bump(); // consume the operator
            let right = self.expr(bp);
            left = if unknown {
                self.err(op.span, "EvalOper",
                    format!("unknown command `{}` (not a registered command in this engine)", op.text));
                cat::VALUE_MASK
            } else {
                self.apply_binary(&name, op.span, left, right)
            };
        }
        left
    }

    /// If `op` can act as an infix operator here, return `(binding power,
    /// operator name, is_unknown)`.
    fn infix(&self, op: Tk) -> Option<(u8, String, bool)> {
        match op.tok {
            Tok::Operator if op.text != "=" && op.text != "!" => {
                if cat::arities(op.text).2 {
                    Some((cat::binary_priority(op.text), op.text.to_string(), false))
                } else {
                    None
                }
            }
            Tok::Keyword | Tok::Command => {
                if cat::arities(op.text).2 {
                    Some((cat::binary_priority(op.text), op.text.to_string(), false))
                } else {
                    None
                }
            }
            // an unknown identifier in operator position is a misspelled infix
            // command — unless it is a known macro, which we leave opaque
            Tok::GlobalVar if !self.macros.contains(op.text) => Some((4, op.text.to_string(), true)),
            _ => None,
        }
    }

    /// Null denotation: parse a value / prefix form; return its type mask.
    fn nud(&mut self) -> u32 {
        let Some(t) = self.peek() else {
            // ran off the end while a value was required
            let span = self.t.last().map(|t| t.span).unwrap_or(Span::new(0, 0));
            self.err(span, "EvalType", "missing operand".into());
            return cat::VALUE_MASK;
        };
        match t.tok {
            Tok::Number => {
                self.bump();
                1
            }
            Tok::Str => {
                self.bump();
                8
            }
            Tok::LocalVar => {
                self.bump();
                cat::VALUE_MASK
            }
            Tok::GlobalVar => {
                self.bump();
                // A preprocessor macro we can't expand: treat its invocation as an
                // opaque value, swallowing an immediately-following `(arg, …)` so it
                // is not read as a command applied to that parenthesis.
                if self.macros.contains(t.text) {
                    if self.at_text("(") {
                        self.skip_balanced();
                    }
                    return cat::VALUE_MASK;
                }
                // An unknown identifier immediately followed by a value is being
                // applied like a (misspelled) prefix command; otherwise it is a
                // perfectly legal global variable.
                if self.peek().map_or(false, is_value_start) {
                    self.err(t.span, "EvalOper",
                        format!("unknown command `{}` (not a registered command in this engine)", t.text));
                    self.expr(cat::UNARY_BP); // consume the operand to recover
                    return cat::VALUE_MASK;
                }
                cat::VALUE_MASK
            }
            Tok::Punct if t.text == "(" => {
                self.bump();
                let m = self.expr(0);
                if self.at_text(")") {
                    self.bump();
                } else if let Some(p) = self.peek() {
                    self.err(p.span, "EvalCloseB", "missing `)`".into());
                }
                m
            }
            Tok::Punct if t.text == "[" => {
                self.bump();
                if !self.at_text("]") {
                    loop {
                        self.expr(0);
                        if self.at_text(",") {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                if self.at_text("]") {
                    self.bump();
                }
                2 // ARRAY
            }
            Tok::Punct if t.text == "{" => {
                self.bump();
                self.block(Some("}")); // a code block is checked as its own statements
                if self.at_text("}") {
                    self.bump();
                }
                8 // code block is a STRING value in this engine
            }
            Tok::Operator => {
                // symbolic prefix (`-`, `+`, `!`) — resolve via the unary catalog.
                // An operator with no unary form (`*`, `/`, `%`, `^`, comparisons)
                // in value position is a misplaced operator, e.g. `5 + * 3`.
                let has_unary = cat::arities(t.text).1;
                self.bump();
                let operand = self.expr(cat::UNARY_BP);
                if has_unary {
                    self.apply_unary(t.text, t.span, operand)
                } else {
                    self.err(t.span, "EvalOper", format!("`{}` has no left operand", t.text));
                    cat::VALUE_MASK
                }
            }
            Tok::Keyword | Tok::Command => {
                let (nular, unary, _binary) = cat::arities(t.text);
                // Does a value operand actually follow this command name?
                let operand_follows = self.t.get(self.pos + 1).copied().map_or(false, is_value_start);
                if unary && operand_follows {
                    self.bump();
                    let operand = self.expr(cat::UNARY_BP);
                    self.apply_unary(t.text, t.span, operand)
                } else if nular {
                    self.bump();
                    // a purely-nular command must not be given an argument
                    if let Some(n) = self.peek() {
                        if starts_operand(n) {
                            self.err(n.span, "EvalType", format!("`{}` takes no arguments", t.text));
                        }
                    }
                    nular_ret(t.text)
                } else if unary {
                    // A unary command with no operand can only be a same-named
                    // *variable* shadowing the command (express.cpp: a set
                    // variable hides the command of the same name). Don't flag.
                    self.bump();
                    cat::VALUE_MASK
                } else {
                    // binary-only command with no left operand
                    self.bump();
                    self.err(t.span, "EvalType", format!("`{}` is missing its left operand", t.text));
                    cat::VALUE_MASK
                }
            }
            _ => {
                // a closer / separator where a value was required
                self.err(t.span, "EvalType", "missing operand".into());
                cat::VALUE_MASK
            }
        }
    }

    fn apply_unary(&mut self, name: &str, span: Span, operand: u32) -> u32 {
        let mut possible = 0u32;
        let mut had = false;
        for c in cat::lookup(name) {
            if c.kind != "unary" {
                continue;
            }
            had = true;
            if c.arg_bits(0) & operand != 0 {
                possible |= c.ret_bits();
            }
        }
        if had && possible == 0 {
            self.err(span, "EvalType",
                format!("`{name}` cannot be applied to {}", type_label(operand)));
            return cat::VALUE_MASK;
        }
        if possible == 0 {
            cat::VALUE_MASK
        } else {
            possible
        }
    }

    fn apply_binary(&mut self, name: &str, span: Span, left: u32, right: u32) -> u32 {
        let mut possible = 0u32;
        let mut had = false;
        for c in cat::lookup(name) {
            if c.kind != "binary" {
                continue;
            }
            had = true;
            if c.arg_bits(0) & left != 0 && c.arg_bits(1) & right != 0 {
                possible |= c.ret_bits();
            }
        }
        if had && possible == 0 {
            self.err(span, "EvalType",
                format!("`{name}` cannot be applied to {} and {}", type_label(left), type_label(right)));
            return cat::VALUE_MASK;
        }
        if possible == 0 {
            cat::VALUE_MASK
        } else {
            possible
        }
    }
}

/// Union of the return types of the nular overloads of `name`.
fn nular_ret(name: &str) -> u32 {
    let mut m = 0u32;
    for c in cat::lookup(name) {
        if c.kind == "nular" {
            m |= c.ret_bits();
        }
    }
    if m == 0 {
        cat::VALUE_MASK
    } else {
        m
    }
}

fn starts_operand(t: Tk) -> bool {
    matches!(t.tok, Tok::Number | Tok::Str)
        || (t.tok == Tok::Punct && matches!(t.text, "(" | "[" | "{"))
}

/// `true` if `t` can begin a value (so an identifier directly before it is being
/// applied as a prefix command rather than used as a bare variable).
fn is_value_start(t: Tk) -> bool {
    match t.tok {
        Tok::Number | Tok::Str | Tok::LocalVar | Tok::GlobalVar => true,
        Tok::Punct => matches!(t.text, "(" | "[" | "{"),
        Tok::Keyword | Tok::Command => {
            let (n, u, _) = cat::arities(t.text);
            n || u
        }
        _ => false,
    }
}

/// Human-readable name for a type mask (only the common single types).
fn type_label(mask: u32) -> &'static str {
    match mask {
        1 => "a number",
        2 => "an array",
        4 => "a boolean",
        8 => "a string",
        16 => "nothing",
        0x100 => "an object",
        0x2000 => "a group",
        0x1000 => "a side",
        _ => "this value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str) -> Vec<Diagnostic> {
        check(s, Dialect::Sqf)
    }

    #[test]
    fn valid_scripts_are_clean() {
        for s in [
            r#"_grp = group player; {_x setDamage 1} forEach units _grp;"#,
            r#"if (alive player) then { hint "ok" } else { hint "dead" };"#,
            r#"_n = count units group player;"#,
            r#"_d = player distance _veh;"#,
            r#"_s = "a" + "b"; _arr = [1,2,3] + [4];"#,
            r#"while {alive player} do { player setDamage 0 };"#,
        ] {
            let d = run(s);
            assert!(d.is_empty(), "false positive on `{s}`: {d:?}");
        }
    }

    #[test]
    fn flags_binary_type_mismatch() {
        // setDamage expects <OBJECT> setDamage <SCALAR>; a string right operand is wrong
        let d = run(r#"player setDamage "x""#);
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].message.contains("setDamage"), "{:?}", d[0].message);
    }

    #[test]
    fn flags_unknown_and_nular_and_unary() {
        assert!(run("_u serDamage 1").iter().any(|d| d.message.contains("unknown command")));
        assert!(run("player 5").iter().any(|d| d.message.contains("takes no arguments")));
        assert!(run("_x = getPos 5").iter().any(|d| d.message.contains("getPos")));
    }

    #[test]
    fn leaves_variables_alone() {
        // unknown operand types (variables) never produce type errors
        let d = run("_a setDamage _b; result = foo + bar;");
        assert!(d.is_empty(), "{d:?}");
    }

    /// Expressions the engine accepts — each grounded in a real overload row.
    /// Must produce **no** diagnostics.
    #[test]
    fn accept_corpus() {
        let cases = [
            "player setDamage 1",                  // OBJECT × SCALAR  (GameStateExt setDamage)
            r#""a" + "b""#,                        // STRING + STRING  (+ soucet)
            "1 + 2",                               // SCALAR + SCALAR
            "[1,2] + [3]",                         // ARRAY + ARRAY
            "_d = player distance _veh",           // OBJECT × OBJECT
            "_n = count units group player",       // group:OBJECT→GROUP, units:GROUP→ARRAY, count:ARRAY→SCALAR
            r#"if (alive player) then { hint "x" }"#, // if:BOOL→IF, then:(IF,STRING)→ANY
            r#"if (alive player) then { hint "a" } else { hint "b" }"#, // else:(STRING,STRING)→ARRAY, then:(IF,ARRAY)
            "while {alive player} do { player setDamage 0 }", // while:STRING→WHILE, do:(WHILE,STRING)
            "_p = getPos player",                  // getPos:OBJECT→ARRAY
            "_r = random 10",                      // random:SCALAR→SCALAR
            "_b = alive player",                   // alive:OBJECT→BOOL
            r#"private "_x""#,                     // private:STRING|ARRAY
            r#"private ["_x"]"#,                    // private:ARRAY form
            "_e = _arr select 0",                  // select:(ARRAY,SCALAR)
        ];
        for src in cases {
            let d = run(src);
            assert!(d.is_empty(), "false positive on `{src}`: {d:?}");
        }
    }

    /// Expressions the engine rejects — each must produce at least one diagnostic.
    #[test]
    fn reject_corpus() {
        let cases: &[(&str, &str)] = &[
            (r#"player setDamage "x""#, "OBJECT × STRING — setDamage wants SCALAR"),
            (r#"_n = 1 + "two""#, "SCALAR + STRING is not an overload of +"),
            ("player distance [0,0,0]", "distance is OBJECT × OBJECT in classic OFP, not position"),
            ("_p = getPos 5", "getPos wants OBJECT, given SCALAR"),
            ("player 5", "player is nular — cannot take an argument"),
            ("x = 1 +", "binary + with no right operand"),
            ("_u serDamage 1", "serDamage is not a registered command"),
            (r#"hnit "hello""#, "hnit is not a registered command"),
        ];
        for (src, why) in cases {
            let d = run(src);
            assert!(!d.is_empty(), "expected a diagnostic for `{src}` ({why})");
        }
    }
}
