# Poseidon Languages

Syntax highlighting and a language server for the **classic *Arma: Cold War
Assault*** (originally released as *Operation Flashpoint*) resource languages:

- **SQF** (`.sqf`) and **SQS** (`.sqs`) — the GameState scripting language
- **ParamFile** configs (`.sqm`, `.ext`, `.cfg`, `config.cpp`) — the class-tree config format

It targets the **classic 2001 dialect** as implemented by the open-sourced
"Poseidon" engine, deliberately *not* modern Arma 3 SQF. The command set is
extracted directly from the engine source, so completion, hover, and diagnostics
match what this engine actually accepts.

## Features

- Smart (semantic) coloring, plus base TextMate grammars
- Completion and hover with engine-derived signatures + authored docs
- Precedence-correct, engine-faithful type diagnostics
- Document symbols (the `class` tree) for config files

## Setup

The extension launches the `poseidon-lsp` server from your `PATH`. Build it from
the [`lsp/`](https://github.com/ofpisnotdead-com/CWR-CE) workspace with
`cargo build --release`, or set `poseidon.server.path` to its location.

## Trademarks & affiliation

This is an independent, community project. It is **not affiliated with,
authorized, or endorsed by** Bohemia Interactive a.s., Codemasters, or Electronic
Arts. "Poseidon" refers only to the engine's own historical codename and is not
claimed by this project as a trademark. *Arma* and *Bohemia Interactive* are
trademarks of Bohemia Interactive a.s.; *Operation Flashpoint* is a trademark of
Codemasters / Electronic Arts. All trademarks are the property of their respective
owners and are used here only descriptively, to identify the engine and dialect
this tool targets.

Licensed GPL-3.0-or-later.
