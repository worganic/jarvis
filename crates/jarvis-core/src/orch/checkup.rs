// Card « Checkup » : le cœur de Jarvis passé au scanner. Chaque partie du corps du personnage (ou chaque
// couche du cœur holographique) représente une fonction réelle de Jarvis ; ce module en fait le bilan à
// partir des signaux déjà connus (aucune mesure lente) : vert = OK, orange = dégradé, rouge = en panne.
// Rendu 3D : ui/checkup.js ; card : ui/widgets.js › renderCheckup.

use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use super::Orchestrator;
use crate::js::{self, J};

const REFRESH_MS: u64 = 15000;
const CARD_KEY: &str = "tool:Checkup";

fn vital(id: &str, label: &str, organ: &str, state: &str, value: impl Into<String>, detail: impl Into<String>) -> J {
    json!({ "id": id, "label": label, "organ": organ, "state": state, "value": value.into(), "detail": detail.into() })
}

/// Erreurs du journal d'erreurs depuis moins d'une heure.
fn recent_errors() -> (usize, usize, String) {
    let now = chrono::Local::now().naive_local();
    let mut errors = 0;
    let mut warns = 0;
    let mut last = String::new();
    for e in crate::logger::logger().recent_errors(30) {
        let at = chrono::NaiveDateTime::parse_from_str(&js::gs(&e, "time"), "%Y-%m-%d %H:%M:%S%.3f").ok();
        if at.map(|a| (now - a).num_minutes() > 60).unwrap_or(true) {
            continue;
        }
        let level = js::gs(&e, "level");
        if level == "WARN" {
            warns += 1;
        } else {
            errors += 1;
        }
        if last.is_empty() {
            last = js::slice_chars(&format!("{} : {}", js::gs(&e, "source"), js::gs(&e, "message")), 90);
        }
    }
    (errors, warns, last)
}

