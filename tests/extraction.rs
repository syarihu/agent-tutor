use std::fs;
use std::path::{Path, PathBuf};

use chrono::{FixedOffset, NaiveDate};
use tutor::{
    Config, DateRange, JudgmentStatus, Ledger, LedgerEntry, OutputFormat, extract, list_projects,
    resolve_date_range, write_output,
};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn config(projects_dir: PathBuf, offset_seconds: i32) -> Config {
    Config {
        dates: DateRange::single(NaiveDate::from_ymd_opt(2026, 9, 17).unwrap()),
        projects_dir,
        boilerplate_threshold: 3,
        context_chars: 600,
        tools_window: 20,
        fixed_offset: Some(FixedOffset::east_opt(offset_seconds).unwrap()),
        ledger_path: None,
        project_filter: None,
        cwd_filter: None,
    }
}

fn main_fixtures() -> (Vec<tutor::OutputTurn>, String) {
    let mut warnings = Vec::new();
    let turns = extract(&config(fixture_root(), 9 * 60 * 60), &mut warnings).unwrap();
    (turns, String::from_utf8(warnings).unwrap())
}

#[test]
fn excludes_non_human_string_array_missing_origin_and_sidechain_records() {
    let (turns, _) = main_fixtures();
    let text: Vec<_> = turns.iter().map(|turn| turn.human.as_str()).collect();

    assert!(!text.contains(&"<system-reminder>do work</system-reminder>"));
    assert!(!text.contains(&"injected prompt"));
    assert!(!text.contains(&"sidechain prompt"));
    assert_eq!(turns.len(), 4);
}

#[test]
fn converts_utc_across_both_local_date_boundaries() {
    let (turns, _) = main_fixtures();
    let positive = turns
        .iter()
        .find(|turn| turn.human == "fix the typo in the header")
        .unwrap();
    assert_eq!(positive.timestamp_local, "2026-09-17T00:30:00+09:00");

    let mut warnings = Vec::new();
    let negative_dir = fixture_root().join("timezone-negative");
    let negative = extract(&config(negative_dir, -7 * 60 * 60), &mut warnings).unwrap();
    assert_eq!(negative.len(), 1);
    assert_eq!(negative[0].timestamp_local, "2026-09-17T19:30:00-07:00");
}

#[test]
fn removes_prompts_seen_in_three_sessions_but_keeps_two() {
    let (turns, warnings) = main_fixtures();
    assert!(
        turns
            .iter()
            .all(|turn| turn.human != "AUTOMATED START PROMPT")
    );
    assert_eq!(
        turns
            .iter()
            .filter(|turn| turn.human == "prompt shared by two sessions")
            .count(),
        2
    );
    assert!(warnings.contains("excluded boilerplate seen in 3 distinct sessions"));
    assert!(warnings.contains("AUTOMATED START PROMPT"));
}

#[test]
fn follows_parent_links_through_attachment_and_tool_result() {
    let (turns, _) = main_fixtures();
    let turn = turns
        .iter()
        .find(|turn| turn.human == "fix the typo in the header")
        .unwrap();

    assert_eq!(
        turn.prev_assistant,
        "I inspected the requested area and found the header issue."
    );
    assert_eq!(turn.prev_tools, ["Bash(git status)", "Write"]);
}

#[test]
fn detects_memory_write_but_not_unrelated_write() {
    let (turns, _) = main_fixtures();
    let captured = turns
        .iter()
        .find(|turn| turn.human == "fix the typo in the header")
        .unwrap();
    let unrelated = turns
        .iter()
        .find(|turn| {
            turn.human == "prompt shared by two sessions" && turn.session == "test-session-1"
        })
        .unwrap();

    assert!(captured.already_captured);
    assert!(!unrelated.already_captured);
}

#[test]
fn numbers_turns_after_boilerplate_removal() {
    let (turns, _) = main_fixtures();
    let session_one: Vec<_> = turns
        .iter()
        .filter(|turn| turn.session == "test-session-1")
        .collect();

    assert_eq!(session_one.len(), 3);
    assert_eq!(session_one[0].turn_index, 1);
    assert_eq!(session_one[1].turn_index, 2);
    assert_eq!(session_one[2].turn_index, 3);
}

