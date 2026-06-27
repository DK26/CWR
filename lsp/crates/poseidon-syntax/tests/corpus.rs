//! Whole-corpus check against the engine's own shipped script/config fixtures.
//!
//! These files are the engine's test inputs; the valid ones must produce zero
//! diagnostics (the false-positive guard) and the deliberately-broken ones must
//! be flagged (the true-positive guard). Skipped when the repo fixtures aren't
//! present.

use poseidon_syntax::{check, config, sqf};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap()
}

fn collect(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, exts, out);
        } else if let Some(x) = p.extension().and_then(|x| x.to_str()) {
            if exts.contains(&x) {
                out.push(p);
            }
        }
    }
}

/// Diagnostics our LSP would emit for one file (lexical + semantic).
fn diagnose(path: &Path, text: &str) -> usize {
    match path.extension().and_then(|x| x.to_str()) {
        Some("sqf") => sqf::lex(text, sqf::Dialect::Sqf).1.len() + check::check(text, sqf::Dialect::Sqf).len(),
        Some("sqs") => sqf::lex(text, sqf::Dialect::Sqs).1.len() + check::check(text, sqf::Dialect::Sqs).len(),
        _ => config::lex(text).1.len(), // .sqm/.ext/.cfg
    }
}

#[test]
fn engine_fixtures_valid_are_clean_and_errors_are_flagged() {
    let fixtures = repo_root().join("tests/fixtures");
    if !fixtures.exists() {
        eprintln!("skipping: {} not present", fixtures.display());
        return;
    }

    let mut files = Vec::new();
    collect(&fixtures, &["sqf", "sqs", "sqm", "ext", "cfg"], &mut files);
    assert!(files.len() > 20, "expected a sizable fixture corpus, got {}", files.len());

    let mut flagged: Vec<(String, usize)> = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let n = diagnose(f, &text);
        if n > 0 {
            let rel = f.strip_prefix(&fixtures).unwrap_or(f).to_string_lossy().replace('\\', "/");
            flagged.push((rel, n));
        }
    }

    flagged.sort();
    eprintln!("{} / {} fixtures flagged:", flagged.len(), files.len());
    for (f, n) in &flagged {
        eprintln!("  {n} diag  {f}");
    }

    // True-positive guard: the deliberately-broken fixture must be flagged.
    assert!(
        flagged.iter().any(|(f, _)| f.ends_with("error.sqf")),
        "error.sqf (`x = 1 +`) should be flagged"
    );

    // False-positive guard: every flagged file must be a deliberate error case.
    // `studio/script.sqs` is the literal placeholder text `dummy script` (two
    // bare words) — not a valid script; the engine evaluator rejects it too.
    let known_error = |f: &str| {
        f.contains("error") || f.contains("invalid") || f.contains("bad") || f == "studio/script.sqs"
    };
    let unexpected: Vec<&String> = flagged.iter().map(|(f, _)| f).filter(|f| !known_error(f)).collect();
    assert!(
        unexpected.is_empty(),
        "false positives on valid fixtures: {unexpected:?}"
    );
}
