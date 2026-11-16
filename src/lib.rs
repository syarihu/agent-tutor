use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub mod destinations;
pub mod mcp;
pub mod ui;

pub use destinations::{
    ApplyType, DestinationCategory, DestinationDef, detect_destinations,
    format_destinations_markdown, load_or_init_destinations,
};
pub use ui::{ReviewItem, run_review_ui};

use chrono::{DateTime, FixedOffset, Local, NaiveDate, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum JudgmentStatus {
    Approved,
    Rejected,
    Deferred,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub session: String,
    pub turn_index: usize,
    pub status: JudgmentStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub entries: HashMap<String, LedgerEntry>,
}

impl Ledger {
    pub fn entry_key(session: &str, turn_index: usize) -> String {
        format!("{session}:{turn_index}")
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let ledger: Self = serde_json::from_reader(reader).map_err(io::Error::other)?;
        Ok(ledger)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::create(path)?;
        serde_json::to_writer_pretty(file, self).map_err(io::Error::other)?;
        Ok(())
    }

    pub fn is_reviewed(&self, session: &str, turn_index: usize) -> bool {
        let key = Self::entry_key(session, turn_index);
        self.entries.get(&key).is_some_and(|entry| {
            matches!(
                entry.status,
                JudgmentStatus::Approved | JudgmentStatus::Rejected
            )
        })
    }

    pub fn record(&mut self, entry: LedgerEntry) {
        let key = Self::entry_key(&entry.session, entry.turn_index);
        self.entries.insert(key, entry);
    }
}

/// Inclusive range of local calendar dates.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DateRange {
    since: NaiveDate,
    until: NaiveDate,
}

impl DateRange {
    pub fn new(since: NaiveDate, until: NaiveDate) -> Result<Self, String> {
        if since > until {
            return Err(format!(
                "since ({since}) must not be later than until ({until})"
            ));
        }
        Ok(Self { since, until })
    }

    pub fn single(date: NaiveDate) -> Self {
        Self {
            since: date,
            until: date,
        }
    }

    pub fn since(&self) -> NaiveDate {
        self.since
    }

    pub fn until(&self) -> NaiveDate {
        self.until
    }

    pub fn contains(&self, date: NaiveDate) -> bool {
        self.since <= date && date <= self.until
    }

    pub fn is_single_day(&self) -> bool {
        self.since == self.until
    }
}

impl fmt::Display for DateRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_single_day() {
            write!(f, "{}", self.since)
        } else {
            write!(f, "{}..{}", self.since, self.until)
        }
    }
}

