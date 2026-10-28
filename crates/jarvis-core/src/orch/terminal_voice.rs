// Voix → terminal (orchestrator/terminal-voice.js) : dictée dans le terminal au premier plan, ou dans un
// Claude de tmux (relais). Chaque phrase s'écrit à la suite, sans jamais appuyer sur Entrée ;
// « je valide le prompt » (seul ou à la fin) appuie sur Entrée. Touches dites : « échap », « contrôle C »…
// « Jarvis, reprends la main » : Jarvis écoute de nouveau. Pendant une réunion enregistrée, le mode est coupé.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use serde_json::{json, Value};

use super::desktop::spoken_punctuation;
use super::terminal_actions::parse_script;
use super::tmux::process_alive;
use super::tmux_voice::{dict_keys, enter_word_re, fold as fold_tmux, validate_re, validate_tail_re};
use super::{OrchRef, Up};
use crate::js::{self, J};

pub fn is_terminal(process: &str) -> bool {
    crate::re!(r"(?i)^(?:windowsterminal|wt|cmd|powershell|pwsh|conhost|openconsole|wezterm-gui|alacritty|mintty|tabby|hyper|warp)$").is_match(process)
}

fn release_re() -> &'static regex::Regex {
    crate::re!(r"^(?:reprends? la main|rends? moi la main|quitte le terminal|sors du terminal|arrete d ecrire(?: dans le terminal)?|stop le terminal|fin du terminal|mode normal)$")
}
fn answer_re() -> &'static regex::Regex {
    crate::re!(r"^(?:oui|non|vas y|d accord|valide|je valide|confirme|accepte|c est bon|ok|annule|refuse|laisse tomber)\b")
}
fn digits_re() -> &'static regex::Regex {
    crate::re!(r"^(?:\d|zero|un|une|deux|trois|quatre|cinq|six|sept|huit|neuf| )+$")
}
fn yes_re() -> &'static regex::Regex {
    crate::re!(r"^(?:oui|vas y|d accord|confirme|confirmer|je confirme|c est bon|ok|valide|je valide)$")
}
fn no_re() -> &'static regex::Regex {
    crate::re!(r"^(?:non|annule|annuler|laisse tomber|stop|pas maintenant)$")
}

/// Touches tmux → SendKeys, pour un terminal hors tmux.
fn sendkeys(k: &str) -> &'static str {
    match k {
        "Enter" => "{ENTER}",
        "Escape" => "{ESC}",
        "C-c" => "^c",
        "C-d" => "^d",
        "Tab" => "{TAB}",
        "BTab" => "+{TAB}",
        "Up" => "{UP}",
        "Down" => "{DOWN}",
        "Left" => "{LEFT}",
        "Right" => "{RIGHT}",
        "C-u" => "{ESC}",
        _ => "",
    }
}

fn key_name(k: &str) -> String {
    match k {
        "Enter" => "Entrée",
        "Escape" => "Échap",
        "C-c" => "Ctrl+C",
        "C-d" => "Ctrl+D",
        "Tab" => "Tab",
        "BTab" => "Maj+Tab",
        "Up" => "↑",
        "Down" => "↓",
        "Left" => "←",
        "Right" => "→",
        "C-u" => "effacer la ligne",
        other => other,
    }
    .to_string()
}

const CARD_TITLE: &str = "Terminal";
const HINT: &str = "Pour envoyer, dis « je valide le prompt ».";
const CONFIRM_MS: i64 = 30000;
const RECENT_MS: i64 = 15000;

fn dest_key(d: &J) -> String {
    if js::gs(d, "kind") == "tmux" { format!("tmux:{}", js::gs(d, "target")) } else { format!("hwnd:{}", js::s(&d["hwnd"])) }
}

#[derive(Clone, Debug)]
pub struct TvTarget {
    pub kind: String,
    pub hwnd: i64,
    pub process: String,
    pub session: String,
    pub label: String,
}

#[derive(Clone)]
struct Awaiting {
    id: String,
    label: String,
    at: i64,
}

pub struct TerminalVoice {
    o: OrchRef,
    pub target: RefCell<Option<TvTarget>>,
    key: RefCell<String>,
    paused_hwnd: Cell<i64>,
    sent: Cell<u32>,
    /// Début de session de dictée pas encore journalisé : il ne l'est qu'à la première phrase écrite.
    debut: RefCell<Option<(String, bool)>>,
    /// Card « Terminal » unique : affichée seulement tant que Claude Code ou AGY tourne dans un terminal.
    card_shown: Cell<bool>,
    /// Fermée par l'utilisateur : elle ne revient qu'au prochain lancement d'un agent.
    card_dismissed: Cell<bool>,
    /// Dernière fenêtre de terminal représentée par la card.
    card_hwnd: Cell<i64>,
    typed: RefCell<HashSet<String>>,
    last_target: RefCell<Option<(TvTarget, i64)>>,
    awaiting: RefCell<HashMap<String, Awaiting>>,
    /// Bouton 🎙 de la card : tant qu'il est actif, ce qui est dit s'écrit dans le terminal au premier plan.
    /// Coupé au démarrage : la dictée dans le terminal ne s'active que sur demande (bouton 🎙).
    armed: Cell<bool>,
}

