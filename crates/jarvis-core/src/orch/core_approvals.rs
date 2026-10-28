// Orchestrateur : validations (accord oral ou code à 4 chiffres), cache des outils idempotents,
// actions locales sous contrôle de sécurité, arrêt (orchestrator/index.js, lignes 4135-4627).

use std::rc::Rc;

use serde_json::json;
use tokio::sync::oneshot;

use super::policy::{self, describe, describe_full};
use super::{Orchestrator, PendingCtl, ToolExec, APPROVAL_TIMEOUT};
use crate::js::{self, Timer, J};

/// Durée de vie des lectures d'outils idempotents en cache (ms).
fn tool_ttl(tool: &str) -> Option<i64> {
    Some(match tool {
        "mcp__google__mail_derniers" => 60000,
        "mcp__google__agenda_jour" => 120000,
        "mcp__google__agenda_semaine" => 180000,
        "mcp__system__lister_fenetres" => 3000,
        "mcp__system__fenetre_active" => 2000,
        "meteo" => 600000,
        _ => return None,
    })
}

fn cache_key(tool: &str, params: &J) -> String {
    let p = if params.is_null() { json!({}) } else { params.clone() };
    format!("{tool}:{p}")
}

impl Orchestrator {
    pub fn verdict_for(&self, tool: &str, input: &J) -> String {
        let (tool, input) = super::security::normalize_tool(tool, input);
        // Clics et saisies dans une application : interdits dans les applis sensibles, sans accord
        // dans les applis de confiance, accord oral ailleurs (ui_control.rs).
        if tool == "mcp__system__ui_agir" {
            if let Some(v) = self.ui_control.verdict(&input) {
                return v.to_string();
            }
        }
        let ws = self.workspace.display().to_string();
        let base = policy::evaluate(&tool, &input, Some(&ws), Some(&self.zones));
        self.security.apply(&tool, &input, &base)
    }

    /// Construit le contexte détaillé d'une demande de validation pour que l'utilisateur sache
    /// exactement quelle tâche, quelle action, quelle cible et quel motif déclenchent le code.
    fn build_validation_info(&self, task_id: &str, tool: &str, input: &J, full_detail: &str) -> J {
        let (task_label, engine_label) = self
            .running
            .borrow()
            .iter()
            .find(|(k, _)| k == task_id)
            .map(|(_, e)| {
                let eng = if e.engine == "agy" { "AGY" } else if e.engine == "claude" { "Claude Code" } else { e.engine.as_str() };
                let cleaned = if let Some(rest) = e.text.strip_prefix("Tu prépares le dossier « Savoir » de Jarvis sur : « ") {
                    let subj = rest.split(" ».").next().unwrap_or(rest);
                    format!("Dossier Data « {} »", policy::shorten(subj, 60))
                } else {
                    policy::shorten(&e.text, 80)
                };
                let prefix = if e.background { "[Tâche en fond] " } else { "" };
                (format!("{prefix}{cleaned}"), eng.to_string())
            })
            .unwrap_or_else(|| ("Action directe".into(), String::new()));

        let action_name = policy::label(tool).unwrap_or("action");
        let action_label = format!("{} ({tool})", js::capitalize(action_name));

        let ws = self.workspace.display().to_string();
        let raw_target = policy::target_of(input);
        let resolved_target = if !raw_target.is_empty() && std::path::Path::new(&raw_target).is_relative() {
            std::path::Path::new(&ws).join(&raw_target).to_string_lossy().into_owned()
        } else {
            raw_target
        };
        let zone_label = if !resolved_target.is_empty() {
            match self.zones.find_zone(&resolved_target) {
                Some(z) => format!("{} ({})", js::gs(&z, "name"), js::gs(&z, "path")),
                None => "Hors des zones déclarées".into(),
            }
        } else {
            String::new()
        };

        let target_label = if !resolved_target.is_empty() {
            resolved_target
        } else {
            full_detail.to_string()
        };

        let base = policy::evaluate(tool, input, Some(&ws), Some(&self.zones));
        let (reason_label, risk_label) = match self.security.key_for(tool, input, &base).and_then(|k| self.security.entry(&k)) {
            Some(entry) => (js::gs(&entry, "label"), js::gs(&entry, "risk")),
            None => ("Action soumise à validation".into(), "élevé".into()),
        };

        json!({
            "task": task_label,
            "engine": engine_label,
            "action": action_label,
            "target": target_label,
            "zone": zone_label,
            "reason": reason_label,
            "risk": risk_label,
        })
    }

