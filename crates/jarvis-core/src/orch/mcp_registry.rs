// Connecteurs MCP externes (orchestrator/mcp-registry.js) : Spotify, Google Workspace, Chrome, dossiers…
// Définitions dans data/mcp-servers.json (inaccessible aux modèles) ; secrets chiffrés par Windows (DPAPI).
// Chaque outil d'un connecteur devient un type d'action de la section Sécurité : « lecture seule »
// = accord direct par défaut, tout le reste = code à 4 chiffres par défaut.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{json, Map, Value};

use crate::js::{self, J};
use crate::secret_box::SecretBox;

const RESERVED: &[&str] = &["jarvis", "system", "google"];
pub const MASK: &str = "••••••••";

pub fn presets() -> &'static Map<String, Value> {
    static P: std::sync::LazyLock<Map<String, Value>> = std::sync::LazyLock::new(|| {
        let v = json!({
            "spotify": {
                "name": "Spotify",
                "description": "Facultatif, pour les demandes complexes confiées au modèle (gérer la bibliothèque, créer une playlist). Pour simplement lancer de la musique, « Spotify intégré » plus haut suffit et ne consomme aucun jeton. Par cette voie, le pilotage de la lecture exige un compte Premium.",
                "widget": "music", "transport": "stdio", "command": "npx", "args": ["-y", "@novalux12/spotify-mcp@latest"],
                "env": [{ "key": "SPOTIFY_CLIENT_ID", "label": "Client ID de ton app Spotify", "required": true, "secret": false }],
                "auth": { "args": ["-y", "@novalux12/spotify-mcp@latest", "auth"], "label": "Se connecter à Spotify" },
                "setup": [
                    "Crée une app sur developer.spotify.com/dashboard.",
                    "Ajoute exactement ce Redirect URI : http://127.0.0.1:8888/callback",
                    "Copie le Client ID ci-dessous, enregistre, puis clique sur « Se connecter à Spotify » (une seule fois)."
                ],
                "link": "https://developer.spotify.com/dashboard"
            },
            "google": {
                "name": "Google Workspace",
                "description": "Gmail, Agenda et Drive (lecture, recherche, envoi, création). Plus complet que le connecteur Gmail intégré.",
                "widget": "workspace", "agy": true, "agyNote": null, "transport": "stdio", "command": "npx", "args": ["-y", "@dguido/google-workspace-mcp"],
                "env": [
                    { "key": "GOOGLE_CLIENT_ID", "label": "Client ID OAuth (application de bureau)", "required": true, "secret": false },
                    { "key": "GOOGLE_CLIENT_SECRET", "label": "Client Secret OAuth", "required": true, "secret": true },
                    { "key": "GOOGLE_WORKSPACE_SERVICES", "label": "Services activés", "required": false, "secret": false, "default": "gmail,calendar,drive" }
                ],
                "auth": { "args": ["-y", "@dguido/google-workspace-mcp", "auth"], "label": "Se connecter à Google" },
                "setup": [
                    "Dans console.cloud.google.com, active les API Gmail, Calendar et Drive.",
                    "Crée un identifiant OAuth 2.0 de type « Application de bureau ».",
                    "Copie Client ID et Client Secret ci-dessous, enregistre, puis « Se connecter à Google »."
                ],
                "link": "https://console.cloud.google.com/apis/credentials"
            },
            "chrome": {
                "name": "Chrome",
                "description": "Piloter un navigateur Chrome : ouvrir une page, lire son contenu, cliquer, remplir un formulaire, capturer.",
                "widget": "browser", "transport": "stdio", "command": "npx", "args": ["-y", "chrome-devtools-mcp@latest"], "env": [],
                "setup": ["Aucune configuration : Chrome s’ouvre au premier outil utilisé (profil dédié)."]
            },
            "filesystem": {
                "name": "Dossier",
                "description": "Donner accès à un dossier précis (lecture, écriture, recherche) par un serveur MCP dédié.",
                "widget": null, "transport": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem"], "env": [],
                "params": [{ "key": "path", "label": "Dossier autorisé (chemin absolu)", "required": true }],
                "setup": ["Le dossier est ajouté à la fin de la commande. Les zones de Jarvis ne s’appliquent pas à ce serveur : choisis un dossier précis."]
            },
            "custom": {
                "name": "Personnalisé",
                "description": "N’importe quel serveur MCP : commande locale (stdio) ou adresse HTTP.",
                "widget": null, "transport": "stdio", "command": "", "args": [], "env": []
            }
        });
        v.as_object().cloned().unwrap()
    });
    &P
}

