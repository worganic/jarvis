// Orchestrateur : démarrage, optimisation, autodiagnostic, configuration MCP des moteurs, suivi des
// appels IA, annulations et arrêt général (orchestrator/index.js, lignes 518 à 1222).

use std::path::PathBuf;
use std::rc::Rc;

use chrono::Timelike;
use serde_json::{json, Map, Value};

use super::{approval_port, Orchestrator, AiCall};
use crate::js::{self, J};

impl Orchestrator {
    /// Configuration MCP d'AGY : serveurs internes et connecteurs gérés par Jarvis (ceux ajoutés à la
    /// main par l'utilisateur ne sont jamais touchés).
    pub fn ensure_agy_mcp(&self) {
        let Some(path) = &self.agy_config_path else { return };
        let r = (|| -> Result<(), String> {
            let mut config = js::read_json(path).unwrap_or(json!({ "mcpServers": {} }));
            if !config["mcpServers"].is_object() {
                config["mcpServers"] = json!({});
            }
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let env = json!({ "JARVIS_APPROVAL_PORT": approval_port().to_string() });
            for name in ["jarvis", "system", "google"] {
                config["mcpServers"][name] = json!({ "command": exe.to_string_lossy(), "args": ["mcp", name], "env": env });
            }
            let managed_file = self.runtime_dir.join("agy-managed.json");
            for id in js::read_json(&managed_file).map(|m| js::str_list(&m)).unwrap_or_default() {
                if let Some(o) = config["mcpServers"].as_object_mut() {
                    o.remove(&id);
                }
            }
            let external = self.mcp_registry.agy_entries(Some(&exe));
            for (id, entry) in &external {
                config["mcpServers"][id] = entry.clone();
            }
            js::write_json(&managed_file, &json!(external.keys().cloned().collect::<Vec<_>>())).map_err(|e| e.to_string())?;
            js::write_json(path, &config).map_err(|e| e.to_string())
        })();
        if let Err(e) = r {
            self.log(format!("configuration MCP AGY : {e}"));
        }
    }

