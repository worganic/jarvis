// Orchestrateur : exécution des tâches IA (Claude Code, AGY, micro-appels) et lecture de leurs
// événements (orchestrator/index.js, lignes 2831-3293).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use super::policy::{describe, shorten};
use super::speech_stream::SpeechStream;
use super::{speakable, Orchestrator, RunningTask};
use crate::js::{self, J};
use crate::proc::LineChild;

/// Fin d'une tâche LLM (paramètres de completeTask).
pub struct Completion {
    pub engine: String,
    pub text: String,
    pub answer: String,
    pub ok: bool,
    pub background: bool,
    pub failure: J,
    pub fail_speech: Option<String>,
}

impl Completion {
    pub fn new(engine: &str, text: &str, answer: &str, ok: bool, background: bool) -> Self {
        Self {
            engine: engine.into(),
            text: text.into(),
            answer: answer.into(),
            ok,
            background,
            failure: json!({}),
            fail_speech: None,
        }
    }
}

/// Lecteur d'événements d'un moteur.
#[derive(Clone, Copy, PartialEq)]
enum EventKind {
    Claude,
    Agy,
}

/// Lancement d'un processus moteur dédié à une tâche.
struct Launch {
    id: String,
    exe: String,
    args: Vec<String>,
    stdin_text: String,
    engine: &'static str,
    text: String,
    background: bool,
    events: EventKind,
    meta: J,
    task_mcp_path: Option<PathBuf>,
    /// Message de journal et phrase en cas d'échec du lancement.
    launch_error: (&'static str, &'static str, Option<&'static str>),
    /// Validations encore en attente refusées à la fermeture du processus.
    deny_pending: bool,
    micro: bool,
}

impl Orchestrator {
    pub fn create_speech(&self, id: &str) -> Rc<SpeechStream> {
        let w = self.self_ref();
        let task = id.to_string();
        Rc::new(SpeechStream::new(Box::new(move |text| {
            if let Some(o) = w.upgrade() {
                o.emit(json!({ "type": "say", "text": text, "stream": true, "task": task }));
            }
        })))
    }

    fn running_meta(&self, id: &str) -> Option<J> {
        self.running.borrow().iter().find(|(k, _)| k == id).map(|(_, e)| e.meta.clone())
    }

    /// Premier mot d'une tâche IA : délai depuis son lancement, gardé pour le Checkup et le journal.
    fn note_first_word(&self, id: &str) {
        let mut running = self.running.borrow_mut();
        let Some((_, e)) = running.iter_mut().find(|(k, _)| k == id) else { return };
        if e.background || js::truthy(&e.meta["premierMotMs"]) {
            return;
        }
        let ms = (js::now_ms() - e.started_at).max(0);
        e.meta["premierMotMs"] = json!(ms);
        let mut v = self.first_word_ms.borrow_mut();
        v.push(ms);
        if v.len() > 30 {
            v.remove(0);
        }
    }

    /// (médiane, nombre de mesures) du délai avant le premier mot.
    pub fn first_word_stats(&self) -> Option<(i64, usize)> {
        let mut v = self.first_word_ms.borrow().clone();
        if v.is_empty() {
            return None;
        }
        v.sort_unstable();
        Some((v[v.len() / 2], v.len()))
    }

    fn mark_used_tools(&self, id: &str) {
        if let Some((_, e)) = self.running.borrow_mut().iter_mut().find(|(k, _)| k == id) {
            e.meta["usedTools"] = json!(true); // réponse liée à une action : jamais en cache
        }
    }

    fn mark_event_error(&self, id: &str) {
        if let Some((_, e)) = self.running.borrow_mut().iter_mut().find(|(k, _)| k == id) {
            e.meta["eventError"] = json!(true);
        }
    }

