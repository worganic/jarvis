// Serveur MCP « google » (orchestrator/google-mcp.js) : Gmail et Agenda, relayés à l'orchestrateur.
// Définitions des outils : data/mcp-google-tools.json (exportées de la v3).

use std::rc::Rc;

use serde_json::json;

use super::system::reply;
use super::{post, serve, J};

pub fn run() {
    let tools: J = serde_json::from_str(include_str!("../../data/mcp-google-tools.json")).unwrap_or(json!([]));
    serve(
        "google",
        tools,
        Rc::new(|name, args| {
            Box::pin(async move {
                let action = if name == "google_statut" { "statut".to_string() } else { name };
                let r = post("/google", json!({ "action": action, "params": args })).await;
                reply(&r, "Erreur Google")
            })
        }),
    );
}
