//! `poseidon-check` — validate classic OFP/CWA scripts and configs from the
//! command line, and look up command signatures + docs. The one-shot interface
//! an autonomous agent or a CI job wants (the LSP server is for editors).
//!
//! Usage:
//!   poseidon-check <PATH>...            validate files/dirs (lang inferred from ext)
//!   poseidon-check --stdin --lang sqf   validate stdin
//!   poseidon-check --json <PATH>...     machine-readable diagnostics
//!   poseidon-check sig <COMMAND>...     show signature(s) + docs for a command
//!   poseidon-check export-ref <DIR>     write the doc corpus as validate_ref JSON
//!
//! Exit codes: 0 = no errors, 1 = error-severity diagnostics found (or an
//! unknown command for `sig`), 2 = usage / I/O error.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use poseidon_cli::{analyze, export_ref, json_for, render_human, render_sig, Lang};

const USAGE: &str = "\
poseidon-check — classic Operation Flashpoint / Cold War Assault linter

USAGE:
    poseidon-check [OPTIONS] <PATH>...
    poseidon-check [OPTIONS] --stdin --lang <LANG>
    poseidon-check sig <COMMAND>...
    poseidon-check export-ref <DIR>

A <PATH> may be a file or a directory; directories are walked recursively for
recognized resources (.sqf .sqs .sqm .ext .cfg config.cpp) — point it at a whole
mission folder.

OPTIONS:
    --json            emit machine-readable JSON diagnostics
    --lang <LANG>     force language: sqf | sqs | config (else inferred from extension)
    --stdin           read source from standard input
    --strict          treat warnings (unknown command, type mismatch, ...) as failures
    --quiet           print only diagnostics, not the per-file 'ok' lines
    -h, --help        show this help
    -V, --version     show version

SUBCOMMANDS:
    sig <COMMAND>...  print the engine-derived signature(s) and docs for a command
    export-ref <DIR>  write the doc corpus as engine `validate_ref` JSON files,
                      ready for `PoseidonEvaluator --validate-ref <DIR>`

EXIT: 0 clean · 1 errors found · 2 bad usage / I/O";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("poseidon-check: {msg}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> std::result::Result<ExitCode, String> {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("poseidon-check {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    if args[0] == "sig" {
        return run_sig(&args[1..]);
    }
    if args[0] == "export-ref" {
        return run_export_ref(&args[1..]);
    }

    let mut json = false;
    let mut quiet = false;
    let mut stdin = false;
    let mut strict = false;
    let mut forced: Option<Lang> = None;
    let mut files: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--json" => json = true,
            "--quiet" => quiet = true,
            "--stdin" => stdin = true,
            "--strict" => strict = true,
            "--lang" => {
                i += 1;
                let v = args.get(i).ok_or("--lang needs a value (sqf|sqs|config)")?;
                forced = Some(Lang::from_str(v).ok_or_else(|| format!("unknown --lang '{v}'"))?);
            }
            s if s.starts_with("--") => return Err(format!("unknown option '{s}'")),
            s => files.push(s.to_string()),
        }
        i += 1;
    }

    // Collect (label, lang, text) inputs.
    let mut inputs: Vec<(String, Lang, String)> = Vec::new();
    if stdin {
        let lang = forced.ok_or("--stdin requires --lang <sqf|sqs|config>")?;
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| format!("reading stdin: {e}"))?;
        inputs.push(("<stdin>".to_string(), lang, text));
    }
    for f in &files {
        let p = Path::new(f);
        if p.is_dir() {
            // Walk a mission/addon directory: lint every recognized resource,
            // inferring language per file (forced --lang is ignored for a walk).
            let mut found = Vec::new();
            walk_dir(p, &mut found);
            if found.is_empty() {
                eprintln!("poseidon-check: no recognized resources under '{f}'");
            }
            for fp in found {
                let label = fp.to_string_lossy().replace('\\', "/");
                let lang = Lang::from_path(&label).expect("walk yields only recognized files");
                let text =
                    std::fs::read_to_string(&fp).map_err(|e| format!("reading '{label}': {e}"))?;
                inputs.push((label, lang, text));
            }
        } else {
            let lang = match forced {
                Some(l) => l,
                None => Lang::from_path(f).ok_or_else(|| {
                    format!("cannot infer language of '{f}' — pass --lang <sqf|sqs|config>")
                })?,
            };
            let text = std::fs::read_to_string(f).map_err(|e| format!("reading '{f}': {e}"))?;
            inputs.push((f.clone(), lang, text));
        }
    }
    if inputs.is_empty() {
        return Err("no input files (or --stdin) given".to_string());
    }

    let mut any_failure = false;
    let mut json_results = Vec::new();
    for (label, lang, text) in &inputs {
        let findings = analyze(text, *lang);
        if poseidon_cli::fails(&findings, strict) {
            any_failure = true;
        }
        if json {
            json_results.push(json_for(label, *lang, &findings));
        } else {
            print!("{}", render_human(label, &findings));
            if !quiet {
                let errs = findings.iter().filter(|f| f.is_error()).count();
                let warns = findings.len() - errs;
                if errs == 0 && warns == 0 {
                    println!("{label}: ok");
                } else {
                    println!("{label}: {errs} error(s), {warns} warning(s)");
                }
            }
        }
    }
    if json {
        println!("{}", serde_json::Value::Array(json_results));
    }

    Ok(if any_failure { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

/// Recursively collect recognized resource files under `dir` (sorted, so output
/// order is deterministic).
fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk_dir(&p, out);
        } else if Lang::from_path(&p.to_string_lossy()).is_some() {
            out.push(p);
        }
    }
}

fn run_export_ref(args: &[String]) -> std::result::Result<ExitCode, String> {
    let dir = args.first().ok_or("export-ref needs a target directory")?;
    let n = export_ref(Path::new(dir)).map_err(|e| format!("writing to '{dir}': {e}"))?;
    println!("wrote {n} validate_ref file(s) to {dir}");
    println!("validate against the engine: PoseidonEvaluator --validate-ref {dir}");
    Ok(ExitCode::SUCCESS)
}

fn run_sig(names: &[String]) -> std::result::Result<ExitCode, String> {
    if names.is_empty() {
        return Err("sig needs at least one command name".to_string());
    }
    let mut all_known = true;
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            println!();
        }
        match render_sig(name) {
            Some(s) => print!("{s}"),
            None => {
                println!("{name}: not a command in this engine (classic OFP/CWA dialect)");
                all_known = false;
            }
        }
    }
    Ok(if all_known { ExitCode::SUCCESS } else { ExitCode::from(1) })
}
