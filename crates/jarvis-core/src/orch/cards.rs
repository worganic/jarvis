// Cards du HUD numérotées, et application rattachée à une card (orchestrator/cards.js).
// Chaque card reçoit un numéro (le plus petit libre) : « ferme la card 2 ». Une card (ou une ligne)
// peut désigner l'application qu'elle représente ; la fenêtre ne reçoit qu'un identifiant opaque.
// Card verrouillée : celle d'une application dont le type est `locked` ne se ferme pas seule.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use serde_json::{json, Map, Value};

use super::{Fut, OrchRef, Up};
use crate::js::{self, J};

/// Numéros par défaut des cards temporaires connues (type 2, de 100 à 199) ; la musique (HUD) a le 100. Les autres cards
/// prennent le premier numéro libre à partir de 120. Modifiables dans l'administration (status.card_numbers).
pub const DEFAULT_SLOTS: [(&str, i64); 9] = [
    ("tool:Terminal", 101), ("mail", 102), ("agenda", 103), ("note", 104), ("meeting", 105),
    ("tool:Mes listes", 106), ("tool:notes", 107), ("tool:reunions", 108), ("tool:projets", 109),
];

/// Numéro par défaut d'un emplacement : celui de la fiche du module (modules/<id>/module.json › card), sinon la valeur de repli.
pub fn default_slot_number(slot: &str) -> Option<i64> {
    crate::modules::slot_default(slot).or_else(|| DEFAULT_SLOTS.iter().find(|s| s.0 == slot).map(|s| s.1))
}

/// Emplacement de numéro d'une card : sa clé, sauf pour les listes dont le titre varie (notes, réunions, projets).
pub fn slot_of(key: &str, data: &J) -> String {
    match js::gs(data, "tool").as_str() {
        t @ ("notes" | "reunions" | "projets") => format!("tool:{t}"),
        _ => key.to_string(),
    }
}

pub fn card_number(spoken: &str) -> Option<i64> {
    let s = spoken.trim();
    if crate::re!(r"^\d{1,3}$").is_match(s) {
        return s.parse().ok();
    }
    Some(match s {
        "un" | "une" | "premiere" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "onze" => 11,
        "douze" => 12,
        _ => return None,
    })
}

/// Actions d'un type d'application : open(app), close(app), alive(app), action(app, action).
pub struct Handler {
    pub locked: bool,
    pub open: Option<Rc<dyn Fn(J) -> Fut<J>>>,
    pub close: Option<Rc<dyn Fn(J) -> Fut<J>>>,
    pub alive: Option<Rc<dyn Fn(J) -> Fut<Option<bool>>>>,
    pub action: Option<Rc<dyn Fn(J, String) -> Fut<J>>>,
}

#[derive(Clone)]
pub struct Card {
    pub n: i64,
    pub key: String,
    /// Emplacement de numéro (clé de la card, ou « tool:notes »… pour les listes à titre variable).
    pub slot: String,
    pub kind: String,
    pub title: String,
    pub app_id: Option<String>,
    pub data: J,
    pub at: i64,
}

pub struct Cards {
    o: OrchRef,
    pub by_key: RefCell<Vec<Card>>, // ordre d'insertion (Map de la v3)
    pub apps: RefCell<Vec<(String, J)>>,
    app_seq: Cell<u64>,
    pub handlers: RefCell<HashMap<String, Rc<Handler>>>,
    /// Dernière annonce vocale d'ouverture ou de fermeture, par card (clé → instant en ms).
    announced: RefCell<HashMap<String, i64>>,
}

/// Une card ouverte ou fermée plusieurs fois de suite n'est annoncée qu'à la première, pendant ce délai.
const ANNOUNCE_QUIET_MS: i64 = 20_000;

impl Cards {
    pub fn new(o: OrchRef) -> Self {
        Self { o, by_key: RefCell::new(vec![]), apps: RefCell::new(vec![]), app_seq: Cell::new(0), handlers: RefCell::new(HashMap::new()), announced: RefCell::new(HashMap::new()) }
    }

    /// Même clé que ui/widgets.js : une card de liste est identifiée par son titre.
    pub fn key_of(kind: &str, data: &J) -> String {
        if kind == "tool" {
            let t = js::gs(data, "title");
            format!("tool:{}", if t.is_empty() { kind.to_string() } else { t })
        } else {
            kind.to_string()
        }
    }

    pub fn has_key(&self, key: &str) -> bool {
        self.by_key.borrow().iter().any(|c| c.key == key)
    }

    pub fn keys(&self) -> Vec<String> {
        self.by_key.borrow().iter().map(|c| c.key.clone()).collect()
    }

