//! herdr-core (`hc` CLI) バックエンド。

use std::process::Command;

use serde::Deserialize;

use crate::herdr::AgentInfo;

/// `hc ls --json` の terminal。agent かどうかは `status` の有無で決まる。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct HcTerminal {
    address: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Deserialize)]
struct LsResponse {
    terminals: Vec<HcTerminal>,
}

/// `hc get <address> --json` の title / name。
#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
struct GetResponse {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// `status` を持たない terminal（ただのシェルなど）は agent ではないので捨てる。
fn parse_hc_ls(json: &str) -> Option<Vec<HcTerminal>> {
    let resp: LsResponse = serde_json::from_str(json).ok()?;
    Some(
        resp.terminals
            .into_iter()
            .filter(|t| t.status.is_some())
            .collect(),
    )
}

/// `hc get <address> --json` をパースする。壊れていれば空の `GetResponse`。
fn parse_hc_get(json: &str) -> GetResponse {
    serde_json::from_str(json).unwrap_or_default()
}

fn hc_output(args: &[&str]) -> Option<String> {
    let output = Command::new("hc").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// herdr-core の agent 一覧を取る。`hc` が無い・失敗・パース不能なら None。
/// agent ごとの `hc get` と git 呼び出しは重いので、スレッドを分けて並行に実行する（順序は保つ）。
pub fn fetch_core_agents() -> Option<Vec<AgentInfo>> {
    let terminals = parse_hc_ls(&hc_output(&["ls", "--json"])?)?;
    Some(std::thread::scope(|scope| {
        let handles: Vec<_> = terminals
            .iter()
            .map(|t| scope.spawn(|| fetch_core_agent(t)))
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect()
    }))
}

fn fetch_core_agent(t: &HcTerminal) -> AgentInfo {
    let GetResponse { title, name } = hc_output(&["get", &t.address, "--json"])
        .map(|json| parse_hc_get(&json))
        .unwrap_or_default();
    AgentInfo::new(
        name,
        t.agent.clone(),
        &t.address,
        t.status.as_deref().unwrap_or_default(),
        t.cwd.as_deref(),
        title,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LS_JSON: &str = r#"{"hidden_finished":37,"terminals":[
        {"address":"t1","agent":"zsh","cwd":"/home/maedana","state":"live","turn":0,"where":"~ / 1"},
        {"address":"t4","agent":"claude-code","cwd":"/home/x/proj","state":"live","status":"blocked","turn":20,"where":"~ / 1"},
        {"address":"t5","state":"live","status":"working","turn":1}
    ]}"#;

    #[test]
    fn parse_hc_ls_keeps_only_terminals_with_status() {
        let terminals = parse_hc_ls(LS_JSON).unwrap();
        assert_eq!(terminals.len(), 2);
        assert_eq!(
            terminals[0],
            HcTerminal {
                address: "t4".into(),
                agent: Some("claude-code".into()),
                cwd: Some("/home/x/proj".into()),
                status: Some("blocked".into()),
            }
        );
    }

    #[test]
    fn parse_hc_ls_allows_missing_agent_and_cwd() {
        let terminals = parse_hc_ls(LS_JSON).unwrap();
        assert_eq!(terminals[1].address, "t5");
        assert_eq!(terminals[1].agent, None);
        assert_eq!(terminals[1].cwd, None);
        assert_eq!(terminals[1].status.as_deref(), Some("working"));
    }

    #[test]
    fn parse_hc_ls_returns_empty_when_no_agents() {
        let json = r#"{"hidden_finished":0,"terminals":[{"address":"t1","agent":"zsh"}]}"#;
        assert_eq!(parse_hc_ls(json), Some(vec![]));
    }

    #[test]
    fn parse_hc_ls_returns_none_on_broken_json() {
        assert_eq!(parse_hc_ls("not json"), None);
        assert_eq!(parse_hc_ls(""), None);
        assert_eq!(parse_hc_ls(r#"{"hidden_finished":1}"#), None);
    }

    #[test]
    fn parse_hc_get_extracts_title_and_name() {
        let json = r#"{"address":"t4","name":"my-agent","title":"herdr-app の設定を dotfiles で管理","cols":66}"#;
        let got = parse_hc_get(json);
        assert_eq!(got.title.as_deref(), Some("herdr-app の設定を dotfiles で管理"));
        assert_eq!(got.name.as_deref(), Some("my-agent"));
    }

    #[test]
    fn parse_hc_get_treats_null_name_as_none() {
        let json = r#"{"address":"t4","name":null,"title":"t"}"#;
        assert_eq!(
            parse_hc_get(json),
            GetResponse { title: Some("t".into()), name: None }
        );
    }

    #[test]
    fn parse_hc_get_returns_default_for_empty_or_broken_json() {
        assert_eq!(parse_hc_get(""), GetResponse::default());
        assert_eq!(parse_hc_get("not json"), GetResponse::default());
    }
}