impl Orchestrator {
    /// Bilan des fonctions vitales : { vitals: [...], score: 0..100, state }.
    pub fn checkup_report(&self) -> J {
        let mut v: Vec<J> = vec![];
        let voice = self.voice_probe.borrow().clone().map(|p| p()).unwrap_or_else(|| "off".into());

        // Cerveau : moteur IA.
        let (engine, model) = {
            let d = self.status.d.borrow();
            (d.engine.clone(), d.active_model.clone().unwrap_or_default())
        };
        let (exe, name) = if engine == "agy" { (self.agy.clone(), "AGY") } else { (self.claude.clone(), "Claude Code") };
        let found = PathBuf::from(&exe).exists() || exe == "agy" || exe == "claude";
        let warm = self.cli_daemon.child().is_some();
        let model_s = if model.is_empty() { String::new() } else { format!(" · {model}") };
        // Réactivité : délai médian avant le premier mot des dernières réponses de l'IA.
        let speed = self.first_word_stats().map(|(ms, n)| format!(" Premier mot en {:.1} s (médiane de {n} demande{}).", ms as f64 / 1000.0, if n > 1 { "s" } else { "" })).unwrap_or_default();
        v.push(if !found {
            vital("brain", "Cerveau", "Moteur IA", "error", "introuvable", format!("Binaire {name} introuvable"))
        } else if !warm {
            vital("brain", "Cerveau", "Moteur IA", "warn", format!("{name}{model_s}"), format!("Démon pas encore préchauffé : la première réponse sera plus lente.{speed}"))
        } else {
            vital("brain", "Cerveau", "Moteur IA", "ok", format!("{name}{model_s}"), format!("Démon préchauffé, prêt à répondre.{speed}"))
        });

        // Mémoire : souvenirs + mémoire sémantique.
        let n = self.memory.list().len();
        let semantic = self.embedder.as_ref().map(|e| e.cache_dir.join(e.model.replace('/', std::path::MAIN_SEPARATOR_STR)).join("onnx").exists());
        v.push(match semantic {
            Some(true) => vital("memory", "Mémoire", "Mémoire long terme", "ok", format!("{n} souvenirs"), "Mémoire sémantique active"),
            _ => vital("memory", "Mémoire", "Mémoire long terme", "warn", format!("{n} souvenirs"), "Mémoire sémantique inactive : modèle d'embeddings absent"),
        });

        // Oreilles : moteur vocal et empreinte vocale.
        let speakers = js::arr(&self.get_speaker_status()["speakers"]).len();
        // Qualité de la reconnaissance : trop de phrases écartées sur 24 h fait passer le voyant à l'orange.
        let quality = self.voice_quality_hint();
        v.push(match voice.as_str() {
            "ready" => match quality {
                Some(("warn", text)) => vital("ears", "Oreilles", "Écoute", "warn", format!("Whisper · {speakers} voix connue{}", if speakers > 1 { "s" } else { "" }), format!("Moteur vocal prêt, mais {text} Voir Administration › Qualité vocale.")),
                other => vital("ears", "Oreilles", "Écoute", "ok", format!("Whisper · {speakers} voix connue{}", if speakers > 1 { "s" } else { "" }), format!("Moteur vocal prêt, micro ouvert.{}", other.map(|(_, t)| format!(" {t}")).unwrap_or_default())),
            },
            "wait" => vital("ears", "Oreilles", "Écoute", "warn", "démarrage", "Moteur vocal en cours de démarrage"),
            _ => vital("ears", "Oreilles", "Écoute", "error", "coupée", "Moteur vocal arrêté : Jarvis n'entend plus"),
        });

        // Bouche : voix de Jarvis.
        let muted = self.context_mute.is_muted.get();
        v.push(if voice != "ready" {
            vital("mouth", "Bouche", "Voix", "error", "muette", "Synthèse vocale indisponible sans le moteur vocal")
        } else if muted {
            vital("mouth", "Bouche", "Voix", "warn", "sourdine", format!("Sourdine : {}", self.context_mute.reason.borrow()))
        } else {
            vital("mouth", "Bouche", "Voix", "ok", "active", "Jarvis peut parler")
        });

        // Yeux : veille du bureau.
        let windows = self.desktop_state.windows().len();
        v.push(if self.desktop_state.ready() {
            vital("eyes", "Yeux", "Vision du bureau", "ok", format!("{windows} fenêtres vues"), "Veille du bureau active")
        } else {
            vital("eyes", "Yeux", "Vision du bureau", "error", "aveugle", "Veille du bureau indisponible")
        });

        // Cœur : noyau (tâches, charge).
        let telem = self.last_telemetry.borrow().clone();
        let cpu = js::num(&telem["cpu"]).unwrap_or(0.0);
        let ram = js::num(&telem["ramPct"]).unwrap_or(0.0);
        let running = self.running.borrow().len();
        let queued = self.queue.borrow().len();
        let tasks = if running + queued == 0 { "au repos".to_string() } else { format!("{running} tâche(s) · {queued} en attente") };
        v.push(if js::gb(&telem, "isOverloaded") {
            vital("heart", "Cœur", "Noyau", "warn", tasks, "Machine surchargée : mode éco automatique")
        } else {
            vital("heart", "Cœur", "Noyau", "ok", tasks, "Noyau Rust et passerelle opérationnels")
        });

        // Colonne vertébrale : journal scellé.
        let integrity = self.history.verify_cached();
        v.push(if js::gb(&integrity, "ok") {
            vital("spine", "Colonne", "Journal scellé", "ok", format!("{} entrées", js::s(&integrity["count"])), "Chaîne d'empreintes intacte")
        } else {
            vital("spine", "Colonne", "Journal scellé", "error", "altéré", format!("Journal compromis : {}", js::gs(&integrity, "reason")))
        });

        // Bras droit : connecteurs.
        let mut on: Vec<&str> = vec![];
        let mut off: Vec<String> = vec![];
        if self.google.is_configured() { on.push("Google") } else { off.push("Google".into()) }
        let tg = self.telegram.state();
        if js::gb(&tg, "configured") && js::gb(&tg, "enabled") {
            if js::truthy(&tg["lastError"]) { off.push("Telegram (erreur)".into()) } else { on.push("Telegram") }
        }
        let mcp = self.mcp_registry.enabled().len();
        let state = if on.is_empty() { "warn" } else if off.iter().any(|x| x.contains("erreur")) { "warn" } else { "ok" };
        v.push(vital("armR", "Bras droit", "Connecteurs", state, format!("{} actif(s) · {mcp} MCP", on.len()),
            if off.is_empty() { format!("{} connecté(s)", on.join(", ")) } else { format!("Non relié : {}", off.join(", ")) }));

        // Bras gauche : outils du PC.
        let chrome = self.browser_bridge.connected();
        let (claude, agy) = self.ai_watch.running();
        let agents = claude + agy;
        v.push(vital("armL", "Bras gauche", "Outils du PC", if chrome { "ok" } else { "warn" },
            format!("{agents} agent(s) suivi(s)"),
            if chrome { "Extension Chrome reliée, outils natifs prêts".to_string() } else { "Extension Chrome non reliée : pas de pilotage des onglets".to_string() }));

        // Jambes : ressources de la machine.
        let disks = self.hardware_probe.borrow().as_ref().map(|p| js::arr(&p.telemetry()["disks"])).unwrap_or_default();
        let full: Vec<String> = disks.iter().filter(|d| js::num(&d["pct"]).unwrap_or(0.0) >= 92.0).map(|d| js::gs(d, "name")).collect();
        let legs_state = if cpu >= 95.0 || ram >= 95.0 { "error" } else if cpu >= 80.0 || ram >= 85.0 || !full.is_empty() { "warn" } else { "ok" };
        v.push(vital("legs", "Jambes", "Ressources", legs_state, format!("CPU {cpu:.0} % · RAM {ram:.0} %"),
            if full.is_empty() { "Machine à l'aise".to_string() } else { format!("Disque presque plein : {}", full.join(", ")) }));

        // Système nerveux : erreurs récentes.
        let (errors, warns, last) = recent_errors();
        v.push(if errors > 0 {
            vital("nerves", "Nerfs", "Erreurs récentes", "error", format!("{errors} erreur(s) · 1 h"), last)
        } else if warns > 0 {
            vital("nerves", "Nerfs", "Erreurs récentes", "warn", format!("{warns} alerte(s) · 1 h"), last)
        } else {
            vital("nerves", "Nerfs", "Erreurs récentes", "ok", "aucune", "Aucune erreur depuis une heure")
        });

        let pts: f64 = v.iter().map(|x| match js::gs(x, "state").as_str() { "ok" => 1.0, "warn" => 0.5, _ => 0.0 }).sum();
        let score = (pts / v.len() as f64 * 100.0).round() as i64;
        let worst = if v.iter().any(|x| x["state"] == "error") { "error" } else if v.iter().any(|x| x["state"] == "warn") { "warn" } else { "ok" };
        json!({ "vitals": v, "score": score, "state": worst })
    }