    pub fn start(self: &Rc<Self>) {
        let _ = std::fs::create_dir_all(&self.workspace);
        if let Err(e) = self.write_mcp_config() {
            self.log(format!("configuration MCP : {e}"));
        }
        self.ensure_agy_mcp();
        self.start_http();
        self.context_mute.start();
        self.started.set(true);
        self.sync_profile_memory();
        self.sync_projects_memory();
        self.refresh_daemon();
        self.reminders.start();
        self.schedules.start();
        let testing = js::env("JARVIS_TEST").is_some();
        if !testing {
            let me = Rc::downgrade(self);
            if self.desktop_state.start() {
                let m = me.clone();
                self.desktop_state.on("change", move |_| {
                    if let Some(o) = m.upgrade() {
                        o.refresh_desktop_card();
                    }
                });
            }
            self.terminal_voice.watch();
            self.browser_bridge.start();
            let m = me.clone();
            self.browser_bridge.on("change", move |_| {
                if let Some(o) = m.upgrade() {
                    o.refresh_desktop_card();
                }
            });
            self.meetings.watch();
            // Page Meet ouverte ou fermée : la voix de Jarvis se coupe / revient (sourdine contextuelle).
            let meet_refresh = {
                let m = me.clone();
                move || {
                    if let Some(o) = m.upgrade() {
                        let cm = o.context_mute.clone();
                        js::spawn(async move {
                            let _ = cm.refresh_meet(false).await;
                        });
                    }
                }
            };
            let r1 = meet_refresh.clone();
            self.browser_bridge.on("change", move |_| r1());
            let r2 = meet_refresh.clone();
            self.desktop_state.on("change", move |_| r2());
            js::set_timeout(5000, meet_refresh);
            self.automations.start();
            self.agents.start();
            self.mail_watch.start();
            self.companion.start();
            self.ai_watch.start();
            self.terminal_colors.start();
            self.context.start();
            self.skills.watch();
            self.telegram.start();
            self.nomade.start();
            self.reseau.start();
            self.collections.start();
            self.start_build_watch();
            // Claude Code déjà ouverts dans psmux : annoncés une fois Jarvis prêt, puis suivis.
            let tv = self.tmux_voice.clone();
            js::set_timeout(9000, move || {
                let tv2 = tv.clone();
                js::spawn(async move { tv2.announce().await });
            });
            let tv = self.tmux_voice.clone();
            self.tmux_watch_timer.every(30000, move || {
                let tv2 = tv.clone();
                js::spawn(async move { tv2.watch().await });
            });
            let tv = self.tmux_voice.clone();
            self.claude_watch_timer.every(4000, move || {
                let tv2 = tv.clone();
                js::spawn(async move { tv2.watch_claude().await });
            });
        }
        self.publish_vocabulary(true);
        // Indexation sémantique en arrière-plan, après le démarrage.
        let me = Rc::downgrade(self);
        self.index_timer.set(20000, move || {
            if let Some(o) = me.upgrade() {
                js::spawn(async move {
                    match o.memory.index().await {
                        Ok(n) if n > 0 => o.log(format!("[mémoire sémantique] {n} souvenir(s) indexé(s)")),
                        Ok(_) => {}
                        Err(e) => o.log(format!("[mémoire sémantique] indexation reportée : {e}")),
                    }
                });
            }
        });
        self.publish_memory();
        let zones: Vec<String> = self.zones.list().iter().map(|z| js::gs(z, "name")).collect();
        let engine = self.status.d.borrow().engine.clone();
        self.record("demarrage", json!({ "workspace": self.workspace.to_string_lossy(), "moteur": engine, "souvenirs": self.memory.len(), "zones": zones }));
        if let Some(fork) = self.history.acknowledge_forks() {
            let d = &fork["data"];
            self.log(format!(
                "journal : embranchement constaté ({} bifurcation(s) du {} au {}, aucune entrée modifiée ni supprimée)",
                js::s(&d["bifurcations"]),
                js::gs(d, "debut"),
                js::gs(d, "fin")
            ));
        }
        let integrity = self.history.verify();
        self.log(format!(
            "orchestrateur prêt · moteur: {engine} · Claude: {} · AGY: {} · dossier {} · {} souvenirs · journal {} entrées {} · zones: {}",
            self.claude,
            self.agy,
            self.workspace.display(),
            self.memory.len(),
            js::s(&integrity["count"]),
            if js::gb(&integrity, "ok") { "intact".to_string() } else { format!("ALTÉRÉ ({} en {})", js::gs(&integrity, "reason"), js::s(&integrity["at"])) },
            zones.join(", ")
        ));
        let me = self.clone();
        js::spawn(async move {
            me.perform_self_check(false).await;
        });
        self.show_default_desktop_card(0);
        self.apply_performance();
    }

    // --- Optimisation ------------------------------------------------------------------------------

    pub fn performance_settings(&self) -> J {
        self.config.get("performance")
    }

    /// Télémétrie CPU & RAM diffusée vers l'interface (et alimentant le mode automatique).
    pub fn start_telemetry(self: &Rc<Self>) {
        if self.telemetry_timer.active() {
            return;
        }
        if self.hardware_probe.borrow().is_none() {
            *self.hardware_probe.borrow_mut() = Some(super::hardware::HardwareProbe::new());
        }
        let probe = self.hardware_probe.borrow().clone().unwrap();
        probe.start();
        let push = {
            let me = Rc::downgrade(self);
            move || {
                let Some(o) = me.upgrade() else { return };
                let telem = super::system::telemetry();
                *o.last_telemetry.borrow_mut() = telem.clone();
                let mut ev = json!({ "type": "telemetry" });
                js::assign(&mut ev, telem.clone());
                if let Some(p) = o.hardware_probe.borrow().as_ref() {
                    js::assign(&mut ev, p.telemetry());
                }
                o.emit(ev);
                let changed = o.auto_perf.borrow().as_ref().filter(|a| a.sample(&telem)).map(|a| a.state());
                if let Some(state) = changed {
                    o.on_auto_perf_change(&state);
                }
                if js::gb(&telem, "isOverloaded") && js::now_ms() - o.last_overload_log.get() > 30000 {
                    o.last_overload_log.set(js::now_ms());
                    crate::logger::logger().warn("system:telemetry", &format!("Charge système élevée : CPU {}%, RAM {}% (Mode éco automatique activé)", js::s(&telem["cpu"]), js::s(&telem["ramPct"])));
                }
            }
        };
        push();
        self.telemetry_timer.every(1500, push);
    }