#[test]
fn warns_on_malformed_json_and_continues() {
    let (turns, warnings) = main_fixtures();
    assert!(warnings.contains("invalid JSON"));
    assert!(
        turns
            .iter()
            .any(|turn| turn.human == "add a regression test")
    );
}

#[test]
fn truncates_by_unicode_characters_and_renders_both_formats() {
    let mut cfg = config(fixture_root(), 9 * 60 * 60);
    cfg.context_chars = 10;
    let mut warnings = Vec::new();
    let turns = extract(&cfg, &mut warnings).unwrap();
    let first = turns
        .iter()
        .find(|turn| turn.human == "fix the typo in the header")
        .unwrap();
    assert_eq!(first.prev_assistant, "…der issue.");

    let mut jsonl = Vec::new();
    write_output(&turns, OutputFormat::Jsonl, &mut jsonl).unwrap();
    assert_eq!(
        String::from_utf8(jsonl).unwrap().lines().count(),
        turns.len()
    );

    let mut markdown = Vec::new();
    write_output(&turns, OutputFormat::Md, &mut markdown).unwrap();
    let markdown = String::from_utf8(markdown).unwrap();
    assert!(markdown.contains("# Session `test-session-1`"));
    assert!(markdown.contains("## Turn 1"));
}

#[test]
fn collects_the_previous_assistant_turn_with_a_bounded_window() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures-tools-window");
    let mut warnings = Vec::new();
    let turns = extract(&config(fixture.clone(), 9 * 60 * 60), &mut warnings).unwrap();
    let turn = turns
        .iter()
        .find(|turn| turn.human == "これが抽出対象なのだ")
        .unwrap();

    assert_eq!(turn.prev_tools, ["Bash ×2", "Read"]);
    assert_eq!(turn.prev_assistant, "確認するのだ\n結果はこうなのだ");
    assert!(turn.prev_assistant.ends_with("結果はこうなのだ"));
    assert!(!turn.prev_tools.iter().any(|tool| tool == "Write"));

    let mut bounded = config(fixture, 9 * 60 * 60);
    bounded.tools_window = 1;
    let turns = extract(&bounded, &mut warnings).unwrap();
    let turn = turns
        .iter()
        .find(|turn| turn.human == "これが抽出対象なのだ")
        .unwrap();
    assert_eq!(turn.prev_assistant, "結果はこうなのだ");
    assert!(turn.prev_tools.is_empty());
}

#[test]
fn summarizes_bash_tools_with_descriptions_fallbacks_truncation_and_collapsing() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("bash-tools");
    fs::create_dir_all(&project).unwrap();

    let long_description = "あ".repeat(61);
    let long_command = "界".repeat(61);
    let records = [
        serde_json::json!({
            "type": "assistant",
            "uuid": "tools",
            "parentUuid": null,
            "sessionId": "bash-session",
            "message": {"content": [
                {"type": "tool_use", "name": "Bash", "input": {
                    "description": "List files in current directory",
                    "command": "find . -type f"
                }},
                {"type": "tool_use", "name": "Bash", "input": {
                    "description": "List files in current directory",
                    "command": "rg --files"
                }},
                {"type": "tool_use", "name": "Bash", "input": {
                    "description": "Inspect repository status",
                    "command": "git status --short"
                }},
                {"type": "tool_use", "name": "Bash", "input": {
                    "command": "git diff\ngit status"
                }},
                {"type": "tool_use", "name": "Bash", "input": {
                    "description": long_description,
                    "command": "ignored"
                }},
                {"type": "tool_use", "name": "Bash", "input": {
                    "command": format!("{long_command}\nignored")
                }}
            ]}
        }),
        serde_json::json!({
            "type": "user",
            "uuid": "human",
            "parentUuid": "tools",
            "sessionId": "bash-session",
            "timestamp": "2026-09-17T03:00:00Z",
            "cwd": "/tmp/bash-tools",
            "isSidechain": false,
            "origin": {"kind": "human"},
            "message": {"content": "summarize the tools"}
        }),
    ];
    let transcript = records
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(project.join("session.jsonl"), transcript).unwrap();

    let mut warnings = Vec::new();
    let turns = extract(&config(temp.path().to_owned(), 9 * 60 * 60), &mut warnings).unwrap();
    assert_eq!(
        turns[0].prev_tools,
        [
            "Bash(List files in current directory) ×2".to_owned(),
            "Bash(Inspect repository status)".to_owned(),
            "Bash(git diff)".to_owned(),
            format!("Bash({}…)", "あ".repeat(60)),
            format!("Bash({}…)", "界".repeat(60)),
        ]
    );
}