fn preset(id: &str) -> &'static Value {
    presets().get(id).unwrap_or_else(|| presets().get("custom").unwrap())
}

pub fn read_only_name(name: &str) -> bool {
    crate::re!(r"^(?i:get|list|search|find|read|fetch|query|describe|show|current|now|status|lookup|browse|view|count|check|doctor|info|recent|whoami)(?:_|$|[A-Z])").is_match(name)
}

fn read_only_fr(name: &str) -> bool {
    crate::re!(r"(?i)(?:^|_)(?:lire|chercher|rechercher|consulter|lister|voir|obtenir)(?:_|$)").is_match(name)
}

pub fn slug(text: &str) -> String {
    let t = js::fold_lower(text);
    let t = crate::re!(r"[^a-z0-9]+").replace_all(&t, "_");
    let t = js::slice_chars(crate::re!(r"^_+|_+$").replace_all(&t, "").as_ref(), 32);
    if t.is_empty() { "connecteur".into() } else { t }
}

pub struct McpRegistry {
    file: PathBuf,
    sbox: Option<Rc<SecretBox>>,
    pub servers: RefCell<Vec<J>>,
}

impl McpRegistry {
    pub fn new(file: PathBuf, sbox: Option<Rc<SecretBox>>) -> Self {
        let me = Self { file, sbox: sbox.filter(|b| b.available()), servers: RefCell::new(vec![]) };
        me.load();
        me
    }

    /// Chiffrement des secrets disponible (DPAPI).
    pub fn encryption(&self) -> bool {
        self.sbox.is_some()
    }

    pub fn load(&self) {
        *self.servers.borrow_mut() = js::read_json(&self.file).map(|r| js::arr(&r["servers"])).unwrap_or_default();
    }

    fn save(&self) -> Result<(), String> {
        js::write_json(&self.file, &json!({ "servers": *self.servers.borrow() })).map_err(|e| e.to_string())
    }

    pub fn presets_list(&self) -> Vec<J> {
        presets()
            .iter()
            .map(|(id, p)| {
                json!({
                    "id": id, "name": p["name"], "description": p["description"], "widget": p["widget"], "transport": p["transport"],
                    "command": p["command"], "args": p["args"], "env": p["env"], "params": if p["params"].is_array() { p["params"].clone() } else { json!([]) },
                    "setup": if p["setup"].is_array() { p["setup"].clone() } else { json!([]) }, "link": p["link"],
                    "auth": if p["auth"].is_object() { json!({ "label": p["auth"]["label"] }) } else { Value::Null },
                    "agy": p["agy"] != json!(false), "agyNote": p["agyNote"]
                })
            })
            .collect()
    }

    // --- Secrets ---------------------------------------------------------------------------------

    fn seal_value(&self, value: &str, secret: bool) -> J {
        if !secret || value.is_empty() {
            return json!({ "value": value });
        }
        match self.sbox.as_ref().and_then(|b| b.encrypt(value).ok()) {
            Some(enc) => json!({ "enc": enc }),
            None => json!({ "value": value, "plain": true }),
        }
    }

    fn open_value(&self, stored: &J) -> String {
        if !stored.is_object() {
            return String::new();
        }
        if js::truthy(&stored["enc"]) {
            return self.sbox.as_ref().and_then(|b| b.decrypt(&js::gs(stored, "enc")).ok()).unwrap_or_default();
        }
        js::gs(stored, "value")
    }

    pub fn env_of(&self, server: &J) -> Map<String, Value> {
        let mut env = Map::new();
        if let Some(e) = server["env"].as_object() {
            for (k, stored) in e {
                let v = self.open_value(stored);
                if !v.is_empty() {
                    env.insert(k.clone(), json!(v));
                }
            }
        }
        env
    }

    // --- Lecture pour l'administration (secrets masqués) ---------------------------------------

    pub fn list(&self) -> Vec<J> {
        self.servers.borrow().iter().map(|s| self.public_view(s)).collect()
    }

