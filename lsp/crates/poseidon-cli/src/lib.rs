//! Core of `poseidon-check`: the one-shot, agent- and CI-friendly front end to
//! the same engine-grounded checker the LSP server uses.
//!
//! An LSP server is built for a human in an editor (cursor, hover, push
//! diagnostics over a stateful document session). An autonomous agent or a CI
//! job wants the opposite: feed it a file, get compiler-style diagnostics and an
//! exit code. This module is that front end — it reuses `poseidon-syntax`'s
//! lexer + `check.rs` type checker verbatim, so the verdicts are identical to
//! what the editor shows.

use poseidon_catalog as cat;
use poseidon_syntax::common::Severity;
use poseidon_syntax::{check, config, sqf};

/// Which resource language a source buffer is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Sqf,
    Sqs,
    /// ParamFile config (`.sqm` / `.ext` / `.cfg` / `config.cpp`).
    Config,
}

impl Lang {
    /// Infer the language from a path, using the SAME extension rules as the
    /// LSP server (`lang_of` in `poseidon-lsp`). A generic `.cpp` that is not
    /// `config.cpp` is intentionally NOT treated as config.
    pub fn from_path(path: &str) -> Option<Lang> {
        let p = path.to_ascii_lowercase();
        if p.ends_with(".sqf") {
            Some(Lang::Sqf)
        } else if p.ends_with(".sqs") {
            Some(Lang::Sqs)
        } else if p.ends_with(".sqm")
            || p.ends_with(".ext")
            || p.ends_with(".cfg")
            || p.ends_with("config.cpp")
        {
            Some(Lang::Config)
        } else {
            None
        }
    }

    /// Parse an explicit `--lang` value.
    pub fn from_str(s: &str) -> Option<Lang> {
        match s.to_ascii_lowercase().as_str() {
            "sqf" => Some(Lang::Sqf),
            "sqs" => Some(Lang::Sqs),
            "config" | "paramfile" | "cfg" | "sqm" | "ext" => Some(Lang::Config),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Sqf => "sqf",
            Lang::Sqs => "sqs",
            Lang::Config => "config",
        }
    }
}

/// A 1-based line/column plus the raw byte offset it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
    pub offset: usize,
}

/// One diagnostic, resolved to line/column positions.
#[derive(Clone, Debug)]
pub struct Finding {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub start: LineCol,
    pub end: LineCol,
}

impl Finding {
    pub fn severity_str(&self) -> &'static str {
        match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Information => "info",
            Severity::Hint => "hint",
        }
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Maps byte offsets to 1-based line/column. Columns count Unicode scalar values
/// (good enough for a terminal; the LSP server does the UTF-16 mapping instead).
struct LineMap {
    line_starts: Vec<usize>,
}

impl LineMap {
    fn new(text: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineMap { line_starts }
    }

    fn locate(&self, text: &str, offset: usize) -> LineCol {
        let offset = offset.min(text.len());
        // last line_start <= offset
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let line_start = self.line_starts[line_idx];
        let col = text[line_start..offset].chars().count() as u32 + 1;
        LineCol {
            line: line_idx as u32 + 1,
            col,
            offset,
        }
    }
}

/// Run lexical + semantic analysis on a buffer, exactly as the server's
/// `refresh()` does: lexer diagnostics for every language, plus the type
/// checker for SQF/SQS (config has no semantic pass yet).
pub fn analyze(text: &str, lang: Lang) -> Vec<Finding> {
    let mut diags = match lang {
        Lang::Sqf => sqf::lex(text, sqf::Dialect::Sqf).1,
        Lang::Sqs => sqf::lex(text, sqf::Dialect::Sqs).1,
        Lang::Config => config::lex(text).1,
    };
    match lang {
        Lang::Sqf => diags.extend(check::check(text, sqf::Dialect::Sqf)),
        Lang::Sqs => diags.extend(check::check(text, sqf::Dialect::Sqs)),
        Lang::Config => {}
    }
    let map = LineMap::new(text);
    let mut findings: Vec<Finding> = diags
        .into_iter()
        .map(|d| Finding {
            severity: d.severity,
            code: d.code,
            message: d.message,
            start: map.locate(text, d.span.start),
            end: map.locate(text, d.span.end),
        })
        .collect();
    findings.sort_by_key(|f| (f.start.offset, f.end.offset));
    findings
}

