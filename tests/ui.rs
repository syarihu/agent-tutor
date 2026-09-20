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
    assert!(ledger.is_reviewed("test-session-123", 4));
    let entry = ledger.entries.get("test-session-123:4").unwrap();
    assert_eq!(entry.status, JudgmentStatus::Rejected);
    assert_eq!(entry.destination.as_deref(), Some("discard"));
    assert_eq!(entry.reason.as_deref(), Some("個人設定のため不要"));
}