    /// Fin commune à toutes les tâches LLM (démon ou processus dédié, Claude ou AGY).
    pub fn complete_task(self: &Rc<Self>, id: &str, c: Completion) {
        let entry = self.running.borrow().iter().find(|(k, _)| k == id).map(|(_, e)| (e.is_daemon, e.meta.clone()));
        let speech = self.speech.borrow_mut().remove(id);
        let engine_label = if c.engine == "agy" { "AGY" } else { "Claude Code" };
        let (engine, text, answer, background) = (c.engine.as_str(), c.text.as_str(), c.answer.as_str(), c.background);

        if c.ok {
            self.emit(json!({ "type": "finish", "id": id, "status": "done" }));
            self.record(
                "tache_fin",
                json!({
                    "tache": id, "moteur": engine, "statut": "terminee",
                    // Réponse intégrale : le journal garde tout le dialogue (bornée à 8 000 caractères).
                    "reponse": js::slice_chars(answer, 8000),
                    "fond": background,
                    "premier_mot_ms": entry.as_ref().map(|(_, m)| m["premierMotMs"].clone()).unwrap_or(Value::Null)
                }),
            );
            // Tâche « terminée »… en expliquant qu'elle ne pouvait pas le faire : c'est un échec à apprendre.
            if let Some(cannot) = super::failures::incapable(answer) {
                self.failures.add(json!({
                    "kind": "ia", "demande": text, "moteur": engine,
                    "erreur": format!("l'IA a répondu qu'elle ne pouvait pas : « {cannot} »"), "tache": id
                }));
            }
            if !answer.is_empty() {
                let at = js::now_ms();
                // Question factuelle générale sans outil : resservie localement pendant 30 min (v5).
                if !background {
                    let used = entry.as_ref().map(|(_, m)| js::gb(m, "usedTools")).unwrap_or(false);
                    self.answer_cache.put(text, answer, used);
                }
                *self.last_answer.borrow_mut() = Some(json!({ "id": id, "text": answer, "query": text, "at": at, "engine": engine }));
                self.emit(json!({ "type": "answer_full", "id": id, "text": answer, "query": text, "at": at, "engine": engine }));
            }
            self.missions.observe(text, true, answer);
            if background && self.savoir.done(text, answer, true) {
                // Dossier Savoir : relu, téléchargé, indexé et annoncé par savoir/mod.rs.
            } else if background && self.research.done(text, answer, true) {
                // Recherche web : annonce, rapport et sources gérés par research.rs.
            } else if background && self.shopping.done(text, answer, true) {
                // Achat : annonce du moins cher et card des offres gérées par shopping.rs.
            } else if background && self.collections.done(text, answer, true) {
                // Liste à fiches : champs proposés par l'IA, gérés par collections/voice.rs.
            } else if background && self.parcels.done(text, answer, true) {
                // Suivi de colis : statut annoncé par parcels.rs.
            } else if background {
                let spoken = if !answer.is_empty() {
                    format!("La tâche en fond {} est terminée : {}", shorten(text, 30), speakable(answer))
                } else {
                    format!("La tâche en fond {} est terminée.", shorten(text, 30))
                };
                self.emit(json!({ "type": "say", "text": spoken, "answer": true }));
            } else {
                // Si la réponse a déjà été lue au fil de l'eau, on ne prononce que la fin restante.
                let streamed = speech.as_ref().map(|s| s.end()).unwrap_or(false);
                if !answer.is_empty() {
                    if !streamed {
                        self.emit(json!({ "type": "say", "text": speakable(answer), "answer": true }));
                    }
                    *self.last_spoken.borrow_mut() = speakable(answer);
                }
            }
        } else {
            let err_str = js::gs(&c.failure, "erreur");
            if err_str.contains("No conversation found") || err_str.contains("conversation not found") {
                *self.session_id.borrow_mut() = None;
            }
            let quota_hit = engine == "agy" && (self.note_engine_error("agy", &err_str) || self.agy_blocked());
            let cur_idx = entry.as_ref().and_then(|(_, m)| m["chainIndex"].as_u64()).unwrap_or(0) as usize;
            let launch_failed_engine = js::gb(&c.failure, "launchError").then_some(engine);
            if let Some((next_engine, next_model, next_idx)) = self.next_available_ai_slot(cur_idx, launch_failed_engine) {
                // Bascule automatique vers l'IA suivante configurée dans la chaîne (jusqu'à 4 IA).
                if let Some(s) = &speech {
                    s.cancel();
                }
                let next_label = if next_engine == "agy" { "AGY" } else { "Claude Code" };
                let statut = if quota_hit { "quota_epuise" } else { "echec_relance" };
                let mut rec = json!({ "tache": id, "moteur": engine, "statut": statut, "rang": cur_idx + 1 });
                js::assign(&mut rec, c.failure.clone());
                rec["fond"] = json!(background);
                rec["relance"] = json!(format!("{next_engine}:{next_model}"));
                self.record("tache_fin", rec);
                self.log(format!("{engine_label} (IA #{}) : échec sur tâche {id}, bascule sur IA #{} ({next_label} · {next_model})", cur_idx + 1, next_idx + 1));
                let detail = if quota_hit && next_engine == "claude" {
                    "quota AGY épuisé → relance sur Claude".to_string()
                } else {
                    format!("échec → bascule sur IA #{} ({next_label} · {next_model})", next_idx + 1)
                };
                self.emit(json!({ "type": "update", "id": id, "detail": detail }));
                self.emit(json!({ "type": "finish", "id": id, "status": "error" }));
                if quota_hit && next_engine == "claude" {
                    self.announce_agy_fallback(true);
                }
                if speech.is_some() {
                    self.emit(json!({ "type": "say_end", "task": id }));
                }
                self.end_ai_call(id, answer);
                self.finish_task(id);
                let speaker = entry.as_ref().and_then(|(_, m)| self.speaker_of(m));
                self.execute_slot(
                    text,
                    background,
                    &next_engine,
                    json!({ "speaker": speaker, "chainIndex": next_idx, "slotModel": next_model }),
                );
                return;
            } else {
                if let Some(s) = &speech {
                    s.cancel();
                }
                let mut rec = json!({ "tache": id, "moteur": engine, "statut": "echec" });
                js::assign(&mut rec, c.failure.clone());
                rec["fond"] = json!(background);
                self.record("tache_fin", rec);
                let err = js::gs(&c.failure, "erreur");
                let reason = if !err.is_empty() {
                    err
                } else if let Some(code) = c.failure.get("code") {
                    format!("code {}", if code.is_null() { "null".to_string() } else { js::s(code) })
                } else {
                    "échec".into()
                };
                self.log(format!("{engine_label} : échec de la tâche {id} ({})", js::slice_chars(&reason, 400)));
                self.failures.add(json!({ "kind": "ia", "demande": text, "moteur": engine, "erreur": reason, "tache": id }));
                self.emit(json!({ "type": "update", "id": id, "detail": shorten(&reason, 60) }));
                self.emit(json!({ "type": "finish", "id": id, "status": "error" }));
                // Recherche web ratée : annoncée par son sujet, pas par la consigne envoyée à Claude.
                self.missions.observe(text, false, "");
                let research_failed = background && (self.savoir.done(text, "", false) || self.research.done(text, "", false) || self.shopping.done(text, "", false) || self.collections.done(text, "", false) || self.parcels.done(text, "", false));
                if !research_failed {
                    let say = c.fail_speech.clone().unwrap_or_else(|| {
                        if background {
                            format!("La tâche en fond {} a échoué.", shorten(text, 30))
                        } else {
                            format!("La tâche a échoué avec {engine_label}.")
                        }
                    });
                    self.emit(json!({ "type": "say", "text": say }));
                }
            }
        }

        // Fin de la prise de parole au fil de l'eau : la voix clôt la réponse d'un seul tenant.
        if speech.is_some() {
            self.emit(json!({ "type": "say_end", "task": id }));
        }

        // Une tâche hors démon a fait avancer la conversation : le démon la reprendra à jour.
        // v2 : une tâche parallèle (branche indépendante) ne touche pas à la conversation principale.
        if let Some((is_daemon, meta)) = &entry {
            if !is_daemon && !js::gb(meta, "micro") && !js::gb(meta, "parallel") {
                self.session_epoch.set(self.session_epoch.get() + 1);
            }
        }
        self.end_ai_call(id, answer);
        self.finish_task(id);
    }

