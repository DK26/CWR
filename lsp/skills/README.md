# Agent skill: `poseidon-scripting`

[`poseidon-scripting/SKILL.md`](poseidon-scripting/SKILL.md) is a **portable,
vendor-neutral** guide for any AI agent (or human) creating and maintaining
classic *Arma: Cold War Assault* (originally *Operation Flashpoint*) mission and
addon resources — SQF, SQS, and ParamFile configs. It is not tied to this
repository or to any one agent tool: drop it into whatever project you author
missions in.

It carries two things training data gets wrong: the **classic-OFP dialect rules**
(which differ from modern Arma 3 SQF) and a **verify-after-edit loop** built on
the standalone [`poseidon-check`](../crates/poseidon-cli) linter.

## Using it in your own project

The file is plain Markdown with a small YAML front-matter header (`name`,
`description`), the widely-adopted "agent skill" format. Adopt it however your
tool expects:

| Tool | How |
|---|---|
| **Claude Code** | Copy the `poseidon-scripting/` folder into your project's `.claude/skills/` (or `~/.claude/skills/` for all projects). It activates automatically when you touch `.sqf`/`.sqs`/config files. |
| **OpenCode / Cursor / others using a rules file** | Append the body of `SKILL.md` to your `AGENTS.md` / rules file, or reference the file from it. |
| **Any other agent** | Hand it the file as context / a system prompt when working on these resources. |
| **No agent at all** | Read it — it is a concise human guide to the dialect and the linter, too. |

The only external dependency is the `poseidon-check` binary on your `PATH`
(install: `cargo install --path crates/poseidon-cli` from a checkout of this
tool, or a published release binary). The skill explains the rest.
