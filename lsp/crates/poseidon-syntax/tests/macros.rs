//! Preprocessor / macro robustness.
//!
//! Real classic-OFP configs and many scripts are macro-heavy (`#include`,
//! `#define`, CBA-style function macros). The engine runs its preprocessor
//! BEFORE the evaluator, so directive lines are never parsed as code. Our lexer
//! must therefore lex a directive line as a single `Macro` token, the checker
//! must skip it, and a macro *invocation* defined in the same file must be
//! treated as an opaque value — while a genuinely unknown identifier is still
//! flagged. These tests pin all of that.

use poseidon_syntax::{check, sqf, Tok};

fn check_sqf(src: &str) -> Vec<String> {
    let mut out: Vec<String> = sqf::lex(src, sqf::Dialect::Sqf)
        .1
        .iter()
        .map(|d| format!("lex[{}]", d.code))
        .collect();
    out.extend(check::check(src, sqf::Dialect::Sqf).iter().map(|d| format!("check[{}]: {}", d.code, d.message)));
    out
}

#[test]
fn define_and_include_lines_lex_as_single_macro_tokens() {
    let src = "#define DEBUG 1\n#include \"macros.hpp\"\nplayer setDamage 0;";
    let (toks, diags) = sqf::lex(src, sqf::Dialect::Sqf);
    assert!(diags.is_empty(), "directive lines should not produce lexical errors: {diags:?}");
    let macros: Vec<_> = toks.iter().filter(|t| t.tok == Tok::Macro).collect();
    assert_eq!(macros.len(), 2, "expected one Macro token per directive line");
}

#[test]
fn directive_lines_produce_no_semantic_diagnostics() {
    // Previously `#define HEAL(u) u setDamage 0` flagged `define`, `HEAL`, `u`, …
    let src = "#define HEAL(unit) unit setDamage 0\nplayer setDamage 0;";
    assert!(check_sqf(src).is_empty(), "{:?}", check_sqf(src));
}

#[test]
fn multiline_define_continuation_is_one_macro() {
    let src = "#define LONG(a,b) \\\n    a setDamage b\nplayer setDamage 0;";
    let (toks, _) = sqf::lex(src, sqf::Dialect::Sqf);
    let macros: Vec<_> = toks.iter().filter(|t| t.tok == Tok::Macro).collect();
    assert_eq!(macros.len(), 1, "a `\\`-continued #define is one logical line");
    assert!(check_sqf(src).is_empty(), "{:?}", check_sqf(src));
}

#[test]
fn defined_macro_invocation_is_treated_as_a_value() {
    let src = "#define HEAL(unit) unit setDamage 0\n_x = player;\nHEAL(_x);\nhint \"ok\";";
    assert!(check_sqf(src).is_empty(), "defined macro call should be clean: {:?}", check_sqf(src));
}

#[test]
fn undefined_macro_like_call_is_still_flagged() {
    // No #define for HEAL → a real unknown command / typo, must still be caught.
    let src = "_x = player;\nHEAL(_x);";
    let diags = check::check(src, sqf::Dialect::Sqf);
    assert!(
        diags.iter().any(|d| d.code == "EvalOper" && d.message.contains("HEAL")),
        "undefined HEAL should be flagged: {diags:?}"
    );
}

#[test]
fn object_macro_used_as_bare_value_is_clean() {
    let src = "#define MAX_UNITS 12\n_n = MAX_UNITS;";
    assert!(check_sqf(src).is_empty(), "{:?}", check_sqf(src));
}

#[test]
fn sqs_label_is_not_mistaken_for_a_macro() {
    // In SQS, a line-leading `#name` is a jump label, not a preprocessor directive.
    let src = "#start\nplayer setDamage 0\ngoto \"start\"";
    let (toks, _) = sqf::lex(src, sqf::Dialect::Sqs);
    assert!(toks.iter().any(|t| t.tok == Tok::Label), "#start should be a Label");
    assert!(!toks.iter().any(|t| t.tok == Tok::Macro), "#start must not be a Macro");
}

#[test]
fn sqs_define_is_still_a_macro() {
    // …but a real directive in an .sqs is still a macro line.
    let src = "#define FOO 1\nplayer setDamage 0";
    let (toks, _) = sqf::lex(src, sqf::Dialect::Sqs);
    assert!(toks.iter().any(|t| t.tok == Tok::Macro), "#define is a Macro even in SQS");
}