    fn next_number(&self) -> i64 {
        let used: HashSet<i64> = self.by_key.borrow().iter().map(|c| c.n).collect();
        // Cards de type 2 (temporaires) : de 100 à 199 ; le 100 est celui de la musique (HUD). Type 1 : de 1 à 99 (fixes).
        let mut n = 120; // 101 à 119 : emplacements des cards connues (DEFAULT_SLOTS)
        while used.contains(&n) {
            n += 1;
        }
        n
    }

    pub fn app(&self, id: &str) -> Option<J> {
        self.apps.borrow().iter().find(|(i, _)| i == id).map(|(_, a)| a.clone())
    }

    pub fn set_app(&self, id: &str, a: J) {
        let mut apps = self.apps.borrow_mut();
        if let Some(e) = apps.iter_mut().find(|(i, _)| i == id) {
            e.1 = a;
        }
    }

    pub fn app_ref(&self, desc: &J) -> Option<String> {
        if !js::truthy(&desc["type"]) {
            return None;
        }
        {
            let mut apps = self.apps.borrow_mut();
            for (id, a) in apps.iter_mut() {
                if a["type"] == desc["type"] && a["target"] == desc["target"] {
                    let agent = if js::truthy(&desc["agentPid"]) { desc["agentPid"].clone() } else if js::truthy(&a["agentPid"]) { a["agentPid"].clone() } else { json!(0) };
                    let mut merged = js::spread(a, desc);
                    merged["agentPid"] = agent;
                    *a = merged;
                    return Some(id.clone());
                }
            }
        }
        let id = format!("app{}", self.app_seq.get() + 1);
        self.app_seq.set(self.app_seq.get() + 1);
        self.apps.borrow_mut().push((id.clone(), js::spread(&json!({ "since": js::now_ms() }), desc)));
        Some(id)
    }

    /// Données d'une card avant envoi aux fenêtres : numéro, applications remplacées par leur identifiant.
    pub fn register(&self, kind: &str, data: &J) -> J {
        if kind == "music" {
            return data.clone();
        }
        let key = Self::key_of(kind, data);
        let prev = self.by_key.borrow().iter().find(|c| c.key == key).map(|c| c.n);
        // Numéro choisi dans l'administration (si aucune autre card ne l'a), sinon celui qu'elle avait, sinon le premier libre.
        let slot = slot_of(&key, data);
        let wanted = self.o.up().status.d.borrow().card_numbers[slot.as_str()].as_i64().or_else(|| default_slot_number(&slot)).filter(|n| !self.by_key.borrow().iter().any(|c| c.n == *n && c.key != key));
        let n = wanted.or(prev).unwrap_or_else(|| self.next_number());
        let mut clean = data.as_object().cloned().unwrap_or_default();
        let app = clean.remove("app").unwrap_or(Value::Null);
        let app_id = self.app_ref(&app);
        let mut out = clean.clone();
        out.insert("cardNo".into(), json!(n));
        out.insert("cardKey".into(), json!(key));
        if let Some(id) = &app_id {
            out.insert("appId".into(), json!(id));
            out.insert("appLabel".into(), json!(js::gs(&app, "label")));
            if self.is_locked(id) {
                out.insert("locked".into(), json!(true));
            }
        }
        if let Some(rows) = data["rows"].as_array() {
            let mapped: Vec<J> = rows
                .iter()
                .map(|r| {
                    if !js::truthy(&r["app"]) {
                        return r.clone();
                    }
                    let mut rest: Map<String, Value> = r.as_object().cloned().unwrap_or_default();
                    let row_app = rest.remove("app").unwrap_or(Value::Null);
                    if let Some(chips) = r["chips"].as_array() {
                        rest.remove("chips");
                        let id = self.app_ref(&row_app);
                        let chips: Vec<J> = chips
                            .iter()
                            .map(|c| {
                                let mut c2 = c.as_object().cloned().unwrap_or_default();
                                let action = c2.remove("action").unwrap_or(Value::Null);
                                c2.insert("call".into(), json!({ "method": "cardAppAction", "args": [id, action] }));
                                Value::Object(c2)
                            })
                            .collect();
                        rest.insert("chips".into(), Value::Array(chips));
                        return Value::Object(rest);
                    }
                    rest.insert("appId".into(), json!(self.app_ref(&row_app)));
                    rest.insert("appLabel".into(), json!(js::gs(&row_app, "label")));
                    Value::Object(rest)
                })
                .collect();
            out.insert("rows".into(), Value::Array(mapped));
        }
        let out = Value::Object(out);
        let title = { let t = js::gs(data, "title"); if t.is_empty() { kind.to_string() } else { t } };
        let card = Card { n, key: key.clone(), slot, kind: kind.to_string(), title, app_id, data: out.clone(), at: js::now_ms() };
        {
            let mut cards = self.by_key.borrow_mut();
            match cards.iter_mut().find(|c| c.key == key) {
                Some(c) => *c = card,
                None => cards.push(card),
            }
        }
        self.prune_apps();
        out
    }

