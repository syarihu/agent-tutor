use std::io::{self, Write};
use std::path::PathBuf;

use chrono::{Local, NaiveDate};
use clap::{Args, Parser, Subcommand};
use tutor::{Config, JudgmentStatus, Ledger, LedgerEntry, OutputFormat, extract, write_output};

/// Extract human-authored turns from Claude Code conversation logs and manage review state.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[command(flatten)]
    extract: ExtractArgs,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Extract human turns (default)
    Extract(ExtractArgs),
    /// List projects that have unreviewed human turns
    Projects(ProjectsArgs),
    /// Record review judgment in the ledger
    Mark(MarkArgs),
    /// Print ledger contents as JSON
    Ledger(LedgerArgs),
    /// Install tutor MCP server configuration into Claude Code (~/.claude.json)
    #[command(alias = "setup-mcp")]
    InstallMcp(InstallMcpArgs),
    /// Manage and inspect available feedback destinations (auto-detected from environment and MCP)
    #[command(alias = "config")]
    Destinations(DestinationsArgs),
    /// Launch web review UI to approve/reject feedback items
    Ui(UiArgs),
    /// Run as a Model Context Protocol (MCP) stdio server
    Mcp,
}

#[derive(Debug, Args, Clone)]
struct ExtractArgs {
    /// Local calendar date to extract (YYYY-MM-DD). Defaults to today.
    #[arg(long, value_name = "YYYY-MM-DD")]
    date: Option<NaiveDate>,

    /// Root directory containing Claude Code project transcript files.
    #[arg(long, default_value_os_t = default_projects_dir(), value_name = "PATH")]
    projects_dir: PathBuf,

    /// Output format written to stdout.
    #[arg(long, value_enum, default_value_t = OutputFormat::Jsonl)]
    format: OutputFormat,

    /// Filter turns by project directory slug or substring.
    #[arg(long, value_name = "PROJECT")]
    project: Option<String>,

    /// Filter turns by exact working directory path.
    #[arg(long, value_name = "PATH")]
    cwd: Option<PathBuf>,

    /// Exclude identical prompts seen in at least this many distinct sessions.
    #[arg(long, default_value_t = 3, value_name = "N")]
    boilerplate_threshold: usize,

    /// Maximum number of characters retained from the previous assistant reply.
    #[arg(long, default_value_t = 600, value_name = "N")]
    context_chars: usize,

    /// Maximum number of assistant records searched before the previous human turn.
    #[arg(long, default_value_t = 20, value_name = "N")]
    tools_window: usize,

    /// Ledger file tracking reviewed turns.
    #[arg(long, value_name = "PATH")]
    ledger: Option<PathBuf>,

    /// Disable filtering against the default ledger.
    #[arg(long)]
    no_ledger: bool,
}

#[derive(Debug, Args)]
struct MarkArgs {
    /// Session ID
    #[arg(long)]
    session: Option<String>,

    /// 1-based turn index
    #[arg(long)]
    turn: Option<usize>,

    /// Review status (approved, rejected, deferred)
    #[arg(long, value_enum)]
    status: Option<JudgmentStatus>,

    /// Intended destination (e.g. project_claude, memory, discard)
    #[arg(long)]
    destination: Option<String>,

    /// Reason for decision
    #[arg(long)]
    reason: Option<String>,

    /// Batch input JSON file containing array of LedgerEntry
    #[arg(long, value_name = "PATH")]
    batch: Option<PathBuf>,

    /// Target ledger file
    #[arg(long, value_name = "PATH")]
    ledger: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct LedgerArgs {
    /// Target ledger file
    #[arg(long, value_name = "PATH")]
    ledger: Option<PathBuf>,
}

#[derive(Debug, Args, Clone)]
struct ProjectsArgs {
    /// Local calendar date (YYYY-MM-DD). Defaults to today.
    #[arg(long, value_name = "YYYY-MM-DD")]
    date: Option<NaiveDate>,

    /// Root directory containing Claude Code project transcript files.
    #[arg(long, default_value_os_t = default_projects_dir(), value_name = "PATH")]
    projects_dir: PathBuf,

    /// Output format (json or table).
    #[arg(long, default_value = "table", value_parser = ["json", "table"])]
    format: String,

    /// Ledger file tracking reviewed turns.
    #[arg(long, value_name = "PATH")]
    ledger: Option<PathBuf>,

    /// Disable filtering against the default ledger.
    #[arg(long)]
    no_ledger: bool,
}

#[derive(Debug, Args, Clone)]
struct InstallMcpArgs {
    /// Target configuration file path (defaults to ~/.claude.json)
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Also install to Claude Desktop configuration
    #[arg(long)]
    desktop: bool,
}

#[derive(Debug, Args, Clone)]
struct DestinationsArgs {
    /// Re-scan environment, CLI tools, and MCP servers, overwriting the configuration file
    #[arg(long)]
    reset: bool,