/// Combine the `date` / `since` / `until` options into one range.
///
/// Nothing given means today; `since` alone runs through `today`; `date` cannot be combined
/// with `since` or `until`; `until` needs `since`.
pub fn resolve_date_range(
    date: Option<NaiveDate>,
    since: Option<NaiveDate>,
    until: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<DateRange, String> {
    match (date, since, until) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            Err("date cannot be combined with since or until".to_owned())
        }
        (None, None, Some(_)) => Err("until requires since".to_owned()),
        (Some(date), None, None) => Ok(DateRange::single(date)),
        (None, None, None) => Ok(DateRange::single(today)),
        (None, Some(since), until) => DateRange::new(since, until.unwrap_or(today)),
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub dates: DateRange,
    pub projects_dir: PathBuf,
    pub boilerplate_threshold: usize,
    pub context_chars: usize,
    pub tools_window: usize,
    /// Intended for deterministic tests. Production uses the machine's local timezone.
    pub fixed_offset: Option<FixedOffset>,
    /// Optional ledger file tracking reviewed turns. Turns present with terminal status are skipped.
    pub ledger_path: Option<PathBuf>,
    /// Optional filter by project directory slug or substring.
    pub project_filter: Option<String>,
    /// Optional filter by working directory path.
    pub cwd_filter: Option<PathBuf>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub project_dir: String,
    pub cwd: String,
    pub turns_count: usize,
    /// Per-day turn counts keyed by "YYYY-MM-DD". Present only for multi-day ranges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_counts: Option<BTreeMap<String, usize>>,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Jsonl,
    Md,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct OutputTurn {
    pub session: String,
    pub project_dir: String,
    pub cwd: String,
    pub turn_index: usize,
    pub timestamp_local: String,
    pub human: String,
    pub prev_assistant: String,
    pub prev_tools: Vec<String>,
    pub already_captured: bool,
}

#[derive(Debug, Clone)]
struct Record {
    uuid: Option<String>,
    parent_uuid: Option<String>,
    session: Option<String>,
    assistant: Option<AssistantData>,
    is_human: bool,
}

#[derive(Debug, Clone, Default)]
struct AssistantData {
    text: String,
    tools: Vec<String>,
    captures_guidance: bool,
}

#[derive(Debug, Clone)]
struct Candidate {
    record_index: usize,
    session: String,
    cwd: String,
    local_date: NaiveDate,
    timestamp_local: String,
    human: String,
    parent_uuid: Option<String>,
}

#[derive(Debug)]
struct SessionFile {
    project_dir: String,
    records: Vec<Record>,
    candidates: Vec<Candidate>,
}

/// Extract all matching turns. Malformed transcript lines are reported to `warnings` and skipped.
pub fn extract(config: &Config, warnings: &mut impl Write) -> io::Result<Vec<OutputTurn>> {
    let Loaded { files, boilerplate } = load_files(config, warnings)?;

    let multi_day = !config.dates.is_single_day();
    let mut removed: Vec<(NaiveDate, &String, usize)> = boilerplate
        .iter()
        .flat_map(|(date, prompts)| {
            prompts
                .iter()
                .map(move |(prompt, count)| (*date, prompt, *count))
        })
        .collect();
    removed.sort_by(|left, right| left.1.cmp(right.1).then(left.0.cmp(&right.0)));
    for (date, prompt, count) in removed {
        let rendered = serde_json::to_string(prompt).unwrap_or_else(|_| "<unprintable>".into());
        if multi_day {
            writeln!(
                warnings,
                "tutor: excluded boilerplate seen in {count} distinct sessions on {date}: {rendered}"
            )?;
        } else {
            writeln!(
                warnings,
                "tutor: excluded boilerplate seen in {count} distinct sessions: {rendered}"
            )?;
        }
    }

    let ledger = config
        .ledger_path
        .as_deref()
        .map(Ledger::load)
        .transpose()?;

    let mut output = Vec::new();
    for file in files {
        if !file
            .candidates
            .iter()
            .any(|candidate| candidate_matches(config, &file, candidate))
        {
            continue;
        }

        let uuid_index: HashMap<&str, usize> = file
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| record.uuid.as_deref().map(|uuid| (uuid, index)))
            .collect();
        for (turn_index, candidate) in file.numbered_candidates(&boilerplate) {
            if !candidate_matches(config, &file, candidate) {
                continue;
            }

            if let Some(ledger) = &ledger
                && ledger.is_reviewed(&candidate.session, turn_index)
            {
                continue;
            }

            let previous = previous_turn_context(
                &file.records,
                &uuid_index,
                &candidate.session,
                candidate.parent_uuid.as_deref(),
                config.tools_window,
            );
            let prev_assistant = truncate_chars_from_end(&previous.text, config.context_chars);
            let prev_tools = collapse_adjacent_tools(previous.tools);

            output.push(OutputTurn {
                session: candidate.session.clone(),
                project_dir: file.project_dir.clone(),
                cwd: candidate.cwd.clone(),
                turn_index,
                timestamp_local: candidate.timestamp_local.clone(),
                human: candidate.human.clone(),
                prev_assistant,
                prev_tools,
                already_captured: captured_before_next_human(&file.records, candidate.record_index),
            });
        }
    }

    Ok(output)
}

/// List all projects with unreviewed human turns in the configured date range.
///
/// When the range spans more than one day, each project also carries per-day counts.
pub fn list_projects(config: &Config, warnings: &mut impl Write) -> io::Result<Vec<ProjectInfo>> {
    let Loaded { files, boilerplate } = load_files(config, warnings)?;

    let ledger = config
        .ledger_path
        .as_deref()
        .map(Ledger::load)
        .transpose()?;

    let mut project_counts: HashMap<(String, String), BTreeMap<NaiveDate, usize>> = HashMap::new();

    for file in files {
        for (turn_index, candidate) in file.numbered_candidates(&boilerplate) {
            if let Some(ledger) = &ledger
                && ledger.is_reviewed(&candidate.session, turn_index)
            {
                continue;
            }

            *project_counts
                .entry((file.project_dir.clone(), candidate.cwd.clone()))
                .or_default()
                .entry(candidate.local_date)
                .or_insert(0) += 1;
        }
    }

    let multi_day = !config.dates.is_single_day();
    let mut results: Vec<ProjectInfo> = project_counts
        .into_iter()
        .map(|((project_dir, cwd), per_day)| ProjectInfo {
            project_dir,
            cwd,
            turns_count: per_day.values().sum(),
            daily_counts: multi_day.then(|| {
                per_day
                    .into_iter()
                    .map(|(date, count)| (date.to_string(), count))
                    .collect()
            }),
        })
        .collect();

    results.sort_by(|a, b| {
        b.turns_count
            .cmp(&a.turns_count)
            .then_with(|| a.project_dir.cmp(&b.project_dir))
    });

    Ok(results)
}

