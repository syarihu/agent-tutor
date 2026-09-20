use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationCategory {
    Conventions,
    Documentation,
    Memory,
    Workflow,
    Issue,
    Knowledge,
    Discard,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyType {
    FileAppend,
    FileCreate,
    CliCommand,
    McpTool,
    None,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct DestinationDef {
    pub id: String,
    pub label: String,
    pub description: String,
    pub category: DestinationCategory,
    pub apply_type: ApplyType,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_tool: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Default path for the tutor destinations configuration: ~/.config/tutor/destinations.json
pub fn default_destinations_path() -> PathBuf {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(config_home).join("tutor/destinations.json")
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/tutor/destinations.json")
    }
}

/// Path to Claude Code global configuration: ~/.claude.json
fn claude_config_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude.json")
}

/// Check if a command is available on PATH
fn has_command(cmd: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(cmd);
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

/// Load registered MCP server names from ~/.claude.json
fn detect_mcp_servers() -> HashSet<String> {
    let mut servers = HashSet::new();
    let config_file = claude_config_path();
    if !config_file.exists() {
        return servers;
    }

    let Ok(content) = std::fs::read_to_string(&config_file) else {
        return servers;
    };
    let Ok(val) = serde_json::from_str::<Value>(&content) else {
        return servers;
    };
    if let Some(mcp_map) = val.get("mcpServers").and_then(Value::as_object) {
        for key in mcp_map.keys() {
            servers.insert(key.to_lowercase());
        }
    }
    servers
}

/// Detect available destinations tailored to the current machine environment and project structure.
pub fn detect_destinations(project_path: Option<&Path>) -> Vec<DestinationDef> {
    let mcp_servers = detect_mcp_servers();
    let mut list = vec![
        // 1. Core Conventions & Memory (always available)
        DestinationDef {
            id: "project_claude".into(),
            label: "project_claude (リポジトリの CLAUDE.md)".into(),
            description: "リポジトリ固有のコマンド、アーキテクチャ方針、コーディング規約".into(),
            category: DestinationCategory::Conventions,
            apply_type: ApplyType::FileAppend,
            enabled: true,
            command: None,
            mcp_server: None,
            mcp_tool: None,
        },
        DestinationDef {
            id: "project_agents".into(),
            label: "project_agents (リポジトリの AGENTS.md)".into(),
            description:
                "エージェント共通の行動規範、プロジェクト固有の制約・ルール (<cwd>/AGENTS.md)"
                    .into(),
            category: DestinationCategory::Conventions,
            apply_type: ApplyType::FileAppend,
            enabled: true,
            command: None,
            mcp_server: None,
            mcp_tool: None,
        },
        DestinationDef {
            id: "global_claude".into(),
            label: "global_claude (~/.claude/CLAUDE.md)".into(),
            description: "全プロジェクト共通の行動規範、個人の作業習慣、ツール共通設定".into(),
            category: DestinationCategory::Conventions,
            apply_type: ApplyType::FileAppend,
            enabled: true,
            command: None,
            mcp_server: None,
            mcp_tool: None,
        },
        DestinationDef {
            id: "global_agents".into(),
            label: "global_agents (グローバルの AGENTS.md)".into(),
            description: "全プロジェクト共通のエージェント行動規範、個人共通の制約・ルール".into(),
            category: DestinationCategory::Conventions,
            apply_type: ApplyType::FileAppend,
            enabled: true,
            command: None,
            mcp_server: None,
            mcp_tool: None,
        },
        DestinationDef {
            id: "project_docs".into(),
            label: "project_docs (docs/ 詳細ドキュメント)".into(),
            description: "プロジェクトの詳細仕様書、モジュール設計、詳細規約".into(),
            category: DestinationCategory::Documentation,
            apply_type: ApplyType::FileAppend,
            enabled: true,
            command: None,
            mcp_server: None,
            mcp_tool: None,
        },
        DestinationDef {
            id: "user_memory".into(),
            label: "user_memory (memory/ 観察メモ)".into(),
            description: "一時的な観察、単発フィードバック、個人の癖の記録".into(),
            category: DestinationCategory::Memory,
            apply_type: ApplyType::FileCreate,
            enabled: true,
            command: None,
            mcp_server: None,
            mcp_tool: None,
        },
        DestinationDef {
            id: "workflow_skill".into(),
            label: "workflow_skill (.claude/skills/ スキル手順)".into(),
            description: "複数コマンドを組み合わせる定型ワークフロー・手順の自動化".into(),
            category: DestinationCategory::Workflow,
            apply_type: ApplyType::FileCreate,
            enabled: true,
            command: None,
            mcp_server: None,
            mcp_tool: None,
        },
    ];

    // 2. Project-specific instructions if directory exists
    if let Some(proj) = project_path {
        if proj.join(".github/copilot-instructions.md").exists() || proj.join(".github").is_dir() {
            list.push(DestinationDef {
                id: "copilot_instructions".into(),
                label: "copilot_instructions (.github/copilot-instructions.md)".into(),
                description: "GitHub Copilot 向けのコーディング指示".into(),
                category: DestinationCategory::Conventions,
                apply_type: ApplyType::FileAppend,
                enabled: true,
                command: None,
                mcp_server: None,
                mcp_tool: None,
            });
        }
        if proj.join(".github/instructions").is_dir() {
            list.push(DestinationDef {
                id: "project_instructions".into(),
                label: "project_instructions (.github/instructions/*.md)".into(),
                description: "ファイルパターンや特定領域に特化した指示".into(),
                category: DestinationCategory::Conventions,
                apply_type: ApplyType::FileCreate,
                enabled: true,
                command: None,
                mcp_server: None,
                mcp_tool: None,
            });
        }
    }

    // 3. Issue & Task Management destinations
    // GitHub CLI or GitHub MCP
    let has_gh_cli = has_command("gh");
    let has_github_mcp = mcp_servers.iter().any(|s| s.contains("github"));
    if has_gh_cli || has_github_mcp {
        list.push(DestinationDef {
            id: "issue_github".into(),
            label: "issue_github (GitHub Issue 起票)".into(),
            description: "バグ報告や将来の改善タスクとしてリポジトリに Issue を起票する".into(),
            category: DestinationCategory::Issue,
            apply_type: if has_gh_cli {
                ApplyType::CliCommand
            } else {
                ApplyType::McpTool
            },
            enabled: true,
            command: if has_gh_cli {
                Some("gh issue create --title \"<title>\" --body \"<body>\"".into())
            } else {
                None
            },
            mcp_server: if has_github_mcp {
                Some("github".into())
            } else {
                None
            },
            mcp_tool: if has_github_mcp {
                Some("create_issue".into())
            } else {
                None
            },
        });
    }

    // Linear MCP or Linear CLI
    let has_linear_mcp = mcp_servers.iter().any(|s| s.contains("linear"));
    let has_linear_cli = has_command("linear");
    if has_linear_mcp || has_linear_cli {
        list.push(DestinationDef {
            id: "issue_linear".into(),
            label: "issue_linear (Linear タスク起票)".into(),
            description: "Linear にバグや機能改善タスクを新規作成する".into(),
            category: DestinationCategory::Issue,
            apply_type: if has_linear_mcp {
                ApplyType::McpTool
            } else {
                ApplyType::CliCommand
            },
            enabled: true,
            command: if has_linear_cli {
                Some("linear issue create".into())
            } else {
                None
            },
            mcp_server: if has_linear_mcp {
                Some("linear-server".into())
            } else {
                None
            },
            mcp_tool: if has_linear_mcp {
                Some("create_issue".into())
            } else {
                None
            },
        });
    }

    // Asana MCP
    let has_asana = mcp_servers.iter().any(|s| s.contains("asana"));
    if has_asana {
        list.push(DestinationDef {
            id: "issue_asana".into(),
            label: "issue_asana (Asana タスク起票)".into(),
            description: "Asana にタスクを新規作成する".into(),
            category: DestinationCategory::Issue,
            apply_type: ApplyType::McpTool,
            enabled: true,
            command: None,
            mcp_server: Some("asana".into()),
            mcp_tool: Some("create_task".into()),
        });
    }

    // Jira MCP
    let has_jira = mcp_servers.iter().any(|s| s.contains("jira"));
    if has_jira {
        list.push(DestinationDef {
            id: "issue_jira".into(),
            label: "issue_jira (Jira イシュー起票)".into(),
            description: "Jira にイシューを新規作成する".into(),
            category: DestinationCategory::Issue,
            apply_type: ApplyType::McpTool,
            enabled: true,
            command: None,
            mcp_server: Some("jira".into()),
            mcp_tool: Some("create_issue".into()),
        });
    }

    // 4. Knowledge Base destinations
    // lk CLI or lk MCP
    let has_lk_cli = has_command("lk");
    let has_lk_mcp = mcp_servers.iter().any(|s| s.contains("lk"));
    if has_lk_cli || has_lk_mcp {
        list.push(DestinationDef {
            id: "knowledge_lk".into(),
            label: "knowledge_lk (lk ナレッジベース)".into(),
            description: "リポジトリを跨ぐ横断的な技術知見・調査メモ・トラブルシュート".into(),
            category: DestinationCategory::Knowledge,
            apply_type: ApplyType::McpTool,
            enabled: true,
            command: if has_lk_cli {
                Some("lk add".into())
            } else {
                None
            },
            mcp_server: Some("lk-knowledge".into()),
            mcp_tool: Some("add_knowledge".into()),
        });
    }

    // Notion MCP
    let has_notion = mcp_servers.iter().any(|s| s.contains("notion"));
    if has_notion {
        list.push(DestinationDef {
            id: "knowledge_notion".into(),
            label: "knowledge_notion (Notion ページ作成)".into(),
            description: "Notion のナレッジデータベースにページを作成して知見を蓄積する".into(),
            category: DestinationCategory::Knowledge,
            apply_type: ApplyType::McpTool,
            enabled: true,
            command: None,
            mcp_server: Some("notion".into()),
            mcp_tool: Some("create_page".into()),
        });
    }

    // 5. Discard (always available)
    list.push(DestinationDef {
        id: "discard".into(),
        label: "discard (反映不要・捨てる)".into(),
        description: "規約化・起票不要（その場限りのタスク指示、日常確認、雑談など）".into(),
        category: DestinationCategory::Discard,
        apply_type: ApplyType::None,
        enabled: true,
        command: None,
        mcp_server: None,
        mcp_tool: None,
    });

    list
}

/// Load destinations configuration from file, or detect and write defaults if file does not exist.
pub fn load_or_init_destinations(
    custom_path: Option<&Path>,
    project_path: Option<&Path>,
    stderr: &mut impl Write,
) -> io::Result<Vec<DestinationDef>> {
    let path = custom_path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_destinations_path);

    if path.exists() {
        let file = File::open(&path)?;
        let reader = BufReader::new(file);
        let mut list: Vec<DestinationDef> =
            serde_json::from_reader(reader).map_err(io::Error::other)?;

        let detected = detect_destinations(project_path);
        let existing_ids: HashSet<String> = list.iter().map(|d| d.id.clone()).collect();
        let mut added = false;
        for d in detected {
            if !existing_ids.contains(&d.id) {
                if let Some(pos) = list.iter().position(|item| item.id == "discard") {
                    list.insert(pos, d);
                } else {
                    list.push(d);
                }
                added = true;
            }
        }
        if added {
            let _ = File::create(&path).map(|file| serde_json::to_writer_pretty(file, &list));
        }
        Ok(list)
    } else {
        let detected = detect_destinations(project_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::create(&path)?;
        serde_json::to_writer_pretty(file, &detected).map_err(io::Error::other)?;
        let _ = writeln!(
            stderr,
            "tutor: initialized destinations configuration in {}",
            path.display()
        );
        Ok(detected)
    }
}

/// Format destinations as a Markdown reference table for LLM prompts.
pub fn format_destinations_markdown(destinations: &[DestinationDef]) -> String {
    let mut out = String::from(
        "# Tutor Canonical Destinations\n\n\
        | Destination ID | Category | Target / Command | Scope & Purpose | Apply Method |\n\
        | :--- | :--- | :--- | :--- | :--- |\n",
    );

    for d in destinations {
        if !d.enabled {
            continue;
        }
        let target = if let Some(cmd) = &d.command {
            cmd.as_str()
        } else if let Some(srv) = &d.mcp_server {
            srv.as_str()
        } else {
            match d.id.as_str() {
                "project_claude" => "<cwd>/CLAUDE.md",
                "project_agents" => "<cwd>/AGENTS.md",
                "global_claude" => "~/.claude/CLAUDE.md",
                "global_agents" => "~/.config/rules/AGENTS.md",
                "copilot_instructions" => ".github/copilot-instructions.md",
                "project_instructions" => ".github/instructions/*.md",
                "project_docs" => "<cwd>/docs/**/*.md",
                "user_memory" => "~/.claude/projects/<slug>/memory/",
                "workflow_skill" => ".claude/skills/<name>/SKILL.md",
                _ => "(none)",
            }
        };

        let method = match d.apply_type {
            ApplyType::FileAppend => "File append / edit",
            ApplyType::FileCreate => "File creation",
            ApplyType::CliCommand => "CLI command execution",
            ApplyType::McpTool => "MCP tool call",
            ApplyType::None => "Mark ledger only",
        };

        let cat_str = format!("{:?}", d.category);
        out.push_str(&format!(
            "| `{}` | {} | `{}` | {} | {} |\n",
            d.id, cat_str, target, d.description, method
        ));
    }

    out
}