    pub fn public_view(&self, s: &J) -> J {
        let p = preset(&js::gs(s, "preset"));
        let mut env = Map::new();
        if let Some(e) = s["env"].as_object() {
            for (key, stored) in e {
                let def = js::arr(&p["env"]).into_iter().find(|d| js::gs(d, "key") == *key);
                let secret = js::truthy(&stored["enc"]) || def.map(|d| js::gb(&d, "secret")).unwrap_or(false) || js::gb(stored, "secret");
                let v = self.open_value(stored);
                env.insert(key.clone(), json!({ "value": if secret { if v.is_empty() { String::new() } else { MASK.to_string() } } else { v }, "secret": secret, "encrypted": js::truthy(&stored["enc"]) }));
            }
        }
        json!({
            "id": s["id"], "name": s["name"], "preset": s["preset"], "transport": s["transport"], "command": s["command"],
            "args": if s["args"].is_array() { s["args"].clone() } else { json!([]) }, "url": js::gs(s, "url"), "env": env,
            "enabled": js::gb(s, "enabled"), "widget": if js::truthy(&s["widget"]) { s["widget"].clone() } else { Value::Null },
            "tools": js::arr(&s["tools"]).iter().map(|t| json!({ "name": t["name"], "description": js::gs(t, "description"), "readOnly": js::gb(t, "readOnly") })).collect::<Vec<_>>(),
            "testedAt": s["testedAt"], "lastError": s["lastError"], "createdAt": s["createdAt"],
            "hasAuth": p["auth"].is_object(), "authLabel": p["auth"]["label"],
            "agy": p["agy"] != json!(false), "agyNote": p["agyNote"]
        })
    }

    pub fn get(&self, id: &str) -> Option<J> {
        self.servers.borrow().iter().find(|s| js::gs(s, "id") == id).cloned()
    }

    fn unique_id(&self, base: &str) -> String {
        let mut id = slug(base);
        if RESERVED.contains(&id.as_str()) {
            id = format!("{id}_ext");
        }
        let root = id.clone();
        let mut n = 2;
        while self.get(&id).is_some() {
            id = format!("{root}_{n}");
            n += 1;
        }
        id
    }

    // --- Écriture --------------------------------------------------------------------------------

    /// def : { preset, name, transport, command, args, url, env: { KEY: valeur }, params: { path } }
    pub fn add(&self, def: &J) -> Result<J, String> {
        let pid = js::gs(def, "preset");
        let pid = if presets().contains_key(&pid) { pid } else { "custom".into() };
        let p = preset(&pid);
        let name = js::slice_chars(if js::truthy(&def["name"]) { js::gs(def, "name") } else { js::gs(p, "name") }.trim(), 60);
        let name = if name.is_empty() { js::gs(p, "name") } else { name };
        let transport = if js::gs(def, "transport") == "http" { "http" } else { "stdio" };
        let base = if js::truthy(&def["id"]) { js::gs(def, "id") } else { name.clone() };
        let mut server = json!({
            "id": self.unique_id(&base), "name": name, "preset": pid, "transport": transport,
            "command": if transport == "stdio" { (if js::truthy(&def["command"]) { js::gs(def, "command") } else { js::gs(p, "command") }).trim().to_string() } else { String::new() },
            "args": if transport == "stdio" { if def["args"].is_array() { json!(js::arr(&def["args"]).iter().map(js::s).collect::<Vec<_>>()) } else { p["args"].clone() } } else { json!([]) },
            "url": if transport == "http" { js::gs(def, "url").trim().to_string() } else { String::new() },
            "env": {}, "enabled": def["enabled"] != json!(false),
            "widget": if def.get("widget").is_some() { def["widget"].clone() } else { p["widget"].clone() },
            "tools": [], "createdAt": js::iso_now()
        });
        if pid == "filesystem" && js::truthy(&def["params"]["path"]) {
            let path = js::gs(&def["params"], "path");
            if !Path::new(&path).is_absolute() {
                return Err("chemin absolu requis".into());
            }
            let mut args = js::arr(&p["args"]);
            args.push(json!(path));
            server["args"] = json!(args);
        }
        if transport == "stdio" && js::gs(&server, "command").is_empty() {
            return Err("commande requise".into());
        }
        if transport == "http" && !crate::re!(r"(?i)^https?://").is_match(&js::gs(&server, "url")) {
            return Err("adresse http(s) requise".into());
        }
        self.apply_env(&mut server, &def["env"], p);
        for e in js::arr(&p["env"]) {
            let k = js::gs(&e, "key");
            if js::truthy(&e["default"]) && !js::truthy(&server["env"][&k]) {
                server["env"][&k] = json!({ "value": e["default"] });
            }
        }
        self.servers.borrow_mut().push(server.clone());
        self.save()?;
        Ok(server)
    }

