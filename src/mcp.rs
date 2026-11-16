use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    Config, DateRange, JudgmentStatus, Ledger, LedgerEntry, OutputFormat, extract,
    parse_ledger_date, resolve_date_range, write_output,
};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

fn default_projects_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude/projects")
}

fn default_ledger_path() -> PathBuf {
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
        PathBuf::from(state_home).join("tutor/ledger.json")
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/state/tutor/ledger.json")
    }
}

pub fn run_mcp_server() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut warnings = io::stderr().lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                writeln!(warnings, "tutor mcp: failed to read line: {err}")?;
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(req) => req,
            Err(err) => {
                writeln!(warnings, "tutor mcp: invalid JSON-RPC message: {err}")?;
                continue;
            }
        };

        if request.method == "notifications/initialized" {
            continue;
        }

        let id = request.id.unwrap_or(Value::Null);
        let response = match handle_request(&request.method, request.params, &mut warnings) {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(result),
                error: None,
            },
            Err(err_msg) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(json!({
                    "code": -32603,
                    "message": err_msg
                })),
            },
        };

        serde_json::to_writer(&mut stdout, &response)?;
        writeln!(stdout)?;
        stdout.flush()?;
    }

    Ok(())
}

fn handle_request(
    method: &str,
    params: Option<Value>,
    warnings: &mut impl Write,
) -> Result<Value, String> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {},
                "prompts": {},
                "resources": {}
            },
            "serverInfo": {
                "name": "tutor",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": "Tutor captures human turns and preceding assistant context from Claude Code transcripts, allowing systematic triaging of feedback into conventions, memory, or documentation."
        })),

        "ping" => Ok(json!({})),

        "tools/list" => Ok(json!({
            "tools": [
                {
                    "name": "tutor_projects",
                    "description": "List all repositories/projects that have unreviewed human conversation turns on the specified date or date range, with turn counts and working directories. For a multi-day range each project also carries per-day counts (daily_counts).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "date": {
                                "type": "string",
                                "description": "Local calendar date to inspect in YYYY-MM-DD format. Defaults to today. Cannot be combined with since/until."
                            },
                            "since": {
                                "type": "string",
                                "description": "First local calendar date of a range, inclusive (YYYY-MM-DD). Without until, the range runs through today. Cannot be combined with date."
                            },
                            "until": {
                                "type": "string",
                                "description": "Last local calendar date of a range, inclusive (YYYY-MM-DD). Requires since. Cannot be combined with date."
                            },
                            "no_ledger": {
                                "type": "boolean",
                                "description": "If true, ignores ledger and counts all turns."
                            }
                        }
                    }
                },
                {
                    "name": "tutor_extract",
                    "description": "Extract unreviewed human turns and preceding assistant context from Claude Code transcripts on a specified local date or date range (since/until, inclusive). Settled turns in the ledger are skipped by default. Long ranges can produce large output; narrow them with project or grep. grep keeps only turns whose human text contains a literal, case-insensitive substring (not a regex; an array matches any of its values).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "date": {
                                "type": "string",
                                "description": "Local calendar date to extract in YYYY-MM-DD format. Defaults to today. Cannot be combined with since/until."
                            },
                            "since": {
                                "type": "string",
                                "description": "First local calendar date of a range, inclusive (YYYY-MM-DD). Without until, the range runs through today. Cannot be combined with date. Long ranges can produce large output; narrow with project."
                            },
                            "until": {
                                "type": "string",
                                "description": "Last local calendar date of a range, inclusive (YYYY-MM-DD). Requires since. Cannot be combined with date."
                            },
                            "project": {
                                "type": "string",
                                "description": "Optional filter by project directory slug or repository substring (e.g. 'agent-proctor', 'dotfiles')."
                            },
                            "grep": {
                                "type": ["string", "array"],
                                "items": {"type": "string"},
                                "description": "Optional filter on the human turn's text: a literal, case-insensitive substring (not a regex), or an array of them where a turn matching any one is kept (OR). Only the human text is searched, not the assistant context. Values must not be empty or whitespace-only. turn_index stays the same as in an unfiltered run. Each turn carries its local `date`; turn_index is numbered per date, so pass both to tutor_mark."
                            },
                            "no_ledger": {
                                "type": "boolean",
                                "description": "If true, disables ledger filtering and returns all candidate turns."
                            },
                            "format": {
                                "type": "string",
                                "enum": ["jsonl", "md"],
                                "description": "Output format: 'jsonl' (default) or 'md'."
                            }
                        }
                    }
                },
                {
                    "name": "tutor_mark",
                    "description": "Record a review judgment (approved, rejected, deferred) into the tutor ledger to prevent redundant reviews.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "session": {
                                "type": "string",
                                "description": "Session ID from the extracted turn."
                            },
                            "turn_index": {
                                "type": "integer",
                                "description": "1-based turn index from the extracted turn. It restarts every local day, so it is only unique together with date."
                            },
                            "date": {
                                "type": "string",
                                "description": "Local date (YYYY-MM-DD) of the turn, from the extracted turn's `date`."
                            },
                            "status": {
                                "type": "string",
                                "enum": ["approved", "rejected", "deferred"],
                                "description": "Review status."
                            },
                            "destination": {
                                "type": "string",
                                "description": "Target destination ID (e.g. global_claude, global_agents, project_claude, project_agents, project_docs, user_memory, workflow_skill, knowledge_lk, discard)."
                            },
                            "reason": {
                                "type": "string",
                                "description": "Reason for the judgment or convention."
                            }
                        },
                        "required": ["session", "turn_index", "date", "status"]
                    }
                },
                {
                    "name": "tutor_review_ui",
                    "description": "Launch a local web UI in the browser for human review of proposed feedback items and diff drafts. Blocks until the user clicks Submit, saves decisions to the ledger, and returns the finalized items.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "items": {
                                "type": "array",
                                "description": "Array of review items containing feedback context, proposed destination, reason, and draft unified diff.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "session": { "type": "string" },
                                        "turn_index": { "type": "integer" },
                                        "date": { "type": "string", "description": "Local date (YYYY-MM-DD) from the extracted turn's `date`. turn_index is only unique per date." },
                                        "human": { "type": "string" },
                                        "prev_assistant": { "type": "string" },
                                        "status": { "type": "string", "enum": ["approved", "rejected", "deferred"] },
                                        "destination": { "type": "string" },
                                        "reason": { "type": "string" },
                                        "user_comment": { "type": "string", "description": "Human feedback, decision rationale, or additional instructions" },
                                        "draft_diff": { "type": "string" },
                                        "cwd": { "type": "string" },
                                        "file_path": { "type": "string" }
                                    },
                                    "required": ["session", "turn_index", "date", "human"]
                                }
                            },
                            "port": {
                                "type": "integer",
                                "description": "Optional port to bind the review web server."
                            },
                            "no_ledger": {
                                "type": "boolean",
                                "description": "If true, does not automatically record decisions to ledger."
                            }
                        },
                        "required": ["items"]
                    }
                },
                {
                    "name": "tutor_destinations",
                    "description": "Get the destination directory and triaging rules (standard destinations and the 3 decision axes: Scope, Maturity, Granularity).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "tutor_ledger",
                    "description": "Get the current tutor ledger entries and reviewed turns.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }
            ]
        })),

        "tools/call" => {
            let params = params.ok_or_else(|| "Missing params for tools/call".to_owned())?;
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing tool name in params".to_owned())?;
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            call_tool(name, arguments, warnings)
        }

        "prompts/list" => Ok(json!({
            "prompts": [
                {
                    "name": "tutor_review",
                    "description": "Daily feedback recovery workflow: inspects projects, extracts unreviewed turns, launches Web Review UI, and applies approved changes.",
                    "arguments": [
                        {
                            "name": "date",
                            "description": "Local calendar date (YYYY-MM-DD). Defaults to today.",
                            "required": false
                        }
                    ]
                },
                {
                    "name": "tutor_projects",
                    "description": "List repositories with conversation transcripts and unreviewed feedback count.",
                    "arguments": [
                        {
                            "name": "date",
                            "description": "Local calendar date (YYYY-MM-DD). Defaults to today.",
                            "required": false
                        }
                    ]
                }
            ]
        })),

        "prompts/get" => {
            let params = params.ok_or_else(|| "Missing params for prompts/get".to_owned())?;
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing prompt name in params".to_owned())?;

            if name == "tutor_review" {
                let date_arg = params
                    .pointer("/arguments/date")
                    .and_then(Value::as_str)
                    .unwrap_or("today");
                let lang_arg = params
                    .pointer("/arguments/lang")
                    .and_then(Value::as_str)
                    .unwrap_or("ja");

                let prompt_text = generate_review_prompt(date_arg, lang_arg);

                Ok(json!({
                    "description": "Daily feedback recovery workflow",
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": prompt_text
                            }
                        }
                    ]
                }))
            } else if name == "tutor_projects" {
                let date_arg = params
                    .pointer("/arguments/date")
                    .and_then(Value::as_str)
                    .unwrap_or("today");

                Ok(json!({
                    "description": "List projects with transcripts",
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!("`tutor_projects` ツールを呼び出して、{date_arg} に会話ログがあるプロジェクト一覧と未レビュー件数を表示してください。")
                            }
                        }
                    ]
                }))
            } else {
                Err(format!("Unknown prompt: {name}"))
            }
        }

        "resources/list" => Ok(json!({
            "resources": [
                {
                    "uri": "tutor://destinations",
                    "name": "Canonical Destinations Table",
                    "description": "List of standard destination layers and their scopes",
                    "mimeType": "text/markdown"
                },
                {
                    "uri": "tutor://guidelines",
                    "name": "Decision Axes and Guidelines",
                    "description": "Scope, Maturity, and Granularity axes for categorizing feedback",
                    "mimeType": "text/markdown"
                }
            ]
        })),

        "resources/read" => {
            let params = params.ok_or_else(|| "Missing params for resources/read".to_owned())?;
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing resource uri".to_owned())?;

            match uri {
                "tutor://destinations" => Ok(json!({
                    "contents": [
                        {
                            "uri": uri,
                            "mimeType": "text/markdown",
                            "text": destinations_guide_text()
                        }
                    ]
                })),
                "tutor://guidelines" => Ok(json!({
                    "contents": [
                        {
                            "uri": uri,
                            "mimeType": "text/markdown",
                            "text": guidelines_text()
                        }
                    ]
                })),
                _ => Err(format!("Unknown resource URI: {uri}")),
            }
        }

        _ => Err(format!("Unknown method: {method}")),
    }
}

