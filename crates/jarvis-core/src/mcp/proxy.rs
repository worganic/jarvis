// Proxy assainisseur MCP pour Gemini AGY (orchestrator/mcp-proxy.js) : `jarvis-core mcp-proxy <commande> …`.
// Intercepte les réponses tools/list et corrige les schémas refusés par l'API Gemini (type « array »
// sans « items »). Tout le reste est relayé tel quel, dans les deux sens.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// Assainit récursivement un schéma JSON (« array » sans « items » → items de type string).
pub fn sanitize(schema: &mut Value) {
    let Some(o) = schema.as_object_mut() else { return };
    let is_array = match o.get("type") {
        Some(Value::String(t)) => t == "array",
        Some(Value::Array(l)) => l.iter().any(|x| x == "array"),
        _ => false,
    };
    let has_items = o.get("items").map(|i| !i.is_null() && i != &Value::Bool(false)).unwrap_or(false);
    if is_array && !has_items {
        o.insert("items".into(), json!({ "type": "string" }));
    }
    if let Some(Value::Object(props)) = o.get_mut("properties") {
        for v in props.values_mut() {
            sanitize(v);
        }
    }
    match o.get_mut("items") {
        Some(Value::Array(l)) => l.iter_mut().for_each(sanitize),
        Some(v @ Value::Object(_)) => sanitize(v),
        _ => {}
    }
    for k in ["anyOf", "oneOf", "allOf"] {
        if let Some(Value::Array(l)) = o.get_mut(k) {
            l.iter_mut().for_each(sanitize);
        }
    }
    if let Some(v @ Value::Object(_)) = o.get_mut("additionalProperties") {
        sanitize(v);
    }
}

fn quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".into();
    }
    if !arg.chars().any(|c| c.is_whitespace() || "\"&|<>^()%!".contains(c)) {
        return arg.to_string();
    }
    format!("\"{}\"", arg.replace('"', "\"\""))
}

pub fn main(args: &[String]) -> i32 {
    let Some(target) = args.first() else {
        eprintln!("Usage: jarvis-core mcp-proxy <command> [args...]");
        return 1;
    };
    let line = std::iter::once(quote_arg(target)).chain(args[1..].iter().map(|a| quote_arg(a))).collect::<Vec<_>>().join(" ");
    let mut cmd = Command::new("cmd.exe");
    cmd.args(["/d", "/s", "/c"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(format!("\"{line}\""));
    }
    let mut child = match cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[mcp-proxy] Erreur de lancement ({target}) : {e}");
            return 1;
        }
    };
    // Relais stdin → enfant (fil séparé).
    let mut child_in = child.stdin.take().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut stdin = std::io::stdin();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if child_in.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = child_in.flush();
                }
            }
        }
    });
    // Sortie de l'enfant : les listes d'outils sont assainies, le reste relayé tel quel.
    let out = child.stdout.take().unwrap();
    let mut stdout = std::io::stdout();
    for line in BufReader::new(out).lines() {
        let Ok(line) = line else { break };
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let relay = match serde_json::from_str::<Value>(l) {
            Ok(mut msg) if msg["result"]["tools"].is_array() => {
                if let Some(tools) = msg["result"]["tools"].as_array_mut() {
                    for t in tools.iter_mut() {
                        for k in ["inputSchema", "outputSchema"] {
                            if let Some(s) = t.get_mut(k) {
                                sanitize(s);
                            }
                        }
                    }
                }
                msg.to_string()
            }
            _ => l.to_string(),
        };
        let _ = writeln!(stdout, "{relay}");
        let _ = stdout.flush();
    }
    child.wait().ok().and_then(|s| s.code()).unwrap_or(1)
}