    fn apply_env(&self, server: &mut J, patch: &J, p: &J) {
        let Some(patch) = patch.as_object() else { return };
        if !server["env"].is_object() {
            server["env"] = json!({});
        }
        for (key, value) in patch {
            if !crate::re!(r"(?i)^[A-Z_][A-Z0-9_]*$").is_match(key) {
                continue;
            }
            let v = js::s(value);
            if v == MASK {
                continue;
            }
            if value.is_null() || v.is_empty() {
                if let Some(e) = server["env"].as_object_mut() {
                    e.remove(key);
                }
                continue;
            }
            let def = js::arr(&p["env"]).into_iter().find(|d| js::gs(d, "key") == *key);
            let secret = match def {
                Some(d) => js::gb(&d, "secret"),
                None => crate::re!(r"(?i)secret|token|key|password|pass").is_match(key),
            };
            let mut sealed = self.seal_value(&v, secret);
            if secret {
                sealed["secret"] = json!(true);
            }
            server["env"][key] = sealed;
        }
    }

    pub fn update(&self, id: &str, patch: &J) -> Result<J, String> {
        let mut server = self.get(id).ok_or("connecteur introuvable")?;
        let p = preset(&js::gs(&server, "preset"));
        if patch.get("name").is_some() {
            let n = js::slice_chars(js::gs(patch, "name").trim(), 60);
            if !n.is_empty() {
                server["name"] = json!(n);
            }
        }
        if js::gs(&server, "preset") == "custom" {
            if patch.get("command").is_some() {
                server["command"] = json!(js::gs(patch, "command").trim());
            }
            if patch["args"].is_array() {
                server["args"] = json!(js::arr(&patch["args"]).iter().map(js::s).collect::<Vec<_>>());
            }
            if patch.get("url").is_some() {
                server["url"] = json!(js::gs(patch, "url").trim());
            }
        }
        if patch.get("widget").is_some() {
            server["widget"] = if js::truthy(&patch["widget"]) { patch["widget"].clone() } else { Value::Null };
        }
        if patch.get("enabled").is_some() {
            server["enabled"] = json!(js::truthy(&patch["enabled"]));
        }
        if patch["env"].is_object() {
            self.apply_env(&mut server, &patch["env"], p);
        }
        self.replace(&server);
        self.save()?;
        Ok(server)
    }

    fn replace(&self, server: &J) {
        let id = js::gs(server, "id");
        if let Some(s) = self.servers.borrow_mut().iter_mut().find(|s| js::gs(s, "id") == id) {
            *s = server.clone();
        }
    }

    pub fn remove(&self, id: &str) -> Option<J> {
        let i = self.servers.borrow().iter().position(|s| js::gs(s, "id") == id)?;
        let removed = self.servers.borrow_mut().remove(i);
        let _ = self.save();
        Some(removed)
    }

    pub fn set_test_result(&self, id: &str, result: &J) -> Option<J> {
        let mut server = self.get(id)?;
        server["testedAt"] = json!(js::iso_now());
        if js::gb(result, "ok") {
            server["lastError"] = Value::Null;
            server["tools"] = json!(js::arr(&result["tools"])
                .iter()
                .map(|t| {
                    let name = js::s(&t["name"]);
                    json!({
                        "name": name, "description": js::slice_chars(&js::gs(t, "description"), 300),
                        "readOnly": js::gb(&t["annotations"], "readOnlyHint") || read_only_name(&name) || read_only_fr(&name)
                    })
                })
                .collect::<Vec<_>>());
        } else {
            server["lastError"] = json!(js::slice_chars(&if js::truthy(&result["error"]) { js::gs(result, "error") } else { "échec".into() }, 300));
        }
        self.replace(&server);
        let _ = self.save();
        Some(server)
    }

    pub fn missing_required(&self, server: &J) -> Vec<String> {
        let p = preset(&js::gs(server, "preset"));
        js::arr(&p["env"])
            .iter()
            .filter(|e| js::gb(e, "required") && self.open_value(&server["env"][js::gs(e, "key")]).is_empty())
            .map(|e| js::gs(e, "key"))
            .collect()
    }

    // --- Sorties vers les moteurs ----------------------------------------------------------------

    pub fn enabled(&self) -> Vec<J> {
        self.servers.borrow().iter().filter(|s| js::gb(s, "enabled") && self.missing_required(s).is_empty()).cloned().collect()
    }