/// Read an optional `YYYY-MM-DD` string argument. A present but non-string, empty or
/// unparsable value is an error rather than a silent fallback.
fn date_arg(args: &Value, key: &str) -> Result<Option<NaiveDate>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let text = value
        .as_str()
        .ok_or_else(|| format!("'{key}' must be a string in YYYY-MM-DD format"))?;
    if text.is_empty() {
        return Err(format!("'{key}' must not be empty"));
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map(Some)
        .map_err(|err| format!("Invalid {key} '{text}': {err}"))
}

/// Read the optional `grep` argument: a non-blank string or a non-empty array of non-blank
/// strings. Anything else is an error rather than a silently ignored filter.
fn grep_arg(args: &Value) -> Result<Vec<String>, String> {
    let Some(value) = args.get("grep") else {
        return Ok(Vec::new());
    };
    let items = match value {
        Value::String(text) => vec![text.as_str()],
        Value::Array(values) => {
            if values.is_empty() {
                return Err("'grep' must not be an empty array".to_owned());
            }
            values
                .iter()
                .map(|item| {
                    item.as_str()
                        .ok_or_else(|| "'grep' array elements must be strings".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        _ => return Err("'grep' must be a string or an array of strings".to_owned()),
    };
    if items.iter().any(|text| text.trim().is_empty()) {
        return Err("'grep' values must not be empty or whitespace-only".to_owned());
    }
    Ok(items.into_iter().map(str::to_owned).collect())
}

/// Parse `date` / `since` / `until` tool arguments into one inclusive range.
fn date_range_arg(args: &Value, today: NaiveDate) -> Result<DateRange, String> {
    resolve_date_range(
        date_arg(args, "date")?,
        date_arg(args, "since")?,
        date_arg(args, "until")?,
        today,
    )
}

fn call_tool(name: &str, args: Value, warnings: &mut impl Write) -> Result<Value, String> {
    match name {
        "tutor_projects" => {
            let dates = date_range_arg(&args, Local::now().date_naive())?;

            let no_ledger = args
                .get("no_ledger")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let ledger_path = if no_ledger {
                None
            } else {
                Some(default_ledger_path())
            };

            let config = Config {
                dates,
                projects_dir: default_projects_dir(),
                boilerplate_threshold: 3,
                context_chars: 600,
                tools_window: 20,
                fixed_offset: None,
                ledger_path,
                project_filter: None,
                cwd_filter: None,
                human_filter: Vec::new(),
            };

            let projects = crate::list_projects(&config, warnings)
                .map_err(|err| format!("Failed to list projects: {err}"))?;

            let text = serde_json::to_string_pretty(&projects)
                .map_err(|err| format!("Failed to serialize projects: {err}"))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": text
                    }
                ]
            }))
        }

        "tutor_extract" => {
            let dates = date_range_arg(&args, Local::now().date_naive())?;

            let grep = grep_arg(&args)?;
            let project = args
                .get("project")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let no_ledger = args
                .get("no_ledger")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let ledger_path = if no_ledger {
                None
            } else {
                Some(default_ledger_path())
            };

            let format = match args.get("format").and_then(Value::as_str) {
                Some("md") => OutputFormat::Md,
                _ => OutputFormat::Jsonl,
            };

            let config = Config {
                dates,
                projects_dir: default_projects_dir(),
                boilerplate_threshold: 3,
                context_chars: 600,
                tools_window: 20,
                fixed_offset: None,
                ledger_path,
                project_filter: project,
                cwd_filter: None,
                human_filter: grep.clone(),
            };

            let turns =
                extract(&config, warnings).map_err(|err| format!("Extraction failed: {err}"))?;

            let mut output_bytes = Vec::new();
            write_output(&turns, format, &mut output_bytes)
                .map_err(|err| format!("Output format failed: {err}"))?;
            let output_text = String::from_utf8(output_bytes)
                .map_err(|err| format!("Failed to encode output string: {err}"))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": if output_text.is_empty() {
                            if grep.is_empty() {
                                format!("No unreviewed turns found for {dates}.")
                            } else {
                                let patterns = grep
                                    .iter()
                                    .map(|pattern| format!("{pattern:?}"))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!(
                                    "No unreviewed turns found for {dates} matching grep {patterns}."
                                )
                            }
                        } else {
                            output_text
                        }
                    }
                ]
            }))
        }

        "tutor_mark" => {
            let session = args
                .get("session")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing 'session' argument".to_owned())?;
            let turn_index = args
                .get("turn_index")
                .and_then(Value::as_u64)
                .ok_or_else(|| "Missing 'turn_index' argument".to_owned())?
                as usize;
            let date = args
                .get("date")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing 'date' argument (YYYY-MM-DD)".to_owned())?;
            parse_ledger_date(date).map_err(|err| format!("Invalid 'date' argument: {err}"))?;
            let status_str = args
                .get("status")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing 'status' argument".to_owned())?;

            let status = match status_str {
                "approved" => JudgmentStatus::Approved,
                "rejected" => JudgmentStatus::Rejected,
                "deferred" => JudgmentStatus::Deferred,
                other => {
                    return Err(format!(
                        "Invalid status '{other}'. Expected 'approved', 'rejected', or 'deferred'."
                    ));
                }
            };

            let destination = args
                .get("destination")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned);

            let ledger_path = default_ledger_path();
            let mut ledger = Ledger::load(&ledger_path)
                .map_err(|err| format!("Failed to load ledger: {err}"))?;

            let entry = LedgerEntry {
                session: session.to_owned(),
                turn_index,
                date: Some(date.to_owned()),
                status,
                destination,
                reason,
                timestamp: Some(Local::now().to_rfc3339()),
            };

            let key = Ledger::entry_key(&entry.session, date, entry.turn_index);
            ledger.record(entry);
            ledger
                .save(&ledger_path)
                .map_err(|err| format!("Failed to save ledger: {err}"))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!("Successfully recorded judgment for {key} ({status_str}) into ledger.")
                    }
                ]
            }))
        }

        "tutor_destinations" => {
            let destinations = crate::load_or_init_destinations(None, None, warnings)
                .map_err(|err| format!("Failed to load destinations: {err}"))?;
            let markdown = format!(
                "{}\n\n{}",
                crate::format_destinations_markdown(&destinations),
                guidelines_text()
            );
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": markdown
                    }
                ]
            }))
        }

        "tutor_ledger" => {
            let ledger_path = default_ledger_path();
            let ledger = Ledger::load(&ledger_path)
                .map_err(|err| format!("Failed to load ledger: {err}"))?;

            let text = serde_json::to_string_pretty(&ledger)
                .map_err(|err| format!("Failed to serialize ledger: {err}"))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": text
                    }
                ]
            }))
        }

        "tutor_review_ui" => {
            let items_val = args
                .get("items")
                .ok_or_else(|| "Missing 'items' argument".to_owned())?;
            let items: Vec<crate::ui::ReviewItem> = serde_json::from_value(items_val.clone())
                .map_err(|err| format!("Invalid 'items' array: {err}"))?;
            // Checked here too so `no_ledger` calls get the same early, clear error.
            let undated = crate::ui::undated_items(&items);
            if !undated.is_empty() {
                return Err(format!(
                    "Items need a valid 'date' (YYYY-MM-DD, from tutor_extract) or a 'timestamp_local'; missing or invalid for: {}",
                    undated.join(", ")
                ));
            }
            let port = args.get("port").and_then(Value::as_u64).map(|p| p as u16);
            let no_ledger = args
                .get("no_ledger")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let ledger_path = if no_ledger {
                None
            } else {
                Some(default_ledger_path())
            };

            let destinations = crate::load_or_init_destinations(None, None, warnings)
                .map_err(|err| format!("Failed to load destinations: {err}"))?;

            let finalized = crate::ui::run_review_ui(
                items,
                port,
                true,
                ledger_path.as_deref(),
                Some(destinations),
            )
            .map_err(|err| format!("Review UI error: {err}"))?;

            let text = serde_json::to_string_pretty(&finalized)
                .map_err(|err| format!("Failed to serialize finalized items: {err}"))?;

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": text
                    }
                ]
            }))
        }

        _ => Err(format!("Unknown tool: {name}")),
    }
}