/// Whether a set of findings should fail the run (drive a non-zero exit code).
/// The checker reports only lexical breakage as `error`; everything semantic
/// (unknown command, type mismatch, missing operand) is a `warning`, so an
/// agent that wants those to fail its verify loop passes `strict = true`.
pub fn fails(findings: &[Finding], strict: bool) -> bool {
    findings
        .iter()
        .any(|f| f.is_error() || (strict && f.severity == Severity::Warning))
}

/// Render the findings for one input in compiler style:
/// `path:line:col: severity[code]: message`.
pub fn render_human(label: &str, findings: &[Finding]) -> String {
    let mut s = String::new();
    for f in findings {
        s.push_str(&format!(
            "{label}:{}:{}: {}[{}]: {}\n",
            f.start.line,
            f.start.col,
            f.severity_str(),
            f.code,
            f.message
        ));
    }
    s
}

/// Build the JSON object for one analyzed input.
pub fn json_for(label: &str, lang: Lang, findings: &[Finding]) -> serde_json::Value {
    let errors = findings.iter().filter(|f| f.is_error()).count();
    let warnings = findings.len() - errors;
    serde_json::json!({
        "file": label,
        "lang": lang.name(),
        "ok": errors == 0,
        "errors": errors,
        "warnings": warnings,
        "diagnostics": findings.iter().map(|f| serde_json::json!({
            "severity": f.severity_str(),
            "code": f.code,
            "message": f.message,
            "start": { "line": f.start.line, "col": f.start.col, "offset": f.start.offset },
            "end": { "line": f.end.line, "col": f.end.col, "offset": f.end.offset },
        })).collect::<Vec<_>>(),
    })
}

/// Map an engine `GameType` name to the `validate_ref` JSON type vocabulary
/// (`Number`/`Bool`/`String`/`Array`/`Object`/`Side`/`Group`/`Nothing`/`Any`),
/// the names the engine's own `EvaluatorHost::ValidateRef` understands. Composite
/// types collapse to their first component.
fn ref_type(game: &str) -> String {
    let first = game.split('|').next().unwrap_or(game).trim();
    let base = first.strip_prefix("Game").unwrap_or(first);
    let base = base.split("Or").next().unwrap_or(base); // ObjectOrArray -> Object
    match base.to_ascii_lowercase().as_str() {
        "scalar" => "Number".to_string(),
        "bool" => "Bool".to_string(),
        "string" => "String".to_string(),
        "array" | "vector" => "Array".to_string(),
        "nothing" => "Nothing".to_string(),
        "object" => "Object".to_string(),
        "side" => "Side".to_string(),
        "group" => "Group".to_string(),
        "void" | "any" => "Any".to_string(),
        other => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => "Any".to_string(),
            }
        }
    }
}

/// Export the doc corpus to a directory of `validate_ref`-schema JSON files
/// (`name, arity, group, description, syntax, return_type, arg_types[],
/// examples[]`), one per documented command. This is the exact format the
/// engine's own `PoseidonEvaluator --validate-ref <dir>` consumes — so the same
/// examples our checker gates can be validated against the live C++ engine.
/// Returns the number of files written.
pub fn export_ref(dir: &std::path::Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let mut n = 0;
    for name in cat::documented_names() {
        let doc = cat::docs(name).expect("listed name resolves");
        let overloads = cat::lookup(name);
        let prim = overloads.first();
        let canonical = prim.map(|c| c.name.clone()).unwrap_or_else(|| name.to_string());
        let arity = prim.map(|c| c.kind.clone()).unwrap_or_default();
        let return_type = prim.map(|c| ref_type(&c.ret)).unwrap_or_else(|| "Any".into());
        let arg_types: Vec<String> =
            prim.map(|c| c.args.iter().map(|a| ref_type(a)).collect()).unwrap_or_default();
        let syntax = prim.map(|c| c.signature()).unwrap_or_default();
        let v = serde_json::json!({
            "name": canonical,
            "arity": arity,
            "group": doc.group,
            "description": doc.description,
            "syntax": syntax,
            "return_type": return_type,
            "arg_types": arg_types,
            "examples": doc.examples,
        });
        std::fs::write(dir.join(format!("{name}.json")), serde_json::to_string_pretty(&v)?)?;
        n += 1;
    }
    Ok(n)
}