    /// Entrées du fichier --mcp-config de Claude Code (et de la configuration d'AGY).
    pub fn claude_entries(&self) -> Map<String, Value> {
        let mut out = Map::new();
        for s in self.enabled() {
            let entry = if js::gs(&s, "transport") == "http" {
                json!({ "type": "http", "url": s["url"] })
            } else {
                json!({ "command": s["command"], "args": if s["args"].is_array() { s["args"].clone() } else { json!([]) }, "env": self.env_of(&s) })
            };
            out.insert(js::gs(&s, "id"), entry);
        }
        out
    }

    /// Entrées d'AGY : schémas d'outils assainis par le proxy (`jarvis-core mcp-proxy …`) pour l'API Gemini.
    pub fn agy_entries(&self, proxy_exe: Option<&Path>) -> Map<String, Value> {
        let mut out = Map::new();
        for (id, entry) in self.claude_entries() {
            let p = self.get(&id).map(|s| preset(&js::gs(&s, "preset")).clone()).unwrap_or_else(|| preset("custom").clone());
            if p["agy"] == json!(false) && proxy_exe.is_none() {
                continue;
            }
            let v = if js::gs(&entry, "type") == "http" {
                json!({ "httpUrl": entry["url"] })
            } else if let Some(exe) = proxy_exe.filter(|e| e.exists()) {
                let mut args = vec![json!("mcp-proxy"), entry["command"].clone()];
                args.extend(js::arr(&entry["args"]));
                json!({ "command": exe.to_string_lossy(), "args": args, "env": entry["env"] })
            } else {
                entry
            };
            out.insert(id, v);
        }
        out
    }

    /// Empreinte de la configuration active : le démon Claude repart si elle change.
    pub fn version(&self) -> String {
        let shape: Vec<J> = self
            .enabled()
            .iter()
            .map(|s| {
                let mut keys: Vec<String> = s["env"].as_object().map(|e| e.keys().cloned().collect()).unwrap_or_default();
                keys.sort();
                let mut o = Map::new();
                o.insert("id".into(), s["id"].clone());
                for (k, f) in [("c", "command"), ("a", "args"), ("u", "url")] {
                    if s.get(f).is_some() && !s[f].is_null() {
                        o.insert(k.into(), s[f].clone());
                    }
                }
                o.insert("e".into(), json!(keys));
                Value::Object(o)
            })
            .collect();
        sha1_smol::Sha1::from(Value::Array(shape).to_string()).hexdigest()[..12].to_string()
    }

    /// Types d'actions pour la section Sécurité.
    pub fn security_entries(&self) -> Vec<J> {
        let mut out = vec![];
        for s in self.servers.borrow().iter() {
            for t in js::arr(&s["tools"]) {
                let ro = js::gb(&t, "readOnly");
                out.push(json!({
                    "key": format!("mcp__{}__{}", js::gs(s, "id"), js::gs(&t, "name")),
                    "group": format!("Connecteur : {}{}", js::gs(s, "name"), if js::gb(s, "enabled") { "" } else { " (désactivé)" }),
                    "label": t["name"], "description": t["description"],
                    "level": if ro { "allow" } else { "code" }, "risk": if ro { "faible" } else { "moyen" }
                }));
            }
        }
        out
    }

    /// Serveur et widget d'un outil appelé par Claude (mcp__serveur__outil).
    pub fn describe_tool(&self, tool: &str) -> Option<J> {
        let m = crate::re!(r"^mcp__([^_].*?)__(.+)$").captures(tool)?;
        let s = self.get(&m[1])?;
        Some(json!({ "server": s["id"], "name": s["name"], "tool": &m[2], "widget": if js::truthy(&s["widget"]) { s["widget"].clone() } else { Value::Null }, "preset": s["preset"] }))
    }

    /// Phrase pour la consigne système : quels connecteurs sont branchés.
    pub fn prompt_line(&self) -> String {
        let active = self.enabled();
        if active.is_empty() {
            return String::new();
        }
        let names: Vec<String> = active.iter().map(|s| format!("{} (outils mcp__{}__*)", js::gs(s, "name"), js::gs(s, "id"))).collect();
        format!("Connecteurs externes branchés : {}. Utilise leurs outils quand la demande s'y rapporte.", names.join(", "))
    }

    /// Commande d'authentification du préréglage (« Se connecter à Spotify »).
    pub fn auth_args(&self, server: &J) -> Option<Vec<String>> {
        let p = preset(&js::gs(server, "preset"));
        p["auth"].is_object().then(|| js::str_list(&p["auth"]["args"]))
    }
}
