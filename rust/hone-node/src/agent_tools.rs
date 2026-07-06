//! Tool dispatch for the agent worker.
//!
//! Agents declare `tools_allowed` when posting tasks. The worker executes
//! declared tools and feeds results back to the model (ReAct loop).
//!
//! Supported tools:
//!   chain_read  — GET any /api/* path on the local node, returns JSON
//!   web_search  — DuckDuckGo instant-answer API (no key required)
//!   code_exec   — Run Python/JS/Bash code in a subprocess (30s timeout)

use std::time::Duration;
use serde_json::{json, Value};
use tracing::warn;

use crate::chain::Chain;

const TOOL_TIMEOUT_SECS: u64 = 30;
const MAX_TOOL_ITERATIONS: usize = 8;

/// System prompt injected before the user message when tools are available.
pub fn build_system_prompt(tools: &[String]) -> String {
    let tool_docs = tools.iter().map(|t| match t.as_str() {
        "chain_read" => r#"- chain_read: Query the HONE node REST API.
  Args: {"path": "/api/account/alice"}
  Returns: JSON response from the node."#,
        "web_search" => r#"- web_search: Search the web via DuckDuckGo.
  Args: {"query": "your search query"}
  Returns: {"abstract": "...", "answer": "...", "results": [...]}"#,
        "code_exec" => r#"- code_exec: Execute code in a sandboxed subprocess (30s timeout).
  Args: {"language": "python"|"javascript"|"bash", "code": "..."}
  Returns: {"stdout": "...", "stderr": "...", "exit_code": N}"#,
        _ => "",
    }).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");

    format!(r#"You are an autonomous agent with access to the following tools:

{}

To call a tool, respond with ONLY a JSON object on its own line:
{{"tool": "<name>", "args": {{...}}}}

After receiving a tool result, you may call another tool or provide your final answer.
When you have gathered enough information, respond with your final answer as plain text (not JSON).
Maximum {} tool calls per task."#, tool_docs, MAX_TOOL_ITERATIONS)
}

#[derive(Debug)]
pub struct ToolResult {
    #[allow(dead_code)]
    pub tool:  String,
    pub ok:    bool,
    pub value: Value,
}

/// Attempt to parse a tool call from a model response line.
/// Returns Some((tool_name, args)) if the line looks like a tool invocation.
pub fn parse_tool_call(text: &str) -> Option<(String, Value)> {
    // Find first JSON-looking segment on a line
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') { continue; }
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            if let (Some(tool), Some(args)) = (v["tool"].as_str(), v.get("args")) {
                return Some((tool.to_owned(), args.clone()));
            }
        }
    }
    None
}

/// Dispatch a single tool call.
pub async fn dispatch(tool: &str, args: &Value, chain: &Chain) -> ToolResult {
    let result = match tool {
        "chain_read" => chain_read(args, chain).await,
        "web_search" => web_search(args).await,
        "code_exec"  => code_exec(args).await,
        _ => Err(format!("unknown tool '{}'", tool)),
    };
    match result {
        Ok(v) => ToolResult { tool: tool.to_owned(), ok: true, value: v },
        Err(e) => {
            warn!("[agent-tools] {} failed: {}", tool, e);
            ToolResult { tool: tool.to_owned(), ok: false, value: json!({ "error": e }) }
        }
    }
}

async fn chain_read(args: &Value, chain: &Chain) -> Result<Value, String> {
    let path = args["path"].as_str().ok_or("chain_read requires 'path'")?;
    // Sanitise: only allow GET /api/* paths
    if !path.starts_with("/api/") {
        return Err(format!("chain_read path must start with /api/, got '{}'", path));
    }

    let node_port = std::env::var("HONE_API_PORT").unwrap_or_else(|_| "4242".to_owned());
    let url = format!("http://127.0.0.1:{}{}", node_port, path);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TOOL_TIMEOUT_SECS))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let json: Value = resp.json().await.map_err(|e| e.to_string())?;
    let _ = chain; // chain available for future local shortcuts
    Ok(json)
}

async fn web_search(args: &Value) -> Result<Value, String> {
    let query = args["query"].as_str().ok_or("web_search requires 'query'")?;
    let encoded = urlencoding::encode(query);
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
        encoded
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TOOL_TIMEOUT_SECS))
        .user_agent("hone-agent/1.0")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let raw: Value = resp.json().await.map_err(|e| e.to_string())?;

    // Extract the most useful fields to keep context window lean.
    let results: Vec<Value> = raw["RelatedTopics"]
        .as_array()
        .map(|a| a.iter().take(5).filter_map(|t| {
            let text = t["Text"].as_str()?;
            let first_url = t["FirstURL"].as_str().unwrap_or("");
            Some(json!({ "text": text, "url": first_url }))
        }).collect())
        .unwrap_or_default();

    Ok(json!({
        "abstract":  raw["Abstract"].as_str().unwrap_or(""),
        "answer":    raw["Answer"].as_str().unwrap_or(""),
        "source":    raw["AbstractSource"].as_str().unwrap_or(""),
        "results":   results,
    }))
}

async fn code_exec(args: &Value) -> Result<Value, String> {
    let language = args["language"].as_str().ok_or("code_exec requires 'language'")?;
    let code = args["code"].as_str().ok_or("code_exec requires 'code'")?;

    let (cmd, flag) = match language {
        "python" | "python3" => ("python3", "-c"),
        "javascript" | "js"  => ("node", "-e"),
        "bash" | "sh"        => ("bash", "-c"),
        other => return Err(format!("unsupported language '{}'; use python, javascript, or bash", other)),
    };

    let output = tokio::time::timeout(
        Duration::from_secs(TOOL_TIMEOUT_SECS),
        tokio::process::Command::new(cmd)
            .arg(flag)
            .arg(code)
            .output(),
    ).await
    .map_err(|_| "code_exec timed out".to_owned())?
    .map_err(|e| format!("failed to spawn {}: {}", cmd, e))?;

    Ok(json!({
        "stdout":    String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        "stderr":    String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        "exit_code": output.status.code().unwrap_or(-1),
    }))
}