/// Render `sig <command>`: the engine-derived signatures plus any authored docs.
/// Returns `None` if the name is not a registered command in this dialect.
pub fn render_sig(name: &str) -> Option<String> {
    let overloads = cat::lookup(name);
    if overloads.is_empty() {
        return None;
    }
    // Deduplicate identical signature strings while preserving order.
    let mut seen = std::collections::HashSet::new();
    let sigs: Vec<String> = overloads
        .iter()
        .map(|c| c.signature())
        .filter(|s| seen.insert(s.clone()))
        .collect();

    let canonical = &overloads[0].name;
    let origin = match overloads[0].origin.as_str() {
        "" => "engine",
        o => o,
    };
    let mut s = String::new();
    s.push_str(canonical);
    if let Some(doc) = cat::docs(name) {
        if !doc.group.is_empty() {
            s.push_str(&format!("  ({})", doc.group));
        }
    }
    s.push('\n');
    for sig in &sigs {
        s.push_str(&format!("  {sig}\n"));
    }
    s.push_str(&format!(
        "  classic Operation Flashpoint / Cold War Assault {origin} command"
    ));
    if sigs.len() > 1 {
        s.push_str(&format!(" \u{00b7} {} overloads", sigs.len()));
    }
    s.push('\n');
    if let Some(doc) = cat::docs(name) {
        s.push('\n');
        s.push_str(&format!("  {}\n", doc.description));
        if !doc.examples.is_empty() {
            s.push_str("\n  examples:\n");
            for ex in &doc.examples {
                s.push_str(&format!("    {ex}\n"));
            }
        }
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_inference_matches_server_rules() {
        assert_eq!(Lang::from_path("mission/init.sqf"), Some(Lang::Sqf));
        assert_eq!(Lang::from_path("a.SQS"), Some(Lang::Sqs));
        assert_eq!(Lang::from_path("mission.sqm"), Some(Lang::Config));
        assert_eq!(Lang::from_path("description.ext"), Some(Lang::Config));
        assert_eq!(Lang::from_path("config.cpp"), Some(Lang::Config));
        // a generic .cpp is engine source, NOT a config — must not be inferred
        assert_eq!(Lang::from_path("express.cpp"), None);
        assert_eq!(Lang::from_path("notes.txt"), None);
    }

    #[test]
    fn explicit_lang_parsing() {
        assert_eq!(Lang::from_str("SQF"), Some(Lang::Sqf));
        assert_eq!(Lang::from_str("config"), Some(Lang::Config));
        assert_eq!(Lang::from_str("paramfile"), Some(Lang::Config));
        assert_eq!(Lang::from_str("nonsense"), None);
    }

    #[test]
    fn clean_sqf_has_no_findings() {
        // every operand is correctly typed → the checker is silent
        assert!(analyze("player setDamage 0", Lang::Sqf).is_empty());
    }

    #[test]
    fn type_mismatch_is_reported_as_warning_with_position() {
        // `hint` wants a STRING; `5` is a SCALAR. The checker reports semantic
        // issues as warnings (never a false red), so this is a warning, not an
        // error — which is exactly why `--strict` exists.
        let f = analyze("hint 5", Lang::Sqf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(!f[0].is_error());
        assert_eq!(f[0].severity, Severity::Warning);
        assert_eq!(f[0].code, "EvalType");
        assert_eq!(f[0].start.line, 1);
        // not a failure by default, but a failure under --strict
        assert!(!fails(&f, false));
        assert!(fails(&f, true));
    }

    #[test]
    fn lexical_breakage_is_an_error_and_always_fails() {
        // an unterminated string is structural → error, fails even non-strict
        let f = analyze("hint \"oops", Lang::Sqf);
        assert!(f.iter().any(|d| d.is_error()));
        assert!(fails(&f, false));
        assert!(fails(&f, true));
    }

    #[test]
    fn line_and_column_are_one_based_and_track_newlines() {
        // unterminated string on line 2, starting after "x = " (col 5)
        let src = "player setDamage 0;\nx = \"oops";
        let f = analyze(src, Lang::Sqf);
        assert!(!f.is_empty());
        let s = &f[0];
        assert_eq!(s.start.line, 2);
        assert_eq!(s.start.col, 5);
    }

    #[test]
    fn human_render_is_compiler_style() {
        let f = analyze("hint 5", Lang::Sqf);
        let out = render_human("init.sqf", &f);
        assert!(out.starts_with("init.sqf:1:"), "{out}");
        assert!(out.contains("warning[EvalType]"), "{out}");
    }

    #[test]
    fn json_marks_errors_and_ok() {
        // a structural error → ok:false, errors:1
        let bad = analyze("hint \"oops", Lang::Sqf);
        let v = json_for("init.sqf", Lang::Sqf, &bad);
        assert_eq!(v["ok"], serde_json::json!(false));
        assert_eq!(v["errors"], serde_json::json!(1));
        assert_eq!(v["diagnostics"][0]["start"]["line"], serde_json::json!(1));

        // a semantic warning → ok:true (no errors) but warnings:1
        let warn = analyze("hint 5", Lang::Sqf);
        let vw = json_for("init.sqf", Lang::Sqf, &warn);
        assert_eq!(vw["ok"], serde_json::json!(true));
        assert_eq!(vw["warnings"], serde_json::json!(1));

        let good = analyze("player setDamage 0", Lang::Sqf);
        assert_eq!(json_for("init.sqf", Lang::Sqf, &good)["ok"], serde_json::json!(true));
    }

    #[test]
    fn sig_renders_signature_and_docs() {
        let s = render_sig("setdamage").expect("setDamage exists");
        assert!(s.contains("setDamage"));
        assert!(s.contains("OBJECT"));
        assert!(s.contains("SCALAR"));
        // documented → description + examples present
        assert!(s.contains("destroyed"));
        assert!(s.contains("examples:"));
        assert!(render_sig("definitely_not_a_command").is_none());
    }

    #[test]
    fn export_ref_writes_validate_ref_schema() {
        let dir = std::env::temp_dir().join("poseidon_export_ref_test");
        let _ = std::fs::remove_dir_all(&dir);
        let n = export_ref(&dir).expect("export");
        assert_eq!(n, cat::doc_count(), "one file per documented command");

        let raw = std::fs::read_to_string(dir.join("setdamage.json")).expect("setdamage.json");
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["name"], serde_json::json!("setDamage"));
        assert_eq!(v["arity"], serde_json::json!("binary"));
        // GameObject × GameScalar → GameNothing, mapped to the validate_ref vocab
        assert_eq!(v["arg_types"], serde_json::json!(["Object", "Number"]));
        assert_eq!(v["return_type"], serde_json::json!("Nothing"));
        assert!(v["examples"].as_array().unwrap().iter().any(|e| e.as_str().unwrap().contains("setDamage")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ref_type_maps_engine_types() {
        assert_eq!(ref_type("GameScalar"), "Number");
        assert_eq!(ref_type("GameObjectOrArray"), "Object"); // composite -> first
        assert_eq!(ref_type("GameNothing|GameScalar"), "Nothing");
        assert_eq!(ref_type("GameVoid"), "Any");
    }

    #[test]
    fn sig_works_without_docs() {
        // a real command we did NOT author docs for still renders its signature
        assert!(cat::docs("acos").is_none(), "test assumes acos is undocumented");
        let s = render_sig("acos").expect("acos exists");
        assert!(s.contains("acos"));
    }
}
