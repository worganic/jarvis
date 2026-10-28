// Client MCP minimal (orchestrator/mcp-client.js) : tester un connecteur depuis l'administration et lister
// ses outils (section Sécurité). Jarvis n'appelle pas les outils lui-même : c'est Claude Code (ou AGY).

use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::js::{self, J};

const PROTOCOL_VERSION: &str = "2025-06-18";

fn client_info() -> J {
    json!({ "name": "jarvis-admin", "version": "1.0.0" })
}

/// Sous Windows, npx / uvx / scripts .cmd exigent un shell : arguments cités pour cmd.exe.
pub fn quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".into();
    }
    if !crate::re!(r#"[\s"&|<>^()%!]"#).is_match(arg) {
        return arg.to_string();
    }
    format!("\"{}\"", arg.replace('"', "\"\""))
}

/// Commande lancée par cmd.exe (comme `spawn(line, { shell: true })` de Node).
pub fn shell_command(command: &str, args: &[String], env: &Map<String, Value>) -> tokio::process::Command {
    let line = std::iter::once(quote_arg(command)).chain(args.iter().map(|a| quote_arg(a))).collect::<Vec<_>>().join(" ");
    let mut cmd = tokio::process::Command::new("cmd.exe");
    cmd.args(["/d", "/s", "/c"]);
    #[cfg(windows)]
    cmd.raw_arg(format!("\"{line}\""));
    for (k, v) in env {
        cmd.env(k, js::s(v));
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    cmd
}

/// Connexion stdio : initialize → notifications/initialized → tools/list (paginé).
pub async fn list_tools_stdio(command: &str, args: &[String], env: &Map<String, Value>, timeout_ms: u64) -> J {
    let started = Instant::now();
    let mut child = match shell_command(command, args, env).spawn() {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": format!("lancement impossible : {e}") }),
    };
    let pid = child.id().unwrap_or(0);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let stderr_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s).await;
        s
    });
    let mut lines = BufReader::new(stdout).lines();
    let mut next_id = 1u64;
    let work = async {
        let mut request = |method: &str, params: J| {
            let id = next_id;
            next_id += 1;
            (id, format!("{}\n", json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })))
        };
        // Réponse d'une requête : les autres lignes (journal du serveur sur stdout) sont ignorées.
        async fn wait(lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>, id: u64) -> Result<J, String> {
            loop {
                let line = lines.next_line().await.map_err(|e| e.to_string())?.ok_or("le serveur s'est arrêté")?;
                let Ok(msg) = serde_json::from_str::<J>(line.trim()) else { continue };
                if msg["id"].as_u64() == Some(id) {
                    if msg["error"].is_object() {
                        return Err(if js::truthy(&msg["error"]["message"]) { js::gs(&msg["error"], "message") } else { "erreur MCP".into() });
                    }
                    return Ok(msg["result"].clone());
                }
            }
        }
        let (id, line) = request("initialize", json!({ "protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": client_info() }));
        stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
        let init = wait(&mut lines, id).await?;
        stdin.write_all(format!("{}\n", json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).as_bytes()).await.map_err(|e| e.to_string())?;
        let mut tools: Vec<J> = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let (id, line) = request("tools/list", params);
            stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
            let res = wait(&mut lines, id).await?;
            tools.extend(js::arr(&res["tools"]));
            cursor = res["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok::<(Vec<J>, J), String>((tools, init["serverInfo"].clone()))
    };
    let result = tokio::time::timeout(Duration::from_millis(timeout_ms), work).await;
    crate::proc::kill_tree(pid);
    let _ = child.kill().await;
    let stderr = tokio::time::timeout(Duration::from_millis(1000), stderr_task).await.ok().and_then(|r| r.ok()).unwrap_or_default();
    let n = stderr.chars().count();
    let tail: String = stderr.chars().skip(n.saturating_sub(2000)).collect();
    let ms = started.elapsed().as_millis() as u64;
    match result {
        Err(_) => json!({ "ok": false, "error": format!("délai dépassé ({} s)", (timeout_ms as f64 / 1000.0).round()), "ms": ms, "stderr": tail }),
        Ok(Err(e)) => json!({ "ok": false, "error": e, "ms": ms, "stderr": tail }),
        Ok(Ok((tools, info))) => json!({ "ok": true, "tools": tools, "serverInfo": info, "ms": ms, "stderr": tail }),
    }
}

