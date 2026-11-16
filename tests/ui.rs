use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use tempfile::tempdir;

use tutor::ui::{ReviewItem, run_review_ui};
use tutor::{JudgmentStatus, Ledger};

#[test]
fn test_review_ui_http_lifecycle_and_ledger() {
    let dir = tempdir().unwrap();
    let ledger_path = dir.path().join("ledger.json");

    let initial_items = vec![ReviewItem {
        session: "test-session-123".to_string(),
        turn_index: 4,
        date: Some("2026-09-18".to_string()),
        project_dir: Some("test-project".to_string()),
        cwd: Some("/path/to/test".to_string()),
        timestamp_local: Some("2026-09-18 10:00:00".to_string()),
        human: "テストコードのフォーマットを統一して".to_string(),
        prev_assistant: Some("了解しました。".to_string()),
        status: "approved".to_string(),
        destination: "project_claude".to_string(),
        reason: Some("プロジェクト規約".to_string()),
        user_comment: None,
        draft_diff: Some("+一致させます".to_string()),
        file_path: Some("CLAUDE.md".to_string()),
    }];

    // Bind to port 0 to get an ephemeral port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener); // release port

    let items_clone = initial_items.clone();
    let ledger_path_clone = ledger_path.clone();

    let server_thread = thread::spawn(move || {
        run_review_ui(
            items_clone,
            Some(port),
            false,
            Some(&ledger_path_clone),
            Some(tutor::detect_destinations(None)),
        )
        .expect("server failed")
    });

    // Wait a brief moment for server to start
    thread::sleep(std::time::Duration::from_millis(100));

    // 1. Test GET /
    {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("<title>Tutor Feedback Review</title>"));
    }

    // 2. Test GET /api/destinations
    {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(b"GET /api/destinations HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("project_claude"));
        assert!(resp.contains("project_agents"));
        assert!(resp.contains("global_agents"));
    }

    // 3. Test GET /api/items
    {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(b"GET /api/items HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("test-session-123"));
    }

    // 3. Test POST /api/submit
    {
        let mut submitted_items = initial_items.clone();
        submitted_items[0].status = "rejected".to_string();
        submitted_items[0].destination = "discard".to_string();
        submitted_items[0].user_comment = Some("個人設定のため不要".to_string());

        let payload = serde_json::json!({
            "items": submitted_items
        });
        let payload_bytes = serde_json::to_vec(&payload).unwrap();

        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let req = format!(
            "POST /api/submit HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
            payload_bytes.len()
        );
        stream.write_all(req.as_bytes()).unwrap();
        stream.write_all(&payload_bytes).unwrap();

        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("{\"status\":\"ok\"}"));
    }

    // Wait for server to finish
    let finalized = server_thread.join().unwrap();
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, "rejected");
    assert_eq!(finalized[0].destination, "discard");
    assert_eq!(
        finalized[0].user_comment.as_deref(),
        Some("個人設定のため不要")
    );

    // Check that ledger was updated with human reason
    let ledger = Ledger::load(&ledger_path).unwrap();
    assert!(ledger.is_reviewed("test-session-123", "2026-09-18", 4));
    let entry = ledger.entries.get("test-session-123:2026-09-18:4").unwrap();
    assert_eq!(entry.status, JudgmentStatus::Rejected);
    assert_eq!(entry.destination.as_deref(), Some("discard"));
    assert_eq!(entry.reason.as_deref(), Some("個人設定のため不要"));
}

fn item(session: &str, turn: usize, date: Option<&str>, timestamp: Option<&str>) -> ReviewItem {
    ReviewItem {
        session: session.to_string(),
        turn_index: turn,
        date: date.map(str::to_string),
        project_dir: None,
        cwd: None,
        timestamp_local: timestamp.map(str::to_string),
        human: "x".to_string(),
        prev_assistant: None,
        status: "approved".to_string(),
        destination: "discard".to_string(),
        reason: None,
        user_comment: None,
        draft_diff: None,
        file_path: None,
    }
}

#[test]
fn review_ui_rejects_undated_items_before_launching() {
    let dir = tempdir().unwrap();
    let ledger_path = dir.path().join("ledger.json");
    let items = vec![
        item("ok", 1, Some("2026-09-18"), None),
        item("bad", 2, None, None),
        item("badfmt", 3, Some("2026-9-1"), None),
    ];
    let error = run_review_ui(items, Some(0), false, Some(&ledger_path), Some(Vec::new()))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("bad:2") && error.contains("badfmt:3"),
        "{error}"
    );
    assert!(!error.contains("ok:1"), "{error}");
    assert!(!ledger_path.exists());
}

#[test]
fn review_ui_records_same_turn_index_on_different_dates_separately() {
    let dir = tempdir().unwrap();
    let ledger_path = dir.path().join("ledger.json");
    let items = vec![
        item("mid", 1, Some("2026-09-10"), None),
        item("mid", 1, None, Some("2026-09-11T00:30:00+09:00")),
    ];

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let submitted = items.clone();
    let path = ledger_path.clone();
    let server = thread::spawn(move || {
        run_review_ui(items, Some(port), false, Some(&path), Some(Vec::new())).unwrap()
    });
    thread::sleep(std::time::Duration::from_millis(100));

    let payload = serde_json::to_vec(&serde_json::json!({ "items": submitted })).unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = format!(
        "POST /api/submit HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
        payload.len()
    );
    stream.write_all(req.as_bytes()).unwrap();
    stream.write_all(&payload).unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 200 OK"));
    server.join().unwrap();

    let ledger = Ledger::load(&ledger_path).unwrap();
    assert_eq!(ledger.entries.len(), 2);
    assert!(ledger.entries.contains_key("mid:2026-09-10:1"));
    assert!(ledger.entries.contains_key("mid:2026-09-11:1"));
}

#[test]
fn review_ui_submit_with_undated_item_gets_an_error_and_keeps_waiting() {
    let dir = tempdir().unwrap();
    let ledger_path = dir.path().join("ledger.json");
    let items = vec![item("s", 1, Some("2026-09-10"), None)];

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let path = ledger_path.clone();
    let initial = items.clone();
    let server = thread::spawn(move || {
        run_review_ui(initial, Some(port), false, Some(&path), Some(Vec::new())).unwrap()
    });
    thread::sleep(std::time::Duration::from_millis(100));

    let post = |items: &[ReviewItem]| {
        let payload = serde_json::to_vec(&serde_json::json!({ "items": items })).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let req = format!(
            "POST /api/submit HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
            payload.len()
        );
        stream.write_all(req.as_bytes()).unwrap();
        stream.write_all(&payload).unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        resp
    };
    let bad = post(&[item("s", 1, None, None)]);
    assert!(bad.starts_with("HTTP/1.1 400"), "{bad}");
    assert!(!ledger_path.exists());
    assert!(post(&items).starts_with("HTTP/1.1 200 OK"));
    server.join().unwrap();
    assert!(
        Ledger::load(&ledger_path)
            .unwrap()
            .entries
            .contains_key("s:2026-09-10:1")
    );
}