    fn speaker_of(&self, meta: &J) -> Option<String> {
        let n = js::gs(&meta["speaker"], "name");
        if !n.is_empty() {
            return Some(n);
        }
        self.current_speaker.borrow().clone().filter(|s| !s.is_empty())
    }

    /// Lance le processus d'une tâche : demande sur l'entrée standard (jamais dans la ligne de
    /// commande), événements JSON lus ligne à ligne, fin de tâche à la fermeture.
    fn launch(self: &Rc<Self>, l: Launch) {
        let answer = Rc::new(RefCell::new(String::new()));
        let errors = Rc::new(RefCell::new(String::new()));
        let (w1, w2) = (self.self_ref(), self.self_ref());
        let (id1, id2) = (l.id.clone(), l.id.clone());
        let (a1, a2) = (answer.clone(), answer.clone());
        let e1 = errors.clone();
        let events = l.events;
        let (engine, text, background, deny_pending, micro) = (l.engine, l.text.clone(), l.background, l.deny_pending, l.micro);
        let envs = vec![("JARVIS_TASK_ID".to_string(), l.id.clone())];
        let spawned = LineChild::spawn_in(
            Some(&self.workspace),
            std::path::Path::new(&l.exe),
            &l.args,
            &envs,
            move |line| {
                let Some(o) = w1.upgrade() else { return };
                let Ok(evt) = serde_json::from_str::<Value>(&line) else { return }; // ligne non-JSON : ignorée
                let cur = a1.borrow().clone();
                let next = match events {
                    EventKind::Claude => o.on_claude_event(&id1, &evt, cur),
                    EventKind::Agy => o.on_agy_event(&id1, &evt, cur),
                };
                *a1.borrow_mut() = next;
            },
            move |line| {
                let mut e = e1.borrow_mut();
                e.push_str(&line);
                e.push('\n');
            },
            move |code| {
                let Some(o) = w2.upgrade() else { return };
                if deny_pending {
                    o.resolve_pending(&id2, json!({ "behavior": "deny", "message": "tâche terminée" }));
                }
                // Tâche annulée par l'utilisateur ou déjà close : rien de plus à annoncer.
                if !o.is_running(&id2) {
                    return;
                }
                let answer = a2.borrow().clone();
                let had_err = o.running_meta(&id2).map(|m| js::gb(&m, "eventError")).unwrap_or(false);
                let ok = code == Some(0) && !had_err;
                if micro {
                    // Échange gardé pour qu'une relance (« et pourquoi ? ») passe par le chemin complet avec son contexte.
                    if ok && !answer.is_empty() {
                        *o.last_micro.borrow_mut() = Some((text.clone(), answer.clone(), js::now_ms()));
                    }
                    o.session_last_at.set(js::now_ms());
                }
                let mut c = Completion::new(engine, &text, &answer, ok, background);
                let err_msg = {
                    let e = errors.borrow();
                    if !e.trim().is_empty() {
                        shorten(&e, 300)
                    } else if had_err && !answer.trim().is_empty() {
                        shorten(&answer, 300)
                    } else {
                        String::new()
                    }
                };
                c.failure = json!({ "code": code, "erreur": err_msg });
                o.complete_task(&id2, c);
            },
        );
        match spawned {
            Ok(child) => {
                self.running.borrow_mut().push((
                    l.id.clone(),
                    RunningTask {
                        text: l.text.clone(),
                        engine: engine.into(),
                        background,
                        started_at: js::now_ms(),
                        is_daemon: false,
                        pid: child.pid,
                        child: Some(child.clone()),
                        task_mcp_path: l.task_mcp_path,
                        meta: l.meta,
                    },
                ));
                *self.running_task.borrow_mut() = Some(l.id.clone());
                let stdin_text = l.stdin_text;
                js::spawn(async move {
                    child.write_and_close(&stdin_text).await;
                });
            }
            Err(err) => {
                // Échec du lancement : la tâche est enregistrée puis close aussitôt, comme en v3.
                self.running.borrow_mut().push((
                    l.id.clone(),
                    RunningTask {
                        text: l.text.clone(),
                        engine: engine.into(),
                        background,
                        started_at: js::now_ms(),
                        is_daemon: false,
                        pid: 0,
                        child: None,
                        task_mcp_path: l.task_mcp_path,
                        meta: l.meta,
                    },
                ));
                *self.running_task.borrow_mut() = Some(l.id.clone());
                let (log_msg, missing, speech) = l.launch_error;
                self.log(format!("{log_msg} : {err}"));
                let mut c = Completion::new(engine, &l.text, &answer.borrow(), false, background);
                c.failure = json!({ "erreur": missing, "launchError": true });
                c.fail_speech = speech.map(String::from);
                self.complete_task(&l.id, c);
            }
        }
    }

