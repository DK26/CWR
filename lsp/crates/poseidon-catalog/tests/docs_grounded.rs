//! Grounding gate for the authored doc corpus (`data/docs.json`).
//!
//! The prose in the corpus is hand-written (the engine ships no doc strings),
//! so it cannot be mechanically derived — but it CAN be mechanically constrained:
//! every documented name must be a real command in THIS engine's catalog, and
//! every entry must actually carry content. (The companion test
//! `poseidon-syntax/tests/doc_examples.rs` additionally type-checks every
//! example.) Together these stop the corpus from drifting into Arma-isms or
//! documenting commands that do not exist in the classic dialect.

use poseidon_catalog as cat;

#[test]
fn every_documented_name_is_a_real_command() {
    let mut unknown = Vec::new();
    for name in cat::documented_names() {
        if !cat::is_command(name) {
            unknown.push(name);
        }
    }
    assert!(
        unknown.is_empty(),
        "docs.json documents commands that do not exist in this engine: {unknown:?}\n\
         (only the classic OFP/CWA command set is valid — no Arma-3 additions)"
    );
}

#[test]
fn every_doc_has_content() {
    let mut empty = Vec::new();
    for name in cat::documented_names() {
        let doc = cat::docs(name).expect("listed name resolves");
        if doc.description.trim().is_empty() {
            empty.push(format!("{name}: empty description"));
        }
        if doc.group.trim().is_empty() {
            empty.push(format!("{name}: empty group"));
        }
    }
    assert!(empty.is_empty(), "doc entries missing content:\n  {}", empty.join("\n  "));
}

#[test]
fn corpus_is_nontrivial() {
    // A sanity floor so an accidental truncation of docs.json is caught.
    assert!(cat::doc_count() >= 40, "doc corpus shrank to {} entries", cat::doc_count());
}