    pub fn is_locked(&self, app_id: &str) -> bool {
        let Some(a) = self.app(app_id) else { return false };
        self.handlers.borrow().get(&js::gs(&a, "type")).map(|h| h.locked).unwrap_or(false)
    }

    /// La fenêtre a fermé la card : son numéro est libre.
    pub fn closed(&self, key: &str) -> J {
        self.by_key.borrow_mut().retain(|c| c.key != key);
        self.prune_apps();
        json!({ "ok": true })
    }

    fn prune_apps(&self) {
        let mut used = HashSet::new();
        for c in self.by_key.borrow().iter() {
            if let Some(id) = &c.app_id {
                used.insert(id.clone());
            }
            for r in js::arr(&c.data["rows"]) {
                if let Some(id) = r["appId"].as_str() {
                    used.insert(id.to_string());
                }
            }
        }
        self.apps.borrow_mut().retain(|(id, _)| used.contains(id));
    }

    /// L'application s'est fermée : sa card disparaît ; dans une liste, seule sa ligne s'en va.
    pub fn app_gone(&self, app_id: &str) {
        self.apps.borrow_mut().retain(|(id, _)| id != app_id);
        for card in self.list() {
            if card.app_id.as_deref() == Some(app_id) {
                self.hide(&card);
                continue;
            }
            let rows = js::arr(&card.data["rows"]);
            if !rows.iter().any(|r| r["appId"].as_str() == Some(app_id)) {
                continue;
            }
            let left: Vec<J> = rows.into_iter().filter(|r| r["appId"].as_str() != Some(app_id)).collect();
            if !left.iter().any(|r| js::truthy(r) && !js::truthy(&r["section"])) {
                self.hide(&card);
                continue;
            }
            let count = left.iter().filter(|r| js::truthy(r) && !js::truthy(&r["section"])).count();
            let mut data = card.data.clone();
            data["rows"] = json!(left);
            data["count"] = json!(count);
            if let Some(c) = self.by_key.borrow_mut().iter_mut().find(|c| c.key == card.key) {
                c.data = data.clone();
            }
            let mut ev = json!({ "type": "widget", "kind": card.kind });
            js::assign(&mut ev, data);
            ev["at"] = json!(js::now_ms());
            self.o.up().emit(ev);
        }
    }

    /// Veille : chaque application suivie tourne-t-elle encore ?
    pub async fn sweep(&self) {
        let apps = self.apps.borrow().clone();
        for (id, a) in apps {
            let h = self.handlers.borrow().get(&js::gs(&a, "type")).cloned();
            let Some(alive) = h.and_then(|h| h.alive.clone()) else { continue };
            if alive(a).await == Some(false) {
                self.app_gone(&id);
            }
        }
    }

    /// Numéros changés dans l'administration : les cards ouvertes les prennent et se redessinent.
    pub fn renumber(&self) {
        let o = self.o.up();
        let numbers = o.status.d.borrow().card_numbers.clone();
        let mut changed: Vec<Card> = vec![];
        {
            let mut cards = self.by_key.borrow_mut();
            for i in 0..cards.len() {
                let Some(n) = numbers[cards[i].slot.as_str()].as_i64().or_else(|| default_slot_number(&cards[i].slot)) else { continue };
                if cards[i].n != n && !cards.iter().any(|c| c.n == n) {
                    cards[i].n = n;
                    cards[i].data["cardNo"] = json!(n);
                    changed.push(cards[i].clone());
                }
            }
        }
        for c in changed {
            let mut ev = json!({ "type": "widget", "kind": c.kind });
            js::assign(&mut ev, c.data.clone());
            ev["at"] = json!(js::now_ms());
            o.emit(ev);
        }
    }

    /// Dire « card 2 » désigne la deuxième card de type 2, de numéro 102 (le numéro complet marche aussi).
    pub fn find(&self, n: i64) -> Option<Card> {
        let n = if (1..100).contains(&n) { n + 100 } else { n };
        self.by_key.borrow().iter().find(|c| c.n == n).cloned()
    }

    pub fn list(&self) -> Vec<Card> {
        let mut l = self.by_key.borrow().clone();
        l.sort_by_key(|c| c.n);
        l
    }

    pub fn hide(&self, card: &Card) {
        self.by_key.borrow_mut().retain(|c| c.key != card.key);
        self.o.up().emit(json!({ "type": "widget_close", "key": card.key }));
    }

    fn handler_for(&self, app_id: &str) -> Option<(J, Rc<Handler>)> {
        let a = self.app(app_id)?;
        let h = self.handlers.borrow().get(&js::gs(&a, "type")).cloned()?;
        Some((a, h))
    }

