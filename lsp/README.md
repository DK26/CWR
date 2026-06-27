# Poseidon Language Server

A standalone, editor-independent **Rust** language server + TextMate grammars for
the classic *Arma: Cold War Assault* (originally released as *Operation
Flashpoint*) resource languages:

- **SQF** (`.sqf`) and **SQS** (`.sqs`) — the GameState scripting language
- **ParamFile** configs (`.sqm`, `.ext`, `.cfg`, `config.cpp`) — the class-tree config format

It targets the **classic 2001 dialect** as implemented by this engine
(`engine/Evaluator/express.cpp`, `engine/Poseidon/IO/ParamFile/*`), deliberately
*not* modern Arma 3 SQF. The command set is extracted directly from the engine
source, so completion and hover match what this engine actually accepts.

## Layout

| Crate / dir | Purpose |
|---|---|
| `crates/poseidon-catalog` | The 600 registered command overloads (525 unique names) + `GameType` rendering + signatures, extracted from `express.cpp` (core language), `Game/Commands/GameStateExt*.cpp` (engine), and `Evaluator/EvalState.cpp` (evaluator host). Also the authored **doc corpus** (`data/docs.json`). |
| `crates/poseidon-syntax` | Lexers + diagnostics for SQF/SQS and ParamFile, plus the config class-tree extractor. Byte-offset based; no editor assumptions. |
| `crates/poseidon-lsp` | The `tower-lsp` server binary (for editors). |
| `crates/poseidon-cli` | The `poseidon-check` binary — one-shot linter + `sig` lookup for agents and CI. Same checker as the server. |
| `editors/vscode` | Thin VS Code client + TextMate grammars (base coloring). |

## What works today

- **Diagnostics** — *lexical*: unterminated strings/comments, unbalanced `() [] {}` (mirrors `EvalNum` / `EvalOpenB` / `EvalCloseB`); *semantic*: a **precedence-correct type checker** — a faithful port of `express.cpp`'s `Vyhod`/`VyhCast` `_checkOnly` path (`crates/poseidon-syntax/src/check.rs`). It evaluates *types* (`GameType` bitmasks) using the engine's own overload resolution (`argType & operandType != 0`, union the matching return types, empty union = error) with the real per-operator `GamePriority`, so operand association is exact. Catches unknown/misspelled commands (`_unit serDamage 1`), arity (`player 5`), unary and **binary** type mismatches (`player setDamage "x"`, `1 + "two"`), missing operands (`x = 1 +`), and even Arma-isms invalid in *this* engine (`player distance [0,0,0]` — classic OFP `distance` is OBJECT×OBJECT only). Variables have unknown type → treated as "any", so they never produce a false error. Verified against the repo's fixtures: **all 23 valid scripts clean, and the one fixture named `error.sqf` (`x = 1 +`) correctly flagged**.
- **Completion** — all 493 identifier-form commands + control keywords, with signatures.
- **Hover** — full overload signatures rendered from the engine type system, e.g. `<OBJECT> setDamage <SCALAR> → NOTHING`, with classic-OFP attribution, plus an authored description + runnable examples for commands the doc corpus covers.
- **Semantic tokens** — commands vs. local/global variables vs. keywords vs. strings/numbers/comments, computed from the real lexer (this is the *smart* coloring; the TextMate grammars give base coloring without the server).
- **Document symbols** — the `class` tree for config files (outline / breadcrumbs / go-to-symbol).

## Using it from an agent or CI — `poseidon-check`

An LSP server is built for a human in an editor (cursor, hover, push diagnostics
over a stateful session). An autonomous agent or a CI job wants the opposite:
feed it a file, get compiler-style diagnostics and an exit code. `poseidon-check`
is that front end, over the *same* checker the server uses.

```sh
cargo build --release -p poseidon-cli      # → target/release/poseidon-check

poseidon-check mission/init.sqf            # validate (language from extension)
poseidon-check mission/                    # lint a whole mission folder (recursive)
poseidon-check --stdin --lang sqf < x.sqf  # validate stdin
poseidon-check --json mission/*.sqf        # machine-readable diagnostics
poseidon-check --strict mission/init.sqf   # make warnings fail too
poseidon-check sig setDamage distance      # signature(s) + docs for a command
poseidon-check export-ref refs/            # emit docs as engine validate_ref JSON
```