fn destinations_guide_text() -> String {
    r#"# Tutor Canonical Destinations

| Destination ID | Target Path / Medium | Scope & Purpose | Recommended Apply Method |
| :--- | :--- | :--- | :--- |
| `global_claude` | `~/.claude/CLAUDE.md` | Common behavioral norms, tool preferences, individual habits across all projects | File append / edit |
| `global_agents` | `~/.config/rules/AGENTS.md` | Common agent behavioral norms, individual rules across all projects | File append / edit |
| `project_claude` | `<cwd>/CLAUDE.md` | Repo-specific development commands, architecture rules, code conventions | File append / edit |
| `project_agents` | `<cwd>/AGENTS.md` | Common agent behavioral norms, repo-specific constraints & rules | File append / edit |
| `project_docs` | `<cwd>/docs/**/*.md` | Detailed specifications, module conventions (Source of Truth) | File edit / Git branch + PR |
| `project_instructions` | `<cwd>/.github/instructions/*.md` | File-pattern or domain-specific granular instructions | File edit |
| `copilot_instructions` | `<cwd>/.github/copilot-instructions.md` | GitHub Copilot instructions | File edit |
| `user_memory` | `~/.claude/projects/<slug>/memory/` | Transitory observations, preliminary feedback, personal preferences | Memory file creation |
| `workflow_skill` | `.claude/skills/<name>/SKILL.md` | Multi-step recurring workflows, complex CLI orchestration | Skill creation |
| `knowledge_lk` | `lk` MCP (`lk-knowledge/add_knowledge`) | Cross-repository technical knowledge and investigation notes | MCP call |
| `discard` | (Ledger only) | Ephemeral prompt, routine confirmation, or chat not requiring rules | Mark ledger only |
"#
    .to_owned()
}

