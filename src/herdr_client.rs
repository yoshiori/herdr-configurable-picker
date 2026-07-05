//! Client for herdr's newline-delimited JSON API over `$HERDR_SOCKET_PATH`.
//!
//! One request per line, one response per line — and one request per
//! CONNECTION (herdr hangs up after answering; see `SocketClient`):
//!   -> {"id":"1","method":"tab.list","params":{}}
//!   <- {"id":"1","result":{"type":"tab_list","tabs":[...]}}
//!   <- {"id":"1","error":{"code":"tab_not_found","message":"..."}}
//!
//! The structs below model only the fields the picker uses; serde ignores
//! unknown fields, so herdr adding fields never breaks us.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentStatus {
    pub fn name(self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Working => "working",
            AgentStatus::Blocked => "blocked",
            AgentStatus::Done => "done",
            AgentStatus::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    pub number: usize,
    pub label: String,
    pub focused: bool,
    pub pane_count: usize,
    pub tab_count: usize,
    pub active_tab_id: String,
    pub agent_status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TabInfo {
    pub tab_id: String,
    pub workspace_id: String,
    pub number: usize,
    pub label: String,
    pub focused: bool,
    pub pane_count: usize,
    pub agent_status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub tab_id: String,
    pub workspace_id: String,
    pub focused: bool,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub display_agent: Option<String>,
    pub agent_status: AgentStatus,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// User/plugin-set status text; wins over the agent state name.
    #[serde(default)]
    pub custom_status: Option<String>,
    pub terminal_id: String,
}

/// The herdr calls the picker needs. `main` talks to this trait so tests can
/// substitute a mock; `SocketClient` is the real implementation.
pub trait HerdrApi {
    fn list_workspaces(&mut self) -> Result<Vec<WorkspaceInfo>>;
    fn list_tabs(&mut self) -> Result<Vec<TabInfo>>;
    fn list_panes(&mut self) -> Result<Vec<PaneInfo>>;
    fn focus_workspace(&mut self, workspace_id: &str) -> Result<()>;
    fn focus_tab(&mut self, tab_id: &str) -> Result<()>;
    /// Socket-only method (no CLI equivalent); works for agentless panes too.
    fn focus_pane(&mut self, pane_id: &str) -> Result<()>;
}

/// Builds one request line (without the trailing newline).
fn request_line(id: &str, method: &str, params: Value) -> String {
    // herdr's Method enum is serde tag="method"/content="params", which
    // requires the params key to be present even when empty.
    json!({"id": id, "method": method, "params": params}).to_string()
}

/// Validates the response envelope for `expected_id` and unwraps `result`.
fn parse_response(line: &str, expected_id: &str) -> Result<Value> {
    let envelope: Value =
        serde_json::from_str(line).context("herdr sent a response that is not valid JSON")?;
    // Errors first: herdr answers requests it cannot even parse (e.g. a
    // method this version does not know) with an EMPTY id, and that error
    // message beats an "id mismatch" complaint every time.
    if let Some(error) = envelope.get("error") {
        let code = error["code"].as_str().unwrap_or("unknown_error");
        let message = error["message"].as_str().unwrap_or("no message");
        bail!("herdr error {code}: {message}");
    }
    let id = envelope["id"].as_str().unwrap_or_default();
    if id != expected_id {
        bail!("herdr response id {id:?} does not match request id {expected_id:?}");
    }
    match envelope.get("result") {
        Some(result) => Ok(result.clone()),
        None => bail!("herdr response carries neither result nor error"),
    }
}

/// Extracts the typed payload out of a `result` object, checking its
/// `type` tag first so version drift produces a clear error.
fn extract_payload<T: serde::de::DeserializeOwned>(
    result: &Value,
    expected_type: &str,
    field: &str,
) -> Result<T> {
    let actual_type = result["type"].as_str().unwrap_or("<missing>");
    if actual_type != expected_type {
        bail!("expected a {expected_type} result from herdr, got {actual_type}");
    }
    serde_json::from_value(result[field].clone())
        .with_context(|| format!("could not parse {field} out of a {expected_type} result"))
}

/// herdr's API server answers exactly ONE request per connection (only
/// events.subscribe and pane.wait_for_output keep the stream open), so the
/// client dials a fresh connection for every call.
#[derive(Debug)]
pub struct SocketClient {
    socket_path: std::path::PathBuf,
    next_id: u64,
}

impl SocketClient {
    pub fn connect(socket_path: &Path) -> Result<Self> {
        // Probe now so a dead socket fails at startup with a clear message
        // instead of on the first request.
        Self::dial(socket_path)?;
        Ok(SocketClient {
            socket_path: socket_path.to_path_buf(),
            next_id: 1,
        })
    }

    fn dial(socket_path: &Path) -> Result<UnixStream> {
        UnixStream::connect(socket_path).with_context(|| {
            format!(
                "cannot connect to the herdr API socket at {}; \
                 the picker must run inside a herdr session",
                socket_path.display()
            )
        })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.to_string();
        self.next_id += 1;

        let mut reader = BufReader::new(Self::dial(&self.socket_path)?);
        let line = request_line(&id, method, params);
        let stream = reader.get_mut();
        stream
            .write_all(line.as_bytes())
            .and_then(|_| stream.write_all(b"\n"))
            .with_context(|| format!("failed to send {method} request to herdr"))?;

        let mut response = String::new();
        let read = reader
            .read_line(&mut response)
            .with_context(|| format!("failed to read herdr's {method} response"))?;
        if read == 0 {
            bail!("herdr closed the connection before answering {method}");
        }
        parse_response(&response, &id)
    }
}

impl HerdrApi for SocketClient {
    fn list_workspaces(&mut self) -> Result<Vec<WorkspaceInfo>> {
        let result = self.call("workspace.list", json!({}))?;
        extract_payload(&result, "workspace_list", "workspaces")
    }

    fn list_tabs(&mut self) -> Result<Vec<TabInfo>> {
        let result = self.call("tab.list", json!({}))?;
        extract_payload(&result, "tab_list", "tabs")
    }

    fn list_panes(&mut self) -> Result<Vec<PaneInfo>> {
        let result = self.call("pane.list", json!({}))?;
        extract_payload(&result, "pane_list", "panes")
    }

    fn focus_workspace(&mut self, workspace_id: &str) -> Result<()> {
        self.call("workspace.focus", json!({"workspace_id": workspace_id}))?;
        Ok(())
    }

    fn focus_tab(&mut self, tab_id: &str) -> Result<()> {
        // Any success result means the focus landed; the payload shape is
        // herdr's business.
        self.call("tab.focus", json!({"tab_id": tab_id}))?;
        Ok(())
    }

    fn focus_pane(&mut self, pane_id: &str) -> Result<()> {
        self.call("pane.focus", json!({"pane_id": pane_id}))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread::JoinHandle;

    // --- Group D: wire format, no socket ---

    #[test]
    fn request_line_always_carries_params() {
        let line = request_line("1", "workspace.list", json!({}));
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            value,
            json!({"id": "1", "method": "workspace.list", "params": {}})
        );
    }

    #[test]
    fn request_line_carries_target_params() {
        let line = request_line("42", "tab.focus", json!({"tab_id": "w1:t2"}));
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            value,
            json!({"id": "42", "method": "tab.focus", "params": {"tab_id": "w1:t2"}})
        );
    }

    #[test]
    fn parses_success_envelope_and_unwraps_result() {
        let result =
            parse_response(r#"{"id":"7","result":{"type":"tab_info","tab":{}}}"#, "7").unwrap();
        assert_eq!(result["type"], "tab_info");
    }

    #[test]
    fn error_envelope_becomes_error_with_code_and_message() {
        let err = parse_response(
            r#"{"id":"1","error":{"code":"tab_not_found","message":"tab w9:t9 not found"}}"#,
            "1",
        )
        .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("tab_not_found"), "error: {text}");
        assert!(text.contains("tab w9:t9 not found"), "error: {text}");
    }

    #[test]
    fn mismatched_response_id_is_an_error() {
        let err = parse_response(r#"{"id":"2","result":{"type":"tab_list","tabs":[]}}"#, "1")
            .unwrap_err();
        assert!(err.to_string().contains("id"), "error: {err}");
    }

    #[test]
    fn error_envelope_with_empty_id_still_surfaces_the_error() {
        // herdr 0.7.1 answers unknown methods with id "" — the message must
        // not be masked by an id-mismatch complaint.
        let err = parse_response(
            r#"{"id":"","error":{"code":"invalid_request","message":"invalid request: unknown variant `pane.focus`"}}"#,
            "4",
        )
        .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("invalid_request"), "error: {text}");
        assert!(text.contains("pane.focus"), "error: {text}");
    }

    #[test]
    fn tab_list_fixture_parses_ignoring_unknown_fields() {
        // Shape copied from herdr's TabInfo, plus invented fields a future
        // herdr might add.
        let result: Value = serde_json::from_str(
            r#"{
                "type": "tab_list",
                "tabs": [{
                    "tab_id": "w1:t1",
                    "workspace_id": "w1",
                    "number": 1,
                    "label": "mothership",
                    "focused": true,
                    "pane_count": 2,
                    "agent_status": "working",
                    "some_future_field": {"nested": true}
                }]
            }"#,
        )
        .unwrap();
        let tabs: Vec<TabInfo> = extract_payload(&result, "tab_list", "tabs").unwrap();
        assert_eq!(
            tabs,
            vec![TabInfo {
                tab_id: "w1:t1".to_string(),
                workspace_id: "w1".to_string(),
                number: 1,
                label: "mothership".to_string(),
                focused: true,
                pane_count: 2,
                agent_status: AgentStatus::Working,
            }]
        );
    }

    #[test]
    fn workspace_list_fixture_parses_ignoring_unknown_fields() {
        let result: Value = serde_json::from_str(
            r#"{
                "type": "workspace_list",
                "workspaces": [{
                    "workspace_id": "w1",
                    "number": 1,
                    "label": "mothership",
                    "focused": true,
                    "pane_count": 3,
                    "tab_count": 2,
                    "active_tab_id": "w1:t2",
                    "agent_status": "idle",
                    "worktree": {"repo_key": "x", "repo_name": "y"}
                }]
            }"#,
        )
        .unwrap();
        let workspaces: Vec<WorkspaceInfo> =
            extract_payload(&result, "workspace_list", "workspaces").unwrap();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].workspace_id, "w1");
        assert_eq!(workspaces[0].active_tab_id, "w1:t2");
    }

    #[test]
    fn unexpected_result_type_is_an_error() {
        let result: Value = serde_json::from_str(r#"{"type": "pane_list", "panes": []}"#).unwrap();
        let err = extract_payload::<Vec<TabInfo>>(&result, "tab_list", "tabs").unwrap_err();
        assert!(err.to_string().contains("pane_list"), "error: {err}");
    }

    #[test]
    fn pane_list_fixture_parses_with_optional_fields_absent_or_present() {
        // First pane: agentless shell with the optional fields missing.
        // Second: agent pane with everything set, plus invented fields.
        let result: Value = serde_json::from_str(
            r#"{
                "type": "pane_list",
                "panes": [{
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1",
                    "focused": false,
                    "agent_status": "unknown",
                    "terminal_id": "term_a",
                    "revision": 3
                }, {
                    "pane_id": "w1:p2",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1",
                    "focused": true,
                    "agent": "claude",
                    "display_agent": "Claude",
                    "agent_status": "working",
                    "cwd": "/home/user/repo",
                    "label": "builder",
                    "title": "make -j8",
                    "terminal_id": "term_b",
                    "state_labels": {"future": "field"}
                }]
            }"#,
        )
        .unwrap();
        let panes: Vec<PaneInfo> = extract_payload(&result, "pane_list", "panes").unwrap();
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].agent, None);
        assert_eq!(panes[0].label, None);
        assert_eq!(panes[1].agent.as_deref(), Some("claude"));
        assert_eq!(panes[1].agent_status, AgentStatus::Working);
        assert!(panes[1].focused);
    }

    #[test]
    fn all_agent_status_values_parse() {
        for (text, expected) in [
            ("idle", AgentStatus::Idle),
            ("working", AgentStatus::Working),
            ("blocked", AgentStatus::Blocked),
            ("done", AgentStatus::Done),
            ("unknown", AgentStatus::Unknown),
        ] {
            let parsed: AgentStatus = serde_json::from_str(&format!("\"{text}\"")).unwrap();
            assert_eq!(parsed, expected);
        }
    }

    // --- Group E: SocketClient against a fake server ---

    /// Fake herdr mirroring the real server's connection model: ONE request
    /// per connection (herdr's handle_connection reads a single line, writes
    /// a single response, and hangs up). Each canned response is served on
    /// its own accepted connection; connections with no request (probes) are
    /// tolerated; once the responses run out the next request gets a silent
    /// hang-up. Join to get the raw request lines the client sent.
    fn spawn_fake_server(responses: Vec<String>) -> (PathBuf, JoinHandle<Vec<String>>) {
        // Keep paths short: sun_path caps out around 104 bytes, and
        // $TMPDIR-based tempdirs can blow past that.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hcp-{}-{}.sock",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind fake server socket");
        let handle = std::thread::spawn(move || {
            let mut received = Vec::new();
            let mut responses = responses.into_iter();
            loop {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    // A connection dropped without a request (startup probe).
                    continue;
                }
                received.push(line.trim_end().to_string());
                let Some(response) = responses.next() else {
                    break; // hang up without answering -> client sees EOF
                };
                let stream = reader.get_mut();
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
                if responses.len() == 0 {
                    break;
                }
            }
            received
        });
        (path, handle)
    }

    #[test]
    fn sequential_calls_work_across_reconnects() {
        // herdr answers exactly one request per connection, so the client
        // must reconnect for every call. This is the regression test for
        // the "workspace.list works, tab.list gets EPIPE" failure seen
        // against a live herdr.
        let (path, server) = spawn_fake_server(vec![
            r#"{"id":"1","result":{"type":"workspace_list","workspaces":[]}}"#.to_string(),
            r#"{"id":"2","result":{"type":"tab_list","tabs":[]}}"#.to_string(),
        ]);

        let mut client = SocketClient::connect(&path).unwrap();
        assert!(client.list_workspaces().unwrap().is_empty());
        assert!(client.list_tabs().unwrap().is_empty());

        let received = server.join().unwrap();
        assert_eq!(received.len(), 2);
        let first: Value = serde_json::from_str(&received[0]).unwrap();
        let second: Value = serde_json::from_str(&received[1]).unwrap();
        assert_eq!(first["method"], "workspace.list");
        assert_eq!(second["method"], "tab.list");
    }

    #[test]
    fn list_tabs_round_trips_through_the_socket() {
        let (path, server) = spawn_fake_server(vec![
            r#"{"id":"1","result":{"type":"tab_list","tabs":[{"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"main","focused":false,"pane_count":1,"agent_status":"idle"}]}}"#
                .to_string(),
        ]);

        let mut client = SocketClient::connect(&path).unwrap();
        let tabs = client.list_tabs().unwrap();

        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].tab_id, "w1:t1");
        drop(client); // the server drains until EOF, so hang up before joining
        let received = server.join().unwrap();
        assert_eq!(received.len(), 1);
        let request: Value = serde_json::from_str(&received[0]).unwrap();
        assert_eq!(request["method"], "tab.list");
        assert_eq!(request["id"], "1");
        assert_eq!(request["params"], json!({}));
    }

    #[test]
    fn focus_tab_sends_target_and_accepts_any_success() {
        let (path, server) = spawn_fake_server(vec![
            r#"{"id":"1","result":{"type":"tab_info","tab":{"whatever":true}}}"#.to_string(),
        ]);

        let mut client = SocketClient::connect(&path).unwrap();
        client.focus_tab("w1:t2").unwrap();

        drop(client); // the server drains until EOF, so hang up before joining
        let received = server.join().unwrap();
        let request: Value = serde_json::from_str(&received[0]).unwrap();
        assert_eq!(request["method"], "tab.focus");
        assert_eq!(request["params"], json!({"tab_id": "w1:t2"}));
    }

    #[test]
    fn pane_and_workspace_focus_send_socket_methods_with_targets() {
        let (path, server) = spawn_fake_server(vec![
            r#"{"id":"1","result":{"type":"workspace_info","workspace":{}}}"#.to_string(),
            r#"{"id":"2","result":{"type":"pane_info","pane":{}}}"#.to_string(),
            r#"{"id":"3","result":{"type":"pane_list","panes":[]}}"#.to_string(),
        ]);

        let mut client = SocketClient::connect(&path).unwrap();
        client.focus_workspace("w2").unwrap();
        client.focus_pane("w2:p3").unwrap();
        assert!(client.list_panes().unwrap().is_empty());

        let received = server.join().unwrap();
        let requests: Vec<Value> = received
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(requests[0]["method"], "workspace.focus");
        assert_eq!(requests[0]["params"], json!({"workspace_id": "w2"}));
        assert_eq!(requests[1]["method"], "pane.focus");
        assert_eq!(requests[1]["params"], json!({"pane_id": "w2:p3"}));
        assert_eq!(requests[2]["method"], "pane.list");
        assert_eq!(requests[2]["params"], json!({}));
    }

    #[test]
    fn server_error_response_surfaces_as_error() {
        let (path, _server) = spawn_fake_server(vec![
            r#"{"id":"1","error":{"code":"tab_not_found","message":"tab w9:t9 not found"}}"#
                .to_string(),
        ]);

        let mut client = SocketClient::connect(&path).unwrap();
        let err = client.focus_tab("w9:t9").unwrap_err();
        assert!(err.to_string().contains("tab_not_found"), "error: {err}");
    }

    #[test]
    fn mismatched_id_from_server_is_an_error() {
        let (path, _server) = spawn_fake_server(vec![
            r#"{"id":"999","result":{"type":"tab_list","tabs":[]}}"#.to_string(),
        ]);

        let mut client = SocketClient::connect(&path).unwrap();
        assert!(client.list_tabs().is_err());
    }

    #[test]
    fn connect_failure_mentions_herdr() {
        let err = SocketClient::connect(Path::new("/nonexistent/herdr.sock")).unwrap_err();
        assert!(
            format!("{err:#}").to_lowercase().contains("herdr"),
            "error should tell the user this must run inside herdr: {err:#}"
        );
    }

    #[test]
    fn eof_before_response_is_an_error_not_a_hang() {
        let (path, _server) = spawn_fake_server(vec![]);

        let mut client = SocketClient::connect(&path).unwrap();
        assert!(client.list_tabs().is_err());
    }
}