    pub fn execute_claude(self: &Rc<Self>, text: &str, background: bool, meta: J) {
        let id = self.next_id("c");
        let slot_model = js::gs(&meta, "slotModel");
        let chain_index = meta["chainIndex"].as_u64().unwrap_or(0) as usize;
        let mut resolved = self.resolve_task_model_and_effort_with(text, "claude", &slot_model);
        if let Some(tools) = self.savoir.tools_for(text) {
            resolved["tools"] = json!(tools);
        }
        let label = if background { format!("[FOND] {}", shorten(text, 20)) } else { shorten(text, 26) };
        let speaker = self.speaker_of(&meta);
        let rlabel = js::gs(&resolved, "label");
        let rank_tag = if chain_index > 0 { format!(" [IA #{}]", chain_index + 1) } else { String::new() };
        self.emit(json!({ "type": "create", "id": id, "label": label, "detail": format!("{rlabel}{rank_tag} · transmission…"), "background": background }));
        self.record(
            "tache_debut",
            json!({
                "tache": id, "moteur": "claude", "demande": text, "fond": background,
                "modele": resolved["model"], "effort": resolved["effort"],
                "eco": js::gb(&resolved, "auto") && js::gs(&resolved, "complexity") == "simple",
                "locuteur": speaker, "rang": chain_index + 1
            }),
        );
        if !background {
            self.speech.borrow_mut().insert(id.clone(), self.create_speech(&id));
        }

        let prompt = format!("{}{}", self.micro_context(), match &speaker {
            Some(s) => format!("[Locuteur : {s}]\n{text}"),
            None => text.to_string(),
        });
        self.start_ai_call(&id, "claude", &rlabel, &prompt, background);

        let parallel = self.is_parallel();
        let task_meta = json!({ "speaker": speaker, "parallel": parallel, "background": background, "chainIndex": chain_index, "slotModel": slot_model });

        // Processus résident déjà chaud : pas de démarrage à froid.
        if !background && chain_index == 0 && self.try_daemon_with_meta(&id, text, &prompt, &resolved, "claude", task_meta.clone()) {
            return;
        }

        let task_mcp_path = self.create_task_mcp_config(&id);
        let session = if background { Vec::new() } else { self.session_args("claude") };
        let mut args = self.claude_args(&resolved, false, &task_mcp_path);
        let forked = parallel && !session.is_empty();
        args.extend(session);
        if forked {
            args.push("--fork-session".into());
        }
        self.launch(Launch {
            id,
            exe: self.claude.clone(),
            args,
            stdin_text: prompt,
            engine: "claude",
            text: text.to_string(),
            background,
            events: EventKind::Claude,
            meta: task_meta,
            task_mcp_path: Some(task_mcp_path),
            launch_error: ("échec du lancement de Claude Code", "Claude Code introuvable", Some("Je n'arrive pas à lancer Claude Code.")),
            deny_pending: true,
            micro: false,
        });
    }