/// Transport HTTP « streamable » : réponses JSON ou flux SSE, session via l'en-tête Mcp-Session-Id.
pub async fn list_tools_http(url: &str, headers: &HashMap<String, String>, timeout_ms: u64) -> J {
    let started = Instant::now();
    let client = crate::net::http();
    let mut session: Option<String> = None;
    let mut next_id = 1u64;
    let mut call = |method: &'static str, params: J, notify: bool| {
        let body = if notify { json!({ "jsonrpc": "2.0", "method": method, "params": params }) } else {
            let b = json!({ "jsonrpc": "2.0", "id": next_id, "method": method, "params": params });
            next_id += 1;
            b
        };
        (body, notify)
    };
    async fn send(client: &reqwest::Client, url: &str, headers: &HashMap<String, String>, session: &mut Option<String>, body: J, notify: bool, timeout_ms: u64) -> Result<J, String> {
        let mut req = client
            .post(url)
            .timeout(Duration::from_millis(timeout_ms))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION);
        if let Some(s) = session.as_ref() {
            req = req.header("Mcp-Session-Id", s);
        }
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let res = req.body(body.to_string()).send().await.map_err(|e| if e.is_timeout() { "délai dépassé".to_string() } else { e.to_string() })?;
        if let Some(sid) = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *session = Some(sid.to_string());
        }
        if notify {
            return Ok(Value::Null);
        }
        if !res.status().is_success() {
            return Err(format!("HTTP {}", res.status().as_u16()));
        }
        let sse = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").contains("text/event-stream");
        let text = res.text().await.map_err(|e| e.to_string())?;
        let mut msg: Option<J> = None;
        if sse {
            for line in text.split('\n') {
                if let Some(d) = line.strip_prefix("data:") {
                    if let Ok(p) = serde_json::from_str::<J>(d.trim()) {
                        if p.get("id").is_some() {
                            msg = Some(p);
                        }
                    }
                }
            }
        } else {
            msg = serde_json::from_str(&text).ok();
        }
        let msg = msg.ok_or("réponse MCP vide")?;
        if msg["error"].is_object() {
            return Err(if js::truthy(&msg["error"]["message"]) { js::gs(&msg["error"], "message") } else { "erreur MCP".into() });
        }
        Ok(msg["result"].clone())
    }
    let r: Result<(Vec<J>, J), String> = async {
        let (b, n) = call("initialize", json!({ "protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": client_info() }), false);
        let init = send(&client, url, headers, &mut session, b, n, timeout_ms).await?;
        let (b, n) = call("notifications/initialized", json!({}), true);
        send(&client, url, headers, &mut session, b, n, timeout_ms).await?;
        let mut tools = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let (b, n) = call("tools/list", params, false);
            let res = send(&client, url, headers, &mut session, b, n, timeout_ms).await?;
            tools.extend(js::arr(&res["tools"]));
            cursor = res["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok((tools, init["serverInfo"].clone()))
    }
    .await;
    let ms = started.elapsed().as_millis() as u64;
    match r {
        Ok((tools, info)) => json!({ "ok": true, "tools": tools, "serverInfo": info, "ms": ms }),
        Err(e) => json!({ "ok": false, "error": e, "ms": ms }),
    }
}

/// Commande ponctuelle (authentification d'un connecteur : ouvre le navigateur de l'utilisateur).
pub async fn run_once(command: &str, args: &[String], env: &Map<String, Value>, timeout_ms: u64) -> J {
    let mut child = match shell_command(command, args, env).spawn() {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e.to_string(), "output": "" }),
    };
    drop(child.stdin.take());
    let pid = child.id().unwrap_or(0);
    match tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await {
        Err(_) => {
            crate::proc::kill_tree(pid);
            json!({ "ok": false, "error": "délai dépassé", "output": "" })
        }
        Ok(Err(e)) => json!({ "ok": false, "error": e.to_string(), "output": "" }),
        Ok(Ok(o)) => {
            let out = format!("{}{}", crate::proc::decode(&o.stdout), crate::proc::decode(&o.stderr));
            let n = out.chars().count();
            let tail: String = out.chars().skip(n.saturating_sub(4000)).collect();
            json!({ "ok": o.status.success(), "code": o.status.code(), "output": tail })
        }
    }
}