fn say_to(o: &super::Orchestrator, text: &str) {
    if !text.is_empty() {
        o.emit(json!({ "type": "say", "text": text }));
    }
}

impl TerminalVoice {
    pub fn new(o: OrchRef) -> Rc<Self> {
        Rc::new(Self {
            o,
            target: RefCell::new(None),
            key: RefCell::new(String::new()),
            paused_hwnd: Cell::new(0),
            sent: Cell::new(0),
            debut: RefCell::new(None),
            card_shown: Cell::new(false),
            card_dismissed: Cell::new(false),
            card_hwnd: Cell::new(0),
            typed: RefCell::new(HashSet::new()),
            last_target: RefCell::new(None),
            awaiting: RefCell::new(HashMap::new()),
            armed: Cell::new(false),
        })
    }

    /// Session tmux du terminal au premier plan, s'il en montre une.
    pub fn target_tmux_session(&self) -> Option<String> {
        self.target.borrow().as_ref().filter(|t| t.kind == "tmux").map(|t| t.session.clone())
    }

    pub fn watch(self: &Rc<Self>) {
        let o = self.o.up();
        for ev in ["foreground", "change"] {
            let me = Rc::downgrade(self);
            o.desktop_state.on(ev, move |_| {
                if let Some(me) = me.upgrade() {
                    me.refresh();
                }
            });
        }
    }

    fn enabled(&self) -> bool {
        let o = self.o.up();
        if !self.armed.get() {
            return false;
        }
        if o.tmux.cfg()["terminalVoice"] == json!(false) {
            return false;
        }
        !o.meetings.recording() // en réunion, on parle aux participants
    }

    /// Fenêtre au premier plan, si c'est un terminal.
    fn detect(&self) -> Option<TvTarget> {
        let o = self.o.up();
        let w = o.desktop_state.foreground()?;
        let process = js::gs(&w, "process");
        if !is_terminal(&process) {
            return None;
        }
        let hwnd = w["hwnd"].as_i64().unwrap_or(0);
        let title = js::gs(&w, "title");
        if let Some(m) = crate::re!(r"^tmux · (.+)$").captures(&title) {
            let s = m[1].trim().to_string();
            return Some(TvTarget { kind: "tmux".into(), hwnd, process, label: format!("tmux · {s}"), session: s });
        }
        Some(TvTarget { kind: "console".into(), hwnd, label: super::desktop_state::app_name(&process), process, session: String::new() })
    }

    pub fn refresh(&self) -> Option<TvTarget> {
        let found = if self.enabled() { self.detect() } else { None };
        if self.paused_hwnd.get() != 0 && found.as_ref().map(|f| f.hwnd != self.paused_hwnd.get()).unwrap_or(true) {
            self.paused_hwnd.set(0);
        }
        let want = found.filter(|f| f.hwnd != self.paused_hwnd.get() || self.paused_hwnd.get() == 0);
        let key = want.as_ref().map(|w| format!("{}|{}|{}", w.kind, w.hwnd, w.session)).unwrap_or_default();
        if key == *self.key.borrow() {
            return self.target.borrow().clone();
        }
        let prev = self.target.borrow().clone();
        match (&prev, &want) {
            (Some(p), None) => *self.last_target.borrow_mut() = Some((p.clone(), js::now_ms())),
            (_, Some(_)) => *self.last_target.borrow_mut() = None,
            _ => {}
        }
        *self.key.borrow_mut() = key;
        *self.target.borrow_mut() = want.clone();
        let o = self.o.up();
        o.emit(json!({ "type": "terminal_voice", "on": want.is_some(), "label": want.as_ref().map(|w| w.label.clone()).unwrap_or_default() }));
        // Titre de fenêtre jamais journalisé : seulement le programme.
        // Journal : seules les sessions où une phrase a été écrite comptent (avant, 250 sur 253 étaient vides).
        match &want {
            Some(w) => *self.debut.borrow_mut() = Some((w.process.clone(), w.kind == "tmux")),
            None => {
                if self.sent.get() > 0 {
                    o.record("terminal_voix_fin", json!({ "phrases": self.sent.get(), "devant": self.front_program() }));
                }
                *self.debut.borrow_mut() = None;
            }
        };
        if want.is_some() {
            self.sent.set(0);
        }
        self.refresh_cards();
        o.tmux_voice.refresh_cards();
        want
    }

    fn front_program(&self) -> String {
        self.o.up().desktop_state.foreground().map(|w| js::gs(&w, "process")).unwrap_or_default()
    }

    /// « ✋ Jarvis » / « Jarvis, reprends la main » : ce terminal est ignoré tant qu'il reste devant.
    pub fn pause(&self) {
        if let Some(t) = self.target.borrow().as_ref() {
            self.paused_hwnd.set(t.hwnd);
        }
        self.refresh();
    }

    // --- Card du terminal ----------------------------------------------------------------------------

    /// Fenêtre de terminal d'un titre « tmux · session » : le nom de la session.
    fn session_of_hwnd(&self, hwnd: i64) -> Option<String> {
        if hwnd == 0 {
            return None;
        }
        let w = self.o.up().desktop_state.windows().into_iter().find(|w| w["hwnd"].as_i64() == Some(hwnd))?;
        crate::re!(r"^tmux · (.+)$").captures(&js::gs(&w, "title")).map(|m| m[1].trim().to_string())
    }