    pub fn stop_telemetry(&self) {
        if let Some(p) = self.hardware_probe.borrow().as_ref() {
            p.stop();
        }
        self.telemetry_timer.clear();
    }

    /// Périphérique audio (casque ou enceintes), détecté toutes les 20 s.
    pub fn start_audio_watch(self: &Rc<Self>) {
        if self.audio_timer.active() {
            return;
        }
        self.publish_audio_device();
        let me = Rc::downgrade(self);
        self.audio_timer.every(20000, move || {
            if let Some(o) = me.upgrade() {
                o.publish_audio_device();
            }
        });
    }

    pub fn stop_audio_watch(&self) {
        self.audio_timer.clear();
    }

    /// Applique les réglages d'optimisation (télémétrie, veille audio, mémoire sémantique). Idempotent.
    pub fn apply_performance(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        let base = self.performance_settings();
        let auto = js::gb(&base, "autoMode");
        let has = self.auto_perf.borrow().is_some();
        if auto && !has {
            *self.auto_perf.borrow_mut() = Some(super::auto_perf::AutoPerf::default());
        } else if !auto && has {
            *self.auto_perf.borrow_mut() = None;
        }
        let perf = self.effective_performance();
        if js::gb(&base, "hardwareMonitoring") || auto {
            self.start_telemetry();
        } else {
            self.stop_telemetry();
        }
        if js::gb(&perf, "audioDeviceWatch") {
            self.start_audio_watch();
        } else {
            self.stop_audio_watch();
            if !self.audio_probed_once.get() {
                self.audio_probed_once.set(true);
                self.publish_audio_device();
            }
        }
        self.apply_semantic_memory(js::gb(&perf, "semanticMemory"));
        self.publish_performance();
    }

    /// Coupe ou rétablit la mémoire sémantique (le modèle d'embeddings n'est plus chargé).
    pub fn apply_semantic_memory(&self, on: bool) {
        if on == !self.semantic_disabled.get() {
            return;
        }
        self.semantic_disabled.set(!on);
        if on {
            self.memory.attach_embedder(self.embedder.clone());
            self.log("[mémoire sémantique] réactivée".into());
        } else {
            self.memory.attach_embedder(None);
            self.index_timer.clear();
            if let Some(e) = &self.embedder {
                e.stop();
            }
            self.log("[mémoire sémantique] désactivée (optimisation) : recherche lexicale seulement".into());
        }
    }

    /// Réglages réellement appliqués : ceux de l'utilisateur, réduits par le mode automatique.
    pub fn effective_performance(&self) -> J {
        let base = self.performance_settings();
        if !js::gb(&base, "autoMode") {
            return base;
        }
        match self.auto_perf.borrow().as_ref() {
            Some(a) => a.effective(&base)["settings"].clone(),
            None => base,
        }
    }

    pub fn auto_perf_info(&self) -> J {
        let base = self.performance_settings();
        let ap = self.auto_perf.borrow();
        match ap.as_ref().filter(|_| js::gb(&base, "autoMode")) {
            None => json!({ "enabled": false, "cpuLevel": 0, "ramLevel": 0, "overridden": [], "labels": [] }),
            Some(a) => {
                let eff = a.effective(&base);
                let mut info = json!({ "enabled": true });
                js::assign(&mut info, a.state());
                info["overridden"] = eff["overridden"].clone();
                info["labels"] = eff["labels"].clone();
                info
            }
        }
    }

