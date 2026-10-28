// Cards de liste (orchestrator/list-cards.js) : chaque élément a un numéro (« annule le rappel 2 »,
// « lance l'application 12 ») et ses boutons d'action. Les boutons agissent par identifiant, jamais
// par numéro. Bouton → listAction(genre, id, action) → ListCards::action().

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use super::reminders::when_label;
use super::{system, OrchRef, Up};
use crate::js::{self, J};

fn call(kind: &str, id: J, action: &str) -> J {
    json!({ "method": "listAction", "args": [kind, id, action] })
}

fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}

pub struct ListCards {
    o: OrchRef,
    numbers: RefCell<HashMap<String, Vec<J>>>,
}

impl ListCards {
    pub fn new(o: OrchRef) -> Self {
        Self { o, numbers: RefCell::new(HashMap::new()) }
    }

    pub fn remember(&self, kind: &str, ids: Vec<J>) {
        self.numbers.borrow_mut().insert(kind.to_string(), ids);
    }

    pub fn by_number(&self, kind: &str, n: i64) -> Option<J> {
        if n < 1 {
            return None;
        }
        self.numbers.borrow().get(kind).and_then(|l| l.get((n - 1) as usize).cloned())
    }

    fn show(&self, card: J) -> J {
        self.o.up().show_widget("tool", card.clone());
        card
    }

    // --- Rappels ---------------------------------------------------------------------------------