    /// Fenêtre de terminal que la card représente (dictée, actions) : celle au premier plan, sinon la
    /// précédente, sinon celle dont le titre évoque un agent, sinon la première.
    fn pick_terminal(&self) -> Option<(i64, String)> {
        let o = self.o.up();
        let wins: Vec<J> = o.desktop_state.windows().into_iter().filter(|w| is_terminal(&js::gs(w, "process"))).collect();
        let pick = |w: &J| (w["hwnd"].as_i64().unwrap_or(0), super::desktop_state::app_name(&js::gs(w, "process")));
        let by_hwnd = |h: i64| wins.iter().find(|w| w["hwnd"].as_i64() == Some(h));
        if let Some(w) = self.target.borrow().as_ref().and_then(|t| by_hwnd(t.hwnd)) {
            return Some(pick(w));
        }
        if let Some(w) = by_hwnd(self.card_hwnd.get()) {
            return Some(pick(w));
        }
        let agentish = crate::re!(r"(?i)claude|agy|antigravity|✳");
        wins.iter().find(|w| agentish.is_match(&js::gs(w, "title"))).or(wins.first()).map(pick)
    }

    /// Card « Terminal » : une seule, présente tant qu'un agent (Claude Code, AGY) tourne dans un terminal.
    fn update_card(&self) {
        let o = self.o.up();
        let (claude, agy) = o.ai_watch.running();
        let key = format!("tool:{CARD_TITLE}");
        if claude + agy == 0 {
            if self.card_shown.get() {
                match o.cards.list().into_iter().find(|c| c.key == key) {
                    Some(c) => o.cards.hide(&c),
                    None => o.emit(json!({ "type": "widget_close", "key": key })),
                }
            }
            self.card_shown.set(false);
            self.card_dismissed.set(false);
            return;
        }
        if self.card_shown.get() && !o.cards.has_key(&key) {
            self.card_shown.set(false);
            self.card_dismissed.set(true);
        }
        if self.card_dismissed.get() {
            return;
        }
        self.draw_card();
        self.card_shown.set(true);
    }

    /// Bouton ✕ de la card : elle se retire sans fermer la fenêtre du terminal.
    pub fn dismiss_card(&self) {
        self.card_dismissed.set(true);
        self.card_shown.set(false);
    }

    fn draw_card(&self) {
        let o = self.o.up();
        let term = self.pick_terminal();
        if let Some((h, _)) = &term {
            self.card_hwnd.set(*h);
        }
        let app = match &term {
            Some((h, name)) => json!({ "type": "terminal_ai", "target": format!("hwnd:{h}"), "hwnd": h, "label": "cette card", "terminal": name }),
            None => json!({ "type": "terminal_ai", "target": "hwnd:ai_watch", "hwnd": "ai_watch", "label": "cette card", "terminal": "Terminal" }),
        };
        let mut rows = o.ai_watch.status_rows();
        if term.is_some() {
            rows.extend(self.action_rows(&app));
        }
        let (badge, tone) = o.ai_watch.badge();
        o.show_widget(
            "tool",
            json!({
                "title": CARD_TITLE, "tool": "terminal", "persist": true, "foldable": true, "badge": badge, "badgeTone": tone,
                "app": app, "rows": rows, "dictate": { "active": self.armed.get() },
                "footer": "« je valide le prompt » : Entrée · « échap », « contrôle C » · ✋ : Jarvis reprend la main · Actions : Administration › Terminal"
            }),
        );
    }

    /// Met à jour la card TERMINAL (agents lancés ou état modifié).
    pub fn update_ai_status(&self) {
        self.update_card();
    }

    /// Lignes d'état de la surveillance de Claude Code et AGY.
    pub fn ai_watch_rows(&self) -> Vec<J> {
        self.o.up().ai_watch.status_rows()
    }

    pub fn refresh_cards(&self) {
        self.update_card();
    }

    /// Veille des cards : la fenêtre existe-t-elle encore ?
    pub fn alive(&self, _app: &J) -> Option<bool> {
        let (claude, agy) = self.o.up().ai_watch.running();
        Some(claude + agy > 0)
    }

    /// Boutons de la card : « 🎙 Dicter ici » et « ✋ Jarvis ».
    pub async fn card_action(&self, app: &J, action: &str) -> J {
        let o = self.o.up();
        if crate::re!(r"^(?:act|confirm):").is_match(action) {
            return self.card_action_button(app, action).await;
        }
        let hwnd_s = js::s(&app["hwnd"]);
        // Bouton 🎙 de l'en-tête : active / coupe la dictée dans le terminal au premier plan (sans en choisir un).
        if action == "dictee" {
            self.armed.set(!self.armed.get());
            self.refresh();
            self.draw_card();
            say_to(&o, if self.armed.get() { "Dictée dans le terminal activée : quand un terminal est devant, ce que tu dis y est écrit." } else { "Dictée dans le terminal coupée." });
            return json!({ "ok": true });
        }
        if action == "dicter" {
            if hwnd_s == "ai_watch" {
                say_to(&o, "Ouvre un terminal pour y dicter du texte.");
                return json!({ "ok": true });
            }
            let hwnd = app["hwnd"].as_i64().unwrap_or(0);
            if self.paused_hwnd.get() == hwnd {
                self.paused_hwnd.set(0);
            }
            let r = o.desktop_action(hwnd, "afficher").await;
            self.refresh();
            if r["ok"] == json!(false) {
                say_to(&o, &if js::truthy(&r["say"]) { js::gs(&r, "say") } else { "Je n’ai pas pu afficher ce terminal.".into() });
            } else {
                say_to(&o, &format!("Je t’écoute : ce que tu dis est écrit dans le terminal. {HINT}"));
            }
            return json!({ "ok": true });
        }
        if action == "reprendre" {
            let is_target = self.target.borrow().as_ref().map(|t| t.hwnd.to_string() == hwnd_s).unwrap_or(false);
            if is_target {
                self.pause();
            } else if o.tmux_voice.relay.borrow().is_some() {
                o.tmux_voice.stop_relay("bouton");
            }
            self.refresh_cards();
            say_to(&o, "Je reprends la main.");
            return json!({ "ok": true });
        }
        json!({ "ok": false })
    }

