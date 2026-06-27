//! Differential parity against the **live C++ engine evaluator**.
//!
//! The repo ships `apps/tools/Evaluator` — `PoseidonEvaluator`, a standalone CLI
//! over the real `EvaluatorHost` (`--eval <expr>` runs an inline expression and
//! returns the engine's exit code: 0 = accepted, non-zero = error). That binary
//! is the actual C++ oracle. Our `oracle.rs` is its no-toolchain stand-in,
//! grounded against the engine team's *recorded* verdicts. This test closes the
//! loop when a build is available: it runs every vector through the real binary
//! and asserts (1) the live engine agrees with the recorded verdict, and (2) our
//! Rust oracle agrees with the live engine.
//!
//! It is **gated**: if no `PoseidonEvaluator` binary is found it prints how to
//! provide one and returns (a no-op pass), so the suite stays green in an
//! environment with no C++ toolchain (like CI by default).
//!
//! Provide the binary via `POSEIDON_EVALUATOR=/path/to/PoseidonEvaluator`, or put
//! it on `PATH`, or build it (`cmake --build` the `PoseidonEvaluator` target).

use poseidon_syntax::oracle;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
struct Vector {
    expr: String,
    expect: String, // "valid" | "error"
    src: String,
}

/// Vectors whose error is purely *runtime* (division by zero, undefined var):
/// the live evaluator RUNS the expression, so it errors at runtime, but our
/// static oracle accepts them. Excluded from the live diff for the same reason
/// `engine_oracle.rs` excludes them from the static diff. Kept explicit.
const RUNTIME_ONLY: &[&str] = &[];

fn vectors_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/engine_vectors.json")
}

/// Locate a `PoseidonEvaluator` binary: explicit env var, then a few likely
/// build locations relative to the repo root, then bare name on `PATH`.
fn find_evaluator() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("POSEIDON_EVALUATOR") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let exe = if cfg!(windows) { "PoseidonEvaluator.exe" } else { "PoseidonEvaluator" };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let candidates = [
        root.join("build").join(exe),
        root.join("build/apps/tools/Evaluator").join(exe),
        root.join("build/apps/tools/Evaluator/Cli").join(exe),
        root.join("out/build").join(exe),
        root.join("cmake-build-debug/apps/tools/Evaluator").join(exe),
    ];
    for c in candidates {
        if c.exists() {
            return Some(c);
        }
    }
    // Fall back to PATH: probe `--version`/help; if it spawns, accept the name.
    if Command::new(exe).arg("--help").output().is_ok() {
        return Some(PathBuf::from(exe));
    }
    None
}

/// Whether the live engine accepts `expr` (exit code 0 from `--eval`).
fn engine_accepts(bin: &Path, expr: &str) -> bool {
    match Command::new(bin).arg("--eval").arg(expr).output() {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

#[test]
fn oracle_matches_live_engine_when_available() {
    let Some(bin) = find_evaluator() else {
        eprintln!(
            "SKIP evaluator_parity: no PoseidonEvaluator found.\n\
             Build it and/or set POSEIDON_EVALUATOR=/path/to/PoseidonEvaluator to enable \
             differential testing against the real C++ engine."
        );
        return;
    };
    eprintln!("evaluator_parity: using {}", bin.display());

    let raw = std::fs::read_to_string(vectors_path()).expect("engine_vectors.json");
    let vectors: Vec<Vector> = serde_json::from_str(&raw).unwrap();

    let mut verdict_drift = Vec::new(); // live engine disagrees with recorded expect
    let mut oracle_drift = Vec::new(); // our oracle disagrees with live engine
    let mut checked = 0;

    for v in &vectors {
        if RUNTIME_ONLY.contains(&v.expr.as_str()) {
            continue;
        }
        let engine_ok = engine_accepts(&bin, &v.expr);
        let recorded_ok = v.expect == "valid";
        let oracle_ok = oracle::accepts(&v.expr);
        checked += 1;

        if engine_ok != recorded_ok {
            verdict_drift.push(format!(
                "{}  (recorded: {}, live engine: {}) [{}]",
                v.expr, v.expect, engine_ok, v.src
            ));
        }
        if oracle_ok != engine_ok {
            oracle_drift.push(format!(
                "{}  (oracle: {}, live engine: {}) [{}]",
                v.expr, oracle_ok, engine_ok, v.src
            ));
        }
    }

    eprintln!(
        "evaluator_parity: {checked} vectors · {} verdict drift · {} oracle drift",
        verdict_drift.len(),
        oracle_drift.len()
    );
    for d in &verdict_drift {
        eprintln!("  VERDICT DRIFT: {d}");
    }
    for d in &oracle_drift {
        eprintln!("  ORACLE  DRIFT: {d}");
    }

    assert!(
        verdict_drift.is_empty(),
        "{} recorded verdict(s) disagree with the live engine — engine_vectors.json is stale",
        verdict_drift.len()
    );
    assert!(
        oracle_drift.is_empty(),
        "{} expression(s) where our Rust oracle disagrees with the live C++ engine",
        oracle_drift.len()
    );
}
