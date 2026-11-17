use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

use crate::hc;

const POLL_INTERVAL_SECS: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentStatus {
    /// herdr の agent panel の "priority" (attention queue) と同じ考え方で、
    /// 手が必要なものほど小さい値を返す。
    pub fn priority(&self) -> u8 {
        match self {
            Self::Blocked => 0,
            Self::Done => 1,
            Self::Working => 2,
            Self::Idle => 3,
            Self::Unknown => 4,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Working => "Working",
            Self::Blocked => "Blocked",
            Self::Done => "Done",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub display_name: String,
    pub status: AgentStatus,
    pub project_name: String,
    pub git_branch: Option<String>,
    pub terminal_title_stripped: Option<String>,
}

impl AgentInfo {
    /// 各バックエンド共通の組み立て。表示名は name → agent → id の順にフォールバックし、
    /// 空の name / title は無いものとして扱う。`git_branch` は cwd から git を呼んで取る。
    pub(crate) fn new(
        name: Option<String>,
        agent: Option<String>,
        id: &str,
        status: &str,
        cwd: Option<&str>,
        title: Option<String>,
    ) -> Self {
        let mut info = Self::without_branch(name, agent, id, status, cwd, title);
        info.git_branch = cwd.and_then(git_branch);
        info
    }

    /// `new` から git 呼び出しを除いた純粋な部分。
    fn without_branch(
        name: Option<String>,
        agent: Option<String>,
        id: &str,
        status: &str,
        cwd: Option<&str>,
        title: Option<String>,
    ) -> Self {
        let display_name = name
            .into_iter()
            .chain(agent)
            .find(|n| !n.is_empty())
            .unwrap_or_else(|| id.to_owned());
        Self {
            display_name,
            status: parse_status(status),
            project_name: cwd.map_or_else(|| "?".into(), project_name),
            git_branch: None,
            terminal_title_stripped: title.filter(|t| !t.is_empty()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HerdrState {
    pub agents: Vec<AgentInfo>,
}

#[derive(Deserialize)]
struct CliResponse {
    result: ResultPayload,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResultPayload {
    AgentList { agents: Vec<RawAgentInfo> },
}

#[derive(Deserialize)]
struct RawAgentInfo {
    terminal_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    agent_status: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    terminal_title_stripped: Option<String>,
}

/// どのセッションのエージェントを表示するか。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionScope {
    /// running な全セッションをまとめて表示する
    #[default]
    All,
    /// オーバーレイを起動したセッションだけを表示する
    Current,
}

#[derive(Deserialize)]
struct SessionListResponse {
    sessions: Vec<SessionEntry>,
}

#[derive(Deserialize)]
struct SessionEntry {
    name: String,
    #[serde(default)]
    running: bool,
}

fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())
}

fn parse_session_list(json: &str) -> Vec<String> {
    serde_json::from_str::<SessionListResponse>(json)
        .map(|r| {
            r.sessions
                .into_iter()
                .filter(|s| s.running)
                .map(|s| s.name)
                .collect()
        })
        .unwrap_or_default()
}

/// `session` を指定しないときは環境変数 (`HERDR_SESSION`) が指すセッションに繋がる。
fn agent_list_args(session: Option<&str>) -> Vec<&str> {
    match session {
        Some(name) => vec!["--session", name, "agent", "list"],
        None => vec!["agent", "list"],
    }
}

fn running_sessions() -> Vec<String> {
    Command::new(herdr_bin())
        .args(["session", "list", "--json"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_session_list(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

fn parse_status(s: &str) -> AgentStatus {
    match s {
        "idle" => AgentStatus::Idle,
        "working" => AgentStatus::Working,
        "blocked" => AgentStatus::Blocked,
        "done" => AgentStatus::Done,
        _ => AgentStatus::Unknown,
    }
}

fn project_name(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string()
}

fn git_branch(cwd: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// 手が必要なものを上に集める。同じステータス内は取得順（セッション順・ペイン順）のまま。
fn sort_agents(agents: &mut [AgentInfo]) {
    agents.sort_by_key(|a| a.status.priority());
}

/// herdr と herdr-core の結果を合わせて並べ替える。
/// 両方取れなかったときだけ None（前回の表示を保つ）。片方だけ取れたならそれだけ表示する。
fn merge(
    herdr: Option<Vec<AgentInfo>>,
    core: Option<Vec<AgentInfo>>,
) -> Option<Vec<AgentInfo>> {
    if herdr.is_none() && core.is_none() {
        return None;
    }
    let mut agents = herdr.unwrap_or_default();
    agents.extend(core.unwrap_or_default());
    sort_agents(&mut agents);
    Some(agents)
}

fn fetch_agents(scope: SessionScope) -> Option<Vec<AgentInfo>> {
    merge(collect_agents(scope), hc::fetch_core_agents())
}

fn collect_agents(scope: SessionScope) -> Option<Vec<AgentInfo>> {
    if scope == SessionScope::Current {
        return fetch_session_agents(None);
    }

    let sessions = running_sessions();
    if sessions.is_empty() {
        // セッション一覧が取れないときは起動元セッションだけにフォールバックする
        return fetch_session_agents(None);
    }

    let mut agents = Vec::new();
    let mut reached_any = false;
    for name in &sessions {
        if let Some(mut found) = fetch_session_agents(Some(name)) {
            reached_any = true;
            agents.append(&mut found);
        }
    }
    reached_any.then_some(agents)
}

fn fetch_session_agents(session: Option<&str>) -> Option<Vec<AgentInfo>> {
    let herdr = herdr_bin();
    let output = Command::new(&herdr)
        .args(agent_list_args(session))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let json = String::from_utf8_lossy(&output.stdout);
    let resp: CliResponse = serde_json::from_str(&json).ok()?;
    match resp.result {
        ResultPayload::AgentList { agents } => Some(
            agents
                .into_iter()
                .map(|a| {
                    AgentInfo::new(
                        a.name,
                        a.agent,
                        &a.terminal_id,
                        &a.agent_status,
                        a.cwd.as_deref(),
                        a.terminal_title_stripped,
                    )
                })
                .collect(),
        ),
    }
}

pub fn start_polling(state: Arc<Mutex<HerdrState>>, scope: SessionScope) {
    std::thread::spawn(move || loop {
        if let Some(agents) = fetch_agents(scope) {
            if let Ok(mut s) = state.lock() {
                s.agents = agents;
            }
        }
        std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_variants() {
        assert_eq!(parse_status("idle"), AgentStatus::Idle);
        assert_eq!(parse_status("working"), AgentStatus::Working);
        assert_eq!(parse_status("blocked"), AgentStatus::Blocked);
        assert_eq!(parse_status("done"), AgentStatus::Done);
        assert_eq!(parse_status("???"), AgentStatus::Unknown);
    }

    #[test]
    fn project_name_extracts_basename() {
        assert_eq!(project_name("/home/user/herdr"), "herdr");
        assert_eq!(project_name("/"), "?");
    }

    #[test]
    fn parse_session_list_returns_running_names() {
        let json = r#"{"sessions":[
            {"default":true,"name":"default","running":true,
             "session_dir":"/x","socket_path":"/x/herdr.sock"},
            {"default":false,"name":"private","running":true,
             "session_dir":"/y","socket_path":"/y/herdr.sock"}
        ]}"#;
        assert_eq!(parse_session_list(json), vec!["default", "private"]);
    }

    #[test]
    fn parse_session_list_skips_stopped_sessions() {
        let json = r#"{"sessions":[
            {"name":"default","running":true},
            {"name":"stopped","running":false}
        ]}"#;
        assert_eq!(parse_session_list(json), vec!["default"]);
    }

    #[test]
    fn parse_session_list_returns_empty_on_broken_json() {
        assert!(parse_session_list("not json").is_empty());
        assert!(parse_session_list("").is_empty());
    }

    #[test]
    fn agent_info_maps_all_fields() {
        let info = AgentInfo::without_branch(
            Some("worker".into()),
            Some("claude-code".into()),
            "t9",
            "blocked",
            Some("/home/x/proj"),
            Some("タイトル".into()),
        );
        assert_eq!(info.display_name, "worker");
        assert_eq!(info.status, AgentStatus::Blocked);
        assert_eq!(info.project_name, "proj");
        assert_eq!(info.git_branch, None);
        assert_eq!(info.terminal_title_stripped.as_deref(), Some("タイトル"));
    }

    #[test]
    fn agent_info_display_name_falls_back_to_agent_then_id() {
        let info = AgentInfo::without_branch(None, Some("claude-code".into()), "t9", "idle", None, None);
        assert_eq!(info.display_name, "claude-code");
        let info = AgentInfo::without_branch(None, None, "t9", "idle", None, None);
        assert_eq!(info.display_name, "t9");
    }

    #[test]
    fn agent_info_treats_empty_name_as_missing() {
        let info = AgentInfo::without_branch(Some(String::new()), Some("a".into()), "t9", "idle", None, None);
        assert_eq!(info.display_name, "a");
    }

    #[test]
    fn agent_info_treats_empty_agent_as_missing() {
        let info = AgentInfo::without_branch(None, Some(String::new()), "t9", "idle", None, None);
        assert_eq!(info.display_name, "t9");
    }

    #[test]
    fn agent_info_without_cwd_uses_question_mark() {
        let info = AgentInfo::without_branch(None, None, "t9", "working", None, None);
        assert_eq!(info.project_name, "?");
    }

    #[test]
    fn agent_info_drops_empty_title() {
        let info = AgentInfo::without_branch(None, None, "t9", "done", None, Some(String::new()));
        assert_eq!(info.terminal_title_stripped, None);
        assert_eq!(info.status, AgentStatus::Done);
    }

    #[test]
    fn agent_info_new_without_cwd_has_no_branch() {
        let info = AgentInfo::new(None, None, "t9", "idle", None, None);
        assert_eq!(info.git_branch, None);
    }

    fn agent(status: AgentStatus, name: &str) -> AgentInfo {
        AgentInfo {
            display_name: name.to_owned(),
            status,
            project_name: name.to_owned(),
            git_branch: None,
            terminal_title_stripped: None,
        }
    }

    fn names(agents: &[AgentInfo]) -> Vec<&str> {
        agents.iter().map(|a| a.project_name.as_str()).collect()
    }

    #[test]
    fn attention_priority_orders_blocked_first_and_idle_last() {
        assert!(AgentStatus::Blocked.priority() < AgentStatus::Done.priority());
        assert!(AgentStatus::Done.priority() < AgentStatus::Working.priority());
        assert!(AgentStatus::Working.priority() < AgentStatus::Idle.priority());
        assert!(AgentStatus::Idle.priority() <= AgentStatus::Unknown.priority());
    }

    #[test]
    fn sort_agents_puts_the_ones_needing_attention_on_top() {
        let mut agents = vec![
            agent(AgentStatus::Idle, "idle"),
            agent(AgentStatus::Working, "working"),
            agent(AgentStatus::Unknown, "unknown"),
            agent(AgentStatus::Done, "done"),
            agent(AgentStatus::Blocked, "blocked"),
        ];
        sort_agents(&mut agents);
        assert_eq!(
            names(&agents),
            ["blocked", "done", "working", "idle", "unknown"]
        );
    }

    #[test]
    fn sort_agents_keeps_original_order_within_a_status() {
        let mut agents = vec![
            agent(AgentStatus::Working, "w1"),
            agent(AgentStatus::Blocked, "b1"),
            agent(AgentStatus::Working, "w2"),
            agent(AgentStatus::Blocked, "b2"),
        ];
        sort_agents(&mut agents);
        assert_eq!(names(&agents), ["b1", "b2", "w1", "w2"]);
    }

    #[test]
    fn agent_list_args_without_session() {
        assert_eq!(agent_list_args(None), vec!["agent", "list"]);
    }

    #[test]
    fn agent_list_args_with_session() {
        assert_eq!(
            agent_list_args(Some("private")),
            vec!["--session", "private", "agent", "list"]
        );
    }

    #[test]
    fn merge_returns_none_when_both_backends_fail() {
        assert!(merge(None, None).is_none());
    }

    #[test]
    fn merge_uses_only_herdr_when_core_is_unavailable() {
        let herdr = vec![agent(AgentStatus::Idle, "i"), agent(AgentStatus::Blocked, "b")];
        let merged = merge(Some(herdr), None).unwrap();
        assert_eq!(names(&merged), ["b", "i"]);
    }

    #[test]
    fn merge_uses_only_core_when_herdr_is_unavailable() {
        let core = vec![agent(AgentStatus::Working, "w")];
        let merged = merge(None, Some(core)).unwrap();
        assert_eq!(names(&merged), ["w"]);
    }

    #[test]
    fn merge_keeps_empty_list_when_a_backend_succeeds_with_no_agents() {
        assert_eq!(merge(Some(vec![]), None).unwrap().len(), 0);
        assert_eq!(merge(None, Some(vec![])).unwrap().len(), 0);
    }

    #[test]
    fn merge_sorts_both_and_puts_herdr_first_within_a_status() {
        let herdr = vec![
            agent(AgentStatus::Working, "herdr-w"),
            agent(AgentStatus::Blocked, "herdr-b"),
        ];
        let core = vec![
            agent(AgentStatus::Working, "core-w"),
            agent(AgentStatus::Blocked, "core-b"),
            agent(AgentStatus::Done, "core-d"),
        ];
        let merged = merge(Some(herdr), Some(core)).unwrap();
        assert_eq!(
            names(&merged),
            ["herdr-b", "core-b", "core-d", "herdr-w", "core-w"]
        );
    }
}