    // --- Actions du catalogue ------------------------------------------------------------------------

    fn pending_for(&self, app: &J) -> Option<(String, Awaiting)> {
        let now = js::now_ms();
        let typ = js::gs(app, "type");
        for (k, p) in self.awaiting.borrow().iter() {
            if now - p.at > CONFIRM_MS {
                continue;
            }
            if typ == "terminal" || typ == "terminal_ai" {
                let hw = js::s(&app["hwnd"]);
                let hit = match self.session_of_hwnd(app["hwnd"].as_i64().unwrap_or(0)) {
                    Some(session) => k.starts_with(&format!("tmux:{session}:")),
                    None => *k == format!("hwnd:{hw}"),
                };
                if hit {
                    return Some((k.clone(), p.clone()));
                }
            } else if typ == "tmux" {
                let t = js::gs(app, "target");
                if *k == format!("tmux:{t}") || k.starts_with(&format!("tmux:{t}.")) || t.starts_with(&format!("{}.", &k[5.min(k.len())..])) {
                    return Some((k.clone(), p.clone()));
                }
            }
        }
        None
    }

    /// Lignes de la card : boutons des actions, ou la demande de confirmation d'une action sensible.
    pub fn action_rows(&self, app: &J) -> Vec<J> {
        let o = self.o.up();
        if let Some((_, p)) = self.pending_for(app) {
            return vec![json!({
                "icon": "⚠", "title": format!("{} ?", p.label), "tone": "warn", "app": app, "sub": "Dis « oui », ou confirme ici (annulé au bout de 30 s)",
                "chips": [{ "icon": "✔", "label": "Confirmer", "action": "confirm:oui", "tone": "ok" }, { "icon": "✖", "label": "Annuler", "action": "confirm:non" }]
            })];
        }
        let list = o.terminal_actions.card_actions();
        if list.is_empty() {
            return vec![];
        }
        let chips: Vec<J> = list
            .iter()
            .map(|a| {
                let first = js::str_list(&a["phrases"]).into_iter().next();
                json!({
                    "icon": a["icon"], "label": a["label"], "action": format!("act:{}", js::gs(a, "id")),
                    "title": format!("{}{}", first.map(|p| format!("Dire : « {p} » · ")).unwrap_or_default(), if js::gb(a, "confirm") { "demande une confirmation" } else { "clic : lancer" })
                })
            })
            .collect();
        vec![json!({ "icon": "⚡", "title": "Actions", "app": app, "chips": chips, "actionList": true })]
    }

    async fn dest_for_app(&self, app: &J) -> Option<J> {
        if js::gs(app, "type") == "tmux" {
            return Some(json!({ "kind": "tmux", "target": app["target"], "label": app["label"], "agent": true }));
        }
        if let Some(session) = self.session_of_hwnd(app["hwnd"].as_i64().unwrap_or(0)) {
            let (target, agent) = self.tmux_pane(&session).await?;
            return Some(json!({
                "kind": "tmux", "target": target,
                "label": format!("tmux · {session}{}", agent.as_ref().map(|a| format!(" · {}", js::gs(a, "label"))).unwrap_or_default()),
                "agent": agent.is_some()
            }));
        }
        let name = if js::truthy(&app["terminal"]) { js::gs(app, "terminal") } else { js::gs(app, "label") };
        Some(json!({ "kind": "console", "hwnd": app["hwnd"], "label": name, "agent": false }))
    }

    pub fn refresh_all_cards(&self) {
        self.refresh_cards();
        self.o.up().tmux_voice.refresh_cards();
    }