    /// Micro-appel (v5) : processus Claude Code dédié, sans outil, sans MCP, sans session.
    pub fn execute_micro(self: &Rc<Self>, text: &str, meta: J) {
        let id = self.next_id("c");
        let speaker = self.speaker_of(&meta);
        self.emit(json!({ "type": "create", "id": id, "label": shorten(text, 26), "detail": "Haiku (micro-appel) · transmission…" }));
        self.record(
            "tache_debut",
            json!({ "tache": id, "moteur": "claude", "demande": text, "fond": false, "modele": "haiku", "effort": "low", "micro": true, "locuteur": speaker }),
        );
        self.speech.borrow_mut().insert(id.clone(), self.create_speech(&id));
        self.start_ai_call(&id, "claude", "Haiku · micro", text, false);

        let profile = self.config.get("profile");
        let dialogue = super::config::dialogue_instructions(&profile, self.memory.get_user_name().as_deref());
        let args = super::micro_call::micro_args(&super::micro_call::micro_system_prompt(&dialogue), "haiku");
        let stdin_text = match &speaker {
            Some(s) => format!("[Locuteur : {s}]\n{text}"),
            None => text.to_string(),
        };
        self.launch(Launch {
            id,
            exe: self.claude.clone(),
            args,
            stdin_text,
            engine: "claude",
            text: text.to_string(),
            background: false,
            events: EventKind::Claude,
            meta: json!({ "speaker": speaker, "micro": true, "chainIndex": 0 }),
            task_mcp_path: None,
            launch_error: ("échec du micro-appel", "Claude Code introuvable", None),
            deny_pending: false,
            micro: true,
        });
    }

