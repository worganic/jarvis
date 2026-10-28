// Orchestrateur : serveur HTTP local des serveurs MCP (approve, memory, clarify, history, system,
// google, meetings, ai-event) — orchestrator/index.js, lignes 3541-3722 et 4349-4590.
// Écoute sur 127.0.0.1 uniquement ; chaque route reçoit un corps JSON et rend du JSON.

use std::convert::Infallible;
use std::rc::Rc;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response};
use serde_json::{json, Value};

use super::policy::{describe, forbidden, shorten};
use super::{approval_port, Orchestrator, WaitingReply};
use crate::js::{self, Timer, J};

/// Réponse d'une route : statut HTTP et corps JSON (aucun corps pour 400/404).
pub struct HttpReply {
    pub status: u16,
    pub body: Option<J>,
}

impl HttpReply {
    pub fn ok(v: J) -> Self {
        Self { status: 200, body: Some(v) }
    }

    pub fn status(code: u16) -> Self {
        Self { status: code, body: None }
    }
}

fn to_response(r: HttpReply) -> Response<Full<Bytes>> {
    let mut b = Response::builder().status(r.status);
    let body = match r.body {
        Some(v) => {
            b = b.header("Content-Type", "application/json");
            Bytes::from(v.to_string())
        }
        None => Bytes::new(),
    };
    b.body(Full::new(body)).unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

fn err(msg: impl Into<String>) -> J {
    json!({ "error": msg.into() })
}

impl Orchestrator {
    /// Démarre le serveur HTTP local (port JARVIS_APPROVAL_PORT, 8799 par défaut).
    pub fn start_http(self: &Rc<Self>) {
        let w = self.self_ref();
        let log = self.log_fn.clone();
        let handle = js::spawn(async move {
            let port = approval_port();
            let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(l) => l,
                Err(e) => {
                    log(format!("serveur de validation : port {port} indisponible ({e})"));
                    return;
                }
            };
            loop {
                let Ok((stream, _)) = listener.accept().await else { continue };
                let w = w.clone();
                tokio::task::spawn_local(async move {
                    let io = hyper_util::rt::TokioIo::new(stream);
                    let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
                        let w = w.clone();
                        async move {
                            let r = match w.upgrade() {
                                Some(o) => o.on_request(req).await,
                                None => HttpReply::status(503),
                            };
                            Ok::<_, Infallible>(to_response(r))
                        }
                    });
                    // Une requête par connexion (« Connection: close ») : les clients d'un coup (hook
                    // ai-state de Claude Code sous Node 19+, agent keep-alive) attendraient sinon la
                    // fermeture du socket, que hyper ne fait jamais, et bloqueraient Claude Code.
                    let _ = hyper::server::conn::http1::Builder::new().keep_alive(false).serve_connection(io, svc).await;
                });
            }
        });
        *self.http_server.borrow_mut() = Some(handle);
    }

    async fn on_request(self: Rc<Self>, req: Request<Incoming>) -> HttpReply {
        if req.method() != hyper::Method::POST {
            return HttpReply::status(404);
        }
        let url = req.uri().path().to_string();
        let Ok(body) = req.into_body().collect().await else { return HttpReply::status(400) };
        let Ok(payload) = serde_json::from_slice::<Value>(&body.to_bytes()) else { return HttpReply::status(400) };
        let route = |p: &str| url.starts_with(p);
        let v = if route("/memory") {
            self.on_memory(&payload).await
        } else if route("/approve") {
            self.on_approval(&payload).await
        } else if route("/clarify") {
            self.on_clarify(&payload).await
        } else if route("/history") {
            self.on_history(&payload)
        } else if route("/system") {
            self.on_system(&payload).await
        } else if route("/google") {
            self.on_google(&payload).await
        } else if route("/meetings") {
            self.on_meetings(&payload).await
        } else if route("/listes") {
            self.on_listes(&payload).await
        } else if route("/documents") {
            self.on_documents(&payload).await
        } else if route("/savoir") {
            self.on_savoir(&payload).await
        } else if route("/ai-event") {
            self.on_ai_event(&payload)
        } else {
            return HttpReply::status(404);
        };
        HttpReply::ok(v)
    }

    fn task_id_for(&self, payload: &J) -> String {
        self.resolve_task_id(payload)
            .or_else(|| self.running_task.borrow().clone())
            .unwrap_or_else(|| self.next_id("c"))
    }

    /// Événements émis par Claude Code ou AGY (surveillance permanente des agents IA).
    fn on_ai_event(&self, payload: &J) -> J {
        let r = self.ai_watch.on_event(payload);
        if r.is_null() {
            json!({ "ok": true })
        } else {
            r
        }
    }

    /// Outil documents_chercher de l'IA : lecture seule des documents des dossiers choisis (le journal garde la
    /// recherche, jamais le contenu).
    async fn on_documents(&self, payload: &J) -> J {
        let task = if js::truthy(&payload["taskId"]) { payload["taskId"].clone() } else { Value::Null };
        let query = js::gs(payload, "query");
        let limit = js::gn(payload, "limit").filter(|n| *n > 0.0).map(|n| (n as usize).min(12)).unwrap_or(6);
        let text = self.documents.ai_search_text(&query, limit).await;
        self.record("documents_recherche", json!({ "tache": task, "recherche": js::slice_chars(&query, 200) }));
        json!({ "text": text })
    }

    /// Outil savoir_chercher de l'IA : lecture seule des dossiers Data (le journal garde la recherche, jamais le contenu).
    async fn on_savoir(&self, payload: &J) -> J {
        let task = if js::truthy(&payload["taskId"]) { payload["taskId"].clone() } else { Value::Null };
        let query = js::gs(payload, "query");
        let limit = js::gn(payload, "limit").filter(|n| *n > 0.0).map(|n| (n as usize).min(12)).unwrap_or(6);
        let text = self.savoir.ai_search_text(&query, &js::gs(payload, "dossier"), limit).await;
        self.record("savoir_question", json!({ "tache": task, "recherche": js::slice_chars(&query, 200) }));
        json!({ "text": text })
    }

    /// Outils listes_chercher et listes_lire de l'IA : lecture seule des listes à fiches (le journal garde la trace,
    /// jamais le contenu des fiches).
    async fn on_listes(&self, payload: &J) -> J {
        let task = if js::truthy(&payload["taskId"]) { payload["taskId"].clone() } else { Value::Null };
        match js::gs(payload, "op").as_str() {
            "search" => {
                let query = js::gs(payload, "query");
                let limit = js::gn(payload, "limit").filter(|n| *n > 0.0).map(|n| (n as usize).min(20)).unwrap_or(8);
                let text = self.collections.ai_search_text(&query, &js::gs(payload, "liste"), limit).await;
                self.record("listes_recherche", json!({ "tache": task, "recherche": js::slice_chars(&query, 200) }));
                json!({ "text": text })
            }
            "read" => {
                let text = self.collections.ai_read_text(&js::gs(payload, "liste"), &js::gs(payload, "element"));
                self.record("listes_lecture", json!({ "tache": task, "liste": js::slice_chars(&js::gs(payload, "liste"), 80) }));
                json!({ "text": text })
            }
            op => err(format!("opération inconnue : {op}")),
        }
    }


    /// Outils reunions_chercher et reunion_lire de l'IA.
    async fn on_meetings(&self, payload: &J) -> J {
        let task = if js::truthy(&payload["taskId"]) { payload["taskId"].clone() } else { Value::Null };
        match js::gs(payload, "op").as_str() {
            "search" => {
                let query = js::gs(payload, "query");
                let limit = js::gn(payload, "limit").filter(|n| *n > 0.0).map(|n| n as usize).unwrap_or(6);
                let results = self.meetings.search(&query, limit).await;
                self.record("reunions_recherche", json!({ "tache": task, "recherche": js::slice_chars(&query, 200), "resultats": results.len() }));
                json!({ "results": results })
            }
            "read" => match self.meetings.read(&js::s(&payload["reference"])) {
                Some(m) => {
                    self.record("reunion_lue", json!({ "tache": task, "reunion": m["titre"] }));
                    json!({ "meeting": m })
                }
                None => err("réunion introuvable"),
            },
            op => err(format!("opération inconnue : {op}")),
        }
    }

    async fn on_memory(&self, payload: &J) -> J {
        match js::gs(payload, "op").as_str() {
            "search" => {
                // Recherche par le sens (repli lexical automatique si le modèle est indisponible).
                let entries = self.memory.search_semantic(&js::gs(payload, "query"), 12).await;
                json!({ "entries": entries })
            }
            "add" => {
                let expires = match super::memory::parse_expiry(&js::gs(payload, "expires")) {
                    Ok(d) => d,
                    Err(e) => return err(&e),
                };
                let entry = self.memory.add_until(&js::gs(payload, "text"), &js::gs(payload, "category"), expires.as_deref());
                match entry {
                    Some(entry) => {
                        self.publish_memory();
                        self.record("memoire_ajout", json!({ "categorie": entry["category"], "texte": entry["text"], "expire": entry["expires"] }));
                        self.refresh_daemon();
                        if let Some(o) = self.self_ref().upgrade() {
                            js::spawn(async move {
                                let _ = o.memory.index().await; // vecteur prêt pour la prochaine recherche
                            });
                        }
                        self.publish_vocabulary(false); // un nouveau nom propre peut aider la reconnaissance vocale
                        self.log(format!("mémorisé : {}", js::gs(&entry, "text")));
                        json!({ "entry": entry })
                    }
                    None => err("texte vide"),
                }
            }
            "forget" => {
                let entry = self.memory.forget(&js::s(&payload["reference"]));
                if let Some(e) = &entry {
                    self.publish_memory();
                    self.record("memoire_oubli", json!({ "categorie": e["category"], "texte": e["text"] }));
                    self.refresh_daemon();
                    self.publish_vocabulary(false);
                    self.log(format!("oublié : {}", js::gs(e, "text")));
                }
                json!({ "entry": entry })
            }
            "clear" => self.clear_memory(),
            op => err(format!("opération inconnue : {op}")),
        }
    }

    /// Lecture seule : le journal se consulte, il ne se modifie jamais.
    fn on_history(&self, payload: &J) -> J {
        let limit = js::gn(payload, "limit").filter(|n| *n > 0.0).map(|n| n as usize).unwrap_or(20);
        let entries = self.history.search(&js::gs(payload, "query"), limit);
        json!({ "entries": entries, "integrity": self.history.verify() })
    }

    async fn on_clarify(&self, payload: &J) -> J {
        let question = js::gs(payload, "question");
        let task_id = self.task_id_for(payload);
        if question.is_empty() {
            return err("question vide");
        }
        // Une demande de précision précédente était en cours : elle est close.
        self.close_waiting_reply(err("nouvelle question posée"));

        let (tx, rx) = tokio::sync::oneshot::channel::<J>();
        let timer = Timer::new();
        {
            let w = self.self_ref();
            let (id, q) = (task_id.clone(), question.clone());
            timer.set(120_000, move || {
                let Some(o) = w.upgrade() else { return };
                let mine = o.waiting_reply.borrow().as_ref().map(|r| r.task_id == id).unwrap_or(false);
                if mine {
                    o.record("clarification_expiree", json!({ "tache": id, "question": q }));
                    o.emit(json!({ "type": "update", "id": id, "status": "running", "detail": "délai de réponse expiré" }));
                    o.close_waiting_reply(err("délai de réponse dépassé (aucun retour utilisateur)"));
                }
            }); // 2 minutes
        }
        *self.waiting_reply.borrow_mut() = Some(WaitingReply { task_id: task_id.clone(), question: question.clone(), tx: Some(tx), timer });

        self.record("clarification_demandee", json!({ "tache": task_id, "question": question }));
        self.emit(json!({
            "type": "update", "id": task_id, "status": "waiting_reply",
            "detail": format!("Question : {}", shorten(&question, 45)), "question": question
        }));
        self.emit(json!({ "type": "say", "text": question, "answer": true }));
        self.log(format!("clarification demandée ({task_id}) : \"{question}\""));
        rx.await.unwrap_or_else(|_| err("question abandonnée"))
    }

    /// Exécute une action système (outils du serveur MCP system). None : action inconnue.
    async fn system_exec(self: &Rc<Self>, action: &str, p: &J) -> Option<Result<J, String>> {
        Some(Ok(match action {
            "media" => self.media_action(p.clone()).await,
            "projets" => self.projects.tool(p).await,
            "lumiere" => self.lights.command(p).await,
            "maison" => self.home.command(p).await,
            // Veille du bureau : état en direct et actions sur les fenêtres.
            "etat_bureau" => {
                let mut st = self.desktop_state.tool_state(p);
                // Onglets de Chrome (extension reliée) : pages numérotées « 2.3 ».
                if js::truthy(&st["ok"]) && self.browser_bridge.connected() {
                    st["chrome"] = self.browser_bridge.tool_list(Some(&self.desktop_state))["instances"].clone();
                }
                st
            }
            "onglets" | "onglet_agir" => self.browser_bridge.tool_action(p, Some(&self.desktop_state)).await,
            "ui_agir" => self.ui_control.tool(p).await,
            "ui_lister" => {
                let mut q = p.clone();
                if !q.is_object() {
                    q = json!({});
                }
                q["action"] = json!("lister");
                self.ui_control.tool(&q).await
            }
            "fenetre" => self.desktop_state.tool_action(p).await,
            "fenetre_fermer" => {
                let cible = js::gs(p, "cible");
                match self.desktop_state.find_window(&cible, false) {
                    None => json!({ "ok": false, "error": format!("aucune fenêtre ne correspond à « {cible} »") }),
                    Some(w) => {
                        let (_, remaining) = self.desktop_state.close_windows(std::slice::from_ref(&w), 3000).await;
                        let app = js::gs(&w, "app");
                        if remaining.is_empty() {
                            json!({ "ok": true, "message": format!("{app} fermé") })
                        } else {
                            json!({ "ok": false, "error": format!("{app} ne s'est pas fermé (confirmation demandée à l'écran ?)") })
                        }
                    }
                }
            }
            _ => return super::system::call(action, p).await,
        }))
    }

    async fn on_system(self: &Rc<Self>, payload: &J) -> J {
        let action = js::gs(payload, "action");
        let params = if payload["params"].is_object() { payload["params"].clone() } else { json!({}) };
        let task_id = self.task_id_for(payload);

        if action == "sourdine_contextuelle" {
            let mode = js::gs(&params, "mode");
            if !mode.is_empty() {
                let state = self.context_mute.set_mode(&mode).await.unwrap_or_else(|_| self.context_mute.get_state());
                self.publish_status();
                self.record("action_systeme", json!({ "tache": task_id, "action": action, "params": params, "resultat": state }));
                let mut r = json!({ "ok": true });
                js::assign(&mut r, state);
                return r;
            }
            let mut r = json!({ "ok": true });
            js::assign(&mut r, self.context_mute.get_state());
            return r;
        }

        if action.is_empty() || !super::system::has(&action) && !["media", "projets", "lumiere", "maison", "etat_bureau", "onglets", "onglet_agir", "ui_agir", "ui_lister", "fenetre", "fenetre_fermer"].contains(&action.as_str()) {
            return err(format!("action système inconnue : {action}"));
        }

        let tool_name = format!("mcp__system__{action}");
        let verdict = self.verdict_for(&tool_name, &params);
        if verdict == "deny" {
            self.record("acces_refuse", json!({ "tache": task_id, "outil": tool_name, "detail": describe(&tool_name, &params), "motif": "interdit" }));
            return err("action interdite par la politique de sécurité");
        }
        if verdict != "allow" && !self.consume_pre_approval(&tool_name) {
            let rx = self.open_validation(&task_id, &tool_name, &params, &verdict);
            let v = rx.await.unwrap_or_else(|_| json!({ "behavior": "deny" }));
            if js::gs(&v, "behavior") != "allow" {
                return err(Some(js::gs(&v, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| "refusé par l’utilisateur".into()));
            }
        }

        let before = self.undo.prepare(&tool_name, &params).await;
        let result = match self.system_exec(&action, &params).await {
            Some(Ok(r)) => r,
            Some(Err(e)) => {
                self.log(format!("erreur action système {action} : {e}"));
                self.record("action_systeme_erreur", json!({ "tache": task_id, "action": action, "params": params, "erreur": e }));
                return err(e);
            }
            None => return err(format!("action système inconnue : {action}")),
        };
        self.undo.record(&tool_name, &params, &result, before);
        // Musique : un échec expliqué (Spotify absent, rien trouvé, rien ne joue) n'est pas une réussite.
        if action == "media" && result["ok"] == json!(false) {
            let erreur = Some(js::gs(&result, "error")).filter(|e| !e.is_empty()).unwrap_or_else(|| "la musique n’a pas démarré".into());
            self.record("action_systeme_erreur", json!({ "tache": task_id, "action": action, "params": params, "erreur": erreur }));
            return err(erreur);
        }
        let resultat = if action == "media" {
            let mut r = json!({ "titre": result["title"], "artiste": result["artist"], "etat": result["status"], "app": result["app"] });
            if js::truthy(&result["opened"]) {
                r["recherche_ouverte"] = result["query"].clone();
            }
            r
        } else {
            result.clone()
        };
        self.record("action_systeme", json!({ "tache": task_id, "action": action, "params": params, "resultat": resultat }));
        self.on_tool_result(&tool_name, &params, &result);
        // L'IA a listé les projets (demande que le routeur local n'a pas reconnue) : la card
        // numérotée s'affiche quand même, comme pour « liste mes projets ».
        let pa = js::gs(&params, "action");
        if action == "projets" && (pa.is_empty() || pa == "lister") && js::truthy(&result["ok"]) {
            let tous = params["tous"] == json!(true) || params["tous"] == json!("true");
            self.show_projects_card(if tous { "tous" } else { "liste" }).await;
        }
        result
    }

    async fn on_google(self: &Rc<Self>, payload: &J) -> J {
        let action = js::gs(payload, "action");
        let params = if payload["params"].is_object() { payload["params"].clone() } else { json!({}) };
        let task_id = self.task_id_for(payload);

        use super::google::GoogleConnector as G;
        let resolved = if G::has(&action) {
            Some(action.clone())
        } else if let Some(stripped) = action.strip_prefix("google_").filter(|s| G::has(s)) {
            Some(stripped.to_string())
        } else if !action.starts_with("google_") && G::has(&format!("google_{action}")) {
            Some(format!("google_{action}"))
        } else {
            None
        };
        let Some(resolved) = resolved.filter(|_| !action.is_empty()) else {
            return err(format!("action google inconnue : {action}"));
        };

        let tool_name = format!("mcp__google__{action}");
        let verdict = self.verdict_for(&tool_name, &params);
        if verdict == "deny" {
            self.record("acces_refuse", json!({ "tache": task_id, "outil": tool_name, "detail": describe(&tool_name, &params), "motif": "interdit" }));
            return err("action interdite par la politique de sécurité");
        }
        if verdict != "allow" && !self.consume_pre_approval(&tool_name) {
            let rx = self.open_validation(&task_id, &tool_name, &params, &verdict);
            let v = rx.await.unwrap_or_else(|_| json!({ "behavior": "deny" }));
            if js::gs(&v, "behavior") != "allow" {
                return err(Some(js::gs(&v, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| "refusé par l’utilisateur".into()));
            }
        }

        match self.google.call(&resolved, &params).await {
            Ok(result) => {
                self.undo.record(&tool_name, &params, &result, None);
                let mut logged = params.clone();
                if js::truthy(&params["corps"]) {
                    logged["corps"] = json!(shorten(&js::gs(&params, "corps"), 50));
                } else if let Some(m) = logged.as_object_mut() {
                    m.remove("corps");
                }
                self.record("action_google", json!({ "tache": task_id, "action": action, "params": logged, "resultat": result }));
                self.on_tool_result(&tool_name, &params, &result);
                result
            }
            Err(e) => {
                self.log(format!("erreur action google {action} : {e}"));
                self.record("action_google_erreur", json!({ "tache": task_id, "action": action, "erreur": e }));
                err(e)
            }
        }
    }

    async fn on_approval(self: &Rc<Self>, payload: &J) -> J {
        let raw_input = if payload["input"].is_object() { payload["input"].clone() } else { json!({}) };
        let (tool, input) = super::security::normalize_tool(&js::gs(payload, "tool"), &raw_input);
        let task_id = self.task_id_for(payload);

        // 1. Refus sec, non contournable : mémoire, journal et réglages de Jarvis.
        if forbidden(&input, &self.guarded()) {
            self.record("acces_refuse", json!({ "tache": task_id, "outil": tool, "detail": describe(&tool, &input), "motif": "fichiers internes" }));
            self.emit(json!({ "type": "update", "id": task_id, "detail": "accès aux fichiers internes de Jarvis refusé" }));
            return json!({ "behavior": "deny", "message": "les fichiers internes de Jarvis sont inaccessibles" });
        }

        let verdict = self.verdict_for(&tool, &input);
        // 2. Interdit : zone interdite, écriture hors zone, ou type d'action interdit par l'utilisateur.
        if verdict == "deny" {
            self.record("acces_refuse", json!({ "tache": task_id, "outil": tool, "detail": describe(&tool, &input), "motif": "interdit par la politique de sécurité" }));
            self.emit(json!({ "type": "update", "id": task_id, "detail": "action interdite par la sécurité" }));
            return json!({ "behavior": "deny", "message": "action interdite par la politique de sécurité de Jarvis" });
        }
        // 3. Accord direct.
        if verdict == "allow" {
            return json!({ "behavior": "allow" });
        }
        // 4. Accord oral (« oui ») ou code à 4 chiffres saisi au clavier.
        let rx = self.open_validation(&task_id, &tool, &input, &verdict);
        let v = rx.await.unwrap_or_else(|_| json!({ "behavior": "deny", "message": "validation abandonnée" }));
        // Outil Windows ou Google accordé : le serveur MCP va rappeler /system ou /google pour
        // l'exécuter. Cet accord vaut pour cet appel-là (60 s, usage unique), sans second code.
        if js::gs(&v, "behavior") == "allow" && crate::re!(r"^mcp__(?:system|google)__").is_match(&tool) {
            self.pre_approved.borrow_mut().insert(tool.clone(), js::now_ms() + 60000);
        }
        v
    }

    /// Fichiers et dossiers internes de Jarvis, inaccessibles aux IA.
    pub fn guarded(&self) -> Vec<String> {
        let mut g = vec![self.data_dir.display().to_string()];
        let hist_dir = self.history.file.parent().map(|p| p.display().to_string());
        if let Some(h) = hist_dir.filter(|h| !g.contains(h)) {
            g.push(h);
        }
        // Les autres fichiers internes sont couverts par le dossier de données : un nom nu
        // bloquerait aussi les fichiers homonymes des projets de l'utilisateur.
        g.extend(["memory.json", "journal.jsonl", "google.json", "calendar.json"].map(String::from));
        g
    }
}