    /// Ouvre une validation (accord oral ou code à 4 chiffres). Le récepteur rend
    /// { behavior: 'allow' } ou { behavior: 'deny', message }.
    pub fn open_validation(&self, task_id: &str, tool: &str, input: &J, level: &str) -> oneshot::Receiver<J> {
        let detail = describe(tool, input);
        // Raison complète (commande entière, chemin complet) pour le clavier du code de sécurité.
        let full_detail = describe_full(tool, input);
        let info = self.build_validation_info(task_id, tool, input, &full_detail);
        let is_voice = level == "voice";
        if !self.is_running(task_id) {
            self.emit(json!({
                "type": "create", "id": task_id, "label": detail,
                "detail": if is_voice { "accord oral requis" } else { "validation critique requise" }
            }));
        }
        let code = if is_voice { None } else { Some((1000 + js::rand_below(9000)).to_string()) };
        let vtype = if is_voice { "voice" } else { "code" };
        let (tx, rx) = oneshot::channel();
        let timer = Timer::new();
        {
            let w = self.self_ref();
            let (id, t) = (task_id.to_string(), tool.to_string());
            timer.set(APPROVAL_TIMEOUT, move || {
                let Some(o) = w.upgrade() else { return };
                o.emit(json!({ "type": "update", "id": id, "status": "error", "detail": "validation expirée" }));
                o.record("validation_expiree", json!({ "tache": id, "outil": t, "niveau": if is_voice { "modere" } else { "critique" } }));
                o.resolve_pending(&id, json!({ "behavior": "deny", "message": "validation expirée" }));
            });
        }
        // Une validation précédente de la même tâche est remplacée (Map.set de la v3).
        self.pending.borrow_mut().retain(|(k, _)| k != task_id);
        if let Some(old) = self.pending_ctl.borrow_mut().remove(task_id) {
            old.timer.clear();
        }
        self.pending.borrow_mut().push((
            task_id.to_string(),
            json!({
                "tool": tool, "code": code, "detail": detail, "fullDetail": full_detail, "info": info,
                "validationType": vtype, "createdAt": js::now_ms()
            }),
        ));
        self.pending_ctl.borrow_mut().insert(task_id.to_string(), PendingCtl { resolve: Some(tx), timer, on_decision: None });
        self.emit(json!({ "type": "update", "id": task_id, "status": "awaiting", "code": code, "validationType": vtype, "detail": detail, "fullDetail": full_detail, "info": info }));
        self.emit(json!({ "type": "security", "pending": self.pending_list() }));
        let a = self.addressing_forms();
        let request = policy::spoken_request(tool, input, Some(&self.zones));
        self.emit(json!({
            "type": "say",
            "text": if is_voice {
                format!("{request}. {} oui pour valider.", if a.vous { "Dites" } else { "Dis" })
            } else {
                format!("{request}. Action critique, {} le code affiché à l'écran.", a.saisis)
            }
        }));
        // Le code lui-même n'est jamais journalisé : c'est un secret d'usage unique.
        self.record(
            "validation_demandee",
            json!({
                "tache": task_id, "outil": tool,
                "niveau": if is_voice { "modere" } else { "critique" },
                "type": if is_voice { "oral" } else { "code_ecrit" },
                "detail": detail
            }),
        );
        self.log(format!(
            "validation {} demandée : {tool}{}",
            if is_voice { "orale" } else { "par code" },
            code.map(|c| format!(" · code {c}")).unwrap_or_default()
        ));
        rx
    }

    /// Clôt la validation en attente d'une tâche avec ce verdict (sans effet s'il n'y en a pas).
    pub fn resolve_pending(&self, task_id: &str, verdict: J) -> bool {
        let ctl = self.pending_ctl.borrow_mut().remove(task_id);
        let had = {
            let mut p = self.pending.borrow_mut();
            let n = p.len();
            p.retain(|(k, _)| k != task_id);
            n != p.len()
        };
        let Some(mut ctl) = ctl else {
            if had {
                self.emit(json!({ "type": "security", "pending": self.pending_list() }));
            }
            return had;
        };
        ctl.timer.clear();
        self.emit(json!({ "type": "security", "pending": self.pending_list() }));
        let allowed = js::gs(&verdict, "behavior") == "allow";
        if let Some(tx) = ctl.resolve.take() {
            let _ = tx.send(verdict);
        }
        if let Some(f) = ctl.on_decision.take() {
            f(allowed);
        }
        true
    }

    /// Validations en cours, pour la section Sécurité (sans les codes).
    pub fn pending_list(&self) -> J {
        J::Array(
            self.pending
                .borrow()
                .iter()
                .map(|(id, e)| {
                    let tool = js::gs(e, "tool");
                    let detail = Some(js::gs(e, "detail")).filter(|d| !d.is_empty()).unwrap_or_else(|| describe(&tool, &json!({})));
                    let full = Some(js::gs(e, "fullDetail")).filter(|d| !d.is_empty()).unwrap_or_else(|| detail.clone());
                    json!({
                        "taskId": id, "tool": tool, "detail": detail, "fullDetail": full,
                        "type": Some(js::gs(e, "validationType")).filter(|t| !t.is_empty()).unwrap_or_else(|| "code".into()),
                        "since": e["createdAt"]
                    })
                })
                .collect(),
        )
    }

