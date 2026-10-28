// Automatisations « quand X, fais Y » (orchestrator/automations.js).
//   « quand Teams s'ouvre, mets la musique en pause » · « quand je ferme Spotify, baisse le son »
//   « quand je passe sur VS Code, active le mode travail » · « quand j'ouvre YouTube dans Chrome, … »
// Déclencheurs : veille du bureau (application ouverte, fermée, au premier plan) et extension Chrome
// (nouvel onglet). Actions : commandes de Jarvis, locales si possible, sinon par l'IA. Garde-fous : une
// règle ne se relance pas avant 10 s ; une seule règle à la fois. Règles : data/automations.json.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use super::macros::split_steps;
use super::{OrchRef, Up};
use crate::js::{self, J};

const COOLDOWN_MS: i64 = 10000;

fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}

fn label_of(t: &str) -> &str {
    match t {
        "app_opened" => "s’ouvre",
        "app_closed" => "se ferme",
        "foreground" => "passe au premier plan",
        "tab_opened" => "est ouvert dans Chrome",
        _ => t,
    }
}

pub fn base36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = vec![];
    while n > 0 {
        out.push(D[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// Phrase dite → { trigger: { type, target }, steps } ou None.
pub fn parse_rule(raw: &str) -> Option<J> {
    let s = crate::re!(r"(?i)^jarvis[,\s]+").replace(raw.trim(), "");
    let s = crate::re!(r"[.!]+$").replace(&s, "").into_owned();
    if !crate::re!(r"(?i)^quand\s").is_match(&s) || crate::re!(r"(?i)^quand je (?:dis|dirai|te dis|te dirai)\b").is_match(&s) {
        return None;
    }
    let tries: [(&regex::Regex, &str); 7] = [
        (crate::re!(r"(?i)^quand\s+j['’]ouvre\s+(.+?)\s+(?:dans|sur)\s+(?:chrome|le navigateur|google chrome)\s*[,:]?\s+(.+)$"), "tab_opened"),
        (crate::re!(r"(?i)^quand\s+(?:j['’]ouvre|je lance|je démarre|je demarre)\s+(?:l['’]application\s+)?(.+?)\s*[,:]\s*(.+)$"), "app_opened"),
        (crate::re!(r"(?i)^quand\s+(?:l['’]application\s+)?(.+?)\s+(?:s['’]ouvre|démarre|demarre|se lance|est lancée?|est ouverte?|s['’]allume)\s*[,:]?\s+(.+)$"), "app_opened"),
        (crate::re!(r"(?i)^quand\s+je\s+ferme\s+(?:l['’]application\s+)?(.+?)\s*[,:]\s*(.+)$"), "app_closed"),
        (crate::re!(r"(?i)^quand\s+(?:l['’]application\s+)?(.+?)\s+(?:se ferme|est fermée?|s['’]arrête|s['’]arrete)\s*[,:]?\s+(.+)$"), "app_closed"),
        (crate::re!(r"(?i)^quand\s+je\s+(?:passe|reviens|bascule|vais)\s+sur\s+(.+?)\s*[,:]\s*(.+)$"), "foreground"),
        (crate::re!(r"(?i)^quand\s+(.+?)\s+(?:passe|revient|arrive)\s+au premier plan\s*[,:]?\s+(.+)$"), "foreground"),
    ];
    for (re, typ) in tries {
        let Some(m) = re.captures(&s) else { continue };
        let target = crate::re!(r"(?i)^(?:le |la |l['’])").replace(m[1].trim(), "").into_owned();
        let steps = split_steps(&m[2]);
        if target.is_empty() || steps.is_empty() {
            return None;
        }
        return Some(json!({ "trigger": { "type": typ, "target": target }, "steps": steps }));
    }
    None
}

pub struct Automations {
    o: OrchRef,
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    rules: RefCell<Vec<J>>,
    running: Cell<bool>,
    known_tabs: RefCell<Option<HashSet<String>>>,
}

impl Automations {
    pub fn new(o: OrchRef, file: PathBuf, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let rules = js::read_json(&file).and_then(|v| v.as_array().cloned()).unwrap_or_default();
        Rc::new(Self { o, file, log, rules: RefCell::new(rules), running: Cell::new(false), known_tabs: RefCell::new(None) })
    }

    pub fn rules(&self) -> Vec<J> {
        self.rules.borrow().clone()
    }

    fn save(&self) {
        if let Err(e) = js::write_json(&self.file, &Value::Array(self.rules.borrow().clone())) {
            (self.log)(format!("automatisations : {e}"));
        }
    }

    pub fn label(&self, rule: &J) -> String {
        let t = &rule["trigger"];
        format!("Quand {} {} → {}", js::gs(t, "target"), label_of(&js::gs(t, "type")), js::str_list(&rule["steps"]).join(", puis "))
    }

    pub fn add(&self, parsed: &J) -> J {
        let mut rule = json!({ "id": format!("a{}", base36(js::now_ms() as u64)) });
        js::assign(&mut rule, parsed.clone());
        js::assign(&mut rule, json!({ "enabled": true, "createdAt": js::iso_now(), "lastRun": 0, "runs": 0 }));
        self.rules.borrow_mut().push(rule.clone());
        self.save();
        rule
    }

    pub fn remove(&self, id: &str) -> bool {
        let before = self.rules.borrow().len();
        self.rules.borrow_mut().retain(|r| js::gs(r, "id") != id);
        self.save();
        before != self.rules.borrow().len()
    }

    pub fn set_enabled(&self, id: &str, on: bool) -> Option<J> {
        let r = {
            let mut rules = self.rules.borrow_mut();
            let r = rules.iter_mut().find(|r| js::gs(r, "id") == id)?;
            r["enabled"] = json!(on);
            r.clone()
        };
        self.save();
        Some(r)
    }

    // --- Déclenchement ------------------------------------------------------------------------------

    pub fn start(self: &Rc<Self>) {
        let o = self.o.up();
        for ev in ["app_opened", "app_closed", "foreground"] {
            let me = Rc::downgrade(self);
            o.desktop_state.on(ev, move |e| {
                if let Some(me) = me.upgrade() {
                    me.fire(ev, e);
                }
            });
        }
        let me = Rc::downgrade(self);
        o.browser_bridge.on("change", move |_| {
            if let Some(me) = me.upgrade() {
                me.on_tabs();
            }
        });
    }

    fn on_tabs(self: &Rc<Self>) {
        let tabs = self.o.up().browser_bridge.all_tabs();
        // Extension déconnectée : on repart de zéro (sinon, à la reconnexion, tous les onglets seraient « nouveaux »).
        if tabs.is_empty() {
            *self.known_tabs.borrow_mut() = None;
            return;
        }
        let ids: HashSet<String> = tabs.iter().map(|t| js::s(&t["id"])).collect();
        let fresh: Vec<J> = match &*self.known_tabs.borrow() {
            Some(known) => tabs.iter().filter(|t| !known.contains(&js::s(&t["id"])) && js::truthy(&t["url"])).cloned().collect(),
            None => vec![],
        };
        *self.known_tabs.borrow_mut() = Some(ids);
        for t in fresh {
            self.fire("tab_opened", &json!({ "url": t["url"], "title": t["title"] }));
        }
    }

    fn matches(rule: &J, typ: &str, e: &J) -> bool {
        if !js::gb(rule, "enabled") || js::gs(&rule["trigger"], "type") != typ {
            return false;
        }
        let want = fold(&js::gs(&rule["trigger"], "target"));
        if want.is_empty() {
            return false;
        }
        if typ == "tab_opened" {
            let hay = format!("{} {}", fold(&js::gs(e, "url")), fold(&js::gs(e, "title")));
            return hay.contains(&want) || hay.contains(&want.replace(' ', ""));
        }
        let wl = want.chars().count();
        [fold(&js::gs(e, "app")), fold(&js::gs(e, "process"))].iter().filter(|n| !n.is_empty()).any(|n| {
            let nl = n.chars().count();
            *n == want || (wl >= 4 && (n.contains(&want) || (want.contains(n.as_str()) && nl >= 4)))
        })
    }

    pub fn fire(self: &Rc<Self>, typ: &str, e: &J) {
        if self.running.get() {
            return;
        }
        let now = js::now_ms();
        let rule = self.rules.borrow().iter().find(|r| Self::matches(r, typ, e) && now - js::gn_or(r, "lastRun", 0.0) as i64 >= COOLDOWN_MS).cloned();
        if let Some(rule) = rule {
            let me = self.clone();
            let e = e.clone();
            js::spawn(async move { me.run(&rule, &e).await });
        }
    }

    pub async fn run(self: &Rc<Self>, rule: &J, e: &J) {
        self.running.set(true);
        let id = js::gs(rule, "id");
        if let Some(r) = self.rules.borrow_mut().iter_mut().find(|r| js::gs(r, "id") == id) {
            r["lastRun"] = json!(js::now_ms());
            r["runs"] = json!(js::gn_or(r, "runs", 0.0) as i64 + 1);
        }
        self.save();
        let o = self.o.up();
        let label = self.label(rule);
        let trig = if js::truthy(&e["app"]) { js::gs(e, "app") } else { js::gs(e, "url") };
        o.record("automatisation", json!({ "regle": label, "declencheur": trig }));
        (self.log)(format!("automatisation : {label}"));
        for step in js::str_list(&rule["steps"]) {
            match o.router.route(&step, false) {
                Some(it) if !it.ignored => {
                    o.run_intent(it, &step).await;
                }
                _ => o.enqueue_ai(step.clone(), json!({ "background": true })),
            }
        }
        // Laisse retomber les événements provoqués par la règle elle-même (appli ouverte…).
        let me = self.clone();
        js::set_timeout(2000, move || me.running.set(false));
    }
}