    /// Le niveau de charge vient de changer : on applique, on trace et on prévient le noyau.
    pub fn on_auto_perf_change(self: &Rc<Self>, state: &J) {
        let info = self.auto_perf_info();
        let labels = js::str_list(&info["labels"]);
        self.log(format!(
            "[optimisation auto] niveau CPU {}/3, RAM {}/2 : {}",
            js::s(&state["cpuLevel"]),
            js::s(&state["ramLevel"]),
            if labels.is_empty() { "tout rétabli".to_string() } else { format!("réduit → {}", labels.join(", ")) }
        ));
        self.record("optimisation_auto", json!({ "cpu": state["cpuLevel"], "ram": state["ramLevel"], "options": labels }));
        self.apply_performance();
        let cb = self.on_auto_perf_applied.borrow().clone();
        if let Some(cb) = cb {
            cb(&self.effective_performance());
        }
    }

    pub fn publish_performance(&self) {
        let mut ev = json!({ "type": "perf_config" });
        js::assign(&mut ev, self.effective_performance());
        ev["auto"] = self.auto_perf_info();
        self.emit(ev);
    }

    pub fn publish_audio_device(self: &Rc<Self>) {
        let me = self.clone();
        js::spawn(async move {
            let dev = super::system::get_audio_device().await;
            if js::gb(&dev, "ok") {
                let mut ev = json!({ "type": "audio_device" });
                js::assign(&mut ev, dev);
                me.emit(ev);
            }
        });
    }

