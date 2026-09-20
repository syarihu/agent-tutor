use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{JudgmentStatus, Ledger, LedgerEntry, OutputTurn};

pub const UI_HTML: &str = include_str!("ui.html");

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub session: String,
    pub turn_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_local: Option<String>,
    pub human: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_assistant: Option<String>,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default = "default_destination")]
    pub destination: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

fn default_status() -> String {
    "approved".to_string()
}

fn default_destination() -> String {
    "project_claude".to_string()
}

impl From<OutputTurn> for ReviewItem {
    fn from(turn: OutputTurn) -> Self {
        Self {
            session: turn.session,
            turn_index: turn.turn_index,
            project_dir: Some(turn.project_dir),
            cwd: Some(turn.cwd),
            timestamp_local: Some(turn.timestamp_local),
            human: turn.human,
            prev_assistant: if turn.prev_assistant.is_empty() {
                None
            } else {
                Some(turn.prev_assistant)
            },
            status: "approved".to_string(),
            destination: "project_claude".to_string(),
            reason: None,
            user_comment: None,
            draft_diff: None,
            file_path: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SubmitPayload {
    items: Vec<ReviewItem>,
}

/// Runs a local HTTP server for reviewing feedback items in a browser.
/// Blocks until the user clicks "Submit" in the web UI, then returns the finalized items.
pub fn run_review_ui(
    items: Vec<ReviewItem>,
    port: Option<u16>,
    open_browser: bool,
    ledger_path: Option<&Path>,
    destinations: Option<Vec<crate::DestinationDef>>,
) -> io::Result<Vec<ReviewItem>> {
    let port = port.unwrap_or(0);
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let local_addr = listener.local_addr()?;
    let url = format!("http://{}", local_addr);

    eprintln!("tutor ui: Review server running at {url}");
    if open_browser {
        let _ = open_in_browser(&url);
    }

    let destinations = destinations.unwrap_or_else(|| {
        let mut err = io::stderr();
        crate::load_or_init_destinations(None, None, &mut err).unwrap_or_default()
    });
    let destinations_json = serde_json::to_string(&destinations).map_err(io::Error::other)?;
    let items_json = serde_json::to_string(&items).map_err(io::Error::other)?;
    let mut submitted_items = None;

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if let Err(e) = handle_connection(
                    &mut stream,
                    &items_json,
                    &destinations_json,
                    &mut submitted_items,
                ) {
                    eprintln!("tutor ui: connection error: {e}");
                }
                if submitted_items.is_some() {
                    break;
                }
            }
            Err(e) => eprintln!("tutor ui: accept error: {e}"),
        }
    }

    let results = submitted_items.unwrap_or(items);

    if let Some(path) = ledger_path {
        let mut ledger = Ledger::load(path).unwrap_or_default();
        for item in &results {
            let status = match item.status.as_str() {
                "approved" => JudgmentStatus::Approved,
                "rejected" => JudgmentStatus::Rejected,
                "deferred" => JudgmentStatus::Deferred,
                _ => JudgmentStatus::Approved,
            };
            let now = chrono::Local::now().to_rfc3339();
            let reason = item
                .user_comment
                .clone()
                .filter(|c| !c.trim().is_empty())
                .or_else(|| item.reason.clone());
            ledger.record(LedgerEntry {
                session: item.session.clone(),
                turn_index: item.turn_index,
                status,
                destination: Some(item.destination.clone()),
                reason,
                timestamp: Some(now),
            });
        }
        if let Err(e) = ledger.save(path) {
            eprintln!("tutor ui: failed to save ledger: {e}");
        }
    }

    Ok(results)
}

fn handle_connection(
    stream: &mut TcpStream,
    items_json: &str,
    destinations_json: &str,
    submitted: &mut Option<Vec<ReviewItem>>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0];
    let path = parts[1];

    let mut content_length: usize = 0;
    loop {
        let mut header_line = String::new();
        if reader.read_line(&mut header_line)? == 0 {
            break;
        }
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }

    match (method, path) {
        ("GET", "/" | "/index.html") => {
            let body = UI_HTML.as_bytes();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes())?;
            stream.write_all(body)?;
        }
        ("GET", "/api/destinations") => {
            let body = destinations_json.as_bytes();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes())?;
            stream.write_all(body)?;
        }
        ("GET", "/api/items") => {
            let body = items_json.as_bytes();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes())?;
            stream.write_all(body)?;
        }
        ("POST", "/api/submit") => {
            let mut body_bytes = vec![0u8; content_length];
            reader.read_exact(&mut body_bytes)?;

            match serde_json::from_slice::<SubmitPayload>(&body_bytes) {
                Ok(payload) => {
                    *submitted = Some(payload.items);
                    let body = b"{\"status\":\"ok\"}";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(response.as_bytes())?;
                    stream.write_all(body)?;
                }
                Err(e) => {
                    let err_msg = format!("{{\"error\":\"{}\"}}", e);
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        err_msg.len(),
                        err_msg
                    );
                    stream.write_all(response.as_bytes())?;
                }
            }
        }
        _ => {
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(response.as_bytes())?;
        }
    }
    stream.flush()?;
    Ok(())
}

fn open_in_browser(url: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .spawn()?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    Ok(())
}
