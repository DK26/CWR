//! Reference oracle: a faithful Rust transliteration of the engine's evaluator
//! parser (`engine/Evaluator/express.cpp` `Vyhod` + `VyhCast`, `_checkOnly`
//! mode). This is the engine's own algorithm — an *iterative priority-stack
//! machine* — deliberately kept structurally distinct from the production
//! checker in `check.rs` (recursive precedence-climbing), so that differential
//! testing between the two surfaces bugs in either. It returns the same kind of
//! accept/reject verdict the engine produces for a single expression.
//!
//! Token preprocessing: a `{ … }` block is one opaque STRING value to the
//! engine (Const, express.cpp:257), so we collapse balanced braces into a single
//! value token, exactly as the engine treats them.

use crate::sqf::{lex, Dialect};
use crate::common::Tok;
use poseidon_catalog as cat;

const Z: i32 = 10; // GamePriority::zavorky
const UNAR: i32 = 8; // GamePriority::unar

#[derive(Clone, Copy, PartialEq)]
enum K {
    Num,
    Str,
    Local,
    Global,
    Cmd,
    Op,
    LParen,
    RParen,
    LBrack,
    RBrack,
    Comma,
    Semi,
}

struct OTok {
    k: K,
    text: String,
}

#[derive(Clone, PartialEq)]
enum Op {
    Bin,         // a reduced value
    Lst,         // a list element
    Name(String),// a pending operator
}

struct Slot {
    ty: u32,
    ub: i8, // 1=unary pending, 2=binary pending, 3=list element, -1=value
    prior: i32,
    op: Op,
}

/// Verdict for one expression: `Ok(result_type)` or `Err(error_kind)`.
pub type Verdict = Result<u32, &'static str>;

/// `true` if the oracle considers `expr` valid (no lexical or evaluation error).
pub fn accepts(expr: &str) -> bool {
    lex(expr, Dialect::Sqf).1.is_empty() && check(expr).iter().all(|v| v.is_ok())
}

/// Run the engine evaluator over `expr` (split into `;`-separated statements).
pub fn check(expr: &str) -> Vec<Verdict> {
    let toks = preprocess(expr);
    let mut m = Machine { t: toks, pos: 0 };
    let mut out = Vec::new();
    loop {
        // skip empty statements
        while matches!(m.peek(), Some(t) if t.k == K::Semi) {
            m.pos += 1;
        }
        if m.peek().is_none() {
            break;
        }
        // assignment `lhs = rhs`: skip a leading `name =` (engine CheckAssignment)
        m.skip_assignment();
        out.push(m.vyhod());
        // resync to the next statement boundary (guarantees progress on error)
        while matches!(m.peek(), Some(t) if t.k != K::Semi) {
            m.pos += 1;
        }
        if matches!(m.peek(), Some(t) if t.k == K::Semi) {
            m.pos += 1;
        }
    }
    if out.is_empty() {
        out.push(Ok(16)); // empty program → NOTHING
    }
    out
}

