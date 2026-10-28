// Serveur MCP « system » (orchestrator/system-mcp.js) : actions Windows relayées à l'orchestrateur, qui
// applique la politique de sécurité, sollicite le code si nécessaire et trace l'action.
// Définitions des outils : data/mcp-system-tools.json (exportées de la v3).

use std::rc::Rc;

use serde_json::json;

use super::{post, serve, text, text_s, J};

/// Réponse d'une action relayée : erreur, message, ou résultat complet (JSON indenté).
pub fn reply(result: &J, error_prefix: &str) -> J {
    if result.get("error").map(|e| !e.is_null()).unwrap_or(false) {
        let e = result["error"].as_str().map(String::from).unwrap_or_else(|| result["error"].to_string());
        return text_s(format!("{error_prefix} : {e}"));
    }
    if let Some(m) = result.get("message").filter(|m| !m.is_null() && m != &&J::String(String::new())) {
        return text(m.clone());
    }
    text_s(serde_json::to_string_pretty(result).unwrap_or_default())
}

pub fn run() {
    let tools: J = serde_json::from_str(include_str!("../../data/mcp-system-tools.json")).unwrap_or(json!([]));
    serve(
        "system",
        tools,
        Rc::new(|name, args| {
            Box::pin(async move {
                let r = post("/system", json!({ "action": name, "params": args })).await;
                reply(&r, "Erreur système")
            })
        }),
    );
}
