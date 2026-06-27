//! Source-grounded completeness check.
//!
//! Reads the engine source directly and asserts that **every** command
//! registration site (`GameNular` / `GameFunction` / `GameOperator` and the
//! `TABLE_COMMAND` macros, anywhere under `engine/`, excluding test-only `*Test*`
//! / `*Tri*` files) is present in our catalog. This scans by directory walk
//! rather than a curated file list, so a registration in a file we didn't
//! anticipate (exactly the `EvalState.cpp` gap we hit) fails the test instead of
//! silently missing commands.
//!
//! Skipped (not failed) when the engine source isn't present, so the crate can
//! also be built standalone.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // crate manifest = <repo>/lsp/crates/poseidon-catalog
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn collect_cpp(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_cpp(&p, out);
        } else if p.extension().map_or(false, |x| x == "cpp") {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.contains("Test") && !name.contains("Tri") {
                out.push(p);
            }
        }
    }
}

#[test]
fn catalog_covers_every_engine_registration() {
    let engine = repo_root().join("engine");
    if !engine.exists() {
        eprintln!("skipping: engine source not present at {}", engine.display());
        return;
    }

    let name_re = regex::Regex::new(r#"Game(?:Nular|Function|Operator)\s*\(\s*[^,]*,\s*"([^"]+)""#).unwrap();
    let tbl_re = regex::Regex::new(r#"TABLE_COMMAND(?:_S)?\(\s*(\w+)"#).unwrap();

    let mut files = Vec::new();
    collect_cpp(&engine, &mut files);
    assert!(files.len() > 5, "expected to find engine .cpp files");

    let mut missing: Vec<String> = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        for line in text.lines() {
            // macro *definitions* build names via `#xxx` / `##` — not real names
            if line.contains("#xxx") || line.contains("##") {
                continue;
            }
            for cap in name_re.captures_iter(line) {
                let name = &cap[1];
                if !poseidon_catalog::is_command(name) {
                    missing.push(format!("{name}  ({})", f.file_name().unwrap().to_string_lossy()));
                }
            }
        }
        for cap in tbl_re.captures_iter(&text) {
            let x = &cap[1];
            for n in [format!("command{x}"), format!("do{x}")] {
                if !poseidon_catalog::is_command(&n) {
                    missing.push(format!("{n}  (TABLE_COMMAND in {})", f.file_name().unwrap().to_string_lossy()));
                }
            }
        }
    }

    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "catalog is missing {} engine-registered command(s):\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}