    /// Bouton de la card : « act:<id> » lance l'action, « confirm:oui|non » répond à la confirmation.
    pub async fn card_action_button(&self, app: &J, action: &str) -> J {
        let o = self.o.up();
        let Some(dest) = self.dest_for_app(app).await else {
            say_to(&o, "Ce terminal ne répond plus.");
            return json!({ "ok": false });
        };
        let mut confirmed = false;
        let act = if let Some(rest) = action.strip_prefix("confirm:") {
            let Some((k, p)) = self.pending_for(app) else {
                self.refresh_all_cards();
                say_to(&o, "Plus rien à confirmer.");
                return json!({ "ok": false });
            };
            self.awaiting.borrow_mut().remove(&k);
            if rest == "non" {
                self.refresh_all_cards();
                say_to(&o, "Annulé.");
                return json!({ "ok": true });
            }
            confirmed = true;
            o.terminal_actions.get(&p.id)
        } else {
            o.terminal_actions.get(action.strip_prefix("act:").unwrap_or(""))
        };
        let Some(act) = act.filter(|a| js::gb(a, "enabled")) else {
            say_to(&o, "Cette action n’existe plus.");
            return json!({ "ok": false });
        };
        if js::gb(&act, "confirm") && !confirmed {
            return self.run_action(&dest, &act, false, false).await;
        }
        // Terminal hors tmux : il doit être devant pour recevoir les touches.
        if js::gs(&dest, "kind") == "console" {
            let r = o.desktop_action(dest["hwnd"].as_i64().unwrap_or(0), "afficher").await;
            if r["ok"] == json!(false) {
                say_to(&o, &if js::truthy(&r["say"]) { js::gs(&r, "say") } else { "Je n’ai pas pu afficher ce terminal.".into() });
                return json!({ "ok": false });
            }
            js::sleep_ms(350).await;
        }
        self.run_action(&dest, &act, false, confirmed).await
    }

    /// Exécute une action du catalogue sur `dest`. Action sensible : demande d'abord « oui ».
    pub async fn run_action(&self, dest: &J, act: &J, doubt: bool, confirmed: bool) -> J {
        let o = self.o.up();
        let key = dest_key(dest);
        if doubt {
            say_to(&o, "Je ne suis pas sûr que ce soit ta voix : action ignorée. Clique sur le bouton de la card, ou appuie au clavier.");
            return json!({ "ok": false });
        }
        let label = js::gs(act, "label");
        if js::gb(act, "confirm") && !confirmed {
            let at = js::now_ms();
            self.awaiting.borrow_mut().insert(key.clone(), Awaiting { id: js::gs(act, "id"), label: label.clone(), at });
            let oc = self.o.clone();
            let k2 = key.clone();
            js::set_timeout((CONFIRM_MS + 200) as u64, move || {
                let orch = oc.up();
                let tv = &orch.terminal_voice;
                let same = tv.awaiting.borrow().get(&k2).map(|a| a.at == at).unwrap_or(false);
                if same {
                    tv.awaiting.borrow_mut().remove(&k2);
                    tv.refresh_all_cards();
                }
            });
            self.refresh_all_cards();
            say_to(&o, &format!("{label} ? Dis « oui », ou clique sur Confirmer."));
            return json!({ "ok": true, "asked": true });
        }
        let (mut steps, errors) = parse_script(&js::gs(act, "script"));
        if !errors.is_empty() || steps.is_empty() {
            say_to(&o, &format!("L’action « {label} » est mal définie : corrige-la dans l’administration."));
            return json!({ "ok": false });
        }
        // Texte dicté en attente : effacé avant la commande, sinon les deux se collent.
        if js::gb(act, "clear") && self.typed.borrow().contains(&key) {
            steps.insert(0, json!({ "key": "C-u" }));
        }
        self.awaiting.borrow_mut().remove(&key);
        let mut typed_last = false;
        for i in 0..steps.len() {
            let st = &steps[i];
            let r = if let Some(w) = st["wait"].as_i64() {
                js::sleep_ms(w as u64).await;
                json!({ "ok": true })
            } else if let Some(text) = st["text"].as_str() {
                let r = self.type_text(dest, text).await;
                typed_last = true;
                if js::gb(&r, "ok") && steps.get(i + 1).map(|n| n["key"].is_string()).unwrap_or(false) {
                    js::sleep_ms(180).await;
                }
                r
            } else if let Some(mode) = st["mode"].as_str() {
                self.cycle_mode(dest, mode).await
            } else if st["key"] == json!("Enter") {
                typed_last = false;
                self.validate(dest).await
            } else if let Some(k) = st["key"].as_str() {
                if k == "C-u" || k == "C-c" {
                    typed_last = false;
                }
                self.press_keys(dest, &[k]).await
            } else {
                json!({ "ok": true })
            };
            if !js::gb(&r, "ok") {
                say_to(&o, &js::gs(&r, "say"));
                self.refresh_all_cards();
                return r;
            }
        }
        if typed_last {
            self.typed.borrow_mut().insert(key);
        } else {
            self.typed.borrow_mut().remove(&key);
        }
        o.record("terminal_action", json!({ "action": act["id"], "tmux": js::gs(dest, "kind") == "tmux" }));
        o.usage_daily.add_local("local");
        self.refresh_all_cards();
        json!({ "ok": true, "action": act["id"] })
    }

    // --- Modes de Claude Code (Maj+Tab) --------------------------------------------------------------

    async fn screen_text(&self, dest: &J) -> String {
        let o = self.o.up();
        if js::gs(dest, "kind") == "tmux" {
            return o.tmux.capture_screen(&js::gs(dest, "target")).await.unwrap_or_default();
        }
        match crate::orch::system::call("lire_ecran_ocr", &json!({})).await {
            Some(Ok(r)) => js::gs(&r, "texte"),
            _ => String::new(),
        }
    }