#[test]
fn recursively_reads_only_jsonl_files() {
    let temp = tempfile::tempdir().unwrap();
    let nested = temp.path().join("fake-project/nested");
    fs::create_dir_all(&nested).unwrap();
    fs::copy(
        fixture_root().join("timezone-negative/session.jsonl"),
        nested.join("session.jsonl"),
    )
    .unwrap();
    fs::write(nested.join("ignored.txt"), "not json").unwrap();

    let mut warnings = Vec::new();
    let turns = extract(&config(temp.path().to_owned(), -7 * 60 * 60), &mut warnings).unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].project_dir, "fake-project");
}

#[test]
fn skips_reviewed_turns_when_ledger_is_present() {
    let temp = tempfile::tempdir().unwrap();
    let ledger_path = temp.path().join("ledger.json");
    let mut ledger = Ledger::default();

    // In fixtures:
    // test-session-1 has 3 turns (turn 1, 2, 3)
    // test-session-2 has 1 turn (turn 1)
    ledger.record(LedgerEntry {
        session: "test-session-1".to_owned(),
        turn_index: 1,
        status: JudgmentStatus::Approved,
        destination: Some("project_claude".into()),
        reason: Some("fixed header typo convention".into()),
        timestamp: None,
    });
    ledger.record(LedgerEntry {
        session: "test-session-2".to_owned(),
        turn_index: 1,
        status: JudgmentStatus::Rejected,
        destination: Some("discard".into()),
        reason: Some("routine check".into()),
        timestamp: None,
    });
    ledger.record(LedgerEntry {
        session: "test-session-1".to_owned(),
        turn_index: 2,
        status: JudgmentStatus::Deferred,
        destination: None,
        reason: Some("need more context".into()),
        timestamp: None,
    });
    ledger.save(&ledger_path).unwrap();

    let mut cfg = config(fixture_root(), 9 * 60 * 60);
    cfg.ledger_path = Some(ledger_path);

    let mut warnings = Vec::new();
    let turns = extract(&cfg, &mut warnings).unwrap();

    // test-session-1 turn 1 (Approved) and test-session-2 turn 1 (Rejected) should be skipped.
    // test-session-1 turn 2 (Deferred) should NOT be skipped.
    // test-session-1 turn 3 (Unreviewed) should NOT be skipped.
    assert_eq!(turns.len(), 2);
    let kept: Vec<_> = turns
        .iter()
        .map(|t| (&t.session[..], t.turn_index))
        .collect();
    assert!(kept.contains(&("test-session-1", 2)));
    assert!(kept.contains(&("test-session-1", 3)));
}

#[test]
fn lists_projects_with_turn_counts() {
    let mut warnings = Vec::new();
    let projects = list_projects(&config(fixture_root(), 9 * 60 * 60), &mut warnings).unwrap();

    // In fixtures: project-one has 3 turns, project-two has 1 turn
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0].project_dir, "project-one");
    assert_eq!(projects[0].turns_count, 3);
    assert_eq!(projects[1].project_dir, "project-two");
    assert_eq!(projects[1].turns_count, 1);
}

#[test]
fn filters_turns_by_project_name() {
    let mut cfg = config(fixture_root(), 9 * 60 * 60);
    cfg.project_filter = Some("project-two".to_owned());

    let mut warnings = Vec::new();
    let turns = extract(&cfg, &mut warnings).unwrap();

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].project_dir, "project-two");
    assert_eq!(turns[0].session, "test-session-2");
}

fn d(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
}

fn human_line(session: &str, uuid: &str, timestamp: &str, text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "uuid": uuid,
        "parentUuid": null,
        "sessionId": session,
        "timestamp": timestamp,
        "cwd": "/tmp/range",
        "isSidechain": false,
        "origin": {"kind": "human"},
        "message": {"content": text}
    })
    .to_string()
}