    // Cache à TTL adaptatif des lectures d'outils idempotents.
    pub fn get_cached_tool(&self, tool: &str, params: &J) -> Option<J> {
        let key = cache_key(tool, params);
        let mut cache = self.tool_cache.borrow_mut();
        let (result, expires) = cache.get(&key)?.clone();
        if js::now_ms() > expires {
            cache.remove(&key);
            return None;
        }
        Some(result)
    }

    pub fn set_cached_tool(&self, tool: &str, params: &J, result: &J) {
        if !js::truthy(result) || result["ok"] == json!(false) || js::truthy(&result["error"]) {
            return;
        }
        let Some(ttl) = tool_ttl(tool) else { return };
        self.tool_cache.borrow_mut().insert(cache_key(tool, params), (result.clone(), js::now_ms() + ttl));
    }

    pub fn invalidate_tool_cache(&self, tool: &str) {
        self.tool_cache.borrow_mut().retain(|k, _| !k.starts_with(tool));
    }

    /// Exécution parallèle d'outils indépendants.
    pub async fn parallel_tools(self: &Rc<Self>, items: Vec<(String, J, ToolExec)>) -> Vec<J> {
        let futs: Vec<_> = items.into_iter().map(|(tool, params, exec)| {
            let o = self.clone();
            async move { o.local_tool(&tool, params, exec).await }
        }).collect();
        futures_util::future::join_all(futs).await
    }

    async fn run_local(&self, tool: &str, params: &J, exec: ToolExec) -> J {
        // « Annule ce que tu viens de faire » : état à relever avant (luminosité, presse-papier).
        let before = self.undo.prepare(tool, params).await;
        let result = exec(params.clone()).await;
        if js::truthy(&result["__error"]) {
            let err = js::gs(&result, "__error");
            self.log(format!("erreur action locale {tool} : {err}"));
            self.record("action_locale_erreur", json!({ "outil": tool, "detail": describe(tool, params), "erreur": err }));
            return json!({ "ok": false, "error": err });
        }
        self.record("action_locale", json!({ "outil": tool, "detail": describe(tool, params), "resultat": result }));
        self.on_tool_result(tool, params, &result);
        self.set_cached_tool(tool, params, &result);
        self.undo.record(tool, params, &result, before);

        // Invalidation des caches associés
        if tool.contains("ouvrir_application") || tool.contains("fermer_application") || tool.contains("activer_fenetre") {
            self.invalidate_tool_cache("mcp__system__lister_fenetres");
            self.invalidate_tool_cache("mcp__system__fenetre_active");
        } else if tool.contains("mail_envoyer") {
            self.invalidate_tool_cache("mcp__google__mail_derniers");
        } else if tool.contains("agenda_creer") {
            self.invalidate_tool_cache("mcp__google__agenda_jour");
            self.invalidate_tool_cache("mcp__google__agenda_semaine");
        }
        result
    }