    /// Custom configuration file path (defaults to ~/.config/tutor/destinations.json)
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Output format (json or table)
    #[arg(long, default_value = "table", value_parser = ["json", "table"])]
    format: String,
}

#[derive(Debug, Args, Clone)]
struct UiArgs {
    /// Local calendar date to extract (YYYY-MM-DD). Defaults to today.
    #[arg(long, value_name = "YYYY-MM-DD")]
    date: Option<NaiveDate>,

    /// Root directory containing Claude Code project transcript files.
    #[arg(long, default_value_os_t = default_projects_dir(), value_name = "PATH")]
    projects_dir: PathBuf,

    /// Filter turns by project directory slug or substring.
    #[arg(long, value_name = "PROJECT")]
    project: Option<String>,

    /// Filter turns by exact working directory path.
    #[arg(long, value_name = "PATH")]
    cwd: Option<PathBuf>,

    /// Load review items from a JSON file instead of extracting from transcripts.
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,

    /// Port to bind the review web server (default: random available port).
    #[arg(long)]
    port: Option<u16>,

    /// Do not automatically open the browser.
    #[arg(long)]
    no_open: bool,

    /// Ledger file tracking reviewed turns.
    #[arg(long, value_name = "PATH")]
    ledger: Option<PathBuf>,

    /// Disable filtering against the default ledger and do not save results.
    #[arg(long)]
    no_ledger: bool,
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

fn resolve_ledger_path(specified: Option<PathBuf>, no_ledger: bool) -> Option<PathBuf> {
    if no_ledger {
        None
    } else if let Some(path) = specified {
        Some(path)
    } else {
        Some(default_ledger_path())
    }
}

fn handle_extract(args: ExtractArgs) -> Result<(), Box<dyn std::error::Error>> {
    let ledger_path = resolve_ledger_path(args.ledger, args.no_ledger);
    let config = Config {
        date: args.date.unwrap_or_else(|| Local::now().date_naive()),
        projects_dir: args.projects_dir,
        boilerplate_threshold: args.boilerplate_threshold,
        context_chars: args.context_chars,
        tools_window: args.tools_window,
        fixed_offset: None,
        ledger_path,
        project_filter: args.project,
        cwd_filter: args.cwd,
    };

    let mut stderr = io::stderr().lock();
    let turns = extract(&config, &mut stderr)?;
    let mut stdout = io::stdout().lock();
    write_output(&turns, args.format, &mut stdout)?;
    Ok(())
}

fn handle_projects(args: ProjectsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let ledger_path = resolve_ledger_path(args.ledger, args.no_ledger);
    let config = Config {
        date: args.date.unwrap_or_else(|| Local::now().date_naive()),
        projects_dir: args.projects_dir,
        boilerplate_threshold: 3,
        context_chars: 600,
        tools_window: 20,
        fixed_offset: None,
        ledger_path,
        project_filter: None,
        cwd_filter: None,
    };

    let mut stderr = io::stderr().lock();
    let projects = tutor::list_projects(&config, &mut stderr)?;
    let mut stdout = io::stdout().lock();

    if args.format == "json" {
        serde_json::to_writer_pretty(&mut stdout, &projects)?;
        writeln!(stdout)?;
    } else if projects.is_empty() {
        writeln!(
            stdout,
            "No projects with unreviewed turns found for {}.",
            config.date
        )?;
    } else {
        writeln!(stdout, "{:<35} {:<6} WORKING DIRECTORY", "PROJECT", "TURNS")?;
        writeln!(stdout, "{:-<35} {:-<6} {:-<40}", "", "", "")?;
        for p in &projects {
            writeln!(
                stdout,
                "{:<35} {:<6} {}",
                p.project_dir, p.turns_count, p.cwd
            )?;
        }
    }
    Ok(())
}

fn handle_mark(args: MarkArgs) -> Result<(), Box<dyn std::error::Error>> {
    let ledger_path = args.ledger.unwrap_or_else(default_ledger_path);
    let mut ledger = Ledger::load(&ledger_path)?;

    if let Some(batch_path) = args.batch {
        let content = std::fs::read_to_string(batch_path)?;
        let entries: Vec<LedgerEntry> = serde_json::from_str(&content)?;
        let count = entries.len();
        for entry in entries {
            ledger.record(entry);
        }
        ledger.save(&ledger_path)?;
        eprintln!(
            "tutor: recorded {count} entries into {}",
            ledger_path.display()
        );
        return Ok(());
    }

    let session = args
        .session
        .ok_or("error: --session is required unless --batch is provided")?;
    let turn_index = args
        .turn
        .ok_or("error: --turn is required unless --batch is provided")?;
    let status = args
        .status
        .ok_or("error: --status is required unless --batch is provided")?;

    let entry = LedgerEntry {
        session,
        turn_index,
        status,
        destination: args.destination,
        reason: args.reason,
        timestamp: Some(Local::now().to_rfc3339()),
    };

    let key = Ledger::entry_key(&entry.session, entry.turn_index);
    ledger.record(entry);
    ledger.save(&ledger_path)?;
    eprintln!(
        "tutor: recorded judgment for {key} into {}",
        ledger_path.display()
    );

    Ok(())
}

fn handle_ledger(args: LedgerArgs) -> Result<(), Box<dyn std::error::Error>> {
    let ledger_path = args.ledger.unwrap_or_else(default_ledger_path);
    let ledger = Ledger::load(&ledger_path)?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &ledger)?;
    writeln!(stdout)?;
    Ok(())
}

fn handle_install_mcp(args: InstallMcpArgs) -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("Could not determine HOME directory")?;

