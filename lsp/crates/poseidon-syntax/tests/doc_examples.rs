//! Grounding gate for the doc-corpus examples.
//!
//! The corpus prose is authored, but its `examples` are not allowed to be
//! aspirational: every example must lex clean AND pass the engine-faithful
//! type checker (`check.rs`, a port of `express.cpp`'s `_checkOnly` path),
//! exactly as if a user had typed it. This is what keeps the documentation
//! honest about THIS dialect — an example that used an Arma-3 form, the wrong
//! operand type, or a non-existent command would be rejected here.

use poseidon_catalog as cat;
use poseidon_syntax::{check, sqf};

fn problems(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for d in sqf::lex(src, sqf::Dialect::Sqf).1 {
        out.push(format!("lex[{}]: {}", d.code, d.message));
    }
    for d in check::check(src, sqf::Dialect::Sqf) {
        out.push(format!("check[{}]: {}", d.code, d.message));
    }
    out
}

#[test]
fn every_doc_example_typechecks() {
    let mut failures = Vec::new();
    for name in cat::documented_names() {
        let doc = cat::docs(name).expect("listed name resolves");
        for ex in &doc.examples {
            let probs = problems(ex);
            if !probs.is_empty() {
                failures.push(format!("`{ex}` (doc: {name})\n      {}", probs.join("\n      ")));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} doc example(s) do not type-check against the engine-faithful checker:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

#[test]
fn documented_commands_have_examples() {
    // Documentation that shows the command in use is far more useful to a human
    // or an agent than prose alone; require at least one example per entry.
    let mut bare = Vec::new();
    for name in cat::documented_names() {
        if cat::docs(name).unwrap().examples.is_empty() {
            bare.push(name);
        }
    }
    assert!(bare.is_empty(), "documented commands without examples: {bare:?}");
}
