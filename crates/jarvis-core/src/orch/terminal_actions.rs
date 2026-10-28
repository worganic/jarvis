// Automatismes du terminal (orchestrator/terminal-actions.js) : une action = un nom, des phrases à dire
// et un petit script de frappes. Catalogue : data/terminal-actions.json (Administration › Terminal).
// Script, une étape par ligne : texte tapé ; {Entrée} {Échap} {Ctrl+C} {Ctrl+D} {Ctrl+U} {Tab} {Maj+Tab}
// {Haut} {Bas} {Gauche} {Droite} ; {attendre 500} ; {Mode plan|bypass|normal|édition}.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use crate::js::{self, J};

pub fn fold(text: &str) -> String {
    let t = js::fold_lower(text);
    let t = crate::re!(r#"['’\-?!,;:.…«»"]"#).replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

fn key_token(name: &str) -> Option<&'static str> {
    Some(match name {
        "entree" | "enter" => "Enter",
        "echap" | "escape" | "esc" => "Escape",
        "ctrl+c" => "C-c",
        "ctrl+d" => "C-d",
        "ctrl+u" => "C-u",
        "tab" => "Tab",
        "maj+tab" => "BTab",
        "haut" => "Up",
        "bas" => "Down",
        "gauche" => "Left",
        "droite" => "Right",
        _ => return None,
    })
}

const MAX_WAIT: i64 = 5000;
const MAX_STEPS: usize = 12;
const VERSION: i64 = 2;

/// Script → (étapes [{ text } | { key } | { wait } | { mode }], erreurs).
pub fn parse_script(script: &str) -> (Vec<J>, Vec<String>) {
    let mut steps = vec![];
    let mut errors = vec![];
    for line in script.split('\n').map(|l| l.trim_end()).filter(|l| !l.trim().is_empty()) {
        let Some(tok) = crate::re!(r"^\{(.+)\}$").captures(line.trim()) else {
            steps.push(json!({ "text": line.trim_start() }));
            continue;
        };
        let name = fold(&tok[1]);
        let name = crate::re!(r"\s*\+\s*").replace_all(&name, "+");
        let name = crate::re!(r"^(ctrl|control|controle|maj|shift) (\w)").replace(&name, "$1+$2");
        let name = crate::re!(r"^(?:control|controle)\+").replace(&name, "ctrl+");
        let name = crate::re!(r"^shift\+").replace(&name, "maj+").into_owned();
        if let Some(w) = crate::re!(r"^(?:attendre|attends|pause|wait) (\d{1,5})$").captures(&name) {
            steps.push(json!({ "wait": w[1].parse::<i64>().unwrap_or(0).min(MAX_WAIT) }));
            continue;
        }
        if let Some(m) = crate::re!(r"^mode (plan|bypass|normal|defaut|edition)$").captures(&name) {
            let mode = match &m[1] {
                "defaut" => "normal",
                "edition" => "edits",
                other => other,
            };
            steps.push(json!({ "mode": mode }));
            continue;
        }
        match key_token(&name) {
            Some(k) => steps.push(json!({ "key": k })),
            None => errors.push(format!("Touche inconnue : {}", line.trim())),
        }
    }
    if steps.len() > MAX_STEPS {
        errors.push(format!("Trop d’étapes ({}, {MAX_STEPS} au plus).", steps.len()));
    }
    steps.truncate(MAX_STEPS);
    (steps, errors)
}

pub fn defaults() -> Vec<J> {
    vec![
        json!({ "id": "valider", "label": "Valider", "icon": "⏎", "phrases": [], "script": "{Entrée}", "confirm": false, "clear": false, "card": true, "enabled": true }),
        json!({ "id": "effacer-prompt", "label": "Effacer le prompt", "icon": "⌫", "phrases": ["efface le prompt", "vide le prompt", "efface la saisie", "vide la saisie"], "script": "{Ctrl+U}", "confirm": false, "clear": false, "card": true, "enabled": true }),
        json!({ "id": "interrompre", "label": "Interrompre", "icon": "■", "phrases": ["interromps", "interromps claude", "interrompre"], "script": "{Échap}", "confirm": false, "clear": false, "card": true, "enabled": true }),
        json!({ "id": "modele", "label": "Changer de modèle", "icon": "◈", "phrases": ["change de modele", "changer de modele", "change le modele", "choisis le modele"], "script": "/model\n{Entrée}", "confirm": false, "clear": true, "card": true, "enabled": true }),
        json!({ "id": "effacer-contexte", "label": "Effacer le contexte", "icon": "∅", "phrases": ["efface le contexte", "vide le contexte", "reinitialise le contexte"], "script": "/clear\n{Entrée}", "confirm": true, "clear": true, "card": true, "enabled": true }),
        json!({ "id": "compacter", "label": "Compacter", "icon": "▤", "phrases": ["compacte le contexte", "compacte la conversation"], "script": "/compact\n{Entrée}", "confirm": false, "clear": true, "card": true, "enabled": true }),
        json!({ "id": "mode-plan", "label": "Mode plan", "icon": "◫", "phrases": ["mode plan", "passe en mode plan", "active le mode plan", "passe en plan"], "script": "{Mode plan}", "confirm": false, "clear": false, "card": true, "enabled": true }),
        json!({ "id": "mode-bypass", "label": "Mode bypass", "icon": "⚠", "phrases": ["mode bypass", "passe en mode bypass", "active le mode bypass", "mode bypass permissions", "passe en bypass permissions"], "script": "{Mode bypass}", "confirm": true, "clear": false, "card": true, "enabled": true }),
    ]
}

/// Actions ajoutées après la première version du fichier.
const ADDED_IN: &[(i64, &[&str])] = &[(2, &["mode-plan", "mode-bypass"])];

/// Un élément du catalogue nettoyé, ou une erreur.
fn sanitize(a: &J, seen: &mut HashSet<String>) -> Result<J, String> {
    if !a.is_object() {
        return Err("Action invalide.".into());
    }
    let label = js::slice_chars(js::gs(a, "label").trim(), 40);
    if label.is_empty() {
        return Err("Une action n’a pas de nom.".into());
    }
    let id = js::gs(a, "id").to_lowercase();
    let id = crate::re!(r"[^a-z0-9_-]+").replace_all(&id, "-");
    let mut id = js::slice_chars(crate::re!(r"^-+|-+$").replace_all(&id, "").as_ref(), 40);
    if id.is_empty() {
        let f = fold(&label).replace(' ', "-");
        id = js::slice_chars(&crate::re!(r"[^a-z0-9-]").replace_all(&f, ""), 40);
        if id.is_empty() {
            id = "action".into();
        }
    }
    let mut unique = id.clone();
    let mut i = 2;
    while seen.contains(&unique) {
        unique = format!("{}-{i}", js::slice_chars(&id, 36));
        i += 1;
    }
    let script = js::slice_chars(&js::gs(a, "script"), 800);
    let (steps, errors) = parse_script(&script);
    if let Some(e) = errors.first() {
        return Err(format!("« {label} » : {e}"));
    }
    if steps.is_empty() {
        return Err(format!("« {label} » : le script est vide."));
    }
    let raw_phrases: Vec<String> = match &a["phrases"] {
        Value::Array(l) => l.iter().map(js::s).collect(),
        other => crate::re!(r"[\n,;]+").split(&js::s(other)).map(String::from).collect(),
    };
    let mut phrases: Vec<String> = vec![];
    for p in raw_phrases.iter().map(|p| fold(p)).filter(|p| p.chars().count() >= 3).take(10).map(|p| js::slice_chars(&p, 80)) {
        if !phrases.contains(&p) {
            phrases.push(p);
        }
    }
    seen.insert(unique.clone());
    let icon = js::slice_chars(js::gs(a, "icon").trim(), 4);
    Ok(json!({
        "id": unique, "label": label, "icon": if icon.is_empty() { "⚡".to_string() } else { icon }, "phrases": phrases, "script": script,
        "confirm": js::gb(a, "confirm"), "clear": js::gb(a, "clear"), "card": a["card"] != json!(false), "enabled": a["enabled"] != json!(false)
    }))
}

pub struct TerminalActions {
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    actions: RefCell<Vec<J>>,
}

impl TerminalActions {
    pub fn new(file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let me = Self { file, log, actions: RefCell::new(vec![]) };
        me.load();
        me
    }

    pub fn load(&self) {
        let raw = js::read_json(&self.file);
        let source = raw.as_ref().filter(|r| r["actions"].is_array()).map(|r| js::arr(&r["actions"])).unwrap_or_else(defaults);
        let mut seen = HashSet::new();
        let mut actions: Vec<J> = source.iter().filter_map(|a| sanitize(a, &mut seen).ok()).collect();
        let mut migrated = false;
        if let Some(r) = &raw {
            let v = r["version"].as_i64().unwrap_or(1);
            if v < VERSION {
                for (ver, ids) in ADDED_IN {
                    if v >= *ver {
                        continue;
                    }
                    for id in *ids {
                        if actions.iter().any(|a| js::gs(a, "id") == *id) {
                            continue;
                        }
                        if let Some(def) = defaults().into_iter().find(|d| js::gs(d, "id") == *id) {
                            if let Ok(a) = sanitize(&def, &mut seen) {
                                actions.push(a);
                            }
                        }
                    }
                }
                migrated = true;
            }
        }
        *self.actions.borrow_mut() = actions;
        if raw.is_none() || migrated {
            self.persist();
        }
    }

    fn persist(&self) {
        if let Err(e) = js::write_json(&self.file, &json!({ "version": VERSION, "actions": *self.actions.borrow() })) {
            (self.log)(format!("[terminal-actions] enregistrement impossible : {e}"));
        }
    }

    pub fn list(&self) -> Vec<J> {
        self.actions.borrow().clone()
    }

    pub fn get(&self, id: &str) -> Option<J> {
        self.actions.borrow().iter().find(|a| js::gs(a, "id") == id).cloned()
    }

    /// Actions affichées en boutons dans la card du terminal.
    pub fn card_actions(&self) -> Vec<J> {
        self.actions.borrow().iter().filter(|a| js::gb(a, "enabled") && js::gb(a, "card")).cloned().collect()
    }

    /// Remplace tout le catalogue (administration).
    pub fn save(&self, list: &J) -> J {
        let Some(list) = list.as_array() else { return json!({ "ok": false, "errors": ["Catalogue invalide."] }) };
        let mut seen = HashSet::new();
        let mut next = vec![];
        let mut errors = vec![];
        for item in list.iter().take(60) {
            match sanitize(item, &mut seen) {
                Ok(a) => next.push(a),
                Err(e) => errors.push(e),
            }
        }
        if !errors.is_empty() {
            return json!({ "ok": false, "errors": errors });
        }
        *self.actions.borrow_mut() = next;
        self.persist();
        json!({ "ok": true, "errors": [], "actions": self.list() })
    }

    pub fn reset(&self) -> J {
        let mut seen = HashSet::new();
        *self.actions.borrow_mut() = defaults().iter().filter_map(|a| sanitize(a, &mut seen).ok()).collect();
        self.persist();
        json!({ "ok": true, "actions": self.list() })
    }

    /// Phrase dite (repliée) → action, seulement si elle correspond EXACTEMENT à une phrase du catalogue.
    pub fn match_(&self, folded: &str) -> Option<J> {
        let f = crate::re!(r"^jarvis ").replace(folded, "");
        let f = crate::re!(r"(?: (?:jarvis|s il te plait|stp|merci))+$").replace(&f, "").trim().to_string();
        if f.is_empty() {
            return None;
        }
        self.actions.borrow().iter().find(|a| js::gb(a, "enabled") && js::str_list(&a["phrases"]).contains(&f)).cloned()
    }
}