    fn checkup_card(&self) -> J {
        let mut card = self.checkup_report();
        let style = self.status.d.borrow().heart_style.clone();
        card["title"] = json!("Checkup");
        card["tool"] = json!("checkup");
        card["checkup"] = json!(true);
        card["heartStyle"] = json!(if style.is_empty() { "classic".into() } else { style });
        card["ttl"] = json!(0);
        card["persist"] = json!(true);
        card
    }

    /// Ouvre la card Checkup (scan complet) ; ses fonctions vitales se mettent à jour toutes les 15 s.
    pub fn show_checkup_card(self: &Rc<Self>) {
        let card = self.checkup_card();
        self.record("checkup", json!({ "score": card["score"], "etat": card["state"] }));
        self.show_widget("tool", card);
        let me = Rc::downgrade(self);
        self.checkup_timer.every(REFRESH_MS, move || {
            let Some(o) = me.upgrade() else { return };
            if !o.cards.has_key(CARD_KEY) {
                o.checkup_timer.clear();
                return;
            }
            let mut card = o.checkup_card();
            card["refresh"] = json!(true); // le scan ne se rejoue pas
            o.show_widget("tool", card);
        });
    }

    /// Phrase dite après le checkup : le bilan en une phrase.
    pub fn checkup_summary(report: &J) -> String {
        let bad: Vec<String> = js::arr(&report["vitals"]).iter().filter(|x| x["state"] != "ok").map(|x| format!("{} ({})", js::gs(x, "organ").to_lowercase(), js::gs(x, "detail"))).collect();
        let score = js::s(&report["score"]);
        if bad.is_empty() {
            format!("Scan terminé : {score} %. Toutes mes fonctions vitales sont au vert.")
        } else {
            format!("Scan terminé : {score} %. À surveiller : {}.", bad.join(" ; "))
        }
    }
}

#[allow(unused)]
fn _v(_: Value) {}