    /// Préfixe de contexte pour la conversation complète après un micro-appel récent (hors session).
    pub fn micro_context(&self) -> String {
        let Some((q, a, at)) = self.last_micro.borrow().clone() else { return String::new() };
        if js::now_ms() - at > 5 * 60 * 1000 {
            return String::new();
        }
        *self.last_micro.borrow_mut() = None;
        format!(
            "[Contexte : juste avant, hors de cette conversation, l'utilisateur a demandé « {} » et tu as répondu « {} ».]\n",
            shorten(&q, 200),
            shorten(&a, 400)
        )
    }

    pub fn execute_agy(self: &Rc<Self>, text: &str, background: bool, meta: J) {
        let id = self.next_id("c");
        let slot_model = js::gs(&meta, "slotModel");
        let chain_index = meta["chainIndex"].as_u64().unwrap_or(0) as usize;
        let resolved = self.resolve_task_model_and_effort_with(text, "agy", &slot_model);
        let label = if background { format!("[FOND] {}", shorten(text, 20)) } else { shorten(text, 26) };
        let speaker = self.speaker_of(&meta);
        let rlabel = js::gs(&resolved, "label");
        let rank_tag = if chain_index > 0 { format!(" [IA #{}]", chain_index + 1) } else { String::new() };
        self.emit(json!({ "type": "create", "id": id, "label": label, "detail": format!("{rlabel}{rank_tag} · transmission…"), "background": background }));
        self.record(
            "tache_debut",
            json!({
                "tache": id, "moteur": "agy", "demande": text, "fond": background,
                "modele": resolved["model"], "effort": resolved["effort"],
                "eco": js::gb(&resolved, "auto") && js::gs(&resolved, "complexity") == "simple",
                "locuteur": speaker, "rang": chain_index + 1
            }),
        );
        if !background {
            self.speech.borrow_mut().insert(id.clone(), self.create_speech(&id));
        }

        let prompt = format!("{}{}", self.micro_context(), match &speaker {
            Some(s) => format!("[Locuteur : {s}]\n{text}"),
            None => text.to_string(),
        });
        self.start_ai_call(&id, "agy", &rlabel, &prompt, background);

        let parallel = self.is_parallel();
        let task_meta = json!({ "speaker": speaker, "parallel": parallel, "background": background, "chainIndex": chain_index, "slotModel": slot_model });

        if !background && chain_index == 0 && self.try_daemon_with_meta(&id, text, &prompt, &resolved, "agy", task_meta.clone()) {
            return;
        }

        let mut args = self.agy_args(&resolved, false);
        let resume_agy = !parallel && !background && self.session_id_for("agy").is_some();
        if resume_agy {
            args.extend(self.session_args("agy"));
        }
        let stdin_text = if resume_agy {
            prompt
        } else {
            format!("{}\n\nDemande de l'utilisateur : {prompt}", self.agy_preamble())
        };
        self.launch(Launch {
            id,
            exe: self.agy.clone(),
            args,
            stdin_text,
            engine: "agy",
            text: text.to_string(),
            background,
            events: EventKind::Agy,
            meta: task_meta,
            task_mcp_path: None,
            launch_error: ("échec du lancement d'AGY", "AGY introuvable", Some("Je n'arrive pas à lancer AGY.")),
            deny_pending: true,
            micro: false,
        });
    }

