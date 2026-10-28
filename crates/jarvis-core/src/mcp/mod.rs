// Serveurs MCP de Jarvis, lancés par Claude Code (ou AGY) sous la forme `jarvis-core mcp <nom>` :
//   jarvis : approbation des outils, mémoire, journal, précisions, réunions (orchestrator/mcp-server.js) ;
//   system : actions Windows (orchestrator/system-mcp.js) ;
//   google : Gmail et Agenda (orchestrator/google-mcp.js).
// Ils ne décident de rien : chaque appel est relayé à l'orchestrateur (HTTP 127.0.0.1:8799), seul
// détenteur de la mémoire et du droit d'autoriser. `jarvis-core mcp-proxy` assainit les schémas pour Gemini.

pub mod jarvis;
pub mod proxy;
pub mod system;
pub mod google;

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub type J = Value;
pub type Fut<T> = Pin<Box<dyn Future<Output = T>>>;

/// Réponse texte d'un outil MCP.
pub fn text(value: J) -> J {
    let t = match value {
        Value::String(s) => s,
        other => other.to_string(),
    };
    json!({ "content": [{ "type": "text", "text": t }] })
}

pub fn text_s(s: impl Into<String>) -> J {
    text(Value::String(s.into()))
}

/// Port HTTP de l'orchestrateur (approbations, mémoire…).
pub fn port() -> u16 {
    std::env::var("JARVIS_APPROVAL_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8799)
}

/// POST JSON vers l'orchestrateur ; ajoute JARVIS_TASK_ID. Erreur réseau : { error }.
pub async fn post(route: &str, payload: J) -> J {
    let mut data = payload;
    if let Ok(task) = std::env::var("JARVIS_TASK_ID") {
        if !task.is_empty() && data.get("taskId").map(|v| v.is_null()).unwrap_or(true) {
            data["taskId"] = json!(task);
        }
    }
    let client = reqwest::Client::builder().no_proxy().build().unwrap_or_default();
    match client.post(format!("http://127.0.0.1:{}{route}", port())).json(&data).send().await {
        Err(_) => json!({ "error": "Jarvis est injoignable" }),
        Ok(res) => res.json::<J>().await.unwrap_or(json!({ "error": "réponse illisible de Jarvis" })),
    }
}

/// Boucle JSON-RPC sur l'entrée / la sortie standard (une ligne par message).
pub fn serve(name: &'static str, tools: J, call: Rc<dyn Fn(String, J) -> Fut<J>>) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let writer = tokio::task::spawn_local(async move {
            let mut out = tokio::io::stdout();
            while let Some(line) = rx.recv().await {
                let _ = out.write_all(line.as_bytes()).await;
                let _ = out.write_all(b"\n").await;
                let _ = out.flush().await;
            }
        });
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        let mut tasks = vec![];
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(msg) = serde_json::from_str::<J>(line.trim()) else { continue };
            let id = msg.get("id").cloned();
            let method = msg["method"].as_str().unwrap_or("").to_string();
            let params = msg["params"].clone();
            let tx = tx.clone();
            let tools = tools.clone();
            let call = call.clone();
            tasks.push(tokio::task::spawn_local(async move {
                let reply = match method.as_str() {
                    "initialize" => Some(json!({
                        "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": name, "version": "1.0.0" }
                    })),
                    "tools/list" => Some(json!({ "tools": tools })),
                    "tools/call" => {
                        let tool = params["name"].as_str().unwrap_or("").to_string();
                        let args = if params["arguments"].is_object() { params["arguments"].clone() } else { json!({}) };
                        Some(call(tool, args).await)
                    }
                    _ => None,
                };
                let Some(id) = id.filter(|v| !v.is_null()) else { return };
                let msg = match reply {
                    Some(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    None => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("méthode inconnue : {method}") } }),
                };
                let _ = tx.send(msg.to_string());
            }));
        }
        for t in tasks {
            let _ = t.await;
        }
        drop(tx);
        let _ = writer.await;
    });
}

/// `jarvis-core mcp <nom>` et `jarvis-core mcp-proxy <commande> …`.
pub fn main(args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("jarvis") => {
            jarvis::run();
            0
        }
        Some("system") => {
            system::run();
            0
        }
        Some("google") => {
            google::run();
            0
        }
        _ => {
            eprintln!("Usage : jarvis-core mcp <jarvis|system|google>");
            1
        }
    }
}