/// Parsed transcripts plus the prompts treated as boilerplate on each local date.
struct Loaded {
    files: Vec<SessionFile>,
    /// local date -> prompt -> number of distinct sessions that sent it that day.
    /// Only prompts at or above `boilerplate_threshold` are kept.
    boilerplate: HashMap<NaiveDate, HashMap<String, usize>>,
}

fn load_files(config: &Config, warnings: &mut impl Write) -> io::Result<Loaded> {
    let mut paths = Vec::new();
    collect_jsonl_files(&config.projects_dir, &mut paths)?;
    paths.sort();

    let mut files = Vec::new();
    let mut prompt_sessions: HashMap<(NaiveDate, String), HashSet<String>> = HashMap::new();

    for path in paths {
        let session_file = parse_file(&path, config, warnings)?;
        for candidate in &session_file.candidates {
            prompt_sessions
                .entry((candidate.local_date, candidate.human.clone()))
                .or_default()
                .insert(candidate.session.clone());
        }
        files.push(session_file);
    }

    let mut boilerplate: HashMap<NaiveDate, HashMap<String, usize>> = HashMap::new();
    for ((date, prompt), sessions) in prompt_sessions {
        if sessions.len() >= config.boilerplate_threshold {
            boilerplate
                .entry(date)
                .or_default()
                .insert(prompt, sessions.len());
        }
    }

    Ok(Loaded { files, boilerplate })
}

impl SessionFile {
    /// Non-boilerplate candidates with their 1-based turn index, numbered per local date so a
    /// range yields the same ledger keys as running each day on its own.
    fn numbered_candidates<'a>(
        &'a self,
        boilerplate: &HashMap<NaiveDate, HashMap<String, usize>>,
    ) -> Vec<(usize, &'a Candidate)> {
        let mut per_date: HashMap<NaiveDate, usize> = HashMap::new();
        self.candidates
            .iter()
            .filter(|candidate| {
                !boilerplate
                    .get(&candidate.local_date)
                    .is_some_and(|prompts| prompts.contains_key(&candidate.human))
            })
            .map(|candidate| {
                let counter = per_date.entry(candidate.local_date).or_insert(0);
                *counter += 1;
                (*counter, candidate)
            })
            .collect()
    }
}

/// Per-candidate filters for `extract`. Add new filters here.
fn candidate_matches(config: &Config, file: &SessionFile, candidate: &Candidate) -> bool {
    if let Some(filter) = &config.project_filter
        && !file.project_dir.contains(filter)
        && !candidate.cwd.contains(filter)
    {
        return false;
    }
    if let Some(cwd) = &config.cwd_filter
        && Path::new(&candidate.cwd) != cwd
    {
        return false;
    }
    true
}

pub fn write_output(
    turns: &[OutputTurn],
    format: OutputFormat,
    writer: &mut impl Write,
) -> io::Result<()> {
    match format {
        OutputFormat::Jsonl => {
            for turn in turns {
                serde_json::to_writer(&mut *writer, turn).map_err(io::Error::other)?;
                writeln!(writer)?;
            }
        }
        OutputFormat::Md => write_markdown(turns, writer)?,
    }
    Ok(())
}

fn collect_jsonl_files(directory: &Path, output: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_jsonl_files(&entry.path(), output)?;
        } else if file_type.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
        {
            output.push(entry.path());
        }
    }
    Ok(())
}