    pub fn on_claude_event(&self, id: &str, event: &J, mut answer: String) -> String {
        // Un micro-appel (v5) vit hors de la conversation : ni sa session ni son modèle ne la remplacent.
        let meta = self.running_meta(id).unwrap_or_else(|| json!({}));
        let micro = js::gb(&meta, "micro");
        let parallel = js::gb(&meta, "parallel");
        let background = js::gb(&meta, "background");
        let sid = js::gs(event, "session_id");
        if !sid.is_empty() && !micro && !parallel && !background {
            self.set_session_id_for("claude", &sid);
        }
        let speech = self.speech.borrow().get(id).cloned();
        let typ = js::gs(event, "type");

        if typ == "system" && js::gs(event, "subtype") == "init" && js::truthy(&event["model"]) && !micro {
            self.status.d.borrow_mut().active_model = Some(js::gs(event, "model"));
            self.status.save();
            self.emit(self.status.snapshot());
        }

        if typ == "rate_limit_event" && self.status.apply_rate_limit(&event["rate_limit_info"]) {
            self.emit(self.status.snapshot());
        }

        // Texte généré mot à mot (--include-partial-messages) → synthèse phrase par phrase.
        if typ == "stream_event" && event["event"].is_object() {
            if let Some(speech) = &speech {
                let ev = &event["event"];
                let et = js::gs(ev, "type");
                if et == "content_block_delta" && js::gs(&ev["delta"], "type") == "text_delta" {
                    if !speech.saw_delta.get() {
                        self.note_first_word(id);
                    }
                    speech.saw_delta.set(true);
                    speech.push(&js::gs(&ev["delta"], "text"));
                } else if et == "content_block_stop" {
                    speech.boundary(); // fin d'un bloc de texte (souvent juste avant un appel d'outil)
                }
            }
        }

        if typ == "assistant" {
            if let Some(parts) = event["message"]["content"].as_array() {
                for part in parts {
                    match part["type"].as_str() {
                        Some("tool_use") => {
                            self.mark_used_tools(id);
                            self.on_connector_tool_use(id, part);
                            let name = js::gs(part, "name");
                            let input = if part["input"].is_object() { part["input"].clone() } else { json!({}) };
                            let detail = describe(&name, &input);
                            self.emit(json!({ "type": "update", "id": id, "detail": detail }));
                            self.record("outil", json!({ "tache": id, "moteur": "claude", "outil": name, "detail": detail }));
                        }
                        Some("text") => {
                            let t = js::gs(part, "text");
                            if !t.trim().is_empty() {
                                answer = t.clone();
                                // Repli si la version de Claude Code n'envoie pas les deltas.
                                if let Some(speech) = &speech {
                                    if !speech.saw_delta.get() {
                                        speech.push(&t);
                                        speech.boundary();
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Résultats d'outils (message « user ») : alimentent les widgets des connecteurs.
        if typ == "user" {
            if let Some(parts) = event["message"]["content"].as_array() {
                for part in parts {
                    if js::gs(part, "type") == "tool_result" && self.tool_uses.borrow().contains_key(&js::gs(part, "tool_use_id")) {
                        self.on_connector_tool_result(part);
                    }
                }
            }
        }

        if typ == "result" {
            self.record_ai_usage(id, &json!({ "usage": event["usage"] }));
            let r = js::gs(event, "result");
            if !r.is_empty() {
                answer = r;
            }
            if js::truthy(&event["is_error"]) {
                self.mark_event_error(id);
                self.emit(json!({ "type": "update", "id": id, "detail": "erreur signalée par Claude Code" }));
            }
        }

        answer
    }

    pub fn on_agy_event(&self, id: &str, event: &J, mut answer: String) -> String {
        let independent = self.running_meta(id).map(|m| js::gb(&m, "parallel") || js::gb(&m, "background")).unwrap_or(false);
        let set_session = |v: &J| {
            let s = js::s(v);
            if !s.is_empty() && !independent {
                self.set_session_id_for("agy", &s);
            }
        };
        set_session(&event["conversation_id"]);
        let speech = self.speech.borrow().get(id).cloned();
        let ev = js::gs(event, "event");

        if ev == "init" && js::truthy(&event["init"]["model"]) {
            self.status.d.borrow_mut().active_model = Some(js::gs(&event["init"], "model"));
            self.status.save();
            self.emit(self.status.snapshot());
        }

        if ev == "step_update" && event["step_update"].is_object() {
            let step = &event["step_update"];
            set_session(&step["conversation_id"]);
            match step["step_type"].as_str() {
                Some("tool") => {
                    let mut tool_name = js::gs(step, "tool_name");
                    if tool_name.is_empty() {
                        tool_name = js::gs(&step["tool_info"], "name");
                    }
                    let input = if step["tool_info"]["parameters"].is_object() { step["tool_info"]["parameters"].clone() } else { json!({}) };
                    let detail = describe(&tool_name, &input);
                    match step["state"].as_str() {
                        Some("ACTIVE") => {
                            self.mark_used_tools(id);
                            if let Some(s) = &speech {
                                s.boundary();
                            }
                            self.emit(json!({ "type": "update", "id": id, "detail": detail }));
                            self.record("outil", json!({ "tache": id, "moteur": "agy", "outil": tool_name, "detail": detail }));
                        }
                        Some("ERROR") => {
                            let msg = Some(js::gs(&step["tool_info"]["error"], "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| "erreur".into());
                            self.emit(json!({ "type": "update", "id": id, "detail": shorten(&format!("erreur: {msg}"), 50) }));
                        }
                        _ => {}
                    }
                }
                Some("agent_response") => {
                    let delta = js::gs(step, "text_delta");
                    if !delta.is_empty() {
                        answer.push_str(&delta);
                        if let Some(s) = &speech {
                            s.push(&delta);
                        }
                    }
                }
                _ => {}
            }
        }

        if ev == "result" {
            self.record_ai_usage(id, event);
            let r = &event["result"];
            if js::truthy(r) {
                set_session(&r["conversation_id"]);
                let resp = js::gs(r, "response");
                if !resp.trim().is_empty() {
                    answer = resp;
                }
                if js::gs(r, "status") == "ERROR" {
                    self.mark_event_error(id);
                    self.emit(json!({ "type": "update", "id": id, "detail": "erreur signalée par AGY" }));
                }
            }
        }

        answer
    }
}
