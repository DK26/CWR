//! Differential test against the engine maintainers' own verdicts.
//!
//! `engine_vectors.json` is harvested from the engine's C++ Evaluator tests
//! (`tests/unit/**/Evaluator/*.cpp`) and the `validate_ref` examples — every
//! expression the engine team `Evaluate`d, tagged with whether they asserted it
//! valid or erroring. We run each through OUR checker and assert agreement.
//!
//! Scope note: our checker is a *static* syntax+type checker, not a full
//! evaluator, so engine errors that are purely *runtime* (division by zero,
//! reading an undefined variable, array-index out of range) are out of scope —
//! the few such vectors are listed in `RUNTIME_ONLY` with a reason.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Vector {
    expr: String,
    expect: String, // "valid" | "error"
    src: String,
}

/// Engine vectors whose error is a *runtime* condition our static checker is
/// not designed to catch (kept explicit so coverage gaps are visible).
const RUNTIME_ONLY: &[&str] = &[
    // (populated during triage; each entry documents why it's out of scope)
];

fn vectors_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/engine_vectors.json")
}

fn has_diagnostic(expr: &str) -> bool {
    use poseidon_syntax::{check, sqf};
    !sqf::lex(expr, sqf::Dialect::Sqf).1.is_empty()
        || !check::check(expr, sqf::Dialect::Sqf).is_empty()
}

#[test]
fn agrees_with_engine_test_verdicts() {
    let raw = std::fs::read_to_string(vectors_path()).expect("engine_vectors.json");
    let vectors: Vec<Vector> = serde_json::from_str(&raw).unwrap();
    assert!(vectors.len() > 50, "expected a sizable oracle corpus, got {}", vectors.len());

    let mut false_positives = Vec::new(); // engine: valid, us: flagged
    let mut missed = Vec::new(); // engine: error, us: clean
    let mut valid_checked = 0;

    for v in &vectors {
        if RUNTIME_ONLY.contains(&v.expr.as_str()) {
            continue;
        }
        let flagged = has_diagnostic(&v.expr);
        match v.expect.as_str() {
            "valid" => {
                valid_checked += 1;
                if flagged {
                    false_positives.push(format!("{}  [{}]", v.expr, v.src));
                }
            }
            "error" => {
                if !flagged {
                    missed.push(format!("{}  [{}]", v.expr, v.src));
                }
            }
            _ => {}
        }
    }

    eprintln!(
        "engine oracle: {} valid checked, {} false positives, {} missed errors",
        valid_checked,
        false_positives.len(),
        missed.len()
    );
    for f in &false_positives {
        eprintln!("  FP (engine says valid): {f}");
    }
    for m in &missed {
        eprintln!("  MISS (engine says error): {m}");
    }

    assert!(
        false_positives.is_empty(),
        "{} expressions the engine accepts were flagged by our checker",
        false_positives.len()
    );
    assert!(missed.is_empty(), "{} engine errors not caught", missed.len());
}
