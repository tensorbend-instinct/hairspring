//! RED (Eric 2026-10-06): no workspace/session switcher. Sessions carry the
//! project root they ran in; the list groups by workspace, newest first,
//! and the resume picker labels each session with its workspace.

use hs_loop::repl::{
    group_sessions_by_workspace, list_sessions, record_session_workspace, session_line,
    sessions_overview, SessionInfo,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn info(id: &str, ws: &str, age_s: u64, preview: &str) -> SessionInfo {
    SessionInfo {
        id: uuid::Uuid::parse_str(id).unwrap(),
        events: 5,
        preview: preview.to_string(),
        modified: UNIX_EPOCH + Duration::from_secs(1_000_000 - age_s),
        workspace: ws.to_string(),
    }
}

const A: &str = "11111111-1111-1111-1111-111111111111";
const B: &str = "22222222-2222-2222-2222-222222222222";
const C: &str = "33333333-3333-3333-3333-333333333333";

#[test]
fn groups_by_workspace_newest_group_first_and_newest_within() {
    let infos = vec![
        info(A, "/work/api", 300, "old api"),
        info(B, "/work/web", 10, "web newest"),
        info(C, "/work/api", 50, "api newer"),
    ];
    let g = group_sessions_by_workspace(&infos);
    assert_eq!(g.len(), 2);
    assert_eq!(g[0].0, "/work/web", "group with the newest session first");
    assert_eq!(g[1].0, "/work/api");
    assert_eq!(g[1].1[0].preview, "api newer", "newest first inside a group");
}

#[test]
fn unknown_workspace_has_its_own_group_label() {
    let v = [info(A, "", 1, "x")];
    let g = group_sessions_by_workspace(&v);
    assert_eq!(g[0].0, "(unrecorded workspace)");
}

#[test]
fn overview_lists_every_session_under_its_workspace() {
    let infos = vec![info(A, "/work/api", 300, "old api"), info(B, "/work/web", 10, "web")];
    let text = sessions_overview(&infos).join("\n");
    assert!(text.contains("/work/web") && text.contains("/work/api"), "{text}");
    assert!(text.contains("old api") && text.contains("web"), "{text}");
}

#[test]
fn picker_line_names_the_workspace() {
    let line = session_line(1, &info(A, "/work/api", 1, "fix parser"));
    assert!(line.contains("api") && line.contains("fix parser"), "{line}");
}

#[test]
fn record_then_list_round_trips_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::parse_str(A).unwrap();
    record_session_workspace(dir.path(), id, std::path::Path::new("/work/api"));
    let rec = std::fs::read_to_string(dir.path().join("session_workspace").join(A)).unwrap();
    assert_eq!(rec.trim(), "/work/api");
    // no streams yet: list is empty, not an error
    assert!(list_sessions(dir.path()).is_empty());
    let _ = SystemTime::now();
}