    pub async fn open_app(&self, app_id: &str) -> J {
        match self.handler_for(app_id).and_then(|(a, h)| h.open.clone().map(|f| (a, f))) {
            Some((a, f)) => f(a).await,
            None => json!({ "ok": false, "say": "Cette application n’est plus suivie." }),
        }
    }

    /// Bouton d'une ligne liée à une application (« 🎙 Dicter ici », « ✋ Jarvis »).
    pub async fn app_action(&self, app_id: &str, action: &str) -> J {
        match self.handler_for(app_id).and_then(|(a, h)| h.action.clone().map(|f| (a, f))) {
            Some((a, f)) => f(a, action.to_string()).await,
            None => json!({ "ok": false, "say": "Cette application n’est plus suivie." }),
        }
    }

    pub async fn close_app(&self, app_id: &str) -> J {
        let Some((a, f)) = self.handler_for(app_id).and_then(|(a, h)| h.close.clone().map(|f| (a, f))) else {
            return json!({ "ok": false, "say": "Cette application n’est plus suivie." });
        };
        let r = f(a).await;
        if js::gb(&r, "ok") {
            self.app_gone(app_id);
        }
        r
    }

    /// Annonce vocale d'une ouverture ou fermeture réussie : la première fois seulement, puis silence pendant 20 s
    /// (la card s'ouvre ou se ferme quand même ; seule la phrase est retirée).
    fn quiet_repeat(&self, key: &str, mut r: J) -> J {
        if js::gb(&r, "ok") && js::truthy(&r["say"]) {
            let now = js::now_ms();
            let mut seen = self.announced.borrow_mut();
            match seen.get(key) {
                Some(t) if now - *t < ANNOUNCE_QUIET_MS => r["say"] = Value::Null,
                _ => {
                    seen.insert(key.to_string(), now);
                }
            }
        }
        r
    }

    /// « Ouvre la card 2 » / clic sur la card : affiche son application.
    pub async fn open(&self, n: i64) -> J {
        let key = self.find(n).map(|c| c.key);
        let r = self.open_inner(n).await;
        match key {
            Some(k) => self.quiet_repeat(&k, r),
            None => r,
        }
    }

    async fn open_inner(&self, n: i64) -> J {
        let Some(card) = self.find(n) else { return json!({ "ok": false, "say": format!("Il n’y a pas de card {n}.") }) };
        let Some(id) = card.app_id else { return json!({ "ok": false, "say": format!("La card {n} n’est liée à aucune application.") }) };
        self.open_app(&id).await
    }

    /// « Ferme la card 2 » / ×. with_app : ferme aussi l'application rattachée (validation comprise).
    pub async fn close(&self, n: i64, with_app: bool) -> J {
        let key = self.find(n).map(|c| c.key);
        let r = self.close_inner(n, with_app).await;
        match key {
            Some(k) => self.quiet_repeat(&k, r),
            None => r,
        }
    }

    async fn close_inner(&self, n: i64, with_app: bool) -> J {
        let Some(card) = self.find(n) else { return json!({ "ok": false, "say": format!("Il n’y a pas de card {n}.") }) };
        let label = card.app_id.as_ref().and_then(|id| self.app(id)).map(|a| js::gs(&a, "label")).filter(|l| !l.is_empty()).unwrap_or_else(|| "son application".into());
        let locked = card.app_id.as_ref().map(|id| self.is_locked(id)).unwrap_or(false);
        if locked && !with_app {
            return json!({ "ok": false, "say": format!("La card {n} reste affichée tant que {label} est ouvert. Dis « ferme la card {n} et son application » pour fermer les deux.") });
        }
        if !locked {
            self.hide(&card);
        }
        let Some(id) = card.app_id.clone().filter(|_| with_app) else { return json!({ "ok": true, "say": format!("Card {n} fermée.") }) };
        let r = self.close_app(&id).await;
        if js::gb(&r, "ok") {
            return json!({ "ok": true, "say": format!("Card {n} et {label} fermées.") });
        }
        let say = js::gs(&r, "say");
        if locked {
            return json!({ "ok": false, "say": if js::gb(&r, "denied") { format!("{label} reste ouvert : la card {n} aussi.") } else if !say.is_empty() { say } else { format!("Je n’ai pas pu fermer {label} : la card {n} reste affichée.") } });
        }
        json!({ "ok": false, "say": if js::gb(&r, "denied") { format!("Card {n} fermée ; {label} reste ouvert.") } else if !say.is_empty() { say } else { format!("Card {n} fermée, mais je n’ai pas pu fermer {label}.") } })
    }

    /// Ferme toutes les cards sauf celles des applications ouvertes (verrouillées).
    pub fn close_all(&self) -> usize {
        let all: Vec<Card> = self.list().into_iter().filter(|c| !c.app_id.as_ref().map(|id| self.is_locked(id)).unwrap_or(false)).collect();
        for c in &all {
            self.hide(c);
        }
        all.len()
    }
}
