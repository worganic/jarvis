// Orchestrateur : commandes de réglage (applyCommand), configuration des moteurs, démon résident,
// cards Bureau / Devant / projets (orchestrator/index.js, lignes 1989-2830).

use std::rc::Rc;

use serde_json::{json, Value};

use super::cli_daemon::{DaemonConfig, DaemonTask};
use super::config::{self, Forms};
use super::policy::shorten;
use super::status::{self, HEART_STYLES};
use super::{OrchRef, Orchestrator, RunningTask, AGENT_FILES_PROMPT, CLAUDE_TOOLS, PERSONA, SESSION_IDLE_MS, SESSION_MAX_TURNS};
use crate::js::{self, J};

/// Fermeture d'une card de réglage après quelques secondes (même rythme qu'en v3).
fn finish_later(o: OrchRef, id: String, status: &'static str, ms: u64, quiet: bool) {
    if quiet {
        return;
    }
    js::set_timeout(ms, move || {
        if let Some(o) = o.upgrade() {
            o.emit(json!({ "type": "finish", "id": id, "status": status }));
        }
    });
}

fn pct(v: f64) -> i64 {
    (v * 100.0).round() as i64
}

impl Orchestrator {
    pub async fn apply_command(self: &Rc<Self>, command: J, quiet: bool) {
        let cmd_id = self.next_id("cmd");
        let emit = |ev: J| {
            if quiet && matches!(ev["type"].as_str(), Some("say" | "create" | "finish")) {
                return;
            }
            self.emit(ev);
        };
        let card = |label: String, detail: String, status: &'static str, ms: u64| {
            emit(json!({ "type": "create", "id": cmd_id, "label": label, "detail": detail }));
            finish_later(self.self_ref(), cmd_id.clone(), status, ms, quiet);
        };
        let say = |text: String| emit(json!({ "type": "say", "text": text }));
        let kind = js::gs(&command, "kind");
        let value = command["value"].clone();
        let vstr = js::s(&value);