    /// Autodiagnostic de tous les sous-systèmes (au démarrage, ou demandé).
    pub async fn perform_self_check(self: &Rc<Self>, manual: bool) -> J {
        let check_id = self.next_id("diag");
        self.emit(json!({ "type": "create", "id": check_id, "label": if manual { "DIAGNOSTIC SYSTÈME" } else { "CONTRÔLE INITIAL DES SYSTÈMES" }, "detail": "Vérification en cours de chaque composant…" }));
        // Au démarrage, pas de « tout est opérationnel » tant que le moteur vocal n'est pas lancé.
        let probe = self.voice_probe.borrow().clone();
        if !manual {
            if let Some(p) = &probe {
                let deadline = js::now_ms() + 120_000;
                while p() == "wait" && js::now_ms() < deadline {
                    js::sleep_ms(500).await;
                }
            }
        }
        let voice_state = probe.map(|p| p()).unwrap_or_else(|| "off".into());
        let logger = crate::logger::logger();
        let mut issues: Vec<String> = vec![];
        let mut checks = json!({ "workspace": false, "engine": false, "journal": false, "memory": false, "zones": false, "tools": false });
        // 1. Espace de travail
        match std::fs::create_dir_all(&self.workspace).and_then(|_| {
            let probe = self.workspace.join(".jarvis-ecriture");
            std::fs::write(&probe, b"ok")?;
            std::fs::remove_file(&probe)
        }) {
            Ok(_) => checks["workspace"] = json!(true),
            Err(e) => {
                issues.push(format!("Espace de travail ({e})"));
                logger.error("self_check:workspace", &e.to_string());
            }
        }
        // 2. Moteur LLM actif
        let engine = self.status.d.borrow().engine.clone();
        let (exe, name) = if engine == "agy" { (self.agy.clone(), "AGY") } else { (self.claude.clone(), "Claude Code") };
        if PathBuf::from(&exe).exists() || exe == "agy" || exe == "claude" {
            checks["engine"] = json!(true);
        } else {
            issues.push(format!("Binaire {name} introuvable"));
            logger.warn("self_check:engine", &format!("Binaire {name} introuvable"));
        }
        // 3. Intégrité du journal scellé
        let integrity = self.history.verify();
        if js::gb(&integrity, "ok") {
            checks["journal"] = json!(true);
        } else {
            issues.push(format!("Journal scellé altéré ({})", js::gs(&integrity, "reason")));
            logger.critical("self_check:journal", &format!("Journal scellé compromis : {}", js::gs(&integrity, "reason")));
        }
        // 4. Mémoire long terme
        checks["memory"] = json!(true);
        // 5. Outils natifs : intégrés au noyau (jarvis-native), toujours présents.
        checks["tools"] = json!(true);
        // 6. Zones d'accès
        checks["zones"] = json!(!self.zones.list().is_empty());
        if voice_state == "wait" {
            issues.push("Service vocal non démarré".into());
        }
        let ok = issues.is_empty();
        self.record("auto_diagnostic", json!({ "ok": ok, "checks": checks, "issues": issues, "manuel": manual }));
        let (cid, me) = (check_id.clone(), Rc::downgrade(self));
        if ok {
            self.emit(json!({ "type": "update", "id": check_id, "detail": "Tous les systèmes sont opérationnels" }));
            js::set_timeout(2500, move || {
                if let Some(o) = me.upgrade() {
                    o.emit(json!({ "type": "finish", "id": cid, "status": "done" }));
                }
            });
            logger.info("self_check", "Diagnostic initial réussi : 100% nominal");
        } else {
            self.emit(json!({ "type": "update", "id": check_id, "detail": format!("Attention : {}", issues.join(", ")) }));
            js::set_timeout(3500, move || {
                if let Some(o) = me.upgrade() {
                    o.emit(json!({ "type": "finish", "id": cid, "status": "error" }));
                }
            });
            logger.warn("self_check", &format!("Diagnostic avec alertes : {}", issues.join(" · ")));
        }
        let h = chrono::Local::now().hour();
        let salutation = if h >= 18 || h < 5 { "Bonsoir" } else { "Bonjour" };
        let spoken;
        if manual {
            spoken = if ok {
                "Diagnostic terminé. Tous les systèmes sont opérationnels et le journal scellé est certifié intact.".to_string()
            } else {
                format!("Diagnostic terminé. Attention : {}. Les autres fonctionnalités sont opérationnelles.", issues.join(", "))
            };
            self.emit(json!({ "type": "say", "text": spoken }));
        } else {
            // Mémoire vide (premier lancement ou purge) : Jarvis engage la prise de contact.
            if self.memory.is_empty() {
                let me = Rc::downgrade(self);
                js::set_timeout(2200, move || {
                    if let Some(o) = me.upgrade() {
                        o.start_onboarding();
                    }
                });
                return json!({ "ok": ok, "spoken": "Protocole de prise de contact démarré", "issues": issues, "checks": checks });
            }
            let a = self.addressing_forms();
            let appellation = if a.call.is_empty() { String::new() } else { format!(" {}", a.call) };
            let bday = if self.memory.is_birthday_today() { format!(" Je {} souhaite un très bel anniversaire !", a.te) } else { String::new() };
            spoken = if ok {
                format!("{salutation}{appellation}.{bday} Tous les systèmes sont opérationnels et vérifiés. Je suis prêt et à {} écoute.", a.ton)
            } else {
                format!("{salutation}{appellation}.{bday} Initialisation terminée. Attention requise sur : {}. Je reste à {} service.", issues.join(", "), a.ton)
            };
            let (me, s) = (Rc::downgrade(self), spoken.clone());
            js::set_timeout(2000, move || {
                if let Some(o) = me.upgrade() {
                    o.emit(json!({ "type": "say", "text": s }));
                }
            });
        }
        json!({ "ok": ok, "spoken": spoken, "issues": issues, "checks": checks })
    }

    // --- Configuration MCP des moteurs ---------------------------------------------------------------
    // Écrite dans data/runtime/ : elle peut contenir les secrets des connecteurs, et Claude Code n'a
    // jamais accès au dossier de données.

