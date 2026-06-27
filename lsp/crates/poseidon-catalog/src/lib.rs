//! Classic Operation Flashpoint / Cold War Assault SQF command catalog.
//!
//! The data in `data/commands.json` is mechanically extracted from the Poseidon
//! engine source (`engine/Evaluator/express.cpp` for the core language and
//! `engine/Poseidon/Game/Commands/GameStateExt*.cpp` for the engine commands).
//! It is the *classic 2001 dialect*, deliberately not modern Arma 3 SQF.

use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;

/// One registered command (a single overload).
#[derive(Debug, Clone, Deserialize)]
pub struct Command {
    /// Canonical (source) spelling, e.g. `setPos`. Lookups are case-insensitive.
    pub name: String,
    /// `"nular"` (0 args), `"unary"` (prefix, 1 arg) or `"binary"` (infix, 2 args).
    pub kind: String,
    /// Return `GameType`, e.g. `GameObject`.
    pub ret: String,
    /// Argument `GameType`s: `[]`, `[arg]`, or `[left, right]`.
    #[serde(default)]
    pub args: Vec<String>,
    /// `"core"` (language built-in) or `"engine"` (game command).
    #[serde(default)]
    pub origin: String,
    /// For binary operators: the `GamePriority` name (e.g. `function`, `soucet`).
    #[serde(default)]
    pub prio: Option<String>,
    /// Source file the row was extracted from.
    #[serde(default)]
    pub src: String,
}

// ---- GameType bitmask (values straight from express.hpp / GameStateExt.hpp) --

/// The "fake" control-flow types (`GameIf`/`GameWhile`/`GameFor`/…).
pub const FAKE_MASK: u32 = 0x1000000 | 0x2000000 | 0x4000000 | 0x8000000 | 0x10000000;
/// `GameVoid` — any *value* type (everything except Nothing and the fake types).
pub const VALUE_MASK: u32 = !(16 | FAKE_MASK);

fn single_bits(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "scalar" => 1,
        "array" => 2,
        "bool" => 4,
        "string" => 8,
        "nothing" => 16,
        "object" => 0x100,
        "vector" => 0x200,
        "trans" => 0x400,
        "orient" => 0x800,
        "side" => 0x1000,
        "group" => 0x2000,
        "file" => 0x4000,
        "if" => 0x1000000,
        "while" => 0x2000000,
        "for" => 0x4000000,
        "forfrom" => 0x8000000,
        "forto" => 0x10000000,
        "void" => VALUE_MASK,
        "any" => !FAKE_MASK,
        "" => 0,
        _ => return None,
    })
}

fn base_bits(name: &str) -> u32 {
    let b = name.strip_prefix("Game").unwrap_or(name);
    if let Some(x) = single_bits(b) {
        return x;
    }
    // OR-composite written as `XxxOrYyy` (e.g. `ObjectOrArray`)
    if let Some(i) = b.find("Or") {
        return base_bits(&b[..i]) | base_bits(&b[i + 2..]);
    }
    // unknown type name → match anything, so we never raise a false type error
    u32::MAX
}

/// Resolve a `GameType` expression (`GameObjectOrArray`, `GameNothing|GameScalar`,
/// `GameVoid`, …) to its bitmask.
pub fn type_bits(expr: &str) -> u32 {
    expr.split('|').map(|p| base_bits(p.trim())).fold(0, |a, b| a | b)
}

/// Ordinal of a `GamePriority` name (low binds looser; from `express.hpp`).
fn prio_ord(p: &str) -> u8 {
    match p.to_ascii_lowercase().as_str() {
        "nula" => 0,
        "logicor" => 1,
        "logicand" => 2,
        "comparison" => 3,
        "function" => 4,
        "functionfirst" => 5,
        "soucet" => 6,
        "soucin" => 7,
        "unar" => 8,
        "mocnina" => 9,
        "zavorky" => 10,
        _ => 4,
    }
}

/// Binding power of a binary operator `name` (lowest priority among its
/// overloads, matching how the engine keys one priority per operator name).
/// Defaults to `function` (4) for names with no recorded priority.
pub fn binary_priority(name: &str) -> u8 {
    lookup(name)
        .iter()
        .filter(|c| c.kind == "binary")
        .filter_map(|c| c.prio.as_deref().map(prio_ord))
        .min()
        .unwrap_or(4)
}