/// Lex with the shared lexer, drop comments, and collapse `{ … }` into one value.
fn preprocess(expr: &str) -> Vec<OTok> {
    let (raw, _) = lex(expr, Dialect::Sqf);
    let toks: Vec<_> = raw.iter().filter(|t| t.tok != Tok::Comment).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        let text = expr[t.span.start..t.span.end].to_string();
        let k = match t.tok {
            Tok::Number => K::Num,
            Tok::Str => K::Str,
            Tok::LocalVar => K::Local,
            Tok::GlobalVar => K::Global,
            Tok::Command | Tok::Keyword => K::Cmd,
            Tok::Operator => K::Op,
            Tok::Punct => match text.as_str() {
                "(" => K::LParen,
                ")" => K::RParen,
                "[" => K::LBrack,
                "]" => K::RBrack,
                "," => K::Comma,
                ";" => K::Semi,
                "{" => {
                    // collapse balanced braces into one STRING value
                    let mut depth = 0;
                    let mut j = i;
                    while j < toks.len() {
                        match &expr[toks[j].span.start..toks[j].span.end] {
                            "{" => depth += 1,
                            "}" => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    out.push(OTok { k: K::Str, text: "{}".into() });
                    i = j + 1;
                    continue;
                }
                _ => {
                    i += 1;
                    continue;
                }
            },
            _ => {
                i += 1;
                continue;
            }
        };
        out.push(OTok { k, text });
        i += 1;
    }
    out
}

struct Machine {
    t: Vec<OTok>,
    pos: usize,
}

impl Machine {
    fn peek(&self) -> Option<&OTok> {
        self.t.get(self.pos)
    }

    /// Skip a leading `name =` (single `=`, not `==`) — assignment LHS.
    fn skip_assignment(&mut self) {
        // find a top-level single `=` before the next `;`
        let mut depth = 0i32;
        let mut j = self.pos;
        while let Some(t) = self.t.get(j) {
            match t.k {
                K::LParen | K::LBrack => depth += 1,
                K::RParen | K::RBrack => depth -= 1,
                K::Semi if depth == 0 => return,
                K::Op if depth == 0 && t.text == "=" => {
                    self.pos = j + 1;
                    return;
                }
                _ => {}
            }
            j += 1;
        }
    }

    /// Port of `GameState::Vyhod` (one expression up to `;`/end) as a single
    /// loop with an explicit value/operator phase, mirroring the engine's
    /// `break` (→ expect value) / `goto CekejBinar` (→ expect operator) flow.
    fn vyhod(&mut self) -> Verdict {
        // empty expression → NOTHING (engine checks this before the loop)
        match self.peek() {
            None => return Ok(16),
            Some(t) if t.k == K::Semi => return Ok(16),
            _ => {}
        }
        let mut st: Vec<Slot> = Vec::new();
        let mut par = 0i32;
        let mut list = 0i32;
        let mut expect_value = true;

        loop {
            let Some(t) = self.peek() else {
                // end of input
                return if expect_value { Err("EvalNum") } else { finish(&mut st, par, list) };
            };
            if t.k == K::Semi {
                return if expect_value { Err("EvalNum") } else { finish(&mut st, par, list) };
            }

            if expect_value {
                match t.k {
                    K::LParen => {
                        self.pos += 1;
                        par += Z;
                    }
                    K::LBrack => {
                        self.pos += 1;
                        if matches!(self.peek(), Some(t) if t.k == K::RBrack) {
                            self.pos += 1;
                            st.push(Slot { ty: 2, ub: -1, prior: -1, op: Op::Bin });
                            expect_value = false;
                        } else {
                            list += Z;
                        }
                    }
                    K::Cmd | K::Op if cat::arities(&t.text).1 => {
                        // a unary operator/function — its operand comes next
                        let name = t.text.clone();
                        self.pos += 1;
                        st.push(Slot { ty: 0, ub: 1, prior: UNAR + par + list, op: Op::Name(name) });
                    }
                    K::Num | K::Str | K::Local | K::Global | K::Cmd => {
                        let ty = match t.k {
                            K::Num => 1,
                            K::Str => 8,
                            K::Local | K::Global => cat::VALUE_MASK,
                            // a nular command yields its type; any other bare
                            // command name (no operand) is a variable reference
                            _ => {
                                if cat::arities(&t.text).0 {
                                    nular_ret(&t.text)
                                } else {
                                    cat::VALUE_MASK
                                }
                            }
                        };
                        self.pos += 1;
                        st.push(Slot { ty, ub: -1, prior: -1, op: Op::Bin });
                        expect_value = false;
                    }
                    // an operator with no unary form, or a stray closer, where a
                    // value was required
                    _ => return Err("EvalNum"),
                }
            } else {
                // expecting an operator / separator (CekejBinar)
                match t.k {
                    K::Comma => {
                        if list <= 0 {
                            return finish(&mut st, par, list); // top-level comma ends it
                        }
                        self.pos += 1;
                        vyhcast(&mut st, par + list)?;
                        let n = st.len();
                        st[n - 1].op = Op::Lst;
                        st[n - 1].ub = 3;
                        st[n - 1].prior = par + list;
                        expect_value = true;
                    }
                    K::RBrack => {
                        self.pos += 1;
                        vyhcast(&mut st, par + list)?;
                        let n = st.len();
                        st[n - 1].op = Op::Lst;
                        st[n - 1].ub = 3;
                        st[n - 1].prior = par + list;
                        list -= Z;
                        if list < 0 {
                            return Err("EvalOpenB");
                        }
                        let mut spp = st.len();
                        while spp > 0 && st[spp - 1].prior >= list + par + Z {
                            spp -= 1;
                        }
                        st.truncate(spp);
                        st.push(Slot { ty: 2, ub: -1, prior: -1, op: Op::Bin }); // ARRAY value
                        // stay in operator phase
                    }
                    K::RParen => {
                        self.pos += 1;
                        par -= Z;
                        if par < 0 {
                            return Err("EvalOpenB");
                        }
                        // stay in operator phase
                    }
                    K::Cmd | K::Op if cat::arities(&t.text).2 => {
                        let name = t.text.clone();
                        let prior = cat::binary_priority(&name) as i32 + par + list;
                        self.pos += 1;
                        vyhcast(&mut st, prior)?;
                        let n = st.len();
                        st[n - 1].op = Op::Name(name);
                        st[n - 1].ub = 2;
                        st[n - 1].prior = prior;
                        expect_value = true;
                    }
                    // an identifier/operator that is not a binary op, or a value,
                    // where an operator was expected
                    _ => return Err("EvalOper"),
                }
            }
        }
    }
}

/// Port of `GameState::VyhCast(Prio)` — reduce the operator stack.
fn vyhcast(st: &mut Vec<Slot>, prio: i32) -> Result<(), &'static str> {
    while !st.is_empty() {
        let n = st.len();
        if st[n - 1].op != Op::Bin {
            return Ok(()); // top is a list element — stop
        }
        if n <= 1 {
            return Ok(());
        }
        match &st[n - 2].op {
            Op::Bin | Op::Lst => return Ok(()),
            Op::Name(_) => {}
        }
        if prio > st[n - 2].prior {
            return Ok(());
        }
        let Op::Name(opname) = st[n - 2].op.clone() else { unreachable!() };
        let result = if st[n - 2].ub == 2 {
            resolve_binary(&opname, st[n - 2].ty, st[n - 1].ty)?
        } else {
            resolve_unary(&opname, st[n - 1].ty)?
        };
        st[n - 2].ty = result;
        st[n - 2].op = Op::Bin;
        st[n - 2].ub = -1;
        // prior of the reduced value is set by the caller's par+list; -1 is fine
        // for a fully-reduced value at statement level.
        st[n - 2].prior = -1;
        st.truncate(n - 1);
    }
    Ok(())
}

