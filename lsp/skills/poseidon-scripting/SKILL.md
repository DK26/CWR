---
name: poseidon-scripting
description: >-
  Use when reading, writing, or reviewing classic Arma: Cold War Assault
  (originally Operation Flashpoint; "Poseidon" engine) mission and addon
  resources — SQF (.sqf), SQS (.sqs), and ParamFile configs (.sqm,
  description.ext, .cfg, config.cpp). Encodes the dialect rules that differ from
  modern Arma 3 SQF (which most training data describes), and a verify-after-edit
  loop using the engine-grounded `poseidon-check` linter.
---

# Poseidon engine scripting (classic Arma: Cold War Assault / OFP)

You are working with the **classic 2001 Arma: Cold War Assault** (originally
*Operation Flashpoint*) scripting and config languages — **not** modern Arma 3
SQF. Most SQF knowledge in training data is Arma 2/3 and is subtly or badly wrong
for this engine. Two rules:

1. **Trust the engine, not your memory.** Before using a command, confirm it
   exists in this dialect and check its signature with `poseidon-check sig`.
2. **Verify every edit.** After writing or changing any `.sqf`/`.sqs`/config
   file, run `poseidon-check` on it and treat the output as ground truth — it is
   a faithful port of the engine's own expression evaluator, so its verdicts are
   what the game would do.

## The tool: `poseidon-check`

A standalone linter + command-reference for this dialect. It is independent of
any editor, agent, or project — install it once and run it on the files in
whatever mission/addon repo you are working in.

If it is not already on your `PATH`, install it from a checkout of the language
tool:

```sh
cargo install --path crates/poseidon-cli     # puts `poseidon-check` on your PATH
```

(or use a prebuilt release binary if one is published for your platform). Confirm
with `poseidon-check --version`.

Then, from your own project:

```sh
poseidon-check mission/init.sqf            # validate (language inferred from extension)
poseidon-check --stdin --lang sqf < x.sqf  # validate a snippet
poseidon-check --json scripts/*.sqf        # machine-readable output, for tooling
poseidon-check --strict mission/init.sqf   # make warnings fail too
poseidon-check sig setDamage distance      # signature(s) + docs for a command
```

Exit codes: `0` clean · `1` errors found (or, with `--strict`, any warning) ·
`2` bad usage / I/O. Wire `--strict` into a verify loop when you want every
diagnostic to gate.

**Severity model (important).** Only *lexical* breakage (unterminated string,
unbalanced brackets) is reported as an `error`. Everything semantic — an unknown
command, a type mismatch, a missing operand — is a **`warning`**, because the
checker treats a variable as unknown ("any") type and never raises a false red.
So a file can be exit-0 "clean" yet still have a real bug flagged as a warning.
**Read the warnings**, or run `--strict` so they fail the check.

## Dialect rules that bite (classic OFP ≠ Arma 3)

- **`distance` is OBJECT × OBJECT only.** `obj1 distance obj2`. There is *no*
  object-to-position form (`obj distance [x,y,z]` was added later in Arma). Use
  vector math on `getPos` if you need distance to a position.
- **Damage is `setDamage` / `setDammage` (both valid), 0..1.** `0` = pristine,
  `1` = destroyed. The British spelling `setDammage` / `getDammage` is original.
- **No reserved keywords.** `if`, `then`, `while`, `do`, `forEach`, `and`, `or`,
  `call`, etc. are ordinary *registered commands*, and a local variable can
  shadow them. Control flow is built from values: `if (cond) then {…} else {…}`
  produces and consumes `If`/`Array` typed values — that is why `then` takes the
  `{…} else {…}` pair.
- **Many Arma 3 commands simply do not exist here**, e.g. `setVehicleInit`,
  `setPosATL`, `params`, `isNil` (function form), most `BIS_fnc_*`. If
  `poseidon-check sig <name>` says "not a command", it is not in this engine —
  do not use it.
- **Identifiers are case-insensitive**, `[A-Za-z_][A-Za-z0-9_]*`. `_`-prefixed
  names are local (script) variables; bare names are commands or globals.
- **Strings are double-quoted; the only escape is `""`** (a doubled quote).
  Single quotes are not string delimiters.
- **Numbers**: decimal/scientific, plus `0x`/`$` hex. Config also accepts `dbNN`.

## SQS vs SQF

- `.sqf` is `;`-separated / expression-structured; `.sqs` is **line-structured**
  (one statement per line) with leading sigils: `~delay`, `@condition`,
  `?cond : stmt`, `#label`, `goto`. `poseidon-check` handles both — give it the
  right extension, or `--lang sqs`.

## ParamFile configs (.sqm / description.ext / .cfg / config.cpp)

- One grammar across all of them: nested `class Name : Base { member = value;
  arr[] = {…}; class Sub {…}; };`. Preprocessor directives (`#include`,
  `#define`, …) are honoured.
- `poseidon-check` lints configs *lexically* (strings, comments, bracket
  balance). Class-inheritance / semantic checks are not implemented yet, so do
  not rely on it to catch an undefined base class.

## Workflow

1. Writing SQF/config? Look up unfamiliar commands first: `poseidon-check sig X`.
2. Save the file.
3. `poseidon-check <file>` (add `--strict` to gate on warnings). Read every
   diagnostic; warnings here are usually real bugs, not noise.
4. Fix and re-run until clean.

## Live checks in an editor (optional)

The same checks are available live in any LSP-capable editor (VS Code, Neovim,
Helix, Emacs, Kate, …) via the `poseidon-lsp` server, which shares the exact same
checker, so its verdicts agree with the CLI. Agents that consume LSP diagnostics
get them automatically after each edit; everyone else can rely on the CLI.

---

*Independent community tool — not affiliated with or endorsed by Bohemia
Interactive, Codemasters, or EA. "Arma" / "Bohemia Interactive" are trademarks of
Bohemia Interactive a.s.; "Operation Flashpoint" is a trademark of Codemasters /
EA; used here only to describe the engine and dialect this targets.*