    pub fn mcp_servers(&self, task_id: Option<&str>) -> Map<String, Value> {
        let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "jarvis-core".into());
        let mut env = json!({ "JARVIS_APPROVAL_PORT": approval_port().to_string() });
        if let Some(t) = task_id {
            env["JARVIS_TASK_ID"] = json!(t);
        }
        let mut out = Map::new();
        for name in ["jarvis", "system", "google"] {
            out.insert(name.into(), json!({ "command": exe, "args": ["mcp", name], "env": env }));
        }
        for (k, v) in self.mcp_registry.claude_entries() {
            out.insert(k, v);
        }
        out
    }

    pub fn write_mcp_config(&self) -> Result<(), String> {
        js::write_json(&self.mcp_config, &json!({ "mcpServers": self.mcp_servers(None) })).map_err(|e| e.to_string())
    }

    pub fn create_task_mcp_config(&self, task_id: &str) -> PathBuf {
        let p = self.runtime_dir.join(format!("mcp-{task_id}.json"));
        match js::write_json(&p, &json!({ "mcpServers": self.mcp_servers(Some(task_id)) })) {
            Ok(_) => p,
            Err(_) => self.mcp_config.clone(),
        }
    }

    /// Connecteurs modifiés (administration) : configurations réécrites, sécurité et démon à jour.
    pub fn refresh_connectors(&self) {
        self.security.set_extra(self.mcp_registry.security_entries());
        if let Err(e) = self.write_mcp_config() {
            self.log(format!("configuration MCP : {e}"));
        }
        self.ensure_agy_mcp();
        self.refresh_daemon();
        self.emit(json!({ "type": "mcp" }));
    }

    pub fn resolve_task_id(&self, payload: &J) -> Option<String> {
        if js::truthy(&payload["taskId"]) {
            return Some(js::gs(payload, "taskId"));
        }
        let running = self.running.borrow();
        if running.len() == 1 {
            return Some(running[0].0.clone());
        }
        running.iter().rev().find(|(_, e)| !e.background).map(|(k, _)| k.clone()).or_else(|| running.last().map(|(k, _)| k.clone()))
    }

    /// Modèle et effort : réglage manuel, sinon choix automatique selon la complexité de la demande.
    pub fn resolve_task_model_and_effort(&self, text: &str, engine: &str) -> J {
        let req = self.status.d.borrow().model.clone().unwrap_or_default();
        self.resolve_task_model_and_effort_with(text, engine, &req)
    }

    pub fn effort_for_model(engine: &str, model: &str, fallback: Option<String>) -> Option<String> {
        if engine == "agy" {
            if model.ends_with("-low") {
                return Some("low".into());
            }
            if model.ends_with("-medium") {
                return Some("medium".into());
            }
            if model.ends_with("-high") {
                return Some("high".into());
            }
        }
        fallback
    }

    pub fn resolve_task_model_and_effort_with(&self, text: &str, engine: &str, requested_model: &str) -> J {
        let d = self.status.d.borrow();
        let model = if !requested_model.is_empty() {
            requested_model.to_string()
        } else {
            d.model.clone().unwrap_or_default()
        };
        let foreign = (engine == "claude" && (model.to_lowercase().contains("gemini") || super::status::is_agy_model_key(&model)))
            || (engine == "agy" && super::status::CLAUDE_MODELS.contains(&model.as_str()));
        let complexity = super::detect_task_complexity(text);
        let explicit = !model.is_empty() && model != "auto" && !foreign;
        if explicit && (!d.economy_mode || complexity != "simple") {
            let effort = Self::effort_for_model(engine, &model, d.effort.clone());
            return json!({ "model": model, "effort": effort, "auto": false, "complexity": "manual", "label": model });
        }
        let effort = d.effort.clone();
        match (engine, complexity) {
            ("claude", "simple") => json!({ "model": "haiku", "effort": "low", "auto": true, "complexity": "simple", "label": "Haiku (Éco)" }),
            ("claude", _) => json!({ "model": "sonnet", "effort": effort.unwrap_or_else(|| "medium".into()), "auto": true, "complexity": "complex", "label": "Sonnet" }),
            (_, "simple") => json!({ "model": "gemini-3.8-flash-low", "effort": "low", "auto": true, "complexity": "simple", "label": "Flash Low (Éco)" }),
            _ => json!({ "model": "gemini-3.8-flash-high", "effort": "high", "auto": true, "complexity": "complex", "label": "Flash High" }),
        }
    }

    // --- Appels aux IA externes : compteur du jour et cœur orange du HUD -------------------------------

    pub fn start_ai_call(&self, id: &str, engine: &str, model: &str, prompt: &str, background: bool) {
        let mut chars = prompt.encode_utf16().count();
        if engine == "agy" && self.session_id_for("agy").is_none() {
            chars += self.agy_preamble().encode_utf16().count();
        }
        let call = AiCall { engine: engine.into(), model: model.into(), started_at: js::now_ms(), prompt_chars: chars, usage_seen: false, background };
        self.ai_calls.borrow_mut().insert(id.into(), call.clone());
        self.usage_daily.add_call(engine);
        self.emit(json!({ "type": "ai_call", "state": "start", "id": id, "engine": engine, "model": model, "startedAt": call.started_at, "background": background }));
        self.publish_status();
    }

    /// Relevé de tokens reçu en fin de tour (événement `result` du moteur).
    pub fn record_ai_usage(&self, id: &str, event: &J) {
        let Some(usage) = super::usage_daily::extract_usage(event) else { return };
        let engine = {
            let mut calls = self.ai_calls.borrow_mut();
            let Some(c) = calls.get_mut(id) else { return };
            c.usage_seen = true;
            c.engine.clone()
        };
        self.usage_daily.add_tokens(&engine, &usage, false);
        self.publish_status();
    }

    pub fn end_ai_call(&self, id: &str, answer: &str) {
        let Some(call) = self.ai_calls.borrow_mut().remove(id) else { return };
        // AGY ne fournit pas toujours son relevé : estimation marquée comme telle (≈).
        if !call.usage_seen && call.engine == "agy" {
            self.usage_daily.add_tokens(&call.engine, &json!({ "input": (call.prompt_chars as f64 / 4.0).ceil() as i64, "output": super::usage_daily::estimate_tokens(answer) }), true);
        }
        self.emit(json!({ "type": "ai_call", "state": "end", "id": id, "engine": call.engine, "durationMs": js::now_ms() - call.started_at }));
        self.publish_status();
    }

    fn remove_running(&self, task_id: &str) -> Option<super::RunningTask> {
        let mut running = self.running.borrow_mut();
        let i = running.iter().position(|(k, _)| k == task_id)?;
        let (_, entry) = running.remove(i);
        let current = self.running_task.borrow().clone();
        if current.as_deref() == Some(task_id) {
            *self.running_task.borrow_mut() = running.last().map(|(k, _)| k.clone());
        }
        Some(entry)
    }

    fn drop_task_mcp(&self, entry: &super::RunningTask) {
        if let Some(p) = &entry.task_mcp_path {
            if *p != self.mcp_config {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    pub fn finish_task(self: &Rc<Self>, task_id: &str) {
        self.end_ai_call(task_id, "");
        if let Some(entry) = self.remove_running(task_id) {
            self.drop_task_mcp(&entry);
        }
        self.next();
        self.refresh_daemon();
    }

    pub fn is_running(&self, task_id: &str) -> bool {
        self.running.borrow().iter().any(|(k, _)| k == task_id)
    }

    pub fn cancel(self: &Rc<Self>, task_id: &str) -> bool {
        if !self.is_running(task_id) {
            return false;
        }
        self.log(format!("interruption de la tâche {task_id}"));
        self.end_ai_call(task_id, "");
        if let Some(speech) = self.speech.borrow_mut().remove(task_id) {
            speech.cancel();
            self.emit(json!({ "type": "say_end", "task": task_id }));
        }
        let Some(entry) = self.remove_running(task_id) else { return false };
        if entry.is_daemon {
            self.cli_daemon.abort();
        } else {
            if entry.pid != 0 {
                crate::proc::kill_tree(entry.pid);
            }
            if let Some(c) = &entry.child {
                c.kill();
            }
        }
        self.drop_task_mcp(&entry);
        // Validations et questions en attente de cette tâche : refusées.
        self.resolve_pending(task_id, json!({ "behavior": "deny", "message": "tâche annulée" }));
        let matches = self.waiting_reply.borrow().as_ref().map(|w| w.task_id == task_id || task_id.is_empty()).unwrap_or(false);
        if matches {
            self.close_waiting_reply(json!({ "error": "tâche annulée" }));
        }
        self.record("tache_annulee", json!({ "tache": task_id, "moteur": entry.engine, "demande": entry.text, "fond": entry.background }));
        self.emit(json!({ "type": "update", "id": task_id, "detail": "tâche arrêtée par l’utilisateur", "cancelled": true }));
        self.emit(json!({ "type": "finish", "id": task_id, "status": "error" }));
        self.next();
        true
    }

    /// Répond à la question de précision en attente et la referme.
    pub fn close_waiting_reply(&self, reply: J) -> bool {
        let Some(mut w) = self.waiting_reply.borrow_mut().take() else { return false };
        w.timer.clear();
        if let Some(tx) = w.tx.take() {
            let _ = tx.send(reply);
        }
        true
    }

    pub fn cancel_all(self: &Rc<Self>) -> usize {
        self.queue.borrow_mut().clear();
        let ids: Vec<String> = self.running.borrow().iter().map(|(k, _)| k.clone()).collect();
        let mut count = ids.into_iter().filter(|id| self.cancel(id)).count();
        if self.close_waiting_reply(json!({ "error": "toutes les tâches ont été annulées" })) {
            count += 1;
        }
        if let Some(ob) = self.onboarding.borrow_mut().take() {
            self.emit(json!({ "type": "finish", "id": ob["taskId"], "status": "error" }));
            count += 1;
        }
        count
    }

    /// Arrêt général (STOP, Échap, « stop ») : validations refusées, tâches tuées, file vidée, relais
    /// et dictée terminés, voix coupée. La musique continue.
    pub fn stop_all(self: &Rc<Self>, origin: &str) -> J {
        let mut count = 0;
        if self.savoir.presenting() {
            self.savoir.end_presentation(false);
            count += 1;
        }
        if self.savoir.exploring().is_some() {
            self.savoir.close_explore(false);
            count += 1;
        }
        let mission_running = self.missions.current.borrow().as_ref().map(|m| js::gb(m, "running")).unwrap_or(false);
        if mission_running {
            self.missions.cancel("arrêtée");
            count += 1;
        }
        let pending: Vec<String> = self.pending.borrow().iter().map(|(k, _)| k.clone()).collect();
        for id in pending {
            self.deny(&id);
            count += 1;
        }
        count += self.cancel_all();
        if self.schedules.cancel_countdown(if origin == "echap" { "Échap" } else { "STOP" }).is_some() {
            count += 1;
        }
        if self.tmux_voice.relay.borrow().is_some() {
            self.tmux_voice.stop_relay("arret");
            count += 1;
        }
        if self.dictation.borrow().is_some() {
            self.stop_dictation("arret");
            count += 1;
        }
        // La prise de notes s'arrête ; une réunion en cours d'enregistrement continue.
        if self.notes.listening.get() {
            self.notes.set_listening(false);
            count += 1;
        }
        self.emit(json!({ "type": "voice_stop" }));
        self.emit(json!({ "type": "stopped", "count": count, "origin": origin }));
        self.record("arret_general", json!({ "origine": origin, "elements": count }));
        json!({ "ok": true, "count": count })
    }

    /// Routage spéculatif sur transcription partielle : pré-analyse et pré-chauffe dès la parole.
    pub fn on_speculative_utterance(&self, text: &str, meta: &J) {
        let clean = text.trim();
        if clean.chars().count() < 4 {
            return;
        }
        let from_voice = js::gs(meta, "source") != "typed";
        if let Some(intent) = self.router.route(clean, from_voice).filter(|i| !i.ignored && i.kind != "composite_intent") {
            if intent.kind.starts_with("music") {
                self.media.touch();
            }
            *self.speculative.borrow_mut() = Some((clean.to_string(), intent, js::now_ms()));
        }
    }
}