fn resolve_binary(name: &str, left: u32, right: u32) -> Result<u32, &'static str> {
    let mut possible = 0u32;
    let mut had = false;
    for c in cat::lookup(name) {
        if c.kind != "binary" {
            continue;
        }
        had = true;
        if c.arg_bits(0) & left != 0 && c.arg_bits(1) & right != 0 {
            possible |= c.ret_bits();
        }
    }
    if had && possible == 0 {
        return Err("EvalType");
    }
    Ok(if possible == 0 { cat::VALUE_MASK } else { possible })
}

fn resolve_unary(name: &str, operand: u32) -> Result<u32, &'static str> {
    let mut possible = 0u32;
    let mut had = false;
    for c in cat::lookup(name) {
        if c.kind != "unary" {
            continue;
        }
        had = true;
        if c.arg_bits(0) & operand != 0 {
            possible |= c.ret_bits();
        }
    }
    if had && possible == 0 {
        return Err("EvalType");
    }
    Ok(if possible == 0 { cat::VALUE_MASK } else { possible })
}

fn nular_ret(name: &str) -> u32 {
    let mut m = 0u32;
    for c in cat::lookup(name) {
        if c.kind == "nular" {
            m |= c.ret_bits();
        }
    }
    if m == 0 {
        cat::VALUE_MASK
    } else {
        m
    }
}

/// Final `VyhCast(0)` + balance checks at end of a statement.
fn finish(st: &mut Vec<Slot>, par: i32, list: i32) -> Verdict {
    if par != 0 || list != 0 {
        return Err("EvalCloseB");
    }
    vyhcast(st, 0)?;
    match st.last() {
        Some(s) => Ok(s.ty),
        None => Ok(16), // NOTHING
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oracle_accepts_valid() {
        for s in ["2 + 3 * 4", "(2 + 3) * 4", "player setDamage 1", r#""a" + "b""#,
                  "if (alive player) then { hint \"x\" }", "[1,2,3] select 0"] {
            assert!(accepts(s), "oracle rejected valid `{s}`: {:?}", check(s));
        }
    }

    #[test]
    fn oracle_rejects_invalid() {
        for s in ["1 + true", "(1 + 2", "5 5", "player setDamage \"x\"", "5 +", "5 + * 3"] {
            assert!(!accepts(s), "oracle accepted invalid `{s}`");
        }
    }
}