/// Binding power assigned to prefix (unary) application — `unar` (8).
pub const UNARY_BP: u8 = 8;

impl Command {
    /// `true` if the name can appear as an identifier token (starts with a
    /// letter or `_`), as opposed to a symbolic operator like `+` or `&&`.
    pub fn is_identifier(&self) -> bool {
        self.name
            .chars()
            .next()
            .map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
    }

    /// Render a `GameType` token the way the engine's `GetTypeName` does:
    /// `GameObjectOrArray` -> `OBJECT|ARRAY`, `GameNothing` -> `NOTHING`,
    /// and the OR-composites `GameNothing|GameScalar` -> `NOTHING|SCALAR`.
    pub fn type_name(ty: &str) -> String {
        ty.split('|')
            .map(|part| {
                let p = part.trim();
                p.strip_prefix("Game").unwrap_or(p).replace("Or", "|").to_ascii_uppercase()
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    /// A one-line signature suitable for hover / completion detail.
    pub fn signature(&self) -> String {
        let ret = Self::type_name(&self.ret);
        match self.kind.as_str() {
            "nular" => format!("{}: {ret}", self.name),
            "unary" => {
                let a = self.args.first().map(|s| Self::type_name(s)).unwrap_or_default();
                format!("{} <{a}> → {ret}", self.name)
            }
            "binary" => {
                let l = self.args.first().map(|s| Self::type_name(s)).unwrap_or_default();
                let r = self.args.get(1).map(|s| Self::type_name(s)).unwrap_or_default();
                format!("<{l}> {} <{r}> → {ret}", self.name)
            }
            _ => self.name.clone(),
        }
    }

    /// Short kind label, e.g. `binary · engine`.
    pub fn detail(&self) -> String {
        format!("{} · {}", self.kind, if self.origin.is_empty() { "engine" } else { &self.origin })
    }

    /// Return-type bitmask.
    pub fn ret_bits(&self) -> u32 {
        type_bits(&self.ret)
    }

    /// Bitmask of argument `i` (0 = left/only operand, 1 = right operand).
    pub fn arg_bits(&self, i: usize) -> u32 {
        self.args.get(i).map(|a| type_bits(a)).unwrap_or(0)
    }
}

/// Authored, human-readable documentation for one command. The engine ships no
/// in-binary doc strings, so this prose is hand-written (kept honest by the
/// grounding tests); the type signature is always taken from [`Command`], never
/// duplicated here, so the two cannot drift.
#[derive(Debug, Clone, Deserialize)]
pub struct Doc {
    /// Loose category for grouping in completion/listings (`object-state`,
    /// `control-flow`, `math`, …). Free-form.
    #[serde(default)]
    pub group: String,
    /// One- or two-sentence summary of what the command does and why.
    pub description: String,
    /// Runnable examples in the classic dialect. Every one is required to
    /// lex-clean and type-check clean (see `tests/doc_examples.rs`).
    #[serde(default)]
    pub examples: Vec<String>,
}

const RAW: &str = include_str!("../data/commands.json");
const DOCS_RAW: &str = include_str!("../data/docs.json");

/// All command overloads, sorted by name.
pub static COMMANDS: Lazy<Vec<Command>> =
    Lazy::new(|| serde_json::from_str(RAW).expect("embedded commands.json is valid"));

/// Lowercased-name -> indices into [`COMMANDS`] (one name may have several overloads).
static BY_NAME: Lazy<HashMap<String, Vec<usize>>> = Lazy::new(|| {
    let mut m: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, c) in COMMANDS.iter().enumerate() {
        m.entry(c.name.to_ascii_lowercase()).or_default().push(i);
    }
    m
});

/// All overloads of `name` (case-insensitive). Empty if unknown.
pub fn lookup(name: &str) -> Vec<&'static Command> {
    BY_NAME
        .get(&name.to_ascii_lowercase())
        .map(|idxs| idxs.iter().map(|&i| &COMMANDS[i]).collect())
        .unwrap_or_default()
}

/// `true` if `name` is a registered command (case-insensitive).
pub fn is_command(name: &str) -> bool {
    BY_NAME.contains_key(&name.to_ascii_lowercase())
}

/// Lowercased-name -> authored [`Doc`]. Keys beginning with `_` (e.g. the
/// in-file `_comment`) are skipped.
static DOCS: Lazy<HashMap<String, Doc>> = Lazy::new(|| {
    let raw: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(DOCS_RAW).expect("embedded docs.json is valid JSON");
    raw.into_iter()
        .filter(|(k, _)| !k.starts_with('_'))
        .map(|(k, v)| {
            let doc: Doc = serde_json::from_value(v)
                .unwrap_or_else(|e| panic!("docs.json entry `{k}` is malformed: {e}"));
            (k.to_ascii_lowercase(), doc)
        })
        .collect()
});

/// Authored documentation for `name` (case-insensitive), if the corpus covers it.
pub fn docs(name: &str) -> Option<&'static Doc> {
    DOCS.get(&name.to_ascii_lowercase())
}

