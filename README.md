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
tutor --since 2026-09-01 --until 2026-09-30      # Extract a date range (inclusive, local dates)
tutor --grep "migration"                         # Only turns whose human text contains "migration"
tutor --since 2026-09-01 --grep lint --grep format  # Range, keeping turns that mention lint or format
tutor projects                                   # List projects with unreviewed turns
tutor projects --since 2026-09-01                # Per-day counts per project, through today

# Interactive Web Review UI
tutor ui                                         # Launch interactive web review UI in browser
tutor ui --project my-project                    # Launch review UI for a specific project

# Adaptive Destinations & Ledger
tutor destinations                               # Inspect detected destination layers
tutor destinations --reset                       # Re-scan environment and reset destinations config
tutor ledger                                     # Inspect reviewed turns ledger
tutor ledger --migrate                           # Add dates to ledger entries written by older versions
tutor mark --session <SESSION_ID> --turn 1 --date 2026-09-17 --status approved --destination project_claude

# MCP Setup & Server
tutor install-mcp                                # Configure MCP server in ~/.claude.json
tutor mcp                                        # Run as MCP stdio server
```

By default, `--date` is today in the machine's local timezone, `--projects-dir` is
`~/.claude/projects`, and output is JSONL. Reviewed turns recorded with `approved` or `rejected`
status in the ledger (`~/.local/state/tutor/ledger.json` by default) are automatically skipped
from extraction. Pass `--no-ledger` to inspect all candidate turns. Run `tutor --help` for all options.

`--since` / `--until` take inclusive local calendar dates and cannot be combined with `--date`;
`--until` requires `--since`, and `--since` alone runs through today. A range returns the same turns
and turn indexes as running each day separately. With a range of more than one day,
`tutor projects` also reports per-day counts (`daily_counts` in JSON output). Long ranges can produce
a lot of extraction output, so check the counts with `tutor projects` first and narrow the extraction
with `--project`.

`--grep <TEXT>` keeps only turns whose human text contains TEXT. It is a literal, case-insensitive
substring match (not a regex), and the preceding assistant context is never searched. Repeat the flag
to match any of several values (OR). Filtering happens after turns are numbered, so a matched turn
keeps the same turn index as in an unfiltered run. `tutor projects` has no `--grep` option,
so its counts are not filtered by it and include turns the filter would drop.

## Ledger keys

`turn_index` restarts at 1 every local day, so a session that runs past midnight has a turn 1 on each
day. Ledger entries are therefore keyed by `session:YYYY-MM-DD:turn_index`, and every extracted turn
carries its local `date`. `tutor mark` needs `--date` (batch entries and the MCP `tutor_mark` need a
`date` too), taken from the extracted turn. Older entries keyed `session:turn_index` stay readable:
a turn is skipped when its dated entry says approved or rejected, or, if it has no dated entry, when
the legacy entry does. A dated entry always wins over a legacy one.

`tutor ledger --migrate [--ledger PATH] [--projects-dir PATH] [--boilerplate-threshold N]` gives legacy
entries a date. It renumbers the transcripts the same way `extract` does, finds the turn each legacy
entry points at, and moves the entry to the dated key; entries that lost their `date` field are
repaired from their key. The ledger is copied to `<ledger>.bak-<timestamp>` first, and nothing is
written when nothing changes. The counts of migrated, ambiguous, not found and repaired entries go to
stderr. Limits:

- If a midnight-spanning session already had a turn N on both days when the entry was marked (judged by
  its `timestamp`), the entry stays legacy, and keeps hiding turn N on both days.
- Entries whose transcripts were deleted stay legacy.
- Numbering is recomputed from the current transcripts and the current timezone, so deleted files or a
  timezone change since the entry was made can assign the wrong date. Use the same
  `--boilerplate-threshold` that `extract` used.

An older `tutor` binary does not recognise the dated keys: it treats those turns as unreviewed.

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
- `tutor_projects`: List repositories/projects with unreviewed turns for a local date or an inclusive `since`/`until` range (per-day counts for ranges)
- `tutor_extract`: Extract candidate turns and context for a local date or an inclusive `since`/`until` range, optionally filtered with `grep` (a string or an array of strings; literal, case-insensitive substrings of the human text, OR-matched)
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
{"session":"session-id","project_dir":"-Users-example-project","cwd":"/Users/example/project","turn_index":1,"date":"2026-09-17","timestamp_local":"2026-09-17T11:04:07+09:00","human":"please fix this","prev_assistant":"What should I change?","prev_tools":[],"already_captured":false}
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
non-sidechain string messages whose UTC timestamp falls on the requested date (or within the
requested date range) after conversion to the machine's local timezone.

Some launchers enter fixed prompts through the same terminal path as a person, so those prompts also
carry `origin.kind == "human"`. `tutor` therefore performs a second pass: identical text seen in at
least three distinct sessions is treated as automation boilerplate. The threshold is configurable,
and every excluded string and its distinct-session count is reported on stderr.
