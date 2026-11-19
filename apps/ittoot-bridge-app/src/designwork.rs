//! The Design page's slow work: talking to the design agent (a local HTTP service in the AI Office).

use std::time::Duration;

/// The agent's default address (design-agent's own default port).
pub const DEFAULT_URL: &str = "http://127.0.0.1:9740";
const QUICK: Duration = Duration::from_secs(4);
/// A chat turn can run many tool calls and model replies.
const LONG: Duration = Duration::from_secs(900);

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build().into()
}

fn finish(mut resp: ureq::http::Response<ureq::Body>) -> Result<serde_json::Value, String> {
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| format!("the agent answered {status} with something that is not JSON"))?;
    if status >= 400 {
        return Err(v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("the agent answered {status}")));
    }
    Ok(v)
}

fn base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

pub fn get(url: &str, path: &str) -> Result<serde_json::Value, String> {
    let resp = agent(QUICK).get(&format!("{}{path}", base(url))).call().map_err(|e| format!("cannot reach the design agent at {}: {e}", base(url)))?;
    finish(resp)
}

pub fn post(url: &str, path: &str, body: &serde_json::Value, long: bool) -> Result<serde_json::Value, String> {
    let resp = agent(if long { LONG } else { Duration::from_secs(120) })
        .post(&format!("{}{path}", base(url)))
        .header("content-type", "application/json")
        .send(body.to_string())
        .map_err(|e| format!("cannot reach the design agent at {}: {e}", base(url)))?;
    finish(resp)
}

/// Open a file or address in the default program.
pub fn open(target: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", target]).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}