        match kind.as_str() {
            "vibe" => {
                let mode = Some(js::gs(&command, "mode")).filter(|m| !m.is_empty()).unwrap_or_else(|| "statut".into());
                if mode == "statut" {
                    let cur = self.vibe_coding.verbosite.borrow().clone();
                    card("Vibe Coding".into(), format!("mode {cur}"), "done", 2000);
                    say(format!("Vibe coding en mode {cur}."));
                    return;
                }
                let msg = self.vibe_coding.set_verbosite(&mode);
                card("Vibe Coding".into(), format!("mode {}", self.vibe_coding.verbosite.borrow().clone()), "done", 2000);
                say(msg.to_string());
                return;
            }
            "engine" => {
                {
                    let mut d = self.status.d.borrow_mut();
                    d.engine = vstr.clone();
                    d.active_model = None;
                }
                self.reset_conversation();
                {
                    let mut d = self.status.d.borrow_mut();
                    let model = d.model.clone().unwrap_or_default();
                    if d.engine == "agy" && (model.is_empty() || status::CLAUDE_MODELS.contains(&model.as_str())) {
                        d.model = Some("gemini-3.8-flash-high".into());
                    } else if d.engine == "claude" && (model.is_empty() || status::is_agy_model_key(&model)) {
                        d.model = Some("sonnet".into());
                    }
                    d.sync_primary_to_chain();
                }
                self.status.save();
                let (engine, model) = {
                    let d = self.status.d.borrow();
                    (d.engine.clone(), d.model.clone().unwrap_or_default())
                };
                if engine == "agy" {
                    self.ensure_agy_mcp();
                }
                self.record("changement_moteur", json!({ "moteur": engine, "modele": model }));
                let engine_label = if engine == "agy" { "AGY (Antigravity)" } else { "Claude Code" };
                card(format!("Moteur {}", engine.to_uppercase()), format!("modèle {model}"), "done", 2000);
                say(format!("Moteur {engine_label}. Modèle {model}. Nouvelle session ouverte."));
            }
            "model" => {
                if vstr == "auto" {
                    {
                        let mut d = self.status.d.borrow_mut();
                        d.model = Some("auto".into());
                        d.active_model = None;
                        d.sync_primary_to_chain();
                    }
                    self.reset_conversation();
                    self.status.save();
                    let engine = self.status.d.borrow().engine.clone();
                    if engine == "agy" {
                        self.ensure_agy_mcp();
                    }
                    self.record("changement_modele", json!({ "moteur": engine, "modele": "auto" }));
                    card("Modèle dynamique (Auto)".into(), "sélection par complexité".into(), "done", 2000);
                    say("Sélection automatique du modèle activée selon la complexité.".into());
                    emit(self.status.snapshot());
                    return;
                }
                {
                    let mut d = self.status.d.borrow_mut();
                    let e = js::gs(&command, "engine");
                    if !e.is_empty() && e != d.engine {
                        d.engine = e;
                    }
                    d.model = Some(vstr.clone());
                    d.active_model = None;
                    d.sync_primary_to_chain();
                }
                self.reset_conversation(); // le modèle ne change pas au milieu d'une conversation
                self.status.save();
                let engine = self.status.d.borrow().engine.clone();
                if engine == "agy" {
                    self.ensure_agy_mcp();
                }
                self.record("changement_modele", json!({ "moteur": engine, "modele": vstr }));
                let engine_label = if engine == "agy" { "AGY" } else { "Claude Code" };
                card(format!("Modèle {vstr}"), format!("moteur {engine_label}"), "done", 2000);
                say(format!("Modèle {vstr} sur {engine_label}. Nouvelle session ouverte."));
            }
            "ai_chain" => {
                let (engine, model, count, changed_primary, has_agy) = {
                    let mut d = self.status.d.borrow_mut();
                    let prev_engine = d.engine.clone();
                    let prev_model = d.model.clone();
                    let chain = status::sanitize_ai_chain(&value, &prev_engine, prev_model.as_deref());
                    let new_engine = js::gs(&chain[0], "engine");
                    let new_model = js::gs(&chain[0], "model");
                    let changed_primary = new_engine != prev_engine || Some(new_model.clone()) != prev_model;
                    d.engine = new_engine.clone();
                    d.model = Some(new_model.clone());
                    if changed_primary {
                        d.active_model = None;
                    }
                    let count = chain.len();
                    let has_agy = chain.iter().any(|s| js::gs(s, "engine") == "agy");
                    d.ai_chain = chain;
                    (new_engine, new_model, count, changed_primary, has_agy)
                };
                if changed_primary {
                    self.reset_conversation();
                }
                self.status.save();
                if has_agy {
                    self.ensure_agy_mcp();
                }
                let chain_json = Value::Array(self.status.d.borrow().ai_chain.clone());
                self.record("changement_chaine_ia", json!({ "chaine": chain_json, "moteur": engine, "modele": model }));
                let engine_label = if engine == "agy" { "AGY (Antigravity)" } else { "Claude Code" };
                let sec = if count > 1 { format!(" (+{} secours)", count - 1) } else { String::new() };
                card(format!("Chaîne IA ({count})"), format!("défaut : {engine_label} · {model}{sec}"), "done", 2000);
                say(format!("Chaîne d'IA enregistrée : {engine_label} ({model}) par défaut{}.", if count > 1 { format!(", avec {} IA de secours", count - 1) } else { String::new() }));
            }
            "economy" => {
                let on = js::truthy(&value);
                self.status.d.borrow_mut().economy_mode = on;
                self.status.save();
                self.record("changement_economie", json!({ "actif": on }));
                card(
                    (if on { "Économie de quota activée" } else { "Économie de quota désactivée" }).into(),
                    (if on { "Haiku/Flash Low sur requêtes simples" } else { "Modèle standard" }).into(),
                    "done",
                    2000,
                );
                say((if on {
                    "Mode économie de quota activé. Les requêtes simples utiliseront un modèle léger et rapide."
                } else {
                    "Mode économie désactivé. Modèle standard réactivé."
                })
                .into());
            }
            "economy_level" => {
                self.status.d.borrow_mut().economy_level = vstr.clone();
                self.status.save();
                self.record("changement_niveau_economie", json!({ "niveau": vstr }));
                let label = match vstr.as_str() {
                    "normal" => "Normal : ce que je ne sais pas faire seul part à l’IA",
                    "local" => "Local d’abord : politesses, commandes et reformulations traitées sans IA",
                    "strict" => "Strict : je demande avant chaque appel à une IA",
                    _ => "undefined",
                };
                card("Économie de tokens".into(), label.into(), "done", 2000);
                say(match vstr.as_str() {
                    "strict" => "Économie stricte : je te demanderai avant chaque appel à une IA.".into(),
                    "normal" => "Économie normale.".into(),
                    _ => "Économie locale d'abord.".into(),
                });
            }
            "context_mute" => {
                let mode = if vstr.is_empty() { "auto".to_string() } else { vstr.clone() };
                let state = self.context_mute.set_mode(&mode).await.unwrap_or_else(|_| self.context_mute.get_state());
                self.record("changement_sourdine", state.clone());
                self.publish_status();
                let m = js::gs(&state, "mode");
                let label = match m.as_str() {
                    "auto" => "automatique (active lors des jeux, visios et enregistrements)",
                    "on" => "activée (silence complet)",
                    _ => "désactivée (voix active)",
                };
                let detail = if js::gb(&state, "isMuted") { format!("Silence ({})", js::gs(&state, "reason")) } else { "Voix active".into() };
                card(format!("Sourdine : {m}"), detail, "done", 2500);
                say(format!("Sourdine contextuelle réglée sur {label}."));
            }
            "theme" => {
                self.status.d.borrow_mut().theme = vstr.clone();
                self.status.save();
                self.record("changement_theme", json!({ "theme": vstr }));
                emit(json!({ "type": "theme_config", "theme": vstr }));
                let theme_label = match vstr.as_str() {
                    "cyan" => "Cyan Hologramme",
                    "gold" => "Stark Ambre & Or",
                    "matrix" => "Émeraude Matrix",
                    "crimson" => "Rubis Crimson",
                    "violet" => "Synthwave Améthyste",
                    other => other,
                }
                .to_string();
                card(format!("Thème : {theme_label}"), "interface holographique".into(), "done", 2000);
                say(format!("Thème visuel changé pour {theme_label}."));
            }
            "heart_style" => {
                let style = if status::heart_style_label(&vstr).is_some() { vstr.clone() } else { "classic".into() };
                let label = status::heart_style_label(&style).unwrap_or("Classique");
                self.status.d.borrow_mut().heart_style = style.clone();
                self.status.save();
                self.record("changement_apparence_coeur", json!({ "apparence": style }));
                emit(json!({ "type": "heart_style_config", "style": style }));
                // Le personnage parle avec sa propre voix (status.speech()) et se présente avec.
                let speech = self.status.speech();
                let mut vc = json!({ "type": "voice_config" });
                js::assign(&mut vc, speech.clone());
                vc["sample"] = json!(false);
                self.emit(vc);
                let sample = js::gs(&speech, "sample");
                let detail = if !sample.is_empty() { format!("voix : {}", js::gs(&speech, "name")) } else { "apparence du cœur".into() };
                card(format!("Cœur : {label}"), detail, "done", 2000);
                say(if style == "classic" {
                    "Cœur holographique rétabli.".into()
                } else if !sample.is_empty() {
                    sample
                } else {
                    format!("{} prend place au cœur du système.", crate::re!(r"\s*\(.*\)$").replace(label, ""))
                });
            }
            "character_voice" | "character_persona" => {
                let voice = kind == "character_voice";
                let on = js::truthy(&value);
                {
                    let mut d = self.status.d.borrow_mut();
                    if voice {
                        d.character_voice = on;
                    } else {
                        d.character_persona = on;
                    }
                }
                self.status.save();
                let key = if voice { "voix_personnage" } else { "caractere_personnage" };
                self.record("changement_apparence_coeur", json!({ key: on }));
                if voice {
                    let mut vc = json!({ "type": "voice_config" });
                    js::assign(&mut vc, self.status.speech());
                    vc["sample"] = json!(false);
                    self.emit(vc);
                }
                self.publish_status();
            }
            "list_heart_styles" => {
                let names: Vec<&str> = HEART_STYLES.iter().map(|h| h.1).collect();
                card("Apparences du cœur".into(), names.join(", "), "done", 3000);
                say(format!("Apparences du cœur : {}. Dites par exemple « affiche Marvin » ou « cœur classique ».", names.join(", ")));
            }
            "list_themes" => {
                card("Thèmes visuels".into(), "Cyan, Gold (Stark), Matrix, Crimson, Violet".into(), "done", 3000);
                say("Thèmes disponibles : Cyan Hologramme, Stark Ambre et Or, Émeraude Matrix, Rubis Crimson, et Synthwave Améthyste.".into());
            }
            "dance_style" => {
                let style = if status::DANCE_STYLES.contains(&vstr.as_str()) { vstr.clone() } else { "auto".into() };
                self.status.d.borrow_mut().dance_style = style.clone();
                self.status.save();
                self.record("changement_danse_personnages", json!({ "style": style }));
                emit(json!({ "type": "dance_style_config", "style": style }));
            }
            "card_numbers" => {
                let numbers = status::sanitize_card_numbers(&command["value"]);
                self.status.d.borrow_mut().card_numbers = numbers.clone();
                self.status.save();
                self.record("changement_numeros_cards", numbers.clone());
                emit(json!({ "type": "card_numbers_config", "numbers": numbers }));
                self.cards.renumber();
            }
            "card_colors" => {
                let colors = status::sanitize_card_colors(&command["value"]);
                self.status.d.borrow_mut().card_colors = colors.clone();
                self.status.save();
                self.record("changement_couleurs_cards", colors.clone());
                emit(json!({ "type": "card_colors_config", "colors": colors }));
            }
            "density" => {
                self.status.d.borrow_mut().density = vstr.clone();
                self.status.save();
                self.record("changement_densite", json!({ "densite": vstr }));
                emit(json!({ "type": "density_config", "density": vstr }));
                let label = match vstr.as_str() {
                    "compact" => "compacte (panneaux denses)",
                    "normal" => "normale (équilibrée)",
                    "large" => "large (haute lisibilité)",
                    other => other,
                }
                .to_string();
                card(format!("Densité : {vstr}"), label.clone(), "done", 2000);
                say(format!("Densité d'affichage réglée sur {label}."));
            }
            "effort" => {
                self.status.d.borrow_mut().effort = Some(vstr.clone());
                self.status.save();
                self.record("changement_effort", json!({ "effort": vstr }));
                card(format!("Effort {vstr}"), "réglage appliqué".into(), "done", 2000);
                say(format!("Effort réglé sur {vstr}."));
            }
            "voice" => {
                let (vid, vname) = (js::gs(&value, "id"), js::gs(&value, "name"));
                {
                    let mut d = self.status.d.borrow_mut();
                    d.voice = vid.clone();
                    d.voice_name = vname.clone();
                }
                self.status.save();
                self.record("changement_voix", json!({ "voix": vname, "id": vid, "pays": value["origin"] }));
                // Voix réellement utilisée : celle du personnage du cœur s'il en a une (status.speech()).
                let speech = self.status.speech();
                let mut vc = json!({ "type": "voice_config" });
                js::assign(&mut vc, speech.clone());
                vc["sample"] = json!(false);
                emit(vc);
                card(format!("Voix {vname}"), format!("{} · {}", js::gs(&value, "gender"), js::gs(&value, "origin")), "done", 2000);
                say(if js::gs(&speech, "voice") == vid {
                    format!("Je parle désormais avec la voix de {vname}.")
                } else {
                    format!("Voix de {vname} retenue : {} garde la sienne tant qu'il est au cœur du système.", js::gs(&speech, "name"))
                });
            }
            "rate" => {
                self.status.d.borrow_mut().voice_rate = vstr.clone();
                self.status.save();
                self.record("changement_vitesse", json!({ "vitesse": vstr }));
                let mut vc = json!({ "type": "voice_config" });
                js::assign(&mut vc, self.status.speech());
                vc["sample"] = json!(false);
                emit(vc);
                card(format!("Vitesse {vstr}"), "cadence vocale mise à jour".into(), "done", 2000);
                say(format!("Vitesse de parole réglée sur {vstr}."));
            }
            "list_voices" => {
                card("Voix disponibles".into(), "Denise, Henri, Éloïse, Sylvie, Antoine, Charline, Ariane".into(), "done", 3000);
                say("Voix disponibles : Denise, Henri et Éloïse pour la France ; Sylvie et Antoine pour le Canada ; Charline pour la Belgique ; et Ariane pour la Suisse.".into());
            }
            "list_zones" => {
                let list = self.zones.list();
                let summary = list
                    .iter()
                    .map(|z| format!("{} (L:{}, E:{})", js::gs(z, "name"), js::gs(z, "read"), js::gs(z, "write")))
                    .collect::<Vec<_>>()
                    .join(" · ");
                card("Zones déclarées".into(), summary, "done", 3500);
                let names = list.iter().map(|z| js::gs(z, "name")).collect::<Vec<_>>().join(", ");
                say(format!("Zones d'accès déclarées : {names}."));
            }
            "zone_add" => {
                let entry = self.zones.add(&js::gs(&command, "name"), &js::gs(&command, "path"), &js::gs(&command, "read"), &js::gs(&command, "write"));
                let (n, p, r, w) = (js::gs(&entry, "name"), js::gs(&entry, "path"), js::gs(&entry, "read"), js::gs(&entry, "write"));
                self.record("zone_ajoutee", json!({ "nom": n, "chemin": p, "lecture": r, "ecriture": w }));
                card(format!("Zone {n}"), format!("{p} (L:{r}, E:{w})"), "done", 2500);
                say(format!("Zone {n} configurée avec succès."));
            }
            "zone_remove" => {
                let name = js::gs(&command, "name");
                match self.zones.remove(&name) {
                    Some(removed) => {
                        let n = js::gs(&removed, "name");
                        self.record("zone_supprimee", json!({ "nom": n, "chemin": removed["path"] }));
                        card("Zone retirée".into(), n.clone(), "done", 2500);
                        say(format!("Zone {n} retirée."));
                    }
                    None => {
                        card("Zone introuvable".into(), name.clone(), "error", 2500);
                        say(format!("Impossible de retirer la zone {name}."));
                    }
                }
            }
            "usage" => {
                card("Consommation".into(), "lecture du quota".into(), "done", 2000);
                say(self.usage_sentence());
            }
            "restart_jarvis" => {
                // Le noyau intercepte « restart_jarvis » : relance du noyau, ou fermeture,
                // recompilation de l'interface et réouverture si l'interface a changé.
                self.record("relance_demandee", json!({ "origine": "voix" }));
                say("Je redémarre. À tout de suite.".into());
                emit(json!({ "type": "restart_jarvis" }));
                return;
            }
            "listening_mode" => {
                self.status.d.borrow_mut().listening_mode = vstr.clone();
                self.status.save();
                self.record("changement_mode_ecoute", json!({ "mode": vstr }));
                let label = match vstr.as_str() {
                    "wakeword" => "Mot d’éveil (« Jarvis » requis)",
                    "ptt" => "Appui pour parler (Push-To-Talk)",
                    "free" => "Écoute continue libre",
                    other => other,
                }
                .to_string();
                card("Mode d'écoute".into(), label, "done", 2500);
                say(match vstr.as_str() {
                    "wakeword" => "Mode mot d'éveil activé. Dis « Jarvis » au début de chaque phrase pour me solliciter.".into(),
                    "ptt" => "Mode appui pour parler activé. Maintiens la touche assignée pour me parler.".into(),
                    "free" => "Je t'écoute : plus besoin de dire « Jarvis ». Dis « Jarvis, arrête de m'écouter » pour revenir au mot d'éveil.".into(),
                    other => format!("Mode d'écoute changé en {other}."),
                });
            }
            "get_listening_mode" => {
                let mode = self.status.d.borrow().listening_mode.clone();
                let label = match mode.as_str() {
                    "wakeword" => "mot d'éveil (Jarvis)",
                    "ptt" => "appui pour parler",
                    "free" => "écoute libre",
                    other => other,
                }
                .to_string();
                card("Mode d'écoute".into(), mode.clone(), "done", 2000);
                say(format!("Le mode d'écoute actuel est : {label}."));
            }
            "authorize" => {
                let zone = js::gs(&command, "zone");
                let duration = js::gn(&command, "duration").unwrap_or(f64::NAN);
                match self.zones.grant_temporary(&zone, duration) {
                    Some(auth) => {
                        let (zn, mins) = (js::gs(&auth["zone"], "name"), js::s(&auth["durationMinutes"]));
                        self.record(
                            "autorisation_temporaire",
                            json!({ "zone": zn, "chemin": auth["zone"]["path"], "duree_minutes": auth["durationMinutes"], "expire_a": auth["expiresAt"] }),
                        );
                        card("Autorisation temporaire".into(), format!("{zn} · {mins} min sans validation"), "done", 2500);
                        say(format!("Écriture autorisée sans confirmation dans la zone {zn} pendant {mins} minutes."));
                    }
                    None => {
                        card("Zone introuvable".into(), if zone.is_empty() { "aucune zone".into() } else { zone }, "error", 2500);
                        say("Impossible de trouver la zone demandée pour l'autorisation.".into());
                    }
                }
            }
            "toggle_journal" => {
                emit(json!({ "type": "toggle_journal" }));
                self.publish_journal();
                say("Panneau du journal et des réponses basculé.".into());
            }
            "verify_journal" => {
                let integrity = self.history.verify();
                self.publish_journal();
                let ok = js::gb(&integrity, "ok");
                let count = js::s(&integrity["count"]);
                let speech = if ok {
                    format!("Journal intact : {count} entrées scellées par empreinte cryptographique.")
                } else {
                    format!("Attention : altération détectée dans le journal, {}.", js::gs(&integrity, "reason"))
                };
                card(
                    "Intégrité du journal".into(),
                    if ok { format!("{count} entrées scellées") } else { "ALTÉRATION DÉTECTÉE".into() },
                    if ok { "done" } else { "error" },
                    2500,
                );
                say(speech);
            }
            "show_last_answer" => {
                let last = self.last_answer.borrow().clone();
                match last {
                    Some(answer) => {
                        emit(json!({ "type": "show_answer", "answer": answer }));
                        card("Dernière réponse".into(), shorten(&js::gs(&answer, "text"), 40), "done", 2500);
                        say("Voici la réponse complète affichée à l'écran.".into());
                    }
                    None => say("Aucune réponse récente enregistrée.".into()),
                }
            }
            "autostart" => {
                let on = js::truthy(&value);
                self.status.d.borrow_mut().autostart = on;
                self.status.save();
                emit(json!({ "type": "autostart", "enable": on }));
                self.record("changement_autostart", json!({ "actif": on }));
                card((if on { "Démarrage auto activé" } else { "Démarrage auto désactivé" }).into(), "Windows".into(), "done", 2500);
                say((if on { "Démarrage automatique avec Windows activé." } else { "Démarrage automatique avec Windows désactivé." }).into());
            }
            "speaker_enroll" => {
                let raw = Some(js::gs(&command, "name"))
                    .filter(|n| !n.is_empty())
                    .or_else(|| self.current_speaker.borrow().clone().filter(|n| !n.is_empty()))
                    .or_else(|| self.memory.get_user_name())
                    .unwrap_or_else(|| "Johann".into());
                let target = js::capitalize(&raw);
                self.record("enrolement_locuteur_demarre", json!({ "nom": target, "echantillons": 8, "origine": "commande" }));
                emit(json!({ "type": "speaker_enroll_start", "name": target, "count": 8 }));
                card(format!("Apprentissage vocal ({target})"), "8 phrases attendues".into(), "done", 3000);
                say(format!(
                    "C'est parti pour l'enregistrement de {target}. Prononce huit phrases variées, une à une, pour que j'apprenne cette empreinte vocale."
                ));
            }
            "speaker_delete" => {
                let spk = self.get_speaker_status();
                let target = js::s(&command["target"]);
                let t = target.to_lowercase().trim().to_string();
                let found = js::arr(&spk["speakers"])
                    .into_iter()
                    .find(|s| js::gs(s, "id").to_lowercase() == t || js::gs(s, "name").to_lowercase() == t);
                match found {
                    None => {
                        card("Locuteur introuvable".into(), if target.is_empty() { "—".into() } else { target.clone() }, "error", 2500);
                        say(format!("Je ne trouve pas de profil vocal pour {}.", if target.is_empty() { "ce nom" } else { &target }));
                    }
                    Some(f) => {
                        let (fid, fname) = (js::gs(&f, "id"), js::gs(&f, "name"));
                        self.record("suppression_locuteur", json!({ "id": fid, "nom": fname, "origine": "commande" }));
                        emit(json!({ "type": "speaker_delete", "id": fid }));
                        card("Profil vocal supprimé".into(), fname.clone(), "done", 2500);
                        say(format!("Le profil vocal de {fname} a été supprimé."));
                    }
                }
            }
            "speaker_list" => {
                let spk = self.get_speaker_status();
                let speakers = js::arr(&spk["speakers"]);
                let count = js::gn(&spk, "speakersCount").filter(|n| *n != 0.0).map(|n| n as usize).unwrap_or(speakers.len());
                card("Profils vocaux".into(), format!("{count} profil(s)"), "done", 2500);
                if count == 0 {
                    say("Aucune empreinte vocale n'est enregistrée pour le moment.".into());
                } else {
                    let names = speakers
                        .iter()
                        .map(|s| format!("{}{}", js::gs(s, "name"), if js::gb(s, "isPrimary") { " (principal)" } else { "" }))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let pl = if count > 1 { "s" } else { "" };
                    say(format!("Il y a {count} profil{pl} vocal enregistré{pl} : {names}."));
                }
            }
            "speaker_mode" => {
                let enabled = js::gb(&command, "enabled");
                self.record("changement_mode_locuteur", json!({ "actif": command["enabled"] }));
                emit(json!({ "type": "speaker_set_enabled", "enabled": command["enabled"] }));
                card(
                    (if enabled { "Filtrage locuteur activé" } else { "Filtrage locuteur désactivé" }).into(),
                    (if enabled { "Seules les voix enregistrées sont écoutées" } else { "Toutes les voix autorisées" }).into(),
                    "done",
                    2500,
                );
                say((if enabled {
                    "Filtrage par empreinte vocale activé. Je ne répondrai désormais qu'aux voix reconnues."
                } else {
                    "Filtrage de locuteur désactivé. Toutes les voix sont acceptées."
                })
                .into());
            }
            "speaker_threshold" => {
                let v = js::num(&value).unwrap_or(0.0);
                self.record("changement_seuil_locuteur", json!({ "seuil": value }));
                emit(json!({ "type": "speaker_set_threshold", "threshold": value }));
                card("Sensibilité locuteur".into(), format!("Seuil {}%", pct(v)), "done", 2500);
                say(format!("Sensibilité de l'empreinte vocale réglée sur {} pour cent.", pct(v)));
            }
            "speaker_reset" => {
                self.record("reinitialisation_locuteur", json!({}));
                emit(json!({ "type": "speaker_reset" }));
                card("Empreintes vocales effacées".into(), "Réinitialisation".into(), "done", 2500);
                say("Toutes les empreintes vocales ont été réinitialisées.".into());
            }
            "speaker_status" => {
                let spk = self.get_speaker_status();
                let speakers = js::arr(&spk["speakers"]);
                let count = js::gn(&spk, "speakersCount").filter(|n| *n != 0.0).map(|n| n as usize).unwrap_or(speakers.len());
                let enrolled = js::truthy(&spk["enrolled"]);
                let th = pct(js::gn(&spk, "threshold").unwrap_or(0.0));
                card(
                    "Empreintes vocales".into(),
                    if enrolled { format!("{count} profil(s) (seuil {th}%)") } else { "Non configurée".into() },
                    "done",
                    2500,
                );
                let names = speakers.iter().map(|s| js::gs(s, "name")).collect::<Vec<_>>().join(", ");
                let pl = if count > 1 { "s" } else { "" };
                say(if enrolled {
                    format!("Filtrage biométrique actif avec {count} profil{pl} [{names}] et un seuil de {th} pour cent.")
                } else {
                    "Aucune empreinte vocale configurée. Dis « enregistre ma voix » pour commencer.".into()
                });
            }
            _ => {}
        }
        emit(self.status.snapshot());
    }

    pub fn claude_path(&self) -> Option<String> {
        (!self.claude.is_empty()).then(|| self.claude.clone())
    }

    pub fn usage_sentence(&self) -> String {
        format!("{} {}", self.today_sentence(), self.quota_sentence())
    }

    /// Consommation du jour (v5) : appels aux IA externes, tokens et réponses locales.
    pub fn today_sentence(&self) -> String {
        let t = self.usage_daily.snapshot();
        let k = |n: i64| if n >= 10000 { format!("{} mille", (n as f64 / 1000.0).round() as i64) } else { n.to_string() };
        let g = |key: &str| t[key].as_i64().unwrap_or(0);
        let approx = if js::gb(&t, "estimated") { "environ " } else { "" };
        let calls = g("calls");
        let calls = if calls > 0 {
            format!(
                "Aujourd'hui, {calls} appel{} aux IA, {approx}{} tokens entrants et {} sortants.",
                if calls > 1 { "s" } else { "" },
                k(g("in")),
                k(g("out"))
            )
        } else {
            "Aujourd'hui, aucun appel aux IA.".into()
        };
        let local = g("local") + g("ignored");
        if local > 0 {
            let pl = if local > 1 { "s" } else { "" };
            format!("{calls} {local} demande{pl} traitée{pl} sans IA.")
        } else {
            calls
        }
    }

    pub fn quota_sentence(&self) -> String {
        let usage = self.status.d.borrow().usage.clone();
        if !js::truthy(&usage) {
            return "Pas encore de relevé du quota.".into();
        }
        let now = js::now_ms();
        let reset = match js::gn(&usage["five"], "resetsAt").filter(|r| *r != 0.0) {
            None => String::new(),
            Some(r) => {
                let minutes = ((r * 1000.0 - now as f64) / 60000.0).round().max(0.0) as i64;
                if minutes >= 60 {
                    format!(" Remise à zéro dans {} heures et {} minutes.", minutes / 60, minutes % 60)
                } else {
                    format!(" Remise à zéro dans {minutes} minutes.")
                }
            }
        };
        let age = ((now as f64 - js::gn(&usage, "at").unwrap_or(0.0)) / 60000.0).round() as i64;
        let freshness = if age >= 5 { format!(" Relevé d'il y a {age} minutes.") } else { String::new() };
        format!(
            "Quota cinq heures : {} pour cent utilisés.{reset} Sur sept jours : {} pour cent.{freshness}",
            js::s(&usage["five"]["pct"]),
            js::s(&usage["week"]["pct"])
        )
    }

    /// Premier créneau IA disponible dans la chaîne configurée (en sautant AGY si son quota est bloqué).
    pub fn first_available_ai_slot(&self) -> (String, String, usize) {
        let d = self.status.d.borrow();
        for (idx, slot) in d.ai_chain.iter().enumerate() {
            let eng = if js::gs(slot, "engine") == "agy" { "agy" } else { "claude" };
            let mdl = {
                let m = js::gs(slot, "model");
                if m.is_empty() { "auto".into() } else { m }
            };
            if eng == "agy" && (self.agy_blocked() || self.agy.is_empty()) {
                continue;
            }
            if eng == "claude" && self.claude.is_empty() {
                continue;
            }
            return (eng.into(), mdl, idx);
        }
        let eng = if d.engine == "agy" && !self.agy_blocked() { "agy" } else { "claude" };
        let mdl = d.model.clone().unwrap_or_else(|| "auto".into());
        (eng.into(), mdl, 0)
    }

    /// Créneau IA suivant dans la chaîne après un échec au rang `after_idx`.
    pub fn next_available_ai_slot(&self, after_idx: usize, launch_failed_engine: Option<&str>) -> Option<(String, String, usize)> {
        let d = self.status.d.borrow();
        for (idx, slot) in d.ai_chain.iter().enumerate().skip(after_idx + 1) {
            let eng = if js::gs(slot, "engine") == "agy" { "agy" } else { "claude" };
            let mdl = {
                let m = js::gs(slot, "model");
                if m.is_empty() { "auto".into() } else { m }
            };
            if launch_failed_engine == Some(eng) {
                continue;
            }
            if eng == "agy" && (self.agy_blocked() || self.agy.is_empty()) {
                continue;
            }
            if eng == "claude" && self.claude.is_empty() {
                continue;
            }
            return Some((eng.into(), mdl, idx));
        }
        // Repli historique si une seule IA (AGY) est configurée et que son quota vient d'expirer.
        if d.ai_chain.len() <= 1 && after_idx == 0 && d.engine == "agy" && self.agy_blocked() && !self.claude.is_empty() && launch_failed_engine != Some("claude") {
            return Some(("claude".into(), "auto".into(), 1));
        }
        None
    }

    pub fn execute_slot(self: &Rc<Self>, text: &str, background: bool, engine: &str, meta: J) {
        if engine == "agy" {
            self.execute_agy(text, background, meta);
        } else {
            self.execute_claude(text, background, meta);
        }
    }

    pub fn execute(self: &Rc<Self>, text: &str, background: bool, meta: J) {
        // Micro-appel (v5) : question générale sans action ni donnée personnelle → Claude Code sans
        // prompt système complet, sans outil ni reprise de conversation (~500 tokens au lieu de ~14 000).
        let (engine, slot_model, slot_idx) = self.first_available_ai_slot();
        let level = self.status.d.borrow().economy_level.clone();
        if !background && engine != "agy" && level != "normal" && super::micro_call::is_micro_candidate(text, self.session_last_at.get()) {
            self.execute_micro(text, meta);
            return;
        }
        self.renew_session_if_stale();
        self.session_last_at.set(js::now_ms());
        self.session_turns.set(self.session_turns.get() + 1);
        if slot_idx > 0 && self.status.d.borrow().engine == "agy" && self.agy_blocked() {
            self.announce_agy_fallback(false);
        }
        let mut m = if meta.is_object() { meta } else { json!({}) };
        m["chainIndex"] = json!(slot_idx);
        m["slotModel"] = json!(slot_model);
        self.execute_slot(text, background, &engine, m);
    }

    // --- Configuration des moteurs (partagée entre démon résident et lancement à la demande) ---

    /// Consigne complète : rôle et règles fixes, puis façon de parler tirée du profil de dialogue.
    pub fn persona(&self) -> String {
        let profile = self.config.get("profile");
        let connectors = self.mcp_registry.prompt_line();
        // Caractère du personnage du cœur (Marvin las, chat bougon…) : le ton seulement.
        let character = self.status.character_line();
        let name = self.memory.get_user_name();
        let mut s = format!("{PERSONA} {}", config::dialogue_instructions(&profile, name.as_deref()));
        if !character.is_empty() {
            s.push(' ');
            s.push_str(&character);
        }
        if !connectors.is_empty() {
            s.push(' ');
            s.push_str(&connectors);
        }
        s
    }

    /// Formules écrites en dur (accueil, questions d'onboarding) accordées au tutoiement/vouvoiement.
    pub fn addressing_forms(&self) -> Forms {
        let profile = self.config.get("profile");
        let name = config::address_name(&profile, self.memory.get_user_name().as_deref());
        let mut f = config::forms(&profile);
        f.honorific = js::gs(&profile, "honorific");
        f.call = if !name.is_empty() { name.clone() } else { f.honorific.clone() };
        f.name = name;
        f
    }

    pub fn addressing(&self) -> J {
        let f = self.addressing_forms();
        json!({
            "vous": f.vous, "toi": f.toi, "te": f.te, "ton": f.ton, "tu": f.tu, "peux": f.peux, "veux": f.veux,
            "souhaites": f.souhaites, "dis": f.dis, "saisis": f.saisis, "tape": f.tape,
            "name": f.name, "honorific": f.honorific, "call": f.call
        })
    }

    /// Bouton « son » du HUD : coupe la voix de Jarvis (sourdine complète : il écrit sans parler,
    /// la phrase en cours s'interrompt), ou la rétablit dans le mode d'avant (automatique : muet en
    /// visio et en jeu). Some(true/false) pour forcer, None pour basculer.
    pub async fn toggle_silence(&self, on: Option<bool>) -> J {
        let st = self.context_mute.get_state();
        let cur = js::gs(&st, "mode");
        let want = on.unwrap_or(cur != "on");
        if want && cur != "on" {
            *self.silence_restore_mode.borrow_mut() = Some(cur.clone());
        }
        let restore = self.silence_restore_mode.borrow().clone().filter(|m| !m.is_empty() && m != "on").unwrap_or_else(|| "auto".into());
        let state = self.context_mute.set_mode(if want { "on" } else { &restore }).await.unwrap_or_else(|_| self.context_mute.get_state());
        if want {
            self.emit(json!({ "type": "voice_stop" }));
        }
        let mut rec = state.clone();
        rec["origine"] = json!("bouton");
        self.record("changement_sourdine", rec);
        self.publish_status();
        let mode = js::gs(&state, "mode");
        json!({ "ok": true, "muted": mode == "on" || js::gb(&state, "isMuted"), "mode": mode, "reason": js::gs(&state, "reason") })
    }

    /// Card « Bureau » affichée par défaut au démarrage (applications ouvertes), compacte et durable.
    pub fn show_default_desktop_card(&self, attempt: u32) {
        if js::env("JARVIS_TEST").is_some() {
            return;
        }
        let ds = &self.desktop_state;
        if !ds.ready() {
            if attempt < 30 {
                let w = self.self_ref();
                js::set_timeout(1000, move || {
                    if let Some(o) = w.upgrade() {
                        o.show_default_desktop_card(attempt + 1);
                    }
                });
            }
            return;
        }
        let card = super::desktop_intents::desktop_card(self, None, true);
        self.show_widget("tool", card.clone());
        *self.last_desktop_card.borrow_mut() = Some(json!({ "kind": "bureau", "monitor": null, "title": card["title"], "compact": true }));
        *self.desktop_card_sig.borrow_mut() = card["rows"].to_string();
    }

    /// Card « Bureau » à renvoyer à une fenêtre qui vient de se connecter. Elle n'est émise qu'une fois, au
    /// démarrage : sans ce rejeu, une fenêtre qui se connecte après (chargement lent, Bureau JARVIS, noyau
    /// relancé seul) ne l'afficherait jamais. Absente si l'utilisateur l'a fermée.
    pub fn desktop_card_replay(&self) -> Option<J> {
        let last = self.last_desktop_card.borrow().clone()?;
        if js::gs(&last, "kind") != "bureau" {
            return None;
        }
        let key = format!("tool:{}", js::gs(&last, "title"));
        let (kind, data) = {
            let cards = self.cards.by_key.borrow();
            let c = cards.iter().find(|c| c.key == key)?;
            (c.kind.clone(), c.data.clone())
        };
        let mut ev = json!({ "type": "widget", "kind": kind });
        js::assign(&mut ev, data);
        ev["at"] = json!(js::now_ms());
        Some(ev)
    }

    /// Card « Bureau » / « Écran n » / « Chrome » encore affichée : remise à jour quand le bureau
    /// change, au plus une fois par seconde et seulement si son contenu a changé.
    pub fn refresh_desktop_card(&self) {
        if self.last_desktop_card.borrow().is_none() || self.desktop_refresh_timer.active() {
            return;
        }
        let w = self.self_ref();
        self.desktop_refresh_timer.set(1000, move || {
            let Some(o) = w.upgrade() else { return };
            let Some(last) = o.last_desktop_card.borrow().clone() else { return };
            let title = js::gs(&last, "title");
            let key = format!("tool:{}", if title.is_empty() { js::gs(&last, "browser") } else { title });
            if !o.cards.has_key(&key) {
                *o.last_desktop_card.borrow_mut() = None;
                return;
            }
            let card = match js::gs(&last, "kind").as_str() {
                "devant" => o.devant.card(Some(o.devant.showing_excluded.get())),
                "navigateur" => super::desktop_intents::browser_card(&o, &js::gs(&last, "browser")),
                _ => super::desktop_intents::desktop_card(&o, last["monitor"].as_i64(), js::gb(&last, "compact")),
            };
            if card.is_null() {
                return;
            }
            let sig = if !card["items"].is_null() {
                json!({ "items": card["items"], "excluded": card["excluded"], "show": card["showExcluded"] }).to_string()
            } else {
                card["rows"].to_string()
            };
            if *o.desktop_card_sig.borrow() == sig {
                return;
            }
            *o.desktop_card_sig.borrow_mut() = sig;
            o.show_widget("tool", card);
        });
    }

    pub fn show_devant_card(&self) -> J {
        self.show_devant_card_with(None)
    }

    pub fn show_devant_card_with(&self, show_excluded: Option<bool>) -> J {
        let card = self.devant.card(show_excluded);
        self.show_widget("tool", card.clone());
        *self.last_desktop_card.borrow_mut() = Some(json!({ "kind": "devant", "title": "Devant" }));
        *self.desktop_card_sig.borrow_mut() = json!({ "items": card["items"], "excluded": card["excluded"], "show": card["showExcluded"] }).to_string();
        card
    }

    pub fn devant_action(&self, action: &str, app_key: &str) -> J {
        let r = self.devant.handle_action(action, app_key);
        self.show_devant_card_with(Some(self.devant.showing_excluded.get()));
        r
    }

    /// Bouton d'une card « Éléments de <appli> » : cliquer sur l'élément n° index (nom en secours).
    pub async fn ui_click_action(self: &Rc<Self>, hwnd: i64, index: i64, name: &str) -> J {
        let Some(w) = self.desktop_state.by_hwnd(hwnd) else {
            return json!({ "ok": false, "say": "Cette fenêtre n’existe plus." });
        };
        let cible = if name.is_empty() { format!("élément {}", index + 1) } else { name.to_string() };
        let params = json!({ "action": "cliquer", "application": w["app"], "cible": cible, "hwnd": w["hwnd"] });
        let o = self.clone();
        let name = name.to_string();
        let r = self
            .local_tool(
                "mcp__system__ui_agir",
                params,
                Box::new(move |_| {
                    Box::pin(async move {
                        if name.is_empty() {
                            o.ui_control.click_index(&w, index).await
                        } else {
                            o.ui_control.click(&w, &name).await
                        }
                    })
                }),
            )
            .await;
        if js::gb(&r, "denied") {
            let err = js::gs(&r, "error");
            return json!({ "ok": false, "say": if err.contains("interdit") { err } else { "Clic refusé.".into() } });
        }
        if js::gb(&r, "ok") {
            json!({ "ok": true, "say": null })
        } else {
            json!({ "ok": false, "say": format!("Je n'ai pas pu cliquer : {}.", Some(js::gs(&r, "error")).filter(|e| !e.is_empty()).unwrap_or_else(|| "échec".into())) })
        }
    }

    /// Bouton d'une ligne d'onglet (card Chrome) : aller sur l'onglet, le recharger, le fermer.
    pub async fn browser_tab_action(self: &Rc<Self>, tab_id: i64, action: &str) -> J {
        let Some(tab) = self.browser_bridge.tab_by_id(tab_id) else {
            return json!({ "ok": false, "say": "Cet onglet n’existe plus." });
        };
        let cmd = match action {
            "activer" => "activate",
            "recharger" => "reload",
            "fermer" => "close",
            _ => return json!({ "ok": false }),
        };
        let args = if action == "fermer" { json!({ "tabIds": [tab["id"]] }) } else { json!({ "tabId": tab["id"] }) };
        let bridge = self.browser_bridge.clone();
        let r = self
            .local_tool(
                "mcp__system__onglets",
                json!({ "action": action, "page": tab["title"] }),
                Box::new(move |_| Box::pin(async move { bridge.request(cmd, args, 8000).await })),
            )
            .await;
        if js::gb(&r, "denied") {
            return json!({ "ok": false, "say": "Action refusée." });
        }
        if js::gb(&r, "ok") {
            json!({ "ok": true, "say": null })
        } else {
            json!({ "ok": false, "say": format!("Je n'ai pas pu : {}.", Some(js::gs(&r, "error")).filter(|e| !e.is_empty()).unwrap_or_else(|| "échec".into())) })
        }
    }

    /// Bouton d'une card Bureau / Chrome : action sur une fenêtre, par son identifiant.
    pub fn desktop_action(self: &Rc<Self>, hwnd: i64, action: &str) -> super::Fut<J> {
        let o = self.clone();
        let action = action.to_string();
        Box::pin(async move {
            let Some(w) = o.desktop_state.by_hwnd(hwnd) else {
                return json!({ "ok": false, "say": "Cette fenêtre n’existe plus." });
            };
            // « basculer » (card Terminal) : premier clic = au premier plan, clic suivant = réduit. La fenêtre est
            // « devant » si elle a été montrée par le clic précédent, n'est pas réduite, et que personne d'autre que
            // Jarvis (le clic sur sa card lui donne le focus) ou elle-même n'a pris le premier plan depuis.
            let action = if action == "basculer" {
                let front = jarvis_native::input::foreground_hwnd();
                let owner = jarvis_native::input::owner_name(front);
                let ours = front == hwnd || front == 0 || owner.starts_with("jarvis") || owner.starts_with("msedgewebview2");
                let on_top = o.toggle_shown.get() == hwnd && !js::gb(&w, "min") && ours;
                o.toggle_shown.set(if on_top { 0 } else { hwnd });
                if on_top { "reduire".to_string() } else { "afficher".to_string() }
            } else {
                action
            };
            let name = if action == "fermer" { "mcp__system__fenetre_fermer" } else { "mcp__system__fenetre" };
            let ecran = if action == "centrer" && js::truthy(&w["monitor"]) { json!(js::s(&w["monitor"])) } else { Value::Null };
            let p = json!({ "action": action, "hwnd": w["hwnd"], "ecran": ecran });
            let ds = o.desktop_state.clone();
            let r = o
                .local_tool(name, json!({ "action": action, "cible": w["app"] }), Box::new(move |_| Box::pin(async move { ds.tool_action(&p).await })))
                .await;
            if js::gb(&r, "denied") {
                return json!({ "ok": false, "say": "Action refusée." });
            }
            if !js::gb(&r, "ok") {
                return json!({ "ok": false, "say": format!("Je n'ai pas pu : {}.", Some(js::gs(&r, "error")).filter(|e| !e.is_empty()).unwrap_or_else(|| "échec".into())) });
            }
            let last_kind = o.last_desktop_card.borrow().as_ref().map(|l| js::gs(l, "kind"));
            let wk = o.self_ref();
            match last_kind.as_deref() {
                Some("devant") => js::set_timeout(100, move || {
                    if let Some(o) = wk.upgrade() {
                        o.desktop_card_sig.borrow_mut().clear();
                        o.show_devant_card_with(Some(o.devant.showing_excluded.get()));
                    }
                }),
                Some(_) => js::set_timeout(100, move || {
                    if let Some(o) = wk.upgrade() {
                        o.refresh_desktop_card();
                    }
                }),
                None => {}
            }
            json!({ "ok": true, "say": null })
        })
    }

    /// Emplacements de la card Bureau. « slot:n » : envoyer la fenêtre à sa place n (l'enregistrer si elle
    /// n'en a pas) ; « layout:n » : y envoyer toutes les fenêtres de la disposition n (l'enregistrer si elle
    /// est vide) ; « …_save:n » réenregistre, « …_clear:n » efface.
    pub async fn window_slot_action(self: &Rc<Self>, hwnd: i64, action: &str) -> J {
        let Some((verb, n)) = action.split_once(':') else { return json!({ "ok": false }) };
        let n: usize = n.parse().unwrap_or(0);
        if !super::window_slots::WindowSlots::valid(n) {
            return json!({ "ok": false });
        }
        let ds = self.desktop_state.clone();
        if !ds.ready() {
            return json!({ "ok": false, "say": "La veille du bureau est indisponible." });
        }
        let live = ds.windows();
        let slots = &self.window_slots;
        let place = |e: &J, w: &J, focus: bool| {
            let ds = ds.clone();
            let args = json!({ "hwnd": w["hwnd"], "x": e["x"], "y": e["y"], "w": e["w"], "h": e["h"], "max": js::gb(e, "max"), "activate": focus });
            async move { ds.request("place", args, 5000).await }
        };
        let r = match verb {
            "slot" | "slot_save" | "slot_clear" => {
                let Some(w) = live.iter().find(|w| w["hwnd"].as_i64() == Some(hwnd)).cloned() else {
                    return json!({ "ok": false, "say": "Cette fenêtre n’existe plus." });
                };
                let app = js::gs(&w, "app");
                match (verb, slots.target(n, &live, hwnd)) {
                    ("slot", Some(e)) => {
                        let r = place(&e, &w, true).await;
                        if js::gb(&r, "ok") { json!({ "ok": true, "say": null }) } else { json!({ "ok": false, "say": format!("Je n'ai pas pu déplacer {app}.") }) }
                    }
                    ("slot_clear", _) => {
                        let had = slots.clear_window(n, hwnd, &live);
                        json!({ "ok": true, "say": if had { format!("Emplacement {n} effacé pour {app}.") } else { format!("{app} n’a pas d’emplacement {n}.") } })
                    }
                    _ => {
                        slots.save_window(n, &w, &live);
                        self.record("emplacement_enregistre", json!({ "emplacement": n, "fenetre": app }));
                        json!({ "ok": true, "say": format!("Emplacement {n} enregistré pour {app}.") })
                    }
                }
            }
            _ => {
                let empty = slots.counts()[n - 1] == 0;
                if verb == "layout_clear" {
                    slots.clear_layout(n);
                    json!({ "ok": true, "say": format!("Disposition {n} effacée.") })
                } else if verb == "layout_save" || empty {
                    let count = slots.save_layout(n, &live);
                    self.record("disposition_enregistree", json!({ "disposition": n, "fenetres": count }));
                    let s = if count > 1 { "s" } else { "" };
                    json!({ "ok": true, "say": format!("Disposition {n} enregistrée : {count} fenêtre{s}.") })
                } else {
                    let targets = slots.targets(n, &live);
                    if targets.is_empty() {
                        json!({ "ok": false, "say": format!("Aucune fenêtre de la disposition {n} n’est ouverte.") })
                    } else {
                        // De la plus en arrière à la plus en avant : chacune passe devant la précédente.
                        let mut failed = 0;
                        for (e, w) in &targets {
                            if !js::gb(&place(e, w, true).await, "ok") {
                                failed += 1;
                            }
                        }
                        if failed > 0 { json!({ "ok": false, "say": format!("{failed} fenêtre(s) n’ont pas pu être déplacées.") }) } else { json!({ "ok": true, "say": null }) }
                    }
                }
            }
        };
        // Les pastilles 1-4 de la card changent : la redessiner même si les fenêtres n'ont pas bougé.
        self.desktop_card_sig.borrow_mut().clear();
        let wk = self.self_ref();
        js::set_timeout(150, move || {
            if let Some(o) = wk.upgrade() {
                o.refresh_desktop_card();
            }
        });
        r
    }

    /// Card numérotée des projets (même card que « liste mes projets »), numéros retenus.
    pub async fn show_projects_card(&self, scope: &str) -> Vec<J> {
        let list = self.projects.ordered(scope).await;
        self.projects.remember_numbers(&list);
        let hidden = if scope == "liste" {
            self.projects.list().iter().filter(|p| !js::truthy(&p["parent"]) && self.projects.is_hidden(p)).count()
        } else {
            0
        };
        let title = if scope == "tous" { "Tous mes projets" } else { "Mes projets" };
        self.show_widget("tool", super::skill_intents::project_card(title, &list, hidden, ""));
        list
    }

    /// Souvenir épinglé « projets » : la liste des projets, favoris et mis de côté, réécrite quand
    /// elle change ; le RAG la retrouve comme les autres.
    pub fn sync_projects_memory(&self) {
        let w = self.self_ref();
        self.projects_memory_timer.set(300, move || {
            let Some(o) = w.upgrade() else { return };
            let before = o.memory.list().into_iter().find(|e| js::gs(e, "pinned") == "projets").map(|e| js::gs(&e, "text")).unwrap_or_default();
            let entry = o.memory.upsert_pinned("projets", &o.projects.memory_text(), "projets");
            if entry.map(|e| js::gs(&e, "text")).unwrap_or_default() == before {
                return;
            }
            o.publish_memory();
            o.refresh_daemon();
            js::spawn(async move {
                let _ = o.memory.index().await;
            });
        });
    }

    pub fn sync_profile_memory(&self) {
        self.memory.upsert_pinned("profile", &config::profile_summary(&self.config.get("profile")), "dialogue");
    }

    /// Consignes des agents pour créer, écrire et modifier des fichiers (AGENT_FILES_PROMPT).
    pub fn agent_files_prompt(&self) -> String {
        AGENT_FILES_PROMPT.replace("{workspace}", &self.workspace.display().to_string())
    }

    pub fn system_prompt(&self) -> String {
        // Une ligne sur ses projets (noms seulement : le détail passe par l'outil projets).
        let projects = self.projects.prompt_line();
        format!(
            "{}{}\n\n{}\n\nCe que tu sais de lui :\n{}",
            self.persona(),
            if projects.is_empty() { String::new() } else { format!("\n\n{projects}") },
            self.agent_files_prompt(),
            self.memory.prompt_block()
        )
    }

    pub fn agy_preamble(&self) -> String {
        let mut lines = vec!["[Consignes système Jarvis :".to_string(), self.persona(), format!("Dossier de travail principal : {}", self.workspace.display())];
        let pl = self.projects.prompt_line();
        if !pl.is_empty() {
            lines.push(pl);
        }
        lines.push("Avant toute action d'écriture ou modification de fichier ou exécution de commande système, appelle impérativement l'outil approve du serveur jarvis pour obtenir l'accord de l'utilisateur. Si approve refuse, n'effectue pas l'action.".into());
        lines.push(self.agent_files_prompt());
        lines.push("Ce que tu sais de lui :".into());
        lines.push(self.memory.prompt_block());
        lines.push("]".into());
        lines.join("\n")
    }

    pub fn claude_args(&self, resolved: &J, persistent: bool, mcp_config: &std::path::Path) -> Vec<String> {
        let mut args: Vec<String> = vec!["-p".into(), "--verbose".into(), "--output-format".into(), "stream-json".into()];
        // Texte envoyé mot à mot : Jarvis commence à parler avant la fin de la réponse.
        args.push("--include-partial-messages".into());
        if persistent {
            args.extend(["--input-format".into(), "stream-json".into()]);
        }
        // Mode manuel imposé : les réglages personnels de l'utilisateur peuvent être en "auto",
        // ce qui court-circuiterait toute validation.
        args.extend(["--permission-mode", "manual", "--permission-prompts", "host", "--permission-prompt-tool", "mcp__jarvis__approve"].map(String::from));
        args.extend(["--mcp-config".into(), mcp_config.display().to_string(), "--strict-mcp-config".into()]);
        // Aucun outil n'est pré-autorisé côté Claude Code : tout passe par la politique de Jarvis.
        args.extend(["--append-system-prompt".into(), self.system_prompt()]);
        // v5 : outils intégrés limités à ceux dont Jarvis se sert.
        // Une tâche peut restreindre ses outils (recherche « Savoir » : web et fichiers du dossier de travail seulement).
        let tools = Some(js::gs(resolved, "tools")).filter(|t| !t.is_empty()).unwrap_or_else(|| CLAUDE_TOOLS.into());
        args.extend(["--tools".into(), tools]);
        let model = js::gs(resolved, "model");
        if !model.is_empty() {
            args.extend(["--model".into(), model]);
        }
        let effort = js::gs(resolved, "effort");
        if !effort.is_empty() {
            args.extend(["--effort".into(), effort]);
        }
        // Déclaration des zones d'accès autorisées.
        for zone in self.zones.list() {
            let p = js::gs(&zone, "path");
            if js::gs(&zone, "id") != "workspace" && std::path::Path::new(&p).exists() {
                args.extend(["--add-dir".into(), p]);
            }
        }
        args
    }

    pub fn agy_args(&self, resolved: &J, persistent: bool) -> Vec<String> {
        let mut args: Vec<String> = vec!["--output-format".into(), "stream-json".into()];
        if persistent {
            args.extend(["--input-format".into(), "stream-json".into()]);
        }
        // Réglage « agy.mode » : bypass (défaut : AGY n'a pas d'outil d'approbation, il agit sans demander),
        // accept-edits (modifications de fichiers acceptées, le reste refusé) ou plan (lecture seule).
        match js::gs(&self.config.get("agy"), "mode").as_str() {
            m @ ("accept-edits" | "plan") => args.extend(["--mode".into(), m.into()]),
            _ => args.push("--dangerously-skip-permissions".into()),
        }
        args.extend(["--add-dir".into(), self.workspace.display().to_string()]);
        let model = js::gs(resolved, "model");
        let model_effort = if model.ends_with("-low") {
            Some("low")
        } else if model.ends_with("-medium") {
            Some("medium")
        } else if model.ends_with("-high") {
            Some("high")
        } else {
            None
        };
        if !model.is_empty() {
            args.extend(["--model".into(), model]);
        }
        let effort = js::gs(resolved, "effort");
        if let Some(me) = model_effort {
            if !effort.is_empty() {
                args.extend(["--effort".into(), me.into()]);
            }
        } else if !effort.is_empty() {
            let e = if ["low", "medium", "high"].contains(&effort.as_str()) { effort } else { "high".into() };
            args.extend(["--effort".into(), e]);
        }
        args
    }

    /// Identifiant de conversation courant pour `engine` uniquement (jamais partagé entre Claude et AGY).
    pub fn session_id_for(&self, engine: &str) -> Option<String> {
        let raw = self.session_id.borrow().clone()?;
        if let Some((eng, id)) = raw.split_once(':') {
            (eng == engine && !id.is_empty()).then(|| id.to_string())
        } else {
            (self.status.d.borrow().engine == engine).then_some(raw)
        }
    }

    pub fn set_session_id_for(&self, engine: &str, id: &str) {
        if !id.is_empty() {
            *self.session_id.borrow_mut() = Some(format!("{engine}:{id}"));
        }
    }

    pub fn session_args(&self, engine: &str) -> Vec<String> {
        match self.session_id_for(engine) {
            None => vec![],
            Some(id) if engine == "agy" => vec!["--conversation".into(), id],
            Some(id) => vec!["--resume".into(), id],
        }
    }

    /// Le démon sert la « voie rapide » : le modèle choisi pour les demandes simples (ou le modèle
    /// imposé). Les demandes complexes en mode auto passent par un lancement dédié.
    pub fn daemon_config(&self) -> Option<DaemonConfig> {
        // Quota AGY épuisé : le démon résident prépare le prochain créneau disponible de la chaîne IA.
        let (eng_owned, slot_model, _) = self.first_available_ai_slot();
        let engine = eng_owned.as_str();
        let lane = self.resolve_task_model_and_effort_with("", engine, &slot_model);
        let args = if engine == "agy" { self.agy_args(&lane, true) } else { self.claude_args(&lane, true, &self.mcp_config) };
        let exe = if engine == "agy" { &self.agy } else { &self.claude };
        if exe.is_empty() {
            return None;
        }
        let preamble = if engine == "agy" && self.session_id_for("agy").is_none() { self.agy_preamble() } else { String::new() };
        // La session n'entre pas dans la signature : seule une conversation qui a avancé ailleurs
        // (session_epoch) ou une configuration différente impose une relance.
        let signature = json!({ "engine": engine, "args": args, "epoch": self.session_epoch.get(), "mcp": self.mcp_registry.version() }).to_string();
        Some(DaemonConfig {
            engine: engine.into(),
            exe: exe.into(),
            args,
            session_args: self.session_args(engine),
            signature,
            preamble,
        })
    }

    pub fn refresh_daemon(&self) {
        if !self.started.get() || self.stopped.get() {
            return;
        }
        if let Some(cfg) = self.daemon_config() {
            self.cli_daemon.ensure(cfg);
        }
    }

    pub fn reset_conversation(&self) {
        *self.session_id.borrow_mut() = None;
        self.session_epoch.set(self.session_epoch.get() + 1);
        self.session_turns.set(0);
    }

    /// v5 : nouvelle conversation après une longue pause ou trop de tours (l'historique renvoyé à
    /// chaque tour est la première source de tokens entrants d'une longue session).
    pub fn renew_session_if_stale(&self) {
        if self.session_id.borrow().is_none() || !self.running.borrow().is_empty() {
            return;
        }
        let last = self.session_last_at.get();
        let idle = last > 0 && js::now_ms() - last > SESSION_IDLE_MS;
        let turns = self.session_turns.get();
        let long = turns >= SESSION_MAX_TURNS;
        if !idle && !long {
            return;
        }
        self.record("session_renouvelee", json!({ "motif": if idle { "inactivite" } else { "longueur" }, "tours": turns }));
        self.log(format!("conversation renouvelée ({})", if idle { "10 min sans échange".to_string() } else { format!("{turns} tours") }));
        self.reset_conversation();
    }

    /// Quota d'un moteur épuisé (message d'AGY : « Individual quota reached … Resets in 25h36m4s »).
    /// Renvoie true si le message signale un quota ; AGY est alors évité jusqu'à sa remise à zéro.
    pub fn note_engine_error(&self, engine: &str, text: &str) -> bool {
        if engine != "agy" || !crate::re!(r"(?i)quota reached|RESOURCE_EXHAUSTED|\b429\b").is_match(text) {
            return false;
        }
        let ms = match crate::re!(r"(?i)Resets in\s+(?:(\d+)h)?\s*(?:(\d+)m)?\s*(?:(\d+)s)?").captures(text) {
            Some(m) => {
                let n = |i: usize| m.get(i).and_then(|x| x.as_str().parse::<i64>().ok()).unwrap_or(0);
                ((n(1) * 60 + n(2)) * 60 + n(3)) * 1000
            }
            None => 60 * 60 * 1000,
        };
        let now = js::now_ms();
        let until = now + ms.max(60 * 1000);
        let cur = self.agy_blocked_until.get();
        if cur == 0 || (cur - until).abs() > 60000 {
            self.agy_blocked_until.set(until);
            self.agy_fallback_announced.set(false);
            self.record("quota_moteur", json!({ "moteur": "agy", "jusqua": js::iso_of(until) }));
            let w = self.self_ref();
            js::set_timeout(0, move || {
                if let Some(o) = w.upgrade() {
                    o.refresh_daemon(); // le démon passe sur Claude dès qu'il est libre
                }
            });
            // Quota rétabli : le démon repasse sur AGY de lui-même.
            let w = self.self_ref();
            self.agy_unblock_timer.set((until - now + 1000) as u64, move || {
                if let Some(o) = w.upgrade() {
                    o.refresh_daemon();
                }
            });
            let local = chrono::DateTime::from_timestamp_millis(until).map(|d| d.with_timezone(&chrono::Local).format("%d/%m/%Y %H:%M:%S").to_string()).unwrap_or_default();
            self.log(format!("AGY : quota épuisé jusqu'à {local} → les demandes passent par Claude"));
        }
        true
    }

    pub fn agy_blocked(&self) -> bool {
        let u = self.agy_blocked_until.get();
        u != 0 && js::now_ms() < u
    }

    pub fn announce_agy_fallback(&self, force: bool) {
        if self.agy_fallback_announced.get() && !force {
            return;
        }
        self.agy_fallback_announced.set(true);
        use chrono::{Local, TimeZone, Timelike};
        let u = self.agy_blocked_until.get();
        let when = Local.timestamp_millis_opt(if u == 0 { js::now_ms() } else { u }).single().unwrap_or_else(Local::now);
        let same_day = when.date_naive() == Local::now().date_naive();
        let hour = format!("{} heures {:02}", when.hour(), when.minute());
        self.emit(json!({
            "type": "say",
            "text": format!("Le quota d'AGY est épuisé jusqu'à {}{hour}. Je passe par Claude en attendant.", if same_day { "" } else { "demain " })
        }));
    }

    pub fn is_parallel(&self) -> bool {
        self.running.borrow().iter().any(|(_, e)| !js::gb(&e.meta, "micro"))
    }

    /// Envoie la demande au processus résident si sa configuration correspond exactement.
    pub fn try_daemon(self: &Rc<Self>, id: &str, text: &str, resolved: &J, engine: &str) -> bool {
        self.try_daemon_with_meta(id, text, text, resolved, engine, json!({}))
    }

    pub fn try_daemon_with_meta(self: &Rc<Self>, id: &str, raw_text: &str, prompt: &str, resolved: &J, engine: &str, meta: J) -> bool {
        if !self.started.get() || self.stopped.get() {
            return false;
        }
        let (_, slot_model, _) = self.first_available_ai_slot();
        let lane = self.resolve_task_model_and_effort_with("", engine, &slot_model);
        if lane["model"] != resolved["model"] || lane["effort"] != resolved["effort"] {
            return false;
        }
        let Some(config) = self.daemon_config() else { return false };
        if config.engine != engine {
            return false;
        }
        if !self.cli_daemon.can_serve(&config.signature) {
            self.cli_daemon.ensure(config); // sera prêt pour la prochaine demande
            return false;
        }

        let answer = Rc::new(std::cell::RefCell::new(String::new()));
        let (w1, w2, w3) = (self.self_ref(), self.self_ref(), self.self_ref());
        let (a1, a2, a3) = (answer.clone(), answer.clone(), answer.clone());
        let (id1, id2, id3) = (id.to_string(), id.to_string(), id.to_string());
        let (e1, e2, e3) = (engine.to_string(), engine.to_string(), engine.to_string());
        let (t2, t3) = (raw_text.to_string(), raw_text.to_string());
        let task = DaemonTask {
            on_event: Box::new(move |evt: &J| {
                let Some(o) = w1.upgrade() else { return };
                let cur = a1.borrow().clone();
                let next = if e1 == "agy" { o.on_agy_event(&id1, evt, cur) } else { o.on_claude_event(&id1, evt, cur) };
                *a1.borrow_mut() = next;
            }),
            on_finish: Box::new(move |is_error: bool| {
                let Some(o) = w2.upgrade() else { return };
                if !o.is_running(&id2) {
                    return;
                }
                let answer = a2.borrow().clone();
                o.complete_task(&id2, super::core_tasks::Completion::new(&e2, &t2, &answer, !is_error, false));
            }),
            on_error: Box::new(move |err: String| {
                let Some(o) = w3.upgrade() else { return };
                if !o.is_running(&id3) {
                    return;
                }
                let answer = a3.borrow().clone();
                let mut c = super::core_tasks::Completion::new(&e3, &t3, &answer, false, false);
                c.failure = json!({ "erreur": err });
                o.complete_task(&id3, c);
            }),
        };
        if !self.cli_daemon.submit(prompt, task) {
            return false;
        }
        let child = self.cli_daemon.child();
        self.running.borrow_mut().push((
            id.to_string(),
            RunningTask {
                text: raw_text.to_string(),
                engine: engine.to_string(),
                background: false,
                started_at: js::now_ms(),
                is_daemon: true,
                pid: child.as_ref().map(|c| c.pid).unwrap_or(0),
                child,
                task_mcp_path: None,
                meta,
            },
        ));
        *self.running_task.borrow_mut() = Some(id.to_string());
        true
    }
}