fn parse_file(path: &Path, config: &Config, warnings: &mut impl Write) -> io::Result<SessionFile> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let project_dir = project_dir_for(path, &config.projects_dir);
    let mut records = Vec::new();
    let mut candidates = Vec::new();

    for (line_index, line) in reader.lines().enumerate() {
        let line_number = line_index + 1;
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                writeln!(
                    warnings,
                    "tutor: warning: {}:{line_number}: could not read line: {error}",
                    path.display()
                )?;
                continue;
            }
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                writeln!(
                    warnings,
                    "tutor: warning: {}:{line_number}: invalid JSON: {error}",
                    path.display()
                )?;
                continue;
            }
        };

        let kind = string_field(&value, "type").unwrap_or_default().to_owned();
        let is_human = structurally_human(&value);
        let assistant = (kind == "assistant").then(|| parse_assistant(&value));
        let record_index = records.len();
        let parent_uuid = optional_string_field(&value, "parentUuid");

        if is_human
            && let Some(human) = value.pointer("/message/content").and_then(Value::as_str)
            && let Some(timestamp) = string_field(&value, "timestamp")
            && let Some((local_date, timestamp_local)) = local_timestamp_in_range(timestamp, config)
            && let Some(session) = string_field(&value, "sessionId")
        {
            candidates.push(Candidate {
                record_index,
                session: session.to_owned(),
                cwd: string_field(&value, "cwd").unwrap_or_default().to_owned(),
                local_date,
                timestamp_local,
                human: human.to_owned(),
                parent_uuid: parent_uuid.clone(),
            });
        }

        records.push(Record {
            uuid: optional_string_field(&value, "uuid"),
            parent_uuid,
            session: optional_string_field(&value, "sessionId"),
            assistant,
            is_human,
        });
    }

    Ok(SessionFile {
        project_dir,
        records,
        candidates,
    })
}

fn structurally_human(value: &Value) -> bool {
    string_field(value, "type") == Some("user")
        && value
            .pointer("/message/content")
            .is_some_and(Value::is_string)
        && value.pointer("/origin/kind").and_then(Value::as_str) == Some("human")
        && value.get("isSidechain").and_then(Value::as_bool) != Some(true)
}

fn parse_assistant(value: &Value) -> AssistantData {
    let mut text_blocks = Vec::new();
    let mut tools = Vec::new();
    let mut captures_guidance = false;

    if let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) {
        for block in blocks {
            match string_field(block, "type") {
                Some("text") => {
                    if let Some(text) = string_field(block, "text") {
                        text_blocks.push(text.to_owned());
                    }
                }
                Some("tool_use") => {
                    let Some(name) = string_field(block, "name") else {
                        continue;
                    };
                    if matches!(name, "Write" | "Edit")
                        && block
                            .pointer("/input/file_path")
                            .and_then(Value::as_str)
                            .is_some_and(is_guidance_path)
                    {
                        captures_guidance = true;
                    }
                    if name == "Bash" {
                        let detail = block
                            .pointer("/input/description")
                            .and_then(Value::as_str)
                            .or_else(|| {
                                block
                                    .pointer("/input/command")
                                    .and_then(Value::as_str)
                                    .and_then(|command| command.lines().next())
                            })
                            .unwrap_or_default();
                        if detail.is_empty() {
                            tools.push(name.to_owned());
                        } else {
                            tools.push(format!("Bash({})", truncate_chars(detail, 60)));
                        }
                    } else {
                        tools.push(name.to_owned());
                    }
                }
                _ => {}
            }
        }
    }

    AssistantData {
        text: text_blocks.join("\n"),
        tools,
        captures_guidance,
    }
}

fn is_guidance_path(path: &str) -> bool {
    path.contains("/memory/") || path.ends_with("CLAUDE.md") || path.ends_with("AGENTS.md")
}

fn local_timestamp_in_range(timestamp: &str, config: &Config) -> Option<(NaiveDate, String)> {
    let utc: DateTime<Utc> = DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .with_timezone(&Utc);
    if let Some(offset) = config.fixed_offset {
        let local = utc.with_timezone(&offset);
        let date = local.date_naive();
        config
            .dates
            .contains(date)
            .then(|| (date, local.to_rfc3339()))
    } else {
        let local = utc.with_timezone(&Local);
        let date = local.date_naive();
        config
            .dates
            .contains(date)
            .then(|| (date, local.to_rfc3339()))
    }
}

