//! Differential testing: the production checker (`check.rs`, recursive
//! precedence-climbing) vs the reference oracle (`oracle.rs`, a faithful
//! transliteration of the engine's iterative `Vyhod`/`VyhCast` stack machine).
//!
//! 1. The oracle is first validated against the engine maintainers' OWN recorded
//!    verdicts (`engine_vectors.json`) so it is grounded, not just self-consistent.
//! 2. Thousands of randomly generated expressions are then run through BOTH; any
//!    accept/reject divergence is a bug in one of them.

use poseidon_syntax::{check, oracle, sqf};

fn checker_accepts(expr: &str) -> bool {
    sqf::lex(expr, sqf::Dialect::Sqf).1.is_empty() && check::check(expr, sqf::Dialect::Sqf).is_empty()
}

// ---- 1. ground the oracle against the engine's own verdicts ----------------

#[derive(serde::Deserialize)]
struct Vector {
    expr: String,
    expect: String,
    src: String,
}

#[test]
fn oracle_matches_engine_recorded_verdicts() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/engine_vectors.json");
    let vectors: Vec<Vector> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut bad = Vec::new();
    for v in &vectors {
        let ok = oracle::accepts(&v.expr);
        let agree = (v.expect == "valid" && ok) || (v.expect == "error" && !ok);
        if !agree {
            bad.push(format!("{} (engine: {}, oracle: {}) [{}]", v.expr, v.expect, ok, v.src));
        }
    }
    assert!(bad.is_empty(), "oracle disagrees with engine verdicts:\n  {}", bad.join("\n  "));
}

// ---- 2. differential fuzzing: checker vs oracle ----------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

const ATOMS: &[&str] = &["1", "2", "3.5", "0", "\"a\"", "\"b\"", "_x", "_y", "foo", "player", "time", "true", "false", "west"];
const UNARY: &[&str] = &["getpos", "alive", "count", "units", "group", "-", "!", "+", "speed", "leader"];
const BINARY: &[&str] = &["+", "-", "*", "/", "==", "!=", "<", ">", "&&", "||", "setpos", "setdamage", "select", "distance", "count", "in"];

// NB: no `{ … }` code blocks. The engine's parser treats a brace block as one
// opaque string and only checks it when it runs; our checker deliberately
// recurses into code blocks to flag errors statically (an LSP feature). That
// designed difference is covered separately — here we differentially test the
// shared expression logic (precedence, overloads, arrays, parens).
fn gen(rng: &mut Rng, depth: u32, out: &mut String) {
    if depth == 0 || out.len() > 400 {
        out.push_str(rng.pick(ATOMS));
        return;
    }
    match rng.below(5) {
        0 => out.push_str(rng.pick(ATOMS)),
        1 => {
            // Operand is parenthesised so it always begins with a value. A bare
            // `unary_command + binop` (e.g. `getpos + 2` = `getpos(+2)`) is the
            // one construct where checker and oracle intentionally differ: the
            // checker reads a command with no value operand as a *variable*
            // shadowing the command (to avoid false positives on real code where
            // a var shadows a command name), while the faithful engine oracle
            // applies it as a command. That deviation is by design, so we don't
            // fuzz it here.
            out.push_str(rng.pick(UNARY));
            out.push_str(" (");
            gen(rng, depth - 1, out);
            out.push(')');
        }
        2 => {
            gen(rng, depth - 1, out);
            out.push(' ');
            out.push_str(rng.pick(BINARY));
            out.push(' ');
            gen(rng, depth - 1, out);
        }
        3 => {
            out.push('(');
            gen(rng, depth - 1, out);
            out.push(')');
        }
        _ => {
            out.push('[');
            gen(rng, depth - 1, out);
            out.push(',');
            gen(rng, depth - 1, out);
            out.push(']');
        }
    }
}

#[test]
fn checker_agrees_with_oracle_on_fuzzed_expressions() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut divergences = Vec::new();
    let mut total = 0;
    for _ in 0..8000 {
        let mut e = String::new();
        gen(&mut rng, 4, &mut e);
        if e.trim().is_empty() {
            continue;
        }
        total += 1;
        let a = checker_accepts(&e);
        let b = oracle::accepts(&e);
        if a != b {
            if divergences.len() < 25 {
                divergences.push(format!("checker={a} oracle={b}: {e}"));
            }
        }
    }
    let n = divergences.len();
    eprintln!("fuzzed {total} expressions, {n} divergences");
    assert!(
        divergences.is_empty(),
        "{n} checker/oracle divergences (first {}):\n  {}",
        divergences.len(),
        divergences.join("\n  ")
    );
}