    /// Action exécutée sans IA à la demande de l'utilisateur (routeur local, v5) : même verdict de
    /// sécurité, même journal et mêmes widgets que les outils appelés par les modèles.
    /// Rend le résultat de exec, ou { ok: false, denied: true, error } si l'action est refusée.
    pub async fn local_tool(&self, tool: &str, params: J, exec: ToolExec) -> J {
        // Vérification du cache pour les outils idempotents
        if let Some(cached) = self.get_cached_tool(tool, &params) {
            self.log(format!("[cache outil] résultat réutilisé pour {tool}"));
            self.on_tool_result(tool, &params, &cached);
            return cached;
        }
        let verdict = self.verdict_for(tool, &params);
        if verdict == "deny" {
            self.record("acces_refuse", json!({ "outil": tool, "detail": describe(tool, &params), "motif": "interdit par la sécurité" }));
            return json!({ "ok": false, "denied": true, "error": "action interdite par les réglages de sécurité" });
        }
        if verdict == "allow" || self.consume_pre_approval(tool) {
            return self.run_local(tool, &params, exec).await;
        }
        let task_id = self.next_id("local");
        let rx = self.open_validation(&task_id, tool, &params, &verdict);
        let v = rx.await.unwrap_or_else(|_| json!({ "behavior": "deny", "message": "refusé" }));
        if js::gs(&v, "behavior") == "allow" {
            let result = self.run_local(tool, &params, exec).await;
            self.emit(json!({ "type": "finish", "id": task_id, "status": if js::truthy(&result["error"]) { "error" } else { "done" } }));
            result
        } else {
            self.emit(json!({ "type": "finish", "id": task_id, "status": "error" }));
            let msg = Some(js::gs(&v, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| "refusé".into());
            json!({ "ok": false, "denied": true, "error": msg })
        }
    }

    /// Effacement complet de la mémoire demandé à la voix ou par le bouton du HUD.
    pub fn request_memory_purge(&self) -> J {
        let tool = "mcp__jarvis__memoire_effacer";
        let level = self.verdict_for(tool, &json!({}));
        let purge = |o: &Orchestrator| {
            o.clear_memory();
            o.emit(json!({ "type": "say", "text": "Mémoire locale et RAG entièrement effacés. Au prochain lancement, nous ferons à nouveau connaissance." }));
        };
        if level == "deny" {
            self.record("acces_refuse", json!({ "outil": tool, "detail": "effacement complet de la mémoire", "motif": "interdit par la sécurité" }));
            self.emit(json!({ "type": "say", "text": "L'effacement complet de la mémoire est interdit par les réglages de sécurité." }));
            return json!({ "ok": false });
        }
        if level == "allow" {
            purge(self);
            return json!({ "ok": true });
        }
        let task_id = self.next_id("purge");
        let rx = self.open_validation(&task_id, tool, &json!({}), &level);
        let w = self.self_ref();
        js::spawn(async move {
            let v = rx.await.unwrap_or_else(|_| json!({ "behavior": "deny" }));
            let Some(o) = w.upgrade() else { return };
            if js::gs(&v, "behavior") == "allow" {
                purge(&o);
                o.emit(json!({ "type": "finish", "id": task_id, "status": "done" }));
            } else {
                o.emit(json!({ "type": "finish", "id": task_id, "status": "error" }));
            }
        });
        json!({ "ok": true, "pending": true })
    }

    /// Accord déjà donné à la demande de permission de Claude Code pour cet outil ?
    pub fn consume_pre_approval(&self, tool: &str) -> bool {
        match self.pre_approved.borrow_mut().remove(tool) {
            Some(until) => until >= js::now_ms(),
            None => false,
        }
    }

    pub fn approve(&self, task_id: &str) {
        let Some(entry) = self.pending.borrow().iter().find(|(k, _)| k == task_id).map(|(_, e)| e.clone()) else { return };
        let vtype = Some(js::gs(&entry, "validationType")).filter(|t| !t.is_empty()).unwrap_or_else(|| "code".into());
        self.record("validation_accordee", json!({ "tache": task_id, "outil": entry["tool"], "type": vtype }));
        self.resolve_pending(task_id, json!({ "behavior": "allow" }));
        self.emit(json!({ "type": "update", "id": task_id, "status": "running", "code": null, "detail": "autorisé, exécution…" }));
    }

    pub fn deny(&self, task_id: &str) {
        let Some(entry) = self.pending.borrow().iter().find(|(k, _)| k == task_id).map(|(_, e)| e.clone()) else { return };
        let vtype = Some(js::gs(&entry, "validationType")).filter(|t| !t.is_empty()).unwrap_or_else(|| "code".into());
        self.record("validation_refusee", json!({ "tache": task_id, "outil": entry["tool"], "type": vtype }));
        self.resolve_pending(task_id, json!({ "behavior": "deny", "message": "refusé par l’utilisateur" }));
        self.emit(json!({ "type": "update", "id": task_id, "status": "running", "code": null, "detail": "action refusée" }));
    }

    /// Arrêt complet : tâches interrompues, veilles et processus enfants arrêtés.
    pub fn stop(self: &Rc<Self>) {
        self.stopped.set(true);
        self.cancel_all();
        self.index_timer.clear();
        self.tmux_watch_timer.clear();
        self.claude_watch_timer.clear();
        self.desktop_refresh_timer.clear();
        self.projects_memory_timer.clear();
        self.agy_unblock_timer.clear();
        self.reminders.stop();
        self.schedules.stop();
        self.mail_watch.stop();
        self.ai_watch.stop();
        self.terminal_colors.stop();
        self.context.stop();
        self.skills.stop();
        self.telegram.stop();
        self.nomade.stop();
        self.notes.stop(); // note non enregistrée : gardée
        self.meetings.shutdown(); // réunion en cours : archivée
        self.desktop_state.stop();
        self.browser_bridge.stop();
        if let Some(e) = &self.embedder {
            e.stop();
        }
        self.telemetry_timer.clear();
        self.audio_timer.clear();
        self.cli_daemon.stop();
        self.media.stop();
        self.context_mute.stop();
        if let Some(h) = self.http_server.borrow_mut().take() {
            h.abort();
        }
    }
}