    let target_paths = if let Some(custom) = args.config {
        vec![custom]
    } else {
        let mut paths = vec![home.join(".claude.json")];
        if args.desktop {
            paths.push(home.join("Library/Application Support/Claude/claude_desktop_config.json"));
        }
        paths
    };

    for path in &target_paths {
        install_mcp_config(path)?;
    }

    Ok(())
}

fn install_mcp_config(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut root: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        serde_json::json!({})
    };

    let servers = root
        .as_object_mut()
        .ok_or("Invalid JSON root, expected an object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("mcpServers is not an object")?;

    servers.insert(
        "tutor".to_owned(),
        serde_json::json!({
            "type": "stdio",
            "command": "tutor",
            "args": ["mcp"],
            "env": {}
        }),
    );

    let formatted = serde_json::to_string_pretty(&root)?;
    std::fs::write(path, formatted)?;
    println!(
        "✓ Successfully configured tutor MCP server in {}",
        path.display()
    );

    Ok(())
}

fn handle_destinations(args: DestinationsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut stderr = io::stderr().lock();
    let path = args
        .config
        .unwrap_or_else(tutor::destinations::default_destinations_path);

    let list = if args.reset || !path.exists() {
        let current_dir = std::env::current_dir().ok();
        let detected = tutor::detect_destinations(current_dir.as_deref());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::File::create(&path)?;
        serde_json::to_writer_pretty(file, &detected)?;
        writeln!(
            stderr,
            "tutor: saved destinations configuration to {}",
            path.display()
        )?;
        detected
    } else {
        tutor::load_or_init_destinations(Some(&path), None, &mut stderr)?
    };

    let mut stdout = io::stdout().lock();
    if args.format == "json" {
        serde_json::to_writer_pretty(&mut stdout, &list)?;
        writeln!(stdout)?;
    } else {
        writeln!(
            stdout,
            "{:<24} {:<15} {:<12} DESCRIPTION",
            "ID", "CATEGORY", "TYPE"
        )?;
        writeln!(stdout, "{:-<24} {:-<15} {:-<12} {:-<35}", "", "", "", "")?;
        for d in &list {
            let cat = format!("{:?}", d.category);
            let apply = format!("{:?}", d.apply_type);
            let status_mark = if d.enabled { "" } else { " (disabled)" };
            writeln!(
                stdout,
                "{:<24} {:<15} {:<12} {}{}",
                d.id, cat, apply, d.description, status_mark
            )?;
        }
    }
    Ok(())
}

fn handle_ui(args: UiArgs) -> Result<(), Box<dyn std::error::Error>> {
    let ledger_path = resolve_ledger_path(args.ledger, args.no_ledger);

    let items: Vec<tutor::ReviewItem> = if let Some(file_path) = args.file {
        let content = std::fs::read_to_string(file_path)?;
        serde_json::from_str(&content)?
    } else {
        let config = Config {
            date: args.date.unwrap_or_else(|| Local::now().date_naive()),
            projects_dir: args.projects_dir,
            boilerplate_threshold: 3,
            context_chars: 600,
            tools_window: 20,
            fixed_offset: None,
            ledger_path: ledger_path.clone(),
            project_filter: args.project,
            cwd_filter: args.cwd,
        };
        let mut stderr = io::stderr().lock();
        let turns = extract(&config, &mut stderr)?;
        turns.into_iter().map(tutor::ReviewItem::from).collect()
    };

    let mut stderr = io::stderr().lock();
    let destinations = tutor::load_or_init_destinations(None, None, &mut stderr).ok();
    let finalized = tutor::run_review_ui(
        items,
        args.port,
        !args.no_open,
        ledger_path.as_deref(),
        destinations,
    )?;

    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &finalized)?;
    writeln!(stdout)?;

    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Extract(args)) => handle_extract(args),
        Some(Commands::Projects(args)) => handle_projects(args),
        Some(Commands::Mark(args)) => handle_mark(args),
        Some(Commands::Ledger(args)) => handle_ledger(args),
        Some(Commands::InstallMcp(args)) => handle_install_mcp(args),
        Some(Commands::Destinations(args)) => handle_destinations(args),
        Some(Commands::Ui(args)) => handle_ui(args),
        Some(Commands::Mcp) => tutor::mcp::run_mcp_server().map_err(Into::into),
        None => handle_extract(cli.extract),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("tutor: {error}");
        std::process::exit(1);
    }
}