/// How many commands the doc corpus currently covers.
pub fn doc_count() -> usize {
    DOCS.len()
}

/// Lowercased names of every documented command (for coverage tests / listings).
pub fn documented_names() -> Vec<&'static str> {
    DOCS.keys().map(|s| s.as_str()).collect()
}

/// Core control-flow / literal words that read best highlighted as keywords
/// rather than generic commands. These are still ordinary registered commands
/// in the engine — keyword status is purely cosmetic.
pub const KEYWORDS: &[&str] = &[
    "if", "then", "else", "while", "do", "for", "from", "to", "step", "foreach",
    "exitwith", "private", "and", "or", "not", "true", "false", "nil", "call", "in",
];

/// `true` if `name` is one of [`KEYWORDS`] (case-insensitive).
pub fn is_keyword(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    KEYWORDS.contains(&l.as_str())
}

/// Which arities `name` is registered with: `(nular, unary, binary)`.
/// A name can have several (e.g. `count` is both unary and binary).
pub fn arities(name: &str) -> (bool, bool, bool) {
    let mut a = (false, false, false);
    for c in lookup(name) {
        match c.kind.as_str() {
            "nular" => a.0 = true,
            "unary" => a.1 = true,
            "binary" => a.2 = true,
            _ => {}
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_loads_and_is_nontrivial() {
        assert!(COMMANDS.len() > 400, "got {}", COMMANDS.len());
    }

    #[test]
    fn known_commands_resolve_case_insensitively() {
        // express.cpp Const() lower-cases every identifier before lookup
        // (strlwr, express.cpp:182), so command matching is case-insensitive.
        assert!(is_command("player"));
        assert!(is_command("PLAYER"));
        assert!(is_command("setPos"));
        assert!(is_command("foreach"));
        assert!(!is_command("definitely_not_a_command"));
    }

    // ---- GameType bit values: express.hpp:44-57, GameStateExt.hpp:12-18 -----

    #[test]
    fn base_type_bits_match_source_constants() {
        // express.hpp:44-48
        assert_eq!(type_bits("GameScalar"), 1);
        assert_eq!(type_bits("GameArray"), 2);
        assert_eq!(type_bits("GameBool"), 4);
        assert_eq!(type_bits("GameString"), 8);
        assert_eq!(type_bits("GameNothing"), 16);
        // express.hpp:49-53
        assert_eq!(type_bits("GameIf"), 0x1000000);
        assert_eq!(type_bits("GameWhile"), 0x2000000);
        assert_eq!(type_bits("GameFor"), 0x4000000);
        assert_eq!(type_bits("GameForFrom"), 0x8000000);
        assert_eq!(type_bits("GameForTo"), 0x10000000);
        // GameStateExt.hpp:12-18
        assert_eq!(type_bits("GameObject"), 0x100);
        assert_eq!(type_bits("GameVector"), 0x200);
        assert_eq!(type_bits("GameTrans"), 0x400);
        assert_eq!(type_bits("GameOrient"), 0x800);
        assert_eq!(type_bits("GameSide"), 0x1000);
        assert_eq!(type_bits("GameGroup"), 0x2000);
        assert_eq!(type_bits("GameFile"), 0x4000);
    }

    #[test]
    fn composite_and_wildcard_type_bits() {
        // OR-composites used in the command tables
        assert_eq!(type_bits("GameObjectOrArray"), 0x100 | 2);
        assert_eq!(type_bits("GameObjectOrGroup"), 0x100 | 0x2000);
        // express.hpp:56 GameVoid = ~(GameNothing | FakeTypes) — any value type
        assert_eq!(type_bits("GameVoid"), VALUE_MASK);
        assert_eq!(VALUE_MASK & 16, 0, "VALUE_MASK must exclude Nothing");
        assert_eq!(VALUE_MASK & FAKE_MASK, 0, "VALUE_MASK must exclude fake types");
        assert_ne!(VALUE_MASK & type_bits("GameObject"), 0, "VALUE_MASK includes Object");
        // express.hpp:57 GameAny = ~FakeTypes — any type including nothing
        assert_eq!(type_bits("GameAny"), !FAKE_MASK);
        assert_ne!(type_bits("GameAny") & 16, 0, "GameAny includes Nothing");
    }

    #[test]
    fn fake_mask_is_the_control_flow_types() {
        // express.hpp:55 FakeTypes = If|While|For|ForFrom|ForTo
        assert_eq!(
            FAKE_MASK,
            type_bits("GameIf")
                | type_bits("GameWhile")
                | type_bits("GameFor")
                | type_bits("GameForFrom")
                | type_bits("GameForTo")
        );
    }

    // ---- GamePriority: express.hpp:503-516 + registered priorities ----------

    #[test]
    fn binary_priorities_match_registrations() {
        // GamePriority ordinals (express.hpp:503-516):
        // nula0 logicOr1 logicAnd2 comparison3 function4 functionFirst5
        // soucet6 soucin7 unar8 mocnina9 zavorky10
        assert_eq!(binary_priority("||"), 1); // logicOr
        assert_eq!(binary_priority("&&"), 2); // logicAnd
        assert_eq!(binary_priority("+"), 6); // soucet
        assert_eq!(binary_priority("-"), 6); // soucet
        assert_eq!(binary_priority("*"), 7); // soucin
        assert_eq!(binary_priority("/"), 7); // soucin
        assert_eq!(binary_priority("^"), 9); // mocnina
        // named binary commands are registered at `function` (4)
        assert_eq!(binary_priority("setpos"), 4);
        assert_eq!(binary_priority("select"), 4);
        // `==` is registered with both function and comparison; we key the
        // lowest (comparison=3), which is its real SQF precedence.
        assert_eq!(binary_priority("=="), 3);
    }

    // ---- arities & signatures: from the registration rows -------------------

    #[test]
    fn arities_match_registration_kinds() {
        // GameNular(GameObject,"player",Player) — nular only
        assert_eq!(arities("player"), (true, false, false));
        // GameOperator(GameNothing,"setDamage",function,…,GameObject,GameScalar) — binary
        assert_eq!(arities("setdamage"), (false, false, true));
        // `if` is a unary function returning GameIf (express.cpp GetDefaultUnary)
        assert_eq!(arities("if"), (false, true, false));
        // `+` is registered as both unary (copy/negate) and binary (soucet)
        let (n, u, b) = arities("+");
        assert!(!n && u && b, "+ should be unary and binary, got {:?}", (n, u, b));
    }

    #[test]
    fn ret_and_arg_bits_from_signature() {
        // GameStateExt.cpp:1218 area: setDamage : GameObject × GameScalar → GameNothing
        let sd = lookup("setdamage");
        let op = sd.iter().find(|c| c.kind == "binary").expect("setDamage binary");
        assert_eq!(op.ret_bits(), 16); // Nothing
        assert_eq!(op.arg_bits(0), 0x100); // Object
        assert_eq!(op.arg_bits(1), 1); // Scalar
    }

    #[test]
    fn signature_and_type_name_rendering() {
        // setPos is binary: <OBJECT> setPos <ARRAY> → NOTHING
        assert!(lookup("setpos").iter().any(|c| c.signature().contains("setPos")));
        assert_eq!(Command::type_name("GameObjectOrArray"), "OBJECT|ARRAY");
        assert_eq!(Command::type_name("GameNothing|GameScalar"), "NOTHING|SCALAR");
        assert_eq!(Command::type_name("GameNothing"), "NOTHING");
    }

    #[test]
    fn keywords_are_real_registered_commands() {
        // Every cosmetic keyword must actually be a registered command, since
        // SQF has no reserved words (express.cpp resolves them at runtime).
        for kw in KEYWORDS {
            assert!(is_command(kw), "keyword `{kw}` is not a registered command");
        }
        assert!(is_keyword("THEN")); // case-insensitive
        assert!(!is_keyword("player"));
    }
}