    pub fn reminders_card(&self) -> J {
        let list = self.o.up().reminders.upcoming();
        self.remember("rappel", list.iter().map(|r| r["id"].clone()).collect());
        let rows: Vec<J> = list
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let repeat = &r["repeat"];
                let what = js::gs(r, "what");
                json!({
                    "icon": if js::truthy(repeat) { "↻" } else { "⏰" },
                    "title": format!("{}. {}", i + 1, if what.is_empty() { "Rappel".to_string() } else { what }),
                    "sub": format!("{}{}", when_label(r["at"].as_i64().unwrap_or(0)), if js::truthy(repeat) { if js::gs(repeat, "every") == "week" { " · chaque semaine" } else { " · chaque jour" } } else { "" }),
                    "call": call("rappel", r["id"].clone(), "detail"),
                    "hint": "Clic : lire ce rappel",
                    "actions": [
                        { "icon": "⏲", "title": "Reporter d’une heure", "call": call("rappel", r["id"].clone(), "reporter") },
                        { "icon": "✕", "tone": "danger", "title": "Annuler ce rappel", "call": call("rappel", r["id"].clone(), "annuler"), "done": "annulé" }
                    ]
                })
            })
            .collect();
        self.show(json!({
            "title": "Mes rappels", "tool": "rappels", "count": list.len(), "ttl": 120000, "empty": "Aucun rappel programmé.", "rows": rows,
            "footer": "« annule le rappel 2 » · « reporte le rappel 1 » · ⏲ reporter d’une heure"
        }))
    }

    // --- Raccourcis (macros) ----------------------------------------------------------------------

    pub fn macros_card(&self) -> J {
        let list = self.o.up().macros.list();
        self.remember("raccourci", list.iter().map(|m| m["name"].clone()).collect());
        let rows: Vec<J> = list
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let name = js::gs(m, "name");
                json!({
                    "icon": "⚡", "title": format!("{}. {name}", i + 1), "sub": js::str_list(&m["steps"]).join(" → "),
                    "call": call("raccourci", json!(name), "lancer"), "hint": format!("Clic : lancer « {name} »"),
                    "actions": [
                        { "icon": "▶", "title": "Lancer", "call": call("raccourci", json!(name), "lancer") },
                        { "icon": "✕", "tone": "danger", "title": "Supprimer ce raccourci", "call": call("raccourci", json!(name), "supprimer"), "done": "supprimé" }
                    ]
                })
            })
            .collect();
        self.show(json!({
            "title": "Mes raccourcis", "tool": "raccourcis", "count": list.len(), "ttl": 120000,
            "empty": "Aucun raccourci. Dis par exemple : « quand je dis mode travail, ouvre VS Code et mets de la musique ».",
            "rows": rows, "footer": "« lance le raccourci 2 » · « supprime le raccourci 1 » · ou dis son nom"
        }))
    }

    // --- Automatisations --------------------------------------------------------------------------

    pub fn automations_card(&self) -> J {
        let o = self.o.up();
        let auto = &o.automations;
        let list = auto.rules();
        self.remember("automatisation", list.iter().map(|r| r["id"].clone()).collect());
        let rows: Vec<J> = list
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let enabled = js::gb(r, "enabled");
                let runs = r["runs"].as_i64().unwrap_or(0);
                let mut row = json!({
                    "icon": if enabled { "⚙" } else { "⏸" },
                    "title": format!("{}. {}", i + 1, crate::re!(r" → .*$").replace(&auto.label(r), "")),
                    "sub": format!("{}{}", js::str_list(&r["steps"]).join(" → "), if enabled { "" } else { " · en pause" }),
                    "meta": if runs > 0 { format!("{runs}×") } else { String::new() },
                    "call": call("automatisation", r["id"].clone(), "lancer"), "hint": "Clic : exécuter maintenant",
                    "actions": [
                        { "icon": "▶", "title": "Exécuter maintenant", "call": call("automatisation", r["id"].clone(), "lancer") },
                        if enabled { json!({ "icon": "⏸", "title": "Mettre en pause", "call": call("automatisation", r["id"].clone(), "pause") }) }
                        else { json!({ "icon": "↺", "title": "Réactiver", "call": call("automatisation", r["id"].clone(), "reprendre") }) },
                        { "icon": "✕", "tone": "danger", "title": "Supprimer", "call": call("automatisation", r["id"].clone(), "supprimer"), "done": "supprimée" }
                    ]
                });
                if !enabled {
                    row["tone"] = json!("muted");
                }
                row
            })
            .collect();
        self.show(json!({
            "title": "Mes automatisations", "tool": "automatisations", "count": list.len(), "ttl": 120000,
            "empty": "Aucune automatisation. Dis par exemple : « quand Teams s’ouvre, mets la musique en pause ».",
            "rows": rows, "footer": "« supprime l’automatisation 2 » · « mets en pause l’automatisation 1 » · pas de relance avant 10 s"
        }))
    }

    // --- Résultats d'une recherche de fichiers ------------------------------------------------------

    pub fn files_card(&self, query: &str, items: &[String]) -> J {
        self.remember("fichier", items.iter().map(|p| json!(p)).collect());
        let rows: Vec<J> = items
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let path = Path::new(p);
                json!({
                    "icon": "▤", "title": format!("{}. {}", i + 1, path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()),
                    "sub": path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
                    "call": call("fichier", json!(p), "ouvrir"), "hint": "Clic : ouvrir",
                    "actions": [
                        { "icon": "▶", "title": "Ouvrir", "call": call("fichier", json!(p), "ouvrir") },
                        { "icon": "📂", "title": "Ouvrir le dossier", "call": call("fichier", json!(p), "dossier") }
                    ]
                })
            })
            .collect();
        self.show(json!({ "title": format!("Fichiers : {query}"), "tool": "recherche locale", "count": items.len(), "ttl": 120000, "rows": rows, "footer": "« ouvre le 2 » · 📂 ouvrir le dossier qui le contient" }))
    }

    // --- Applications installées ------------------------------------------------------------------

    pub fn running_window(&self, name: &str) -> Option<J> {
        let o = self.o.up();
        if !o.desktop_state.ready() {
            return None;
        }
        let key = fold(name);
        let first = key.split(' ').next().unwrap_or("").to_string();
        o.desktop_state.windows().into_iter().find(|w| {
            let app = fold(&js::gs(w, "app"));
            let pr = fold(&js::gs(w, "process"));
            app == key || pr == key || (first.chars().count() > 3 && (app == first || pr == first))
        })
    }

    pub fn apps_card(&self, filter: &str) -> J {
        let o = self.o.up();
        o.app_index.ensure();
        let want = fold(filter);
        let mut list: Vec<super::app_index::AppEntry> = o.app_index.entries.borrow().iter().filter(|e| want.is_empty() || e.key.contains(&want)).cloned().collect();
        list.sort_by(|a, b| js::fold_lower(&a.name).cmp(&js::fold_lower(&b.name)));
        self.remember("appli", list.iter().map(|e| json!(e.name)).collect());
        let mut running = 0;
        let rows: Vec<J> = list
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let w = self.running_window(&e.name);
                let mut row = json!({ "icon": if w.is_some() { "▶" } else { "◧" }, "title": format!("{}. {}", i + 1, e.name) });
                match &w {
                    Some(w) => {
                        running += 1;
                        row["tone"] = json!("ok");
                        row["sub"] = json!(format!("ouverte · écran {} · {}", js::s(&w["monitor"]), js::gs(w, "state")));
                        row["call"] = json!({ "method": "desktopAction", "args": [w["hwnd"], "afficher"] });
                        row["hint"] = json!("Clic : afficher");
                        row["actions"] = json!([
                            { "icon": "◎", "title": "Mettre au premier plan", "call": { "method": "desktopAction", "args": [w["hwnd"], "afficher"] } },
                            { "icon": "⇄", "title": "Changer d’écran", "call": { "method": "desktopAction", "args": [w["hwnd"], "deplacer"] } },
                            { "icon": "✕", "tone": "danger", "title": "Fermer (accord oral)", "call": { "method": "desktopAction", "args": [w["hwnd"], "fermer"] } }
                        ]);
                    }
                    None => {
                        row["sub"] = json!("");
                        row["call"] = call("appli", json!(e.name), "lancer");
                        row["hint"] = json!("Clic : lancer");
                        row["actions"] = json!([{ "icon": "▶", "title": "Lancer", "call": call("appli", json!(e.name), "lancer") }]);
                    }
                }
                row
            })
            .collect();
        self.show(json!({
            "title": if want.is_empty() { "Applications installées".to_string() } else { format!("Applications : {filter}") },
            "tool": "applications", "wide": true, "count": list.len(), "rows": rows, "ttl": 180000, "running": running,
            "empty": if want.is_empty() { "Aucune application trouvée dans le menu Démarrer.".to_string() } else { format!("Aucune application installée ne correspond à « {filter} ».") },
            "footer": "« lance l’application 12 » · ▶ lancer · ▣ afficher · ✕ fermer (celles qui sont ouvertes)"
        }))
    }

    // --- Voix enregistrées ------------------------------------------------------------------------

    pub fn voices_card(&self) -> J {
        let st = self.o.up().get_speaker_status();
        let list = js::arr(&st["speakers"]);
        self.remember("voix", list.iter().map(|s| s["id"].clone()).collect());
        let rows: Vec<J> = list
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let primary = js::gb(s, "isPrimary");
                let name = js::gs(s, "name");
                let samples = s["samples"].as_i64().unwrap_or(0);
                let sub: Vec<String> = [if primary { "voix principale".to_string() } else { String::new() }, if samples > 0 { format!("{samples} échantillons") } else { String::new() }].into_iter().filter(|x| !x.is_empty()).collect();
                let mut actions = vec![];
                if !primary {
                    actions.push(json!({ "icon": "★", "title": "Définir comme voix principale", "call": call("voix", s["id"].clone(), "principal") }));
                }
                actions.push(json!({ "icon": "✕", "tone": "danger", "title": "Supprimer cette empreinte", "say": format!("Supprime la voix de {name}") }));
                let mut row = json!({
                    "icon": if primary { "★" } else { "◉" }, "title": format!("{}. {name}", i + 1), "sub": sub.join(" · "),
                    "hint": if primary { "Voix principale active" } else { "Clic : définir comme voix principale" }, "actions": actions
                });
                if primary {
                    row["tone"] = json!("fav");
                } else {
                    row["call"] = call("voix", s["id"].clone(), "principal");
                }
                row
            })
            .collect();
        self.show(json!({
            "title": "Voix enregistrées", "tool": "empreintes", "count": list.len(), "ttl": 120000, "empty": "Aucune empreinte vocale. Dis « enregistre ma voix ».",
            "rows": rows, "footer": if st["enabled"] == json!(false) { "reconnaissance du locuteur désactivée" } else { "★ voix principale · ✕ supprimer" }
        }))
    }

    // --- Agenda ----------------------------------------------------------------------------------------

    pub fn event_start(e: &J) -> Option<i64> {
        let start = js::gs(e, "start");
        if crate::re!(r"T\d{2}:\d{2}").is_match(&start) {
            return js::parse_date_ms(&start);
        }
        let day = crate::re!(r"\d{4}-\d{2}-\d{2}").find(&start).map(|m| m.as_str().to_string())?;
        let time = js::gs(e, "time");
        if crate::re!(r"^\d{1,2}:\d{2}$").is_match(&time) {
            return js::parse_date_ms(&format!("{day}T{:0>5}:00", time));
        }
        None
    }

    pub fn agenda_rows(&self, items: &[J]) -> Vec<J> {
        let now = js::now_ms();
        items
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let at = Self::event_start(e);
                let when = match at {
                    Some(a) => js::local_of(a).format("%d/%m %H:%M").to_string(),
                    None => {
                        let t = js::gs(e, "time");
                        if t.is_empty() { js::slice_chars(&js::gs(e, "start"), 16).replace('T', " ") } else { t }
                    }
                };
                let remind_at = at.map(|a| a - 15 * 60000);
                let title = { let t = js::gs(e, "title"); if t.is_empty() { "Rendez-vous".to_string() } else { t } };
                let sub: Vec<String> = [when, js::gs(e, "place")].into_iter().filter(|x| !x.is_empty()).collect();
                let actions = match remind_at {
                    Some(r) if r > now => json!([{ "icon": "🔔", "title": "Me le rappeler 15 minutes avant", "call": call("agenda", json!(format!("{r}|{title}")), "rappel"), "done": "rappel programmé" }]),
                    _ => json!([]),
                };
                json!({
                    "icon": "▦", "title": format!("{}. {title}", i + 1), "sub": sub.join(" · "),
                    "call": call("agenda", json!(format!("{}|{title}", remind_at.unwrap_or(0))), "ouvrir"),
                    "hint": "Clic : ouvrir dans Google Agenda (Chrome)", "actions": actions
                })
            })
            .collect()
    }

    /// Rendez-vous au format de la card Agenda, d'après les événements du connecteur Google (ou de l'agenda local).
    pub fn agenda_items(events: &[J]) -> Vec<J> {
        events
            .iter()
            .map(|e| {
                let start = Some(js::gs(e, "debut")).filter(|s| !s.is_empty()).unwrap_or_else(|| js::gs(e, "date"));
                json!({ "title": js::gs(e, "titre"), "start": start, "time": js::gs(e, "heure"), "place": js::gs(e, "lieu") })
            })
            .collect()
    }

    /// Card Agenda : trois rendez-vous au plus, et un bouton qui ouvre Google Agenda dans le navigateur.
    pub fn agenda_card(&self, title: String, items: &[J]) {
        let o = self.o.up();
        let shown: Vec<J> = items.iter().take(3).cloned().collect();
        let more = items.len().saturating_sub(shown.len());
        let mut rows = self.agenda_rows(&shown);
        if rows.is_empty() {
            rows.push(json!({ "icon": "▦", "title": "Aucun rendez-vous à venir", "tone": "muted" }));
        }
        rows.push(json!({
            "icon": "🗓", "title": "Ouvrir Google Agenda",
            "sub": if more > 0 { format!("+ {more} autre(s) rendez-vous dans ton agenda") } else { "dans le navigateur".to_string() },
            "call": call("agenda", json!(""), "ouvrir"), "hint": "Ouvre ton agenda Google dans le navigateur"
        }));
        o.show_widget(
            "agenda",
            json!({
                "title": title, "rows": rows, "count": shown.len(), "items": shown,
                "footer": if shown.is_empty() { "" } else { "🔔 : me le rappeler 15 minutes avant" }
            }),
        );
    }

    /// Les trois prochains rendez-vous (card Agenda, bouton « Agenda » de la card Actions).
    pub async fn next_appointments(&self) -> J {
        let o = self.o.up();
        let r = o.google.agenda_semaine(&json!({})).await;
        let now = js::now_ms();
        let mut items = Self::agenda_items(&js::arr(&r["evenements"]));
        items.retain(|e| Self::event_start(e).map(|a| a >= now).unwrap_or(true));
        items.sort_by_key(|e| Self::event_start(e).unwrap_or(i64::MAX));
        self.agenda_card("Prochains rendez-vous".into(), &items);
        json!({ "ok": true, "say": null })
    }

    // --- Actions des boutons ------------------------------------------------------------------------

    pub async fn action(&self, kind: &str, id: &J, action: &str) -> J {
        let o = self.o.up();
        let not = |say: &str| json!({ "ok": false, "say": say });
        match kind {
            "rappel" => {
                let rid = js::num(id).unwrap_or(0.0) as i64;
                let Some(r) = o.reminders.upcoming().into_iter().find(|x| x["id"].as_i64() == Some(rid)) else { return not("Ce rappel n’existe plus.") };
                let what = { let w = js::gs(&r, "what"); if w.is_empty() { "rappel".to_string() } else { w } };
                let at = r["at"].as_i64().unwrap_or(0);
                match action {
                    "detail" | "lire" => json!({ "ok": true, "say": format!("Rappel : {what}, prévu {}.", when_label(at)) }),
                    "annuler" => {
                        o.reminders.cancel(rid);
                        self.reminders_card();
                        json!({ "ok": true, "say": format!("Rappel annulé : {what}.") })
                    }
                    "reporter" => {
                        o.reminders.cancel(rid);
                        let when = js::now_ms().max(at) + 3_600_000;
                        o.reminders.add(&js::gs(&r, "what"), when, r["repeat"].clone());
                        self.reminders_card();
                        json!({ "ok": true, "say": format!("Rappel reporté {}.", when_label(when)) })
                    }
                    _ => not("Action inconnue."),
                }
            }
            "raccourci" => {
                let name = js::s(id);
                let Some(m) = o.macros.list().into_iter().find(|x| js::gs(x, "name") == name) else { return not("Ce raccourci n’existe plus.") };
                match action {
                    "lancer" => {
                        o.run_macro(&m, false).await;
                        json!({ "ok": true, "say": null })
                    }
                    "supprimer" => {
                        o.macros.remove(&name);
                        o.record("macro_supprimee", json!({ "nom": name }));
                        self.macros_card();
                        json!({ "ok": true, "say": format!("Raccourci {name} supprimé.") })
                    }
                    _ => not("Action inconnue."),
                }
            }
            "automatisation" => {
                let auto = o.automations.clone();
                let Some(r) = auto.rules().into_iter().find(|x| &x["id"] == id) else { return not("Cette automatisation n’existe plus.") };
                let rid = js::s(&r["id"]);
                match action {
                    "lancer" => {
                        auto.run(&r, &json!({})).await;
                        json!({ "ok": true, "say": null })
                    }
                    "supprimer" => {
                        auto.remove(&rid);
                        o.record("automatisation_supprimee", json!({ "regle": auto.label(&r) }));
                        self.automations_card();
                        json!({ "ok": true, "say": "Automatisation supprimée." })
                    }
                    "pause" | "reprendre" => {
                        auto.set_enabled(&rid, action == "reprendre");
                        self.automations_card();
                        json!({ "ok": true, "say": if action == "pause" { "Automatisation en pause." } else { "Automatisation réactivée." } })
                    }
                    _ => not("Action inconnue."),
                }
            }
            "fichier" => {
                let p = js::s(id);
                let known = o.last_file_results.borrow().as_ref().map(|(items, _)| items.contains(&p)).unwrap_or(false);
                if !known {
                    return not("Ce résultat n’est plus valable : relance la recherche.");
                }
                match action {
                    "ouvrir" => {
                        let o2 = self.o.clone();
                        let r = o.local_tool("mcp__system__ouvrir_fichier", json!({ "chemin": p }), Box::new(move |prm| Box::pin(async move { o2.up().file_search.open(&js::gs(&prm, "chemin")).unwrap_or_else(|e| json!({ "ok": false, "error": e })) }))).await;
                        if js::truthy(&r["error"]) || js::gb(&r, "denied") {
                            json!({ "ok": false, "say": if js::gb(&r, "denied") { "Ouverture refusée.".to_string() } else { format!("Impossible d'ouvrir ce fichier : {}", js::gs(&r, "error")) } })
                        } else {
                            json!({ "ok": true, "say": null })
                        }
                    }
                    "dossier" => {
                        let dir = Path::new(&p).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                        let r = o.local_tool("mcp__system__ouvrir_dossier", json!({ "chemin": dir }), Box::new(|prm| Box::pin(async move { system::ouvrir_dossier(&prm).await.unwrap_or_else(|e| json!({ "ok": false, "error": e })) }))).await;
                        if js::truthy(&r["error"]) || js::gb(&r, "denied") { not("Je n’ai pas pu ouvrir le dossier.") } else { json!({ "ok": true, "say": null }) }
                    }
                    _ => not("Action inconnue."),
                }
            }
            "note" => {
                let file = super::zones::resolve(&js::s(id));
                let dir = super::zones::resolve(&o.notes.dir.to_string_lossy()).to_lowercase();
                if !file.to_lowercase().starts_with(&format!("{dir}\\")) || !Path::new(&file).exists() {
                    return not("Cette note n’existe plus.");
                }
                o.notes.load(Path::new(&file));
                o.notes.show();
                if action == "resumer" {
                    return o.notes.summarize().await;
                }
                json!({ "ok": true, "say": null })
            }
            "projet" => {
                let Some(p) = o.projects.find(&js::s(id)) else { return not("Ce projet n’existe plus.") };
                let name = js::gs(&p, "name");
                match action {
                    "ouvrir" | "detail" => {
                        let info = o.projects.info(&p).await;
                        o.show_widget("tool", super::skill_intents::project_info_card(&info));
                        json!({ "ok": true, "say": format!("Projet {name} : détails affichés.") })
                    }
                    "dossier" => {
                        let r = o.local_tool("mcp__system__ouvrir_dossier", json!({ "chemin": p["path"] }), Box::new(|prm| Box::pin(async move { system::ouvrir_dossier(&prm).await.unwrap_or_else(|e| json!({ "ok": false, "error": e })) }))).await;
                        if js::truthy(&r["error"]) || js::gb(&r, "denied") { not("Je n’ai pas pu ouvrir le dossier du projet.") } else { json!({ "ok": true, "say": null }) }
                    }
                    "powershell" => {
                        let command = { let c = js::gs(&o.config.get("tmux"), "claudeCommand"); if c.is_empty() { "claude".to_string() } else { c } };
                        if command.contains("--dangerously-skip-permissions") {
                            o.record("agent_sans_validation", json!({ "origine": "card", "projet": name }));
                        }
                        super::projects::open_claude_powershell(&js::gs(&p, "path"), &name, &command);
                        o.record("action_locale", json!({ "outil": "claude_powershell", "detail": p["path"], "origine": "card" }));
                        json!({ "ok": true, "say": format!("Claude Code s'ouvre dans {name}, dans PowerShell.") })
                    }
                    _ => not("Action inconnue."),
                }
            }
            "appli" => {
                let name = js::s(id);
                let entry = o.app_index.entries.borrow().iter().find(|e| e.name == name).cloned();
                let Some(entry) = entry else { return not("Cette application n’est plus dans le menu Démarrer.") };
                if action != "lancer" {
                    return not("Action inconnue.");
                }
                if let Some(w) = self.running_window(&entry.name) {
                    return o.desktop_action(w["hwnd"].as_i64().unwrap_or(0), "afficher").await;
                }
                let o2 = self.o.clone();
                let e2 = entry.clone();
                let r = o.local_tool("mcp__system__ouvrir_application", json!({ "nom": entry.name }), Box::new(move |_| Box::pin(async move { o2.up().app_index.open(&e2).unwrap_or_else(|e| json!({ "ok": false, "error": e })) }))).await;
                if js::truthy(&r["error"]) || js::gb(&r, "denied") {
                    json!({ "ok": false, "say": if js::gb(&r, "denied") { "Lancement refusé.".to_string() } else { format!("Je n'arrive pas à lancer {}.", entry.name) } })
                } else {
                    json!({ "ok": true, "say": format!("J'ouvre {}.", entry.name) })
                }
            }
            "produit" => o.shopping.act(&js::s(id), action).await,
            "voix" if action == "principal" => {
                o.emit(json!({ "type": "speaker_primary", "id": id }));
                o.record("locuteur_principal", json!({ "id": id, "origine": "card" }));
                let me = self.o.clone();
                js::set_timeout(800, move || {
                    me.up().list_cards.voices_card();
                });
                json!({ "ok": true, "say": "Voix principale changée." })
            }
            "agenda" => match action {
                "prochains" => self.next_appointments().await,
                "ouvrir" | "chrome" => {
                    let r = o.local_tool("mcp__system__ouvrir_url", json!({ "url": "https://calendar.google.com/calendar/u/0/r" }), Box::new(|prm| Box::pin(async move { system::ouvrir_url(&prm).await.unwrap_or_else(|e| json!({ "ok": false, "error": e })) }))).await;
                    if r["ok"] == json!(false) {
                        json!({ "ok": false, "say": if js::truthy(&r["error"]) { js::gs(&r, "error") } else { "Ouverture refusée.".into() } })
                    } else {
                        json!({ "ok": true, "say": null })
                    }
                }
                "rappel" => {
                    let s = js::s(id);
                    let (at, title) = s.split_once('|').unwrap_or((&s, ""));
                    let when: i64 = at.parse().unwrap_or(0);
                    if when == 0 || when < js::now_ms() {
                        return not("Ce rendez-vous est trop proche pour un rappel.");
                    }
                    o.reminders.add(title, when, Value::Null);
                    json!({ "ok": true, "say": format!("Je te le rappellerai {}.", when_label(when)) })
                }
                _ => not("Action inconnue."),
            },
            "smartlist" => {
                let lid = js::s(id);
                if action == "show" {
                    if let Some(list) = o.smart_lists.get(&lid) {
                        o.smart_lists.show_list_card(&list);
                        return json!({ "ok": true, "say": format!("Affichage de la {}.", js::gs(&list, "title")) });
                    }
                }
                o.smart_lists.action(&lid, action, &json!({})).await
            }
            _ => not("Action inconnue."),
        }
    }
}