fn previous_turn_context<'a>(
    records: &'a [Record],
    uuid_index: &HashMap<&str, usize>,
    session: &str,
    mut parent_uuid: Option<&'a str>,
    tools_window: usize,
) -> AssistantData {
    let mut assistants = Vec::new();
    let mut visited = HashSet::new();

    while let Some(uuid) = parent_uuid {
        if !visited.insert(uuid) {
            break;
        }
        let Some(record) = uuid_index.get(uuid).and_then(|index| records.get(*index)) else {
            break;
        };
        if record.session.as_deref() != Some(session) || record.is_human {
            break;
        }
        if let Some(assistant) = &record.assistant {
            if assistants.len() == tools_window {
                break;
            }
            assistants.push(assistant);
        }
        parent_uuid = record.parent_uuid.as_deref();
    }

    assistants.reverse();
    AssistantData {
        text: assistants
            .iter()
            .filter(|assistant| !assistant.text.is_empty())
            .map(|assistant| assistant.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        tools: assistants
            .into_iter()
            .flat_map(|assistant| assistant.tools.iter().cloned())
            .collect(),
        captures_guidance: false,
    }
}

fn captured_before_next_human(records: &[Record], candidate_index: usize) -> bool {
    for record in records.iter().skip(candidate_index + 1) {
        if record.is_human {
            break;
        }
        if record
            .assistant
            .as_ref()
            .is_some_and(|assistant| assistant.captures_guidance)
        {
            return true;
        }
    }
    false
}

fn collapse_adjacent_tools(tools: Vec<String>) -> Vec<String> {
    let mut collapsed = Vec::new();
    let mut tools = tools.into_iter();
    let Some(mut current) = tools.next() else {
        return collapsed;
    };
    let mut count = 1;

    for tool in tools {
        if tool == current {
            count += 1;
        } else {
            collapsed.push(render_tool_count(current, count));
            current = tool;
            count = 1;
        }
    }
    collapsed.push(render_tool_count(current, count));
    collapsed
}

fn render_tool_count(tool: String, count: usize) -> String {
    if count == 1 {
        tool
    } else {
        format!("{tool} ×{count}")
    }
}

fn truncate_chars_from_end(text: &str, limit: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= limit {
        text.to_owned()
    } else {
        let suffix: String = text.chars().skip(char_count - limit).collect();
        format!("…{suffix}")
    }
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(limit).collect::<String>())
    }
}

fn project_dir_for(path: &Path, projects_dir: &Path) -> String {
    path.strip_prefix(projects_dir)
        .ok()
        .and_then(|relative| relative.components().next())
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .or_else(|| {
            path.parent()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

fn optional_string_field(value: &Value, name: &str) -> Option<String> {
    string_field(value, name).map(str::to_owned)
}

fn string_field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value.get(name).and_then(Value::as_str)
}

fn write_markdown(turns: &[OutputTurn], writer: &mut impl Write) -> io::Result<()> {
    let mut previous_session: Option<&str> = None;
    for turn in turns {
        if previous_session != Some(&turn.session) {
            if previous_session.is_some() {
                writeln!(writer)?;
            }
            writeln!(writer, "# Session `{}`", turn.session)?;
            writeln!(writer)?;
            writeln!(writer, "- Project: `{}`", turn.project_dir)?;
            writeln!(writer, "- Working directory: `{}`", turn.cwd)?;
            previous_session = Some(&turn.session);
        }
        writeln!(writer)?;
        writeln!(
            writer,
            "## Turn {} — {}",
            turn.turn_index, turn.timestamp_local
        )?;
        writeln!(writer)?;
        writeln!(writer, "**Human**")?;
        writeln!(writer)?;
        write_blockquote(writer, &turn.human)?;
        writeln!(writer)?;
        writeln!(writer, "**Previous assistant**")?;
        writeln!(writer)?;
        if turn.prev_assistant.is_empty() {
            writeln!(writer, "_(none)_")?;
        } else {
            write_blockquote(writer, &turn.prev_assistant)?;
        }
        writeln!(writer)?;
        let tools = if turn.prev_tools.is_empty() {
            "_(none)_".to_owned()
        } else {
            turn.prev_tools
                .iter()
                .map(|tool| format!("`{tool}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        writeln!(writer, "- Previous tools: {tools}")?;
        writeln!(writer, "- Already captured: `{}`", turn.already_captured)?;
    }
    Ok(())
}

fn write_blockquote(writer: &mut impl Write, text: &str) -> io::Result<()> {
    for line in text.lines() {
        writeln!(writer, "> {line}")?;
    }
    if text.is_empty() {
        writeln!(writer, ">")?;
    }
    Ok(())
}
