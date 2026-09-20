English | [日本語](README.ja.md)

<p align="center">
  <img src="docs/images/agent-tutor-logo.png" alt="agent-tutor" width="720">
</p>

# agent-tutor

`agent-tutor` (`tutor`) helps you coach and nurture AI coding agents (such as Claude Code) through iterative human feedback.

Instead of repeating corrections or manually maintaining rules, `tutor` extracts human-authored turns made on a specified local calendar day from conversation transcripts. Each turn is emitted with preceding assistant replies, tool history, and a flag indicating whether the guidance was already captured into memory or `CLAUDE.md`. Through CLI, MCP, or an interactive Web Review UI, you can triage feedback and systematically draft improvements to rules, documentation, memory, skills, or issue trackers.

## Usage

```console
# Basic extraction
tutor                                            # Extract today's human turns (JSONL)
tutor --date 2026-09-17 --format md              # Markdown output for a specific date
tutor projects                                   # List projects with unreviewed turns

# Interactive Web Review UI
tutor ui                                         # Launch interactive web review UI in browser
tutor ui --project my-project                    # Launch review UI for a specific project

# Adaptive Destinations & Ledger
tutor destinations                               # Inspect detected destination layers
tutor destinations --reset                       # Re-scan environment and reset destinations config
tutor ledger                                     # Inspect reviewed turns ledger
tutor mark --session <SESSION_ID> --turn 1 --status approved --destination project_claude

# MCP Setup & Server
tutor install-mcp                                # Configure MCP server in ~/.claude.json
tutor mcp                                        # Run as MCP stdio server
```

By default, `--date` is today in the machine's local timezone, `--projects-dir` is
`~/.claude/projects`, and output is JSONL. Reviewed turns recorded with `approved` or `rejected`
status in the ledger (`~/.local/state/tutor/ledger.json` by default) are automatically skipped
from extraction. Pass `--no-ledger` to inspect all candidate turns. Run `tutor --help` for all options.

## Installation and Development

```console
# Install from source via cargo
cargo install --git https://github.com/syarihu/agent-tutor

# Or build locally with Makefile
make install      # Build optimized release binary and install to ~/.cargo/bin/tutor
make dev          # Build debug binary and symlink ~/.cargo/bin/tutor to target/debug/tutor
make install-mcp  # Automatically configure tutor MCP server in ~/.claude.json
make status       # Show active tutor binary location and version
make test         # Run test suite
```

In `make dev` mode, `~/.cargo/bin/tutor` is symlinked to `target/debug/tutor`. Running `cargo build` after editing code will immediately update the active CLI and MCP commands without re-installing.

### Model Context Protocol (MCP) Setup

Run `tutor install-mcp` (or `make install-mcp`) to automatically configure `~/.claude.json` with the tutor MCP server:

```console
tutor install-mcp
```

Or manually configure your MCP client:

```json
{
  "mcpServers": {
    "tutor": {
      "type": "stdio",
      "command": "tutor",
      "args": ["mcp"]
    }
  }
}
```

Available MCP tools:
- `tutor_projects`: List repositories/projects with unreviewed turns for a local date
- `tutor_extract`: Extract candidate turns and context for a local date
- `tutor_review_ui`: Launch interactive Web Review UI in browser and return finalized decisions
- `tutor_mark`: Record review decisions (approved, rejected, deferred) into the ledger
- `tutor_destinations`: Get the canonical destinations table and triaging guidelines
- `tutor_ledger`: Inspect current ledger entries

Available MCP prompts:
- `tutor_review`: Run the complete end-to-end feedback triaging workflow

## Web Review UI

`tutor ui` (or the MCP tool `tutor_review_ui`) launches a lightweight local HTTP server and opens an interactive review interface in your browser:

- **Candidate triaging**: Review each human turn side-by-side with preceding assistant context.
- **Draft diffs and issue templates**: Inspect and edit proposed changes before applying them.
- **Adaptive destinations**: Route suggestions directly to `CLAUDE.md`, documentation, `memory/`, skills, or task managers.
- **Feedback & rejections**: Add comments or revision requests directly below review decisions (`user_comment`).
- **Ledger persistence**: Reviewed items are saved automatically to `~/.local/state/tutor/ledger.json` upon submission.

## Destinations and Adaptive Detection

`tutor` automatically detects available destinations based on your local environment:
- **Core conventions**: Project-level (`<cwd>/CLAUDE.md`, `<cwd>/AGENTS.md`), global (`~/.claude/CLAUDE.md`, `~/.config/rules/AGENTS.md`), documentation (`docs/`), and temporary memory (`memory/`).
- **CLI & MCP tools**: Automatically enables issue tracking or knowledge destinations when tools like `gh` (GitHub), `linear`, `jira`, `asana`, `notion`, or `lk` are configured on your machine or registered in `~/.claude.json`.

Run `tutor destinations` to inspect the detected layers, or edit `~/.config/tutor/destinations.json` to customize. Pass `--reset` to re-scan the environment and regenerate configuration.

## Output

The default output contains one JSON object per selected turn:

```json
{"session":"session-id","project_dir":"-Users-example-project","cwd":"/Users/example/project","turn_index":1,"timestamp_local":"2026-09-17T11:04:07+09:00","human":"please fix this","prev_assistant":"What should I change?","prev_tools":[],"already_captured":false}
```

`--format md` groups the same information beneath session headings, with one block per turn.
`prev_assistant` is truncated by Unicode character count. For Bash calls, `prev_tools` includes the
description when present, otherwise the first command line; either value is limited to 60 Unicode
characters. `already_captured` is true when a `Write` or `Edit` call before the next human turn
targets a `/memory/` path or a path ending in `CLAUDE.md` or `AGENTS.md`.

## Human-turn selection and boilerplate

The authoritative signal for a genuinely human-originated transcript record is
`origin.kind == "human"`. This excludes injected system reminders, attachment events, tool results,
and subagent prompts without brittle inspection of their text. Candidate turns must additionally be
non-sidechain string messages whose UTC timestamp falls on the requested date after conversion to
the machine's local timezone.

Some launchers enter fixed prompts through the same terminal path as a person, so those prompts also
carry `origin.kind == "human"`. `tutor` therefore performs a second pass: identical text seen in at
least three distinct sessions is treated as automation boilerplate. The threshold is configurable,
and every excluded string and its distinct-session count is reported on stderr.
