use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

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

fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())
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

fn fetch_agents() -> Option<Vec<AgentInfo>> {
    let herdr = herdr_bin();
    let output = Command::new(&herdr)
        .args(["agent", "list"])
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
                    let display_name = a
                        .name
                        .or(a.agent)
                        .unwrap_or_else(|| a.terminal_id.clone());
                    let proj = a
                        .cwd
                        .as_deref()
                        .map_or_else(|| "?".into(), project_name);
                    let branch = a.cwd.as_deref().and_then(git_branch);
                    AgentInfo {
                        display_name,
                        status: parse_status(&a.agent_status),
                        project_name: proj,
                        git_branch: branch,
                        terminal_title_stripped: a.terminal_title_stripped,
                    }
                })
                .collect(),
        ),
    }
}

pub fn start_polling(state: Arc<Mutex<HerdrState>>) {
    std::thread::spawn(move || loop {
        if let Some(agents) = fetch_agents() {
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
}