fn write_session(root: &Path, project: &str, file: &str, lines: &[(&str, &str, &str)]) {
    let dir = root.join(project);
    fs::create_dir_all(&dir).unwrap();
    let body = lines
        .iter()
        .enumerate()
        .map(|(index, (session, timestamp, text))| {
            human_line(session, &format!("{session}-{index}"), timestamp, text)
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(dir.join(file), body).unwrap();
}

/// Transcripts spread over 2026-09-10..=12 (UTC offset 0). The prompt "shared" is sent by four
/// distinct sessions overall but by at most two on any single day, and "BOILER" by three on the 12th.
fn range_fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_session(
        root,
        "proj-a",
        "s1.jsonl",
        &[
            ("s1", "2026-09-10T01:00:00Z", "alpha1"),
            ("s1", "2026-09-10T02:00:00Z", "alpha2"),
            ("s1", "2026-09-11T03:00:00Z", "beta1"),
            ("s1", "2026-09-13T03:00:00Z", "out of range"),
        ],
    );
    write_session(
        root,
        "proj-a",
        "s2.jsonl",
        &[
            ("s2", "2026-09-10T04:00:00Z", "shared"),
            ("s2", "2026-09-11T04:00:00Z", "x2"),
        ],
    );
    write_session(
        root,
        "proj-b",
        "s3.jsonl",
        &[
            ("s3", "2026-09-10T05:00:00Z", "shared"),
            ("s3", "2026-09-12T05:00:00Z", "BOILER"),
            ("s3", "2026-09-12T06:00:00Z", "shared"),
        ],
    );
    write_session(
        root,
        "proj-b",
        "s4.jsonl",
        &[
            ("s4", "2026-09-11T05:00:00Z", "shared"),
            ("s4", "2026-09-12T05:00:00Z", "BOILER"),
            ("s4", "2026-09-12T07:00:00Z", "y"),
        ],
    );
    write_session(
        root,
        "proj-b",
        "s5.jsonl",
        &[("s5", "2026-09-12T05:00:00Z", "BOILER")],
    );
    temp
}

fn range_config(root: &Path, dates: DateRange) -> Config {
    let mut cfg = config(root.to_owned(), 0);
    cfg.dates = dates;
    cfg
}

fn keys(turns: &[tutor::OutputTurn]) -> Vec<(String, usize, String, bool)> {
    turns
        .iter()
        .map(|turn| {
            (
                turn.session.clone(),
                turn.turn_index,
                turn.human.clone(),
                turn.already_captured,
            )
        })
        .collect()
}

#[test]
fn range_equals_union_of_single_days() {
    let temp = range_fixture();
    let mut warnings = Vec::new();
    let ranged = extract(
        &range_config(temp.path(), DateRange::new(d(10), d(12)).unwrap()),
        &mut warnings,
    )
    .unwrap();

    let mut union = Vec::new();
    for day in 10..=12 {
        let single = extract(
            &range_config(temp.path(), DateRange::single(d(day))),
            &mut warnings,
        )
        .unwrap();
        union.extend(keys(&single));
    }
    let mut ranged_keys = keys(&ranged);
    union.sort();
    ranged_keys.sort();
    assert_eq!(ranged_keys, union);
    assert_eq!(ranged.len(), 9);

    let s1: Vec<_> = ranged
        .iter()
        .filter(|turn| turn.session == "s1")
        .map(|turn| (turn.human.as_str(), turn.turn_index))
        .collect();
    assert_eq!(s1, [("alpha1", 1), ("alpha2", 2), ("beta1", 1)]);
    assert!(ranged.iter().all(|turn| turn.human != "out of range"));
}

#[test]
fn single_day_range_matches_date_result() {
    let temp = range_fixture();
    let mut warnings = Vec::new();
    let cfg = range_config(temp.path(), DateRange::new(d(10), d(10)).unwrap());
    assert!(cfg.dates.is_single_day());
    let turns = extract(&cfg, &mut warnings).unwrap();
    assert_eq!(
        keys(&turns),
        [
            ("s1".to_owned(), 1, "alpha1".to_owned(), false),
            ("s1".to_owned(), 2, "alpha2".to_owned(), false),
            ("s2".to_owned(), 1, "shared".to_owned(), false),
            ("s3".to_owned(), 1, "shared".to_owned(), false),
        ]
    );
}

#[test]
fn boilerplate_is_counted_per_day_in_a_range() {
    let temp = range_fixture();
    let mut warnings = Vec::new();
    let turns = extract(
        &range_config(temp.path(), DateRange::new(d(10), d(12)).unwrap()),
        &mut warnings,
    )
    .unwrap();
    let warnings = String::from_utf8(warnings).unwrap();

    assert_eq!(turns.iter().filter(|t| t.human == "shared").count(), 4);
    assert!(turns.iter().all(|turn| turn.human != "BOILER"));
    assert!(warnings.contains("excluded boilerplate seen in 3 distinct sessions on 2026-09-12"));

    // s3 and s4 lose their BOILER turn, so numbering restarts at 1 on that day.
    let s4: Vec<_> = turns
        .iter()
        .filter(|turn| turn.session == "s4")
        .map(|turn| (turn.human.as_str(), turn.turn_index))
        .collect();
    assert_eq!(s4, [("shared", 1), ("y", 1)]);
}

#[test]
fn single_day_boilerplate_warning_has_no_date() {
    let temp = range_fixture();
    let mut warnings = Vec::new();
    extract(
        &range_config(temp.path(), DateRange::single(d(12))),
        &mut warnings,
    )
    .unwrap();
    let warnings = String::from_utf8(warnings).unwrap();
    assert!(warnings.contains("excluded boilerplate seen in 3 distinct sessions: \"BOILER\""));
}

#[test]
fn list_projects_reports_daily_counts_only_for_multi_day_ranges() {
    let temp = range_fixture();
    let mut warnings = Vec::new();

    let ranged = list_projects(
        &range_config(temp.path(), DateRange::new(d(10), d(12)).unwrap()),
        &mut warnings,
    )
    .unwrap();
    assert_eq!(ranged.len(), 2);
    let proj_a = ranged.iter().find(|p| p.project_dir == "proj-a").unwrap();
    assert_eq!(proj_a.turns_count, 5);
    let daily = proj_a.daily_counts.as_ref().unwrap();
    assert_eq!(daily.get("2026-09-10"), Some(&3));
    assert_eq!(daily.get("2026-09-11"), Some(&2));
    assert_eq!(daily.get("2026-09-12"), None);
    let proj_b = ranged.iter().find(|p| p.project_dir == "proj-b").unwrap();
    assert_eq!(proj_b.turns_count, 4);

    let single = list_projects(
        &range_config(temp.path(), DateRange::single(d(10))),
        &mut warnings,
    )
    .unwrap();
    assert!(single.iter().all(|p| p.daily_counts.is_none()));
    let json = serde_json::to_string(&single).unwrap();
    assert!(!json.contains("daily_counts"));
}

#[test]
fn resolves_date_ranges() {
    let today = d(20);
    assert_eq!(
        resolve_date_range(None, None, None, today).unwrap(),
        DateRange::single(today)
    );
    assert_eq!(
        resolve_date_range(Some(d(5)), None, None, today).unwrap(),
        DateRange::single(d(5))
    );
    assert_eq!(
        resolve_date_range(None, Some(d(5)), None, today).unwrap(),
        DateRange::new(d(5), today).unwrap()
    );
    assert_eq!(
        resolve_date_range(None, Some(d(5)), Some(d(7)), today).unwrap(),
        DateRange::new(d(5), d(7)).unwrap()
    );
    assert!(resolve_date_range(Some(d(5)), Some(d(5)), None, today).is_err());
    assert!(resolve_date_range(Some(d(5)), None, Some(d(5)), today).is_err());
    assert!(resolve_date_range(None, None, Some(d(5)), today).is_err());
    assert!(resolve_date_range(None, Some(d(7)), Some(d(5)), today).is_err());
    assert!(resolve_date_range(None, Some(d(25)), None, today).is_err());
}

#[test]
fn date_range_displays_single_day_and_range() {
    assert_eq!(DateRange::single(d(10)).to_string(), "2026-09-10");
    assert_eq!(
        DateRange::new(d(1), d(30)).unwrap().to_string(),
        "2026-09-01..2026-09-30"
    );
    assert!(DateRange::new(d(2), d(1)).is_err());
}