Exit codes: `0` clean · `1` errors (or, with `--strict`, any warning) · `2`
usage/I/O. **Severity model:** only lexical breakage (unterminated string,
unbalanced brackets) is an `error`; everything semantic — unknown command, type
mismatch, missing operand — is a `warning`, because the checker treats a
variable as "any type" and never raises a false red. So pass `--strict` (or read
the warnings) when you want semantic issues to gate a verify loop.

For **OpenCode** and other agents that consume LSP `publishDiagnostics`, point
them at `poseidon-lsp` and the type checker fires automatically after each edit —
no CLI wiring needed. The CLI covers the rest (one-shot validation, `sig`
lookup) for agents that drive a shell instead.

There is also a portable, vendor-neutral agent skill at
[`skills/poseidon-scripting`](skills/poseidon-scripting/SKILL.md) that teaches an
agent the classic-OFP dialect rules (which differ from Arma 3) and the
verify-after-edit workflow above. It is not tied to this repo or any one tool —
drop it into whatever project you author missions in (see
[skills/README.md](skills/README.md) for per-tool adoption).

## Documentation corpus

`crates/poseidon-catalog/data/docs.json` adds a human-readable description +
runnable examples per command, surfaced in hover and `poseidon-check sig`. The
engine ships no in-binary doc strings, so this prose is **authored** — the one
part of the project not mechanically extracted from source. It is kept honest by
two gates: every documented name must be a real command in this engine
([docs_grounded.rs](crates/poseidon-catalog/tests/docs_grounded.rs)), and every
example must lex-clean and type-check clean against the engine-faithful checker
([doc_examples.rs](crates/poseidon-syntax/tests/doc_examples.rs)) — so an example
can never quietly use an Arma-3 form or a wrong operand type. The signature shown
alongside always comes from the catalog, never duplicated in the corpus, so the
two cannot drift. Coverage is a curated common subset, not all 537 names;
undocumented commands still hover/`sig` with their signature.

`poseidon-check export-ref <dir>` writes the corpus out in the engine's *own*
`validate_ref` JSON schema — the exact format `apps/tools/Evaluator`
(`PoseidonEvaluator --validate-ref <dir>`) consumes — so the same examples our
checker gates can also be run through the real C++ engine.

## Testing — grounded in the engine source

`cargo test` runs **92 tests**, and every expectation is tied to specific engine
source, not to general SQF knowledge:

- **Reference oracle + differential fuzzing** ([oracle.rs](crates/poseidon-syntax/src/oracle.rs), [differential.rs](crates/poseidon-syntax/tests/differential.rs)) — the strongest one. `oracle.rs` is a faithful **Rust transliteration of the engine's evaluator** (`express.cpp` `Vyhod`/`VyhCast`, `_checkOnly` mode) as an iterative priority-stack machine — deliberately a *different shape* from the production checker's recursive precedence-climbing, so the two are independent implementations of the same algorithm. The oracle is first grounded against the engine maintainers' own recorded verdicts (below); then **8,000 randomly generated expressions** are run through both the checker and the oracle, requiring identical accept/reject — currently **0 divergences**.
- **Live-engine parity** ([evaluator_parity.rs](crates/poseidon-syntax/tests/evaluator_parity.rs)) — the repo ships `apps/tools/Evaluator` = `PoseidonEvaluator`, a standalone CLI over the *real* C++ `EvaluatorHost` (`--eval <expr>` returns the engine's accept/reject as an exit code). This test runs every vector through that binary and asserts (1) the live engine agrees with the recorded verdict and (2) our Rust `oracle.rs` agrees with the live engine — upgrading the grounding from "recorded verdicts" to "the running engine." It is **gated**: with no C++ build present (e.g. default CI) it prints how to provide one (`POSEIDON_EVALUATOR=…`) and skips. So the oracle is the no-toolchain stand-in; when the toolchain exists, this checks it against the genuine article.
- **Engine-verdict corpus** ([engine_oracle.rs](crates/poseidon-syntax/tests/engine_oracle.rs)) — `engine_vectors.json` is harvested from the engine's *own* C++ Evaluator tests (`tests/unit/**/Evaluator/*.cpp`) and `validate_ref` examples: every expression the engine team `Evaluate`d, tagged with the verdict *they* asserted. We require agreement (and the oracle does too). Building all this caught real checker bugs: a variable shadowing a same-named command (`health - damage`), and misplaced operators (`5 + * 3`). Runtime-only engine errors (division by zero, undefined-variable reads) are out of a static checker's scope and excluded.
- **Preprocessor / macro robustness** ([macros.rs](crates/poseidon-syntax/tests/macros.rs)) — the engine preprocesses before evaluating, so `#define`/`#include` directive lines are lexed as opaque `Macro` tokens and skipped by the checker (no more false positives on the directive body), `\`-continued `#define`s are one logical line, and a macro *defined in the file* is treated as an opaque value at its call site — while an undefined `MACRO(x)` is still flagged as an unknown command.

- **Catalog** — `type_bits` for each `GameType` is asserted against the constants in [express.hpp:44-57](../engine/Evaluator/express.hpp#L44) / [GameStateExt.hpp:12-18](../engine/Poseidon/Game/Commands/GameStateExt.hpp#L12); operator priorities against the `GamePriority` enum (express.hpp:503-516); arities and signatures against the registration rows.
- **Completeness scan** ([completeness.rs](crates/poseidon-catalog/tests/completeness.rs)) — reads the engine source directly and asserts the catalog covers *every* `GameNular`/`GameFunction`/`GameOperator`/`TABLE_COMMAND` registration under `engine/`. This caught 12 commands an earlier extractor missed (namespace-qualified impl fns + a `SceneDraw.cpp` registration site) and now fails on any future gap.
- **Lexer** — token rules asserted against `express.cpp` `Const`/`Vyhod`/`sejmid` (identifier charset, `0x`/`$` hex, `""` string escaping, case-insensitivity, 2-char operators, SQS sigils).
- **Checker** — an accept/reject corpus where each case cites the overload row that makes it valid or invalid, plus precedence cases (`then`/`else` threading via `functionFirst` > `function`), grounded in `VyhCast`'s overload rule.
- **Config** — class/enum/member/array grammar and the eight preprocessor directives, grounded in `ParamFile.cpp`/`Preproc.cpp`.
- **Doc corpus** — [docs_grounded.rs](crates/poseidon-catalog/tests/docs_grounded.rs) asserts every documented name is a real command (no Arma-isms); [doc_examples.rs](crates/poseidon-syntax/tests/doc_examples.rs) runs every authored example through the lexer + checker so the docs can't drift from the dialect.
- **CLI** — [poseidon-cli unit tests](crates/poseidon-cli/src/lib.rs) cover language inference, the error-vs-warning severity model, `--strict`, JSON shape, `sig` rendering, and `export-ref`'s `validate_ref` mapping; [cli.rs](crates/poseidon-cli/tests/cli.rs) spawns the real `poseidon-check` binary and asserts stdout + exit codes (the contract agents/CI depend on), including walking a real shipped mission directory.
- **Server** — `lang_of`, semantic-token classification + multi-line splitting, diagnostic position mapping, `LineIndex` byte↔UTF-16 round-trips (incl. astral chars), hover (signature + authored docs), completion, document symbols.
- **Integration** — [corpus.rs](crates/poseidon-syntax/tests/corpus.rs) runs every shipped `.sqf`/`.sqs`/config fixture (valid ⇒ clean, `error.sqf` ⇒ flagged); [protocol.rs](crates/poseidon-lsp/tests/protocol.rs) spawns the real binary and drives a full LSP round-trip over stdio.

When no C++ toolchain is present (the default here, and in the Rust-only CI job),
the oracle *is* the engine's evaluator ported to Rust from `express.cpp`, grounded
against the engine team's recorded verdicts, and the production checker is
differentially fuzzed against it. When a build of `apps/tools/Evaluator`
(`PoseidonEvaluator`) **is** available, the gated `evaluator_parity` test closes
the loop against the genuine compiled engine (set `POSEIDON_EVALUATOR=…` or build
the target). The one intentional checker-vs-oracle deviation (a unary command
with no value operand is read as a variable shadowing the command, to avoid false
positives on real code) is documented where the fuzzer skips it.

## Build & use

```sh
cd lsp
cargo build --release          # produces target/release/{poseidon-lsp, poseidon-check}
```

`poseidon-lsp` is the editor server; `poseidon-check` is the agent/CI linter
(see [Using it from an agent or CI](#using-it-from-an-agent-or-ci--poseidon-check)).

**VS Code:** `cd editors/vscode && npm install`, then run the extension (F5) or
package it with `vsce`. It launches `poseidon-lsp` from `PATH` (override via the
`poseidon.server.path` setting).

**Any other editor** (Neovim, Helix, Emacs, Kate…): point its LSP client at the
`poseidon-lsp` binary over stdio — that is the whole interface; nothing is
VS Code-specific.

```lua
-- Neovim example
vim.lsp.start({ name = "poseidon", cmd = { "poseidon-lsp" },
  root_dir = vim.fn.getcwd() })
```

## Faithfulness notes

- Identifiers are `[A-Za-z_][A-Za-z0-9_]*`, **case-insensitive**; `_`-prefixed = local variable.
- Strings are double-quoted with `""` as the only escape. An unterminated string is recovered at end-of-line (the engine tolerates newlines in strings; editors do not benefit from that, so we stop early for better diagnostics).
- Numbers: decimal/scientific, `0x` and `$` hex; config also accepts `dbNN` decibels.
- Keyword-ness is **data-driven**: `if/then/while/forEach/and/or` etc. are ordinary registered commands in the engine, and a local variable shadows them. We highlight them as keywords purely cosmetically.
- **Preprocessor** runs before the evaluator, so `#define`/`#include`/`#ifdef`… lines (including `\`-continued ones) are opaque to the type checker. We do not expand macros (no preprocessor), but in-file `#define`d names are recognized so their call sites aren't mistaken for unknown commands.

CI: [`.github/workflows/poseidon-lsp.yml`](../.github/workflows/poseidon-lsp.yml) builds and tests the `lsp/` workspace and lints the shipped mission fixtures on every change — a self-contained, HEMTT-style "lint the resources as part of the build."

## Roadmap (not yet implemented)

- **Variable flow typing**: the checker treats every variable as "any type". Tracking assignments (`_x = getPos y` ⇒ `_x : ARRAY`) within a scope would let it catch mismatches through variables, not just literals.
- **Config semantics**: `class` inheritance resolution, duplicate-member and undefined-base-class diagnostics, value-type checks against a config schema.
- **Go-to-definition / find-references** for config `class` inheritance and across `#include`d files.
- **Doc corpus coverage**: the authored corpus (`crates/poseidon-catalog/data/docs.json`, surfaced in hover + `poseidon-check sig`) currently covers a curated common subset, not all 537 names. Extend it toward full coverage — every entry is gated by the grounding tests, so additions stay honest.
- **SQS line-field offset mapping** for exact diagnostic ranges on `~`/`@`/`?` lines.
- Incremental (delta) document sync and semantic-tokens range requests for large files.

## Trademarks & affiliation

This is an independent, community tool. It is **not affiliated with, authorized,
or endorsed by** Bohemia Interactive a.s., Codemasters, or Electronic Arts.
"Poseidon" refers only to the engine's own historical codename (as published in
this repository's source) and is **not claimed by this project as a trademark**.
*Arma* and *Bohemia Interactive* are trademarks of Bohemia Interactive a.s.;
*Operation Flashpoint* is a trademark of Codemasters / Electronic Arts. All
trademarks are the property of their respective owners and are used only
descriptively, to identify the engine and dialect this tool targets.