    /// Mode affiché par la barre d'état de Claude Code (le signe le plus bas dans le texte l'emporte).
    pub fn detect_mode(text: &str) -> Option<&'static str> {
        let mut best = None;
        let mut at: i64 = -1;
        for (mode, re) in [("bypass", crate::re!(r"(?i)bypass\s+permissions?")), ("plan", crate::re!(r"(?i)plan\s+mode")), ("edits", crate::re!(r"(?i)accept\s+edits?"))] {
            for m in re.find_iter(text) {
                if m.start() as i64 > at {
                    at = m.start() as i64;
                    best = Some(mode);
                }
            }
        }
        if best.is_some() {
            return best;
        }
        crate::re!(r"(?i)\? for shortcuts|esc to interrupt|shift\+tab to cycle").is_match(text).then_some("normal")
    }

    async fn cycle_mode(&self, dest: &J, target: &str) -> J {
        let o = self.o.up();
        let name = |m: &str| match m {
            "plan" => "mode plan",
            "bypass" => "mode bypass permissions",
            "normal" => "mode normal",
            _ => "mode édition automatique",
        };
        let mut current = Self::detect_mode(&self.screen_text(dest).await);
        if current == Some(target) {
            say_to(&o, &format!("Déjà en {}.", name(target)));
            return json!({ "ok": true });
        }
        if current.is_none() {
            let r = self.press_keys(dest, &["BTab"]).await;
            if js::gb(&r, "ok") {
                say_to(&o, "Je ne vois pas la barre d’état de Claude Code : j’ai appuyé une fois sur Maj+Tab.");
            }
            return r;
        }
        for _ in 0..5 {
            let r = self.press_keys(dest, &["BTab"]).await;
            if !js::gb(&r, "ok") {
                return r;
            }
            js::sleep_ms(450).await;
            current = Self::detect_mode(&self.screen_text(dest).await);
            if current == Some(target) {
                say_to(&o, &format!("{} activé.", js::capitalize(name(target))));
                return json!({ "ok": true });
            }
            if current.is_none() {
                break;
            }
        }
        json!({
            "ok": false,
            "say": if target == "bypass" { "Le mode bypass permissions n’est pas disponible dans cette session : Claude Code doit être lancé avec l’option bypass permissions.".to_string() } else { format!("Je n’ai pas réussi à passer en {}.", name(target)) }
        })
    }

    // --- Dictée --------------------------------------------------------------------------------------

    /// Phrase dite pendant que le terminal est devant : true si elle a été traitée ici. rejected : voix
    /// non reconnue, le texte est écrit mais rien n'est envoyé (ni Entrée, ni touche, ni action).
    pub async fn handle(&self, clean: &str, doubt: bool, rejected: bool) -> bool {
        let Some(t) = self.refresh() else { return false };
        let o = self.o.up();
        let f = fold_tmux(clean);
        if rejected {
            let raw = crate::re!(r"(?i)^jarvis[\s,]+").replace(clean.trim(), "").into_owned();
            let sends = validate_re().is_match(&f) || enter_word_re().is_match(&f) || validate_tail_re().is_match(&raw)
                || dict_keys().into_iter().any(|(re, _)| re.is_match(&f)) || o.terminal_actions.match_(&f).is_some();
            if sends {
                say_to(&o, "Je ne reconnais pas bien ta voix : je n’envoie rien. Appuie sur Entrée au clavier.");
                return true;
            }
        }
        if release_re().is_match(&f) {
            self.pause();
            let n = self.sent.get();
            let s = if n > 1 { "s" } else { "" };
            say_to(&o, &format!("Je reprends la main.{}", if n > 0 { format!(" {n} phrase{s} écrite{s} dans le terminal.") } else { String::new() }));
            return true;
        }
        if self.for_jarvis(clean, &f) {
            return false;
        }
        let Some(dest) = self.dest_of(&t).await else {
            say_to(&o, &format!("La session tmux {} ne répond pas.", t.session));
            return true;
        };
        let r = self.dictate(&dest, clean, doubt).await;
        if js::gb(&r, "typed") {
            if let Some((programme, tmux)) = self.debut.borrow_mut().take() {
                o.record("terminal_voix_debut", json!({ "programme": programme, "tmux": tmux }));
            }
            self.sent.set(self.sent.get() + 1);
        }
        o.usage_daily.add_local("local");
        true
    }

    /// Réglage de l'écoute ou réponse à une validation de Jarvis en attente : la phrase va à Jarvis.
    fn for_jarvis(&self, clean: &str, f: &str) -> bool {
        if validate_re().is_match(f) {
            return false;
        }
        let o = self.o.up();
        if o.status.parse(clean).map(|s| js::gs(&s, "kind").contains("listening_mode")).unwrap_or(false) {
            return true;
        }
        !o.pending.borrow().is_empty() && (answer_re().is_match(f) || digits_re().is_match(f))
    }

    async fn dest_of(&self, t: &TvTarget) -> Option<J> {
        if t.kind != "tmux" {
            return Some(json!({ "kind": "console", "hwnd": t.hwnd, "label": t.label, "agent": false }));
        }
        let (target, agent) = self.tmux_pane(&t.session).await?;
        Some(json!({
            "kind": "tmux", "target": target,
            "label": format!("{}{}", t.label, agent.as_ref().map(|a| format!(" · {}", js::gs(a, "label"))).unwrap_or_default()),
            "agent": agent.is_some()
        }))
    }

    /// Volet actif de la session tmux, et l'agent qui y tourne s'il y en a un.
    async fn tmux_pane(&self, session: &str) -> Option<(String, Option<J>)> {
        let o = self.o.up();
        let panes = o.tmux.list_panes().await.ok()?;
        let pane = panes.iter().find(|p| js::gs(p, "session") == session && js::gb(p, "active")).or_else(|| panes.iter().find(|p| js::gs(p, "session") == session))?;
        let target = js::gs(pane, "target");
        let agents = o.tmux_voice.discover(10000).await;
        let agent = agents.into_iter().find(|a| {
            let pid = a["agentPid"].as_u64().unwrap_or(0) as u32;
            js::gs(a, "target") == target && (pid == 0 || process_alive(pid))
        });
        Some((target, agent))
    }

    /// Une phrase dictée vers une destination. Retourne { ok, typed, validated }.
    pub async fn dictate(&self, dest: &J, clean: &str, doubt: bool) -> J {
        let o = self.o.up();
        let key = dest_key(dest);
        let f = fold_tmux(clean);
        // Actions du catalogue : « oui » à une confirmation en attente, puis phrase exacte d'une action.
        let pend = self.awaiting.borrow().get(&key).cloned();
        if let Some(p) = pend.filter(|p| js::now_ms() - p.at < CONFIRM_MS && (yes_re().is_match(&f) || no_re().is_match(&f))) {
            if no_re().is_match(&f) {
                self.awaiting.borrow_mut().remove(&key);
                self.refresh_all_cards();
                say_to(&o, "Annulé.");
                return json!({ "ok": true });
            }
            if doubt {
                say_to(&o, "Je ne suis pas sûr que ce soit ta voix : rien n’est lancé. Clique sur Confirmer.");
                return json!({ "ok": false });
            }
            if let Some(act) = o.terminal_actions.get(&p.id) {
                return self.run_action(dest, &act, false, true).await;
            }
        }
        if let Some(hit) = o.terminal_actions.match_(&f) {
            return self.run_action(dest, &hit, doubt, false).await;
        }
        // « Entrée », « je valide » seuls.
        if enter_word_re().is_match(&f) {
            if !self.typed.borrow().contains(&key) {
                say_to(&o, HINT);
                return json!({ "ok": true });
            }
            return self.send_pending(dest, &key, doubt).await;
        }
        if let Some((_, keys)) = dict_keys().into_iter().find(|(re, _)| re.is_match(&f)) {
            if doubt {
                say_to(&o, "Je ne suis pas sûr que ce soit ta voix : touche ignorée. Appuie au clavier.");
                return json!({ "ok": false });
            }
            let r = self.press_keys(dest, keys).await;
            if !js::gb(&r, "ok") {
                say_to(&o, &js::gs(&r, "say"));
            } else if keys.contains(&"C-c") || keys.contains(&"C-u") {
                self.typed.borrow_mut().remove(&key);
            }
            return r;
        }
        // « Je valide le prompt », seul ou à la fin de la phrase.
        let mut raw = crate::re!(r"(?i)^jarvis[\s,]+").replace(clean.trim(), "").into_owned();
        let mut validate = validate_re().is_match(&f);
        if validate {
            raw.clear();
        } else if let Some(m) = validate_tail_re().find(&raw) {
            if m.start() > 0 {
                validate = true;
                raw.truncate(m.start());
            }
        }
        let mut text = if raw.is_empty() { String::new() } else { spoken_punctuation(&raw).trim().to_string() };
        // Shell : une phrase dite finit souvent par un point (Whisper), pas une ligne de commande.
        if !text.is_empty() && !js::gb(dest, "agent") && text.ends_with('.') {
            text.pop();
        }
        let mut typed = false;
        if !text.is_empty() {
            let t = if self.typed.borrow().contains(&key) { format!(" {text}") } else { text };
            let r = self.type_text(dest, &t).await;
            if !js::gb(&r, "ok") {
                say_to(&o, &js::gs(&r, "say"));
                return r;
            }
            self.typed.borrow_mut().insert(key.clone());
            typed = true;
        }
        if !validate {
            return json!({ "ok": true, "typed": typed });
        }
        let mut r = self.send_pending(dest, &key, doubt).await;
        r["typed"] = json!(typed);
        r
    }

    /// Entrée sur du texte dicté. Voix douteuse : acceptée seulement si du texte dicté attend.
    async fn send_pending(&self, dest: &J, key: &str, doubt: bool) -> J {
        let o = self.o.up();
        if doubt {
            if !self.typed.borrow().contains(key) {
                say_to(&o, "Je ne suis pas sûr que ce soit ta voix : je n’envoie pas. Redis « je valide le prompt », ou appuie sur Entrée.");
                return json!({ "ok": true });
            }
            o.record("terminal_validation_douteuse", json!({ "tmux": js::gs(dest, "kind") == "tmux" }));
        }
        let r = self.validate(dest).await;
        if !js::gb(&r, "ok") {
            say_to(&o, &js::gs(&r, "say"));
            return r;
        }
        self.typed.borrow_mut().remove(key);
        json!({ "ok": true, "validated": true })
    }

    /// Mode terminal arrêté depuis peu : « je valide le prompt » envoie quand même Entrée (texte en attente).
    pub async fn handle_recent(&self, clean: &str, doubt: bool) -> bool {
        let Some((l, ended)) = self.last_target.borrow().clone() else { return false };
        if self.target.borrow().is_some() || js::now_ms() - ended > RECENT_MS || !self.enabled() {
            return false;
        }
        let f = fold_tmux(clean);
        let o = self.o.up();
        if !validate_re().is_match(&f) || !o.pending.borrow().is_empty() {
            return false;
        }
        let Some(dest) = self.dest_of(&l).await else { return false };
        let console = js::gs(&dest, "kind") == "console";
        if console && self.alive(&json!({ "hwnd": l.hwnd })) == Some(false) {
            return false;
        }
        let key = dest_key(&dest);
        if !self.typed.borrow().contains(&key) {
            return false;
        }
        if console {
            let r = o.desktop_action(l.hwnd, "afficher").await;
            if r["ok"] == json!(false) {
                return false;
            }
            js::sleep_ms(350).await;
        }
        let res = self.send_pending(&dest, &key, doubt).await;
        o.record("terminal_validation_differee", json!({ "tmux": !console, "ok": js::gb(&res, "ok"), "depuis_s": ((js::now_ms() - ended) as f64 / 1000.0).round() as i64 }));
        true
    }

    async fn local(&self, tool: &str, params: J, dest: &J, text: Option<String>, keys: Option<Vec<String>>) -> J {
        let o = self.o.clone();
        let dest = dest.clone();
        self.o
            .up()
            .local_tool(
                tool,
                params,
                Box::new(move |_| {
                    Box::pin(async move {
                        let orch = o.up();
                        if js::gs(&dest, "kind") == "tmux" {
                            let target = js::gs(&dest, "target");
                            let r = match (&text, &keys) {
                                (Some(t), _) => orch.tmux.send_line(&target, t, false).await,
                                (None, Some(k)) => orch.tmux.send_keys(&target, &k.iter().map(|s| s.as_str()).collect::<Vec<_>>()).await,
                                _ => Ok(()),
                            };
                            match r {
                                Ok(()) => json!({ "ok": true }),
                                Err(e) => json!({ "ok": false, "error": e.to_string() }),
                            }
                        } else {
                            let hwnd = dest["hwnd"].as_i64().unwrap_or(0);
                            let sk: String = keys.map(|k| k.iter().map(|x| sendkeys(x)).collect()).unwrap_or_default();
                            orch.desktop.paste_in_terminal(hwnd, &text.unwrap_or_default(), &sk).await
                        }
                    })
                }),
            )
            .await
    }

    async fn type_text(&self, dest: &J, text: &str) -> J {
        let label = js::gs(dest, "label");
        let r = self.local("mcp__system__terminal_dicter", json!({ "texte": text.trim(), "cible": label, "entree": false }), dest, Some(text.to_string()), None).await;
        // Trace pour comprendre qui prend le premier plan (nom du programme seulement, jamais de titre).
        if js::gb(&r, "refocalise") {
            self.o.up().record("terminal_refocalise", json!({ "programme": dest["label"] }));
        } else if js::gb(&r, "autre") {
            self.o.up().record("terminal_dictee_refusee", json!({ "devant": r["devant"] }));
        }
        if js::gb(&r, "ok") {
            return json!({ "ok": true });
        }
        let say = if js::gb(&r, "denied") {
            "Saisie refusée.".to_string()
        } else if js::gb(&r, "autre") {
            js::gs(&r, "message")
        } else {
            format!("Je n’ai pas pu écrire dans {}.", if js::gs(dest, "kind") == "tmux" { "tmux" } else { "le terminal" })
        };
        json!({ "ok": false, "say": say })
    }

    /// « Je valide le prompt » : Entrée.
    async fn validate(&self, dest: &J) -> J {
        let params = json!({ "cible": dest["label"], "programme": if js::gb(dest, "agent") { "agent" } else { "shell" } });
        let r = self.local("mcp__system__terminal_valider", params, dest, None, Some(vec!["Enter".into()])).await;
        if js::gb(&r, "ok") {
            return json!({ "ok": true });
        }
        let say = if js::gb(&r, "denied") { "Envoi refusé.".to_string() } else if js::gb(&r, "autre") { js::gs(&r, "message") } else { "Je n’ai pas pu appuyer sur Entrée.".into() };
        json!({ "ok": false, "say": say })
    }

    async fn press_keys(&self, dest: &J, keys: &[&str]) -> J {
        let name = keys.iter().map(|k| key_name(k)).collect::<Vec<_>>().join(" ");
        let sk: String = keys.iter().map(|k| sendkeys(k)).collect();
        if js::gs(dest, "kind") != "tmux" && sk.is_empty() {
            return json!({ "ok": false, "say": "Cette touche n’existe pas dans ce terminal." });
        }
        let r = self
            .local("mcp__system__terminal_dicter", json!({ "texte": format!("[touche {name}]"), "cible": dest["label"], "entree": false }), dest, None, Some(keys.iter().map(|k| k.to_string()).collect()))
            .await;
        if js::gb(&r, "ok") {
            return json!({ "ok": true });
        }
        let say = if js::gb(&r, "denied") { "Touche refusée.".to_string() } else if js::gb(&r, "autre") { js::gs(&r, "message") } else { format!("Je n’ai pas pu appuyer sur {name}.") };
        json!({ "ok": false, "say": say })
    }
}

#[allow(unused)]
fn _v(_: Value) {}
