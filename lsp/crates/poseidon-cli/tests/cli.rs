//! End-to-end tests for the `poseidon-check` binary: spawn the real executable
//! the way an agent or CI job would, feed it source over stdin / files, and
//! assert on stdout and the process exit code (the contract agents rely on).

use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_poseidon-check");

struct Run {
    code: i32,
    out: String,
}

/// Run the binary with `args`, optionally piping `stdin`.
fn run(args: &[&str], stdin: Option<&str>) -> Run {
    let mut cmd = Command::new(BIN);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().expect("spawn poseidon-check");
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().expect("wait");
    Run {
        code: out.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&out.stdout).into_owned(),
    }
}

#[test]
fn clean_script_exits_zero() {
    let r = run(&["--stdin", "--lang", "sqf"], Some("player setDamage 0"));
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("ok"), "{}", r.out);
}

#[test]
fn lexical_error_exits_one() {
    let r = run(&["--stdin", "--lang", "sqf"], Some("hint \"oops"));
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("error[EvalNum]"), "{}", r.out);
}

#[test]
fn warning_passes_by_default_fails_under_strict() {
    let lenient = run(&["--stdin", "--lang", "sqf"], Some("hint 5"));
    assert_eq!(lenient.code, 0, "{}", lenient.out);
    assert!(lenient.out.contains("warning[EvalType]"), "{}", lenient.out);

    let strict = run(&["--stdin", "--lang", "sqf", "--strict"], Some("hint 5"));
    assert_eq!(strict.code, 1, "{}", strict.out);
}

#[test]
fn json_output_is_valid_and_structured() {
    let r = run(&["--stdin", "--lang", "sqf", "--json"], Some("hint 5"));
    let v: serde_json::Value = serde_json::from_str(&r.out).expect("valid JSON");
    assert!(v.is_array());
    let first = &v[0];
    assert_eq!(first["lang"], serde_json::json!("sqf"));
    assert_eq!(first["warnings"], serde_json::json!(1));
    assert_eq!(first["diagnostics"][0]["code"], serde_json::json!("EvalType"));
}

#[test]
fn sig_prints_signature_and_docs() {
    let r = run(&["sig", "setDamage"], None);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("setDamage"));
    assert!(r.out.contains("SCALAR"));
    assert!(r.out.contains("examples:"));
}

#[test]
fn sig_unknown_command_exits_one() {
    let r = run(&["sig", "setVehicleInit"], None); // an Arma-3 command, not in this engine
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("not a command"), "{}", r.out);
}

#[test]
fn help_exits_zero() {
    let r = run(&["--help"], None);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("poseidon-check"));
    assert!(r.out.contains("sig <COMMAND>"));
}

#[test]
fn unknown_extension_without_lang_is_usage_error() {
    // language can't be inferred from .txt and no --lang given → exit 2
    let dir = std::env::temp_dir();
    let path = dir.join("poseidon_cli_test_notes.txt");
    std::fs::write(&path, "player setDamage 0").unwrap();
    let r = run(&[path.to_str().unwrap()], None);
    let _ = std::fs::remove_file(&path);
    assert_eq!(r.code, 2, "{}", r.out);
}

#[test]
fn lints_a_real_mission_directory() {
    // Point the CLI at an actual classic-OFP mission folder shipped in the repo
    // (init.sqs + description.ext + mission.sqm) — exercises directory walking,
    // per-file language inference, and confirms real engine fixtures are clean.
    let mission = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/mission_linter/valid-mission");
    if !mission.exists() {
        eprintln!("skip: mission fixture not present at {}", mission.display());
        return;
    }
    let r = run(&[mission.to_str().unwrap()], None);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("init.sqs"), "should have walked the mission: {}", r.out);
    assert!(r.out.contains("mission.sqm"), "{}", r.out);
}

#[test]
fn export_ref_subcommand_writes_engine_schema() {
    let dir = std::env::temp_dir().join("poseidon_cli_export_ref");
    let _ = std::fs::remove_dir_all(&dir);
    let r = run(&["export-ref", dir.to_str().unwrap()], None);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("validate-ref"), "{}", r.out);
    let raw = std::fs::read_to_string(dir.join("hint.json")).expect("hint.json written");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["name"], serde_json::json!("hint"));
    assert_eq!(v["arg_types"], serde_json::json!(["String"]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validates_a_real_file_with_inferred_language() {
    let dir = std::env::temp_dir();
    let path = dir.join("poseidon_cli_test_init.sqf");
    std::fs::write(&path, "player setDamage 0;\nhint \"hi\"").unwrap();
    let r = run(&[path.to_str().unwrap()], None);
    let _ = std::fs::remove_file(&path);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("ok"), "{}", r.out);
}