fn guidelines_text() -> String {
    r#"# Decision Axes for Feedback Triaging

1. **Scope Axis: Global vs Project**
   - Personal habits, universal tool settings, user interaction preferences -> `global_claude` / `global_agents`
   - Repository-specific conventions, architecture rules, tech stack rules -> `project_claude` / `project_agents` / `project_*`

2. **Maturity Axis: Memory vs Rule vs Skill**
   - First occurrence, tentative observation, individual feedback -> `user_memory`
   - Recurring pattern (2+ times), strict constraint, definite requirement -> `CLAUDE.md` / `AGENTS.md` / `project_docs`
   - Multi-command procedure, structured workflow -> `workflow_skill`

3. **Granularity Axis: Core vs Reference**
   - Essential norms always present in context (keep concise, <200 lines) -> `CLAUDE.md` / `AGENTS.md`
   - Deep reference loaded on demand for specific files -> `project_docs` / `.github/instructions`
"#
    .to_owned()
}

pub fn generate_review_prompt(date_str: &str, lang: &str) -> String {
    if lang == "en" {
        format!(
            "Please triage human feedback from Claude Code conversation logs for {date_str}.\n\n\
            Follow this structured workflow:\n\
            1. Call `tutor_projects` with date=\"{date_str}\" to list repositories with unreviewed turns, and ask the user which project to triage (or triage all).\n\
            2. Call `tutor_destinations` to inspect available destination layers (conventions, documentation, issues/tasks, knowledge) for this environment.\n\
            3. Call `tutor_extract` for the selected project to fetch unreviewed human turns. Each turn has a `date` (local date) and a `turn_index` that is numbered per date.\n\
            4. If no turns are returned, notify that all feedback has been reviewed and finish.\n\
            5. For each turn, analyze the feedback and choose the best destination:\n\
               - For conventions/docs: produce a unified diff draft\n\
               - For issue/task management (e.g. GitHub Issues, Linear): produce an issue title and markdown body draft\n\
               - For knowledge tools (e.g. lk, Notion): produce a structured knowledge entry draft\n\
            6. Call `tutor_review_ui` with the array of items to launch the Web Review UI in the browser for user confirmation. Copy `session`, `turn_index` and `date` from the extracted turn into each item unchanged.\n\
            7. Once the user submits in the UI, apply all approved changes according to their destination type:\n\
               - Files: edit or append to files\n\
               - GitHub Issues: run `gh issue create --title ... --body ...`\n\
               - Linear / MCP tools: invoke the appropriate MCP tool (e.g. Linear MCP, lk MCP)\n\
            8. Report the applied changes, created issues, and knowledge entries to the user."
        )
    } else {
        format!(
            "Claude Code の会話ログから人間の指摘を回収し、規約・ドキュメント・Issue/タスク・ナレッジへの反映案を提示してください。対象日: {date_str}\n\n\
            以下の手順に沿って自律的に実行してください：\n\
            1. `tutor_projects`（date: \"{date_str}\"）を呼び出し、会話ログが存在するリポジトリ・プロジェクト一覧と未レビュー件数をユーザーに提示して、どのリポジトリを対象にするか確認する（全件も可）。\n\
            2. `tutor_destinations` を呼び出して、この環境で利用可能な宛先一覧（規約・ドキュメント・GitHub Issue / Linear などのタスク管理・ナレッジ）を確認する。\n\
            3. 選択されたプロジェクトの未判定ターンを `tutor_extract`（project: ..., date: \"{date_str}\"）で取得する。各ターンには `date`（ローカル日付）と、日付ごとに採番された `turn_index` が含まれる。\n\
            4. 取得結果が0件の場合は、「対象リポジトリの未判定な指摘はありませんでした」と報告して終了する。\n\
            5. 各指摘について最適な宛先（規約、docs、GitHub Issue、Linear、ナレッジ等）を選び、提案宛先、理由、ドラフトを生成する：\n\
               - 規約・ドキュメントの場合: 具体的な【適用差分ドラフト（diff）】\n\
               - GitHub Issue / Linear などの場合: 【起票ドラフト（タイトル・本文）】\n\
               - ナレッジの場合: 【知見ドラフト（タイトル・内容）】\n\
            6. 生成したアイテム配列を渡して `tutor_review_ui` を呼び出し、ブラウザで Web レビュー UI を起動する。各アイテムの `session`・`turn_index`・`date` は、抽出されたターンの値をそのまま渡す。\n\
            7. ユーザーがブラウザで確定（Submit）したら、返却された結果のうち承認（approved）されたものを各宛先に応じた方法で反映する（台帳は Web UI 側で自動記録済み）：\n\
               - ファイル宛先: ファイルに書き込み・編集\n\
               - GitHub Issue: `gh issue create --title ... --body ...` を実行して起票\n\
               - Linear などの MCP ツール: 対応する MCP ツールを呼び出して起票\n\
               8. 反映したファイル、起票した Issue/タスク、登録した知見の一覧をユーザーに報告して完了する。"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
    }

    #[test]
    fn date_range_arg_defaults_to_today() {
        assert_eq!(
            date_range_arg(&json!({}), d(20)).unwrap(),
            DateRange::single(d(20))
        );
    }

    #[test]
    fn date_range_arg_accepts_date_and_ranges() {
        assert_eq!(
            date_range_arg(&json!({"date": "2026-09-05"}), d(20)).unwrap(),
            DateRange::single(d(5))
        );
        assert_eq!(
            date_range_arg(&json!({"since": "2026-09-05"}), d(20)).unwrap(),
            DateRange::new(d(5), d(20)).unwrap()
        );
        assert_eq!(
            date_range_arg(
                &json!({"since": "2026-09-05", "until": "2026-09-07"}),
                d(20)
            )
            .unwrap(),
            DateRange::new(d(5), d(7)).unwrap()
        );
    }

    #[test]
    fn date_range_arg_rejects_bad_values() {
        for args in [
            json!({"date": 20260905}),
            json!({"since": null}),
            json!({"until": true}),
            json!({"date": ""}),
            json!({"since": ""}),
            json!({"date": "not-a-date"}),
            json!({"since": "2026-13-01"}),
        ] {
            assert!(date_range_arg(&args, d(20)).is_err(), "{args}");
        }
    }

    #[test]
    fn date_range_arg_rejects_conflicts() {
        for args in [
            json!({"date": "2026-09-05", "since": "2026-09-01"}),
            json!({"date": "2026-09-05", "until": "2026-09-07"}),
            json!({"until": "2026-09-07"}),
            json!({"since": "2026-09-07", "until": "2026-09-05"}),
        ] {
            assert!(date_range_arg(&args, d(20)).is_err(), "{args}");
        }
    }

    #[test]
    fn grep_arg_accepts_string_and_array() {
        assert!(grep_arg(&json!({})).unwrap().is_empty());
        assert_eq!(grep_arg(&json!({"grep": "foo"})).unwrap(), ["foo"]);
        assert_eq!(
            grep_arg(&json!({"grep": ["foo", "Bar baz"]})).unwrap(),
            ["foo", "Bar baz"]
        );
    }

    #[test]
    fn grep_arg_rejects_bad_values() {
        for args in [
            json!({"grep": null}),
            json!({"grep": 1}),
            json!({"grep": true}),
            json!({"grep": {}}),
            json!({"grep": []}),
            json!({"grep": ""}),
            json!({"grep": "  "}),
            json!({"grep": ["ok", ""]}),
            json!({"grep": ["ok", " \t"]}),
            json!({"grep": ["ok", 1]}),
            json!({"grep": [null]}),
        ] {
            assert!(grep_arg(&args).is_err(), "{args}");
        }
    }
}
